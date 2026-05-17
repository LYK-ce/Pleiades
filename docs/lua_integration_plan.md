# Lua 集成方案

> Presented by KeJi
> Date: 2026-05-17
> Branch: alpha

---

## 1. 目标

将 Lua (mlua) 集成为 Pleiades 的策略脚本引擎，替代已删除的 VM 体系（Vm_Base / ML_VM / Orchestrator_VM / TOML 模板）。

**原则：渐进式集成。** 每一步都有可验证的测试，不追求一步到位。

---

## 2. 当前现状

| 组件 | 状态 |
|------|------|
| mlua 依赖 (Cargo.toml) | ✅ `0.12.0-rc.1`, features: lua54+vendored+async |
| `Src/Lua/engine.rs` | ✅ `LuaContext::new()` 沙箱, 2 tests |
| `Src/Lua/registry.rs` | ✅ `ProgramRegistry::scan()`, 1 test |
| `programs/hello.lua` | ✅ 演示脚本 (COMMAND + DESCRIPTION + execute) |
| `Src/Lua/capability_binding.rs` | ❌ 不存在 — Rust→Lua 函数桥接缺失 |
| 业务 Lua 脚本 | ❌ pipeline.lua / run.lua / profile.lua 均未编写 |
| Core 路由简化 | ❌ branch_user.rs 仍为硬编码分支 |

**关键缺失：** 可以创建 Lua 沙箱、扫描脚本，但无法将 Rust 函数注入 Lua 环境，也无法在测试中加载并执行 Lua 脚本中的 `execute()` 函数。

---

## 3. 集成路线

### Phase 0 — 设计文档
> **目标：产出 `docs/lua_design.md`，明确模块边界、数据结构、接口约定。**

在编码之前完成设计文档，确保模块间接口清晰。

#### 产出文件

`docs/lua_design.md` — Lua 模块设计文档，格式参考 `docs/storage_design.md` 和 `docs/ml_engine_design.md`。

#### 文档内容

| 章节 | 内容 |
|------|------|
| 模块概述 | Lua 模块定位、核心定义、模块结构、调用关系 |
| 模块边界 | 与 ML_Engine / Network / Session_Manager 的职责划分 |
| 数据结构 | LuaContext, ProgramEntry, ProgramRegistry |
| 脚本格式 | COMMAND/DESCRIPTION/execute 约定 |
| 沙箱安全模型 | 禁用 API 列表、白名单、隔离策略 |
| 能力函数桥接 | create_function vs create_async_function、ABCD 四级能力函数 |
| 数据转换 | Lua Table ↔ Rust struct 双向转换规范 |
| 执行流程 | Core → Lua 调用入口、脚本生命周期 |
| MlSession 集成 | UserData 对象模型调用示例 |
| 错误处理 | 各类错误的捕获和传播路径 |

#### 验收标准

设计文档完成，与现有代码一致。

---

### Phase 1 — 最小验证（本次）
> **目标：在 Rust 测试中加载并执行 Lua 脚本的 `execute()` 函数。**

仅依赖当前已有的 `LuaContext` 和 `ProgramRegistry`，不新建文件、不注册能力函数。

#### 3.1.1 新增测试

**位置：** `Src/Lua/engine.rs`（追加测试）

**测试用例：**

| # | 测试名 | 内容 |
|---|--------|------|
| 0.1 | `test_load_and_execute_hello_lua` | 从 `programs/hello.lua` 加载脚本，调用 `execute({msg="test"})`，验证返回 `"ok"` |
| 0.2 | `test_script_missing_execute_errors` | 加载不含 `execute` 函数的脚本，验证获取该函数时返回错误 |
| 0.3 | `test_sandbox_cannot_read_files` | 脚本中尝试 `io.open`，验证被沙箱阻止 |

#### 3.1.2 验证方式

