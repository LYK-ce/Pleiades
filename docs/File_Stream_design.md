# File_Stream 设计文档

Presented by KeJi
Date ： 2026-05-15

## 1. 模块概述

`File_Stream` 模块负责 Pleiades 分布式推理系统的**文件流式传输**。它定义文件流的 in-band 协议（header + ACK + body），提供纯函数式的帧读写工具，不做状态管理。

### 核心定义

> **File_Stream = 文件流协议 + 帧读写工具。**
> 单向一次性传输，流自带元数据（in-band header），接收方自描述决定是否接受。
> 与 `Tensor_Stream` 对称，同属 `Network` 的子目录。

### 模块结构

```
File_Stream/
├── mod.rs        ← 子模块声明 + 统一重导出
└── protocol.rs   ← 协议常量 + 纯函数（header/ACK/data 读写）
```

### 调用关系

```
                  Orchestrator / Core
                        │
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
   open_file_stream  FileStreamArrived  send/receive_file_data
   (Capability)      (Network_Service)  (Capability)
          │             │               │
          ▼             ▼               ▼
   Stream_Control   Accept Stream    File_Stream::protocol
                                    (Header/ACK/Data读写)
```

---

## 2. 协议格式

### 2.1 协议标识符

```
FILE_STREAM_PROTOCOL = "/pleiades/file-stream/1.0.0"
```

### 2.2 完整线上帧序列

```
发送方                                    接收方
  │                                        │
  │── open_file_stream(peer) ─────────────→│ FileStreamArrived
  │                                        │
  │═══ In-band Header ════════════════════→│
  │ [2B name_len BE]                       │
  │ [name UTF-8]                           │
  │ [8B file_size LE]                      │
  │ [2B checksum_len BE]                   │
  │ [checksum UTF-8]                       │
  │                                        │
  │←══════ ACK ════════════════════════════│
  │ [1B ACK] 0x01=ACCEPT / 0x00=REJECT    │
  │                                        │
  │═══ File Body (仅 ACCEPT 时) ══════════→│
  │ raw file data (64KB chunks)            │→ compile Job → spawn → write disk
  │                                        │
```

### 2.3 Header 帧

| 字段 | 大小 | 编码 | 说明 |
|------|------|------|------|
| `name_len` | 2 字节 | big-endian u16 | 文件名 UTF-8 字节长度 |
| `file_name` | n 字节 | UTF-8 | 文件名（Storage file_id） |
| `file_size` | 8 字节 | little-endian u64 | 文件总大小 |
| `checksum_len` | 2 字节 | big-endian u16 | 校验和字符串字节长度 |
| `checksum` | n 字节 | UTF-8 | 校验和字符串（如 "blake3:abcdef..."） |

### 2.4 ACK 帧

| 值 | 含义 |
|----|------|
| `0x01` | ACCEPT — 空间足够，开始传输 |
| `0x00` | REJECT — 空间不足或其他原因拒绝 |

### 2.5 Body 帧

仅在 ACK = `0x01` 时发送。64KB 分块的原始文件数据，无额外帧头。

### 2.6 分块常量

```
CHUNK_SIZE = 64 * 1024  (64KB)
```

---

## 3. 函数接口

File_Stream 全部为 **纯 async 函数**，无状态、无结构体。

### 3.1 Header 读写

```rust
/// 写入 File Stream Header（发送方在 open_file_stream 后立即调用）
pub async fn Write_File_Stream_Header(
    stream: &mut libp2p::Stream,
    file_name: &str,
    file_size: u64,
    checksum: &str,
) -> io::Result<()>;

/// 读取 File Stream Header（接收方 Core 在 FileStreamArrived 时调用）
/// 返回 (file_name, file_size, checksum_string)
pub async fn Read_File_Stream_Header(
    stream: &mut libp2p::Stream,
) -> io::Result<(String, u64, String)>;
```

### 3.2 ACK 读写

```rust
/// 写入 ACK 字节（接收方检查空间后调用）
pub async fn Write_File_Stream_Ack(
    stream: &mut libp2p::Stream,
    accepted: bool,
) -> io::Result<()>;

/// 读取 ACK 字节（发送方写完 header 后调用）
/// 返回 true = ACCEPT, false = REJECT
pub async fn Read_File_Stream_Ack(
    stream: &mut libp2p::Stream,
) -> io::Result<bool>;
```

### 3.3 数据传输

