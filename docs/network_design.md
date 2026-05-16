# Network 设计文档

Presented by KeJi
Date ： 2026-05-16

## 1. 模块概述

`Network` 模块负责 Pleiades 分布式推理系统的 **P2P 网络通信**。基于 libp2p 构建，提供 TCP + mDNS 局域网环境下的节点发现、连接管理、请求-响应、流式传输。

### 核心定义

> **Network = Swarm 编排 + 四种传输机制 + 对上层 Capability 接口。**
> 事件循环（`tokio::select!`）统一调度 Swarm 事件、外部命令、入站流。
> 四种传输机制：Request-Response / File_Stream / Tensor_Stream / Bandwidth_Stream。
> 对外暴露 `Network_Capability` trait，Orchestrator 和 Lua 脚本通过此接口调用网络操作。

### 模块结构

```
Network/
├── mod.rs                  ← 模块入口 + re-export
├── network_service.rs      ← Network_Service（Init + Start 事件循环编排）
├── swarm_events.rs         ← Swarm 事件处理（分发器 + Mdns/Kademlia/RR/Ping handler）
├── command_handler.rs      ← 外部命令处理（Handle_Command）
├── capability.rs           ← Network_Capability trait + Network_Service_Capability 实现
├── node_handle.rs          ← NodeHandle API + NodeCommand 枚举 + InboundRequest
├── Request_Response/       ← 请求-响应传输子系统
│   ├── mod.rs
│   ├── codec.rs            ← TLV 编解码 / DataType / Network_Data / PleiadesCodec
│   ├── inbound.rs          ← 入站路由管理 (Inbound_Manager)
│   └── outbound.rs         ← 出站响应路由 (Outbound_Manager)
├── File_Stream/            ← 文件流传输子系统
│   ├── mod.rs
│   └── protocol.rs         ← Header / ACK / Send_File_Data / Receive_File_Data
├── Tensor_Stream/          ← 张量流传输子系统
│   ├── mod.rs
│   ├── protocol.rs         ← 帧格式 / Handshake / Tensor_Buffer / EOF 哨兵
│   └── rendezvous.rs       ← 双向流匹配器 (cross-context handoff)
└── Bandwidth_Stream/       ← 带宽测速流子系统
    ├── mod.rs
    └── protocol.rs         ← iperf 风格固定时长推流测速
```

### 调用关系

```
Orchestrator / Lua 脚本
         │
         ▼
  Box<dyn Network_Capability>         ← capability.rs (trait)
         │
         ├── send_data / send_response  → NodeHandle → cmd_tx → Handle_Command
         ├── dial / disconnect          → NodeHandle → cmd_tx → Handle_Command
         ├── put_record / get_record    → NodeHandle → cmd_tx → Handle_Command
         ├── open_file_stream           → stream::Control (直接)
         ├── open_tensor_stream         → stream::Control (直接)
         └── test_bandwidth             → stream::Control (直接)
         │
         ▼
  Network_Service 事件循环              ← network_service.rs
         │
         ├── Swarm 事件 → swarm_events.rs
         ├── 外部命令   → command_handler.rs
         ├── 入站文件流  → orchestrator_event_tx → Orchestrator
         ├── 入站张量流  → rendezvous → Orchestrator
         └── 入站带宽流  → Receive_And_Count + 回传结果
```

---

## 2. 数据结构

### 2.1 NetworkConfig — 网络配置

```rust
pub struct NetworkConfig {
    pub lan_enabled: bool,              // 局域网发现 (mDNS)
    pub wan_enabled: bool,              // 广域网发现 (Kademlia)
    pub transport_protocol: String,     // "TCP" 或 "QUIC"
    pub listen_port: u16,               // 0 = 随机端口
    pub bootstrap_peers: Vec<String>,   // 引导节点
    pub cleanup_interval: u64,          // 清理间隔（秒）
    pub timeout_interval: u64,          // 超时间隔（秒）
    pub heartbeat_interval: u64,        // 心跳间隔（秒）
    pub heartbeat_timeout: u64,         // 心跳超时（秒）
    pub request_response_timeout: u64,  // Request-Response 超时（秒）
}
```

默认值：Lan 启用、WAN 禁用、TCP、300s 清理/超时/RR 超时、60s 心跳间隔、10s 心跳超时。

