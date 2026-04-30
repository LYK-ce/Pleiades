# Tensor Port Switch 设计文档

## 1. 问题背景

当前分布式流水线推理链条中，张量流（Tensor Stream）的管理存在以下问题：

### 1.1 现有设计的不足

1. **Tensor_IO_Broker 定位不清**：作为"中转站"存在，Job 异步执行时从 Broker 存取 stream，但设计别扭——outbound 由 Job 自己打开却要绕经 Broker 再取回。

2. **ML Thread 直连 stream**：当前 `Tensor_IO_Handle` 直接持有 `libp2p::Stream`。如果网络连接断开（节点崩溃/离开），ML Thread 立即得到 IO Error → Session 死亡 → 整条推理链崩溃。无法动态恢复。

3. **无热切换能力**：一旦 stream 绑定到 Session，无法在运行中更换目标节点。节点拓扑变化（加入/离开/故障转移）需要销毁整条链重建。

### 1.2 鲁棒性目标

分布式推理系统需要适应设备随时加入/离开的动态环境。分析后确定**两个最小鲁棒性原语**：

| 原语 | 语义 | 实现 |
|------|------|------|
| **P1: ReRoute** | 调整流的收发方向 | Tensor Port Switch（本文档） |
| **P2: ReloadLayers** | 调整节点加载的模型层 | Session 重建 / Run_Program 注入 |

P2 在 ML Engine 层已有机制（`Session_Command::Run_Program` 换行为、`Shutdown+Create` 换层范围）。**P1 是当前唯一的基础设施缺口。**

---

## 2. 设计方案：共享锁 + Switch 管理

### 2.1 核心理念

- **Core 管理张量流**：实际的 `libp2p::Stream` 由 Switch 模块持有，通过 `Mutex` 保护。
- **ML Thread 只拿接口**：Session 通过 `Arc` 引用访问 Mutex 内的 stream，直接读/写，零拷贝。
- **Core 热切换**：换目标时锁住 Mutex → 替换 stream → 解锁。ML Thread 下一次调用自动使用新 stream。

### 2.2 数据路径（零拷贝，零中间层）

```
发送路径: ML Thread → lock(outbound) → stream.write(data) → unlock → 网络 → 对端
接收路径: 对端 → 网络 → stream → lock(inbound) → stream.read → buffer → unlock → ML Thread
```

没有 channel，没有 bridge task，没有额外内存拷贝。与当前 `Tensor_IO_Handle` 直连方案性能完全相同。

### 2.3 广播/冗余支持

Outbound 使用 `Vec<Stream>` 而非单个 stream。ML Thread 的一次 `Send()` 写入所有目标，支持冗余备份：

```
ML Thread → Send(data) → lock → write to Stream A, Stream B, ... → unlock
```

ML Thread 不知道有几个接收端，不知道数据发给了谁。Core 通过 Switch 接口动态 Add/Remove 目标。

---

## 3. 数据结构

### 3.1 Switch 内部条目

```rust
struct Port_Entry {
    /// 入站流（单源，受 Mutex 保护）
    inbound: Arc<std::sync::Mutex<libp2p::Stream>>,
    /// 出站流（多目标广播，受 Mutex 保护）
    outbound: Arc<std::sync::Mutex<Vec<libp2p::Stream>>>,
}
```

### 3.2 ML Thread 端点（Session 持有）

```rust
pub struct Tensor_IO_Endpoint {
    /// 入站流引用（与 Switch 共享同一 Arc）
    inbound: Arc<std::sync::Mutex<libp2p::Stream>>,
    /// 出站流引用（与 Switch 共享同一 Arc）
    outbound: Arc<std::sync::Mutex<Vec<libp2p::Stream>>>,
    /// 线程私有的预分配缓冲区（复用，无需共享）
    buffer: Tensor_Buffer,
    /// tokio runtime 句柄（用于 block_on）
    rt: tokio::runtime::Handle,
    /// 最后成功发送的 offset（AtomicU64，Core 可无锁读取）
    last_send_offset: Arc<AtomicU64>,
    /// 最后成功接收的 offset（AtomicU64，Core 可无锁读取）
    last_recv_offset: Arc<AtomicU64>,
    /// 失败上报通道（发送失败索引到 Core）
    failure_tx: mpsc::UnboundedSender<FailureReport>,
}

/// 失败上报结构
pub struct FailureReport {
    pub job_id: JobId,
    /// 失败的出站目标索引列表
    pub failed_indices: Vec<usize>,
}
```

