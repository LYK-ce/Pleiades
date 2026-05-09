# Pleiades Orchestrator 层设计文档（终版）

**版本**：v1.4  
**日期**：2026-04-21  
**状态**：定稿

---

## 1. Orchestrator Core 设计文档

**定位**：节点的**中央作业调度器**。只管理 Job 的生死，不介入 Job 内部业务。

---

### 1.1 与 Command Portal 的关系

**内核与内部路由层**。

- **Portal 功能**已退化为 Core 的**内部方法层**，不再保留独立的 Portal 结构体。Core 自己持有路由所需的工具（`Compiler`、`IoBroker`、`Capabilities`），命令路由作为 Core 的私有方法（`route_user`、`route_network`）存在。
- Core 直接接收两类外部控制意图，包括：
  - **用户命令**（CLI/TUI/GUI 发来的 `UserCommand`）
  - **网络控制事件**（如远程发来的 `Pipeline_Flow`）
- Core 负责**来源识别与路由决策**，将不同来源的请求统一翻译成**生命周期指令**（`spawn Run`、`spawn WorkerRelay`、`cancel`、`quit`）。
- 查询/配置类命令（`DisplayPeer`、`SetDevice`）由 Core 直接调用 Capability 处理，**不进入调度循环**。

---

### 1.2 与 JobExecutor 的关系

**调度器与执行器**。

- **Core spawn** `JobExecutor`，传入：
  - `JobKind`：业务类型
  - `TaskProgram`：由 Compiler 预编译的指令序列
  - `CancellationToken`：取消信号
  - `Arc<Capabilities>`：共享能力引用
  - `IoHandle`：由 Core 向 IO 模块申请后传入
  - `lifecycle_tx`：死亡通知发送端（mpsc 广播通道）
- **Core 不编译 TaskProgram**，只负责调度成品。
- **JobExecutor 完成后** 通过 `lifecycle_tx` 发送 `LifecycleEvent::Done(job_id, result)`，Core 从注册表移除。
- Core 对 Job **不追踪过程**，只感知**起点**（spawn）和**终点**（Done / Cancel）。

---

### 1.3 作业注册表

```rust
struct JobHandle {
    kind: JobKind,
    cancel: CancellationToken,
}
```

Core 维护 `HashMap<JobId, JobHandle>`。

- **插入**：spawn 时写入。
- **移除**：收到 `LifecycleEvent::Done` 时删除。
- **查询**：Cancel 时按 `JobId` 查找目标；Quit 时遍历全部取消。

注册表**不保存业务状态**（如执行到第几步、当前 prompt 是什么），也**不记录请求来源**。

---

### 1.4 主循环与并发模型

**单线程顺序调度**，通过 `select!` 双通道监听：

```rust
loop {
    tokio::select! {
        // 1. 接收 Portal 发来的生命周期指令（唯一入口）
        Some(cmd) = portal_cmd_rx.recv(), if !shutting_down => {
            match cmd {
                SpawnRun(program, io, cancel) => spawn_job(JobKind::Run, ...),
                SpawnWorkerRelay(program, io, cancel) => spawn_job(JobKind::WorkerRelay, ...),
                Cancel(job_id) => cancel_job(job_id),
                Quit => shutdown(),
            }
        }
        
        // 2. 接收 JobExecutor 死亡通知
        Some(event) = lifecycle_rx.recv() => {
            registry.remove(&event.job_id);
        }
    }
}
```

- **Portal 命令通道**：Portal 已将用户命令和网络事件统一路由，Core 内核看到的只有 spawn / cancel / quit。
- **生命周期通道**：接收 JobExecutor 的 `Done` 通知。
- **禁止 await 长时操作**：spawn 是创建 tokio task，不阻塞；cancel 是发信号，不阻塞。

---

### 1.5 生命周期事件处理

