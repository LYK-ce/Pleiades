Presented by KeJi
Date: 2026-06-01

# Task 15: DeepSeek V4 Flash 集成

> 状态：⏸️ 暂缓并归档 (2026-06-15)。方案已确定，转入后续迭代。

---

## 背景

DeepSeek V4 Flash（284B, 43 层, Flash 版）只有 safetensors 格式发布，无 GGUF。Pleiades 现有架构无法直接加载。

社区已有两个关键资源：
- **nsparks** 的 llama.cpp fork 实现了 DeepSeek V4 的 GGUF 支持（新增 F8_E4M3_B128 + MXFP4 类型），但上游 candle 未合并
- **MScanter** 的 `deepseek-v4-candle` 用纯 Rust + candle 重写了 DeepSeek V4 全部架构组件，并通过 59 个 TDD 测试验证正确性

MScanter 仓库信息：
- 地址：`https://github.com/MScanter/deepseek-v4-candle`
- MIT 许可，独立实现，非 fork
- 核心模块：`quant.rs` (FP4/FP8 dequant), `loader.rs` (safetensors mmap), `attention.rs` (MLA), `moe.rs` (MoE), `mhc.rs` (超连接), `block.rs`, `model.rs`
- 使用方式：**直接 clone 后复制/参考代码，不作为 git submodule**

本任务利用 MScanter 的架构实现，通过 PGGUF 容器包装 safetensors 权重，将 DeepSeek V4 Flash 集成到 Pleiades。

---

## 目标

在 Pleiades 中支持 DeepSeek V4 Flash 模型，借助 MScanter 的 candle 实现，将 safetensors 权重原样存入 PGGUF（不膨胀），Rust 侧根据 metadata 标记自动分发到 safetensors 加载路径。

---

## 方案

### 核心思路

**GGUF 当容器，不转换精度：**

```
Python: safetensors 分片原样 → PGGUF (~160GB)
         metadata 写入 pleiades.weight_format = "safetensors"

Rust:   GGUF_Analyze 读 metadata → 检测 safetensors 标记
         → mmap safetensors blob → MScanter 的 quant.rs dequant
         → MScanter 的架构组件 (attention, moe, mhc, block)
         → MlSession forward/sample/decode（不改）
```

### 与现有路径的关系

```
GGUF_Load_Model:
  match metadata.get("pleiades.weight_format"):
      Some("safetensors") → DeepSeekV4Model::from_pgguf()  [新路径]
      _                   → gguf_file::Content → QMatMul   [现有路径，兼容所有已有 PGGUF]
```

---

## 任务拆解

### 15.1 Python：safetensors 打包进 PGGUF

**改 `Tool/hf2gguf/`，新增 `--wrap-native` 模式。**

- 解析 `config.json` → GGUF metadata keys
- 解析 `tokenizer.json` + `tokenizer_config.json` → GGUF tokenizer metadata
- 解析 `model.safetensors.index.json`
- 将 46 个 safetensors 分片作为 GGUF tensor blobs 写入：tensor 名 `safetensors/shard-N`，内容 = 分片文件原始字节
- 写入 `pleiades.weight_format = "safetensors"` 标记
- 写入 `pleiades.model_id` + `pleiades.layer_bitmap`

**产出**：单个 `DeepSeek-V4-Flash.pgguf`，~160GB。

**涉及文件**：
- `Tool/hf2gguf/converter.py` — 修改，加 native-wrap 模式
- `Tool/hf2gguf/wrap_native.py` — 新增

---

### 15.2 Rust：PGGUF 读取分支

**改 `gguf_model_manager.rs` / `gguf_model.rs`。**

- `GGUF_Analyze_And_Convert` 读 `pleiades.weight_format`
- `= "safetensors"` → 跳过 GGUF→PGGUF 转换，直接返回
- `GGUF_Load_Model` 检测标记 → 分发到 safetensors 路径

**涉及文件**：
- `Src/ML_Engine/gguf_model_manager.rs` — 修改 (~5 行)
- `Src/ML_Engine/gguf_model.rs` — 修改，`AnyModel` 加变体 + 加载分支

---

### 15.3 Rust：集成 MScanter 架构

**新增 `ML_Engine/GGUF_Models/deepseek_v4/`。**

| 文件 | 来源 | 说明 |
|------|------|------|
| `mod.rs` | 新写 | `DeepSeekV4Model` 结构体 + `from_pgguf()` 入口 + `AnyModel` 集成 |
| `attention.rs` | MScanter | MLA + sink softmax + sparse attn |
| `moe.rs` | MScanter | sqrt(softplus) gate + SwiGLU experts |
| `mhc.rs` | MScanter | Sinkhorn 双向随机矩阵 |
| `block.rs` | MScanter | mHC 包装的 decoder layer |
| `model.rs` | MScanter | `Transformer::from_config` |
| `quant.rs` | MScanter | FP8/FP4/UE8M0 → f32 dequant |
| `loader.rs` | 改编 | 从 PGGUF tensor blob 读 safetensors（而非直接读文件） |

