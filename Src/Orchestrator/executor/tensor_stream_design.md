# Tensor Stream 设计文档

**日期**：2026-04-28
**状态**：设计阶段

---

## 1. 问题背景

分布式推理场景中，节点之间需要建立 Tensor Stream（双向张量流）进行推理数据的实时传输。每个节点持有一个 `Tensor_IO_Handle`，包含：
- `inbound_stream`：从上游节点到达的入站流
- `outbound_stream`：主动 open 到下游节点的出站流

### 1.1 环形流水线拓扑

以 Coordinator(C) → Worker1(W1) → Worker2(W2) → C 为例：

```
C.forward(层0-9) → [outbound→W1] → W1.forward(层10-19) → [outbound→W2] → W2.forward(层20-28) → [outbound→C] → C.Sample()
```

| 节点 | outbound（主动 open） | inbound（被动接收） |
|------|----------------------|-------------------|
| C    | C → W1               | W2 → C            |
| W1   | W1 → W2              | C → W1            |
| W2   | W2 → C               | W1 → W2           |

### 1.2 核心架构约束

```
Job 能发（通过 network capability）     ✅
Job 能收（直接接收 inbound）            ❌ → 全走 Core
```

所有 inbound 事件（`TensorStreamArrived`）进入 Core 的 `select!` 循环，Job 无法直接接收。这要求 Core 和 Job 之间有桥接机制来移交入站资源。

---

## 2. 设计方案：Tensor_IO_Broker + Handshake

### 2.1 核心思想

类似 `LLM_IO_Broker` 的 `Allocate → Take` 模式，创建 `Tensor_IO_Broker` 作为 Core 和 Job 之间的共享信箱：

- **Job** 负责打开 outbound（`OpenTensorStream` 指令），存入 Broker
- **Core** 负责接收 inbound（`TensorStreamArrived` 事件），存入 Broker
- **Broker** 负责收集两条流、组装 `Tensor_IO_Handle`、通知等待的 Job
- **Job** 通过 `TakeTensorIo` 指令从 Broker 取出组装好的 handle

### 2.2 JobId 路由

Broker 使用 **JobId**（而非 PeerId）作为路由 key，解决一台机器同时运行多个 pipeline 时的歧义问题。

JobId 通过以下协议交换：
1. Coordinator 发送 `PipelineFlow` 命令时携带自己的 `coordinator_job_id` 和下游的 `next_job_id`
2. 远端 Core 收到后 spawn Relay Job，在 response 中返回 `my_job_id`
3. tensor stream 建立时，handshake 帧携带 `target_job_id`（接收方的 JobId）
4. 接收方 Core 从 handshake 读取 `target_job_id` → `broker.Store_Inbound(job_id, stream)`

---

## 3. 涉及的指令

### 3.1 OpenTensorStream — 打开出站流并存入 Broker

```rust
OpenTensorStream {
    peer: SlotId,       // 下游 PeerId
}
```

handler 职责：
1. 从 slot 读取下游 PeerId
2. 调用 `network.open_tensor_stream(peer)` → 得到 outbound stream
3. 在 stream 上写入 handshake 帧（target_job_id，即下游 Job 的 ID）
4. 调用 `broker.Store_Outbound(self.job_id, stream)` 将 outbound 存入 Broker

注意：下游 Job 的 ID 需在此之前通过 PipelineFlow 协议获取并存入 slot。

### 3.2 TakeInboundStream — 从 Broker 取出入站流

```rust
TakeInboundStream {
    result: SlotId,     // 输出：SlotValue::Stream（inbound raw stream）
}
```

handler 职责：
1. 调用 `broker.Take_Inbound(self.job_id).await`（阻塞直到 inbound 到达）
2. 将 raw `libp2p::Stream` 存入 result 槽位

### 3.3 TakeOutboundStream — 从 Broker 取出出站流

```rust
TakeOutboundStream {
    result: SlotId,     // 输出：SlotValue::Stream（outbound raw stream）
}
```

