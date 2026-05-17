# Orchestrator 设计文档

Presented by KeJi
Date ： 2026-05-16

## 1. 模块概述

`Orchestrator` 模块是 Pleiades 的**策略路由与组件编排层**。不负责模型推理（ML Engine）、不负责网络传输（Network）、不负责文件管理（Storage）、不负责节点管理（PeerManagement）、不负责调度计算（Scheduler）。只做命令路由和组件编排。

### 核心定义

> **Orchestrator = 命令路由 + 组件编排。** 接收来自 TUI/CLI 的用户命令和来自 Network 的入站请求，组合调用各 Capability 方法完成编排。业务策略/控制流留给未来 Lua 层。

### 模块结构

```
Orchestrator/
├── mod.rs               ← Capabilities 结构体 + test_utils + 内存查询
├── core.rs              ← Core struct + select! 4分支事件循环 + Job 管理
├── core/
│   ├── branch_user.rs   ← B1: UserCommand 路由 (todo!() 占位)
│   ├── branch_command.rs← B2: InboundRequest 路由 (todo!() 占位)
│   ├── branch_stream.rs ← B3: Network_Inbound_Event 路由 (todo!() 占位)
│   ├── branch_lifecycle.rs ← B4: LifecycleEvent 路由
│   └── job_executor.rs  ← JobExecutor stub
├── command.rs           ← UserCommand + NetworkProtocol 定义 + 解析/序列化
├── job.rs               ← JobId / JobKind / JobState / JobResult / LifecycleEvent
└── inference_id.rs      ← 全局唯一推理 ID 生成
```

---

## 2. 调用关系

```
TUI / CLI
    │
    ▼ user_cmd_tx.send(UserCommand)
Core::run()
    │
    ├── B1: user_cmd_rx → route_user() → Capability 方法
    ├── B2: inbound_rx  → route_inbound() → todo!()
    ├── B3: network_inbound_rx → route_stream() → todo!()
    └── B4: lifecycle_rx → route_lifecycle() → EventBus + registry
    │
    ▼
Capabilities (Arc<Capabilities>)
    ├── network: Box<dyn Network_Capability>
    ├── storage: Arc<dyn StorageCapability>
    ├── peer_manager: Box<dyn Peer_Management_Capability>
    ├── session: Box<dyn Session_Capability>
    └── event_bus: Arc<EventBus>

额外自由函数:
    use crate::ml_engine::{analyze_model, split_model};
    use crate::scheduler::Scheduler_Capability;  // 按需 use，不在 Capabilities 中
```

> ML Engine 不在 Capabilities 中。`MlSession` 是 `mlua::UserData`，方法为 `&mut self`，
> 不适合 trait object。推理留给未来 Lua 层。分析/切分（`analyze_model`/`split_model`）是独立 async 函数，
> Orchestrator 直接 `use` 调用。

---

## 3. 数据结构

### 3.1 Capabilities

```rust
pub struct Capabilities {
    pub network: Box<dyn Network_Capability>,
    pub storage: Arc<dyn StorageCapability>,
    pub peer_manager: Box<dyn Peer_Management_Capability>,
    pub session: Box<dyn Session_Capability>,
    pub event_bus: Arc<EventBus>,
}
```

6 个字段 → **5 个字段**。所有字段使用各组件最新的 Reforge 后接口。

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

    device_preference: String,
    scheduler_strategy_preference: Scheduler_Strategy,
}
```

> 注：`Scheduler_Strategy` 和 `scheduler_strategy_preference` 已移除。

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
    /// 通用 Lua 脚本执行（未来统一入口）
    Execute { command, params, reply },
    // ─── 过渡期硬编码命令（未来迁移到 Lua 脚本） ───
    Run { script, model_path, reply },
    Cancel { job_id, reply },
    Quit { reply },
    DisplayPeer { reply },
    SetDevice { device, reply },
    DistributeModel { model_path, peers, reply },
    List { reply },
    Send { file_path, peer_id, reply },
    Profile { model_id, reply },
}
```

10 个变体（1 个通用 Execute + 4 个系统命令 + 5 个过渡命令）。

### 3.6 NetworkProtocol

```rust
pub enum NetworkProtocol {
    Establish_Tensor_Stream { inference_id, target_peer_id },
    Join_Pipeline { inference_id, model_file_id, device, layer_start, layer_end },
    Profile_Request { model_id, layer_count, device },
}
```

文本协议（`|` 分隔），通过 `Parse_Network_Command` / `Serialize_Network_Command` 解析和序列化。

---

## 4. 事件循环

### 4.1 4 分支 select!

```rust
pub async fn run(mut self) {
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
        }
    }
}
```

| # | 通道 | 守卫 | 方法 | 当前状态 |
|---|------|------|------|---------|
| B1 | user_cmd_rx | !shutting_down | route_user() | 部分 Capability 调用, 推理 todo!() |
| B2 | inbound_rx | !shutting_down | route_inbound() | todo!() |
| B3 | network_inbound_rx | !shutting_down | route_stream() | todo!() |
| B4 | lifecycle_rx | 无 | route_lifecycle() | ✅ 实现 |

### 4.2 route_user() 命令状态