### 2.2 PleiadesNetworkBehaviour — libp2p 行为组合

```rust
#[derive(NetworkBehaviour)]
pub struct PleiadesNetworkBehaviour {
    pub mdns: mdns::tokio::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub request_response: request_response::Behaviour<PleiadesCodec>,
    pub stream: stream::Behaviour,
    pub ping: ping::Behaviour,
}
```

5 种 libp2p 行为：mDNS 发现、Kademlia DHT、请求-响应（统一 `DATA_PROTOCOL`）、流式传输、Ping 心跳。

### 2.3 Network_Service — 核心服务实例

```rust
pub struct Network_Service {
    pub(crate) swarm: Swarm<PleiadesNetworkBehaviour>,
    pub(crate) local_peer_id: PeerId,
    pub(crate) peer_handle: Box<dyn Peer_Management_Capability>,
    pub(crate) cmd_rx: mpsc::Receiver<NodeCommand>,
    pub(crate) config: NetworkConfig,
    pub(crate) event_bus: Arc<EventBus>,

    // 组件化管理器
    pub(crate) inbound_manager: Inbound_Manager,
    pub(crate) outbound_manager: Outbound_Manager,

    // Orchestrator 事件转发
    pub(crate) orchestrator_event_tx: mpsc::Sender<Network_Inbound_Event>,

    // 流控制句柄（仅 accept 入站流）
    pub(crate) file_accept_control: stream::Control,
    pub(crate) tensor_accept_control: stream::Control,
    pub(crate) bandwidth_accept_control: stream::Control,

    // 张量流匹配器
    pub(crate) rendezvous: Arc<RendezvousMap>,
}
```

13 个字段，分 5 组：核心运行时、组件管理器、事件转发、流控制、Rendezvous。

### 2.4 Network_Capability — 对外 Trait

```rust
#[async_trait]
pub trait Network_Capability: Send + Sync {
    // 请求-响应（委托 NodeHandle）
    async fn send_data(&self, peer: PeerId, data_type: DataType, payload: Vec<u8>)
        -> Result<Network_Data, Network_Error>;
    async fn send_response(&self, request_id: u64, data_type: DataType, payload: Vec<u8>)
        -> Result<(), Network_Error>;

    // 连接管理
    async fn dial(&self, addr: Multiaddr) -> Result<(), Network_Error>;
    async fn disconnect(&self, peer: PeerId) -> Result<(), Network_Error>;

    // 文件流（直接操作 stream::Control）
    async fn open_file_stream(&self, peer: PeerId) -> Result<libp2p::Stream, Network_Error>;
    async fn send_file_data(&self, stream: &mut libp2p::Stream, file_path: &Path)
        -> Result<(), Network_Error>;
    async fn receive_file_data(&self, stream: &mut libp2p::Stream, dest_path: &Path, file_size: u64)
        -> Result<(), Network_Error>;

    // 张量流（直接操作 stream::Control + RendezvousMap）
    async fn open_tensor_stream(&self, peer: PeerId, inference_id: u64)
        -> Result<libp2p::Stream, Network_Error>;
    async fn accept_tensor_stream(&self, inference_id: u64, timeout_secs: u64)
        -> Result<libp2p::Stream, Network_Error>;

    // DHT
    async fn put_record(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Network_Error>;
    async fn get_record(&self, key: Vec<u8>) -> Result<(), Network_Error>;

    // 带宽测试（直接操作 stream::Control）
    async fn test_bandwidth(&self, peer: PeerId) -> Result<u64, Network_Error>;
}
```

13 个方法，按传输机制分为 6 组。

### 2.5 NodeCommand — 命令枚举

```rust
pub enum NodeCommand {
    SendData { peer, data_type, payload, response_tx },
    SendResponse { request_id, data_type, payload },
    PutRecord { key, value },
    GetRecord { key },
    Dial { addr },
    Disconnect { peer },
    Stop,
}
```

7 个变体。`SendData` 支持可选 `response_tx`（fire-and-forget 或等待 Response）。

---

## 3. 事件循环架构

### 3.1 初始化流程

