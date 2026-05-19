# ML Engine 设计文档

Presented by KeJi
Date ： 2026-05-17

## 1. 模块概述

`ML_Engine` 模块负责 Pleiades 分布式推理系统的**模型推理计算**。不负责网络传输（TensorStream）、不负责文本 IO（Session_Manager）、不负责控制流（Lua）。

### 核心定义

> **ML Engine = 纯推理计算层。** 通过 `MlSession` userdata 暴露给 Lua，类似 Python class。
> Lua 脚本实例化 session，调用对象方法，控制流完全在 Lua 侧。

### 对外形式：Lua UserData（非 trait）

```
其他模块:                            ML Engine:
┌────────────────────┐              ┌─────────────────────┐
│ Capability trait   │              │ MlSession userdata  │
│ Box<dyn Trait>     │              │ impl mlua::UserData │
│ async fn(&self)    │              │ fn(&mut self)       │
└────────────────────┘              └─────────────────────┘
        ↑                                    ↑
   &self 共享引用                     &mut self, 不适合 trait object
   Arc<RwLock<>> 分散状态             纯 owned 状态, Lua 直接持有
```

### 模块结构

```
ML_Engine/
├── context.rs           ← MlSession (UserData) + MlContext (内部状态) + 方法 impl
├── capability.rs        ← 独立操作：analyze_model / split_model
├── gguf_model.rs        ← 模型抽象：load / unload / inference / encode / decode
├── gguf_model_manager.rs← 底层 GGUF 操作：analyze / load_layer / split
├── gguf_tensor.rs       ← 张量序列化（网络传输用）
├── GGUF_Models/
│   ├── mod.rs
│   └── qwen3.rs         ← Qwen3 权重结构 + Forward
└── mod.rs               ← 模块入口
```

### 调用关系

```
Lua 脚本 (OS 线程)
   │
   ├── ml.new("cpu")                              → sess (空壳 userdata)
   ├── sess:load_model(path, 0, 999999)            → 填充模型
   ├── sess:encode(prompt)                       → token IDs
   ├── sess:tensorize(tokens)                    → Tensor
    ├── sess:forward(tensor, offset)               → Tensor (logits/hidden)
    ├── sess:sample(logits, temperature)           → 下一个 token
   ├── sess:decode(token_id)                      → 文本
   ├── sess:get_eos()                             → EOS token
   ├── sess:get_offset()                          → 位置偏移量
   ├── sess:has_model()                           → 模型是否已加载
   └── sess:unload()                              → 卸载模型，回到空壳
         │
         ▼
      MlSession (userdata 包装)
         │
         ▼
       MlContext (内部状态: model(Option) + device + offset + rng + eos)
         │
         ▼
     GGUF_Model → Model_Weights (embedding + layers + norm + lm_head)
```

### 状态机

```
          ml.new("cpu")
             │
             ▼
    ┌─────────────────┐
    │ model = None     │  ← 空壳
    │ device = Cpu     │
    │ eos = 151645     │
    └───────┬─────────┘
            │ sess:load_model(path, 0, 40)
            ▼
    ┌─────────────────┐
    │ model = Some(..) │  ← 已加载
    │ eos = model.eos  │
    └───────┬─────────┘
            │ sess:unload()
            ▼
    ┌─────────────────┐
    │ model = None     │  ← 回到空壳（可重新 load）
    │ eos = 151645     │
    └─────────────────┘
```

---

## 2. 数据结构

### 2.1 MlSession — Lua 可见句柄

```rust
/// 推理会话 userdata。Lua 侧通过 `sess:method()` 调用。
pub struct MlSession {
    ctx: MlContext,   // 内部状态（含状态机），Lua 不可见
}
```

MlSession 是状态机：空壳创建 → load_model → 推理 → unload → 空壳。通过 `impl mlua::UserData` 注册方法到 Lua。

### 2.2 MlContext — 内部推理状态（Lua 不可见）

```rust
struct MlContext {
    model: Option<GGUF_Model>,  // 模型权重 + tokenizer（None = 空壳）
    offset: usize,              // 自增序列位置
    rng_state: u64,             // xoshiro 随机数生成器状态
    eos_token_id: u32,          // EOS token ID
    device: Device,             // 运行设备
}
```

5 个字段。`model` 为 `Option`，空壳时为 `None`。`device` 在 `new()` 时确定，不随模型变化。推理输出由 `forward()` 直接返回，不存内部缓冲区。

### 2.3 GGUF_Model — 模型抽象

```rust
struct GGUF_Model {
    model: Model_Weights,                   // 权重
    tokenizer: Option<Tokenizer>,           // 分词器
    inference_config: Inference_Config,      // eos_token
    arch_info: Model_Arch_Info,             // 架构元信息
    device: Device,                          // 运行设备
    has_input_head: bool,                   // 是否含 embedding
    has_output_head: bool,                  // 是否含 lm_head
}
```