### 3.3 Switch 主体

```rust
pub struct Tensor_Port_Switch {
    entries: tokio::sync::Mutex<HashMap<JobId, Port_Entry>>,
}
```

---

## 4. 接口设计

### 4.1 Core 控制面（Trait）

```rust
#[async_trait]
pub trait Tensor_IO_Capability: Send + Sync {
    /// 注册 Job + 初始绑定 stream → 返回 ML 接口
    async fn Register(
        &self,
        job_id: JobId,
        inbound: libp2p::Stream,
        outbound: Vec<libp2p::Stream>,
    ) -> Result<Tensor_IO_Endpoint, Tensor_IO_Error>;

    /// 热切换入站源
    async fn Swap_Input(&self, job_id: JobId, new_stream: libp2p::Stream) -> Result<(), Tensor_IO_Error>;

    /// 添加一个出站目标
    async fn Add_Output(&self, job_id: JobId, stream: libp2p::Stream) -> Result<(), Tensor_IO_Error>;

    /// 移除一个出站目标（按索引）
    async fn Remove_Output(&self, job_id: JobId, index: usize) -> Result<(), Tensor_IO_Error>;

    /// 替换全部出站目标
    async fn Set_Outputs(&self, job_id: JobId, streams: Vec<libp2p::Stream>) -> Result<(), Tensor_IO_Error>;

    /// 清理（Job 结束后调用）
    async fn Deregister(&self, job_id: JobId);

    /// 查询是否活跃
    async fn Is_Active(&self, job_id: JobId) -> bool;
}
```

### 4.2 ML Thread 数据面

```rust
/// I/O 超时时间（防止 stream 断开时长时间持锁）
const IO_TIMEOUT: Duration = Duration::from_secs(5);

impl Tensor_IO_Endpoint {
    /// 阻塞发送张量（广播到所有出站目标）
    ///
    /// 单个目标失败不中断推理，记录失败索引并上报 Core。
    /// Mutex 使用 unwrap_or_else 防中毒处理。
    pub fn Send(&mut self, offset: u64, data: &[u8]) -> io::Result<()> {
        let mut streams = self.outbound.lock().unwrap_or_else(|e| e.into_inner());
        let mut failed_indices = Vec::new();
        for (idx, stream) in streams.iter_mut().enumerate() {
            let result = self.rt.block_on(async {
                tokio::time::timeout(IO_TIMEOUT, Send_Tensor_Frame(stream, offset, data)).await
            });
            match result {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => { failed_indices.push(idx); }
            }
        }
        // 上报失败索引（供 Core 异步处理）
        if !failed_indices.is_empty() {
            self.report_failures(failed_indices);
        }
        // 记录最后成功 offset
        self.last_send_offset.store(offset, Ordering::Relaxed);
        Ok(())
    }

    /// 阻塞接收张量（从入站源读取一帧）
    ///
    /// 带超时保护，超时返回 TimedOut 错误，释放锁后 Core 可执行 Swap。
    pub fn Receive(&mut self) -> io::Result<u64> {
        let mut stream = self.inbound.lock().unwrap_or_else(|e| e.into_inner());
        let result = self.rt.block_on(async {
            tokio::time::timeout(IO_TIMEOUT, Receive_Tensor_Frame(&mut *stream, &mut self.buffer)).await
        });
        match result {
            Ok(inner) => {
                if let Ok(offset) = &inner {
                    self.last_recv_offset.store(*offset, Ordering::Relaxed);
                }
                inner
            }
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "receive timeout"))
        }
    }

    /// 获取最近一次 Receive 的数据
    pub fn Get_Buffer(&self) -> &[u8] {
        self.buffer.As_Slice()
    }

    /// 发送 EOF 到所有出站目标
    pub fn Send_EOF(&mut self) -> io::Result<()> {
        let mut streams = self.outbound.lock().unwrap_or_else(|e| e.into_inner());
        for stream in streams.iter_mut() {
            let _ = self.rt.block_on(async {
                tokio::time::timeout(IO_TIMEOUT, Send_EOF(stream)).await
            });
        }
        Ok(())
    }

    /// 获取最后成功发送的 offset（供 Core 读取做一致性决策）
    pub fn Last_Send_Offset(&self) -> u64 {
        self.last_send_offset.load(Ordering::Relaxed)
    }

    /// 获取最后成功接收的 offset（供 Core 读取做一致性决策）
    pub fn Last_Recv_Offset(&self) -> u64 {
        self.last_recv_offset.load(Ordering::Relaxed)
    }
}
```

