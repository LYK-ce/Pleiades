# Request_Response 设计文档

Presented by KeJi
Date ： 2026-05-16

## 1. 模块概述

`Request_Response` 是 Network 模块内的**请求-响应传输子系统**，与 `File_Stream`、`Tensor_Stream` 并列，为 Network 提供三种传输机制之一。

### 核心定义

> **Request_Response = TLV 编解码 + 入站路由管理 + 出站响应路由。**
> 基于 libp2p `request_response` 行为，采用统一 TLV 帧格式，网络层只搬运字节流，不关心上层语义。
> 入站请求按 DataType 预筛选分流，出站响应通过 oneshot 回传调用方。

### 模块结构

```
Network/Request_Response/
├── mod.rs               ← 子模块入口 + re-export
├── codec.rs             ← TLV编解码器、DataType枚举、Network_Data帧、PleiadesCodec
├── inbound.rs           ← 入站请求路由管理 (Inbound_Manager)
└── outbound.rs          ← 出站响应路由管理 (Outbound_Manager)
```

### 调用关系

```
Orchestrator / Network_Service
         │
         ▼
  Network_Capability (send_data / send_response)
         │
         ├── NodeHandle.Send_Data ──→ cmd_tx ──→ Network_Service.Handle_Command
         │                                              │
         │                              ┌───────────────┴───────────────┐
         │                              ▼                               ▼
         │                     Inbound_Manager                   Outbound_Manager
         │                     (request_id→Channel)             (request_id→oneshot)
         │
         └── NodeHandle.Send_Response ──→ Network_Service.Handle_Command
                                                    │
                                                    ▼
                                           Inbound_Manager.Take_Reply_Channel
```

所有数据帧通过 `PleiadesCodec` 统一编解码，线上格式为 TLV（Type-Length-Value）。

---

## 2. 数据结构

### 2.1 DataType — 数据类型标记

```rust
#[repr(u8)]
enum DataType {
    Command       = 0,  // 命令/控制消息 → 转发给 Orchestrator
    Data          = 1,  // 数据（张量、中间结果）→ Network 内部回复 OK
    File          = 2,  // 文件通知（元数据协商）→ 转发给 Orchestrator
    Info          = 4,  // 信息通知 → Network 内部回复 OK
    BandwidthTest = 5,  // 带宽测试 → Network 内部回显
}
```

DataType 决定入站请求的分流策略：
- `Command` / `File` → Inbound_Manager 转发给 Orchestrator Core
- `BandwidthTest` / `Data` / `Info` → Network_Service 内部直接回复

### 2.2 Network_Data — 统一网络数据帧

```rust
struct Network_Data {
    data_type: DataType,  // 1 byte on wire
    payload: Vec<u8>,      // length bytes on wire
}
```

Request 和 Response 使用完全相同的结构体，线上格式一致。网络层只搬运字节，上层（Orchestrator）负责 payload 的序列化/反序列化。

**线上帧格式**：
```
+----------+------------------+---------------------+
|  Type    |  Length           |  Payload             |
|  1 byte  |  8 bytes BE u64  |  Length bytes        |
+----------+------------------+---------------------+
```

最大帧大小限制：2 GB。

### 2.3 InboundRequest — 入站请求

```rust
struct InboundRequest {
    request_id: u64,       // Network_Service 内部分配的请求编号
    peer: PeerId,          // 发送方节点 ID
    data_type: DataType,   // 数据类型标记
    payload: Vec<u8>,      // 原始载荷字节流
}
```

不包含 libp2p 内部类型（ResponseChannel）。Orchestrator 通过 `request_id` 调用 `send_response` 回复。

### 2.4 NodeCommand（请求-响应相关部分）

```rust
enum NodeCommand {
    SendData {
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
        response_tx: Option<oneshot::Sender<Result<Network_Data, String>>>,
    },
    SendResponse {
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    },
    // ... 其他命令
}
```

`response_tx` 为 `None` 时表示 fire-and-forget，为 `Some` 时等待对方回复。

### 2.5 Inbound_Manager — 入站请求管理器

```rust
struct Inbound_Manager {
    pending_replies: HashMap<u64, ResponseChannel<Network_Data>>,  // request_id → 回复通道
    inbound_tx: mpsc::Sender<InboundRequest>,                       // 转发通道（→ Orchestrator）
    next_inbound_id: u64,                                           // ID 自增计数器
}
```

