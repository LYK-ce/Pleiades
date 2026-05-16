# Lua 脚本策略引擎设计方案

## 1. 背景

当前 Pleiades 的推理流程由两层 VM 驱动：

```
用户命令 → ProgramSelector(TOML→指令反序列化) → JobExecutor(Orchestrator_VM step循环 + ML_VM step循环) → 推理
```

这个架构的问题：

| 组件 | 职责 | 问题 |
|------|------|------|
| `Vm_Base` (slot.rs + vm.rs) | SlotFile 变量管理 + 7 条基础指令 | 用 HashMap+指令模拟变量/控制流，本质是手工实现一个编程语言 |
| `ML_VM` (instruction.rs + engine.rs) | 推理领域指令 + step() 循环 | 15 条指令 + match 分发，冗余 |
| `Orchestrator_VM` (instruction.rs + engine.rs) | 编排领域指令 + step() 循环 | 10 条领域指令，同样 match 分发 |
| `program_selector.rs` | TOML 反序列化 + 槽位查表 + 指令转换 | ~900 行，充当编译器角色 |
| `programs/*.tmpl` | 3 层 TOML 模板（orchestrator + ml） | 写策略就是写指令序列，可读性差、无控制流 |

**核心问题：我们在 TOML + 指令集上手工实现了一个微型的、残缺的编程语言。** Lua 本身就是成熟的、轻量的、为被嵌入而设计的语言——直接用 Lua 作为策略层，删掉整个自研 VM。

## 2. 目标

用 Lua (mlua) 替代全部自研 VM 和 TOML 模板系统：

- **策略脚本**：用户用 Lua 编写推理/调度策略，放入 `programs/` 目录
- **自描述**：脚本文件顶部的 `COMMAND` / `DESCRIPTION` 声明其身份
- **启动扫描**：Pleiades 启动时扫描 `programs/` 目录，构建命令注册表
- **TUI 动态补全**：所有扫描到的命令自动加入 TUI 补全列表
- **热加载**：`reload` 命令可运行时重扫目录，无需重启
- **Rust 能力层**：所有底层能力（模型加载、网络传输、调度计算）作为 Rust 函数注册到 Lua 作用域

## 3. 架构总览

```
┌──────────────────────────────────────────────────────────┐
│               Lua 策略脚本 (programs/*.lua)              │
│                                                          │
│  COMMAND = "pipeline"                                    │
│  DESCRIPTION = "..."                                     │
│  function execute(params, caps)                          │
│      local info = caps:analyze_model(params.model)       │
│      local plan = caps:plan_weighted(info, peers)        │
│      ...                                                 │
│  end                                                     │
├──────────────────────────────────────────────────────────┤
│               mlua FFI 边界                               │
│  Lua → Rust: caps:analyze_model() → Rust 函数调用         │
│  Rust → Lua: 返回值自动转换 (Table/String/Number/Bool)     │
├──────────────────────────────────────────────────────────┤
│               Rust 能力层 (Capabilities)                  │
│                                                          │
│  analyze_model / create_session / run_program             │
│  plan_uniform / plan_weighted                             │
│  establish_streams / join_workers                         │
│  get_available_peers / get_local_peer                     │
│  send_file / split_model / profile_model                  │
│  ml_input / ml_encode / ml_prefill / ml_inference         │
│  ml_sample / ml_decode / ml_output / ml_end_output        │
│  allocate_io / take_io                                    │
│                                                          │
│  infrastructure: Storage / Network / PeerManager          │
│                  Scheduler / EventBus / TensorIO           │
└──────────────────────────────────────────────────────────┘
```

Lua 处理**控制流和策略决策**（`if`/`while`/`for`/变量/函数），Rust 处理**重量级操作**（模型推理、网络通信、调度计算）。

## 4. 脚本格式约定

每个 Lua 脚本包含两个部分：

### 4.1 元数据（顶层变量）

```lua
COMMAND = "pipeline"
DESCRIPTION = "分布式流水线推理 (可选 strategy: uniform/weighted)"
```

### 4.2 入口函数

```lua
function execute(params, caps)
    -- params  : Table，用户命令行参数 { model = "...", device = "..." }
    -- caps    : Table，注册的 Rust 能力函数集
    -- 返回值  : 可选，可为 nil / string / table
end
```

### 4.3 完整示例

