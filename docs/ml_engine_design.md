# ML Engine 设计文档

Presented by KeJi
Date ： 2026-05-16

## 1. 模块概述

`ML_Engine` 模块负责 Pleiades 分布式推理系统的**模型推理计算**。它不负责网络传输（TensorStream 的事）、不负责文本 IO（Session_Manager 的事）、不负责控制流（Lua 的事）。对外暴露一组原子函数，由上层编排。

### 核心定义

> **ML Engine = 纯推理计算层。** 只负责模型加载、推理计算、编解码、采样。

### 模块结构

```
ML_Engine/
├── context.rs           ← MlContext 结构体 + 7 个公开方法 + 状态查询
├── capability.rs        ← 独立操作：Analyze_Model / Split_Model
├── gguf_model.rs        ← 模型抽象：load / unload / inference / encode / decode
├── gguf_model_manager.rs← 底层 GGUF 操作：analyze / load_layer / split
├── gguf_tensor.rs       ← 张量序列化（网络传输用）
├── GGUF_Models/
│   ├── mod.rs
│   └── qwen3.rs         ← Qwen3 权重结构 + Forward
├── pipeline.rs          ← 参数/结果类型（旧，远期删除）
├── session.rs           ← Session/线程管理（旧，远期删除）
├── service.rs           ← ML_Engine_Service（旧，远期删除）
├── ML_VM/               ← 指令执行引擎（旧，远期删除）
└── mod.rs               ← 模块入口
```

### 调用关系（目标架构）

```
Lua 脚本
   │
   ├── ml:load_model / unload_model
   ├── ml:encode / decode
   ├── ml:tensorize / forward / sample
   └── ml:get_output_tensor / get_eos / get_offset
         │
         ▼
      MlContext          ← context.rs (模型运行时状态)
         │
         ▼
   GGUF_Model            ← gguf_model.rs (模型权重 + tokenizer)
         │
         ▼
   Model_Weights         ← qwen3.rs (embedding + layers + norm + lm_head)
```

---

## 2. 数据结构

### 2.1 MlContext — 推理运行时上下文

Lua 线程绑定的推理状态。外部不可见，仅通过公开方法操作。

```rust
struct MlContext {
    /// 模型权重 + tokenizer
    model: GGUF_Model,
    /// 推理输出缓冲区（logits 或 hidden state）
    output: Option<Tensor>,
    /// 自增序列位置（forward offset=nil 时自动 += seq_len）
    offset: usize,
    /// 采样随机数生成器状态 (xoshiro)
    rng_state: u64,
    /// EOS token ID（从模型获取）
    eos_token_id: u32,
}
```

5 个字段。不设 input 缓冲区——Tensor 由调用方直接传入 `forward()`。

### 2.2 GGUF_Model — 模型抽象

完整的模型容器。详见 `gguf_model.rs`。

```rust
struct GGUF_Model {
    model: Model_Weights,           // 权重（embedding + layers + norm + lm_head）
    tokenizer: Option<Tokenizer>,   // 分词器
    inference_config: Inference_Config, // eos_token
    arch_info: Model_Arch_Info,     // 架构元信息
    device: Device,                 // 运行设备
    has_input_head: bool,           // 是否含 embedding 层
    has_output_head: bool,          // 是否含 lm_head 层
}
```

### 2.3 Model_Arch_Info — 架构元信息

GGUF 文件解析结果。详见 `gguf_model_manager.rs`。

```rust
struct Model_Arch_Info {
    architecture: String,       // "qwen3"
    num_layers: usize,          // transformer block 层数 (28)
    embedding_length: usize,    // 隐藏维度 (4096)
    head_count: usize,          // 注意力头数
    head_count_kv: usize,       // KV 注意力头数
    head_dim: usize,            // 每头维度
    context_length: usize,      // 最大上下文长度
    vocab_size: usize,          // 词表大小
    eos_token_id: u32,          // EOS token ID
    // + layers / non_layer_tensors / is_split ...
}
```

---

## 3. 公开方法定义

### 3.1 模型生命周期

#### `load_model(path, device, start, end) → MlContext`

从 GGUF 文件加载模型，按层范围选择性加载（支持分布式 Worker 只加载部分层）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `path` | `&Path` | GGUF/PGGUF 文件路径 |
| `device` | `&str` | "cpu" 或 "cuda" |
| `start` | `usize` | 起始层（0=embedding, 1..=N=block, N+1=output） |
| `end` | `usize` | 结束层 |

内部调用 `GGUF_Load_Model(start, end, path, device)` → 组装 `GGUF_Model` → 包装为 `MlContext`。

#### `unload_model(ctx: MlContext)`

Drop MlContext，释放模型权重、KV Cache、tokenizer。

---

### 3.2 编解码