```rust
#[test]
fn test_load_and_execute_hello_lua() {
    let lua = LuaContext::new().expect("create lua");
    let script = std::fs::read_to_string("programs/hello.lua").expect("read script");

    // 1. 加载脚本，执行顶层（注册 COMMAND/DESCRIPTION/execute）
    lua.load(&script).eval::<()>().expect("eval script");

    // 2. 读取元数据
    let command: String = lua.globals().get("COMMAND").expect("COMMAND");
    assert_eq!(command, "hello");

    // 3. 构造参数 table
    let params = lua.create_table().expect("params table");
    params.set("msg", "integration test").expect("set msg");

    // 4. 调用 execute(params)
    let execute: mlua::Function = lua.globals().get("execute").expect("execute fn");
    let result: String = execute.call(params).expect("call execute");
    assert_eq!(result, "ok");
}
```

#### 3.1.3 验收标准

```
cargo test --lib lua
```
输出所有 Lua 测试通过（从 3 个扩展至 6 个）。

---

### Phase 2 — 能力函数桥接层
> **目标：在测试中将一个 Rust 函数注册到 Lua，Lua 脚本调用该函数并获得返回值。**

#### 3.1.1 新建文件

`Src/Lua/capability_binding.rs`

#### 3.1.2 实现内容

1. **`register_test_functions(lua: &Lua)`** — 向 Lua 的 `caps` 表注册一个同步函数（例如 `caps:echo(msg)` → 返回 msg）。
2. **`register_async_function(lua: &Lua, caps: &Capabilities)`** — 向 Lua 的 `caps` 表注册一个异步函数（例如 `caps:list_files()` → 调用 `storage.list()` 返回文件列表）。

#### 3.1.3 测试用例

| # | 测试名 | 内容 |
|---|--------|------|
| 1.1 | `test_sync_function_binding` | 注册 `echo` 到 Lua，Lua 调用 `caps:echo("hi")` 返回 `"hi"` |
| 1.2 | `test_async_function_binding` | 注册 `ping`（返回 `"pong"`），Lua 通过 `caps:ping()` 获取 |
| 1.3 | `test_lua_calls_rust_with_table` | Lua 构造 `{a=1, b=2}` 传入 Rust，Rust 返回 `a+b` |

#### 3.1.4 验收标准

```
cargo test --lib lua::capability_binding
```
新增 3 个测试通过。

---

### Phase 3 — 脚本端到端集成测试
> **目标：`programs/hello.lua` 调用一个注册的 Rust 函数，在集成测试中验证。**

#### 3.2.1 新建文件

`tests/t09_lua_integration.rs`

#### 3.2.2 测试内容

```rust
#[tokio::test]
async fn test_hello_script_calls_caps() {
    let lua = LuaContext::new().unwrap();
    let script = std::fs::read_to_string("programs/hello.lua").unwrap();
    lua.load(&script).eval::<()>().unwrap();

    // 注册能力函数
    let caps = lua.create_table().unwrap();
    caps.set("echo", lua.create_function(|_, msg: String| Ok(msg)).unwrap()).unwrap();
    lua.globals().set("caps", caps).unwrap();

    // 调用 execute
    let execute: mlua::Function = lua.globals().get("execute").unwrap();
    let result: String = execute.call(lua.create_table().unwrap()).unwrap();
    assert_eq!(result, "ok");
}
```

#### 3.2.3 验收标准

```
cargo test --test t09_lua_integration
```
集成测试通过。

---

### Phase 4 — 首个业务脚本：pipeline.lua
> **目标：用 Lua 脚本完整表达分布式 Pipeline 推理流程。**

#### 3.3.1 新建文件

`programs/pipeline.lua`

#### 3.3.2 脚本结构