```lua
-- programs/pipeline.lua
COMMAND = "pipeline"
DESCRIPTION = "分布式流水线推理"

function execute(params, caps)
    -- 分析模型
    local info = caps:analyze_model(params.model)

    -- 获取可用节点
    local peers = caps:get_available_peers()
    if #peers == 0 then
        return single_machine(params, caps)
    end

    -- 选择调度策略
    local plan
    if params.strategy == "weighted" then
        plan = caps:plan_weighted(info, peers)
    else
        plan = caps:plan_uniform(info, peers)
    end

    caps:establish_streams(plan)

    -- 创建 Coordinator Session
    local session = caps:create_session(
        params.model, params.device,
        plan.coord_layer_start, plan.coord_layer_end
    )

    caps:run_program(session)
    caps:shutdown_session(session)
    caps:join_workers(plan)
end

function single_machine(params, caps)
    local session = caps:create_session(params.model, params.device, 0, 999999)
    caps:run_program(session)
    caps:shutdown_session(session)
end
```

```lua
-- programs/profile.lua
COMMAND = "profile"
DESCRIPTION = "单机性能测试 (单层推理耗时 + 内存)"

function execute(params, caps)
    caps:profile_model(params.model)
end
```

```lua
-- programs/list.lua
COMMAND = "list"
DESCRIPTION = "列出所有可用节点"

function execute(params, caps)
    local peers = caps:get_available_peers()
    for _, p in ipairs(peers) do
        print(p.peer_id .. "  [" .. p.status .. "]  " .. p.memory_mb .. "MB")
    end
end
```

### 4.4 脚本内可用 API 全集

运行时注册到 Lua 的完整能力集（分三级）：

#### A 级：无需模型的能力

| 函数 | 返回值 | 说明 |
|------|--------|------|
| `caps:get_available_peers()` | `[{peer_id, status, memory_mb, latency_ms, bandwidth_mbps}]` | 空闲 + 已连接节点 |
| `caps:get_local_peer()` | `{peer_id, status, memory_mb, ...}` | 本机节点信息 |
| `caps:send_file(peer_id, file_path)` | `bool` | 发送文件到指定节点 |
| `caps:allocate_io()` | `{ml_tx, ml_rx}` | 分配文本 IO 通道 |
| `caps:publish_event(type, payload)` | `()` | 发布事件到 EventBus |

#### B 级：模型操作

| 函数 | 返回值 | 说明 |
|------|--------|------|
| `caps:analyze_model(model_path)` | `ModelInfo` | 解析 GGUF 返回层数/架构/层大小 |
| `caps:split_model(source, start, end)` | `string` (分片路径) | 切分 GGUF 模型 |
| `caps:profile_model(model_path)` | `Duration` | 单层推理计时 + 内存查询 |

#### C 级：Session 生命周期

| 函数 | 返回值 | 说明 |
|------|--------|------|
| `caps:create_session(model, device, start, end)` | `SessionHandle` | 创建推理 Session |
| `caps:run_program(session)` | `{text, tokens, tok_per_sec, total_secs}` | 执行推理 |
| `caps:shutdown_session(session)` | `()` | 销毁 Session |

#### D 级：ML 推理指令（run_program 内部自动加载的 ml 函数表）

| 函数 | 说明 |
|------|------|
| `ml:input()` | 从 IO 通道读取用户输入 |
| `ml:encode()` | Tokenize |
| `ml:prefill()` | Prefill 阶段 |
| `ml:inference(input_type, src)` | 单步前向推理 |
| `ml:sample(tensor_slot)` | 采样下一个 token |
| `ml:decode()` | Detokenize |
| `ml:output()` | 输出当前 token 到 IO 通道 |
| `ml:end_output()` | 发送 EOF 信号 |
| `ml:send()` | 发送隐状态到下游 |
| `ml:receive()` | 从上游接收隐状态 |
| `ml:send_eof()` | 发送 EOF 完成信号 |
| `ml:get_flag(name)` | 读取 FLAG1 等状态标志 |
| `ml:timer_start()` | 开始计时 |
| `ml:timer_end()` | 结束计时，返回 Duration |
| `ml:get_text()` | 获取生成的文本 |
| `ml:get_tokens()` | 获取生成的 token 列表 |

## 5. Rust 侧实现

### 5.1 依赖

```toml
[dependencies]
mlua = { version = "0.12.0-rc.1", features = ["lua54", "vendored", "async"] }
```

