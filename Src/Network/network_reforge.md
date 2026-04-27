# Network Reforge 设计文档

## 一、背景

原 Network 层的设计时与 Control 层耦合：
- 文件传输由 `File_Transfer_Manager` 内部 `tokio::spawn` 独立完成，直接操作 `tokio::fs`，绕过 Storage 模块的锁管理和文件注册表
- 所有操作（包括文件传输、张量流建立）都通过 `NodeCommand` → `Network_Service` 事件循环 → 内部处理，外部对传输过程**无控制权**
- 接收到的文件保存到硬编码路径 `"Pleiades_Workspace"`，与 `StorageManager` 的 `base_dir` 完全独立
- 入站文件流到达时，Network 层自行 spawn 接收任务，上层（Orchestrator）无法感知或管理这些接收作业

现在 Orchestrator 取代 Control 层，需要将 Network 重构为 **Capability 模式**，与 ML Engine、Storage 等模块保持一致的对外接口风格。

---

## 二、设计目标

1. **Capability 封装**：Network 对外暴露 `Network_Capability` trait，Orchestrator 通过 trait 接口调用网络操作
2. **Storage 集成**：文件传输通过 StorageManager 的 `acquire_read` / `acquire_write` 获取锁保护，传输期间文件不会被意外修改或删除
3. **消除双层 spawn**：文件传输的 I/O 操作在调用方（Orchestrator Executor）自己的 tokio task 中执行，Network 层不再内部 spawn
4. **事件分流**：简单事件（peer 发现、连接、ping、DHT）由 Network 内部处理；复杂入站事件（文件传输请求、pipeline 加入请求）转发给 Orchestrator 由其 spawn Job 处理
5. **职责清晰**：Network 层只负责通信原语（连接管理、流打开/关闭、帧读写），不负责文件 I/O 策略和业务流程

---

## 三、架构总览

```
┌─────────────────────────────────────────────────────────────┐
│                     Orchestrator                             │
│                                                             │
│  Core ──► Arc<Capabilities> ──► network 字段                 │
│           │                     (Box<dyn Network_Capability>)│
│           │                                                 │
│  JobExecutor / TaskEngine                                   │
│    └─ handler_data.rs (文件传输指令)                          │
│       1. capabilities.network.send_data(File 元数据) → ACCEPT│
│       2. capabilities.storage.acquire_read(file_id)         │
│       3. capabilities.network.open_file_stream(peer)        │
│       4. capabilities.network.send_file_data(stream, path)  │
│       5. drop ReadGuard (释放锁)                             │
│                                                             │
│  Core::run() select!                                        │
│    └─ network_event_rx.recv()                               │
│       InboundFileStream → compile + spawn_job               │
│       InboundPipelineRequest → compile + spawn_job          │
└───────────────┬──────────────────────────────────────────────┘
                │ (trait 接口)
                ▼
┌─────────────────────────────────────────────────────────────┐
│             Network_Capability trait                          │
│             (Src/Network/capability.rs) ← 新增文件            │
│                                                             │
│  === 请求-响应 ===                                            │
│  send_data(peer, data_type, payload) → Network_Data         │
│  send_response(request_id, data_type, payload)              │
│                                                             │
│  === 连接管理 ===                                            │
│  dial(addr)                                                 │
│  disconnect(peer)                                           │
│                                                             │
│  === 文件流传输 ===                                           │
│  open_file_stream(peer) → libp2p::Stream                    │
│  send_file_data(stream, file_path) → ()                    │
│  receive_file_data(stream, dest_path, file_size) → ()       │
│                                                             │
│  === 张量流 ===                                              │
│  open_tensor_stream(peer) → libp2p::Stream                  │
│                                                             │
│  === DHT ===                                                │
│  put_record(key, value)                                     │
│  get_record(key)                                            │
└───────────────┬──────────────────────────────────────────────┘
                │ (impl)
                ▼
┌─────────────────────────────────────────────────────────────┐
│             Network_Service_Capability                        │
│             (Src/Network/capability.rs)                       │
│                                                             │
│  node_handle: NodeHandle               ← 简单操作走命令通道  │
│  file_stream_control: stream::Control  ← 文件流直接 open     │
│  tensor_stream_control: stream::Control← 张量流直接 open     │
└──────────────────────────────────────────────────────────────┘
```

---

## 四、事件分流表

### 4.1 Network 内部处理（不转发）

