# Orchestrator Reforge

> Presented by KeJi
> Date: 2026-05-16

## 0. 核心定义

> **Orchestrator = 策略路由 + 组件编排。**
> 不负责模型推理（ML Engine 的事）、不负责网络传输（Network 的事）、
> 不负责文件管理（Storage 的事）、不负责节点管理（PeerManagement 的事）、
> 不负责调度计算（Scheduler 的事）。只做**命令路由**和**组件编排**。

---

## 1. 背景

### 1.1 当前架构

```
用户命令 (TUI/CLI)
    │
    ▼
Core::run() ─── tokio::select! 4分支
    ├── B1: UserCommand     (9 变体)
    ├── B2: InboundRequest  (Command + File)
    ├── B3: Network_Inbound (FileStream + TensorStream)
    └── B4: LifecycleEvent  (Job Done)
    │
    ▼
ProgramSelector ─── include_str! 嵌入 TOML 模板 → 指令序列 (920行)
    │
    ▼
Orchestrator_VM ─── 17 条指令, step() match 分发 (1100行)
    │
    ▼
Capabilities ─── 8 个 trait objects
    │
    ├── Box<dyn ML_Engine_Capability>    ← ❌ trait 未定义
    ├── Box<dyn Network_Capability>      ← ✅ 13 方法
    ├── Box<dyn Peer_Management_Capability> ← ✅ 11 方法
    ├── Box<dyn Scheduler_Capability>    ← ✅ 1 方法
    ├── Arc<StorageManager>              ← ✅ 7 方法
    ├── Arc<EventBus>                    ← ✅ Publish/Subscribe
    ├── Arc<LLM_IO_Broker>               ← ❌ 类型不存在
    └── Arc<Tensor_Port_Switch>          ← ❌ 类型不存在
```

### 1.2 各组件 Reforge 状态

| 组件 | Reforge 状态 | 新接口形式 |
|------|-------------|-----------|
| ML Engine | ✅ 已完成 | `MlSession` UserData (mlua), 2 个独立 async 函数 (analyze/split), 无 trait |
| Network | ✅ 已完成 | `Network_Capability` trait, 13 个 async 方法 |
| Storage | ✅ 已完成 | `StorageCapability` trait, 7 个 async 方法 |
| PeerManagement | ✅ 已完成 | `Peer_Management_Capability` trait, 11 个 async 方法 |
| Scheduler | ✅ 已完成 | `Scheduler_Capability` trait, 1 个 async 方法 |
| Session_Manager | ✅ 已完成 | `Session_Capability` trait, 5 个 async 方法 |
| EventBus | ✅ 已完成 | 实体类型, Publish/Subscribe |
| Tensor Stream | ✅ 已完成 | 已回归 `Network_Capability` (open/accept_tensor_stream) |
| **Orchestrator** | ❌ 未 Reforge | 仍使用旧的 VM + TOML + 未定义 trait |

### 1.3 核心问题

1. **`ML_Engine_Capability` trait 不存在** — 全代码库引用但从未定义。ML Engine Reforge 后改为 `MlSession` UserData（Lua sess:method() 方式调用），推理方法全部 `&mut self`，不适合 trait object
2. **`LLM_IO_Broker` / `Tensor_Port_Switch` 类型不存在** — 已分别迁移到 `Session_Capability` 和 `Network_Capability`，但 Orchestrator 仍引用旧类型
3. **Orchestrator_VM 冗余** — 17 条指令通过 match 分发到 handler，本质是手工实现的方法调用
4. **ProgramSelector 冗余** — 920 行 TOML→指令转换，TOML 模板无控制流，策略编写困难

---

## 2. 目标架构

