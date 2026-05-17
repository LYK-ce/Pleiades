# Code Review: Lua 集成 + MlSession 重构

> Presented by KeJi
> Date: 2026-05-17
> Branch: machine-learning-reforge
> Commit: a658d5a

---

## 审查范围

| 文件 | 操作 | 行数 |
|------|------|------|
| `Src/Lua/engine.rs` | 修改 (+3 tests) | 111 |
| `Src/Lua/capability_binding.rs` | **新建** | 371 |
| `Src/Lua/registry.rs` | 未修改 | — |
| `Src/Lua/mod.rs` | 修改 (+1 mod) | 10 |
| `Src/ML_Engine/context.rs` | 重写 | 423 |
| `Src/Orchestrator/core.rs` | 修改 (+ProgramRegistry) | 174 |
| `Src/Orchestrator/core/branch_user.rs` | 修改 (+execute_lua_script) | 145 |
| `Src/TUI/app.rs` | 修改 (+available_commands) | 364 |
| `Src/lib.rs` | 修改 (-vm_base) | 94 |
| `tests/t09_lua_integration.rs` | **新建** | 91 |
| `programs/pipeline.lua` | **新建** | 57 |
| `programs/run.lua` | **新建** | 25 |
| `programs/list.lua` | **新建** | 23 |

---

## 一、架构设计评审

### 1.1 MlSession 两步构造 ✅ EXCELLENT

```
new("cpu") → 空壳 → load_model() → 推理 → unload() → 空壳
```

**优点：**
- 将构造与加载解耦，空壳可独立测试
- 状态机清晰，unload 可重用
- `model: Option<GGUF_Model>` 语义正确
- `device` 独立于 model，tensorize 空壳可用

**建议：** 无。设计已成熟。

### 1.2 能力函数分层注册 ✅ GOOD

```
register_caps()        — 通用演示函数 (echo/add/ping/table_sum)
register_storage_caps() — Storage trait 桥接 (list/exists)
register_ml_caps()     — ML Engine 桥接 (ml.new)
```

**优点：** 分层清晰，每个注册函数独立可测。

---

## 二、问题清单

### 🔴 P0 — 需要修复

#### P0-1: `register_storage_caps` 覆盖 `caps` 表

**文件:** `Src/Lua/capability_binding.rs:73`

```rust
let caps: mlua::Table = lua.globals().get("caps")
    .unwrap_or_else(|_| lua.create_table().unwrap());
```

当先调 `register_caps` 再调 `register_storage_caps` 时，后者取到已有 `caps` 表追加函数——这是正确的。但最后一行：

```rust
lua.globals().set("caps", caps)?;  // 重复 set，无副作用但多余
```

**影响:** 低。仅多余调用，不丢函数。

**修复:** 检查 caps 是否已存在于 globals，避免重复 set。或统一约定：各注册函数只往 caps 追加，由调用方决定何时 set globals。

---

#### P0-2: `execute_lua_script` 的 `_caps` 参数未使用

**文件:** `Src/Orchestrator/core/branch_user.rs:116`

```rust
async fn execute_lua_script(
    path: &std::path::Path,
    params: &std::collections::HashMap<String, String>,
    _caps: &std::sync::Arc<crate::orchestrator::Capabilities>,  // ← 未使用
) -> Result<mlua::Value, String> {
```

Core 传入了 `Capabilities` 但 `execute_lua_script` 只调了 `register_caps`（演示函数），没有调 `register_storage_caps` 或 `register_ml_caps`。

**影响:** 高。生产路径的 Lua 脚本无法访问真实的 Storage/Session/Network 能力，只能访问 echo/add/ping/table_sum。

**修复:**
```rust
// Phase 2 待实现：注册真实能力函数
// register_storage_caps(&lua, caps.storage.clone(), handle)?;
// register_ml_caps(&lua)?;
```

至少加 TODO 注释标明当前仅为演示路径。

---

### 🟡 P1 — 应该修复

#### P1-1: `unload()` 不重置 `rng_state`

**文件:** `Src/ML_Engine/context.rs:143`

```rust
pub fn unload(&mut self) {
    if let Some(model) = self.ctx.model.take() {
        GGUF_Unload_Model(model);
    }
    self.ctx.output = None;
    self.ctx.offset = 0;
    self.ctx.eos_token_id = 151645;
    // rng_state 未重置！
}
```

`load_model` 也不重置 `rng_state`。这导致 repeated load→unload→load 循环时，采样随机状态跨模型延续。

**影响:** 低（当前无热加载场景）。但在同一 session 内换模型时可能产生确定性差异。