| 事件 | 当前处理方式 | 改造后 |
|------|------------|--------|
| mDNS 发现/离开 | `Handle_Mdns_Event` → `event_sender` 通知 + Kademlia 添加地址 | **移除 `event_sender` 通知**，仅保留 Kademlia 添加地址。PeerManager 已通过 `peer_handle` 独立跟踪节点状态 |
| Kademlia DHT 结果 | `Handle_Kademlia_Event` → `event_sender` | **移除 `event_sender` 通知**。DHT 当前未使用，未来通过 `orchestrator_event_tx` 或 Capability oneshot 模式投递结果 |
| Ping 心跳 | `Handle_Ping_Event` → `peer_handle.update_heartbeat` | **不变**（不依赖 `event_sender`） |
| 连接建立/断开 | `peer_handle.add_peer/remove_peer` + `event_sender` | **移除 `event_sender` 通知**，仅保留 `peer_handle` 操作。PeerManager 已完成实际跟踪 |
| Request-Response 入站 | `inbound_manager.Register_Inbound` → `inbound_tx` | **不变**（`inbound_rx` 接收方改为 Orchestrator） |
| Request-Response 出站回复 | `outbound_manager.Route_Response` | **不变** |
| 带宽测试 | `Test_Bandwidth` → `peer_handle.update_bandwidth` | **不变**（不依赖 `event_sender`） |

> **关键决策**：`event_sender` / `NetworkEvent` 整条通道**完全移除**。
> 所有以前通过 `event_sender` 上报的事件（Peer 发现/连接/DHT/文件传输进度）均为纯通知，
> 实际功能已由 `peer_handle`（PeerManager）或 Orchestrator Job 承担。
> TUI 如需显示节点状态，直接从 PeerManager 查询。

### 4.2 转发给 Orchestrator 处理（新增）

| 事件 | 当前处理方式 | 改造后 |
|------|------------|--------|
| **入站文件流** | `file_transfer_manager.Spawn_Receive(...)` → 内部 tokio::spawn 接收 | **转发** `libp2p::Stream` 给 Orchestrator → spawn Job 接收 |
| **入站张量流** | `inbound_tensor_manager.Set_Stream(...)` | **转发** `libp2p::Stream` 给 Orchestrator → 由 Job 管理 |
| **入站 Pipeline 请求** | 暂未实现 | 通过 Request-Response 入站 → Orchestrator 识别后 spawn Job |

### 4.3 转发机制

Network_Service 新增一个 `orchestrator_event_tx` 通道，用于转发复杂入站事件：

```rust
/// Network 层转发给 Orchestrator 的事件
pub enum Network_Inbound_Event {
    /// 入站文件流（远端节点主动发送文件）
    FileStreamArrived {
        peer: PeerId,
        stream: libp2p::Stream,
    },
    /// 入站张量流（远端节点建立 pipeline 连接）
    TensorStreamArrived {
        peer: PeerId,
        stream: libp2p::Stream,
    },
}
```

Network_Service 的 select! 循环中：
```rust
// 改造前：内部 spawn 接收
Some((peer_id, stream)) = incoming_file_streams.next() => {
    self.file_transfer_manager.Spawn_Receive(peer_id, stream, self.event_sender.clone());
}

// 改造后：转发给 Orchestrator
Some((peer_id, stream)) = incoming_file_streams.next() => {
    let _ = self.orchestrator_event_tx.send(
        Network_Inbound_Event::FileStreamArrived { peer: peer_id, stream }
    ).await;
}
```

---

## 五、Network_Capability trait 详细定义

### 5.1 错误类型

```rust
#[derive(Debug)]
pub enum Network_Error {
    /// 连接相关错误
    ConnectionFailed(String),
    /// 流打开失败
    StreamOpenFailed(String),
    /// 流 I/O 错误
    StreamIoError(String),
    /// 请求超时
    Timeout(String),
    /// 对方拒绝
    Rejected(String),
    /// 命令发送失败（内部通道已关闭）
    ChannelClosed(String),
}
```

### 5.2 trait 定义