handler 职责：
1. 调用 `broker.Take_Outbound(self.job_id).await`（阻塞直到 outbound 就绪）
2. 将 raw `libp2p::Stream` 存入 result 槽位

### 3.4 BuildTensorIo — 组装 Tensor_IO_Handle

```rust
BuildTensorIo {
    inbound: SlotId,    // TakeInboundStream 的输出
    outbound: SlotId,   // TakeOutboundStream 的输出
    result: SlotId,     // 输出：SlotValue::TensorIo
}
```

handler 职责：
1. 从 slot take 两条 raw stream
2. 调用 `Tensor_IO_Handle::New(inbound, outbound, rt)` 组装
3. 将 handle 存入 result 槽位

---

## 4. Tensor_IO_Broker 设计

### 4.1 设计思想

Broker 只负责**存储和取出** raw stream，**不负责组装** `Tensor_IO_Handle`。
组装由 `BuildTensorIo` 指令在 Job 内完成。

每条流独立存取：
- `Store_Inbound` / `Take_Inbound` — inbound 流（Core 存，Job 取）
- `Store_Outbound` / `Take_Outbound` — outbound 流（Job 存，Job 取）

### 4.2 数据结构

```rust
pub struct Tensor_IO_Broker {
    entries: Mutex<HashMap<JobId, TensorIoEntry>>,
}

struct TensorIoEntry {
    inbound: Option<libp2p::Stream>,
    outbound: Option<libp2p::Stream>,
    inbound_notify: Arc<Notify>,    // inbound 到达时通知
    outbound_notify: Arc<Notify>,   // outbound 到达时通知
}
```

### 4.3 API

```rust
impl Tensor_IO_Broker {
    /// Core 在 spawn 分布式 Job 时调用，注册 job_id
    pub async fn Prepare(&self, job_id: JobId) { ... }

    /// Core 收到 TensorStreamArrived 时调用
    /// 从 stream handshake 读取 target_job_id 后调用此方法
    /// 存入 inbound，通知等待的 Job
    pub async fn Store_Inbound(&self, job_id: JobId, stream: libp2p::Stream) {
        let mut map = self.entries.lock().await;
        if let Some(entry) = map.get_mut(&job_id) {
            entry.inbound = Some(stream);
            entry.inbound_notify.notify_one();
        }
    }

    /// Job 的 OpenTensorStream handler 调用
    /// 存入 outbound，通知等待的 Job
    pub async fn Store_Outbound(&self, job_id: JobId, stream: libp2p::Stream) {
        let mut map = self.entries.lock().await;
        if let Some(entry) = map.get_mut(&job_id) {
            entry.outbound = Some(stream);
            entry.outbound_notify.notify_one();
        }
    }

    /// Job 的 TakeInboundStream handler 调用
    /// 阻塞直到 inbound stream 到达
    pub async fn Take_Inbound(&self, job_id: JobId) -> Result<libp2p::Stream, Error> {
        loop {
            let notify = {
                let mut map = self.entries.lock().await;
                let entry = map.get_mut(&job_id).ok_or("not found")?;
                if let Some(stream) = entry.inbound.take() {
                    return Ok(stream);
                }
                entry.inbound_notify.clone()
            };
            notify.notified().await;
        }
    }

    /// Job 的 TakeOutboundStream handler 调用
    /// 阻塞直到 outbound stream 就绪
    pub async fn Take_Outbound(&self, job_id: JobId) -> Result<libp2p::Stream, Error> {
        loop {
            let notify = {
                let mut map = self.entries.lock().await;
                let entry = map.get_mut(&job_id).ok_or("not found")?;
                if let Some(stream) = entry.outbound.take() {
                    return Ok(stream);
                }
                entry.outbound_notify.clone()
            };
            notify.notified().await;
        }
    }

    /// Core 在 Job 结束后调用，清理内部索引
    pub async fn Deallocate(&self, job_id: JobId) { ... }
}
```

---

## 5. JobId 交换协议

### 5.1 PipelineFlow 命令扩展