```lua
COMMAND = "pipeline"
DESCRIPTION = "分布式流水线推理"

function execute(params, caps)
    -- Phase 1: 模型分析
    local arch = caps:analyze_model(params.model)

    -- Phase 2: 节点发现 + 调度
    local peers = caps:get_available_peers()
    local plan
    if params.strategy == "weighted" then
        plan = caps:plan_weighted(arch, peers)
    else
        plan = caps:plan_uniform(arch, peers)
    end

    -- Phase 3: 分发模型分片
    for _, w in ipairs(plan.workers) do
        caps:send_file(w.peer_id,
            caps:split_model(params.model, w.layer_start, w.layer_end))
    end

    -- Phase 4: 建立张量流
    caps:establish_streams(plan)

    -- Phase 5: Coordinator 推理
    local sess = caps:create_session(params.model, params.device,
        plan.coord_layer_start, plan.coord_layer_end)
    local io = caps:allocate_io()

    -- 推理循环（Lua 层控制流）
    local prompt = io:input()
    local tokens = sess:encode(prompt)
    sess:forward(sess:tensorize(tokens), 0)

    for i = 1, 120 do
        local hidden = sess:get_output_tensor()
        caps:send_tensor(hidden, plan.workers[1].peer_id)

        local result = caps:receive_tensor()
        sess:forward(result, nil)

        local tok = sess:sample(0.8)
        io:output(sess:decode(tok))
        if tok == sess:get_eos() then break end

        sess:forward(sess:tensorize({tok}), nil)
    end

    io:end_output()
    caps:send_eof(plan)
    sess:unload()
end
```

#### 3.3.3 所需注册的能力函数

| 函数 | 级别 | 说明 |
|------|------|------|
| `caps:analyze_model(path)` | B | 解析 GGUF 返回架构信息 |
| `caps:get_available_peers()` | A | 查询空闲节点 |
| `caps:plan_uniform(arch, peers)` | B | 均匀调度 |
| `caps:plan_weighted(arch, peers)` | B | 加权调度 |
| `caps:split_model(path, start, end)` | B | 切分模型 |
| `caps:send_file(peer, path)` | A | 发送文件 |
| `caps:establish_streams(plan)` | A | 建立张量流 |
| `caps:create_session(model, device, start, end)` | C | 创建推理会话 |
| `caps:allocate_io()` | A | 分配 IO 通道 |
| `caps:send_tensor(t, peer)` | A | 发送张量 |
| `caps:receive_tensor()` | A | 接收张量 |
| `caps:send_eof(plan)` | A | 发送 EOF |

---

### Phase 5 — Core 路由简化
> **目标：`branch_user.rs` 从 10+ 硬编码分支精简为统一 Execute 分支。**

#### 3.4.1 变更

- `UserCommand` 新增 `Execute { command, params, reply }` 变体
- `route_user()` 中 Execute 分支：查 `ProgramRegistry` → 加载脚本 → 注入 `caps` → 调用 `execute(params, caps)`
- 旧的 `Run` / `Pipeline` / `Profile` / `Send` / `List` 变体标记为 deprecated，最终删除
- `JobKind` 精简为 `Execute` / `Send` / `Receive` 三类

---

### Phase 6 — TUI 集成 + 热加载

- 命令补全数据源 → `ProgramRegistry::command_names()`
- `reload` 命令运行时重扫 `programs/`
- `Commands_Reloaded` 事件通知 TUI 刷新补全列表

---

### Phase 7 — 遗留清理

- 删除 `programs/orchestrator/*.tmpl` (6 个) 和 `programs/ml/*.tmpl` (4 个)
- 删除 `Src/Orchestrator/program_selector.rs`
- 清理 `Capabilities` 结构体

---

## 4. 关键设计决策

### 4.1 同步 vs 异步函数注册

mlua 0.12 支持两种注册方式：

| 方式 | 适用 | 签名 |
|------|------|------|
| `lua.create_function` | 纯计算、无需 await | `fn(&Lua, Args) → Result<Ret>` |
| `lua.create_async_function` | 需要调用 async Rust 方法 | `fn(&Lua, Args) → Future<Result<Ret>>` |

