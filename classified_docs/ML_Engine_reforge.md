# ML Engine Reforge

> Presented by KeJi
> Date: 2026-05-16

## 0. 核心定义

> **ML Engine = 纯推理计算层。** 只负责模型加载、推理计算、编解码、采样。
> 不负责网络传输（TensorStream 的事）、不负责文本 IO（Session_Manager 的事）、
> 不负责控制流（Lua 的事）。对外暴露一组原子函数，由 Lua 脚本编排。

---

## 1. 职责边界

```
┌──────────────────────────────────────────────────────────┐
│                     Lua 脚本 (programs/*.lua)            │
│  控制流: if / while / for / 变量 / 函数                   │
│                                                          │
│  ts:send() / ts:receive()  ← TensorStream 提供           │
│  io:input() / io:output()  ← Session_Manager 提供        │
│  ml:load_model / encode / prefill / inference / sample   │
│                              ← ML Engine 提供             │
└──────────────────────────────────────────────────────────┘
```

| 职责 | 负责方 | 说明 |
|------|--------|------|
| 模型加载/卸载 | ML Engine | GGUF 文件读取、层范围选择、设备放置 |
| Token 编解码 | ML Engine | Tokenize / Detokenize（含 Qwen3 对话模板） |
| 前向推理 | ML Engine | Prefill（批量）、Decode（单 token）、外部 Tensor 推理 |
| 采样 | ML Engine | Temperature / Argmax 采样 |
| 状态查询 | ML Engine | EOS token、当前 offset、输出 Tensor 读取/写入 |
| 文本输入/输出 | Session_Manager | IO 通道阻塞读写、流式推送 |
| 张量网络传输 | TensorStream | Send / Receive / SendEOF |
| 控制流 | Lua | 循环、条件、变量管理 |

---

## 2. 对外方法定义

### 2.1 模型生命周期

#### `load_model(path, device, start, end) → session`

从 GGUF 文件加载模型，支持按层范围加载（分布式 Worker 只加载部分层）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `path` | String | GGUF/PGGUF 文件路径 |
| `device` | String | 推理设备："cpu" 或 "cuda" |
| `start` | usize | 起始层索引（0=embedding, 1..=N=transformer block, N+1=output） |
| `end` | usize | 结束层索引 |

| 返回 | 类型 | 说明 |
|------|------|------|
| `session` | 句柄 | 绑定到当前 Lua 线程的模型会话，后续所有 ml 方法需要传入 |

内部流程：
1. 调用 `GGUF_Analyze` 解析模型架构
2. 校验层范围有效性
3. 构建 `RotaryEmbedding`
4. 按需加载 embedding（start==0）、transformer blocks（start..end 相交部分）、output head（end==N+1）
5. 加载 tokenizer（如果文件包含）
6. 组装 `Model_Weights` → `GGUF_Model`
7. 初始化内部推理状态（KV Cache 为空）
8. 返回 session 句柄

#### `unload_model(session)`

卸载模型并释放所有资源（模型权重、KV Cache、tokenizer）。

---

### 2.2 编解码

#### `encode(session, text) → Vec<u32>`

将原始输入文本编码为 token ID 序列。

| 参数 | 类型 | 说明 |
|------|------|------|
| `session` | 句柄 | 模型会话 |
| `text` | String | 用户输入的原始文本 |

| 返回 | 类型 | 说明 |
|------|------|------|
| token_ids | `Vec<u32>` | 编码后的 token ID 列表 |

内部流程：
1. 获取 session 的 tokenizer
2. 包装 Qwen3 对话模板：`<|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n`
3. 调用 tokenizer.encode(text, add_bos=true) → token IDs
4. 返回 token ID 序列

> 错误：若 session 模型不含 tokenizer（Worker 分片模型），返回错误。

#### `decode(session, token_id) → String`

将单个 token ID 解码为文本字符串。

| 参数 | 类型 | 说明 |
|------|------|------|
| `session` | 句柄 | 模型会话 |
| `token_id` | u32 | 需要解码的 token ID |

| 返回 | 类型 | 说明 |
|------|------|------|
| text | String | 解码后的文本（单个 token 对应的字符串） |

内部流程：
1. 获取 session 的 tokenizer
2. 若 token_id == eos_token_id，返回空字符串或 EOS 标记
3. 调用 tokenizer.decode([token_id], skip_eos=true) → String
4. 返回文本

---

### 2.3 推理

session 内部持有两个 Tensor 缓冲区（对外不可见）：

```
ctx.input: Tensor    ← 推理输入
ctx.output: Tensor   ← 推理输出（logits 或 hidden state），供 sample() 和 get_output_tensor() 使用
```

