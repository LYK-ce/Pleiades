完全可以。把 IO 抽成**独立模块**，Core 作为通道生命周期的**起点**，这比把 IO 藏在 Inference 里更干净。Core 管理 Job，IO 通道作为 Job 的**配套资源**，由 Core 统一申请、传递、回收。

---

## Session IO 通道管理层设计（独立模块版）

**定位**：`pleiades::io` 独立模块。负责通道的创建、注册、查询与回收。Core 掌握通道所有权，Inference 只负责绑定使用。

---

### 1. 模块边界

| 组件 | 职责 |
|------|------|
| **IO 模块** | 创建通道对；维护 `JobId → IoHandle` 注册表；对外提供查询接口 |
| **Core** | 在 `spawn` 时向 IO 模块申请通道；将通道句柄传给 JobExecutor；Job 结束后通知 IO 模块回收 |
| **JobExecutor / TaskEngine** | 在 `CreateSession` 时将通道句柄作为参数提交给 Inference Capability |
| **Inference Capability** | 接收通道句柄，将其绑定到 ML Thread（inference thread） |
| **前端** | 通过 IO 模块查询 `JobId` 获取通道端点，直接读写 |

---

### 2. 核心结构

```rust
// pleiades::io
pub struct IoBroker {
    registry: HashMap<JobId, IoHandle>,
}

pub struct IoHandle {
    pub input_tx: std::sync::mpsc::Sender<String>,   // 外部 → ML Thread
    pub output_rx: tokio::sync::mpsc::Receiver<String>, // ML Thread → 外部（已桥接）
}
```

---

### 3. 生命周期（Core 主导）

| 阶段 | 动作 | 负责方 |
|------|------|--------|
| **申请** | Core 根据 `JobKind` 向 `IoBroker` 申请通道。`Run` 申请输入+输出；`WorkerRelay` 可申请输出日志通道或跳过 | **Core** |
| **传递** | Core 将 `IoHandle` 作为参数传给 `JobExecutor`。TaskEngine 将其存入 SlotFile，供 `CreateSession` 指令消费 | **Core → JobExecutor** |
| **绑定** | `CreateSession` 执行时，Inference Capability 从 Slot 取出通道，移交给 ML Thread | **Inference Capability** |
| **使用** | 前端通过 `io_broker.get_handle(job_id)` 获取 `IoHandle`，直接对话 ML Thread | **前端 / IO 模块** |
| **回收** | Core 收到 `LifecycleEvent::Done` 后，调用 `io_broker.recycle(job_id)`，移除注册表，通道自然关闭 | **Core** |

---

### 4. SlotFile 扩展

TaskEngine 需要能持有和传递通道句柄：

```rust
pub enum SlotValue {
    // ... 原有类型
    IoHandle(IoHandle),  // 新增：IO 通道句柄包
}
```

`CreateSession` 指令签名调整为：
```rust
TaskInstruction::CreateSession { 
    model, layers, device, tensor_io, 
    io_handle: SlotId,  // 从 Slot 中取出 IoHandle 绑定到 Session
    result