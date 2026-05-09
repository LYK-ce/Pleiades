# Pleiades Orchestrator — 对话上下文恢复文档

**版本**：Phase 2 进行中 + LLM_IO 模块已完成 + StorageManager 设计完成  
**日期**：2026-04-23  
**状态**：TaskEngine/JobExecutor 冒烟通过，Handler 部分填充，LLM_IO 刚实现待集成，StorageManager 待编码

---

## 1. 架构总览（已收敛）

```
┌─────────────────────────────────────────────┐
│              外部世界（CLI / Network）         │
│         TUI 模式    /    API 模式            │
└──────────┬──────────────────────┬─────────────┘
           │                      │
           ▼                      ▼
┌─────────────────────┐  ┌──────────────────────┐
│   LLM_IO 模块         │  │   PeerManager        │
│  (input_tx/output_rx)│  │  （Network 内部）      │
│  通道工厂，不进入内核  │  └──────────────────────┘
└──────────┬──────────┘           │
           │                      ▼（不上报 Core）
           │            ┌──────────────────────┐
           │            │   Command Portal        │
           │            │  （Core 内部路由层）      │
           │            └──────────┬───────────┘
           │                       │
           ▼                       ▼
    ML Thread（直传）      ┌──────────────────────┐
    （Session Thread）      │   Orchestrator Core   │
                           │  • 作业注册表          │
                           │  • spawn / cancel     │
                           │  • 生命周期管理        │
                           └──────────┬──────────┘
                                      │
                                      ▼
                           ┌──────────────────────┐
                           │    JobExecutor        │
                           │  （统一执行框架）       │
                           │  • TaskEngine（私有）  │
                           │  • 状态机驱动          │
                           └──────────┬───────────┘
                                      │
                                      ▼
                           ┌──────────────────────┐
                           │    Capabilities       │
                           │  • Network            │
                           │  • Inference          │
                           │  • StorageManager     │
                           │  • ResourceManager    │
                           └──────────────────────┘
```

---

## 2. 关键设计决策（不可回退）

### 2.1 Core 与 Portal
- **Portal 已退化为 Core 的私有方法层**，不是独立结构体。
- Core 的 `run()` 通过 `select!` 接收 `UserCommand` / `NetworkCommand`，内部调用 `route_user()` / `route_network()`（**同步方法，无 await**）。
- 查询/配置类命令（`DisplayPeer`、`SetDevice`）由 Portal 直接调用 Capability，**不进入 Core 内核**。

### 2.2 Input 数据流
- **Input/Prompt 不经过 Core、不经过 JobExecutor**，由前端通过 LLM_IO 模块直传 ML Thread。
- `JobExecutor` **无 `cmd_rx`**，`select!` 仅监听 Cancel 与 `step()` 两路。

### 2.3 网络事件
- `PeerDiscovered` / `PeerLeft` / `ConnectionEstablished` / `ConnectionClosed` **不上报 Core**，由 Network Capability 内部的 `PeerManager` 处理。
- Core 仅在 `DisplayPeer` 或 `DiscoverPeers` 时主动查询 `PeerManager` 快照。

### 2.4 文件接收
- 入站文件传输**不独立 spawn Job**，作为 `WorkerRelay` 作业的 `ReceiveFile` 内部步骤执行。

### 2.5 TaskProgram 编译前置
- **Compiler 由 Portal 调用**，生成 `TaskProgram`（正向 + 补偿 + 标签表）后传给 Core。
- Core 和 JobExecutor 只消费成品，不编译。

### 2.6 IO 通道生命周期
- **IO 通道由 Core 在 spawn 前申请**（通过 LLM_IO_Broker），作为参数传入 JobExecutor。
- TaskEngine 执行 `CreateSession` 时从 SlotFile 取出绑定到 ML Thread。

### 2.7 TaskEngine IP 管理范式
- **先默认 `ip += 1`**，所有指令执行前指针前进。
- `JumpIf` 条件为真时，**覆盖 `ip = target`**；条件为假时自然顺序执行下一条。
- 绝不能用 `if !matches!(instr, JumpIf)` 排除自增，否则假分支会死循环。

### 2.8 TaskEngine 生命周期
- TaskEngine 是**单次 Job 的临时解释器**，Job 结束后随 Executor 销毁，不会复用。
- `load()` 不需要重置 `mode`（新实例默认 Forward），也不需要清理旧 slots（实例已销毁）。