底层统一调用 `GGUF_Model_Inference(model, input, offset)`。不区分 prefill / decode / tensor 三条路径，统一为一个 `forward`。

#### `forward(session, tensor, offset)`

单次前向推理。将输入 Tensor 送入模型，结果存入内部 `ctx.output`。

| 参数 | 类型 | 说明 |
|------|------|------|
| `session` | 句柄 | 模型会话 |
| `tensor` | Tensor | 输入 Tensor。通过 `tensorize()` 构造（[1, seq_len] u32），或从 TensorStream 接收（[1, seq_len, hidden_dim] f32） |
| `offset` | usize \| nil | 位置偏移。传 nil 时使用内部自增 offset |

内部流程：
1. 若 offset 为 nil，使用内部 `ctx.offset`
2. 调用 `GGUF_Model_Inference(model, tensor, offset=offset)`
   - 模型内部根据 `has_input_head` 自动判断：有 embedding 层则走 embedding → transformer，无则直接 transformer
3. 结果存入 `ctx.output`
4. 若 offset 为 nil：`ctx.offset += tensor.dim(1)`（自动递增 seq_len）

> 同一个方法覆盖所有推理场景。Coordinator/单机传入 tensorize 后的 u32 Tensor，Worker 传入网络收到的 hidden state Tensor。

#### `tensorize(session, token_ids) → Tensor`

纯数据转换：token IDs → Tensor。不涉及模型计算。

| 参数 | 类型 | 说明 |
|------|------|------|
| `session` | 句柄 | 模型会话 |
| `token_ids` | `Vec<u32>` | token ID 序列 |

| 返回 | 类型 | 说明 |
|------|------|------|
| tensor | Tensor | shape `[1, seq_len]`，dtype u32 |

内部流程：
1. `Tensor::new(&token_ids, &device)?.unsqueeze(0)?` → `[1, seq_len]` u32
2. 返回 Tensor

> 仅做 shape 变换，不查 embedding 表。Embedding 层的查表在 `forward` 内部由模型自动完成。

---

### 2.4 采样

#### `sample(session, temperature) → u32`

从内部 `ctx.output`（logits）中采样下一个 token。

| 参数 | 类型 | 说明 |
|------|------|------|
| `session` | 句柄 | 模型会话 |
| `temperature` | f64 | 采样温度（0.0 = argmax，>0 = 温度缩放 + softmax 采样） |

| 返回 | 类型 | 说明 |
|------|------|------|
| next_token | u32 | 采样出的下一个 token ID |

内部流程：
1. 从 `ctx.output` 提取最后一个位置的 logits
2. 若 temperature ≤ 0：argmax 取最大值索引
3. 若 temperature > 0：logits / temperature → softmax → 加权随机采样
   - 使用 session 内部的 xoshiro 随机数生成器（seed 从 Lua params 传入）
4. 返回 token ID

> 不维护 token 序列。Lua 脚本自己管理 generated_tokens 列表。

---

### 2.5 状态查询

#### `get_output_tensor(session) → Tensor`

获取内部 `ctx.output`，供 TensorStream 发送到下游节点。

| 返回 | 类型 | 说明 |
|------|------|------|
| tensor | Tensor | 当前推理输出（logits 或 hidden state） |

> Coordinator 模式：prefill/inference 后将 output tensor 通过 ts:send() 发给 Worker。
> 注意：返回的是引用或 clone，不消耗内部状态。

#### `set_input_tensor(session, tensor)`

设置内部 `ctx.input`，供 inference_tensor() 使用。由 Lua 在 ts:receive() 收到张量后调用。

| 参数 | 类型 | 说明 |
|------|------|------|
| `tensor` | Tensor | 从上游节点接收的 hidden state |

> Worker 模式：receive → set_input_tensor → inference_tensor → get_output_tensor → send

#### `get_eos(session) → u32`

获取模型 EOS token ID。

#### `get_offset(session) → usize`

获取当前序列位置偏移量。

---

## 3. 内部状态定义

```rust
/// Lua 线程绑定的推理上下文（外部不可见）
struct MlContext {
    /// 模型权重 + tokenizer
    model: GGUF_Model,
    /// 推理输出缓冲区（logits 或 hidden state）
    output: Option<Tensor>,
    /// 自增序列位置（forward offset=nil 时自动 += seq_len）
    offset: usize,
    /// 采样随机数生成器状态
    rng_state: u64,
    /// EOS token ID（从模型获取）
    eos_token_id: u32,
}
```

- `output` 是内部缓冲区，供 `sample()` 和 `get_output_tensor()` 读取
- `offset` 由 forward 自动维护（当 offset 参数为 nil 时）
- 不需要 `input` 缓冲区 — Tensor 由调用方直接传给 `forward()`