**涉及文件**：全部新增，~8 个文件，~1200 行（MScanter 现有代码为主）。

---

### 15.4 Rust：补齐功能

| 功能 | 量 | 说明 |
|------|-----|------|
| **KV cache** | ~80 行 | `Mla` 加 `kv_cache: Option<Tensor>`，forward 改 `&mut self`，新 token 拼接缓存而非重算全部 |
| **hash routing** | ~10 行 | 加载前 3 层 `layers.{n}.ffn.gate.tid2eid`，`Gate::route_hashed` 查表 |

#### 现有 KV cache 基础设施（需感知）

代码库已有完善的 KV cache 体系，DeepSeek V4 需对齐：

| 组件 | 位置 | 说明 |
|------|------|------|
| `MLA_KV_Cache` | `deepseek_v3.rs:48` | V3 的 MLA 压缩 KV 缓存，存 kv_latent 而非完整 K/V，有 `Append`/`Reset`。V4 也走 MLA，可直接复用设计 |
| `Clear_Kv_Cache` | `gguf_model.rs:770` | 清除全部层 KV，每轮对话前调用。V4 需在 `DeepSeekV4Model` 和 `attention.rs` 实现对应方法 |
| `extract_kv_cache` | `gguf_model.rs:66` | 提取全部层 KV 用于 offload。支持 Qwen3/Qwen3MoE/DeepSeekV3 三个分支，需新增 V4 分支 |
| `restore_kv_cache` | `gguf_model.rs:108` | 恢复 KV 到各层。同上，需新增 V4 分支 |
| `offload_to_cpu/cuda` | `context.rs:547` | MlSession 的 KV offload 管理，调用 extract/restore |
| `save_kv_cache/load_kv_cache` | `context.rs:608` | `.kvcache/` 目录持久化，启动时 `main.rs:72` 自动创建目录 |
| `reset_kv_cache` | `context.rs:519` | MlSession 暴露的 Lua API：`sess:reset_kv_cache()` |

**V4 特有考虑：**

- V4 的 KV 是单头 latent（`head_dim=512`，1 个 KV 头），比 V3 更简单，缓存量极小（~1KB/token/layer）
- V4 的 `Compressor` 和 `Indexer` 有中间状态（compressed KV blocks、indexer scores），但这些都是 per-forward 重算的，不需要缓存
- MScanter 的 `forward` 目前接受 `start_pos` 参数，天然支持增量推理——只需在 `Mla` 结构体保存上一轮的 KV tensor

#### 现有 weight_format 兼容性

已有 PGGUF 文件不含 `pleiades.weight_format` key。加载逻辑用 `match` 兜底：

```rust
match metadata.get("pleiades.weight_format") {
    Some("safetensors") => /* V4 新路径 */,
    _                   => /* 现有 GGUF 路径，兼容全部已有模型 */,
}
```

涉及文件：
- `Src/ML_Engine/GGUF_Models/deepseek_v4/attention.rs` — 加 KV cache（参考 `MLA_KV_Cache`）
- `Src/ML_Engine/GGUF_Models/deepseek_v4/moe.rs` — 加 hash routing
- `Src/ML_Engine/gguf_model.rs` — `extract_kv_cache`/`restore_kv_cache` 加 V4 分支

---

### 15.5 MlSession 适配

**不需要改。** `tensorize → forward → sample → decode` 接口不变。

`DeepSeekV4Model::forward(input, offset)` 直接委托给 `Transformer::forward`。

---

### 15.6 验证

| 步骤 | 内容 |
|------|------|
| L1 | `GGUF_Analyze` 正确读取 metadata |
| L2 | `from_pgguf` 从 160GB PGGUF 加载全部 43 层 |
| L3 | `forward` 单步输出与 Python 参考一致 |
| L4 | 多 token 自回归生成通顺中文 |

---

## 不改的东西

- MlSession 的 inference loop
- Orchestrator / Core 主循环
- Session Manager
- TUI
- 现有的 Qwen3 / DeepSeek V3 模型加载逻辑

---

## 文件变更总览

```
分支：hf2gguf
已有：
  Src/ML_Engine/GGUF_Models/deepseek_v3.rs  (task14 已改进)
  programs/user/pipe_7.lua / pipe_8.lua      (task14 已新增)

新增：
  Src/ML_Engine/GGUF_Models/deepseek_v4/
    mod.rs, attention.rs, moe.rs, mhc.rs, block.rs, model.rs, quant.rs, loader.rs
  Tool/hf2gguf/wrap_native.py

修改：
  Src/ML_Engine/gguf_model_manager.rs  (~5 行)
  Src/ML_Engine/gguf_model.rs         (~20 行)
  Tool/hf2gguf/converter.py
```

---

## 人类评审

<!-- 在此区域写下评审意见 -->