```
用户命令 (TUI/CLI)
    │
    ▼
Core::run() ─── tokio::select! 4分支 (保留)
    ├── B1: UserCommand     → route_user()
    ├── B2: InboundRequest  → route_inbound()
    ├── B3: Network_Inbound → route_stream()
    └── B4: LifecycleEvent  → route_lifecycle()
    │
    ▼
Core 方法 ─── 直接调用 Capability 方法, 业务逻辑 todo!() 占位
    │
    ▼
Capabilities (新) ─── 所有组件的最新接口
    │
    ├── (模型分析/切分: 直接调用 analyze_model / split_model 自由函数)
    ├── network: Box<dyn Network_Capability>
    ├── storage: Arc<dyn StorageCapability>
    ├── peer_manager: Box<dyn Peer_Management_Capability>
    ├── scheduler: Box<dyn Scheduler_Capability>
    ├── session: Box<dyn Session_Capability>
    └── event_bus: Arc<EventBus>
```

> **注意**: `MlSession` (模型推理) 不放在 Capabilities 中。它是 `mlua::UserData`，
> 由未来 Lua 层通过 `sess:method()` 调用，Orchestrator 不直接持有。

### 2.1 职责边界

| 职责 | 负责方 | 说明 |
|------|--------|------|
| 命令路由 | Orchestrator Core | 解析 UserCommand/InboundRequest，分发到对应 handler |
| 组件编排 | Orchestrator Core | 组合调用各 Capability，按正确顺序编排 |
| 策略/控制流 | **未来 Lua** | 本次不实现，用 todo!() 占位 |
| 模型推理 | ML Engine → **Lua** | `MlSession` UserData，Lua `sess:forward()/sample()` |
| 模型分析/切分 | ML Engine → **Orchestrator** | `analyze_model()` / `split_model()` 自由 async 函数 |
| 网络传输 | Network | `Network_Capability` trait |
| 文件管理 | Storage | `StorageCapability` trait |
| 节点管理 | PeerManagement | `Peer_Management_Capability` trait |
| 调度计算 | Scheduler | `Scheduler_Capability` trait |
| 会话/IO | Session_Manager | `Session_Capability` trait |
| 事件通知 | EventBus | Publish/Subscribe |

---

## 3. 新 Capabilities 结构体

```rust
/// 组件能力容器 — 仅持有最新接口
pub struct Capabilities {
    /// 网络通信
    pub network: Box<dyn Network_Capability>,
    /// 文件存储
    pub storage: Arc<dyn StorageCapability>,
    /// 节点管理
    pub peer_manager: Box<dyn Peer_Management_Capability>,
    /// 会话管理 (IO 通道)
    pub session: Box<dyn Session_Capability>,
    /// 事件总线
    pub event_bus: Arc<EventBus>,
}
```

> **ML Engine 不在 Capabilities 中**：
> - 推理方法 (`load_model`/`forward`/`sample`/...) 是 `MlSession` 的方法 (`&mut self`)，
>   不适合 trait object。由未来 Lua 层通过 `mlua::UserData` 调用。
> - 分析/切分 (`analyze_model`/`split_model`) 是独立 async 函数，
>   Orchestrator 直接 `use ml_engine::{analyze_model, split_model}` 调用。

### 3.1 vs 旧 Capabilities 对照

| 旧字段 | 新字段 | 说明 |
|--------|--------|------|
| `ml_engine: Box<dyn ML_Engine_Capability>` | **(删除)** | 推理 → 未来 Lua; 分析/切分 → 自由函数 |
| `network: Box<dyn Network_Capability>` | 同 | 保留 |
| `storage: Arc<StorageManager>` | `storage: Arc<dyn StorageCapability>` | 具体类型 → trait object |
| `peer_manager: Box<dyn Peer_Management_Capability>` | 同 | 保留 |
| `scheduler: Box<dyn Scheduler_Capability>` | **(删除)** | 本阶段不需要调度器 |
| `io_broker: Arc<LLM_IO_Broker>` | `session: Box<dyn Session_Capability>` | 替换为 Session_Capability |
| `tensor_switch: Arc<Tensor_Port_Switch>` | **(删除)** | 已回归 Network_Capability |
| `event_bus: Arc<EventBus>` | 同 | 保留 |

---

## 4. 删除清单

> ⚠️ **作用域限制**：本次只修改 `Src/Orchestrator/` 目录内的文件。
> `Src/Vm_Base/`、`Src/ML_Engine/ML_VM/`、`programs/*.tmpl` 等外部文件**不动**，
> 仅通过删除 Orchestrator 对它们的引用使编译通过。

