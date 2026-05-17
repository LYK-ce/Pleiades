# Lua 模块设计文档

Presented by KeJi
Date ： 2026-05-17

## 1. 模块概述

`Lua` 模块为 Pleiades 提供 **Lua 脚本策略引擎**。它将 mlua 封装为安全的沙箱运行时，替代旧有的三层 VM 体系（Vm_Base → ML_VM / Orchestrator_VM），实现策略脚本化。

### 核心定义

> **Lua 模块 = 沙箱运行时 + 脚本注册表 + 能力函数桥接。**
> 负责 Lua 环境创建、沙箱安全配置、能力函数注册、脚本扫描与命令路由。
> 不负责具体业务逻辑 — 那由 Lua 脚本和 Rust 能力函数共同完成。

### 模块结构

```
Lua/
├── engine.rs              ← LuaContext: 沙箱创建 + 基础测试
├── registry.rs            ← ProgramRegistry: 脚本扫描 + 命令注册表
├── capability_binding.rs  ← (待建) 能力函数注册: Rust → Lua 桥接
└── mod.rs                 ← 模块入口
```

### 调用关系

```
TUI / CLI
   │
   ▼
Core::route_user()
   │
   ├── ProgramRegistry::get(command)  → 脚本路径
   ├── LuaContext::new()              → 沙箱化 Lua 实例
   ├── capability_binding::register() → 注入 caps 函数表
   ├── lua.load(script)               → 执行顶层（注册 COMMAND/DESCRIPTION/execute）
   └── execute_fn.call(params)        → 调用入口函数
         │
         ▼
   Lua 脚本 (programs/*.lua)
         │
         ├── caps:analyze_model()  ─┐
         ├── caps:get_peers()       │ Rust 能力函数
         ├── sess:forward()         │ (mlua UserData /
         └── sess:sample()         ─┘  create_async_function)
```

---

## 2. 与其他模块的边界

| 职责 | 负责方 | 说明 |
|------|--------|------|
| 沙箱创建 | Lua 模块 | `LuaContext::new()` — 禁用 os/io/require |
| 脚本扫描 | Lua 模块 | `ProgramRegistry::scan()` — 收集 COMMAND/DESCRIPTION |
| 能力函数注册 | Lua 模块 | `capability_binding.rs` — 将 Capabilities 方法注入 Lua |
| 模型推理 | ML_Engine | `MlSession` (mlua::UserData) — Lua 直接调用 sess:forward() |
| 网络通信 | Network | 通过聚合能力函数暴露（不直接给 Lua trait） |
| 文本 IO | Session_Manager | 通过 IoHandle/IoFrontend 暴露给 Lua |
| 控制流 | Lua 脚本 | if/while/for/变量/函数 — 全部在 Lua 侧 |

---

## 3. 数据结构

### 3.1 LuaContext — 沙箱工厂

```rust
/// 沙箱化的 Lua 实例工厂。不持有状态，仅提供创建方法。
pub struct LuaContext;

impl LuaContext {
    /// 创建 Lua 5.4 实例，禁用 os / io / require / dofile / loadfile。
    /// 保留 string / table / math 标准库。
    pub fn new() -> mlua::Result<Lua>;
}
```

**被禁用的全局 API：**

| API | 风险 | 替代方案 |
|-----|------|---------|
| `os` | 系统调用、进程控制 | 通过 caps 函数受限访问 |
| `io` | 文件系统读写 | Storage 模块 + caps 函数 |
| `require` | 加载任意 C 模块 | 不需要，所有能力在 Rust 侧 |
| `dofile` | 动态执行外部文件 | ProgramRegistry 控制加载 |
| `loadfile` | 同上 | 同上 |

### 3.2 ProgramEntry — 脚本元数据

```rust
/// 单个 Lua 脚本的注册信息
pub struct ProgramEntry {
    /// 脚本文件路径
    pub path: PathBuf,
    /// 命令名（来自 Lua 顶层 COMMAND 变量）
    pub command: String,
    /// 命令描述（来自 Lua 顶层 DESCRIPTION 变量）
    pub description: String,
}
```