### 2.9 存储模型
- **放弃每个 Job 单独 workspace 的设计**，改为共享 workspace。
- `EnsureWorkspace` / `CleanupWorkspace` 指令**待删除**（Phase 2 不再需要）。
- StorageManager 提供统一扁平命名空间，文件级 RwLock 并发控制。

### 2.10 对话历史
- **对话历史由 ML Thread 内部通过 KV Cache 维护**（Candle 支持）。
- IO 层（LLM_IO）**不感知、不维护**对话历史，只负责单轮文本转发。
- 回退/编辑操作通过截断 KV Cache 实现，由 ML Thread 内部处理。

### 2.11 LLM_IO 定位
- LLM_IO 是**通道工厂**，不是给 TaskEngine 调用的 Capability。
- 它作为**独立服务**被 Core 和前端共享，**不进入 `executor::Capabilities`**。
- 前端二选一（TUI 或 API），同一时刻只存在一个输入源。

---

## 3. 模块目录结构（Phase 2 当前）

```
src/
├── orchestrator/
│   ├── mod.rs
│   ├── slot.rs              // SlotId + SlotValue + SlotFile（公共契约）
│   ├── instruction.rs       // TaskInstruction + TaskProgram（公共契约）
│   ├── job.rs               // JobId + JobKind + JobState + LifecycleEvent
│   ├── compiler.rs          // Compiler + TaskProgramBuilder（私有）
│   ├── core.rs              // OrchestratorCore + Portal 路由方法
│   ├── command.rs           // UserCommand + NetworkCommand
│   └── executor/
│       ├── mod.rs           // JobExecutor + Capability Traits
│       ├── task_engine.rs   // TaskEngine 结构体 + step() match 骨架
│       ├── handler_data.rs      // Const, Move — ✅ 已实现 + 测试通过
│       ├── handler_storage.rs   // EnsureWorkspace, CleanupWorkspace — ⚠️ 待删除
│       ├── handler_compute.rs   // AcquireDevice — ⚠️ 空壳
│       ├── handler_inference.rs // CreateSession, ShutdownSession — ⚠️ 空壳
│       └── handler_control.rs   // JumpIf, Abort — ✅ 已实现 + 测试通过
│
├── storage/
│   ├── mod.rs              // 模块入口
│   ├── capability.rs       // StorageCapability trait（通用文件存储）
│   ├── manager.rs          // StorageManager 实现（待编码）
│   └── handle.rs           // ReadHandle / WriteHandle
│
├── llm_io/
│   ├── mod.rs              // 模块入口 + 集成测试
│   ├── capability.rs       // LLM_IO_Capability trait + IoChannels/IoFrontend/IoHandle
│   └── broker.rs           // LLM_IO_Broker 实现 — ✅ 已实现，待命名修复
│
└── ...（其他领域模块）
```

---

## 4. Slot 系统（已实现）

### 4.1 核心类型
```rust
pub struct SlotId(pub u32);

pub enum SlotValue {
    Nil,
    Bool(bool),
    U64(u64),
    String(String),
    PathBuf(PathBuf),
    DeviceLease(DeviceLease),     // Phase 2 Stub
    SessionHandle(SessionHandle), // Phase 2 Stub
    Error(String),
    // IoHandle(IoHandle),        // ← 待添加（Phase 2 下一步）
}

pub struct SlotFile {
    slots: HashMap<SlotId, SlotValue>,
}
```

### 4.2 方法
- 通用：`set`、`get`、`take`、`remove`
- 类型安全便利方法：`get_string`、`get_path`、`get_bool`、`get_u64`、`take_device`、`take_session`、`get_error`
- **待添加**：`take_io_handle`（配合 LLM_IO 集成）

### 4.3 约束
- `set` 允许覆盖，旧值自然 Drop。
- `take` 后原槽位置 `Nil`。
- 类型不匹配返回 `Err(String)`，由 TaskEngine 捕获并 `Abort`。

---

## 5. 指令集（Phase 2，当前 9 条，待精简为 7 条）