### 4.1 删除的 Orchestrator 文件

| 删除项 | 行数 | 原因 |
|--------|------|------|
| `Src/Orchestrator/Orchestrator_VM/` 全部 | ~1100 | VM 指令分发 → 直接方法调用 |
| `Src/Orchestrator/program_selector.rs` | ~920 | TOML 模板 → 未来 Lua (todo!()) |

### 4.2 删除的指令/类型

| 删除项 | 位置 | 原因 |
|--------|------|------|
| `OrchestratorInstruction` (17 变体) | `Orchestrator_VM/instruction.rs` | 不再需要指令枚举 |
| `OrchestratorSlotValue` (7 变体) | `Orchestrator_VM/slots.rs` | 不再需要槽位系统 |
| `Orchestrator_VM` (struct) | `Orchestrator_VM/engine.rs` | 不再需要 VM 引擎 |
| `ProgramSelector` (struct) | `program_selector.rs` | 不再需要 TOML 编译 |
| `JobExecutor` (旧) | `core/job_executor.rs` | 重写为直接 Capability 调用 |
| `JobKind` (精简) | `job.rs` | 移除 Run/Coordinator/Relay/Pipeline 等 VM 相关变体 |

---

## 5. 保留并修改的文件

> 以下仅列出 `Src/Orchestrator/` 目录内的文件。**不修改** `Src/lib.rs`、`Src/main.rs`、
> `Src/Vm_Base/`、`Src/ML_Engine/` 等其他模块。

| 文件 | 操作 | 说明 |
|------|------|------|
| `Src/Orchestrator/mod.rs` | ✏️ 重写 | 新 Capabilities 结构体（6 字段）+ test_utils stub |
| `Src/Orchestrator/core.rs` | ✏️ 重写 | select! 4 分支保留，route_* 改为直接调用 capability |
| `Src/Orchestrator/command.rs` | ✏️ 修改 | UserCommand 保留，移除 VM 相关类型引用，修复 JOIN_PIPELINE 重复 arm |
| `Src/Orchestrator/job.rs` | ✏️ 修改 | JobKind 精简，JobId 保留 |
| `Src/Orchestrator/core/branch_user.rs` | ✏️ 重写 | 每个命令分支 → todo!() 占位 |
| `Src/Orchestrator/core/branch_command.rs` | ✏️ 重写 | InboundRequest 处理 → todo!() 占位 |
| `Src/Orchestrator/core/branch_stream.rs` | ✏️ 重写 | Stream 事件处理 → todo!() 占位 |
| `Src/Orchestrator/core/branch_lifecycle.rs` | ✏️ 重写 | 生命周期处理 |
| `Src/Orchestrator/core/job_executor.rs` | ✏️ 重写 | 简化为最小 stub |
| `Src/Orchestrator/inference_id.rs` | ✅ 保留 | 全局 ID 生成 |

---

## 6. Core 新结构

### 6.1 Core struct

```rust
pub struct Core {
    /// Job 注册表
    registry: HashMap<JobId, JobHandle>,
    /// 关闭标志
    shutting_down: bool,

    /// 组件能力
    capabilities: Arc<Capabilities>,

    // ──── 4 通道 ────
    /// B1: 用户命令（TUI → Core）
    user_cmd_rx: mpsc::Receiver<UserCommand>,
    /// B2: Request-Response 入站请求（Network → Core）
    inbound_rx: mpsc::Receiver<InboundRequest>,
    /// B3: Stream 入站事件（Network → Core）
    network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,
    /// B4: 生命周期事件（Job → Core）
    lifecycle_rx: mpsc::Receiver<LifecycleEvent>,

    /// 生命周期发送端（spawn 时传给 Job）
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,

    /// 设备偏好
    device_preference: String,
    /// 调度策略偏好
    scheduler_strategy_preference: Scheduler_Strategy,
}
```

### 6.2 route_user() 命令分支 (B1)