| 指令 | Portal 来源 | Core 动作 |
|------|------------|----------|
| `Run { model_path }` | 用户 | 申请 IO 通道 → 生成 `JobId` → 创建 `JobExecutor(Run)` → 注册到表 → 启动 task |
| `PipelineFlow { ... }` | 网络 | 申请 IO 通道 → 生成 `JobId` → 创建 `JobExecutor(WorkerRelay)` → 注册到表 → 启动 task |
| `Cancel { job_id }` | 用户 | 查表 → 调用对应 `JobHandle.cancel.cancel()` |
| `Quit` | 用户 | 设置 `shutting_down = true`，关闭 Portal 命令接收，遍历注册表全部 cancel |
| `Done` | JobExecutor | 从注册表移除；若 `shutting_down` 且表为空，退出主循环 |

**Quit 优雅退出**：
1. Portal 停止接收新的外部请求。
2. Core 遍历注册表，对所有活跃 Job 发 Cancel。
3. 等待所有 `Done` 通知（注册表清空）。
4. 退出主循环，进程结束。

---

### 1.6 错误处理

| 错误场景 | 处理方式 |
|---------|---------|
| spawn 失败（如系统资源不足） | Core 拒绝，通过 UI Capability 报错，**不注册到表** |
| Cancel 时 `JobId` 不存在 | 静默忽略（可能 Job 已自然结束） |
| JobExecutor 启动即崩溃 | 仍需通过生命周期通道发 `Done`，Core 正常移除 |
| Quit 时 Job 清理卡住 | 依赖外部超时机制（如进程级 SIGKILL），Core 本身不处理超时 |

---

### 1.7 边界红线

- **禁止编译 TaskProgram**：只传 `JobKind` 和成品，具体程序由 Compiler 编译、Executor 加载。
- **禁止感知业务过程**：不问 Job 执行到第几步。
- **禁止感知请求来源**：内核不区分"这是用户发起的 Run 还是网络发起的 WorkerRelay"，只执行 spawn。
- **禁止直接操作资源**：不申请 GPU、不操作文件、不建立网络连接。
- **禁止阻塞主循环**：spawn 和 cancel 必须是即发即走的信号操作。
- **禁止处理业务数据流**：Prompt 不经过 Core。
- **禁止绕过 Portal 接收外部请求**：所有改变作业生命周期的意图必须经过 Portal 路由。

---

## 2. Core内部路由层

**定位**：Orchestrator Core 的**内部命令路由层**。所有改变作业生命周期的外部意图，无论来自用户还是网络，直接进入 Core 的相应通道，由 Core 内部方法进行路由。

---

### 2.1 与 Core 内核的关系

**路由与调度一体化**。

- Core 直接接收**两类来源**的请求，通过内部方法 `route_user` 和 `route_network` 进行识别与路由，统一翻译成**生命周期指令**（`spawn Run`、`spawn WorkerRelay`、`cancel`、`quit`）。
- **Core 内核**既负责路由决策，也负责执行具体的 spawn / cancel / shutdown。内核**不区分请求来源**（用户或网络），但路由方法会根据命令类型调用不同逻辑。
- 查询与配置类请求（如查看节点列表、修改设备偏好）由 Core **直接调用 Capability** 完成，**不进入调度循环**。

---

### 2.2 输入契约（分来源定义）

Core 消费两类结构化输入，**不解析原始字符串**。

#### 2.2.1 用户命令（UserCommand）
来自 CLI / TUI / GUI 等前端接入层。

| 命令 | 语义 |
|------|------|
| `Run { model_path }` | 启动本地推理作业 |
| `Cancel { job_id? }` | 取消指定作业；`None` 表示取消当前焦点作业 |
| `Quit` | 请求优雅退出 |
| `DisplayPeer` | 查询当前节点列表 |
| `SetDevice { device }` | 修改默认计算设备偏好 |

#### 2.2.2 网络命令（NetworkCommand）
来自 Network Capability，仅包含**需要改变作业生命周期**的事件。

| 命令 | 语义 |
|------|------|
| `PipelineFlow { peer_id, ... }` | 远程 Coordinator 请求本节点作为 Worker 加入流水线 |

**以下网络事件不进入 Core**：
- `PeerDiscovered` / `PeerLeft` / 连接状态变更 → 由 PeerManager 内部处理
- `FileStreamProgress` → Network Capability 直发 UI
- 文件传输完成通知 → 作为 WorkerRelay Job 内部步骤处理