---

## 4. 三种推理模式的 Lua 调用序列

### 4.1 单机推理（Run）

```
ml:load_model(model_path, device, 0, 999999)    → sess
io:input()                                        → prompt
ml:forward(sess, ml:tensorize(sess, ml:encode(sess, prompt)), 0)

for i = 1, max_tokens do
    local tok = ml:sample(sess, temperature)
    io:output(ml:decode(sess, tok))
    if tok == ml:get_eos(sess) then break end
    ml:forward(sess, ml:tensorize(sess, {tok}), nil)
end
io:end_output()
```

### 4.2 协调者（Coordinator）

```
ml:load_model(model_path, device, coord_start, coord_end) → sess
io:input()                                                 → prompt
ml:forward(sess, ml:tensorize(sess, ml:encode(sess, prompt)), 0)
ts:send(ml:get_output_tensor(sess))   -- hidden state → Worker1

for i = 1, max_tokens do
    local t = ts:receive()            -- 从最后一个 Worker 收 logits
    ml:forward(sess, t, nil)          -- nil = 内部自增 offset
    local tok = ml:sample(sess, temperature)
    io:output(ml:decode(sess, tok))
    if tok == ml:get_eos(sess) then break end
    ml:forward(sess, ml:tensorize(sess, {tok}), nil)
    ts:send(ml:get_output_tensor(sess))
end
ts:send_eof()
io:end_output()
```

### 4.3 工作节点（Worker）

```
ml:load_model(model_path, device, layer_start, layer_end) → sess

while true do
    local is_eof, t, offset = ts:receive()
    if is_eof then break end
    ml:forward(sess, t, offset)
    ts:send(ml:get_output_tensor(sess))
end
```

---

## 5. 已删除 / 待删除的代码

### ✅ 已删除（ML Engine 内部）

| 删除项 | 行数 | 原因 |
|--------|------|------|
| `ML_VM/` 全部（engine.rs + instruction.rs + slots.rs + mod.rs） | ~930 | Lua 替代指令执行 |
| `session.rs` Session/Handle/Command/Config | ~298 | 线程即 Session |
| `service.rs` ML_Engine_Service | ~270 | 简化为 MlContext |
| `pipeline.rs` Pipeline_Params + Pipeline_Result + Model_Info | ~77 | 参数走 Lua params，结果流式输出 |
| `capability.rs` ML_Engine_Capability trait + ML_Engine_Error + ML_Session_Config | ~180 | 替换为 MlContext + 独立函数 |
| **合计（ML Engine）** | **~1755** | |

### ⚠ 待删除（其他模块）
| `Orchestrator_VM/` instruction.rs + engine.rs + slots.rs + handler_*.rs | ~650 | Lua 替代编排 |
| `program_selector.rs` | ~920 | Lua 替代 TOML |
| `branch_user.rs` 简化 | ~250 | 统一 Execute 分支 |
| TOML 模板 10个 | ~300 | 替换为 .lua |
| **合计** | **~4655** | |

---

## 6. 保留的代码

| 文件 | 行数 | 保留原因 |
|------|------|------|
| `gguf_model.rs` GGUF_Load_Model / Unload / Inference / Encode / Decode | ~375 | 核心推理函数 |
| `gguf_model_manager.rs` GGUF_Analyze / Load_Layer / Split_Model | ~650 | 模型分析/加载/切分 |
| `gguf_tensor.rs` GGUF_Tensor_Packet / Serialize / Deserialize | ~253 | 张量序列化（TensorStream 用） |
| `GGUF_Models/qwen3.rs` 全部 | ~653 | Qwen3 权重结构 + Forward |
| `ML_Engine/mod.rs` | ~49 | 模块入口 |
| 其他所有模块 | — | **本次不修改** |

---

## 7. 实施计划

### 本次 Reforge（仅改 ML_Engine 内部文件，不动其他模块）

1. ✅ 编写 `docs/ml_engine_design.md` — ML Engine 设计文档
2. ✅ 新建 `Src/ML_Engine/context.rs` — `MlContext` 结构体 + 7 个公开方法 + 状态查询
3. ✅ 重写 `Src/ML_Engine/capability.rs` — 删除 trait/Error/Config，保留 `analyze_model` / `split_model` 两个独立 async 函数
4. ✅ 不修改 `mod.rs`、`service.rs`、`session.rs`、`pipeline.rs`
5. ✅ 不修改 Lua 模块（不做绑定操作）
6. ⚠ 编译失败（其他模块仍引用旧 `ML_Engine_Capability` trait）