| 命令 | 状态 | 说明 |
|------|------|------|
| Execute | todo!() | **未来统一入口**: ProgramRegistry 查表 → Lua execute(params, caps) |
| Run | todo!() | 过渡: analyze → session → Lua 脚本 (将迁移到 Execute) |
| Cancel | ✅ | cancel_job + reply |
| Quit | ✅ | shutdown + reply |
| DisplayPeer | ✅ | peer_manager.Get_Peers() |
| SetDevice | ✅ | 更新 device_preference |
| DistributeModel | todo!() | 过渡: analyze + split + send (将迁移到 Execute) |
| List | ✅ | storage.list() |
| Send | todo!() | 过渡: send file to peer (将迁移到 Execute) |
| Profile | todo!() | 过渡: 模型性能测试 (将迁移到 Execute) |

### 4.3 Job 管理

```rust
fn register_job(&mut self, job_id: JobId, kind: JobKind);
fn cancel_job(&mut self, job_id: JobId);
fn shutdown(&mut self);
```

不再有 `spawn_job(instructions)` — VM 指令序列已移除。未来 Job 由 Lua 线程或直接 Capability 调用替代。

---

## 5. 与外部模块的接口

### 5.1 已集成的 Capability 调用

| 模块 | 调用示例 | 使用位置 |
|------|---------|---------|
| Peer_Management | `peer_manager.Get_Peers().await` | route_user DisplayPeer |
| Storage | `storage.list().await` | route_user List |

### 5.2 待集成的 Capability 调用 (todo!())

| 模块 | 方法 | 用途 |
|------|------|------|
| ML Engine (自由函数) | `analyze_model(path).await` | Run/DistributeModel |
| ML Engine (自由函数) | `split_model(src, start, end, out_dir).await` | DistributeModel |
| Session | `session.create_session(model_id).await` | Run |
| Network | `network.send_data(peer, dt, payload).await` | DistributeModel |
| Network | `network.open_file_stream(peer).await` | Send/DistributeModel |

### 5.3 ML Engine 特殊说明

ML Engine 不再以 trait object 形式存在于 Capabilities 中：

- **推理方法** (`load_model`/`forward`/`sample`/`encode`/`decode`/`tensorize`/`unload`) — 全部是 `MlSession` 的方法 (`&mut self`)，由 Lua `sess:method()` 调用
- **独立函数** (`analyze_model`/`split_model`) — 普通 async 函数，Orchestrator 直接 `use crate::ml_engine::{analyze_model, split_model}` 后 `await` 调用

---

## 6. 与旧架构对比

| 维度 | 旧（Reforge 前） | 新（Reforge 后） |
|------|----------------|-----------------|
| Capabilities 字段数 | 8 | **5** |
| ML Engine 持有方式 | `Box<dyn ML_Engine_Capability>` (trait 未定义) | 不在 Capabilities 中 |
| 指令系统 | Orchestrator_VM 17 条指令 + match 分发 | 直接方法调用 |
| 策略模板 | ProgramSelector 920 行 TOML→指令 | todo!()（未来 Lua） |
| IO 通道 | Arc\<LLM_IO_Broker\> (类型不存在) | Box\<dyn Session_Capability\> |
| Tensor 流 | Arc\<Tensor_Port_Switch\> (类型不存在) | Network_Capability 内置 |
| Job 执行 | JobExecutor + Orchestrator_VM step() 循环 | JobExecutor stub |
| 编译依赖 | 依赖 Vm_Base + ML_VM + TOML | 不依赖 VM 体系 |
| 编译状态 | ❌ 37 错误 (trait 未定义等) | ⚠️ 0 Orchestrator 错误 (外部模块有预存错误) |

---

## 7. 已删除的代码

| 删除项 | 行数 | 原因 |
|--------|------|------|
| `Orchestrator_VM/` (7 文件) | ~1100 | VM 指令分发不再需要 |
| `program_selector.rs` | ~920 | TOML 模板不再需要 |
| **合计** | **~2020** | |

## 8. 已知限制

| 限制 | 说明 |
|------|------|
| 推理未实现 | Run/Profile 全部 todo!()，等待 Lua 层 |
| 网络命令未实现 | B2/B3 全部 todo!()，等待后续接入 |
| 模型分发未实现 | DistributeModel/Send todo!() |
| test_utils stub 使用旧方法名 | StubPeerManager 中 `List_Peers` 等方法已更新为最新 trait |
| 外部模块有预存错误 | Scheduler/Network/TUI 等模块与最新 trait 不完全兼容 |
| SessionManager 缺 Take_Frontend | TUI `run` 命令用 `JobId` 取 `IoFrontend`，但 SessionManager 索引体系是 `session_id`。当前 `Take_Frontend(job_id)` 为 stub（返回错误），需重构为 Core 返回 session_id → TUI 调 `connect(session_id)` 的完整流程 |

---

## 9. 状态

### ✅ 已完成

- Capabilities 结构体重写 (6 字段，使用最新接口)
- Core 事件循环 (4 分支 select!)
- UserCommand 全部 10 个变体定义 + 路由骨架
- NetworkProtocol 3 个变体 + 解析/序列化
- JobKind 精简 + JobId/JobState/JobResult/LifecycleEvent
- DisplayPeer / SetDevice / List / Cancel / Quit 实现
- JOIN_PIPELINE 重复 arm bug 修复
- test_utils 更新为最新 trait 方法
- Orchestrator_VM + program_selector 删除

### ⚠ todo!() 占位

- Run / Profile / DistributeModel / Send 命令
- B2 入站请求处理 (Establish_Tensor_Stream / Join_Pipeline)
- B3 Stream 事件处理 (FileStreamArrived / TensorStreamArrived)
- JobExecutor 当前为 stub (立即返回 Success)