---

### 2.3 路由策略（分来源处理）

| 来源 | 命令 | 路由目标 | 说明 |
|------|------|---------|------|
| **用户** | `Run` | **Compiler** → 编译 `TaskProgram` → **Core** `spawn(JobKind::Run)` | 启动本地推理 |
| **用户** | `Cancel` | **Core** `cancel(job_id)` | 取消作业 |
| **用户** | `Quit` | **Core** `shutdown()` | 优雅退出 |
| **网络** | `PipelineFlow` | **Compiler** → 编译 `TaskProgram` → **Core** `spawn(JobKind::WorkerRelay)` | 启动 Worker 作业 |
| **用户** | `DisplayPeer` | **Core 直接处理** → `NetworkCapability.list_peers()` → UI | 纯查询，不触及作业生命周期 |
| **用户** | `SetDevice` | **Core 直接处理** → `ComputeManager.set_preference()` | 纯配置，不触及作业生命周期 |

**核心原则**：只有**改变作业生命周期**的请求才进入调度循环；其余查询与配置由 Core 当场消化。

---

### 2.4 并发模型

**单消费者顺序处理**。

- Core 内部路由方法与调度循环**同线程**，背后是单消费者通道。
- 用户命令与网络命令进入**同一队列**，Core 按 FIFO 顺序处理。
- **禁止并行消费**。`Run` 后立即 `Cancel` 必须严格按序，防止竞态。

---

### 2.5 错误处理

| 错误场景 | 责任方 | 处理方式 |
|---------|--------|---------|
| 前端解析错误（非法参数、语法错误） | **前端接入层** | 前端本地拦截，**不提交到 Core** |
| 网络事件格式异常 | **Network Capability** | Network 层过滤，**不提交到 Core** |
| 路由错误（未知命令类型） | **Core** | 直接拒绝，返回错误给来源方 |
| 编译失败（参数非法） | **Compiler** | Core 拒绝请求，**不进入调度循环** |
| 内核执行错误（GPU 不足、模型不存在） | **Core 内核 / JobExecutor** | 内核通过 UI Capability 暴露，Core 不阻塞等待 |
| 查询/配置失败 | **Core** | 捕获 Capability 错误，同步返回给来源方 |

Core **不吞错误**，也不替下游组件重试。

---

### 2.6 边界红线

- **禁止直接操作 JobExecutor**：只能通过 Core 调度。
- **禁止处理 Prompt 或任何业务数据流**：Prompt 由前端直传 JobExecutor，不经过 Core。
- **禁止让非生命周期事件进入调度循环**：Peer 事件、文件进度等不得转给内核。
- **禁止内部使用 `select!` 并发处理多条命令**：一次一条，顺序推进。
- **禁止 await 长时操作**：查询必须是 Capability 的同步快照读；生命周期指令转交调度后立即返回。

---

## 3. Job 模块设计文档

**定位**：Orchestrator 层的**公共契约**。定义所有组件共享的 Job 标识、类型、状态与生命周期事件。

---

### 3.1 核心类型