每个分支不实现具体业务逻辑，仅调用 Capability 方法组合，无法完成的部分用 `todo!()`：

```rust
async fn route_user(&mut self, cmd: UserCommand) {
    match cmd {
        // 单机推理 (未来: 启动 Lua 线程, 脚本调用 sess:forward/sample)
        UserCommand::Run { model_path, reply } => {
            // 1. analyze_model(model_path).await → Model_Arch_Info
            // 2. session.create_session(model_id).await → (session_id, IoHandle)
            // 3. 启动 Lua 线程: ml.load_model → sess:encode → sess:forward → sess:sample → ...
            //    (todo!() — 本次不做 Lua 绑定)
            let _ = reply.send(Err("Run: not yet implemented".into()));
        }

        // 分布式推理 (未来: 启动 Coordinator Lua 线程编排)
        UserCommand::Pipeline { model_path, strategy, reply } => {
            // 1. analyze_model(model_path).await → Model_Arch_Info
            // 2. peer_manager.Get_Peers().await → 可用节点
            // 3. scheduler.Plan_Pipeline(input).await → Pipeline_Plan
            // 4. 分发模型分片 (todo!())
            // 5. 建立 tensor stream (network.open/accept_tensor_stream)
            // 6. 启动 Coordinator Lua 线程 (todo!())
            let _ = reply.send(Err("Pipeline: not yet implemented".into()));
        }

        // 取消作业
        UserCommand::Cancel { job_id, reply } => {
            todo!("Cancel")
        }

        // 优雅退出
        UserCommand::Quit { reply } => {
            self.shutting_down = true;
            let _ = reply.send(());
        }

        // 查看节点
        UserCommand::DisplayPeer { reply } => {
            match self.capabilities.peer_manager.Get_Peers().await {
                Ok(peers) => {
                    let list: Vec<String> = peers.iter()
                        .map(|p| format!("{} [{:?}]", p.peer_id, p.status))
                        .collect();
                    let _ = reply.send(Ok(list));
                }
                Err(e) => {
                    let _ = reply.send(Err(format!("{}", e)));
                }
            }
        }

        // 设置设备
        UserCommand::SetDevice { device, reply } => {
            self.device_preference = device;
            let _ = reply.send(Ok(()));
        }

        // 模型分发 (直接调用 ML Engine 自由函数 analyze_model + split_model)
        UserCommand::DistributeModel { model_path, peers, reply } => {
            // 1. analyze_model(&Path::new(&model_path)).await → Model_Arch_Info
            // 2. split_model(&Path::new(&model_path), start, end, &output_dir).await
            // 3. storage.acquire_read → network.send_data (元数据协商)
            // 4. network.open_file_stream → send_file_data
            let _ = reply.send(Err("DistributeModel: not yet implemented".into()));
        }

        // 列出模型
        UserCommand::List { reply } => {
            match self.capabilities.storage.list().await {
                Ok(entries) => {
                    let list: Vec<String> = entries.iter()
                        .map(|e| format!("{} ({})", e.file_name, e.size))
                        .collect();
                    let _ = reply.send(Ok(list));
                }
                Err(e) => {
                    let _ = reply.send(Err(format!("{}", e)));
                }
            }
        }

        // 发送文件
        UserCommand::Send { file_path, peer_id, reply } => {
            todo!("Send")
        }

        // 性能测试
        UserCommand::Profile { model_id, reply } => {
            todo!("Profile")
        }
    }
}
```

### 6.3 route_inbound() (B2)

```rust
async fn route_inbound(&mut self, req: InboundRequest) {
    match req.data_type {
        DataType::Command => {
            // 解析 NetworkProtocol → 分发
            todo!("Inbound Command routing")
        }
        DataType::File => {
            // 文件元数据协商 → pending_file_receives
            todo!("Inbound File metadata")
        }
        _ => {} // Network 内部已处理
    }
}
```

### 6.4 route_stream() (B3)