---

## 3. 公开方法定义

### 3.1 构造 / 析构

#### `ml.new(device) → MlSession`

创建空壳 MlSession，不加载模型。解析 device 字符串（"cpu"/"cuda"），初始化空 MlContext。

Lua 侧通过 `ml` 函数表调用：`local sess = ml.new("cpu")`。

空壳状态下可用：
- `tensorize()` — 纯数据转换，仅需 device
- `get_eos()` — 返回默认值 151645
- `get_offset()` — 返回 0
- `has_model()` — 返回 false

#### `sess:load_model(path, start, end) → ()`

实例方法。若已有模型则先卸载。内部调用 `GGUF_Load_Model`，将模型填入 `ctx.model`。

Lua 侧：`sess:load_model("model.gguf", 0, 40)`。

#### `sess:unload()`

实例方法。清空 `ctx.model`，释放模型资源，回到空壳状态。不消耗 self，可重复 `load_model`。

#### `sess:has_model() → bool`

检查模型是否已加载。

---

### 3.2 编解码 (Lua: `sess:method()`)

| Lua 调用 | Rust 签名 | 说明 |
|----------|----------|------|
| `sess:encode(text)` | `encode(&self, text: &str) → Vec<u32>` | 文本 → token IDs（含 Qwen3 模板） |
| `sess:decode(token_id)` | `decode(&self, token_id: u32) → String` | token ID → 文本 |

> 编解码方法需要模型已加载（含 tokenizer），空壳时返回错误 `"no model loaded"`。

---

### 3.3 推理

| Lua 调用 | Rust 签名 | 说明 |
|----------|----------|------|
| `sess:tensorize(token_ids)` | `tensorize(&self, &[u32]) → Tensor` | Vec<u32> → [1, seq_len] u32 Tensor，**空壳可用** |
| `sess:forward(tensor, offset)` | `forward(&mut self, &Tensor, Option<usize>) → Tensor` | 单次前向推理，返回 logits/hidden states |

`forward` 返回 Tensor（对标 PyTorch `output = model(input)`）。结果不存内部缓冲区，由调用方持有。有 embedding 层则自动 embedding → forward，无则直接 forward。`offset=None` 时自动递增。

> `tensorize` 是纯数据转换，不依赖模型，空壳状态可直接使用。

---

### 3.4 采样

| Lua 调用 | Rust 签名 | 说明 |
|----------|----------|------|
| `sess:sample(logits, temperature)` | `sample(&mut self, &Tensor, f64) → u32` | 从 logits tensor 采样下一个 token |

对标 PyTorch: `probs = softmax(logits[:,-1,:] / temp); tok = multinomial(probs, 1)`。

`temperature ≤ 0` 时 argmax，否则 softmax 随机采样 (xoshiro)。logits 由 `forward()` 返回值直接传入。

---

### 3.5 状态查询

| Lua 调用 | 返回 | 说明 |
|----------|------|------|
| `sess:get_eos()` | u32 | EOS token ID（空壳=151645，加载后=模型值） |
| `sess:get_offset()` | usize | 当前序列位置偏移量 |

---

## 4. UserData 注册 (Rust → Lua 桥接)

```rust
impl mlua::UserData for MlSession {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("load_model", |_, sess, (p,s,e)| sess.load_model(Path::new(&p), s, e));
        methods.add_method("has_model", |_, sess, ()| Ok(sess.has_model()));
        methods.add_method_mut("unload", |_, sess, ()| { sess.unload(); Ok(()) });
        methods.add_method("encode",     |_, sess, text| sess.encode(&text));
        methods.add_method("decode",     |_, sess, id| sess.decode(id));
        methods.add_method("tensorize",  |_, sess, ids| sess.tensorize(&ids));
        methods.add_method_mut("forward", |_, sess, (t, off)| { let o = sess.forward(&t, off)?; Ok(LuaTensor(o)) });
        methods.add_method_mut("sample", |_, sess, (logits, temp)| sess.sample(&logits, temp));
        methods.add_method("get_eos",    |_, sess, ()| Ok(sess.get_eos()));
        methods.add_method("get_offset", |_, sess, ()| Ok(sess.get_offset()));
    }
}
```

`load_model` 和 `unload` 使用 `add_method_mut`（需要 `&mut self`）。mlua 内部做借用检查，Lua 调用方无需感知。

---

## 5. 三种推理模式 (Lua 代码)

### 5.1 单机推理（Run）

```lua
local sess = ml.new("cpu")
sess:load_model("model.gguf", 0, 999999)
local tokens = sess:encode(io:input())
sess:forward(sess:tensorize(tokens), 0)

for i = 1, 120 do
    local tok = sess:sample(0.8)
    io:output(sess:decode(tok))
    if tok == sess:get_eos() then break end
    sess:forward(sess:tensorize({tok}), nil)
end
io:end_output()
sess:unload()
```

