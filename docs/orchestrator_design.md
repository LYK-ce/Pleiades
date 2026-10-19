# Orchestrator 设计文档

Presented by KeJi
Date ： 2026-05-16
Modified Date ： 2026-10-19

## 1. 模块概述

`Orchestrator` 模块是 Pleiades 的**策略路由与组件编排层**。不负责模型推理（ML Engine）、不负责网络传输（Network）、不负责文件管理（Storage）、不负责节点管理（PeerManagement）。只做命令路由和组件编排。

### 核心定义

> **Orchestrator = 命令路由 + 组件编排。** 接收来自 TUI/CLI 的用户命令和来自 Network 的入站请求，组合调用各 Capability 方法完成编排。业务策略/控制流留给未来 Lua 层。

### 模块结构

```
Orchestrator/
├── mod.rs               ← Capabilities 结构体 + DeviceCapability trait + test_utils + 内存查询
├── core.rs              ← Core struct + select! 5分支事件循环 + Job 管理 + flush
├── core/
│   ├── branch_user.rs   ← B1: UserCommand 路由（Lua 脚本执行 + 各命令实现）
│   ├── branch_command.rs← B2: InboundRequest 路由（ExecRemote → Lua 脚本）
│   ├── branch_stream.rs ← B3: Network_Inbound_Event 路由（文件/Session 流，spawn 异步化）
│   ├── branch_lifecycle.rs ← B4: LifecycleEvent 路由
│   └── job_executor.rs  ← JobExecutor（ReceiveFile 用最小实现）
├── command.rs           ← UserCommand + NetworkProtocol 定义 + 解析/序列化
├── job.rs               ← JobId / JobKind / JobState / JobResult / LifecycleEvent
├── inference_id.rs      ← 全局唯一推理 ID 生成
└── local_tensor_stream/ ← 本地张量流配对 Hub（frames.rs + mod.rs）
```

---

## 2. 调用关系

```
TUI / CLI
    │
    ▼ user_cmd_tx.send(UserCommand)
Core::run()
    │
    ├── B1: user_cmd_rx → route_user() → Lua 脚本 / Capability 方法
    ├── B2: inbound_rx  → route_inbound() → ExecRemote → spawn_lua_script()
    ├── B3: network_inbound_rx → route_stream() → spawn 文件/Session 流处理
    └── B4: lifecycle_rx → route_lifecycle() → EventBus + registry
    │
    ▼
Capabilities (Arc<Capabilities>)
    ├── network: Box<dyn Network_Capability>
    ├── storage: Arc<dyn StorageCapability>
    ├── peer_manager: Arc<dyn Peer_Management_Capability>
    ├── event_bus: Arc<EventBus>
    ├── local_stream_hub: Arc<LocalStreamHub>
    └── device_caps: RwLock<Vec<Arc<dyn DeviceCapability>>>

额外自由函数:
    use crate::ml_engine::{analyze_model, split_model};
```

> ML Engine 不在 Capabilities 中。`MlSession` 是 `mlua::UserData`，方法为 `&mut self`，
> 不适合 trait object。推理留给 Lua 层。分析/切分（`analyze_model`/`split_model`）是独立 async 函数，
> Orchestrator 直接 `use` 调用。
>
> 旧文档中的 `use crate::scheduler::Scheduler_Capability;` 已移除：`Scheduler` 模块已删除（全库 0 命中 `Scheduler_Capability`），调度计算不再作为 Capability 存在。

---

## 3. 数据结构

### 3.1 Capabilities

```rust
pub struct Capabilities {
    pub network: Box<dyn Network_Capability>,
    pub storage: Arc<dyn StorageCapability>,
    pub peer_manager: Arc<dyn Peer_Management_Capability>,
    pub event_bus: Arc<EventBus>,
    pub local_stream_hub: Arc<LocalStreamHub>,
    pub device_caps: RwLock<Vec<Arc<dyn DeviceCapability>>>,
}
```