核心方法：
- `Register_Inbound(peer, request, channel) → request_id` — 分配 ID、存储 Channel、转发给 Orchestrator
- `Take_Reply_Channel(request_id) → Option<ResponseChannel>` — 取出 Channel 供 Send_Response 使用
- `Clear_All()` — 节点停止时清空所有 pending 状态

### 2.6 Outbound_Manager — 出站响应路由管理器

```rust
struct Outbound_Manager {
    pending_responses: HashMap<OutboundRequestId, oneshot::Sender<Result<Network_Data, String>>>,
}
```

核心方法：
- `Register_Outbound(outbound_id, response_tx)` — 注册出站请求的回传通道
- `Route_Response(outbound_id, response)` — 收到 Response 时回传
- `Route_Failure(outbound_id, error)` — 发送失败时通知
- `Clear_All()` — 节点停止时通知所有等待方

---

## 3. 核心实现

### 3.1 TLV 编解码流程

```
发送方                                接收方
─────                                ─────
Network_Data { type, payload }       网络字节流
    │                                    │
    ▼                                    ▼
Write_Frame(io, data)              Read_Frame(io)
  ├── write_all([type as u8])        ├── read_exact(1) → DataType
  ├── write_all(length.to_be())      ├── read_exact(8) → length (BE u64)
  ├── write_all(payload)             ├── 校验 length ≤ 2GB
  └── flush()                        ├── read_exact(length) → payload
                                     └── Network_Data { type, payload }
```

### 3.2 PleiadesCodec — libp2p Codec 适配

```rust
impl Codec for PleiadesCodec {
    type Request  = Network_Data;
    type Response = Network_Data;

    fn read_request  → Box::pin(Read_Frame(io))
    fn read_response → Box::pin(Read_Frame(io))
    fn write_request  → Box::pin(Write_Frame(io, req))
    fn write_response → Box::pin(Write_Frame(io, res))
}
```

Request 和 Response 使用相同的 `Network_Data` 类型和相同的 `Read_Frame`/`Write_Frame` 函数，不做任何业务序列化。

### 3.3 入站请求路由流程

```
Network_Service.Handle_Request_Response_Event
    │
    ├── Message::Request { peer, request, channel }
    │       │
    │       ├── DataType::BandwidthTest → Handle_Bandwidth_Test_Inbound (内部回显)
    │       ├── DataType::Data         → 回复 OK (内部)
    │       ├── DataType::Info         → 回复 OK (内部)
    │       └── DataType::Command|File → Inbound_Manager.Register_Inbound
    │           ├── 分配 request_id
    │           ├── 存储 ResponseChannel
    │           ├── 创建 InboundRequest
    │           └── 发送到 inbound_tx → Orchestrator Core 接收处理
    │
    └── Message::Response { request_id, response }
            └── Outbound_Manager.Route_Response(request_id, response)
                └── oneshot.tx.send(Ok(response)) → Send_Data 调用方收到
```

### 3.4 出站请求-响应流程

```
调用方                           Network_Service                    远端节点
─────                           ───────────────                    ──────
network.send_data(peer,           │                                  │
    type, payload)                │                                  │
    │                             │                                  │
    ├── oneshot::channel()        │                                  │
    ├── cmd_tx.send(SendData {    │                                  │
    │     peer, type, payload,    │                                  │
    │     response_tx: Some(tx)   │                                  │
    │   })                        │                                  │
    │                             ├── Handle_Command(                │
    │                             │     SendData)                    │
    │                             │   ├── swarm.behaviour_mut()      │
    │                             │   │   .request_response          │
    │                             │   │   .send_request(peer, data)  │
    │                             │   └── Outbound_Manager           │
    │                             │       .Register_Outbound(        │
    │                             │         outbound_id, tx)         │
    │                             │                                  │
    │                             │  ──────────────网络──────────→   │
    │                             │                                  ├── 入站 Request
    │                             │                                  ├── 处理
    │                             │  ←──────────────网络──────────   ├── 发送 Response
    │                             │                                  │
    │                             ├── Message::Response {            │
    │                             │     request_id: outbound_id,     │
    │                             │     response }                   │
    │                             │   └── Outbound_Manager            │
    │                             │       .Route_Response(id, resp)  │
    │                             │                                  │
    ├── rx.await                  │                                  │
    └── Ok(response)              │                                  │
```