`vendored`：静态编译 Lua 5.4，无需目标机安装 Lua。增量约 300KB。

### 5.2 沙箱配置

每个策略脚本运行在隔离的 Lua 环境中，禁用危险的全局 API：

```rust
fn sandbox(lua: &Lua) -> mlua::Result<()> {
    // 禁用文件系统访问
    lua.globals().set("io", mlua::Value::Nil)?;
    lua.globals().set("os", mlua::Value::Nil)?;
    // 禁用 require（禁止加载任意 C 模块）
    lua.globals().set("require", mlua::Value::Nil)?;
    // 禁用 dofile / loadfile
    lua.globals().set("dofile", mlua::Value::Nil)?;
    lua.globals().set("loadfile", mlua::Value::Nil)?;
    Ok(())
}
```

### 5.3 能力函数注册

将 Rust 函数绑定到 Lua 的 `caps` 表：

```rust
fn register_capabilities(lua: &Lua, caps: Arc<Capabilities>) -> mlua::Result<()> {
    let caps_table = lua.create_table()?;

    // ─── 节点查询 ───
    let caps_clone = caps.clone();
    caps_table.set("get_available_peers", lua.create_async_function(move |_, _: ()| {
        let caps = caps_clone.clone();
        async move {
            let peers = caps.peer_manager.get_idle_peers().await?;
            Ok(peers_to_lua_table(&peers))
        }
    })?)?;

    // ─── 模型分析 ───
    let caps_clone = caps.clone();
    caps_table.set("analyze_model", lua.create_async_function(move |_, model_path: String| {
        let caps = caps_clone.clone();
        async move {
            let info = caps.ml_engine.Analyze_Model(&model_path).await?;
            Ok(model_info_to_lua_table(&info))
        }
    })?)?;

    // ─── 调度 ───
    let caps_clone = caps.clone();
    caps_table.set("plan_weighted", lua.create_async_function(move |_, (info, peers): (LuaTable, LuaTable)| {
        let caps = caps_clone.clone();
        async move {
            let input = /* 从 Lua table 反序列化 */;
            let plan = caps.scheduler.Plan_Pipeline(input).await?;
            Ok(plan_to_lua_table(&plan))
        }
    })?)?;

    // ─── Session 管理 ───
    caps_table.set("create_session", lua.create_async_function(/* ... */)?)?;
    caps_table.set("run_program", lua.create_async_function(/* ... */)?)?;
    caps_table.set("shutdown_session", lua.create_async_function(/* ... */)?)?;

    // ─── 网络/文件 ───
    caps_table.set("send_file", lua.create_async_function(/* ... */)?)?;
    caps_table.set("establish_streams", lua.create_async_function(/* ... */)?)?;
    caps_table.set("join_workers", lua.create_async_function(/* ... */)?)?;

    // ─── Profile ───
    caps_table.set("profile_model", lua.create_async_function(/* ... */)?)?;

    lua.globals().set("caps", caps_table)?;
    Ok(())
}
```

### 5.4 启动扫描

Pleiades 启动时扫描 `programs/` 目录：

```rust
use std::collections::HashMap;
use std::fs;

pub struct ProgramEntry {
    pub path: PathBuf,
    pub command: String,
    pub description: String,
}

pub struct ProgramRegistry {
    programs: HashMap<String, ProgramEntry>,
}

impl ProgramRegistry {
    pub fn scan() -> mlua::Result<Self> {
        let mut map = HashMap::new();

        for entry in fs::read_dir("programs")? {
            let path = entry?.path();
            if path.extension() != Some(OsStr::new("lua")) {
                continue;
            }

            let lua = Lua::new();
            sandbox(&lua)?;
            lua.load(fs::read_to_string(&path)?).eval()?;

            let command: String = lua.globals().get("COMMAND")?;
            let description: String = lua.globals().get("DESCRIPTION")?;

            map.insert(command.clone(), ProgramEntry {
                path: path.clone(),
                command,
                description,
            });
        }

        Ok(Self { programs: map })
    }

    /// 返回所有命令名，供 TUI 补全
    pub fn command_names(&self) -> Vec<&String> {
        self.programs.keys().collect()
    }

    /// 获取某个命令的描述（TUI 悬浮提示）
    pub fn description(&self, cmd: &str) -> Option<&str> {
        self.programs.get(cmd).map(|e| e.description.as_str())
    }
}
```