```rust
async fn route_stream(&mut self, event: Network_Inbound_Event) {
    match event {
        Network_Inbound_Event::FileStreamArrived { peer, stream } => {
            todo!("FileStreamArrived")
        }
        Network_Inbound_Event::TensorStreamArrived { peer, stream } => {
            // network.accept_tensor_stream 已在 Network_Service 内部处理 rendezvous
            todo!("TensorStreamArrived")
        }
    }
}
```

### 6.5 route_lifecycle() (B4)

```rust
fn route_lifecycle(&mut self, event: LifecycleEvent) {
    match event {
        LifecycleEvent::Done { job_id, result } => {
            self.event_bus.Publish(Bus_Event::Job_Completed { job_id, result });
            self.registry.remove(&job_id);
        }
    }
}
```

---

## 7. UserCommand 精简

```rust
pub enum UserCommand {
    Run {
        model_path: String,
        reply: oneshot::Sender<Result<JobId, String>>,
    },
    Cancel {
        job_id: JobId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Quit {
        reply: oneshot::Sender<()>,
    },
    DisplayPeer {
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    SetDevice {
        device: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    DistributeModel {
        model_path: String,
        peers: Vec<(String, usize, usize)>,   // (peer_id, layer_start, layer_end)
        reply: oneshot::Sender<Result<JobId, String>>,
    },
    List {
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    Send {
        file_path: String,
        peer_id: String,
        reply: oneshot::Sender<Result<JobId, String>>,
    },
    Pipeline {
        model_path: String,
        strategy: Option<String>,
        reply: oneshot::Sender<Result<JobId, String>>,
    },
    Profile {
        model_id: String,
        reply: oneshot::Sender<Result<JobId, String>>,
    },
}
```

> 变体保持不变，仅确保不再依赖 VM 相关类型。

---

## 8. JobKind 精简

```rust
pub enum JobKind {
    /// 通用任务 (todo!() 占位)
    Generic,
    /// 文件接收 (Network → Storage)
    ReceiveFile,
}
```

> 原 `Run`/`Coordinator`/`Relay`/`Pipeline`/`Distribute`/`Send`/`Profile` → 全部合并为 `Generic`，
> 业务逻辑由 Core 的 route_* 方法直接编排，不再需要 JobKind 区分。

---

## 9. 实施计划

> **作用域**: 仅修改 `Src/Orchestrator/` 目录。不动 `main.rs`、`lib.rs`、`Vm_Base/`、`ML_Engine/` 等外部模块。

### Phase 1：删除 Orchestrator 内的旧 VM 体系

| # | 操作 | 文件 |
|---|------|------|
| 1.1 | 🗑️ 删除 | `Src/Orchestrator/Orchestrator_VM/` 全部 (7 文件) |
| 1.2 | 🗑️ 删除 | `Src/Orchestrator/program_selector.rs` |

### Phase 2：重写 Capabilities

| # | 操作 | 文件 |
|---|------|------|
| 2.1 | ✏️ 重写 | `Src/Orchestrator/mod.rs` — 新 Capabilities 结构体（6 字段，不含 ML Engine） |
| 2.2 | ✏️ 重写 | `Src/Orchestrator/job.rs` — JobKind 精简 |

### Phase 3：重写 Core

| # | 操作 | 文件 |
|---|------|------|
| 3.1 | ✏️ 重写 | `Src/Orchestrator/core.rs` — Core struct + run() select! 循环 |
| 3.2 | ✏️ 重写 | `Src/Orchestrator/core/branch_user.rs` — route_user() 全部 todo!() 占位 |
| 3.3 | ✏️ 重写 | `Src/Orchestrator/core/branch_command.rs` — route_inbound() todo!() 占位 |
| 3.4 | ✏️ 重写 | `Src/Orchestrator/core/branch_stream.rs` — route_stream() todo!() 占位 |
| 3.5 | ✏️ 重写 | `Src/Orchestrator/core/branch_lifecycle.rs` — route_lifecycle() |
| 3.6 | ✏️ 重写 | `Src/Orchestrator/core/job_executor.rs` — 简化为最小 stub |
| 3.7 | ✏️ 修改 | `Src/Orchestrator/command.rs` — 修复 JOIN_PIPELINE 重复 arm |