#### `encode(ctx, text) → Vec<u32>`

文本 → token IDs。自动包装 Qwen3 对话模板。

```
"你好" → "<|im_start|>user\n你好<|im_end|>\n<|im_start|>assistant\n" → [151644, 872, 198, ...]
```

内部调用 `GGUF_Encode(&ctx.model, text)`。若模型不含 tokenizer，返回错误。

#### `decode(ctx, token_id) → String`

单个 token ID → 文本。自动跳过 EOS token。

内部调用 `GGUF_Decode(&ctx.model, &[token_id])`。

---

### 3.3 推理

#### `tensorize(ctx, token_ids) → Tensor`

纯数据转换：`Vec<u32>` → `Tensor`。不涉及模型计算。

```
[151644, 872, 198]  --Tensor::new + unsqueeze-->  [1, 3] u32 Tensor
```

仅做 shape 变换。Embedding 层查表在 `forward` 内部由模型自动完成。

#### `forward(ctx, tensor, offset)`

单次前向推理。输入 Tensor 送入模型，结果存入 `ctx.output`。

| 参数 | 类型 | 说明 |
|------|------|------|
| `tensor` | `&Tensor` | 输入（u32 [1, seq_len] 或 f32 [1, seq_len, hidden]） |
| `offset` | `Option<usize>` | 位置偏移。`None` 时使用内部自增 offset |

内部调用 `GGUF_Model_Inference(&mut ctx.model, tensor, offset)`。

- 有 embedding 层：自动查表 → transformer blocks → forward
- 无 embedding 层：直接 transformer blocks → forward
- `offset = None`：自动 `ctx.offset += seq_len`

---

### 3.4 采样

#### `sample(ctx, temperature) → u32`

从 `ctx.output`（logits）采样下一个 token。

| 参数 | 类型 | 说明 |
|------|------|------|
| `temperature` | `f64` | 0.0 = argmax，>0 = 温度缩放 + softmax 随机采样 |

内部流程：
1. 从 `ctx.output` 提取最后位置的 logits
2. `temperature ≤ 0`：argmax
3. `temperature > 0`：logits / T → softmax → 加权随机（xoshiro 生成器）
4. 返回 token ID

---

### 3.5 状态查询

#### `get_output_tensor(ctx) → &Tensor`

返回 `ctx.output` 的引用。供 TensorStream 发送到下游节点。

#### `get_eos(ctx) → u32`

返回 `ctx.eos_token_id`。

#### `get_offset(ctx) → usize`

返回 `ctx.offset`。

---

## 4. 三种推理模式调用序列

### 4.1 单机推理（Run）

```
ctx = ml:load_model(model, "cpu", 0, 999999)
token_ids = ml:encode(ctx, io:input())
ml:forward(ctx, ml:tensorize(ctx, token_ids), 0)

loop:
    tok = ml:sample(ctx, temperature)
    io:output(ml:decode(ctx, tok))
    if tok == ml:get_eos(ctx) → break
    ml:forward(ctx, ml:tensorize(ctx, {tok}), nil)
io:end_output()
```

### 4.2 协调者（Coordinator）

```
ctx = ml:load_model(model, device, coord_start, coord_end)
ml:forward(ctx, ml:tensorize(ctx, ml:encode(ctx, io:input())), 0)
ts:send(ml:get_output_tensor(ctx))

loop:
    t = ts:receive()
    ml:forward(ctx, t, nil)
    tok = ml:sample(ctx, temperature)
    io:output(ml:decode(ctx, tok))
    if tok == ml:get_eos(ctx) → break
    ml:forward(ctx, ml:tensorize(ctx, {tok}), nil)
    ts:send(ml:get_output_tensor(ctx))
ts:send_eof()
```

### 4.3 工作节点（Worker）

```
ctx = ml:load_model(model, device, layer_start, layer_end)

loop:
    is_eof, t, offset = ts:receive()
    if is_eof → break
    ml:forward(ctx, t, offset)
    ts:send(ml:get_output_tensor(ctx))
```

---

## 5. 底层函数

以下函数由 `gguf_model.rs` / `gguf_model_manager.rs` / `gguf_tensor.rs` 提供，不直接暴露给上层。

### 5.1 模型操作

| 函数 | 签名 | 说明 |
|------|------|------|
| `GGUF_Load_Model` | `(start, end, path, device) → GGUF_Model` | 按层范围加载模型 |
| `GGUF_Unload_Model` | `(GGUF_Model)` | 释放模型 |
| `GGUF_Model_Inference` | `(model, &Tensor, offset) → Tensor` | 单次前向推理 |
| `GGUF_Encode` | `(model, text) → Vec<u32>` | 文本编码 |
| `GGUF_Decode` | `(model, token_ids) → String` | Token 解码 |

### 5.2 模型管理