| 类别 | 指令 | 说明 | 状态 |
|------|------|------|------|
| 数据搬运 | `Const { value, dst }` | 常量加载 | ✅ 已实现 |
| 数据搬运 | `Move { src, dst }` | 槽位拷贝 | ✅ 已实现 |
| 存储资源 | `EnsureWorkspace { job_id }` | 幂等创建隔离目录 | ⚠️ **待删除** |
| 存储资源 | `CleanupWorkspace { job_id }` | 清理目录 | ⚠️ **待删除** |
| 计算资源 | `AcquireDevice { preferred, result }` | 申请设备租约 | ⚠️ 空壳 |
| 推理会话 | `CreateSession { model, device, result }` | 创建 Session | ⚠️ 空壳，待加 `io` 参数 |
| 推理会话 | `ShutdownSession { session }` | 关闭 Session | ⚠️ 空壳 |
| 控制流 | `JumpIf { condition, label }` | 条件跳转 | ✅ 已实现 |
| 控制流 | `Abort { reason }` | 终止并触发补偿链 | ✅ 已实现 |

**待新增指令（Phase 2 下一步）**：
- `CreateSession` 增加 `io: SlotId` 参数。

---

## 6. TaskProgram 结构

```rust
pub struct TaskProgram {
    pub instructions: Vec<TaskInstruction>,   // 正向序列
    pub compensation: Vec<TaskInstruction>,  // 补偿序列
    pub labels: HashMap<String, usize>,        // 标签映射
}
```

- `instructions` 和 `compensation` 是**独立的 Vec**，不是混排。
- `labels` 仅用于 `JumpIf`，补偿序列中不使用标签。

---

## 7. TaskEngine 设计（已确认骨架）

### 7.1 结构体
```rust
pub struct TaskEngine {
    ip: usize,
    slots: SlotFile,
    program: Option<TaskProgram>,
    mode: ExecutionMode,  // Forward / Compensation
}

enum ExecutionMode {
    Forward,
    Compensation,
}
```

### 7.2 执行模式
- `Forward`：读取 `program.instructions`。
- `Compensation`：读取 `program.compensation`，`enter_compensation()` 切换并重置 `ip = 0`。

### 7.3 Handler 拆分
- `task_engine.rs` 只做 `match` 分发。
- 各 `handler_*.rs` 文件通过 `impl TaskEngine` 添加 `pub(super)` 私有方法。
- Handler 不持有独立状态，不感知 `ExecutionMode`，不修改 `ip`（`JumpIf` 除外）。

### 7.4 async 保留
- `step()` 和 `enter_compensation()` 保持 `async fn`，即使 Phase 2 内部无 await。
- 原因：Phase 3/4 的 `SendFile`、`PrepareTensorStreams` 等 handler 必然需要 `.await`，提前保留签名避免全链重构。

### 7.5 Capabilities 持有方式
- **当前已改为 TaskEngine 内部持有 `Arc<Capabilities>`**（而非 step 参数传入）。

---

## 8. Compiler 设计（已实现骨架）

- `Compiler` 无状态，不缓存。
- `compile_run` / `compile_worker_relay` 参数校验后，内部用 `TaskProgramBuilder` 构造指令序列。
- `TaskProgramBuilder` 私有，提供：`push_instruction`、`push_compensation`、`label_here`、`build`。
- Phase 2 占位：`compile_run` 和 `compile_worker_relay` 内部仍是 `todo!()`，待填充真实指令序列（需加入 IoHandle Const 指令）。

---

## 9. LLM_IO 模块（刚实现，待集成）

### 9.1 设计定位
- **大语言模型文本交互层**，负责为外部前端（TUI 或 API）与 ML Thread 之间建立有状态的双向文本通道。
- 输入：文本 Prompt（`String`），输出：文本 Completion（`String`）。
- **不是给 TaskEngine 调用的 Capability**，而是 Core 层的独立服务。

### 9.2 核心类型
```rust
pub struct IoChannels {
    pub frontend: IoFrontend,
    pub ml_side: IoHandle,
}

pub struct IoFrontend {
    pub input_tx: mpsc::Sender<String>,
    pub output_rx: mpsc::Receiver<String>,
}

pub struct IoHandle {
    pub input_rx: mpsc::Receiver<String>,
    pub output_tx: mpsc::Sender<String>,
}
```