### Phase 4：验证

| # | 操作 | 说明 |
|---|------|------|
| 4.1 | ✅ 验证 | `cargo check -p pleiades` — 确保 Orchestrator 内部编译通过 |

### Phase 5：编写设计文档

| # | 操作 | 文件 |
|---|------|------|
| 5.1 | 🆕 新建 | `docs/orchestrator_design.md` — 重构后的 Orchestrator 正式设计文档 |

---

## 10. 不变的内容

> 以下模块**完全不修改**：

| 模块 | 说明 |
|------|------|
| `Src/ML_Engine/` 全部 | 不动 |
| `Src/Network/` 全部 | 不动 |
| `Src/Storage/` 全部 | 不动 |
| `Src/PeerManagement/` 全部 | 不动 |
| `Src/Scheduler/` 全部 | 不动 |
| `Src/Session_Manager/` 全部 | 不动 |
| `Src/EventBus/` 全部 | 不动 |
| `Src/Lua/` 全部 | 不动 |
| `Src/Config/` 全部 | 不动 |
| `Src/TUI/` 全部 | 不动 |
| `Src/Vm_Base/` 全部 | 不动（本次不删除） |
| `Src/lib.rs` | 不动 |
| `Src/main.rs` | 不动 |
| `programs/*.tmpl` | 不动（本次不删除） |
| `Src/Orchestrator/inference_id.rs` | 保留 |
| `Src/Orchestrator/command.rs` (NetworkProtocol) | 保留，仅修 bug |

---

## 11. 设计原则

1. **只做集成，不做业务**：所有 `UserCommand` 分支用 `todo!()` 占位，只验证 Capability 调用通路正确
2. **不做 Lua 绑定**：`Src/Lua/` 完全不动，`MlSession` 推理方法不在此次集成范围
3. **直接调用，不走 VM**：删除指令枚举/match 分发/槽位系统，改为直接方法调用传参
4. **编译通过为第一目标**：删除所有引用旧类型的代码，新代码只验证类型正确
5. **ML Engine 特殊处理**：`analyze_model`/`split_model` 作为自由函数直接 `await`；`MlSession` 推理留给 Lua 层，Orchestrator 不持有
6. **Capability 使用最新接口**：TensorStream → Network_Capability, IO → Session_Capability

---

## 12. 预期删除代码量

| 类别 | 文件 | 行数 |
|------|------|------|
| Orchestrator_VM | instruction.rs + engine.rs + slots.rs + 3 handler + mod.rs | ~1100 |
| program_selector | 单文件 | ~920 |
| core/branch_*.rs | 重写简化 | ~400 |
| **合计 (Orchestrator 内部)** | | **~2420** |

> 外部文件 (`Vm_Base/` ~780, `ML_VM/` ~930, TOML 模板 ~300) 本次不动。

---

## 13. 讨论项

| # | 问题 | 结论 |
|---|------|------|
| 1 | MlSession 放在 Capabilities 里吗？ | 不。MlSession 是 `mlua::UserData`，方法都是 `&mut self`，不适合 trait object。推理由未来 Lua 层调用 |
| 2 | ML_Engine_Capability trait 还需要吗？ | 不需要。推理用 `MlSession` UserData，分析/切分用自由函数 |
| 3 | Orchestrator 如何调用模型推理？ | 本次不做。推理全部 todo!()。未来通过 Lua 脚本 `sess:forward()` 调用 |
| 4 | LLM_IO_Broker 替代方案？ | 使用 Session_Capability (create_session → IoHandle) |
| 5 | Tensor_Port_Switch 替代方案？ | Network_Capability 已内置 open/accept_tensor_stream + RendezvousMap |
| 6 | 旧 Orchestrator_VM handler 中的业务逻辑如何迁移？ | 不迁移。所有业务逻辑用 todo!() 占位，后续统一用 Lua 实现 |
| 7 | Job/JobExecutor 概念保留吗？ | 保留 JobId + registry，但 JobExecutor 简化为最小 stub |