### 3.3 ProgramRegistry — 命令注册表

```rust
/// 命令注册表：命令名 → 脚本条目
pub struct ProgramRegistry {
    programs: HashMap<String, ProgramEntry>,
}
```

**方法：**

| 方法 | 签名 | 说明 |
|------|------|------|
| `scan()` | `fn scan() → mlua::Result<Self>` | 扫描 programs/ 目录，收集所有 .lua 脚本元数据 |
| `command_names()` | `fn command_names() → Vec<&String>` | 返回所有命令名（TUI 补全用） |
| `get()` | `fn get(cmd: &str) → Option<&ProgramEntry>` | 按命令名查找脚本条目 |

---

## 4. 脚本格式约定

### 4.1 元数据

每个 `.lua` 脚本文件顶部声明两个顶层变量：

```lua
COMMAND = "pipeline"
DESCRIPTION = "分布式流水线推理"
```

- `COMMAND` 是必须的顶层字符串变量。缺少此变量时，脚本被 `scan()` 跳过。
- `DESCRIPTION` 是可选的顶层字符串变量，TUI 悬浮提示使用。

### 4.2 入口函数

```lua
function execute(params, caps)
    -- params : Table — 用户命令行参数
    -- caps   : Table — 注册的 Rust 能力函数集
end
```

- `params` 由 Core 从命令行解析后注入（`{model = "...", device = "..."}`）。
- `caps` 由 `capability_binding::register()` 在脚本执行前注入。
- 返回值可选，Lua 侧可为 nil / string / table。

### 4.3 完整示例

```lua
-- programs/hello.lua
COMMAND = "hello"
DESCRIPTION = "演示脚本：验证 Lua 引擎正常启动"

function execute(params)
    print("Hello from Pleiades Lua engine!")
    print("Params received: msg = " .. (params.msg or "(none)"))
    return "ok"
end
```

---

## 5. 沙箱安全模型

```
Lua 脚本
   │
   ├── ✅ 可用: string / table / math / print
   ├── ✅ 可用: caps 函数表 (白名单注册)
   ├── ✅ 可用: 自定义变量/函数/控制流
   │
   ├── ❌ 禁用: os (系统调用)
   ├── ❌ 禁用: io (文件访问)
   ├── ❌ 禁用: require (C 模块加载)
   ├── ❌ 禁用: dofile / loadfile (动态执行)
   │
   └── 限制: 每脚本独立 Lua 实例，无跨脚本状态共享
```

**设计原则：**

- 每个脚本运行在**独立**的 `Lua` 实例中，执行完毕后销毁。
- 脚本间不共享全局状态（函数表 `caps` 在每次执行时重新注入）。
- 脚本运行时间无强制限制（未来可加 `mlua::Function::call` 超时包装）。
- 内存上限由 OS 进程管理，Lua GC 在实例 drop 时全量回收。

---

## 6. 能力函数桥接（capability_binding.rs）

### 6.1 设计目标

将 Rust 的 `Capabilities` 容器中的异步方法暴露为 Lua 可调用的函数，挂载到 `caps` 表下。

### 6.2 注册方式

mlua 0.12 支持两种注册方式：

| 方式 | 适用 | 签名 |
|------|------|------|
| `lua.create_function` | 同步、纯计算 | `fn(&Lua, Args) → Result<Ret>` |
| `lua.create_async_function` | 异步、需要 tokio runtime | `fn(&Lua, Args) → Future<Result<Ret>>` |

由于 `Capabilities` 中所有 trait 方法均为 `async fn`，统一使用 `create_async_function`。

### 6.3 注册流程