```
Init(config, keypair, peer_handle, event_bus)
  ├── 创建通道: cmd_tx/rx, inbound_tx/rx, orchestrator_event_tx/rx
  ├── SwarmBuilder (TCP + noise + yamux + 5 behaviours)
  ├── 创建 6 个 stream::Control
  │     ├── file_accept/open → Network_Service / Capability
  │     ├── tensor_accept/open → Network_Service / Capability
  │     └── bandwidth_accept/stream → Network_Service / Capability
  ├── 创建 Inbound_Manager + Outbound_Manager
  ├── 创建 NodeHandle + RendezvousMap + Network_Service_Capability
  └── 返回 (Network_Service, NodeHandle, inbound_rx, Capability, orchestrator_event_rx)
```

### 3.2 事件循环（Start）

```
tokio::select! 5 个分支:
  ├── SwarmEvent → self.Handle_Swarm_Event(event)   → swarm_events.rs
  ├── NodeCommand → self.Handle_Command(cmd)         → command_handler.rs
  ├── 入站文件流  → orchestrator_event_tx.send(...)  → Orchestrator
  ├── 入站张量流  → Read_Tensor_Stream_Handshake     → rendezvous
  └── 入站带宽流  → Receive_And_Count + Write_Bandwidth_Result
```

### 3.3 事件分发规则

| 事件类型 | 处理方 | 说明 |
|---------|--------|------|
| mDNS 发现/离开 | `swarm_events.rs` | 更新 Kademlia + 发布 EventBus 事件 |
| Kademlia 查询结果 | `swarm_events.rs` | 仅记录日志 |
| 入站 Request (Command/File) | `swarm_events.rs` → Inbound_Manager → Orchestrator | 转发给上层决策 |
| 入站 Request (Data/Info) | `swarm_events.rs` | Network 内部回复 "OK" |
| 出站 Response | `swarm_events.rs` → Outbound_Manager | oneshot 回传调用方 |
| OutboundFailure | `swarm_events.rs` → Outbound_Manager | 通知等待方失败 |
| 连接建立/断开 | `swarm_events.rs` | 更新 PeerManager + 发布 EventBus |
| Ping 心跳 | `swarm_events.rs` | 更新 PeerManager 延迟/状态 |
| 入站文件流 | `network_service.rs (select!)` | 转发给 Orchestrator |
| 入站张量流 | `network_service.rs (select!)` | rendezvous 匹配 |
| 入站带宽流 | `network_service.rs (select!)` | 计数 + 回传 |

### 3.4 命令处理规则

| 命令 | 操作 |
|------|------|
| `SendData` | `swarm.request_response.send_request()` + 可选 outbound_manager 注册 |
| `SendResponse` | `inbound_manager.Take_Reply_Channel()` + `swarm.request_response.send_response()` |
| `PutRecord` | `swarm.kademlia.put_record()` |
| `GetRecord` | `swarm.kademlia.get_record()` |
| `Dial` | `swarm.dial(addr)` |
| `Disconnect` | `swarm.disconnect_peer_id(peer)` + peer_handle.Remove_Peer |
| `Stop` | 断开所有连接 + 清空 PeerManager + 清空 pending → `return false` |

---

## 4. 四种传输机制

### 4.1 Request-Response

统一 TLV 帧格式 (`PleiadesCodec`)，通过 `request_response::Behaviour` 承载。`DataType` 决定分流策略。

```
入站 Request:
  Command / File → Inbound_Manager → Orchestrator
  Data / Info    → Network 内部回复 "OK"
```

### 4.2 File_Stream

流协议 `/pleiades/file-stream/1.0.0`。元数据（文件名、大小、校验）通过 Request-Response 协商，流中只传分块 raw data。Header + ACK 握手、64KB 分块。

### 4.3 Tensor_Stream

流协议 `/pleiades/tensor-stream/1.0.0`。持久化 stream，多帧格式（offset+length+data）、`offset=u64::MAX` EOF 哨兵、`inference_id` Handshake。`RendezvousMap` 实现跨 tokio task ↔ OS thread 的流交接。

### 4.4 Bandwidth_Stream

流协议 `/pleiades/bandwidth/1.0.0`。iperf 风格：发送方推送固定时长（默认 3s）64KB zero chunks，接收方计数后回传 `total_bytes`。单次出结果。出站走 `Capability::test_bandwidth`，不阻塞事件循环。

---

## 5. 模块导出