---

## 5. Core 使用流程

### 5.1 建立分布式推理链条

```
1. Core 协商：发送 REQUEST_PIPELINE → 获得 relay_job_id
2. Core 建立网络连接：open_tensor_stream → 获得 outbound stream
3. Core 等待入站连接：TensorStreamArrived → 获得 inbound stream
4. Core 注册到 Switch：
   endpoint = switch.Register(job_id, inbound, vec![outbound])
5. Core 注入 endpoint 到 Session 并启动 Job：
   session_config.tensor_io = Some(endpoint)
   spawn_job(...)
```

### 5.2 故障恢复（运行中热切换）

```
1. Core 检测到 Worker A 崩溃（连接断开/心跳超时）
2. Core 连接到 Worker B：open_tensor_stream → 获得新 outbound stream
3. Core 等待 Worker B 的入站连接 → 获得新 inbound stream
4. Core 执行热切换：
   switch.Swap_Input(job_id, new_inbound)
   switch.Set_Outputs(job_id, vec![new_outbound])
5. ML Thread 完全无感，继续推理
```

### 5.3 冗余备份

```
1. Core 注册时绑定多个目标：
   switch.Register(job_id, inbound, vec![stream_a, stream_b])
2. 正常运行中 ML Thread 每次 Send 写入 A 和 B
3. Worker A 崩溃：
   switch.Remove_Output(job_id, 0)  // 移除 A
   switch.Swap_Input(job_id, stream_from_b)  // 切换到 B 的返回流
4. ML Thread 无感
```

---

## 6. 并发分析

### 6.1 锁竞争

| 时段 | ML Thread | 锁状态 | Core |
|------|-----------|--------|------|
| 推理计算 (~50ms) | 不持锁 | 空闲 | 可随时 Swap ✅ |
| I/O 读写 (~1ms) | 持锁 | 占用 | 等待 (极短) |

推理计算时间远大于 I/O 时间，Core 想 Swap 时 99% 情况能立即拿到锁。

### 6.2 Mutex 选择

使用 `std::sync::Mutex`（非 tokio::sync::Mutex）：
- ML Thread 是 OS 线程，需要 blocking 语义
- `block_on()` 持有 `std::sync::Mutex` 是安全的（block_on 从外部看是同步阻塞）
- 所有 `lock()` 调用使用 `unwrap_or_else(|e| e.into_inner())` 防止 Mutex 中毒传播
- 如需更好性能可考虑 `parking_lot::Mutex`（天然不会中毒）

### 6.3 广播写入失败处理

策略：
1. 单个 stream 写入失败时记录其索引
2. `Send()` 返回 Ok（不因单个目标失败中断推理）
3. 通过上报机制通知 Core
4. Core 异步调用 `Remove_Output` 清理坏目标

---

## 7. 与现有模块的关系

### 7.1 替代关系

| 被替代 | 新方案 |
|--------|--------|
| `Tensor_IO_Broker`（Orchestrator 内） | `Tensor_Port_Switch`（独立模块） |
| `Tensor_IO_Handle`（网络层直连） | `Tensor_IO_Endpoint`（共享锁间接） |
| 4 条 TaskInstruction（OpenTensorStream/TakeInbound/TakeOutbound/BuildTensorIo） | Core 在 spawn 前完成连接，直接注入 endpoint |
| `RequestPipeline` TaskInstruction | Core 层协商逻辑 |