**修复:** `unload()` 中将 `rng_state` 重置为初始 seed，与 `new()` 一致。

---

#### P1-2: `sample()` 未检查模型是否加载

**文件:** `Src/ML_Engine/context.rs:196`

```rust
pub fn sample(&mut self, temperature: f64) -> Result<u32, String> {
    let logits = self.ctx.output.as_ref()
        .ok_or("No output tensor. Call forward() first.")?;
    // ...
}
```

`encode`/`decode`/`forward` 都检查了 `model.is_some()`，但 `sample` 没有。理论上无模型时 forward 不会产生 output，但防御性编程应统一。

**影响:** 低。当前无法绕过 forward 直接调 sample（output 为 None），但如果未来加入了 `set_output_tensor` 之类的方法，可能绕过。

**修复:** 统一加 `model.as_ref().ok_or("no model")?` 或依赖 `output.as_ref()` 的守卫即可（当前已足够）。

---

#### P1-3: ML Engine Lua 绑定中 `tensorize` 返回 shape 而非 Tensor 值

**文件:** `Src/ML_Engine/context.rs:321`

```rust
methods.add_method("tensorize", |lua, sess, token_ids: Vec<u32>| {
    let t = sess.tensorize(&token_ids)?;
    let dims = t.dims();
    // 返回 {dim0, dim1} shape，而非 Tensor 数据
});
```

这是有意的设计权衡——Tensor 不能穿 FFI。但设计文档 `ml_engine_design.md` §3.3 中仍标为 "Vec<u32> → Tensor"，未反映返回值已变。

**影响:** 低。测试已覆盖 shape 返回。仅文档不一致。

**修复:** 更新设计文档，注明 Lua 侧 `tensorize()` 返回 `{batch, seq_len}` 的 shape table。

---

#### P1-4: `execute_lua_script` 使用 `call_async` 但 Lua 非 Send

**文件:** `Src/Orchestrator/core/branch_user.rs:143`

```rust
execute.call_async::<mlua::Value>(params_table).await
```

`mlua::Function::call_async` 会在当前线程上 poll future。由于 Core 的 `route_user` 是 `async fn`，且我们在 `tokio::select!` 中直接 `.await`（未 spawn），所以能正常工作。但如果未来有人将这段代码改为 `tokio::spawn`（就像初版那样），会编译失败。

**影响:** 中。当前正确，但脆弱。

**修复:** 在函数文档中加 `// SAFETY: Lua is !Send, must not be spawned across threads` 注释。

---

### 🟢 P2 — 建议优化

#### P2-1: 默认 EOS token (151645) 硬编码两处

**文件:** `context.rs:99` 和 `context.rs:147`

```rust
eos_token_id: 151645, // Qwen3 默认 EOS  (L99, L147)
```

**建议:** 提取为常量 `const QWEN3_DEFAULT_EOS: u32 = 151645;`。

---

#### P2-2: `register_caps` 中 ping 是 async 但无需 async

**文件:** `capability_binding.rs:43`

```rust
caps.set("ping", lua.create_async_function(|_, (): ()| async move {
    Ok::<_, mlua::Error>("pong".to_string())
})?)?;
```

ping 不执行任何 async 操作，可改为 `create_function`。保留 async 版本作为 `create_async_function` 的示例也合理。

**建议:** 保留（教学目的），但加注释说明。

---

#### P2-3: Storage test 中的 `Value::to_string()` 可能 panic

**文件:** `capability_binding.rs:247`

```rust
let result: mlua::Value = lua.load(...).eval()?;
let result_str = result.to_string().expect("to_string");
```

`mlua::Value::to_string()` 对某些类型（如 Function、Thread）返回 `None`。当前脚本返回字符串拼接结果，不会触发，但不够安全。

**建议:** 直接 `eval::<String>()` 替代 `eval::<Value>()` + `to_string()`。

---

#### P2-4: `t09_lua_integration` 中 `test_async_caps_from_script` 可能非确定性

**文件:** `tests/t09_lua_integration.rs:83`

```rust
#[tokio::test]
async fn test_async_caps_from_script() {
    let lua = LuaContext::new()?;
    register_caps(&lua)?;
    // ...
    let result: String = execute.call_async(()).await?;
}
```

`mlua` 内部使用独立异步运行时。在 `#[tokio::test]` 多线程 runtime 下，Lua（非 Send）跨线程行为取决于 mlua 的 `send` feature（当前未启用）。

**现状:** 测试通过（当前），但依赖 mlua 的实现细节。