```rust
/// 发送文件数据（纯 raw data，64KB chunks）
/// 从 path 读取文件 → 分块写入 stream → flush
pub async fn Send_File_Data(
    stream: &mut libp2p::Stream,
    file_path: &Path,
) -> io::Result<()>;

/// 接收文件数据（纯 raw data，64KB chunks）
/// 创建目标文件 → 精确读取 file_size 字节 → 分块写入磁盘 → flush
pub async fn Receive_File_Data(
    stream: &mut libp2p::Stream,
    dest_path: &Path,
    file_size: u64,
) -> io::Result<()>;
```

---

## 4. Trait 集成

### 4.1 Network_Capability trait 中的文件流方法

```rust
#[async_trait]
pub trait Network_Capability: Send + Sync {
    /// 打开到目标节点的文件流
    async fn open_file_stream(&self, peer: PeerId) -> Result<libp2p::Stream, Network_Error>;

    /// 通过已打开的流发送文件数据
    async fn send_file_data(
        &self, stream: &mut libp2p::Stream, file_path: &Path,
    ) -> Result<(), Network_Error>;

    /// 从入站流接收文件数据
    async fn receive_file_data(
        &self, stream: &mut libp2p::Stream, dest_path: &Path, file_size: u64,
    ) -> Result<(), Network_Error>;
}
```

### 4.2 Network_Service_Capability 中的实现

```rust
pub struct Network_Service_Capability {
    node_handle: NodeHandle,
    file_stream_control: stream::Control,  // ← 出站文件流
    tensor_stream_control: stream::Control,
    tensor_rendezvous: Arc<RendezvousMap>,
}

impl Network_Service_Capability {
    pub fn New(
        node_handle: NodeHandle,
        file_stream_control: stream::Control,   // ← 由 Network_Service::Init 创建
        tensor_stream_control: stream::Control,
        tensor_rendezvous: Arc<RendezvousMap>,
    ) -> Self { /* ... */ }
}
```

### 4.3 方法委托关系

| Trait 方法 | 实现方式 |
|-----------|---------|
| `open_file_stream` | `file_stream_control.clone().open_stream(peer, FILE_STREAM_PROTOCOL)` |
| `send_file_data` | `File_Stream::protocol::Send_File_Data(stream, file_path)` |
| `receive_file_data` | `File_Stream::protocol::Receive_File_Data(stream, dest_path, file_size)` |

---

## 5. 服务集成

### 5.1 Network_Service 中的文件流控制句柄

```rust
pub struct Network_Service {
    // 入站文件流接受
    file_accept_control: stream::Control,
    // ...
}
```

两个 `stream::Control` 均在 `Init()` 中创建：

```rust
let file_accept_control = node_swarm.behaviour().stream.new_control();  // Service 用于 accept
let file_open_control = node_swarm.behaviour().stream.new_control();    // Capability 用于 open
```

### 5.2 入站流事件处理

在 `Start()` 的 `select!` 循环中：

```rust
let mut incoming_file_streams = self.file_accept_control
    .accept(StreamProtocol::new(FILE_STREAM_PROTOCOL))
    .expect("文件流协议注册失败");

// select! 分支
Some((peer_id, stream)) = incoming_file_streams.next() => {
    // 直接转发给 Orchestrator，不读 header
    self.orchestrator_event_tx.send(
        Network_Inbound_Event::FileStreamArrived { peer: peer_id, stream }
    ).await;
}
```

### 5.3 Network_Inbound_Event

```rust
pub enum Network_Inbound_Event {
    /// 入站文件流（远端节点主动发送文件）
    FileStreamArrived {
        peer: PeerId,
        stream: libp2p::Stream,
    },
    /// 入站张量流（已改为 rendezvous 匹配，不再转发）
    TensorStreamArrived {
        peer: PeerId,
        stream: libp2p::Stream,
    },
}
```

---

## 6. 完整触发流程

### 6.1 发送端（Sender）

```
handle_send_file (Orchestrator)
  │
  ├── 1. StorageManager::acquire_read(file_id) → (path, ReadGuard)
  ├── 2. StorageCapability::checksum(file_id) → checksum_string
  ├── 3. Network_Capability::open_file_stream(peer)
  │       └── file_stream_control.open_stream(peer, "/pleiades/file-stream/1.0.0")
  │
  ├── 4. File_Stream::Write_File_Stream_Header(&mut stream, file_name, file_size, &checksum)
  │
  ├── 5. File_Stream::Read_File_Stream_Ack(&mut stream) → bool
  │       ├── false → Abort("peer rejected")
  │       └── true  → 继续
  │
  ├── 6. Network_Capability::send_file_data(&mut stream, &path)
  │       └── File_Stream::Send_File_Data → 64KB chunks
  │
  └── 7. drop(read_guard) → 释放读锁
```