```rust
fn register_capabilities(lua: &Lua, caps: &Arc<Capabilities>) -> mlua::Result<()> {
    let caps_table = lua.create_table()?;

    // 每个能力函数：clone Arc → create_async_function → set 到 caps_table

    // ─── A 级：无需模型 ───
    caps_table.set("get_available_peers", lua.create_async_function(move |_, ()| {
        let caps = caps.clone();
        async move { /* caps.peer_manager.Get_Peers().await */ Ok(()) }
    })?)?;

    // ─── B 级：模型操作 ───
    caps_table.set("analyze_model", lua.create_async_function(move |_, path: String| {
        let caps = caps.clone();
        async move { /* analyze_model(Path::new(&path)).await */ Ok(()) }
    })?)?;

    // ... 更多函数

    lua.globals().set("caps", caps_table)?;
    Ok(())
}
```

### 6.4 能力函数分级

#### A 级：无需模型的基础能力

| 函数 | 说明 | 对应 Rust 调用 |
|------|------|---------------|
| `caps:get_available_peers()` | 查询空闲节点列表 | `peer_manager.Get_Peers()` |
| `caps:get_local_peer()` | 本机节点信息 | `peer_manager.Get_Peer(local_id)` |
| `caps:send_file(peer, path)` | 发送文件到节点 | `storage.acquire_read` + `network.send_file` |
| `caps:allocate_io()` | 分配文本 IO 通道 | `session.connect()` |
| `caps:publish_event(type, payload)` | 发布事件到 EventBus | `event_bus.Publish()` |

#### B 级：模型文件操作

| 函数 | 说明 | 对应 Rust 调用 |
|------|------|---------------|
| `caps:analyze_model(path)` | 解析 GGUF 架构信息 | `ml_engine::analyze_model()` |
| `caps:split_model(src, start, end)` | 切分 GGUF 模型分片 | `ml_engine::split_model()` |
| `caps:plan_uniform(arch, peers)` | 均匀调度 | Orchestrator 调度逻辑 |
| `caps:plan_weighted(arch, peers)` | 加权调度 | Orchestrator 调度逻辑 |

#### C 级：Session 生命周期

| 函数 | 说明 | 对应 Rust 调用 |
|------|------|---------------|
| `caps:create_session(model, device, start, end)` | 创建推理会话 | `MlSession::load_model()` → userdata |
| `caps:destroy_session(sess)` | 销毁会话 | `sess:unload()` |

#### D 级：ML 推理指令

D 级函数不注册到 `caps`，而是通过 `MlSession` userdata 的方法暴露：

| Lua 调用 | Rust 方法 |
|----------|----------|
| `sess:encode(text)` | `MlSession::encode()` |
| `sess:decode(token_id)` | `MlSession::decode()` |
| `sess:tensorize(token_ids)` | `MlSession::tensorize()` |
| `sess:forward(tensor, offset)` | `MlSession::forward()` |
| `sess:sample(temperature)` | `MlSession::sample()` |
| `sess:get_eos()` | `MlSession::get_eos()` |
| `sess:get_offset()` | `MlSession::get_offset()` |
| `sess:get_output_tensor()` | `MlSession::get_output_tensor()` |
| `sess:set_input_tensor(t)` | `MlSession::set_input_tensor()` |
| `sess:unload()` | `MlSession::unload()` |

### 6.5 数据转换

Lua Table ↔ Rust struct 的双向转换需要手动实现：

```rust
// Lua → Rust: 从 params table 提取字段
fn lua_table_to_params(table: &mlua::Table) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in table.pairs::<String, String>() {
        let (k, v) = pair?;
        map.insert(k, v);
    }
    Ok(map)
}

// Rust → Lua: 将 Vec<PeerInfo> 转换为 Lua table
fn peers_to_lua_table<'lua>(lua: &'lua Lua, peers: &[PeerInfo]) -> mlua::Result<mlua::Table<'lua>> {
    let tbl = lua.create_table()?;
    for (i, p) in peers.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("peer_id", p.peer_id.to_string())?;
        entry.set("status", format!("{:?}", p.status))?;
        entry.set("memory_mb", p.profile.memory_mb)?;
        tbl.set(i + 1, entry)?; // Lua 数组从 1 开始
    }
    Ok(tbl)
}
```