### 9.3 Trait 定义
```rust
#[async_trait]
pub trait LLM_IO_Capability: Send + Sync {
    async fn allocate(&self, job_id: JobId) -> Result<IoChannels, LLM_IO_Error>;
    async fn deallocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error>;
    async fn is_active(&self, job_id: JobId) -> bool;
}
```

### 9.4 实现状态
- `LLM_IO_Broker` 已实现，测试已写。
- **待修复**：方法命名从 PascalCase 改为 snake_case（`New`→`new`，`Allocate`→`allocate` 等）。
- **待运行测试**：broker.rs 7 个测试 + capability.rs 3 个测试 + mod.rs 2 个测试。

---

## 10. StorageManager 设计（已完成设计文档，待编码）

### 10.1 定位
- `StorageCapability` trait 的**真实实现**（通用文件存储，非 Job 级 workspace）。
- 统一扁平命名空间，文件级 RwLock 并发控制。
- 惰性发现、实时校验码（默认 XxHash64）。

### 10.2 接口
```rust
#[async_trait]
pub trait StorageCapability: Send + Sync {
    async fn open_read(&self, file_id: &str) -> Result<ReadHandle, StorageError>;
    async fn open_write(&self, file_id: &str) -> Result<WriteHandle, StorageError>;
    async fn remove(&self, file_id: &str) -> Result<(), StorageError>;
    async fn exists(&self, file_id: &str) -> Result<bool, StorageError>;
    async fn list(&self) -> Result<Vec<String>, StorageError>;
    async fn checksum(&self, file_id: &str, algo: Option<ChecksumAlgorithm>) -> Result<String, StorageError>;
}
```

### 10.3 状态
- 设计文档已完成（`storage_manager_design.md`）。
- 代码实现待开始。

---

## 11. 边界红线（防退化）

| 红线 | 说明 |
|------|------|
| Core 不得 await 长时操作 | 只有 `select!` 的通道接收是 async，内部流转全同步 |
| TaskEngine::step() 内部禁止 `select!` | 纯顺序执行单条指令 |
| Capability 不得泄漏业务状态 | 不感知"当前是 Phase 几" |
| 所有跨步骤状态必须在 SlotFile 显式化 | 禁止全局 `Option<T>` |
| 数据面不得经过 JobExecutor / TaskEngine | Tensor/Input 零拷贝直传 ML Thread |
| JobKind 扩展必须是加法 | 新增类型不得修改旧类型的 TaskProgram 编译逻辑 |
| Job 不得直接操作 OS 资源 | 必须通过 StorageManager / ComputeManager 申请句柄/租约 |
| ComputeManager 必须提供 RAII 租约 | GPU 设备通过 `DeviceLease` 申请，Drop 自动释放 |
| StorageManager 必须提供 Workspace 隔离 | 不同 Job 的文件操作必须在不同目录 |

---

## 12. 待完成事项（Phase 2 剩余）

### 12.1 紧急（阻塞下一步）
1. **LLM_IO 命名修复**：`New`→`new`，`Allocate`→`allocate`，`Deallocate`→`deallocate`，`Is_Active`→`is_active`
2. **SlotValue 扩展**：增加 `IoHandle(IoHandle)` 变体 + `take_io_handle` 方法
3. **CreateSession 指令扩展**：增加 `io: SlotId` 参数
4. **InferenceCapability 扩展**：`create_session` 签名增加 `io: IoHandle`
5. **UserCommand::Run 扩展**：增加 `io_handle: IoHandle` 字段
6. **JobExecutor 适配**：`new()` 接收 `io_handle`，放入 SlotFile
7. **Compiler 更新**：`compile_run` 生成包含 `Const { IoHandle }` 的指令序列

### 12.2 重要（Phase 2 必须）
8. **删除 `EnsureWorkspace` / `CleanupWorkspace`**：从指令集、handler_storage.rs、Compiler 中移除
9. **填充 `handler_compute.rs`**：`AcquireDevice` 真实逻辑 + 测试
10. **填充 `handler_inference.rs`**：`CreateSession` / `ShutdownSession` 真实逻辑 + 测试
11. **StorageManager 编码**：实现 `manager.rs` + 单元测试
12. **Core 集成测试**：spawn job → 自然退出 → 收到 Done 信号

### 12.3 可选（Phase 2 收尾）
13. **TUI 原型**：验证端到端文本交互
14. **API 网关原型**：HTTP 端口模式验证
15. **Compiler 完整实现**：`compile_run` / `compile_worker_relay` 填充真实序列