```rust
#[async_trait]
pub trait Network_Capability: Send + Sync {
    // ========================================
    // 请求-响应（委托 NodeHandle）
    // ========================================

    /// 发送数据并等待对方响应
    ///
    /// 内部通过 NodeHandle.Send_Data 实现，30 秒超时。
    ///
    /// # 用法
    /// ```ignore
    /// let response = network.send_data(peer, DataType::Command, payload).await?;
    /// ```
    async fn send_data(
        &self,
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<Network_Data, Network_Error>;

    /// 回复入站请求
    ///
    /// # 用法
    /// ```ignore
    /// network.send_response(request_id, DataType::Command, b"OK".to_vec()).await?;
    /// ```
    async fn send_response(
        &self,
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Network_Error>;

    // ========================================
    // 连接管理（委托 NodeHandle）
    // ========================================

    /// 主动连接到指定地址
    async fn dial(&self, addr: Multiaddr) -> Result<(), Network_Error>;

    /// 断开与指定节点的连接
    async fn disconnect(&self, peer: PeerId) -> Result<(), Network_Error>;

    // ========================================
    // 文件流传输（直接操作 stream::Control）
    // ========================================

    /// 打开到目标节点的文件流连接
    ///
    /// 返回 raw libp2p::Stream，调用方在自己的 tokio task 中使用。
    /// 不通过 Network_Service 事件循环，不 spawn 新任务。
    ///
    /// # 用法（发送端完整流程）
    /// ```ignore
    /// // 1. 发送文件元数据，等待对方 ACCEPT
    /// let metadata = encode_file_metadata(file_name, file_size);
    /// let response = network.send_data(peer, DataType::File, metadata).await?;
    /// if response.payload != b"ACCEPT" { return Err("rejected"); }
    ///
    /// // 2. 获取 Storage 读锁
    /// let (path, _guard) = storage.acquire_read(file_id).await?;
    ///
    /// // 3. 打开文件流并发送纯数据
    /// let mut stream = network.open_file_stream(peer).await?;
    /// network.send_file_data(&mut stream, &path).await?;
    /// // _guard drop → 释放读锁
    /// ```
    async fn open_file_stream(&self, peer: PeerId) -> Result<libp2p::Stream, Network_Error>;

    /// 通过已打开的流发送文件数据（纯 raw data，无 header）
    ///
    /// 文件元数据（文件名、大小）已通过 Request-Response 协商完成，
    /// 流中只包含分块的文件原始数据。
    /// 在调用方的 tokio task 中执行，不 spawn 新任务。
    ///
    /// # 参数
    /// - `stream`: 由 `open_file_stream` 返回的流
    /// - `file_path`: 待发送文件的路径（由 StorageManager.acquire_read 返回）
    async fn send_file_data(
        &self,
        stream: &mut libp2p::Stream,
        file_path: &std::path::Path,
    ) -> Result<(), Network_Error>;

    /// 从入站流接收文件数据并写入指定路径（纯 raw data，无 header）
    ///
    /// 文件元数据（文件名、大小）已通过 Request-Response 协商获得，
    /// 调用方据此通过 StorageManager 获取写锁和目标路径。
    /// 在调用方的 tokio task 中执行，不 spawn 新任务。
    ///
    /// # 用法（接收端完整流程）
    /// ```ignore
    /// // 前置：Orchestrator 收到 InboundRequest(DataType::File)
    /// // 解析出 file_name、file_size → 回复 ACCEPT
    /// // 然后等待 Network_Inbound_Event::FileStreamArrived
    ///
    /// // 1. 获取 Storage 写锁（file_name 已从元数据协商中获得）
    /// let (dest_path, _guard) = storage.acquire_write(&file_name).await?;
    ///
    /// // 2. 从入站流接收文件数据
    /// network.receive_file_data(&mut stream, &dest_path, file_size).await?;
    ///
    /// // 3. _guard drop → 释放写锁，文件已注册到 Storage 索引
    /// ```
    ///
    /// # 参数
    /// - `stream`: 入站流（由 Network_Inbound_Event::FileStreamArrived 提供）
    /// - `dest_path`: 目标文件路径（由 StorageManager.acquire_write 返回）
    /// - `file_size`: 文件大小（由元数据协商获得）
    async fn receive_file_data(
        &self,
        stream: &mut libp2p::Stream,
        dest_path: &std::path::Path,
        file_size: u64,
    ) -> Result<(), Network_Error>;

    // ========================================
    // 张量流（直接操作 stream::Control）
    // ========================================

    /// 打开到目标节点的张量流连接
    ///
    /// 返回 raw libp2p::Stream，由 ML Engine 的 Tensor_IO_Handle 持有。
    ///
    /// # 用法
    /// ```ignore
    /// let outbound = network.open_tensor_stream(peer).await?;
    /// // 入站 stream 由 Network_Inbound_Event::TensorStreamArrived 提供
    /// let handle = Tensor_IO_Handle::New(inbound, outbound, rt);
    /// ```
    async fn open_tensor_stream(&self, peer: PeerId) -> Result<libp2p::Stream, Network_Error>;

    // ========================================
    // DHT（委托 NodeHandle）
    // ========================================

    /// DHT 写入
    async fn put_record(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Network_Error>;

    /// DHT 读取
    async fn get_record(&self, key: Vec<u8>) -> Result<(), Network_Error>;
}
```

---

## 六、Network_Service_Capability 实现

```rust
pub struct Network_Service_Capability {
    /// 简单操作走命令通道（Send_Data, Send_Response, Dial, Disconnect, DHT）
    node_handle: NodeHandle,
    /// 文件流直接 open（不经过 Network_Service 事件循环）
    file_stream_control: stream::Control,
    /// 张量流直接 open
    tensor_stream_control: stream::Control,
}

impl Network_Service_Capability {
    pub fn New(
        node_handle: NodeHandle,
        file_stream_control: stream::Control,
        tensor_stream_control: stream::Control,
    ) -> Self {
        Self {
            node_handle,
            file_stream_control,
            tensor_stream_control,
        }
    }
}
```

### 各方法实现策略

| 方法 | 实现方式 |
|------|---------|
| `send_data` | 委托 `self.node_handle.Send_Data(peer, data_type, payload).await` |
| `send_response` | 委托 `self.node_handle.Send_Response(request_id, data_type, payload).await` |
| `dial` | 委托 `self.node_handle.Dial(addr).await` |
| `disconnect` | 委托 `self.node_handle.Disconnect(peer).await` |
| `put_record` | 委托 `self.node_handle.Put_Record(key, value).await` |
| `get_record` | 委托 `self.node_handle.Get_Record(key).await` |
| `open_file_stream` | 直接 `self.file_stream_control.clone().open_stream(peer, FILE_STREAM_PROTOCOL).await` |
| `send_file_data` | 调用 `stream_protocol::Send_File_Data(stream, file_path)`（纯数据，无 header） |
| `receive_file_data` | 调用 `stream_protocol::Receive_File_Data(stream, dest_path, file_size)`（纯数据，无 header） |
| `open_tensor_stream` | 直接 `self.tensor_stream_control.clone().open_stream(peer, TENSOR_STREAM_PROTOCOL).await` |

---

## 七、端到端用法示例

### 7.1 出站文件发送（Orchestrator Executor 中）

```rust
// handler_data.rs 或专用的 handler_file.rs 中

// 1. 发送文件元数据，等待对方确认
let file_name = "model_split_0_12.pgguf";
let file_size = tokio::fs::metadata(&local_path).await?.len();
let metadata = encode_file_metadata(file_name, file_size); // 自定义序列化
let response = capabilities.network
    .send_data(target_peer, DataType::File, metadata).await
    .map_err(|e| format!("send metadata failed: {}", e))?;
if response.payload != b"ACCEPT" {
    return Err("File transfer rejected by remote peer".into());
}

// 2. 从 Storage 获取读锁和路径
let (file_path, _read_guard) = capabilities.storage
    .acquire_read(file_id).await
    .map_err(|e| format!("Storage acquire_read failed: {}", e))?;

// 3. 打开文件流（直接 open，不经过 Network_Service 事件循环）
let mut stream = capabilities.network
    .open_file_stream(target_peer).await
    .map_err(|e| format!("open_file_stream failed: {}", e))?;

// 4. 发送纯文件数据（在当前 task 中 await，不额外 spawn）
capabilities.network
    .send_file_data(&mut stream, &file_path).await
    .map_err(|e| format!("send_file_data failed: {}", e))?;

// 5. _read_guard drop → 释放 Storage 读锁
```

### 7.2 入站文件接收（Orchestrator spawn 的 Job 中）

文件接收流程涉及两个阶段：

**阶段 1**：Orchestrator Core 收到 `InboundRequest(DataType::File)`
```rust
// Core::run() select! 分支 — 处理入站请求
// 1. 解析文件元数据（file_name, file_size）
let (file_name, file_size) = decode_file_metadata(&request.payload);

// 2. 回复 ACCEPT
capabilities.network
    .send_response(request.request_id, DataType::File, b"ACCEPT".to_vec()).await?;

// 3. 记录 pending 接收信息，等待 FileStreamArrived 事件
//    （通过 peer_id 关联：同一个 peer 接下来的入站文件流就是这次协商对应的）
self.pending_file_receives.insert(request.peer, PendingReceive { file_name, file_size });
```

**阶段 2**：Orchestrator Core 收到 `Network_Inbound_Event::FileStreamArrived`
```rust
// Core::run() select! 分支 — 处理入站文件流
// 1. 查找对应的 pending 接收信息
let pending = self.pending_file_receives.remove(&peer)?;

// 2. compile 接收作业 → spawn Job，将 stream/file_name/file_size 注入 SlotFile
```

**Job Executor 执行**：
```rust
// 1. 通过 Storage 获取写锁和路径（file_name 由协商阶段获得，已在 SlotFile 中）
let (dest_path, _write_guard) = capabilities.storage
    .acquire_write(&file_name).await
    .map_err(|e| format!("Storage acquire_write failed: {}", e))?;

// 2. 从入站流接收纯文件数据（在当前 task 中 await）
capabilities.network
    .receive_file_data(&mut stream, &dest_path, file_size).await
    .map_err(|e| format!("receive_file_data failed: {}", e))?;

// 3. _write_guard drop → 释放写锁，文件已注册到 Storage 索引
```

### 7.3 出站张量流建立（Orchestrator Executor 中）

```rust
// 1. 打开出站张量流
let outbound_stream = capabilities.network
    .open_tensor_stream(target_peer).await
    .map_err(|e| format!("open_tensor_stream failed: {}", e))?;

// 2. 入站张量流由 Network_Inbound_Event::TensorStreamArrived 传入
//    通过 SlotFile 获取
let inbound_stream = slots.take_stream(SLOT_INBOUND_TENSOR)?;

// 3. 构建 Tensor_IO_Handle
let tensor_io = Tensor_IO_Handle::New(inbound_stream, outbound_stream, rt);
```

---

## 八、Network_Service 改造

### 8.1 Init() 签名变更

```rust
pub async fn Init(
    config: NetworkConfig,
    keypair: Keypair,
    peer_handle: PeerHandle,
    // event_sender 已移除：NetworkEvent 通道不再需要
) -> Result<(
    Self,
    NodeHandle,
    mpsc::Receiver<InboundRequest>,
    Network_Service_Capability,           // ← 新增
    mpsc::Receiver<Network_Inbound_Event>, // ← 新增
), Box<dyn Error>>
```

内部创建：
1. 额外两个 `stream::Control`（file + tensor），传给 `Network_Service_Capability`
2. `orchestrator_event_tx/rx` 通道，tx 由 `Network_Service` 持有，rx 传给 Orchestrator

**移除的参数/字段**：
- `event_sender: mpsc::Sender<NetworkEvent>` — 从 `Init()` 参数和 `Network_Service` 结构体中移除
- `NetworkEvent` 枚举 — 整个删除（所有变体均为纯通知，实际功能由 PeerManager 和 Orchestrator 承担）

改造后 Network_Service 对外只有两条通道：
```
Network_Service
    ├─ inbound_tx ─────────► Orchestrator   (Request-Response 入站消息)
    └─ orchestrator_event_tx ► Orchestrator  (入站 Stream、未来 DHT 结果等)
```

### 8.2 事件循环改造

```rust
// 入站文件流：转发给 Orchestrator
Some((peer_id, stream)) = incoming_file_streams.next() => {
    let _ = self.orchestrator_event_tx.send(
        Network_Inbound_Event::FileStreamArrived { peer: peer_id, stream }
    ).await;
}

// 入站张量流：转发给 Orchestrator
Some((peer_id, stream)) = incoming_tensor_streams.next() => {
    let _ = self.orchestrator_event_tx.send(
        Network_Inbound_Event::TensorStreamArrived { peer: peer_id, stream }
    ).await;
}
```

### 8.3 可移除的组件

| 组件 | 处置 |
|------|------|
| `NetworkEvent` 枚举 | **整个删除**。所有变体均为纯通知，实际功能已由 PeerManager / Orchestrator 承担 |
| `event_sender: mpsc::Sender<NetworkEvent>` | **删除**。从 `Init()` 参数和 `Network_Service` 结构体中移除 |
| `File_Transfer_Manager` | **删除**。`Accept_Incoming` 移到 Network_Service 直接调用；`Spawn_Send` / `Spawn_Receive` 被 Capability 方法取代 |
| `NodeCommand::SendFileStream` | **删除**。出站文件传输不再经过命令通道 |
| `Tensor_Stream_Manager`（inbound/outbound 两个实例） | **删除**。stream 管理由 Orchestrator 的 Job 负责，Network 只负责打开/转发 stream |
| `NodeCommand::CreateTensorStream / OpenTensorStream / TakeTensorStreams / CloseTensorStream` | **删除**。所有张量流操作通过 Capability 直接调用 stream::Control |
| `NodeCommand::GetPeers / GetPeerInfo` | **删除**。由 PeerManager Capability 负责 |
| `NodeCommand::UpdateInfo` | **保留或移至 PeerManager Capability**。带宽测试涉及网络通信，可保留或后续迁移 |

### 8.4 保留的组件

| 组件 | 说明 |
|------|------|
| `Network_Service` 事件循环 | 保留：Swarm 事件处理、命令处理、协议注册 |
| `Inbound_Manager` | 保留：Request-Response 入站管理 |
| `Outbound_Manager` | 保留：Response 路由回调用方 |
| `NodeHandle` | 保留：简单操作（SendData, SendResponse, Dial, Disconnect, DHT）仍需命令通道 |
| `data_protocol.rs` | 保留：TLV 帧格式不变 |
| `stream_protocol.rs` | **重构**：移除流内嵌 header，简化为 `Send_File_Data` + `Receive_File_Data`（纯数据传输） |
| `tensor_stream_protocol.rs` | 保留：张量帧协议不变 |

---

## 九、stream_protocol.rs 重构

### 9.1 当前结构

```rust
// 一体化函数：header + data 混合在流中
pub async fn Send_File_Stream(stream, file_path, event_sender, peer) -> io::Result<()>
//   写入: [4B name_len][name_bytes][8B file_size][data chunks...]

pub async fn Receive_File_Stream(stream, save_dir, event_sender, peer) -> io::Result<PathBuf>
//   读取: [4B name_len][name_bytes][8B file_size][data chunks...]
```

### 9.2 问题

流内嵌 header（文件名 + 文件大小）与 Request-Response 元数据协商**冗余**：
- 发送方通过 `Send_Data(DataType::File, metadata)` 已经告知接收方文件名和大小
- 接收方回复 ACCEPT 后，**已经知道即将到来的文件是什么**
- 流中再嵌入一次 header 是多余的

### 9.3 重构后

移除流内嵌 header，流中只包含纯 raw data chunks：

```rust
/// 发送文件数据（纯 raw data，无 header）
///
/// 文件元数据已通过 Request-Response 协商完成。
/// 打开文件 → 分块读取 → 写入流。
///
/// # 参数
/// - `stream`: 已打开的出站流
/// - `file_path`: 待发送文件的路径
pub async fn Send_File_Data(
    stream: &mut libp2p::Stream,
    file_path: &Path,
) -> io::Result<()>

/// 接收文件数据并写入指定路径（纯 raw data，无 header）
///
/// 文件元数据（file_name, file_size）已通过 Request-Response 协商获得，
/// 调用方据此通过 StorageManager 获取写锁和目标路径。
///
/// # 参数
/// - `stream`: 入站流
/// - `dest_path`: 目标文件路径
/// - `file_size`: 文件大小（由协商获得）
pub async fn Receive_File_Data(
    stream: &mut libp2p::Stream,
    dest_path: &Path,
    file_size: u64,
) -> io::Result<()>
```

注意：
- 原有的 `event_sender` 和 `peer` 参数移除（进度上报暂缓，Phase 3 通过 Orchestrator 机制实现）
- 原有的 `Send_File_Stream` / `Receive_File_Stream` 可保留用于 Control 层向后兼容，待 Control 层被 Orchestrator 完全替代后删除

---

## 十、NodeCommand 精简

### 10.1 保留的命令

```rust
pub enum NodeCommand {
    /// 请求-响应发送（保留）
    SendData {
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
        response_tx: Option<oneshot::Sender<Result<Network_Data, String>>>,
    },
    /// 回复入站请求（保留）
    SendResponse {
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    },
    /// DHT 写入（保留）
    PutRecord { key: Vec<u8>, value: Vec<u8> },
    /// DHT 读取（保留）
    GetRecord { key: Vec<u8> },
    /// 主动连接（保留）
    Dial { addr: Multiaddr },
    /// 断开连接（保留）
    Disconnect { peer: PeerId },
    /// 停止节点（保留）
    Stop,
}
```

### 10.2 删除的命令

```rust
// 以下全部删除：
SendFileStream { ... }         // → Network_Capability.open_file_stream + send_file_data
GetPeers { ... }               // → PeerManager Capability
GetPeerInfo { ... }            // → PeerManager Capability
CreateTensorStream { ... }     // → Network_Capability.open_tensor_stream
OpenTensorStream { ... }       // → Network_Capability.open_tensor_stream
TakeTensorStreams { ... }      // → 直接由事件转发
CloseTensorStream { ... }      // → stream drop 自动关闭
UpdateInfo { ... }             // → 保留或移至 PeerManager
```

---

## 十一、Orchestrator Capabilities 集成

改造后，`Capabilities` 结构体新增 `network` 字段：

```rust
pub struct Capabilities {
    pub storage: StorageManager,
    pub io_broker: LLM_IO_Broker,
    pub compute: Box<dyn ComputeCapability>,
    pub inference: Box<dyn InferenceCapability>,
    pub network: Box<dyn Network_Capability>,    // ← 新增
    // pub peer_manager: ...                     // 未来 PeerManager Capability
}
```

Orchestrator Core 的 `run()` select! 循环新增 `network_inbound_rx` 分支：

```rust
loop {
    tokio::select! {
        // ... 现有分支 ...

        // 处理 Network 转发的入站事件
        Some(event) = self.network_inbound_rx.recv() => {
            match event {
                Network_Inbound_Event::FileStreamArrived { peer, stream } => {
                    // compile 接收作业 → spawn Job
                    // stream 存入 SlotFile 供 Executor 使用
                }
                Network_Inbound_Event::TensorStreamArrived { peer, stream } => {
                    // 保存入站 tensor stream 供后续 Job 使用
                }
            }
        }
    }
}
```

---

## 十二、实施计划

> **前置变更**：Control 层和 TUI 层已从 `lib.rs` 中移除（不再编译），
> 因此所有步骤不再需要考虑向后兼容。原 8 步合并为 6 步。

### 步骤 1：新增 capability.rs + Network_Error ✅ 已完成

- **文件**：`Src/Network/capability.rs`（新建）
- **改动**：
  - 定义 `Network_Error` 枚举（6 变体 + Display + Error impl）
  - 定义 `Network_Capability` trait（async_trait，10 个 async 方法）
  - 定义 `Network_Inbound_Event` 枚举（FileStreamArrived / TensorStreamArrived）
  - 4 个内联测试
- **影响**：纯新增文件，不修改现有代码

### 步骤 2：重构 stream_protocol.rs ✅ 已完成

- **文件**：`Src/Network/stream_protocol.rs`
- **改动**：
  - 新增 `Send_File_Data`（纯数据发送，无流内嵌 header，无 event_sender/peer）
  - 新增 `Receive_File_Data`（纯数据接收，无流内嵌 header，确保父目录存在）
  - 旧 `Send_File_Stream` / `Receive_File_Stream` 保留在文件内（`file_transfer_manager.rs` 内部仍引用），但已从 `mod.rs` 公共导出中移除
- **影响**：旧函数将在步骤 5 中随 `file_transfer_manager.rs` 一并删除

### 步骤 3：实现 Network_Service_Capability

- **文件**：`Src/Network/capability.rs`（追加）
- **改动**：
  - 实现 `Network_Service_Capability` 结构体（持有 NodeHandle + 两个 stream::Control）
  - 为其 `impl Network_Capability`：
    - `send_data` / `send_response` / `dial` / `disconnect` / `put_record` / `get_record` → 委托 NodeHandle
    - `open_file_stream` → `file_stream_control.clone().open_stream()`
    - `open_tensor_stream` → `tensor_stream_control.clone().open_stream()`
    - `send_file_data` → 调用 `stream_protocol::Send_File_Data`
    - `receive_file_data` → 调用 `stream_protocol::Receive_File_Data`
- **影响**：纯新增代码，不修改现有逻辑

### 步骤 4：改造 Network_Service（Init + 事件循环）

> 原步骤 4 + 5 合并。由于 Control 层已移除，无需兼容旧签名。

- **文件**：`Src/Network/network_service.rs`
- **改动**：
  - **删除** `NetworkEvent` 枚举定义
  - **删除** `event_sender: mpsc::Sender<NetworkEvent>` 字段和 `Init()` 参数
  - **删除** 事件循环中所有 `self.event_sender.send(...)` 调用
  - **新增** `orchestrator_event_tx: mpsc::Sender<Network_Inbound_Event>` 字段
  - **新增** 创建额外的 `stream::Control`（file + tensor）用于 Capability
  - **修改** `Init()` 返回值：新增 `Network_Service_Capability` + `mpsc::Receiver<Network_Inbound_Event>`
  - **修改** 入站文件流分支：`file_transfer_manager.Spawn_Receive(...)` → `orchestrator_event_tx.send(FileStreamArrived { peer, stream })`
  - **修改** 入站张量流分支：`tensor_stream_manager.Set_Stream(...)` → `orchestrator_event_tx.send(TensorStreamArrived { peer, stream })`
  - **删除** `file_transfer_manager` 和 `tensor_stream_manager` 字段
- **影响**：`Init()` 签名变更，无外部调用方需适配（Control 已移除，main.rs 是空壳）

### 步骤 5：删除废弃组件 + 精简 NodeCommand

> 原步骤 6。由于无向后兼容要求，可一次性清理。

- **删除文件**：`Src/Network/file_transfer_manager.rs`
- **删除文件**：`Src/Network/tensor_stream_manager.rs`
- **修改** `Src/Network/stream_protocol.rs`：
  - 删除旧函数 `Send_File_Stream` / `Receive_File_Stream`
  - 删除 `use super::network_service::NetworkEvent` 导入
  - 删除 `use libp2p::PeerId` 导入（新函数不需要）
  - 删除 `PROGRESS_INTERVAL` 常量
- **修改** `Src/Network/node_handle.rs`：
  - 删除 `NodeCommand::SendFileStream`
  - 删除 `NodeCommand::GetPeers`
  - 删除 `NodeCommand::GetPeerInfo`
  - 删除 `NodeCommand::CreateTensorStream`
  - 删除 `NodeCommand::OpenTensorStream`
  - 删除 `NodeCommand::TakeTensorStreams`
  - 删除 `NodeCommand::CloseTensorStream`
  - 删除对应的 `NodeHandle` 方法（`Send_File`、`Send_File_Stream`、`Get_Peers`、`Get_Peer_Info`、`Create_Tensor_Stream`、`Open_Tensor_Stream`、`Take_Tensor_Streams`、`Close_Tensor_Stream`）
  - 新增 `NodeHandle::Disconnect` 方法（Capability 需要）
- **修改** `Src/Network/network_service.rs`：删除废弃的 `Handle_Command` 分支
- **修改** `Src/Network/mod.rs`：移除 `file_transfer_manager` 和 `tensor_stream_manager` 模块声明及导出

### 步骤 6：集成到 Orchestrator + 测试

> 原步骤 7 + 8 合并。

- **修改** `Src/Orchestrator/mod.rs`：`Capabilities` 新增 `pub network: Box<dyn Network_Capability>` 字段
- **修改** `Src/Orchestrator/core.rs`：
  - `Core` 新增 `network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>` 字段
  - `run()` 的 select! 新增入站事件分支
- **修改** 所有测试中 `Capabilities` 构造加入 `network` stub
- **测试**：
  - `Network_Service_Capability` 方法测试（mock NodeHandle）
  - `Network_Inbound_Event` 通道传输正确性
  - Orchestrator 入站事件路由测试

---

## 十三、依赖图

```
步骤 1 (capability.rs 定义)           ✅ 已完成
    ↓
步骤 2 (stream_protocol.rs 重构)      ✅ 已完成
    ↓
步骤 3 (Network_Service_Capability 实现)
    ↓
步骤 4 (Network_Service 改造: Init + 事件循环)
    ↓
步骤 5 (删除废弃组件 + 精简 NodeCommand)
    ↓
步骤 6 (集成到 Orchestrator + 测试)
```

---

## 十四、风险与注意事项

| 风险 | 缓解措施 |
|------|---------|
| `libp2p::Stream` 不可 Clone，需在通道中移动所有权 | `Network_Inbound_Event` 中直接持有 `libp2p::Stream`，通过 mpsc 通道移动 |
| `stream::Control` 需要多次 `clone()` | `stream::Control` 支持 Clone，每个 Capability 实例持有独立 clone |
| `Send_File_Data` / `Receive_File_Data` 直接操作 `tokio::fs` | 由调用方先通过 StorageManager 获取锁和路径，函数本身只做 I/O，符合职责分离 |
| 进度上报机制缺失 | 后续通过 Orchestrator 的 LifecycleEvent 或专用进度通道实现 |
| `Network_Service::Init()` 返回值增多 | 可考虑用 `NetworkInitResult` 结构体打包返回值 |