| 函数 | 签名 | 说明 |
|------|------|------|
| `GGUF_Analyze` | `(path) → Model_Arch_Info` | 解析模型结构 |
| `GGUF_Load_Layer` | `(path, index, device) → GGUF_Layer_Weights` | 加载单层权重 |
| `GGUF_Split_Model` | `(src, start, end, out_dir)` | 切分模型文件 |

### 5.3 张量传输

| 函数 | 签名 | 说明 |
|------|------|------|
| `GGUF_Tensor_Serialize` | `(packet) → Vec<u8>` | 序列化（TensorStream 用） |
| `GGUF_Tensor_Deserialize` | `(bytes) → GGUF_Tensor_Packet` | 反序列化 |

---

## 6. 模型权重结构

```
Model_Weights
├── embed_tokens: Option<Embedding>     输入 Embedding 层
├── layers: Vec<Layer_Weights>          N 层 Transformer Block
│   ├── ln1: RmsNorm                    Pre-Attention LayerNorm
│   ├── self_attn: Attention_Weights    Self-Attention
│   │   ├── q_proj / k_proj / v_proj / o_proj: QMatMul
│   │   ├── q_norm / k_norm: RmsNorm    QK Norm (Qwen3)
│   │   ├── rotary_emb: Rotary_Embedding RoPE 位置编码
│   │   └── kv_cache: ConcatKvCache     推理时累积 KV
│   ├── ln2: RmsNorm                    Pre-MLP LayerNorm
│   └── mlp: Mlp_Weights                FFN
│       ├── gate_proj / up_proj / down_proj: QMatMul
│       └── act_fn: Activation (Silu)
├── norm: Option<RmsNorm>              输出 LayerNorm
└── lm_head: Option<QMatMul>            输出投影 → logits
```

层编号规则：
- 0 = embedding 层
- 1..=N = transformer block 层
- N+1 = output 层（norm + lm_head）

---

## 7. 错误处理

所有公开方法返回 `Result<T, String>`。

```rust
// context.rs 中的典型错误处理
pub fn encode(&mut self, text: &str) -> Result<Vec<u32>, String> {
    let tokenizer = self.model.tokenizer.as_ref()
        .ok_or("Model has no tokenizer")?;
    // ...
}

pub fn forward(&mut self, tensor: &Tensor, offset: Option<usize>) -> Result<(), String> {
    let off = offset.unwrap_or(self.offset);
    let output = GGUF_Model_Inference(&mut self.model, tensor, off)
        .map_err(|e| format!("Forward failed: {e}"))?;
    // ...
}
```

不定义 `ML_Engine_Error` 枚举——String 错误信息足够上层理解。

---

## 8. 与旧架构的差异

| 维度 | 旧（ML_Engine_Capability trait） | 新（MlContext） |
|------|-------------------------------|----------------|
| 接口形式 | async trait, 5 方法 | 同步 struct, 7 方法 + 状态查询 |
| Session 管理 | Create_Session / Shutdown_Session（线程 + 通道） | 线程即 MlContext 生命周期 |
| 推理方式 | Run_Program_VM（批量执行 MlInstruction） | forward / sample 单步调用 |
| 错误类型 | ML_Engine_Error 枚举（5 变体） | String |
| 参数传递 | ML_Session_Config + Pipeline_Params | 函数参数 + Lua params |
| 结果返回 | Pipeline_Result 打包 | 流式 output + get_output_tensor |
| 控制流 | Jump/JumpIf 指令模拟 | Lua 原生控制流 |

---

## 9. 状态

### ✅ 已完成

- `context.rs` — `MlContext` 结构体 + 7 个公开方法 + 状态查询
- `capability.rs` — `analyze_model` / `split_model` 独立 async 函数
- 设计文档

### ⚠ 待处理

- `session.rs` / `service.rs` / `pipeline.rs` / `ML_VM/` / `Vm_Base/` 等旧架构代码保留，远期删除
- `ml:*` 函数表未注册到 Lua（`Src/Lua/capability_binding.rs` 待后续实现）
- 编译失败：其他模块仍在引用旧的 `ML_Engine_Capability` trait

### 🟡 Code Review 遗留（P2）

- **#4** `tensorize` 应拒绝空 `&[]` token_ids（当前静默创建 `[1,0]` Tensor）
- **#5** `sample` 后不清空 `output`（应加文档声明调用约定）
- **#6** 缺少 `clear_kv_cache()` 方法（新对话需重置 KV Cache 但不重载模型）
- **#7** `load_model` PRNG seed 硬编码为 `299792458`，应允许调用方传入
- **#8** `analyze_model` 返回裸 `Model_Arch_Info`，应考虑返回含 `layer_sizes_bytes` 的 `Model_Info`
- **#9** `MlContext` 缺少单元测试
