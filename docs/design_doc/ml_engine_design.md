# ML_Engine 设计文档

Presented by KeJi
Created Date ： 2026-07-05
Modified Date ： 2026-07-05

---

## 目录

- [1. 模块概述](#1-模块概述)
- [2. 核心抽象](#2-核心抽象)
  - [Stage — 单步计算单元](#stage--单步计算单元)
  - [Model — 完整推理单元](#model--完整推理单元)
- [3. 结构体定义](#3-结构体定义)
  - [MlContext — 推理会话](#mlcontext--推理会话)
  - [GGUF_Model — 模型容器](#gguf_model--模型容器)
  - [Model_Arch_Info — 模型架构信息](#model_arch_info--模型架构信息)
  - [Message — 对话消息](#message--对话消息)
- [4. 模块方法](#4-模块方法)
  - [模型文件操作](#模型文件操作)
  - [MlContext 生命周期](#mlcontext-生命周期)
  - [MlContext 编解码](#mlcontext-编解码)
  - [MlContext 推理](#mlcontext-推理)
  - [MlContext offload](#mlcontext-offload)
- [5. 模块结构](#5-模块结构)
- [6. 使用示例](#6-使用示例)
- [7. 已知限制](#7-已知限制)

---

## 1. 模块概述

**模组等级：Level 0** — 仅依赖第三方 crate（candle-core、gguf_file、shimmytok、minijinja），不调用其他项目模块。

ML_Engine 是 Pleiades 的推理引擎核心，负责模型文件分析、权重加载、切分、推理会话管理。对外通过 `MlContext`（Lua userdata）暴露推理接口，通过独立函数提供文件级操作（分析/切分）。

**原则**：模型细节封装在 `GGUF_Models/qwen3/` 内，`gguf_model.rs` 只做编排调度。

分层架构：

```
上层
  context.rs          ← 推理会话生命周期（MlContext）
  capability.rs       ← 异步封装（spawn_blocking）
  mod.rs              ← re-export 公开符号

中层
  gguf_model.rs       ← 模型组装中枢（GGUF_Load_Model 编排）
  gguf_model_manager.rs ← GGUF 文件操作（分析/加载/切分）

底层
  GGUF_Models/
    common/           ← Stage/Model trait + RoPE + SwiGLU MLP
    qwen3/            ← Qwen3/Qwen3MoE 权重定义 + Stage/Model impl + Load_Stages
```

---

## 2. 核心抽象

### Stage — 单步计算单元

```rust
pub trait Stage: Send {
    fn forward(&mut self, x: &Tensor, offset: usize, mask: Option<&Tensor>) -> Result<Tensor>;
    fn kv_cache(&self) -> Option<&ConcatKvCache> { None }
    fn kv_cache_mut(&mut self) -> Option<&mut ConcatKvCache> { None }
    fn clear_kv_cache(&mut self) { /* 默认: kv_cache_mut().reset() */ }
}
```

**定位**：模型的最小可编排计算单元。Embedding、单个 Transformer Layer、Output 头均实现此 trait。

**实现者**：

| 实现 | 位置 | 说明 |
|------|------|------|
| `Qwen3_Embedding_Stage` | `qwen3/qwen3.rs` | 输入层，无 KV Cache |
| `Qwen3_Output_Stage` | `qwen3/qwen3.rs` | 输出头（norm + lm_head），无 KV Cache |
| `Layer_Weights` | `qwen3/qwen3.rs` | Qwen3 Dense Transformer 层 |
| `Qwen3MoE_Layer` | `qwen3/qwen3_moe.rs` | Qwen3 MoE Transformer 层 |

所有 Stage 通过 `Vec<Box<dyn Stage>>` 统一编排，`Model::Forward` 遍历执行。

### Model — 完整推理单元

```rust
pub trait Model: Send {
    fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor>;
    fn Clear_Kv_Cache(&mut self);
    fn extract_kv_cache(&self) -> Result<Vec<(Tensor, Tensor)>, String>;
    fn restore_kv_cache(&mut self, kvs: Vec<(Tensor, Tensor)>) -> Result<(), String>;
}
```

**定位**：组装好的完整模型。对外隐藏底层 Stage 编排细节，调用方（`MlContext`）只需关心这一个接口。

**实现者**：

| 实现 | 位置 | 说明 |
|------|------|------|
| `Model_Weights` | `qwen3/qwen3.rs` | Qwen3 Dense，stages 为 `[Embedding, L0..Ln, Output]` |
| `Qwen3MoE_Model` | `qwen3/qwen3_moe.rs` | Qwen3 MoE，同上但 Transformer 层为 MoE 变体 |

**Forward 语义**：`offset` 为 KV Cache 位置计数器。Prefill 时 `offset=0`，decode 时逐 token 递增（由 `MlContext.offset` 自动管理）。

**两层关系**：

```
MlContext::forward()
  → Model::Forward()
    → 构建 causal_mask（模型级，依赖 device/dtype）
    → 遍历 stages:
        Stage::forward(x, offset, mask?)  ← 单步
```

---

## 3. 结构体定义

### MlContext — 推理会话

```rust
pub struct MlContext {
    model: Option<GGUF_Model>,                    // 模型权重（None = 空壳）
    tokenizer: Option<shimmytok::Tokenizer>,       // tokenizer（独立加载）
    offset: usize,                                 // 自回归位置计数器
    rng_state: u64,                                // xoshiro256** 采样状态
    eos_token_id: u32,                             // EOS token ID
    chat_template: Option<String>,                 // GGUF metadata 中的 chat_template
    device: Device,                                // 运行设备

    // offload 状态
    offloaded_kv: Option<Vec<(Tensor, Tensor)>>,   // CPU 上的 KV Cache 副本
    offloaded_model_path: Option<PathBuf>,         // reload 用模型路径
    offloaded_layer_start: usize,                  // reload 用层范围
    offloaded_layer_end: usize,
}
```

**设计要点**：

- **空壳模式**：`new()` 创建的 session 不加载模型，可调用 `tensorize()`、`set_seed()` 等无模型依赖方法。`load_model()` 填充空壳。
- **tokenizer 独立**：`load_tokenizer()` 单独加载，与模型生命周期解耦。Session 初始化时先 load_tokenizer 获取 metadata，后 load_model。
- **offset 自动管理**：`forward()` 中若未传入显式 offset，自动使用 `self.offset` 并在每次调用后递增 `seq_len`。`load_model()` 和 `unload()` 重置为 0。

### GGUF_Model — 模型容器

```rust
pub struct GGUF_Model {
    pub model: Box<dyn Model + Send>,   // trait object（Qwen3 / Qwen3MoE）
    pub tokenizer: Option<Tokenizer>,   // 预留字段（当前由 MlContext 管理）
    pub arch_info: Model_Arch_Info,     // 架构元信息
    pub model_path: PathBuf,            // 模型文件路径
    pub device: Device,                 // 推理设备
    pub has_input_head: bool,           // 是否含 embedding
    pub has_output_head: bool,          // 是否含 lm_head
}
```

由 `GGUF_Load_Model()` 组装，`MlContext.load_model()` 持有。通过 `Box<dyn Model + Send>` 实现多态。

### Model_Arch_Info — 模型架构信息

```rust
pub struct Model_Arch_Info {
    pub architecture: String,          // qwen3 / qwen3moe
    pub num_layers: usize,             // Transformer 层数
    pub embedding_length: usize,
    pub head_count: usize,
    pub head_count_kv: usize,
    pub head_dim: usize,
    pub feed_forward_length: usize,
    pub context_length: usize,
    pub rms_norm_eps: f64,
    pub rope_freq_base: f64,
    pub vocab_size: usize,
    pub eos_token_id: u32,
    pub chat_template: Option<String>,  // GGUF metadata 中的 Jinja2 模板
    pub layers: Vec<Layer_Info>,        // 逐层 tensor 详情
    pub non_layer_tensors: Vec<Tensor_Detail>,
    pub metadata_raw: HashMap<String, String>,

    // PGGUF 专有字段
    pub is_split: bool,                 // 是否切分模型
    pub split_start: usize,
    pub split_end: usize,
    pub model_id: Option<u32>,          // xxhash32 唯一标识
    pub layer_bitmap: Option<[u8; 32]>, // 256 位层位图
    pub weight_format: Option<String>,  // safetensors 标记（DeepSeek V4）
    pub num_shards: Option<usize>,
}
```

由 `GGUF_Analyze_From_Content` / `GGUF_Analyze_And_Convert` 解析 GGUF metadata 填充。

### Message — 对话消息

```rust
pub struct Message {
    pub role: String,     // "system" / "user" / "assistant"
    pub content: String,
}
```

OpenAI 兼容格式。用于 `MlContext::encode()` 的 chat_template 渲染。

---

## 4. 模块方法

### 模型文件操作

#### analyze_model

```rust
pub async fn analyze_model(path: &Path) -> Result<Model_Arch_Info, String>;
```

| | 说明 |
|------|------|
| **输入** | `.gguf` 或 `.pgguf` 文件路径 |
| **输出** | 模型架构信息 |
| **内部逻辑** | `spawn_blocking` → `GGUF_Analyze_And_Convert`（GGUF 自动转 PGGUF） |

异步封装，供 Storage flush、Lua 分析模型时调用。

#### split_model

```rust
pub async fn split_model(
    src: &Path, start: usize, end: usize,
    output_dir: &Path, keep_tokenizer: bool,
) -> Result<(), String>;
```

| | 说明 |
|------|------|
| **输入** | 源文件 + 层范围 + 输出目录 + 是否保留 tokenizer |
| **输出** | `{stem}_split_{start}_{end}.pgguf` 写入 output_dir |
| **内部逻辑** | `spawn_blocking` → `GGUF_Split_Model` |

层编号规则：`0=embedding, 1..N=transformer, N+1=output`。

#### GGUF_Load_Model

```rust
pub fn GGUF_Load_Model(
    start: usize, end: usize, model_path: &Path, device: &Device,
) -> Result<GGUF_Model>;
```

| | 说明 |
|------|------|
| **输入** | 层范围 + 模型路径 + 设备 |
| **输出** | 组装好的 `GGUF_Model`（含权重 + Tokenizer + arch_info） |
| **内部逻辑** | 路径解析（自动补 `.pgguf`/`.gguf`）→ PGGUF 转换 → 架构校验 → 构建 RoPE → 加载 embedding → 逐层加载（`Load_Stages`）→ 加载 output head → 组装 |

**编排模式**：通用准备逻辑（路径、Range、RoPE、embedding、output head）在 `gguf_model.rs` 中，模型特定的层加载循环在 `qwen3/qwen3.rs::Load_Stages` 和 `qwen3/qwen3_moe.rs::Load_Stages` 中。

### MlContext 生命周期

#### new

```rust
pub fn new(device: &str) -> Result<Self, String>;
```

| | 说明 |
|------|------|
| **输入** | `"cpu"` 或 `"cuda"` |
| **输出** | 空壳 `MlContext`（model=None, tokenizer=None） |
| **副作用** | 随机种子基于当前系统时间 |

#### load_model

```rust
pub fn load_model(&mut self, path: &Path, start: usize, end: usize) -> Result<(), String>;
```

| | 说明 |
|------|------|
| **输入** | `.pgguf` 路径 + 层范围 |
| **输出** | 模型加载到 session |
| **内部逻辑** | 先加载新模型（失败则旧模型不动）→ 卸载旧模型 → 替换 → 保存 reload 信息 |

#### load_tokenizer

```rust
pub fn load_tokenizer(&mut self, path: &Path) -> Result<(), String>;
```

| | 说明 |
|------|------|
| **输入** | GGUF/PGGUF 文件路径 |
| **输出** | tokenizer + eos_token_id + chat_template 注入 |
| **内部逻辑** | `shimmytok::from_gguf_file` → `GGUF_Analyze_From_Content` 取 metadata |

#### unload / has_model

```rust
pub fn unload(&mut self);        // 卸载模型 + tokenizer，回到空壳
pub fn has_model(&self) -> bool; // 模型是否已加载
```

### MlContext 编解码

#### encode

```rust
pub fn encode(&self, messages: &[Message]) -> Result<Vec<u32>, String>;
```

| | 说明 |
|------|------|
| **输入** | OpenAI 格式的 Message 数组 |
| **输出** | token ID 序列 |
| **内部逻辑** | `apply_chat_template`（优先 GGUF 模板 → fallback 硬编码 Qwen3）→ tokenize |

chat_template 渲染使用 minijinja + `pycompat::unknown_method_callback`，原生支持 Python 方法调用（`.startswith()`, `.split()`, `.strip()` 等）。

#### decode

```rust
pub fn decode(&self, token_id: u32) -> Result<String, String>;
```

单 token 解码。EOS token 解码为空字符串。

### MlContext 推理

#### tensorize

```rust
pub fn tensorize(&self, token_ids: &[u32]) -> Result<Tensor, String>;
```

| | 说明 |
|------|------|
| **输入** | token ID 数组 |
| **输出** | `Tensor[1, seq_len]` |
| **内部逻辑** | `Tensor::new` → `unsqueeze(0)` |

不依赖模型，空壳状态即可调用。

#### forward

```rust
pub fn forward(&mut self, tensor: &Tensor, offset: Option<usize>) -> Result<Tensor, String>;
```

| | 说明 |
|------|------|
| **输入** | 输入 tensor + 可选 offset |
| **输出** | logits tensor |
| **内部逻辑** | `GGUF_Model_Inference` → offset 自动管理（None → self.offset，自动递增） |

#### sample

```rust
pub fn sample(&mut self, logits: &Tensor, temperature: f64) -> Result<u32, String>;
```

| | 说明 |
|------|------|
| **输入** | logits + 温度 |
| **输出** | 采样出的 token ID |
| **内部逻辑** | xoshiro256** 随机数生成 → top-p (nucleus) sampling |

#### reset_kv_cache

```rust
pub fn reset_kv_cache(&mut self);
```

清除模型内所有层的 KV Cache。每轮对话开始前调用。

### MlContext offload

#### offload_to_cpu

```rust
pub fn offload_to_cpu(&mut self) -> Result<(), String>;
```

| | 说明 |
|------|------|
| **内部逻辑** | extract KV Cache → 移到 CPU → 释放 GPU 模型 → 保存 reload 信息 |

#### offload_to_cuda

```rust
pub fn offload_to_cuda(&mut self) -> Result<(), String>;
```

| | 说明 |
|------|------|
| **内部逻辑** | 用 reload 信息重新加载模型 → KV 移回 GPU → restore → 清空 offload 状态 |

#### offload_save / offload_load

```rust
pub fn offload_save(&mut self, file_id: &str) -> Result<(), String>;
pub fn offload_load(file_id: &str, device_str: &str) -> Result<MlContext, String>;
```

KVCX 格式的二进制度盘序列化。save 写入 `kvcache_dir/file_id`，load 反序列化并恢复完整推理状态。

**KVCX 格式**：

```
Header:  KVCX (4B magic) + version (u32 LE)
         + model_path_len (u32) + model_path (UTF-8)
         + start (u32) + end (u32) + rng_state (u64)
         + offset (u64) + eos_token_id (u32)
         + chat_template_len (u32) + chat_template (UTF-8)
         + num_layers (u32)

Per-layer:  ndim (u32) + shape[] (u64 × ndim)
            + data_len (u64) + data (f32 LE × elem_count)
            × 2 (K then V)
```

---

## 5. 模块结构

```
Src/ML_Engine/
├── mod.rs                   ← re-export 公开符号
├── device.rs                ← Parse_Device_Str (71行)
├── lua_tensor.rs            ← LuaTensor UserData
│
├── gguf_model_manager.rs    ← GGUF 文件操作 (869行)
│   ├── Model_Arch_Info / Layer_Info / Tensor_Detail / GGUF_Layer_Weights
│   ├── GGUF_Analyze_From_Content / GGUF_Analyze_And_Convert
│   ├── GGUF_Load_Layer / GGUF_Split_Model
│   └── Resolve_Model_Path / Bitmap / Build_Layer_Bitmap
│
├── gguf_model.rs            ← 模型组装编排 (270行)
│   ├── GGUF_Model
│   ├── GGUF_Load_Model（编排，调度到 qwen3::Load_Stages）
│   └── GGUF_Model_Inference / GGUF_Model_Clear_KV_Cache
│
├── context.rs               ← 推理会话 (1004行)
│   ├── MlContext 生命周期 + 编解码 + 推理 + offload
│   ├── chat_template 渲染（minijinja + pycompat）
│   └── mlua UserData 注册
│
├── capability.rs            ← 异步封装 (89行)
│   ├── analyze_model
│   └── split_model
│
├── gguf_model_legacy.rs     ← DeepSeek/Llama 参考代码（不编译）
│
└── GGUF_Models/
    ├── mod.rs               ← pub mod common / qwen3
    ├── common/
    │   ├── mod.rs
    │   ├── stage.rs         ← trait Stage (23行，零依赖)
    │   ├── model.rs         ← trait Model + extract/restore 工具 (50行)
    │   ├── rope.rs          ← Rotary_Embedding
    │   └── swiglu_mlp.rs    ← SwiGLU MLP
    └── qwen3/
        ├── mod.rs           ← 模块声明 + re-export
        ├── qwen3.rs         ← Qwen3 Dense: Attention/Layer/Model + Stage/Model impl + Load_Stages
        └── qwen3_moe.rs     ← Qwen3 MoE: Qwen3MoE_Layer/Model + Stage/Model impl + MoE_Cfg + Load_Stages
```

---

## 6. 使用示例

### Rust 侧

```rust
use pleiades::ml_engine::{MlContext, context::Message, analyze_model};

// 分析模型
let info = analyze_model(Path::new("model.pgguf")).await?;
println!("架构: {}, 层数: {}", info.architecture, info.num_layers);

// 创建会话
let mut sess = MlContext::new("cpu")?;

// 加载 tokenizer（获取 metadata）
sess.load_tokenizer(Path::new("model.pgguf"))?;

// 加载模型权重
sess.load_model(Path::new("model.pgguf"), 0, 29)?;

// 编码
let messages = vec![Message {
    role: "user".into(),
    content: "hello".into(),
}];
let tokens = sess.encode(&messages)?;

// 推理
let input = sess.tensorize(&tokens)?;
let logits = sess.forward(&input, Some(0))?;
let token = sess.sample(&logits, 0.8)?;
let text = sess.decode(token)?;

// 下一轮
sess.reset_kv_cache();

// 卸载
sess.unload();
```

### Lua 侧

```lua
-- 创建会话
local sess = ml.new("cpu")

-- 加载模型
sess:load_tokenizer("model.pgguf")
sess:load_model("model.pgguf", 0, 29)

-- 编码
local tokens = sess:encode("hello")

-- 推理
local input = sess:tensorize(tokens)
local logits = sess:forward(input, 0)
local tok = sess:sample(logits, 0.8)
local text = sess:decode(tok)

-- 清理
sess:reset_kv_cache()
sess:unload()
```

---

## 7. 已知限制

| 限制 | 说明 |
|------|------|
| 当前仅支持 Qwen3/Qwen3MoE | DeepSeek/Llama 代码在 `gguf_model_legacy.rs`，通过 feature flag 门控 |
| PGGUF 强制转换 | `GGUF_Load_Model` 内部调用 `Analyze_And_Convert`，GGUF 自动转 PGGUF 并删除原始文件 |
| tokenizer 依赖文件路径 | shimmytok 仅支持 `from_gguf_file(path)`，无法复用已打开的 Content |
| chat_template fallback | 若无 GGUF 模板，使用硬编码 Qwen3 格式（仅最后一轮 user 消息） |
| 单设备绑定 | session 创建时绑定 device，运行时不可切换。offload 机制可临时移出/移回 |
| forward/sample 非异步 | 在 `spawn_blocking` 中运行，不阻塞 tokio 运行时 |
| offload 仅支持 CPU↔GPU | 不支持 GPU→GPU 或 CPU→其他加速器 |
| sample 无 top-k | 仅实现 top-p (nucleus) sampling |
| context.rs 过大 | ~1000 行，chat_template 渲染和 offload 序列化可拆分为独立文件 |
