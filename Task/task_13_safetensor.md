Presented by KeJi
Date: 2026-05-31

# Task 13: Safetensor 支持实现

> 状态：13.1~13.8 已完成，待 13.9 集成验证（测试模型：Qwen3-0.6B）

---

## 背景

Pleiades 目前仅支持 GGUF 格式的模型加载（通过 `GGUF_Load_Model` 从 `.pgguf` 文件加载权重与 tokenizer）。HuggingFace 生态的主流格式是 safetensors，要扩展模型来源，需要支持将 safetensors 格式的模型转换为内部 PGGUF 格式。

safetensors 文件只存权重张量（`{tensor_name: raw_bytes}`），不含架构描述和 tokenizer。这些信息由同目录下的 `config.json` 和 `tokenizer.json` 提供。

## 目标

实现一个 **Python 转换器**，将 HuggingFace 标准目录布局的 safetensors 模型转换为一比一的全精度 PGGUF 格式。**不涉及量化**，权重保持原始精度（FP16/BF16/FP32）。Rust 推理侧零改动。

```
HuggingFace 目录                 Python 转换器              Pleiades
──────────────                   ─────────────              ────────
config.json         ─┐
*.safetensors       ─┤──→  hf2gguf.py  ──→ model.pgguf ──→ GGUF_Load_Model
tokenizer.json      ─┘                                   (零改动)
```

## 方案

### 技术选型

| 层 | 选择 | 说明 |
|----|------|------|
| 语言 | Python 3 | 生态成熟，两个格式均有官方库 |
| 读 safetensors | `safetensors` (0.7.0) | HuggingFace 官方，`safe_open()` |
| 写 GGUF | `gguf` (0.19.0) | llama.cpp 的 gguf-py，`GGUFWriter` |
| 依赖管理 | venv (`/workspace/.venv`) | 已创建，已安装上述依赖 |

### 转换流程

```
1. 读取 config.json
   └→ 提取架构信息：architecture, num_layers, hidden_size, head_count, ...
   └→ 映射为 GGUF metadata keys

2. 读取 tokenizer.json
   └→ 提取 vocab, merges, special tokens, chat_template
   └→ 通过 GGUFWriter.add_token_* / add_chat_template 写入

3. 读取 *.safetensors
   ├ 有 model.safetensors.index.json？
   │  ├ 是 → 按 weight_map 逐个分片文件读取
   │  └ 否 → 直接读 model.safetensors
   └→ HashMap<String, Tensor>

4. Tensor name 映射
   └→ model.layers.0.self_attn.q_proj.weight → blk.0.attn_q.weight
   └→ 按架构查表映射

5. 写入 GGUF
   └→ GGUFWriter 写入 metadata + 逐 tensor 写入权重

6. 追加 PGGUF 元数据
   └→ xxhash32 → model_id
   └→ 构建 256-bit layer_bitmap
   └→ 写入 pleiades.model_id / pleiades.layer_bitmap
```

### Tensor Name 映射（Qwen3 示例）

```
HF safetensors name                        GGUF name
──────────────────────────────────────     ──────────────────────
model.embed_tokens.weight                  token_embd.weight
model.layers.{n}.self_attn.q_proj.weight   blk.{n}.attn_q.weight
model.layers.{n}.self_attn.k_proj.weight   blk.{n}.attn_k.weight
model.layers.{n}.self_attn.v_proj.weight   blk.{n}.attn_v.weight
model.layers.{n}.self_attn.o_proj.weight   blk.{n}.attn_output.weight
model.layers.{n}.input_layernorm.weight    blk.{n}.attn_norm.weight
model.layers.{n}.post_attention_layernorm  blk.{n}.ffn_norm.weight
model.layers.{n}.mlp.gate_proj.weight      blk.{n}.ffn_gate.weight
model.layers.{n}.mlp.up_proj.weight        blk.{n}.ffn_up.weight
model.layers.{n}.mlp.down_proj.weight      blk.{n}.ffn_down.weight
model.norm.weight                          output_norm.weight
lm_head.weight                             output.weight
```

> **weight tying**: 若 `lm_head.weight` 不存在，GGUF 侧不写入 `output.weight`，Rust 加载时会自动 fallback 到 `token_embd.weight`。

### 分片支持

大模型的 safetensors 会分片为多个文件：

```
Qwen3-235B/
├── config.json
├── tokenizer.json
├── model.safetensors.index.json    ← weight_map: tensor → file
├── model-00001-of-00010.safetensors
├── model-00002-of-00010.safetensors
└── ...
```

转换器先检查是否存在 `index.json`：
- **有**：解析 `weight_map`，按需打开对应分片
- **无**：直接读唯一的 `.safetensors` 文件

### 全精度 GGUF 的可行性

**不涉及量化，全部权重保持 FP16/BF16/FP32 原始精度。**

candle 的 `QTensor` 可承载任意 dtype（包括非量化的 F16/F32），`QMatMul` 对非量化 QTensor 会 fallback 到普通矩阵乘法，`RmsNorm::from_qtensor` 直接 dequantize。因此 Rust 侧的 `GGUF_Load_Model` 路径无需任何修改即可加载全精度 GGUF。

> 代价：无量化压缩，大模型内存占用为原始精度大小。但对于开发验证和中小模型，这不是问题。

---

## 任务拆解

### 13.1 创建分支

**基于 `reforge` 创建 `hf2gguf` 分支**

```bash
git checkout reforge
git checkout -b hf2gguf
```

---

### 13.2 Python 项目骨架

**目录结构**：