**策略：** Capabilities trait 方法均为 `async fn`，统一使用 `create_async_function`。纯同步辅助函数（如 `echo`）仅在测试中使用 `create_function`。

### 4.2 张量跨边界

`candle_core::Tensor` 不能直接通过 FFI 传递给 Lua。需要特殊处理：

- **短期方案：** 张量不穿 Lua 边界。`sess:forward(tensor, offset)` 中的 tensor 由内部 `MlSession` 持有，Lua 侧用整数句柄引用。
- **长期方案：** 将 `MlSession` 注册为 `mlua::UserData`（已完成），Lua 调用 `sess:forward(tokens, offset)` 时 tokens 为 `Vec<u32>`，由 `tensorize` 内部转换。

### 4.3 Capabilities 暴露粒度

不将底层 trait（Storage / Network / PeerManager / SessionManager）直接暴露给 Lua。暴露的是**业务级聚合函数**，内部组合调用多个 trait 方法。

```
Lua 调用:  caps:send_file(peer, path)
          ─────────────────────────────
Rust 内部: storage.acquire_read(path) → network.open_file_stream(peer) → network.send_file_data(stream, path)
```

### 4.4 错误处理

- Rust 函数返回 `Result<T, String>`，mlua 自动将 `Err(String)` 转换为 Lua error
- Lua 侧通过 `pcall` 捕获错误，`execute()` 失败时 Core 获取 Lua error 信息

---

## 5. 测试策略

| Phase | 测试类型 | 位置 | 新增测试数 |
|-------|---------|------|-----------|
| 0 | — | `docs/lua_design.md` | 设计文档 |
| 1 | 单元测试 | `Src/Lua/engine.rs` | +3 |
| 2 | 单元测试 | `Src/Lua/capability_binding.rs` | +3 |
| 3 | 集成测试 | `tests/t09_lua_integration.rs` | +2 |
| 4 | 集成测试 | `tests/t09_lua_integration.rs` | +3 |
| 5 | 现有测试回归 | `cargo test --lib orchestrator` | — |
| 6 | TUI 手动验证 | — | — |
| 7 | 全量回归 | `cargo test` | — |

**原则：** 每个 Phase 结束后 `cargo test --no-default-features` 全量通过，测试数只增不减。

---

## 6. 文件变更总览

| Phase | 文件 | 操作 |
|-------|------|------|
| 0 | `docs/lua_design.md` | **新建** |
| 1 | `Src/Lua/engine.rs` | 追加测试 |
| 2 | `Src/Lua/capability_binding.rs` | **新建** |
| 2 | `Src/Lua/mod.rs` | 添加 `pub mod capability_binding` |
| 3 | `tests/t09_lua_integration.rs` | **新建** |
| 4 | `programs/pipeline.lua` | **新建** |
| 4 | `programs/run.lua` | **新建** |
| 4 | `programs/list.lua` | **新建** |
| 5 | `Src/Orchestrator/command.rs` | 新增 Execute 变体 |
| 5 | `Src/Orchestrator/core/branch_user.rs` | 简化为统一分支 |
| 6 | `Src/TUI/app.rs` | 命令补全数据源切换 |
| 7 | `programs/orchestrator/*.tmpl` | 删除 |
| 7 | `programs/ml/*.tmpl` | 删除 |
| 7 | `Src/Orchestrator/program_selector.rs` | 删除 |

---

## 7. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| mlua 0.12.0-rc.1 API 不稳定 | 编译断裂 | 锁定版本，正式版发布后再升级 |
| async function 注册复杂度 | Phase 1 延期 | Phase 0 先做同步验证，Phase 1 逐个函数注册 |
| 张量跨 FFI 性能 | 推理吞吐下降 | 张量不穿 FFI 边界，仅传 Vec<u32>/句柄 |
| Lua 脚本错误难调试 | 开发效率降低 | 沙箱中保留 `print`，Core 捕获 Lua error 并输出堆栈 |