### 7.2 保留不变

| 模块 | 说明 |
|------|------|
| `tensor_stream_protocol.rs` 的帧格式 | `Send_Tensor_Frame` / `Receive_Tensor_Frame` 继续使用 |
| `LLM_IO_Broker` | 文本平面不变，与张量平面并行 |
| ML Thread 的 `Instruction::Send` / `Instruction::Receive` | 仅改底层调用（从 `tio.Send()` → `endpoint.Send()`） |
| Worker 的 ML 程序 `Loop[Receive, BreakIf, Inference, Send]` | 不变 |

### 7.3 Capabilities 变化

```rust
pub struct Capabilities {
    pub storage: Arc<StorageManager>,
    pub ml_engine: Box<dyn ML_Engine_Capability>,
    pub network: Box<dyn Network_Service_Capability>,
    pub peer_manager: Box<dyn Peer_Management_Capability>,
    pub event_bus: Arc<EventBus>,
    pub io_broker: Arc<LLM_IO_Broker>,
    pub tensor_switch: Arc<Tensor_Port_Switch>,  // 替代 tensor_io_broker
}
```

---

## 8. "先连接后启动"的 Core 层编排

引入 Switch 后，分布式 Job 的 spawn 流程变为：

### 8.1 Coordinator 侧

```
1. UserCommand::Coordinate 触发
2. Core → send REQUEST_PIPELINE → 获得 relay_job_id
3. Core → open_tensor_stream(relay_peer) + handshake
4. Core ← TensorStreamArrived（Relay 主动连过来的 outbound）
5. Core → tensor_switch.Register(coord_job_id, inbound, vec![outbound])
6. Core → compile 简化后的 Coordinator 程序（无网络指令）
7. Core → spawn Job（注入 endpoint + io_handle）
```

### 8.2 Relay 侧

```
1. Core 收到 REQUEST_PIPELINE → 分配 relay_job_id → 回复 OK
2. Core → open_tensor_stream(coordinator_peer) + handshake
3. Core ← TensorStreamArrived（Coordinator 主动连过来的 outbound）
4. Core → tensor_switch.Register(relay_job_id, inbound, vec![outbound])
5. Core → compile Relay 程序（无网络指令）
6. Core → spawn Job（注入 endpoint + io_handle）
```

### 8.3 简化后的 TaskProgram

```text
// Coordinator（无网络指令，和单机推理几乎一样）
Const(model) → Const(device) → Const(layers) → CreateSession(with tensor_io) → RunProgram

// Relay（完全相同结构）
Const(model) → Const(device) → Const(layers) → CreateSession(with tensor_io) → RunProgram
```

所有网络连接建立和流管理在 spawn 之前由 Core 完成，Job 只做纯推理。

---

## 9. 未来扩展

| 扩展方向 | 实现方式 |
|---------|---------|
| 多 Worker 链式流水线 (A→B→C) | 每个节点 Register 时 inbound/outbound 接不同邻居 |
| 故障检测 | 心跳 / 连接断开事件 → Core 触发 Swap |
| 负载均衡 | Core 监控延迟 → Swap 到更快节点 |
| 冗余备份（Fan-out） | Register 时 outbound 传入多个 stream |
| Coordinator 纯网关模式 | Coordinator 不加载模型，只做 tokenize+sample |
| 广域网高延迟适配 | 加 atomic swap_flag 避免 Mutex 长时锁定 |

---

## 10. 鲁棒性保证

### 10.1 I/O 超时保护

所有 `block_on` 调用均包装 `tokio::time::timeout(IO_TIMEOUT, ...)`：

- **目的**：防止 stream 断开/网络分区时 ML Thread 长时间持锁，阻塞 Core 的 Swap 操作
- **超时值**：`IO_TIMEOUT = 5s`（可配置）；正常 LAN 环境下单帧 I/O < 10ms，5s 足够区分"慢"和"死"
- **超时后行为**：`Receive()` 返回 `io::ErrorKind::TimedOut` → ML Engine 设置 `FLAG4+FLAG1` → Loop 退出 → Session 进入等待状态