#### 3.1.1 JobId

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);
```

- 全局唯一，由 Core 在 spawn 时生成（原子递增或 UUID 截取）。
- `Copy` + `Hash`，便于在注册表和通道中传递。

#### 3.1.2 JobKind

```rust
#[derive(Debug, Clone, Copy)]
pub enum JobKind {
    Run,
    WorkerRelay,
}
```

- Core 路由时决定 spawn 哪种 Executor。
- Executor 加载 TaskProgram 时根据 `JobKind` 匹配编译模板。

#### 3.1.3 JobState

```rust
#[derive(Debug, Clone, Copy)]
pub enum JobState {
    Preparing,
    Ready,
    Executing,
    CleaningUp,
    Done,
}
```

- 由 JobExecutor 维护并上报，Core 可选地缓存以支持查询（如 `DisplayJob` 命令）。
- Core 注册表**不强制依赖**此状态来做调度决策，主要供 UI 展示。

#### 3.1.4 JobHandle

```rust
pub struct JobHandle {
    pub kind: JobKind,
    pub cancel: CancellationToken,
}
```

- Core 注册表 `HashMap<JobId, JobHandle>` 的 Value。
- **极简**：只存取消信号和类型，不存业务状态、不存 SlotFile、不存 IO 句柄。
- `CancellationToken` 由 Core 创建，spawn 时 clone 一份给 JobExecutor。

---

### 3.2 生命周期事件

```rust
pub enum LifecycleEvent {
    Done {
        job_id: JobId,
        result: JobResult,
    },
}
```

- JobExecutor → Core 的**单向通知**。
- 仅 `Done` 一种事件。Preparing/Executing 等状态变化**不上报**，由 UI Capability 内部展示。
- Core 收到后执行 `registry.remove(job_id)`。

#### 3.2.1 JobResult

```rust
pub enum JobResult {
    Success,
    Cancelled,
    Failed(String),
}
```

- `Success`：正常执行到正向序列末尾。
- `Cancelled`：由 Cancel 信号触发补偿链后结束。
- `Failed`：正向执行中某条指令返回 `Err`，触发补偿链后结束。

---

### 3.3 设计约束

| 约束 | 说明 |
|------|------|
| **轻量** | `JobHandle` 禁止膨胀。业务状态在 SlotFile，通道在 IO 模块，都不进注册表 |
| **不可变** | Core 插入注册表后，不修改 `JobHandle` 字段。取消通过 `cancel` 发信号，不动结构体 |
| **无生命周期依赖** | `LifecycleEvent` 不包含 `Arc<JobHandle>` 或自引用，避免循环引用 |
| **Copy 优先** | `JobId`、`JobKind`、`JobState` 均 `Copy`，减少注册表操作时的所有权摩擦 |

---

## 7. Instruction 模块设计文档

**定位**：Orchestrator 层的**指令契约**。定义 Compiler 生成、TaskEngine 解释执行的所有任务指令类型。纯数据定义，无运行时逻辑。

---

### 7.1 模块归属

`pleiades::orchestrator::instruction`

- 由 **Compiler** 生成 `TaskProgram` 时填充指令序列。
- 由 **TaskEngine** 加载后逐条解释执行。
- 作为公共模块，供 Compiler 和 Executor 平等依赖。

---

### 7.2 设计原则

- **业务即数据**：作业流程不是硬编码函数，而是编译后的指令序列。
- **纯数据定义**：`instruction.rs` 只包含枚举和结构体定义，不实现任何执行逻辑。
- **最小够用**：Phase 2 仅定义 9 条基础指令，覆盖数据搬运、资源申请、会话管理、存储隔离与基础控制流。

---

### 7.3 Phase 2 指令集

#### 7.3.1 数据搬运指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `Const { value, dst }` | `value: SlotValue`, `dst: SlotId` | 将常量值写入目标槽位 |
| `Move { src, dst }` | `src: SlotId`, `dst: SlotId` | 从源槽位取出值，写入目标槽位 |

#### 7.3.2 存储资源指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `EnsureWorkspace { job_id }` | `job_id: JobId` | 为指定作业创建隔离工作目录。幂等，已存在时不报错 |
| `CleanupWorkspace { job_id }` | `job_id: JobId` | 清理指定作业的隔离工作目录及临时文件 |

#### 7.3.3 计算资源指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `AcquireDevice { preferred, result }` | `preferred: SlotId`, `result: SlotId` | 从 `preferred` 槽位读取设备偏好字符串，向 ComputeManager 申请租约，结果写入 `result` 槽位。Phase 2 占位返回空 Stub |

#### 7.3.4 推理会话指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `CreateSession { model, device, result }` | `model: SlotId`, `device: SlotId`, `result: SlotId` | 从 `model` 槽位读取模型路径，从 `device` 槽位 **take** 设备租约，创建 ML Session，句柄写入 `result` 槽位 |
| `ShutdownSession { session }` | `session: SlotId` | 从 `session` 槽位 **take** Session 句柄，调用 Inference Capability 关闭 |

#### 7.3.5 控制流指令

| 指令 | 参数 | 语义 |
|------|------|------|
| `JumpIf { condition, label }` | `condition: SlotId`, `label: String` | 从 `condition` 槽位读取布尔值。为真时，指令指针跳转到 `TaskProgram.labels` 中对应标签的索引 |
| `Abort { reason }` | `reason: String` | 立即终止正向执行，触发补偿链 |

---

### 7.4 程序容器

```rust
TaskProgram {
    instructions: Vec<TaskInstruction>,   // 正向执行序列
    compensation: Vec<TaskInstruction>,    // 补偿/清理序列（Cancel 或 Abort 时执行）
    labels: HashMap<String, usize>,        // 标签名 → 指令索引映射
}
```

**说明**：
- `TaskProgram` 由 Compiler 构建，传递给 JobExecutor。
- 正向序列按顺序执行，直到完成或遇到 `Abort`。
- 补偿序列在 Cancel 或 Abort 时逆序执行，用于释放已获取资源。
- 标签用于支持条件跳转（`JumpIf`）。

---

### 7.5 设计约束

| 约束 | 说明 |
|------|------|
| **纯数据** | `TaskInstruction` 只包含数据字段，不包含任何方法或行为 |
| **无依赖** | 不依赖任何运行时组件（如 Capability、网络、文件系统） |
| **类型安全** | 所有槽位引用均通过 `SlotId`，确保编译期可验证 |
| **不可变** | `TaskProgram` 在编译后保持不变，执行期间不修改指令序列 |
| **最小集合** | 仅包含 Phase 2 必需的指令，未来扩展通过新增变体实现 |

---

### 3.4 边界红线

- `job` 模块**禁止**依赖 `TaskEngine`、`SlotFile`、`IoHandle` 等执行期类型。
- `JobHandle` **禁止**持有 `mpsc::Sender` 或 `JoinHandle`，Core 不直接和 Executor 通信，只发 Cancel 信号和接收 Done 通知。
- `JobState` **禁止**作为 Core 调度决策的依据（如"只有 Ready 才能 Cancel"）。Cancel 信号始终可发，由 Executor 内部判断响应时机。

---

## 4. JobExecutor 设计文档

**定位**：统一执行框架。所有业务（`Run`、`WorkerRelay`）共享同一结构，接收预编译的 `TaskProgram`，驱动 `TaskEngine` 完成生命周期管理。

---

### 4.1 与 Core 的关系

**被调度者与调度者**。

- **Core spawn** JobExecutor，传入：
  - `JobKind`：决定加载哪种业务模板
  - `TaskProgram`：由 Compiler 预编译的正向指令 + 补偿指令序列
  - `CancellationToken`：用于接收取消信号
  - `Arc<Capabilities>`：共享能力引用
  - `IoHandle`：由 Core 向 IO 模块申请后传入，供 ML Thread 绑定
  - `lifecycle_tx`：死亡通知发送端
- **JobExecutor 不编译 TaskProgram**，只负责解释执行成品。
- JobExecutor 完成后通过 `lifecycle_tx` 发送 `LifecycleEvent::Done(job_id)`，Core 从注册表移除。

---

### 4.2 与 TaskEngine 的关系

**驱动器与私有引擎**。

- **模块归属**：TaskEngine 是 JobExecutor 的私有内部模块，不对外暴露。代码组织为 `src/orchestrator/executor/mod.rs`（JobExecutor 结构体）和 `src/orchestrator/executor/task_engine.rs`（TaskEngine 实现）。`mod task_engine;` 为私有模块声明，外部模块无法引用 TaskEngine。
- **职责划分**：
  - **TaskEngine** 是 JobExecutor 内部的指令级 VM，负责解释执行 `TaskProgram`，维护 SlotFile（跨步骤显式状态），推进指令指针 IP，执行补偿序列。
  - **JobExecutor** 持有 TaskEngine 实例，通过 `select!` 监听 Cancel 与 `step()` 结果，根据 StepResult 推进 JobState，发送死亡通知。
- **SlotFile** 由 TaskEngine 维护，跨步骤显式传递资源句柄（路径、节点列表、设备租约、Session 句柄等）。
- TaskEngine 的 `step()` 返回 `StepResult` 驱动 JobExecutor 的状态流转。
- `step()` 内部**纯顺序执行**，禁止 `select!`，单条指令执行完毕立即返回。

---

### 4.3 与 ML Thread 的关系（通过 IO 模块）

- **IO 通道**由 Core 在 spawn 前申请，作为参数传入 JobExecutor。
- TaskEngine 执行 `CreateSession` 时，将 `IoHandle` 从 SlotFile 取出，提交给 Inference Capability。
- Inference Capability 将通道绑定到 ML Thread。
- **此后 Input/Output 直传 ML Thread**，JobExecutor 不参与数据中转。

---

### 4.4 状态机

```
Preparing（资源准备：分析、切分、建流、申请设备、创建 Session）
    │
    ▼