### 6.2 接收端（Receiver）

```
FileStreamArrived (Core B3)
  │
  ├── 1. File_Stream::Read_File_Stream_Header(&mut stream)
  │       → (file_name, file_size, checksum_string)
  │
  ├── 2. 检查存储空间（如有 quota）
  │       ├── 不足 → File_Stream::Write_File_Stream_Ack(stream, false) → return
  │       └── 足够 → 继续
  │
  ├── 3. File_Stream::Write_File_Stream_Ack(&mut stream, true)
  │
  ├── 4. compile ReceiveFile Job
  │       └── 将 stream + file_name + file_size + checksum 注入 SlotFile
  │
  └── 5. spawn Job → Executor
          │
          ├── StorageManager::acquire_write(&file_name) → (dest_path, WriteGuard)
          ├── Network_Capability::receive_file_data(&mut stream, &dest_path, file_size)
          │       └── File_Stream::Receive_File_Data → 64KB chunks → 写入磁盘
          ├── 本地 checksum 校验
          │       ├── 匹配 → 完成
          │       └── 不匹配 → 删除文件 → EventBus::File_Progress 通知
          └── drop(write_guard) → 释放写锁
```

---

## 7. 与 Tensor_Stream 的对比

| 维度 | File_Stream | Tensor_Stream |
|------|------------|---------------|
| **流向** | 单向一次性（写完即关） | 双向持久（推理全生命周期） |
| **建立方式** | Stream → Job | Job → Stream |
| **匹配机制** | 不需要（header 自描述） | RendezvousMap（inference_id 配对） |
| **对端认知** | 接收方可完全不知情 | 两侧必须知道彼此（pipeline 环） |
| **本质** | 资源投递 | 会话建立 |
| **帧格式** | Header(变长) + ACK(1B) + Body(分块) | [8B offset][8B len][data] + EOF 哨兵 |
| **子模块** | `protocol.rs` | `protocol.rs` + `rendezvous.rs` |
| **状态管理** | 无（纯函数） | RendezvousMap（Mutex 保护的双 HashMap） |

---

## 8. 已知风险

### 8.1 无传输进度上报

当前 `Send_File_Data` / `Receive_File_Data` 内部有 `tracing::debug!` 日志但无结构化进度事件。通过 `EventBus::File_Progress` 发布进度待后续实现。

- **当前状态**：仅日志输出
- **未来方案**：在分块循环中按一定间隔（如每 10 个 chunk）调用 `EventBus::Publish(File_Progress { ... })`

### 8.2 接收方 checksum 校验时机

接收方 Job 在写入磁盘后才做 checksum 校验。如果 checksum 不匹配，文件已落盘但随后被删除。这在正常场景下没问题，但极端情况下（进程崩溃在删除前）会残留损坏文件。

- **当前状态**：事后校验 + 删除
- **缓解**：Storage `flush()` 可重新校验 checksum 清理残留

### 8.3 无断点续传

当前协议不支持传输中断后恢复。如果流在中途断开（`UnexpectedEof`），接收方丢弃已接收的数据并返回错误，发送方需从头重传。

- **当前状态**：不支持
- **影响**：大文件传输稳定性依赖网络质量

### 8.4 单流串行

当前设计一次只能传输一个文件（一个 stream 对应一个文件）。批量文件传输需上层依次调用 `open_file_stream` + `send_file_data`。

- **当前状态**：串行
- **未来扩展**：Header 可增加 `file_count` 字段支持多文件批量传输

---

## 9. 重构历史

| 变更 | 说明 |
|------|------|
| 从 `stream_protocol.rs` 提取为 `File_Stream/` 子目录 | 与 `Tensor_Stream/` 对称，提升组织性 |
| 协议格式不变 | in-band header + 1-byte ACK + 分块 body |
| 函数签名不变 | `Write_File_Stream_Header` / `Read_File_Stream_Header` / `Write_File_Stream_Ack` / `Read_File_Stream_Ack` / `Send_File_Data` / `Receive_File_Data` |
| import 路径更新 | `network::stream_protocol` → `network::file_stream::protocol` |