超时保证了**故障场景下 Mutex 的最大持有时间为 IO_TIMEOUT**，Core 可在此后立即拿到锁执行 Swap。

### 10.2 Mutex 中毒处理

所有 `lock()` 调用使用：

```rust
self.inbound.lock().unwrap_or_else(|e| e.into_inner())
```

- **语义**：即使之前持有锁的线程 panic，当前调用者仍无条件获取锁内数据
- **适用场景**：ML Thread panic 后 Core 仍能 Swap/Deregister；极小概率 Core panic 后 ML Thread 仍能继续
- **无需第三方依赖**：标准库 `std::sync::Mutex` 即可满足

### 10.3 数据一致性（Core 统管）

数据一致性完全由 Core 管理，ML Thread 不感知拓扑变化：

#### Offset 追踪

`Tensor_IO_Endpoint` 内部维护两个 `AtomicU64`：
- `last_send_offset`：最后成功 Send 的 offset
- `last_recv_offset`：最后成功 Receive 的 offset

Core 在执行 Swap 前读取这两个值，用于决策新 Worker 的起始 offset。

#### Swap 后的一致性保证

```
1. Core 检测到故障
2. Core 读取 endpoint.Last_Send_Offset() → 得到 last_ok = N
3. Core 连接到新 Worker B，协商 "从 offset N+1 开始接收"
4. Core 执行 Swap_Input + Set_Outputs
5. ML Thread 下次 Receive() 从新 stream 读 offset N+1 的帧 → 无缝衔接
```

ML Thread 不需要重试、不需要回退——Core 保证新 stream 上的数据从正确位置开始。

### 10.4 广播偏序处理（Core 统管）

广播写入中某个目标失败的偏序问题由 Core 处理：

#### 失败上报机制

1. `Send()` 循环写入 `Vec<Stream>` 中每个目标
2. 某个 stream 超时/失败 → 记录其索引到 `failed_indices`
3. 通过 `failure_tx`（unbounded channel）异步上报 Core
4. `Send()` 返回 Ok（不中断推理）

#### Core 收到上报后的决策

| 场景 | Core 动作 |
|------|-----------|
| 冗余备份中 B 失败 | `Remove_Output(job_id, B_index)` → 继续使用 A |
| 唯一目标失败 | 触发故障恢复流程（连新 Worker → Swap） |
| 备份节点恢复可用 | `Add_Output(job_id, new_stream)` → 注意新 stream 从当前 offset 开始，前序数据不追赶 |

#### 不追赶原则

如果备份目标 B 在 step N 失败后被移除，后续重新接入新备份 C 时：
- C 从当前 step M (M > N) 开始接收
- 不回溯补发 N~M 之间的数据
- C 仅用于 M 之后的冗余；真正 failover 时 Core 负责指示新主节点从正确位置开始

### 10.5 职责边界总结

| 层 | 职责 |
|----|------|
| **Core** | 连接管理、故障检测、Swap 决策、offset 同步、偏序恢复、Register/Deregister |
| **Tensor_Port_Switch** | 锁管理、stream 存储、提供 Swap/Add/Remove 原子操作 |
| **Tensor_IO_Endpoint** | lock + timeout + block_on I/O、unwrap_or_else 防中毒、offset 记录、失败索引上报 |
| **ML Thread** | 纯推理计算、调用 Send/Receive、不感知拓扑变化 |

---

## 11. 实施计划

### 11.1 影响范围

共涉及 **1 个新建文件 + 1 个删除文件 + 18 个修改文件**。

### 11.2 Phase 1：新建 Tensor_Port_Switch 模块

**目标**：独立可编译的新模块，不破坏现有代码。