```rust
Pipeline_Flow {
    next_peer: PeerId,       // 下游 Peer
    next_job_id: u64,        // 下游 Job 的 ID（供 Open 时 handshake 使用）
    coordinator_job_id: u64, // Coordinator 的 JobId（供末端 Worker 回连时使用）
}
```

Response 扩展：
```
"OK|<my_job_id>"   // 返回新 spawn 的 Relay Job 的 JobId
```

### 5.2 PipelineFlow 发送顺序

Coordinator 从末端向前发（先 W2 再 W1），保证每一步能拿到下游的 JobId：

```
1. C → W2: PipelineFlow { next_peer: C,  next_job_id: C_JOB_ID(100) }
   W2 响应: "OK|300"（W2 的 Relay Job ID）

2. C → W1: PipelineFlow { next_peer: W2, next_job_id: 300 }
   W1 响应: "OK|200"（W1 的 Relay Job ID）

至此所有 JobId 已交换完毕。
```

### 5.3 Tensor Stream Handshake

发起方在 open stream 后写入固定格式 handshake 帧（8 字节）：

```
[target_job_id: u64 little-endian]
```

接收方 Core 从入站 stream 读取 8 字节 → 解析 target_job_id → `broker.Store_Inbound(job_id, stream)`。

---

## 6. 完整时序图

### 6.1 Coordinator 启动 → 全部就绪

```
Coordinator (C, JobId=100)              Worker1  (W1)              Worker2  (W2)
       │                                    │                          │
       ├── AnalyzeModel                     │                          │
       ├── SplitModel                       │                          │
       ├── SendFile(W1)                 ────→ 收到模型分片              │
       ├── SendFile(W2)                     │                      ────→ 收到模型分片
       │                                    │                          │
       ├── PipelineFlow(W2,next=C,100)      │                      ────→ spawn Relay(300)
       │←─ Response "OK|300"                │                          │
       │                                    │                          │
       ├── PipelineFlow(W1,next=W2,300)────→ spawn Relay(200)          │
       │←─ Response "OK|200"                │                          │
       │                                    │                          │
  [所有 JobId 已交换]                       │                          │
       │                                    │                          │
       ├── OpenTensorStream(W1)          ────→ handshake[target=200]   │
       │   → broker.Store_Outbound(100)     │  → broker.Store_Inbound(200)
       │                                    │                          │
       │                                    ├── OpenTensorStream(W2)────→ handshake[target=300]
       │                                    │   → broker.Store_Outbound(200) → broker.Store_Inbound(300)
       │                                    │                          │
       │                                    │                          ├── OpenTensorStream(C)
       │   ←────────────────────────────────────────────── handshake[target=100]
       │   broker.Store_Inbound(100)        │                          │  → broker.Store_Outbound(300)
       │                                    │                          │
       ├── TakeInboundStream               ├── TakeInboundStream      ├── TakeInboundStream
       │   broker.Take_Inbound(100) ✅     │   broker.Take_Inbound(200) ✅  broker.Take_Inbound(300) ✅
       ├── TakeOutboundStream              ├── TakeOutboundStream     ├── TakeOutboundStream
       │   broker.Take_Outbound(100) ✅    │   broker.Take_Outbound(200) ✅ broker.Take_Outbound(300) ✅
       ├── BuildTensorIo                    ├── BuildTensorIo          ├── BuildTensorIo
       │                                    │                          │
       ├── CreateSession(tensor_io)         ├── CreateSession(tensor_io) ├── CreateSession(tensor_io)
       ├── RunProgram(coordinator)          ├── RunProgram(relay)       ├── RunProgram(relay)
```

---

## 7. Compiler 指令序列

### 7.1 Coordinator（compile_coordinator）