### 5.2 协调者（Coordinator）

```lua
local sess = ml.new(device)
sess:load_model(model, coord_start, coord_end)
sess:forward(sess:tensorize(sess:encode(io:input())), 0)
ts:send(sess:get_output_tensor(sess))

for i = 1, 120 do
    local t = ts:receive()
    sess:forward(t, nil)
    local tok = sess:sample(0.8)
    io:output(sess:decode(tok))
    if tok == sess:get_eos() then break end
    sess:forward(sess:tensorize({tok}), nil)
    ts:send(sess:get_output_tensor(sess))
end
ts:send_eof()
sess:unload()
```

### 5.3 工作节点（Worker）

```lua
local sess = ml.new(device)
sess:load_model(model, layer_start, layer_end)

while true do
    local is_eof, t, offset = ts:receive()
    if is_eof then break end
    sess:forward(t, offset)
    ts:send(sess:get_output_tensor(sess))
end
```

---

## 6. 底层函数

由 `gguf_model.rs` / `gguf_model_manager.rs` 提供，MlSession 内部调用。

| 函数 | 签名 | 用途 |
|------|------|------|
| `GGUF_Load_Model` | `(start, end, path, device) → GGUF_Model` | load_model 内部 |
| `GGUF_Unload_Model` | `(GGUF_Model)` | unload 内部 |
| `GGUF_Model_Inference` | `(model, &Tensor, offset) → Tensor` | forward 内部 |
| `GGUF_Encode` | `(model, text) → Vec<u32>` | encode 内部 |
| `GGUF_Decode` | `(model, token_ids) → String` | decode 内部 |
| `GGUF_Analyze` | `(path) → Model_Arch_Info` | analyze_model 内部 |
| `GGUF_Split_Model` | `(src, start, end, out_dir)` | split_model 内部 |

---

## 7. 为什么 ML Engine 不定义 trait

| | 其他 capability | ML Engine |
|---|---|---|
| 方法签名 | `&self`（共享引用） | `&mut self`（forward/sample/unload/load_model） |
| 状态方案 | `Arc<RwLock<>>` 内部锁 | 纯 owned 状态，Lua 直接持有 |
| trait object | `Box<dyn Trait>` 可用 | `&mut self` + trait object 冲突 |
| 解决方案 | trait | `impl mlua::UserData` 暴露给 Lua |

核心矛盾：`forward`、`sample`、`load_model`、`unload` 都需要 `&mut self`（状态会变），而 `Box<dyn Trait>` 不支持 `&mut self` 的 async trait。

---

## 8. 与旧架构对比

| 维度 | 旧（ML_Engine_Capability trait） | 新（MlSession UserData） |
|------|-------------------------------|-------------------------|
| 接口形式 | async trait, 5 方法, `Box<dyn Trait>` | 同步 struct, `impl mlua::UserData` |
| Session | Create_Session（线程+通道+注册表） | `ml.new()` 空壳 + `sess:load_model()` 填充 |
| 推理方式 | Run_Program_VM（批量执行指令序列） | `sess:forward()` / `sess:sample()` 单步 |
| 调用路径 | Orchestrator → trait → channel → OS thread | Lua 线程 → `sess:method()` 直接调用 |
| 抽象层数 | 4 层 | 1 层 |
| 调用风格 | Lua: `ml:function(sess, ...)` | Lua: `sess:method(...)` |
| 生命周期 | HashMap 注册 + shutdown 协议 | 空壳→load→推理→unload→空壳，Lua GC drop |

---

## 9. 状态

### ✅ 已完成

- `context.rs` — `MlSession` userdata + `MlContext` 内部状态 + `impl mlua::UserData`
- `capability.rs` — `analyze_model` / `split_model` 独立 async 函数
- MlSession 两步构造：`new(device)` 空壳 + `load_model(path, start, end)` 填充
- 旧代码已删除: `session.rs`, `service.rs`, `pipeline.rs`, `ML_VM/`, `Vm_Base/`

### ⚠ 待处理

- `forward` 方法中 Tensor 跨 Lua 边界传输待实现（当前占位 `mlua::Error::runtime`）
- Lua 侧 `ml` 函数表注册 + `sess` userdata 实例化待完成

### 🟡 Code Review 遗留 (P2)

- **#4** `tensorize` 已添加空 token_ids 检查 ✅
- **#5** `sample` 后不清空 output — 已加文档注释
- **#6** 缺少 `clear_kv_cache()` 方法 — `unload()` 已清空 output+offset
- **#7** `load_model` PRNG seed 硬编码
- **#8** `analyze_model` 应返回含 `layer_sizes_bytes` 的视图类型
- **#9** `MlSession` 单元测试：7 个，覆盖空壳 + 生命周期 ✅