| # | 文件 | 操作 | 详情 |
|---|------|------|------|
| 1.1 | `Src/LLM_IO/tensor_port_switch.rs` | 🆕 新建 | `Tensor_Port_Switch` + `Tensor_IO_Endpoint` + `Port_Entry` + `FailureReport` + `Tensor_IO_Error` + `Tensor_IO_Capability` trait 实现 |
| 1.2 | `Src/LLM_IO/mod.rs` | ✏️ 修改 | 添加 `pub mod tensor_port_switch;` 和聚合导出 |

**估计代码量**：~250 行
**验证**：`cargo check` 通过。

### 11.3 Phase 2：ML Engine 替换 Tensor_IO_Handle → Tensor_IO_Endpoint

**目标**：Session 持有新类型，Send/Receive/SendEOF 指令调用新接口。

| # | 文件 | 操作 | 详情 |
|---|------|------|------|
| 2.1 | `Src/ML_Engine/capability.rs` | ✏️ | `ML_Session_Config.tensor_io: Option<Tensor_IO_Handle>` → `Option<Tensor_IO_Endpoint>` |
| 2.2 | `Src/ML_Engine/ml_thread_engine.rs` | ✏️ | `Session.tensor_io` 类型变更；Send/Receive/SendEOF 执行改调 `endpoint.Send()` / `endpoint.Receive()`（接口签名兼容，改动极小） |
| 2.3 | `Src/ML_Engine/ml_inference_service.rs` | ✏️ | `Create_Session` 中 `tensor_io` 参数类型跟随 |
| 2.4 | `Src/ML_Engine/service.rs` | ✏️ | 线程传参类型跟随 |

**关键约束**：`Tensor_IO_Endpoint` 必须 `Send`（跨线程传入 OS 线程）。`Arc<Mutex<_>>` 天然满足。

### 11.4 Phase 3：Orchestrator 重构

**目标**：删除 4 条 TaskInstruction + Tensor_IO_Broker 引用，简化 pipeline 编译，Core 改为"先连接后启动"。

#### 3a. 指令集 + 编译器简化

| # | 文件 | 操作 | 详情 |
|---|------|------|------|
| 3.1 | `Src/Orchestrator/instruction.rs` | ✏️ | **删除** `OpenTensorStream` / `TakeInboundStream` / `TakeOutboundStream` / `BuildTensorIo` 四条指令定义 |
| 3.2 | `Src/Orchestrator/compiler.rs` | ✏️ | `build_coordinator_pipeline` / `build_relay_pipeline` 简化：删除网络指令序列，保留 `Const → CreateSession → RunProgram`；删除 `SLOT_INBOUND` / `SLOT_OUTBOUND` / `SLOT_TENSOR_IO` / `SLOT_TARGET_JOB` 常量；`RequestPipeline` 从 Job 指令移至 Core 层 |
| 3.3 | `Src/Orchestrator/slot.rs` | ✏️ | 删除 `SlotValue::TensorIo` variant 和 `take_tensor_io()` 方法 |
| 3.4 | `Src/Orchestrator/executor/task_engine.rs` | ✏️ | 删除 4 条指令的 match arm |
| 3.5 | `Src/Orchestrator/executor/handler_network.rs` | ✏️ | 删除 `handle_open_tensor_stream` / `handle_take_inbound_stream` / `handle_take_outbound_stream` / `handle_build_tensor_io` |

#### 3b. Capabilities + Core 重构

| # | 文件 | 操作 | 详情 |
|---|------|------|------|
| 3.6 | `Src/Orchestrator/mod.rs` | ✏️ | `Capabilities.tensor_io_broker` → `tensor_switch: Arc<Tensor_Port_Switch>` |
| 3.7 | `Src/Orchestrator/core.rs` | ✏️ | (a) `spawn_relay_job` 改为先建连接 → `switch.Register()` → 注入 endpoint → spawn 纯推理 Job；(b) `handle_network_inbound` 中 TensorStreamArrived 分支改为路由到 Switch；(c) `handle_lifecycle_event` 清理调用 `switch.Deregister()` |
| 3.8 | `Src/Orchestrator/executor/handler_inference.rs` | ✏️ | `handle_create_session` 中 tensor_io 改为从 Job 预注入 config 获取 |

#### Core 层新逻辑伪代码