Ready（资源就绪，等待进入核心逻辑）
    │
    ▼
Executing（核心逻辑：提交推理、运行 Relay）
    │
    ├── 正常完成 ──► CleaningUp（正向清理）──► Done
    │
    ├── Cancel 信号 ──► CleaningUp（补偿链）──► Done
    │
    └── 指令失败 ──► CleaningUp（补偿链）──► Done
```

- **Preparing**：执行资源准备指令。
- **Ready**：所有资源已就绪，尚未启动核心计算。作为资源就绪的显式检查点。
- **Executing**：执行核心计算逻辑。
- **CleaningUp**：执行清理指令或补偿链，释放已获取资源。
- **Done**：终态。Executor 发送死亡通知后自我销毁。

---

### 4.5 执行循环

```rust
impl JobExecutor {
    pub async fn run(mut self) {
        self.task_engine.load(self.program);
        
        loop {
            tokio::select! {
                biased;
                
                _ = self.cancel.cancelled() => {
                    self.task_engine.enter_compensation().await;
                    break;
                }
                
                result = self.task_engine.step(&self.capabilities) => {
                    match result {
                        StepResult::Continue => continue,
                        StepResult::Ready => { self.state = JobState::Ready; }
                        StepResult::Done => break,
                        StepResult::Abort(e) => {
                            self.report_error(e).await;
                            self.task_engine.enter_compensation().await;
                            break;
                        }
                    }
                }
            }
        }
        
        let _ = self.lifecycle_tx.send(LifecycleEvent::Done(self.job_id)).await;
        self.cleanup().await;
    }
}
```

**关键约束**：
- `select!` 仅监听 Cancel 与 `step()`，**两路**。
- `step()` 内部**禁止 `select!`**，纯顺序执行单条指令。
- **没有 `cmd_rx`**，Input 直传 ML Thread，不经过 Executor。
- **没有 `Yield` 指令**，不进入 Suspending 状态。

---

### 4.6 输入与生命周期回传

| 方向 | 内容 | 机制 |
|------|------|------|
| Core → JobExecutor | Cancel 信号 | `CancellationToken.cancelled()` |
| JobExecutor → Core | 死亡通知 | `mpsc::Sender<LifecycleEvent>` 发送 `Done(job_id)` |

- **Input 不经过 Executor**。前端通过 IO 模块直传 ML Thread。
- 死亡通知使用 **mpsc 广播通道**，Core 统一接收所有 Job 的完成事件。

---

### 4.7 补偿链（Saga）

TaskProgram 由 Portal 预编译时，同时生成**正向指令序列**与**反向补偿序列**。

| 触发条件 | 执行内容 |
|---------|---------|
| **Cancel 信号** | `enter_compensation()` 执行补偿序列，逆序释放已获取资源 |
| **指令 Abort** | 同上，补偿后上报错误 |
| **正常 Done** | 执行正向序列末尾的清理指令，不触发补偿 |

**补偿映射示例**：
- `CreateSession` → `ShutdownSession`
- `PrepareTensorStreams` → `CloseTensorStreams`
- `BroadcastCommand(WORK)` → `BroadcastCommand(RESET)`
- `AcquireDevice` → `DeviceLease` Drop（自动）
- `SplitModel` / `ReceiveFile` → `CleanupWorkspace`

**补偿失败处理**：记录日志，继续执行后续补偿，不中断清理流程。最终通过 UI 暴露未完全清理的警告。

---

### 4.8 清理闭环

| 资源 | 释放方式 |
|------|---------|
| Session | `ShutdownSession` 指令显式关闭 |
| Tensor Stream | `CloseTensorStreams` 指令显式关闭 |
| DeviceLease | `take` 移交给 ML Thread，Session 关闭后 Drop 自动释放 |
| Workspace 文件 | `CleanupWorkspace` 指令显式清理 |
| SlotFile 残余 | JobExecutor Drop 时 `SlotFile` Drop，RAII 自动兜底 |

**原则**：关键资源由补偿链/清理指令显式释放；句柄级资源由 Rust Drop 自动兜底。

---

### 4.9 边界红线

- **禁止直接操作 socket、文件、GPU**。必须通过 Capability 调用。
- **`step()` 内部禁止 `select!`**。单条指令顺序执行。
- **禁止持有业务状态**。只通过 SlotFile 显式化。
- **禁止处理 Input 数据流**。Input 直传 ML Thread。
- **禁止编译 TaskProgram**。只接收成品执行。
- **禁止 fire-and-forget spawn**。Capability 内部的长时操作必须是可 await 的纯 Future。
- **禁止暴露 TaskEngine**：TaskEngine 是 JobExecutor 的私有内部模块，禁止暴露给 Core、Portal、Compiler 等外部模块引用。
- **TaskEngine 不感知 Cancel**：Cancel 信号由 Executor 的 `select!` 捕获，TaskEngine 只负责执行补偿序列，不主动检测取消。

---

## 5. Compiler 设计文档

**定位**：Orchestrator 层的**指令编译器**。将高层业务意图翻译成 Executor 可直接执行的 `TaskProgram`。

---

### 5.1 与 Core 的关系

**被调用者与调用者**。

- **Core** 收集业务参数（`model_path`、`device`、`peer_id` 等），调用 Compiler 生成 `TaskProgram`。
- **Compiler** 无状态，不保存上下文，编译完成后立即返回成品。
- Core 拿到 `TaskProgram` 后，连同 `JobKind` 一起提交给调度循环 spawn。

---

### 5.2 与 Core 的关系

**无直接关系**。

- Compiler 不持有 Core 引用，不感知作业生命周期。
- Core 只接收 Compiler 的产出（`TaskProgram`），不介入编译过程。

---

### 5.3 编译内容

| 输入 | 输出 |
|------|------|
| `JobKind::Run` + 模型路径 + 设备偏好 + `JobId` | `TaskProgram`（正向指令 + 补偿指令 + 标签表） |
| `JobKind::WorkerRelay` + `JobId` + 可选参数 | 同上 |

编译过程使用内部 `TaskProgramBuilder` 构造指令序列，**不暴露给外部**。

---

### 5.4 关键设计

#### 5.4.1 即时编译
Portal 收到请求后**立刻编译**，毫秒级完成。仅做内存中的指令构造，不涉及文件 IO 或模型加载。

#### 5.4.2 补偿指令自动生成
`TaskProgramBuilder::build()` 扫描正向指令，自动推导并生成对应的**补偿指令序列**，按资源依赖关系排序。

#### 5.4.3 参数校验
Compiler 只检查参数**合法性**（路径非空、设备名格式等）。非法参数编译失败，Portal 直接拒绝，**不进入 Core**。

资源类错误（模型不存在、GPU 不足）在执行期暴露，由 Executor 补偿链处理。

---

### 5.5 错误处理

| 错误场景 | 处理方式 |
|---------|---------|
| 参数非法（空路径、未知设备名） | 编译失败，返回 `Err`，Portal 拒绝请求 |
| 内部 Builder 错误（Slot 溢出、标签未定义） | 编译失败，视为 Bug，记录日志 |
| 资源错误 | **不处理**。交给 Executor 执行期处理 |

---

### 5.6 边界红线

- **禁止持有状态**：不缓存、不保存已编译程序。
- **禁止引用 Capability**：编译是纯内存操作，不触及网络、文件、GPU。
- **禁止阻塞**：编译必须是同步或轻量异步，禁止 await 长时操作。
- **禁止暴露 Builder**：`TaskProgramBuilder` 是 Compiler 内部实现细节。

---

## 6. Slot 模块设计文档

**定位**：Orchestrator 层的**显式状态容器**。所有跨步骤的资源、配置与中间结果，必须通过 SlotFile 传递，禁止隐式全局变量。

---

### 6.1 模块归属

`pleiades::orchestrator::slot`

- `SlotId` 与 `SlotValue` 是 Orchestrator 层的**公共词汇**，Compiler 生成指令、TaskEngine 解释执行、Capability 消费句柄，均依赖此模块。
- `SlotFile` 由 `TaskEngine` 私有持有，每个 `JobExecutor` 一份，Job 结束后随 Drop 自动清理。

---

### 6.2 核心结构

| 类型 | 说明 |
|------|------|
| `SlotId(u32)` | 槽位编号，类似寄存器索引。由 `TaskProgramBuilder` 在编译时分配。 |
| `SlotValue` | 枚举，封装所有可存入槽位的类型。 |
| `SlotFile` | `HashMap<SlotId, SlotValue>` 的包装，提供类型安全的访问方法。 |

---

### 6.3 SlotValue 变体（Phase 2）

采用**显式枚举**，编译期类型明确，调试友好。

| 变体 | 用途 |
|------|------|
| `Nil` | 空槽位，初始状态或 `take` 后的残留 |
| `Bool(bool)` | 控制流条件（`JumpIf`） |
| `U64(u64)` | 数值参数（如 `max_tokens`） |
| `String(String)` | 设备名、错误描述 |
| `PathBuf(PathBuf)` | 模型路径、文件位置 |
| `DeviceLease(...)` | 计算设备租约（RAII，Phase 2 先用 Stub） |
| `SessionHandle(...)` | ML Session 句柄（Phase 2 先用 Stub） |
| `Error(String)` | 步骤执行失败的错误传递 |

**原则**：类型数量可控，不引入 `dyn Any`。未来新增句柄类型时，直接扩展枚举变体。

---

### 6.4 SlotFile 方法

#### 6.4.1 通用操作

| 方法 | 语义 |
|------|------|
| `set(slot, value)` | 写入或覆盖槽位。旧值自然 `Drop`，RAII 资源自动释放。 |
| `get(slot) -> Option<&SlotValue>` | 只读引用，不转移所有权。 |
| `take(slot) -> Option<SlotValue>` | 取出所有权，原槽位置 `Nil`。 |
| `remove(slot) -> Option<SlotValue>` | 删除槽位，返回旧值。 |

#### 6.4.2 类型安全便利方法

针对 Phase 2 高频场景，提供专用访问接口，避免调用方手动 `match`：

| 方法 | 场景 |
|------|------|
| `get_string(slot) -> Result<&String, String>` | 读取设备名、路径字符串 |
| `get_path(slot) -> Result<&PathBuf, String>` | 读取模型路径 |
| `get_bool(slot) -> Result<bool, String>` | 读取 `JumpIf` 条件 |
| `take_device(slot) -> Result<DeviceLease, String>` | 移交设备租约给 `CreateSession` |
| `take_session(slot) -> Result<SessionHandle, String>` | 移交 Session 给 `ShutdownSession` |

类型不匹配时返回 `Err(String)`，由 `TaskEngine` 捕获并进入 `Abort`。

---

### 6.5 设计约束

| 红线 | 说明 |
|------|------|
| **显式化** | 所有跨步骤状态必须在 SlotFile 中声明，禁止隐式全局变量 |
| **所有权明确** | `set` 覆盖时旧值 `Drop`；`take` 后原槽位变 `Nil` |
| **类型校验** | 运行时检查，不匹配即 `Abort`，不静默转换 |
| **Job 隔离** | 每个 JobExecutor 独立 `SlotFile`，Job 结束后全量 Drop |
| **不感知业务** | `SlotFile` 只存数据，不解释语义（语义由 `TaskInstruction` 定义） |

---