**6 个字段**。旧 `session: Box<dyn Session_Capability>` 字段已移除——会话由 Core 内部持有的 `SessionManager` 管理（`Core::new` 中创建）；`local_stream_hub` 承载本地张量流配对，`device_caps` 为设备端（ugv/uav）注入的设备能力列表（`spawn_lua_script` 遍历调用 `register_lua_caps`）。所有字段使用各组件最新的 Reforge 后接口。

### 3.2 Core

```rust
pub struct Core {
    registry: HashMap<JobId, JobHandle>,
    shutting_down: bool,
    capabilities: Arc<Capabilities>,

    user_cmd_rx: mpsc::Receiver<UserCommand>,          // B1
    inbound_rx: mpsc::Receiver<InboundRequest>,         // B2
    network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>, // B3
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    lifecycle_rx: mpsc::Receiver<LifecycleEvent>,       // B4

    program_registry: ProgramRegistry,                  // Lua 脚本注册表
    session_mgr: Arc<Mutex<SessionManager>>,            // 会话管理
    device_preference: String,
}
```

> 注：旧字段 `scheduler_strategy_preference: Scheduler_Strategy` 已随 `Scheduler` 模块一并移除。

### 3.3 JobHandle

```rust
struct JobHandle {
    kind: JobKind,
    cancel: CancellationToken,
    inference_id: Option<u64>,
}
```

### 3.4 JobKind

```rust
pub enum JobKind {
    Generic,       // 通用任务 (未来由 Lua 替代)
    ReceiveFile,   // 文件接收 (Network → Storage)
}
```

旧 `Run`/`Coordinator`/`Relay`/`Pipeline`/`Distribute`/`Send`/`Profile` 全部合并为 `Generic`。

### 3.5 UserCommand

```rust
pub enum UserCommand {
    // ─── Lua 脚本执行（统一入口） ───
    Execute { command, params },                 // ProgramRegistry 查表 → Lua execute(params, caps)
    ExecRemote { peer, command, params },        // 在远程节点执行 Lua 脚本

    // ─── 系统命令 ───
    Cancel { job_id },                           // 取消指定作业
    Quit { reply },                              // 优雅退出（oneshot 回执）
    DisplayPeer,                                 // 查询节点列表
    SetDevice { device },                        // 切换计算设备偏好
    SetName { name },                            // 设置本地节点名称
    Reload,                                      // 重新加载 programs/user/ Lua 脚本
    Help,                                        // 帮助信息
    Flush,                                       // 刷新存储索引
    List,                                        // 列出 Storage 追踪的文件
    Dial { addr },                               // 主动拨号

    // ─── 推理 / 会话 ───
    Run { script, model_path },                  // 本地推理（尚未实现）
    Session { model_id },                        // 创建推理 Session
    SessionInference { command, session_id, model_path }, // ML Thread 连接指定 Session
    Api { session_id },                          // OpenAI 兼容 API Server
    Webui { port },                              // YOLO 检测结果 WebSocket 展示

    // ─── 过渡命令（未来迁移到 Lua 脚本） ───
    DistributeModel { model_path, peers },       // 分发模型分片（尚未实现）
    Send { file_path, peer_id },                 // 发送文件（走 programs/user/send.lua）
    Profile { model_id },                        // 单层推理耗时（尚未实现）
}
```

**20 个变体**（2 个 Lua 入口 + 10 个系统命令 + 5 个推理/会话 + 3 个过渡命令）。仅 `Quit` 保留 `reply`（oneshot）通道，其余命令结果统一经 EventBus `Output`/`State` 发布。

### 3.6 NetworkProtocol

```rust
pub enum NetworkProtocol {
    Establish_Tensor_Stream { inference_id, target_peer_id },
    Join_Pipeline { inference_id, model_file_id, device, layer_start, layer_end },
    Profile_Request { model_id, layer_count, device },
    ExecRemote { command, params_json },          // 请求远程节点执行 Lua 脚本
}
```