```rust
async fn spawn_distributed_job(&mut self, ...) {
    let job_id = self.next_job_id();
    // 1. 协商 (原 RequestPipeline handler 逻辑提升到 Core)
    let relay_job_id = self.send_request_pipeline(peer, ...).await?;
    // 2. 建立 tensor stream (原 OpenTensorStream handler 逻辑)
    let mut outbound = self.capabilities.network.open_tensor_stream(peer).await?;
    Write_Tensor_Stream_Handshake(&mut outbound, relay_job_id).await?;
    // 3. 等待 inbound (TensorStreamArrived 事件异步到达)
    let inbound = self.wait_for_tensor_stream(job_id).await?;
    // 4. 注册到 Switch，获得 endpoint
    let endpoint = self.capabilities.tensor_switch
        .Register(job_id, inbound, vec![outbound]).await?;
    // 5. 编译简化 program + spawn
    let program = self.compiler.build_coordinator_pipeline_v2(...);
    self.spawn_job(job_id, program, io_handle, Some(endpoint));
}
```

### 11.5 Phase 4：清理旧代码

| # | 文件 | 操作 | 详情 |
|---|------|------|------|
| 4.1 | `Src/Orchestrator/tensor_io_broker.rs` | 🗑️ 删除 | 整个文件（~240 行） |
| 4.2 | `Src/Network/tensor_stream_protocol.rs` | ✏️ | 删除 `Tensor_IO_Handle` struct + impl + Debug impl（~90 行）；**保留** `Tensor_Buffer` / `Send_Tensor_Frame` / `Receive_Tensor_Frame` / `Send_EOF` / Handshake 函数 |
| 4.3 | `Src/Network/mod.rs` | ✏️ | 移除 `Tensor_IO_Handle` re-export |
| 4.4 | `Src/lib.rs` | ✏️ | 移除 `Tensor_IO_Handle` re-export |

### 11.6 Phase 5：入口 + 测试修复

| # | 文件 | 操作 | 详情 |
|---|------|------|------|
| 5.1 | `Src/main.rs` | ✏️ | `Tensor_IO_Broker::New()` → `Arc::new(Tensor_Port_Switch::New())` |
| 5.2 | `tests/common/mod.rs` | ✏️ | 测试 Capabilities 构造更新 |
| 5.3 | `tests/t08_e2e_inference.rs` | ✏️ | 同上 |
| 5.4 | `tests/t05_ml_engine_integration.rs` | ✏️ | `tensor_io: None` 类型跟随变化 |
| 5.5 | executor 测试辅助代码 | ✏️ | `handler_control.rs` / `handler_data.rs` / `handler_inference.rs` / `executor/mod.rs` / `task_engine.rs` — 5 处 Capabilities 构造 |

### 11.7 推荐执行顺序

```
Phase 1 → Phase 2 → Phase 4.2-4.4 → Phase 3 → Phase 4.1 → Phase 5
```

**理由**：
- Phase 1 (新建) 独立可编译，零风险
- Phase 2 (ML Engine) 改类型，`Tensor_IO_Handle` 尚未删除，两种可暂时共存
- Phase 4.2-4.4 (删除旧 Handle) 在 ML Engine 不再引用后即可
- Phase 3 (Orchestrator 重构) 是最大改动，此时新旧基础设施都已就位
- Phase 4.1 (删除 Broker) 在 Orchestrator 不再引用后
- Phase 5 (测试) 最后统一修复

**每个 Phase 完成后应可通过 `cargo check`。**

### 11.8 代码量估算

| Phase | 新增 | 修改 | 删除 | 净变化 |
|-------|------|------|------|--------|
| Phase 1 | ~250 | ~5 | 0 | +255 |
| Phase 2 | 0 | ~30 | ~5 | +25 |
| Phase 3 | ~80 | ~100 | ~300 | -120 |
| Phase 4 | 0 | ~10 | ~330 | -320 |
| Phase 5 | 0 | ~40 | ~40 | 0 |
| **合计** | **~330** | **~185** | **~675** | **-160** |

净删除 ~160 行，架构更简洁。