```
Tool/hf2gguf/
├── __init__.py
├── converter.py          # 主转换逻辑编排
├── tokenizer_writer.py   # tokenizer.json → GGUF metadata
├── pgguf_writer.py       # pleiades 元数据（model_id, layer_bitmap）
├── config_reader.py      # config.json 解析 → GGUF metadata keys
├── mappings/
│   ├── __init__.py
│   ├── qwen3.py          # Qwen3 映射表
│   ├── qwen3_moe.py      # Qwen3MoE 映射表
│   └── deepseek.py       # DeepSeek 映射表
└── utils.py              # 公共工具（分片索引解析等）

Tool/convert_hf_to_gguf.py  # CLI 入口
```

**涉及文件**：全部新建，共 ~10 个文件。

---

### 13.3 Qwen3 映射表 + 基础转换器

**涉及文件**：
- `Tool/hf2gguf/mappings/qwen3.py` — 新增
- `Tool/hf2gguf/converter.py` — 新增
- `Tool/hf2gguf/config_reader.py` — 新增
- `Tool/hf2gguf/utils.py` — 新增
- `Tool/convert_hf_to_gguf.py` — 新增

**内容**：
1. `qwen3.py` 定义 `TENSOR_MAP: dict[str, str]`，HF name → GGUF name
2. `config_reader.py` 解析 `config.json`，提取 Qwen3 架构参数，构造 GGUF metadata dict
3. `converter.py` 编排：读 config → 读 safetensors → 映射 → 写 GGUF
4. `utils.py` 实现分片索引解析等公共逻辑
5. `convert_hf_to_gguf.py` 提供 CLI：`python convert_hf_to_gguf.py <model_dir> [-o output.pgguf]`

---

### 13.4 Tokenizer 写入

**涉及文件**：
- `Tool/hf2gguf/tokenizer_writer.py` — 新增

**内容**：
- 读取 `tokenizer.json`
- 提取 vocab, merges, special tokens (bos/eos/unk), chat_template
- 通过 `GGUFWriter.add_token_list/add_token_merges/add_chat_template/...` 写入

**参考**：现有 Rust 侧通过 `shimmytok::Tokenizer::from_gguf_file()` 从 GGUF 读 tokenizer，目标是与该接口兼容。

---

### 13.5 PGGUF 元数据追加

**涉及文件**：
- `Tool/hf2gguf/pgguf_writer.py` — 新增

**内容**：
1. 对整个 GGUF 内容计算 `xxhash32` → `model_id`
2. 按层构建 256-bit `layer_bitmap`
3. 写入 `pleiades.model_id` 和 `pleiades.layer_bitmap`

**参考**：Rust 侧 `GGUF_Analyze_And_Convert`（`gguf_model_manager.rs`）的对应逻辑。

---

### 13.6 分片 safetensors 支持

**涉及文件**：
- `Tool/hf2gguf/utils.py` — 修改（加分片索引解析）

**内容**：
- 解析 `model.safetensors.index.json` 的 `weight_map`
- 实现 `open_sharded_tensors(model_dir)` → 统一的 tensor 迭代器

---

### 13.7 Qwen3MoE 映射

**涉及文件**：
- `Tool/hf2gguf/mappings/qwen3_moe.py` — 新增

**额外张量**（相比 Qwen3 多出的 MoE 相关）：

```
HF name                                     GGUF name
──────────────────────────────────────      ────────────────────────────
model.layers.{n}.mlp.gate.wg.weight         blk.{n}.ffn_gate_inp.weight
model.layers.{n}.mlp.experts.gate_proj      blk.{n}.ffn_gate_exps.weight
model.layers.{n}.mlp.experts.up_proj        blk.{n}.ffn_up_exps.weight
model.layers.{n}.mlp.experts.down_proj      blk.{n}.ffn_down_exps.weight
```

---

### 13.8 DeepSeek 映射

**涉及文件**：
- `Tool/hf2gguf/mappings/deepseek.py` — 新增

DeepSeek 使用 MLA（Multi-head Latent Attention），命名规则不同，需要独立映射表。

---

### 13.9 集成验证

**不涉及新文件**。验证步骤：

1. 准备测试模型（Qwen3-0.6B safetensors 版本，从 HuggingFace 下载）
2. 运行 `python Tool/convert_hf_to_gguf.py <model_dir>`
3. 用 Rust 侧 `GGUF_Load_Model` 加载产出的 `.pgguf`
4. 运行一次推理，验证 tokenizer encode/decode + forward 正常

---

### 13.10 Storage 集成（后续）

**涉及文件**（Rust 侧）：
- `Src/Storage/storage_manager.rs` — 修改

**内容**：
- `flush()` 扫描时识别 `config.json` + `*.safetensors` 目录结构
- 自动调用 Python 转换器（或提示用户手动转换）

---

### 13.11 文档

**涉及文件**：
- `docs/hf2gguf.md` — 新增，使用说明
- `Task/task_13_safetensor.md` — 本文档，持续更新

---

## 分支与文件变更总览

```
分支：reforge → hf2gguf

新增文件 (12个)：
  Tool/hf2gguf/__init__.py
  Tool/hf2gguf/converter.py
  Tool/hf2gguf/config_reader.py
  Tool/hf2gguf/tokenizer_writer.py
  Tool/hf2gguf/pgguf_writer.py
  Tool/hf2gguf/utils.py
  Tool/hf2gguf/mappings/__init__.py
  Tool/hf2gguf/mappings/qwen3.py
  Tool/hf2gguf/mappings/qwen3_moe.py
  Tool/hf2gguf/mappings/deepseek.py
  Tool/convert_hf_to_gguf.py
  docs/hf2gguf.md

已有文件修改：
  （无，Rust 侧零改动）

后续可能修改（13.10 Storage 集成）：
  Src/Storage/storage_manager.rs
```

---

## 人类评审

<!-- 在此区域写下评审意见 -->