### 5.5 命令执行

用户输入命令后，Core 加载脚本并调用 `execute`：

```rust
async fn execute_command(
    caps: Arc<Capabilities>,
    registry: &ProgramRegistry,
    command: &str,
    params: HashMap<String, String>,
) -> Result<Value, String> {
    let entry = registry.programs.get(command)
        .ok_or_else(|| format!("未找到命令: {}", command))?;

    let script = fs::read_to_string(&entry.path)
        .map_err(|e| format!("读取脚本失败: {}", e))?;

    let lua = Lua::new();
    sandbox(&lua).map_err(|e| e.to_string())?;
    register_capabilities(&lua, caps).map_err(|e| e.to_string())?;

    // 加载脚本（执行顶层，注册 COMMAND/DESCRIPTION/execute）
    lua.load(&script).eval().map_err(|e| format!("脚本解析失败: {}", e))?;

    // 注入参数
    let params_table = lua.create_table().map_err(|e| e.to_string())?;
    for (k, v) in params {
        params_table.set(k, v).map_err(|e| e.to_string())?;
    }
    lua.globals().set("params", params_table).map_err(|e| e.to_string())?;

    // 调用入口函数
    let execute_fn: Function = lua.globals().get("execute")
        .map_err(|_| format!("脚本 {} 缺少 execute 函数", command))?;

    Ok(execute_fn.call_async::<Value>(()).await.map_err(|e| e.to_string())?)
}
```

### 5.6 Core 路由简化

原 `branch_user.rs` 中每个 `UserCommand` 变体（Run/Pipeline/Profile/Send/List）各有 30-60 行硬编码分支。简化为：

```rust
UserCommand::Execute { command, params, reply } => {
    let caps = self.capabilities.clone();
    let registry = self.program_registry.read().await;

    if !registry.programs.contains_key(&command) {
        let _ = reply.send(Err(format!("未知命令: {}", command)));
        return;
    }

    tokio::spawn(async move {
        let result = execute_command(caps, &registry, &command, params).await;
        let _ = reply.send(result);
    });
}
```

仅 `Send` / `Receive` (涉及网络流事件，需要走 Core 的 B3 分支) 保留特殊路由。

### 5.7 TUI 命令补全

启动时将 `program_registry.command_names()` 注入 TUI 的命令补全列表：

```rust
// TUI 初始化时
let commands = registry.command_names().iter()
    .map(|s| s.to_string())
    .collect::<Vec<_>>();
tui_app.set_available_commands(commands);
```

用户输入 `p` → Tab → 补全为 `pipeline` 或 `profile`。

### 5.8 热加载

```rust
UserCommand::Reload { reply } => {
    match ProgramRegistry::scan() {
        Ok(new_registry) => {
            *self.program_registry.write().await = new_registry;
            // 通知 TUI 更新命令列表

            self.capabilities.event_bus.Publish(
                Bus_Event::Commands_Reloaded
            );
            let _ = reply.send(Ok(()));
        }
        Err(e) => {
            let _ = reply.send(Err(format!("重载失败: {}", e)));
        }
    }
}
```

## 6. 可删除的代码

引入 Lua 后，以下文件和代码行变为冗余：

| 文件/目录 | 行数（估计） | 原因 |
|-----------|-------------|------|
| `Src/Vm_Base/vm.rs` | ~450 | Lua 原生变量/控制流替代 |
| `Src/Vm_Base/slot.rs` | ~320 | Lua Table 替代 SlotFile |
| `Src/Vm_Base/mod.rs` | ~10 | 模块声明 |
| `Src/ML_Engine/ML_VM/instruction.rs` | ~130 | MlInstruction 枚举不再需要 |
| `Src/ML_Engine/ML_VM/engine.rs` | ~600 | step() 循环不再需要 |
| `Src/ML_Engine/ML_VM/slots.rs` | ~200 | MlSlots 不再需要 |
| `Src/Orchestrator/Orchestrator_VM/instruction.rs` | ~100 | OrchestratorInstruction 不再需要 |
| `Src/Orchestrator/Orchestrator_VM/engine.rs` | ~400 | step() + dispatch 不再需要 |
| `Src/Orchestrator/program_selector.rs` | ~920 | TOML 解析/槽位查表/指令转换不再需要 |
| `programs/orchestrator/*.tmpl` (6 个) | ~100 | 替换为 .lua |
| `programs/ml/*.tmpl` (4 个) | ~200 | 替换为 .lua |
| `Src/Orchestrator/core/job_executor.rs` 中 VM 相关 | ~100 | 简化为 Lua 调用 |
| **合计** | **~3530** | |