4 个变体。文本协议（`|` 分隔），通过 `Parse_Network_Command` / `Serialize_Network_Command` 解析和序列化。当前 B2 实际处理 `ExecRemote`（`EXEC|{command}|{params_json}`），其余变体仅记录日志并回复 `FAIL|未实现的协议`。

---

## 4. 事件循环

### 4.1 5 分支 select!

```rust
pub async fn run(mut self) {
    const SHUTDOWN_GRACE_SECS: u64 = 30;
    loop {
        if self.shutting_down && self.registry.is_empty() { break; }
        tokio::select! {
            Some(cmd) = self.user_cmd_rx.recv(), if !self.shutting_down => {
                self.route_user(cmd).await;
            }
            Some(req) = self.inbound_rx.recv(), if !self.shutting_down => {
                self.route_inbound(req).await;
            }
            Some(event) = self.network_inbound_rx.recv(), if !self.shutting_down => {
                self.route_stream(event).await;
            }
            Some(event) = self.lifecycle_rx.recv() => {
                self.route_lifecycle(event);
            }
            _ = tokio::time::sleep(Duration::from_secs(SHUTDOWN_GRACE_SECS)),
                if self.shutting_down => {
                break;  // shutdown 超时兜底
            }
        }
    }
}
```

| # | 通道 | 守卫 | 方法 | 当前状态 |
|---|------|------|------|---------|
| B1 | user_cmd_rx | !shutting_down | route_user() | ✅ 实现（Lua 脚本 + 系统命令；Run/DistributeModel/Profile 未实现） |
| B2 | inbound_rx | !shutting_down | route_inbound() | ✅ 实现（`ExecRemote` → Lua 脚本；File 入站 TODO） |
| B3 | network_inbound_rx | !shutting_down | route_stream() | ✅ 实现（文件/Session 流 spawn 异步处理） |
| B4 | lifecycle_rx | 无 | route_lifecycle() | ✅ 实现 |
| B5 | shutdown 超时 | shutting_down | — | ✅ 实现（30s 兜底，防 Job 卡死进程不退出） |

### 4.2 route_user() 命令状态

| 命令 | 状态 | 说明 |
|------|------|------|
| Execute | ✅ | **统一入口**: ProgramRegistry 查表 → `spawn_lua_script`（fire-and-forget） |
| ExecRemote | ✅ | 解析 peer 名称 → `NetworkProtocol::ExecRemote` → `network.send_data`（请求-响应） |
| Cancel | ✅ | cancel_job + Output |
| Quit | ✅ | shutdown + oneshot reply |
| DisplayPeer | ✅ | peer_manager.Get_Peers() |
| SetName | ✅ | Set_Local_Name + 持久化 config + gossipsub 广播 |
| SetDevice | ✅ | 更新 device_preference |
| Reload | ✅ | program_registry.reload_user() |
| Help | ✅ | BUILTIN_COMMANDS → Output |
| Flush | ✅ | `do_flush()`（spawn 异步） |
| List | ✅ | storage.List() |
| Dial | ✅ | addr 解析 → network.dial（spawn 异步） |
| Run | ⚠️ 未实现 | 返回 "Run: not yet implemented"（将迁移到 Execute） |
| DistributeModel | ⚠️ 未实现 | 返回 "DistributeModel: not yet implemented"（将迁移到 Execute） |
| Send | ✅ | 查 `send` 脚本 → `spawn_lua_script`（file + peer 参数） |
| Profile | ⚠️ 未实现 | 返回 "Profile: 尚未实现"（将迁移到 Execute） |
| Session | ✅ | session_mgr.create_session + gossipsub sessions 广播 |
| SessionInference | ✅ | 查脚本 → `spawn_lua_script`（session_id + model_path） |
| Api | ✅ | `spawn_api_server`（OpenAI 兼容） |
| Webui | ✅ | `spawn_webui_server`（YOLO 检测结果 WS 展示） |

### 4.3 Job 管理

```rust
fn register_job(&mut self, job_id: JobId, kind: JobKind);
fn cancel_job(&mut self, job_id: JobId);
fn shutdown(&mut self);
```