```
正向序列：
  Const(model_path)                         → SLOT_MODEL
  AnalyzeModel { model: SLOT_MODEL }        → SLOT_MODEL_INFO
  SplitModel { ... }
  SendFile(to W1)
  SendFile(to W2)
  SendPipelineFlow(W2, next=C, 100)         → SLOT_W2_JOB_ID  (从 response 解析)
  SendPipelineFlow(W1, next=W2, W2_JOB_ID)  → SLOT_W1_JOB_ID  (从 response 解析)
  OpenTensorStream { peer: SLOT_W1 }
  TakeInboundStream { result: SLOT_INBOUND }
  TakeOutboundStream { result: SLOT_OUTBOUND }
  BuildTensorIo { inbound: SLOT_INBOUND, outbound: SLOT_OUTBOUND, result: SLOT_TENSOR_IO }
  Const(device)                             → SLOT_DEVICE
  CreateSession { ..., tensor_io: Some(SLOT_TENSOR_IO) }
  RunProgram { session: SLOT_SESSION }

补偿序列：
  ShutdownSession { session: SLOT_SESSION }
```

### 7.2 Relay/Worker（compile_relay）

```
正向序列：
  ReceiveFile { result: SLOT_MODEL }
  OpenTensorStream { peer: SLOT_NEXT_PEER }
  TakeInboundStream { result: SLOT_INBOUND }
  TakeOutboundStream { result: SLOT_OUTBOUND }
  BuildTensorIo { inbound: SLOT_INBOUND, outbound: SLOT_OUTBOUND, result: SLOT_TENSOR_IO }
  Const(device)                              → SLOT_DEVICE
  CreateSession { ..., tensor_io: Some(SLOT_TENSOR_IO) }
  RunProgram { session: SLOT_SESSION }

补偿序列：
  ShutdownSession { session: SLOT_SESSION }
```

### 7.3 单机 Run（compile_run）— 无变化

```
正向序列：
  Const(model) → Const(device) → Const(start) → Const(end)
  → CreateSession { tensor_io: None }
  → RunProgram

补偿序列：
  ShutdownSession
```

---

## 8. 需要的代码改动清单

### 8.1 新增文件
- `Src/Orchestrator/tensor_io_broker.rs` — Tensor_IO_Broker 实现

### 8.2 修改文件
- `Src/Orchestrator/mod.rs` — Capabilities 新增 `tensor_io_broker` 字段
- `Src/Orchestrator/instruction.rs` — 新增 `OpenTensorStream`（修改签名）、`TakeInboundStream`、`TakeOutboundStream`、`BuildTensorIo`、`SendPipelineFlow` 指令
- `Src/Orchestrator/executor/task_engine.rs` — step() 路由新指令
- `Src/Orchestrator/executor/handler_network.rs` — 实现 `handle_open_tensor_stream`（写 handshake + 存 broker）、`handle_take_inbound_stream`、`handle_take_outbound_stream`、`handle_build_tensor_io`
- `Src/Orchestrator/core.rs` — `handle_network_inbound` 实现 TensorStreamArrived 路由（读 handshake → broker.Store_Inbound）
- `Src/Orchestrator/compiler.rs` — `compile_coordinator` 和 `compile_relay` 生成新指令序列
- `Src/Orchestrator/slot.rs` — 新增 `SlotValue::TensorIo(Mutex<Option<Tensor_IO_Handle>>)` 变体 + `take_tensor_io()` 方法
- `Src/Control/network_control_command.rs` — `Pipeline_Flow` 扩展字段（next_job_id、coordinator_job_id）
- `Src/Network/tensor_stream_protocol.rs` — handshake 帧读写函数
- `Src/Orchestrator/executor/handler_inference.rs` — `CreateSession` 支持可选 `tensor_io` 槽位

### 8.3 优先级
1. **P0**：Tensor_IO_Broker 核心（Prepare/Store_Inbound/Store_Outbound/Take_Inbound/Take_Outbound/Deallocate）
2. **P0**：tensor stream handshake 帧格式
3. **P1**：handler_network 实现（OpenTensorStream + TakeInboundStream + TakeOutboundStream + BuildTensorIo）
4. **P1**：Core 路由逻辑（TensorStreamArrived → read handshake → broker）
5. **P2**：PipelineFlow 命令扩展 + compile_coordinator/compile_relay