---

## 13. 已废弃的设计（防止回退）

- ~~Portal 作为独立结构体~~ → 已退化为 Core 私有方法
- ~~`cmd_rx` 接收 Input~~ → 已移除，Input 直传 ML Thread
- ~~`Yield` 指令 + `Suspending` 状态~~ → 已移除
- ~~`FileReceive` 独立 Job~~ → 已合并为 WorkerRelay 内部步骤
- ~~Peer 事件上报 Core~~ → 已下沉到 PeerManager
- ~~`DisplayPeer` / `SetDevice` 进入 Core 内核~~ → 已改为 Portal 直接调用 Capability
- ~~`if !matches!(instr, JumpIf) { ip += 1 }`~~ → 已改为先默认 +1，JumpIf 覆盖
- ~~每个 Job 单独 workspace~~ → 已改为共享 workspace，删除 EnsureWorkspace/CleanupWorkspace

---

## 14. 测试状态汇总

| 模块 | 测试数 | 状态 |
|------|--------|------|
| TaskEngine 骨架 | 5 | ✅ 全部通过 |
| JobExecutor 外壳 | 3 | ✅ 全部通过 |
| handler_data.rs | 4 | ✅ 全部通过 |
| handler_control.rs | 5 | ✅ 全部通过 |
| handler_storage.rs | 4 | ✅ 全部通过（模块待删除） |
| LLM_IO capability.rs | 3 | ⏳ 待运行 |
| LLM_IO broker.rs | 7 | ⏳ 待运行 |
| LLM_IO mod.rs | 2 | ⏳ 待运行 |
| StorageManager | 0 | ⏳ 待编码 |
| Core 集成 | 0 | ⏳ 待编码 |

---

## 15. 关键代码片段（恢复用）

### 15.1 TaskEngine::step() 核心逻辑
```rust
pub async fn step(&mut self) -> StepResult {
    let program = match self.program.as_ref() {
        Some(p) => p,
        None => return StepResult::Abort("program not loaded".to_string()),
    };
    let sequence = match self.mode {
        ExecutionMode::Forward => &program.instructions,
        ExecutionMode::Compensation => &program.compensation,
    };
    if self.ip >= sequence.len() {
        return StepResult::Done;
    }
    let instr = sequence[self.ip].clone(); // clone 释放借用
    self.ip += 1;
    match instr { ... }
}
```

### 15.2 JobExecutor::run() 核心逻辑
```rust
pub async fn run(mut self) {
    self.task_engine.load(&self.program);
    let mut exit_reason = ExitReason::Success;
    loop {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => {
                exit_reason = ExitReason::Cancelled;
                self.task_engine.enter_compensation().await;
                self.run_compensation().await;
                break;
            }
            result = self.task_engine.step() => {
                match result {
                    StepResult::Continue => continue,
                    StepResult::Ready => { self.state = JobState::Ready; }
                    StepResult::Done => break,
                    StepResult::Abort(e) => {
                        self.report_error(&e).await;
                        exit_reason = ExitReason::Failed(e);
                        self.task_engine.enter_compensation().await;
                        self.run_compensation().await;
                        break;
                    }
                }
            }
        }
    }
    // 发送 LifecycleEvent + cleanup
}
```

### 15.3 LLM_IO_Broker 分配逻辑
```rust
async fn allocate(&self, job_id: JobId) -> Result<IoChannels, LLM_IO_Error> {
    let mut map = self.channels.lock().await;
    if map.contains_key(&job_id) {
        return Err(LLM_IO_Error::AllocationFailed(
            format!("job_id {:?} already has an active channel", job_id),
        ));
    }
    let (input_tx, input_rx) = mpsc::channel::<String>(64);
    let (output_tx, output_rx) = mpsc::channel::<String>(64);
    let frontend = IoFrontend { input_tx: input_tx.clone(), output_rx };
    let ml_side = IoHandle { input_rx, output_tx: output_tx.clone() };
    let entry = ChannelEntry { _frontend_input_tx: input_tx, _ml_output_tx: output_tx };
    map.insert(job_id, entry);
    Ok(IoChannels { frontend, ml_side })
}
```

---

**恢复对话时，以此文档为基准继续 Phase 2 实现。**