不再有 `spawn_job(instructions)` — VM 指令序列已移除。Job 由 Lua 线程或直接 Capability 调用承担；当前 Lua 脚本走 fire-and-forget（`spawn_lua_script`），不注册 Job。

---

## 5. 与外部模块的接口

### 5.1 已集成的 Capability 调用

| 模块 | 调用示例 | 使用位置 |
|------|---------|---------|
| Peer_Management | `peer_manager.Get_Peers().await` | route_user DisplayPeer |
| Peer_Management | `peer_manager.Get_Peer_By_Name(&name).await` | route_user ExecRemote / Send |
| Peer_Management | `peer_manager.Set_Local_Name(&name).await` | route_user SetName |
| Storage | `storage.List().await` | route_user List |
| Storage | `storage.Flush().await` / `Acquire_Write()` | route_user Flush / route_stream 文件接收 |
| Network | `network.send_data(peer, DataType::Command, payload).await` | route_user ExecRemote |
| Network | `network.dial(addr).await` | route_user Dial |
| Network | `network.publish_gossipsub(topic, payload).await` | SetName / Flush / Session |
| Network | `network.send_response(request_id, ...).await` | route_inbound |
| Session | `SessionManager::create_session / list_sessions` | route_user Session / Api |
| EventBus | `event_bus.Publish(Output / State / Notify)` | 所有命令结果回传 |

### 5.2 待实现 / TODO 项

| 模块 | 方法 | 用途 |
|------|------|------|
| ML Engine (自由函数) | `analyze_model(path).await` | Run / DistributeModel（当前未实现，将迁移到 Lua 脚本） |
| ML Engine (自由函数) | `split_model(src, start, end, out_dir).await` | DistributeModel（同上） |
| Network | File 入站元数据协商 → `pending_file_receives` | route_inbound `DataType::File` 分支（TODO） |
| JobExecutor | 完整 ReceiveFile 实现 | 当前为最小实现（立即 Done/Success） |

### 5.3 ML Engine 特殊说明

ML Engine 不再以 trait object 形式存在于 Capabilities 中：

- **推理方法** (`load_model`/`forward`/`sample`/`encode`/`decode`/`tensorize`/`unload`) — 全部是 `MlSession` 的方法 (`&mut self`)，由 Lua `sess:method()` 调用
- **独立函数** (`analyze_model`/`split_model`) — 普通 async 函数，Orchestrator 直接 `use crate::ml_engine::{analyze_model, split_model}` 后 `await` 调用

---

## 6. 与旧架构对比

| 维度 | 旧（Reforge 前） | 新（Reforge 后） |
|------|----------------|-----------------|
| Capabilities 字段数 | 8 | **6**（network/storage/peer_manager/event_bus/local_stream_hub/device_caps） |
| ML Engine 持有方式 | `Box<dyn ML_Engine_Capability>` (trait 未定义) | 不在 Capabilities 中 |
| 指令系统 | Orchestrator_VM 17 条指令 + match 分发 | 直接方法调用 |
| 策略模板 | ProgramSelector 920 行 TOML→指令 | Lua 脚本（ProgramRegistry + `spawn_lua_script`） |
| IO 通道 | Arc\<LLM_IO_Broker\> (类型不存在) | Core 内部 `Arc<Mutex<SessionManager>>`（Capabilities 不再含 session） |
| Tensor 流 | Arc\<Tensor_Port_Switch\> (类型不存在) | Network_Capability 内置 + `local_stream_hub` |
| Job 执行 | JobExecutor + Orchestrator_VM step() 循环 | JobExecutor 最小实现 |
| 编译依赖 | 依赖 Vm_Base + ML_VM + TOML | 不依赖 VM 体系 |
| 编译状态 | ❌ 37 错误 (trait 未定义等) | ⚠️ 0 Orchestrator 错误 (外部模块有预存错误) |

---

## 7. 已删除的代码