**建议:** 文档记录此测试对 mlua `send` feature 的依赖状态。

---

#### P2-5: 废代码残留

| 位置 | 内容 |
|------|------|
| `core.rs:170-172` | `JOB_ID_COUNTER` + `generate_id()` 从未使用 |
| `core.rs:40-43` | `JobHandle.kind` + `inference_id` 从未读取 |
| `core.rs:77` | `lifecycle_tx` 从未读取 |
| `branch_user.rs:37-108` | `Run`/`DistributeModel`/`Send`/`Profile` 变体仍为 `todo!()` |

**建议:** 后续 Phase 清理。当前不影响功能。

---

## 三、测试覆盖评审

| 层级 | 测试数 | 覆盖内容 |
|------|--------|---------|
| engine.rs | 6 | 沙箱创建、危险 API 禁用、hello.lua 加载执行 |
| capability_binding.rs | 11 | sync/async 函数注册、table 传参、Storage 桥接、ML 空壳 |
| context.rs | 7 | 空壳构造、tensorize、无模型错误、unload 幂等 |
| t09_lua_integration | 4 | 磁盘脚本加载、caps 组合调用、async caps |
| **合计** | **28** | |

### 覆盖率评估

| 模块 | 覆盖 | 缺失 |
|------|------|------|
| Lua 沙箱 | ✅ 充分 | — |
| 能力注册 | ✅ 充分 | register_storage_caps 错误路径 |
| MlSession 空壳 | ✅ 充分 | load_model 成功路径（需要 GGUF 文件） |
| MlSession 加载态 | ❌ 未覆盖 | forward → sample → decode 完整流程 |
| Core 路由 | ❌ 未覆盖 | Execute 分支的集成测试 |
| TUI 命令补全 | ❌ 未覆盖 | — |

**关键缺失:** MlSession 加载态（forward/sample/decode）因需要真实 GGUF 文件无法在 CI 中测试。建议后续创建一个小型 dummy GGUF 文件（最小 Qwen3 配置）用于集成测试。

---

## 四、安全性评审

### 4.1 Lua 沙箱 ✅

```
✅ os → Nil         ✅ io → Nil
✅ require → Nil    ✅ dofile → Nil
✅ loadfile → Nil
```

5 个危险 API 全部禁用。沙箱内仅保留 `string`/`table`/`math` 标准库。

### 4.2 路径安全 ✅

`StorageManager::Validate_File_Id` 拒绝 `/`、`\\`、`..`、`.` 前缀。Lua 侧通过 `caps.storage_exists(file_id)` 调用时，Storage 仍会验证 file_id。

### 4.3 脚本隔离 ⚠️

当前每个 `execute_lua_script` 调用创建**独立** `Lua` 实例，执行完后销毁。脚本间无状态共享。但同一 Core 事件循环内可以顺序执行多个脚本，每个脚本的 `Lua` 实例独立。

**建议:** 未来如需预编译缓存，加 `HashMap<String, Vec<u8>>` 存储预编译字节码。

---

## 五、性能评审

### 5.1 Lua 实例创建开销

每次 Execute 命令都 `Lua::new()` + `register_caps()` + `load(eval)`. mlua `vendored` 模式下创建 Lua 实例约 1ms，对交互式 CLI/TUI 可接受。

### 5.2 `register_storage_caps` 中的 `block_on`

```rust
let entries = h.block_on(s.list())?;
```

在 Lua 的同步闭包内使用 `Handle::block_on` 执行异步 Storage API。单次调用阻塞当前线程但时间极短（HashMap 读锁 + 迭代器收集），可接受。

**注意:** 未来如果 `s.list()` 涉及 I/O（如 flush 触发磁盘扫描），会阻塞 Lua 执行线程。应考虑改为仅缓存数据，异步刷新。

---

## 六、总结

| 等级 | 数量 | 说明 |
|------|------|------|
| 🔴 P0 | 2 | register_storage_caps 覆盖 caps、execute_lua_script 未使用 Caps |
| 🟡 P1 | 4 | unload 不重置 rng、sample 缺 model 检查、tensorize 文档不一致、call_async 脆弱性 |
| 🟢 P2 | 5 | 硬编码 EOS、ping 无需 async、Value::to_string、async 测试非确定性、废代码 |

**整体评分: 7.5/10**

架构设计优秀（MlSession 状态机、能力函数分层注册），测试覆盖扎实（28 测试）。主要问题在生产路径 `execute_lua_script` 未接入真实 Capabilities（P0-2），以及少量防御性编程遗漏（P1-1、P1-2）。