## 7. 保留但修改的部分

| 模块 | 修改内容 |
|------|----------|
| `Src/Orchestrator/orchestrator_vm/inference_handler.rs` | 核心逻辑保留，包装为 Loki 可调用的 Rust 函数 |
| `Src/Orchestrator/orchestrator_vm/network_handler.rs` | 同上 |
| `Src/Orchestrator/orchestrator_vm/scheduler_handler.rs` | 同上 |
| `Src/Orchestrator/core/branch_user.rs` | 大规模简化为两条分支：执行命令 / 特殊路由 |
| `Src/Orchestrator/job.rs` | JobKind 仅保留 Send/Receive（策略全部走 Execute） |
| `Src/ML_Engine/engine.rs` | Model 状态管理保留，指令分发删除 |
| `Src/TUI/mod.rs` | 添加命令补全数据源 + Reload 事件监听 |

## 8. Capabilities 统一重命名

为 Lua 用户准备更直观的名字：

| 当前名称（Rust trait module） | Lua 可见名称 |
|-----------------------------|-------------|
| `network` | 不直接暴露（通过高层 API） |
| `peer_manager` | `caps:get_available_peers()` 等 |
| `scheduler` | `caps:plan_uniform()` / `caps:plan_weighted()` |
| `ml_engine` | `caps:analyze_model()` / `caps:create_session()` / `caps:run_program()` |
| `storage` | 不直接暴露（透明使用） |
| `io_broker` | `caps:allocate_io()` |
| `tensor_switch` | 不直接暴露（透明使用） |
| `event_bus` | `caps:publish_event()` |

## 9. 实施计划

### Phase 1：mlua 基础设施（最低可行）

1. `Cargo.toml` 添加 `mlua = { version = "0.12.0-rc.1", features = ["lua54", "vendored", "async"] }`
2. 新建 `Src/Lua/` 模块：`sandbox.rs` + `registry.rs` + `capability_binding.rs`
3. 实现 `sandbox()` + `ProgramRegistry::scan()`
4. 注册 3 个示范能力函数（`get_available_peers` + `analyze_model` + `plan_uniform`）
5. 写 `programs/pipeline.lua` 替代 `Pipeline.tmpl`
6. 验证：`cargo test` 全量通过

### Phase 2：迁移全部能力

7. 注册全部 A/B/C/D 级函数
8. 迁移全部 TOML 模板 → `.lua` 脚本
9. `Core::route_user` 简化为统一执行分支
10. `JobKind` 精简
11. 删除 `program_selector.rs` + TOML 模板

### Phase 3：TUI 集成

12. 命令补全数据源切换为 `ProgramRegistry::command_names()`
13. `reload` 命令实现
14. `Commands_Reloaded` 事件通知 TUI 刷新

### Phase 4：删除遗留代码

15. 删除 `Vm_Base/` 整个目录
16. 删除 `ML_VM/` 目录中 instruction.rs / engine.rs / slots.rs
17. 删除 `Orchestrator_VM/` 目录中 instruction.rs / engine.rs / slots.rs
18. 整理 `Capabilities` 结构体（移除不再需要的字段）
19. 全量测试回归

### 测试策略

每个 Phase 完成后跑 `cargo test`，确保测试数不降级。Phase 1-2 期间旧测试和新 Lua 集成测试并存。Phase 4 一次性删除遗留代码并更新对应测试。

## 10. Lua 语法速查（供策略编写者参考）

Lua 语法足够接近 Python，核心差异就这几个：

```
Python           Lua
==============   ==============
def f():         function f() ... end
if x:            if x then ... end
while cond:      while cond do ... end
for x in list:   for _, x in ipairs(list) do ... end
a == b           a == b
a != b           a ~= b
True/False       true/false
None             nil
#comment         -- comment
list.append(x)   table.insert(list, x)
{}               {} (空 table)
x is None        x == nil
```

`Table` 在 Lua 中同时是列表和字典：
```lua
local list = {"a", "b", "c"}           -- 索引从 1 开始
local dict = {model = "x.gguf", device = "cuda"}
print(list[1])    -- "a"
print(dict.model) -- "x.gguf"
```