---

## 7. 执行流程

### 7.1 Core 调用入口

```
1. Core::route_user() 收到 UserCommand::Execute { command, params, reply }
2. ProgramRegistry::get(&command) → 获取脚本路径
3. LuaContext::new() → 创建沙箱 Lua 实例
4. capability_binding::register(&lua, &caps) → 注入 caps 函数表
5. lua.load(script_content).eval() → 执行 Lua 顶层（注册 COMMAND/DESCRIPTION/execute）
6. 构造 params table → lua.globals().set("params", params_table)
7. let execute_fn: Function = lua.globals().get("execute")
8. execute_fn.call_async(params_table) → 等待 Lua 脚本执行完毕
9. 结果通过 reply oneshot 通道返回给 TUI/CLI
```

### 7.2 生命周期

```
                    Lua 实例创建
                         │
                    ┌────▼────┐
                    │ 沙箱配置  │ (禁用 os/io/require)
                    └────┬────┘
                         │
                    ┌────▼────┐
                    │ 注册 caps │ (注入能力函数表)
                    └────┬────┘
                         │
                    ┌────▼────┐
                    │ 加载脚本  │ (eval 顶层)
                    └────┬────┘
                         │
                    ┌────▼────┐
                    │execute() │ (Lua 脚本执行)
                    └────┬────┘
                         │
                    ┌────▼────┐
                    │ 返回结果  │
                    └────┬────┘
                         │
                    Lua 实例 drop (GC 回收全部资源)
```

---

## 8. 与 MlSession 的集成

`MlSession` 是 `mlua::UserData` 类型，在 Lua 侧表现为对象：

```lua
-- Phase 3+ 的推理脚本示例
local sess = caps:create_session(model, "cpu", 0, 39)
local tokens = sess:encode(prompt)
sess:forward(sess:tensorize(tokens), 0)

for i = 1, 120 do
    local tok = sess:sample(0.8)
    print(sess:decode(tok))
    if tok == sess:get_eos() then break end
    sess:forward(sess:tensorize({tok}), nil)
end

caps:destroy_session(sess)
```

`caps:create_session()` 内部调用 `MlSession::load_model()`，返回的 `MlSession` 通过 `mlua::UserData` 机制直接暴露给 Lua，无需额外包装。

---

## 9. 错误处理

| 错误来源 | 处理方式 |
|---------|---------|
| 脚本语法错误 | `lua.load().eval()` 返回 `mlua::Error`，Core 捕获并回复前端 |
| 缺少 `execute` 函数 | `lua.globals().get::<Function>("execute")` 返回 `Err` |
| 缺少 `COMMAND` 变量 | `scan()` 跳过该脚本（静默忽略） |
| Rust 能力函数返回 `Err` | `create_async_function` 自动将 `Err(String)` 转为 Lua error |
| Lua 运行时错误 | `execute_fn.call()` 返回 `mlua::Error`，含堆栈信息 |

---

## 10. 状态

### ✅ 已完成

- `engine.rs` — `LuaContext::new()` 沙箱创建，2 tests
- `registry.rs` — `ProgramRegistry::scan()` 脚本扫描，1 test
- `programs/hello.lua` — 演示脚本
- `Cargo.toml` — mlua 依赖已添加

### ⚠ 待完成

- `capability_binding.rs` — 能力函数注册层（核心缺失）
- `programs/pipeline.lua` / `run.lua` / `list.lua` — 业务脚本
- Core 路由简化 — `branch_user.rs` 统一 Execute 分支
- TUI 命令补全 — 接入 `ProgramRegistry::command_names()`
- `reload` 命令 — 运行时热加载

### 🟡 待优化

- 脚本执行无超时控制
- 无 Lua 内存配额限制
- 错误堆栈信息仅 `mlua::Error` 提供，未做格式化美化