---

## 4. 模块导出

```rust
// Request_Response/mod.rs
pub mod codec;
pub mod inbound;
pub mod outbound;

pub use codec::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use inbound::Inbound_Manager;
pub use outbound::Outbound_Manager;
```

Network 层重新导出：
```rust
// Network/mod.rs
pub mod request_response;
pub use request_response::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
```

---

## 5. 与 Network_Service 的协作

| 职责 | 负责方 | 说明 |
|------|--------|------|
| TLV 编解码 | codec.rs | libp2p Codec trait 实现，Swarm 自动调用 |
| DataType 预筛选分流 | network_service.rs | Handle_Request_Response_Event 中按 DataType 分流 |
| 入站请求存储与转发 | inbound.rs | 分配 request_id，存储 ResponseChannel，转发给 Orchestrator |
| 出站响应回传 | outbound.rs | oneshot 路由回 Send_Data 调用方 |
| Send_Data / Send_Response API | node_handle.rs | 通过 mpsc 向 Network_Service 发送 NodeCommand |
| 统一 Capability 接口 | capability.rs | Network_Capability trait 封装 |

### 请求生命周期

```
1. 远端发起请求 → libp2p Swarm 接收 → PleiadesCodec 解码为 Network_Data
2. Network_Service 按 DataType 分流：
   - Command/File → Inbound_Manager.Register_Inbound → Orchestrator
   - 其他 → Network_Service 内部直接回复
3. Orchestrator 处理完毕后 → capability.send_response(request_id, type, payload)
4. NodeHandle.Send_Response → cmd_tx → Network_Service
5. Network_Service → Inbound_Manager.Take_Reply_Channel → swarm.send_response
6. PleiadesCodec 编码为 TLV → 发送回远端
```

---

## 6. 已知风险

### 6.1 request_id 溢出

`next_inbound_id` 从 1 开始自增，永不回收。理论上在 `u64` 范围内足够安全，但长期运行的节点若收到极端高频入站请求（如每秒百万次），可能在数百年后溢出。当前场景（局域网低频命令/文件协商）无此风险。

### 6.2 inbound_tx 通道满

`inbound_tx` 通道容量为 100。若 Orchestrator 处理速度慢于 Network 入站请求到达速度，`Register_Inbound` 中的 `send().await` 会阻塞 Network 事件循环，导致 Swarm 事件积压。当前通过 tokio::select! 的公平调度缓解，但高频入站场景需要调整通道容量。

### 6.3 oneshot 超时消息丢失

`Send_Data` 的 Response 等待超时（默认 300s）后，调用方收到 `Timeout` 错误，但远端可能仍在处理并发回 Response。此时 `Outbound_Manager.Route_Response` 会发现 `pending_responses` 中无匹配 entry，仅记录 debug 日志。Response 数据被丢弃，无业务影响但浪费带宽。

### 6.4 2GB 帧大小限制

单帧最大 2GB。对于超大模型文件的元数据传输（如千层模型的层位图），2GB 足够。但若未来需要在单帧中传输完整张量数据，可能需要改用 Tensor_Stream 或 File_Stream。

---

## 7. 重构历史

从 v1 到 v2（本次）的变更：

| 变更 | 说明 |
|------|------|
| 提取 Request_Response 子目录 | `data_protocol.rs` → `Request_Response/codec.rs` |
|  | `inbound_manager.rs` → `Request_Response/inbound.rs` |
|  | `outbound_manager.rs` → `Request_Response/outbound.rs` |
| 新建 Request_Response/mod.rs | 子模块入口 + re-export |
| 更新 Network/mod.rs | 删除 3 个 `pub mod`，新增 `pub mod request_response` |
| 更新 Network 内 imports | `super::data_protocol` → `super::request_response::codec` 等 |

无外部接口变更。`Network/mod.rs` 的 re-export 路径保持兼容，`DataType` / `Network_Data` / `PleiadesCodec` / `DATA_PROTOCOL` 对外暴露路径不变。

---

## 8. TODO

暂无。