```rust
// Network/mod.rs

// 子模块
pub mod capability;
pub mod network_service;
pub mod node_handle;
pub mod request_response;
pub mod file_stream;
pub mod tensor_stream;
pub mod bandwidth_stream;
mod command_handler;      // 内部
mod swarm_events;         // 内部

// 核心类型
pub use capability::{Network_Capability, Network_Error, Network_Inbound_Event,
                     Network_Service_Capability};
pub use network_service::{NetworkConfig, Network_Service};
pub use node_handle::{NodeCommand, NodeHandle, InboundRequest};
pub use request_response::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};

// File_Stream 协议类型
pub use file_stream::{...};

// Tensor_Stream 协议类型
pub use tensor_stream::protocol::{...};
pub use tensor_stream::rendezvous::RendezvousMap;

// Bandwidth_Stream 协议类型
pub use bandwidth_stream::{...};
```

---

## 6. 与 Orchestrator 的协作

| 职责 | 负责方 | 说明 |
|------|--------|------|
| 请求-响应（send_data/send_response） | Capability → mpsc → Handle_Command | 命令通道模式 |
| 连接管理（dial/disconnect） | Capability → mpsc → Handle_Command | 同上 |
| DHT（put_record/get_record） | Capability → mpsc → Handle_Command | 同上 |
| 文件流（open/send/receive） | Capability → stream::Control | 直接操作，不经过事件循环 |
| 张量流（open/accept） | Capability → stream::Control + RendezvousMap | 直接操作 + 跨上下文匹配 |
| 带宽测速 | Capability → stream::Control | 直接操作，不阻塞事件循环 |
| 入站请求转发 | Handle_Request_Response_Event → inbound_tx | 复用 mpsc 通道 |
| 入站文件流转发 | select! 分支 → orchestrator_event_tx | 异步事件通知 |
| 入站张量流交接 | select! 分支 → rendezvous | 双向匹配 |
| 连接/发现/Ping 事件 | swarm_events.rs → PeerManager + EventBus | Network 内部闭环 |

---

## 7. 已知风险

### 7.1 select! 分支阻塞

入站张量流 Handshake、入站带宽流 Receive_And_Count 是持续 async 操作，会阻塞该分支。`tokio::select!` 公平调度，其他分支（Swarm 事件、外部命令）不受影响。但极端并发入站流场景下，`incoming_streams` 通道可能积压。

### 7.2 cmd_tx 通道容量

`cmd_tx` 通道容量为 100。高频 Send_Data / Dial 场景下可能满。当前为局域网低频命令，无此风险。

### 7.3 RendezvousMap 内存泄漏

`RendezvousMap` 存储待匹配的出入站流。如果 Job 取消或超时，未匹配的 entry 不会自动清理。未来需增加 TTL 过期机制。

### 7.4 仅支持局域网

当前传输层 `TCP + mDNS`，无 STUN/TURN/中继。广域网需扩展 QUIC + relay 节点。

---

## 8. 重构历史

| 变更 | 说明 |
|------|------|
| 提取 Request_Response 子目录 | `data_protocol.rs`/`inbound_manager.rs`/`outbound_manager.rs` → `Request_Response/` |
| 新建 Bandwidth_Stream 子模块 | iperf 风格 stream 测速，替代 `DataType::BandwidthTest` |
| 删除 `DataType::BandwidthTest` | 不再占用 request_response 枚举槽位 |
| 删除 `NodeCommand::UpdateInfo` | 带宽测速走 Capability，不走命令通道 |
| 出站测速走 Capability | `test_bandwidth` 直接操作 `stream::Control`，不阻塞事件循环 |
| 拆分 swarm_events.rs | Handle_Mdns/Kademlia/RR/Ping + Handle_Swarm_Event 分发器 |
| 拆分 command_handler.rs | Handle_Command 独立文件 |
| Network_Service 字段 `pub(crate)` | 支持跨文件 impl block 访问 |

---

## 9. TODO

### WAN 广域网支持

- QUIC 传输层替换 / 扩展
- STUN/TURN 中继发现
- WAN 模式下 Kademlia 引导优化

### Peer_Management_Capability 方法名统一

- `List_Peers` → `Get_Peers`
- `Update_Heartbeat` / `Update_Status` / `Update_Bandwidth` 待统一为 `Update_Peer` + `PeerProfile`

### RendezvousMap TTL

- 未匹配流 30s 自动过期清理