| 删除项 | 行数 | 原因 |
|--------|------|------|
| `Orchestrator_VM/` (7 文件) | ~1100 | VM 指令分发不再需要 |
| `program_selector.rs` | ~920 | TOML 模板不再需要 |
| **合计** | **~2020** | |

> 另：`Scheduler/` 模块已整体删除（全库 0 命中 `scheduler` / `Scheduler_Capability`），调度计算不再由 Orchestrator 调用；删除行数待确认。

## 8. 已知限制

| 限制 | 说明 |
|------|------|
| 推理未实现 | Run / Profile 返回"未实现"提示，等待 Lua 层 |
| 模型分发未实现 | DistributeModel 未实现；Send 已通过 `programs/user/send.lua` 落地 |
| File 入站未实现 | B2 `DataType::File` 分支仍为 TODO（无元数据协商） |
| JobExecutor 为最小实现 | 立即 Done/Success，ReceiveFile 完整逻辑待补 |
| test_utils stub 使用旧方法名 | StubPeerManager 中 `List_Peers` 等方法已更新为最新 trait |
| 外部模块兼容性 | Network/TUI 等模块与最新 trait 的兼容性待确认 |
| SessionManager 缺 Take_Frontend | TUI `run` 命令用 `JobId` 取 `IoFrontend`，但 SessionManager 索引体系是 `session_id`（现状待确认） |

---

## 9. 状态

### ✅ 已完成

- Capabilities 结构体重写（6 字段，使用最新接口）
- Core 事件循环（5 分支 select!，含 shutdown 超时兜底）
- UserCommand 全部 20 个变体定义 + 路由实现
- NetworkProtocol 4 个变体 + 解析/序列化
- JobKind 精简 + JobId/JobState/JobResult/LifecycleEvent
- Execute / ExecRemote / Cancel / Quit / DisplayPeer / SetName / SetDevice / Reload / Help / Flush / List / Dial / Send / Session / SessionInference / Api / Webui 实现
- B2 ExecRemote 请求-响应、B3 文件/Session 流入站处理
- Lua 脚本 fire-and-forget（`spawn_lua_script` 独立线程，P0-1 方案）
- test_utils 更新为最新 trait 方法
- Orchestrator_VM + program_selector + Scheduler 删除

### ⚠ 未实现 / 占位

- Run / Profile / DistributeModel 命令（返回"未实现"提示）
- B2 `DataType::File` 入站（TODO）
- JobExecutor 最小实现（立即返回 Success）

---

## 10. 问题追踪（并入自 `docs/Orchestrator_design.md`）

### P0-1: Lua 脚本执行阻塞 Core 主循环 ✅ 已解决

**问题**: `UserCommand::Execute` 和 `UserCommand::Send` 在 `route_user()` 中直接 `await execute_lua_script(...)`，该函数内部：
1. 同步 `std::fs::read_to_string` 阻塞 tokio 线程
2. Lua 脚本执行时长不可控
3. Core 的 `select!` 循环在此期间完全停滞，B2/B3/B4 事件无法处理

**方案**: fire-and-forget 独立 OS 线程。

```
┌─ Core (select!) ─────┐         ┌─ 独立线程 ────────────────────┐
│  Execute/Send → fire  │         │  rt.block_on(async {          │
│  立即返回，不等待       │         │    lua = LuaContext::new()     │
│                        │         │    注册 caps                  │
│                        │         │    execute(...)               │
│                        │         │  })                          │
│                        │         │  结果 → event_bus.Publish()   │
└───────────────────────┘         └───────────────────────────────┘
```

**实现**: 由原 async `execute_lua_script()`（已删除）改为 `spawn_lua_script()`（`std::thread::spawn` + `new_current_thread` runtime）

**决策**:
- 纯 fire-and-forget，不追踪 Job 生命周期
- 不提供 cancel 机制（后续迭代考虑）
- 不通过 mpsc 回传结果（线程内直接 publish EventBus）
- `mlua::Lua: !Send` 不构成障碍 — 实例在线程内创建/销毁

**文件**: `pleiades-base/src/Orchestrator/core/branch_user.rs`（`spawn_lua_script`）
