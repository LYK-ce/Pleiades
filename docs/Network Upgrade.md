#Presented by KeJi
#Date ： 2026-04-01

# Network Layer Upgrade 设计方案

## 1. 改动背景与目标

当前网络层（`Src/Network/`）在协议设计和职责划分上存在以下问题：

1. **协议碎片化**：定义了 3 个独立协议标识符（`/pleiades/cmd/1.0.0`、`/pleiades/file/1.0.0`、`/pleiades/tensor/1.0.0`），以及对应的 `CommandRequest/Response`、`FileRequest/Response`、`TensorRequest/Response` 共 6 组消息结构体。实际上在 libp2p `request_response::Behaviour` 中只注册了 `COMMAND_PROTOCOL` 一个协议标识符，另外两个协议标识符从未被使用。
2. **网络层越权承担序列化职责**：`PleiadesCodec` 使用 `bincode` 对上层业务结构（含 `serde` derive）进行序列化/反序列化，导致网络层与上层数据结构强耦合。上层如果修改了任何一个字段，网络层也需要同步修改。
3. **对外接口分散**：`NodeHandle` 暴露了 `Send_Command`、`Send_File`、`Send_Tensor` 三个独立的发送方法，上层事件也拆成 `CommandReceived`、`FileReceived`、`TensorReceived` 三种。

**改动目标**：

- 网络层**只负责发送和接收字节流**，不负责序列化/反序列化
- 用一个统一的 **Data Protocol** 替代原先的 3 个协议
- 传输帧格式：`类型(Type) + 长度(Length) + 载荷(Payload)`
- 对外提供统一的 **`Send_Data`** 接口

---

## 2. 新协议设计

### 2.1 协议标识符

替换原先 3 个协议常量为 1 个：

```rust
// 旧
pub const COMMAND_PROTOCOL: &str = "/pleiades/cmd/1.0.0";
pub const FILE_PROTOCOL: &str = "/pleiades/file/1.0.0";
pub const TENSOR_PROTOCOL: &str = "/pleiades/tensor/1.0.0";

// 新
pub const DATA_PROTOCOL: &str = "/pleiades/data/1.0.0";
```

### 2.2 数据类型枚举

定义数据的逻辑类型，纯标记用途，不影响网络层处理逻辑：

```rust
/// 数据帧类型标记
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DataType {
    /// 命令/控制消息
    Command = 0,
    /// 数据（张量、中间结果等）
    Data = 1,
    /// 文件通知（仅传输文件元数据：文件名、大小等，不传输文件内容）
    File = 2,
}
```

提供 `u8` 与 `DataType` 之间的转换方法：

```rust
impl DataType {
    pub fn From_U8(v: u8) -> std::io::Result<Self> {
        match v {
            0 => Ok(DataType::Command),
            1 => Ok(DataType::Data),
            2 => Ok(DataType::File),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown DataType: {}", v),
            )),
        }
    }
}
```

### 2.3 传输帧格式 (Wire Format)

采用 TLV（Type-Length-Value）格式，网络层只需要知道如何读写这个固定的帧头，不需要理解 payload 内容：

```
┌──────────┬──────────────┬─────────────────────┐
│  Type    │   Length     │      Payload        │
│  1 byte  │  8 bytes BE  │  Length bytes        │
└──────────┴──────────────┴─────────────────────┘
```

- **Type** (1 byte)：`DataType` 的 `u8` 表示，标记此帧的逻辑用途
- **Length** (8 bytes, big-endian u64)：Payload 字节数（使用 u64 以支持大文件传输）
- **Payload** (变长)：原始字节流，由上层自行序列化/反序列化

最大帧大小限制提升至 **2 GB** (`2 * 1024 * 1024 * 1024`)。

### 2.3.1 大数据传输说明

**DataType::File 的用途说明**

`DataType::File` 类型**仅用于传输文件通知/元数据**（如文件名、文件大小、哈希等），**不用于传输实际文件内容**。这是一个设计决策：网络层的 request-response 模式适合传输控制消息和中等大小数据，但不适合直接传输大型文件。

**各类型用途与大小范围**：

| 数据类型 | 用途 | 典型 payload | 大小范围 |
|----------|------|-------------|----------|
| Command | 命令/控制消息 | 自定义命令字节流 | < 1 KB |
| Data | 张量/激活值等计算数据 | 序列化后的 tensor 字节流 | 数 KB ~ 数十 MB |
| File | 文件通知/元数据 | 文件名、文件大小、哈希等 | < 1 KB |

**关于实际文件传输**：

当前 MVP 阶段**暂不实现**文件内容的网络传输功能。实际的文件传输（如模型文件分发）将在未来通过以下方案实现：

1. 发送方通过 `Send_Data(peer, DataType::File, metadata_bytes)` 通知接收方文件元信息
2. 接收方确认后，双方通过**独立的流式传输协议**（如 libp2p-stream 或自定义 streaming protocol）进行文件内容传输
3. 流式传输可以支持任意大小文件、断点续传、进度追踪等高级特性

这种"通知 + 流式传输"的分离设计是业界常见模式（类似 FTP 的 control + data channel），确保控制面和数据面解耦。

**Data 类型的大小限制**：
- Length 字段为 u64，但当前实际限制为 **2GB**，防止内存溢出
- 对于 Data 类型（张量等），2GB 足以覆盖当前所有场景
- Codec 读取 payload 时先分配 `Vec<u8>`，接收端需要有足够内存

### 2.4 请求与响应结构

网络层不再定义业务结构，只定义字节级容器：

```rust
/// 统一数据请求（网络层只看字节流）
#[derive(Debug, Clone)]
pub struct DataRequest {
    /// 数据类型标记
    pub data_type: DataType,
    /// 原始载荷字节流（上层负责序列化）
    pub payload: Vec<u8>,
}

/// 统一数据响应（网络层只看字节流）
#[derive(Debug, Clone)]
pub struct DataResponse {
    /// 数据类型标记
    pub data_type: DataType,
    /// 原始载荷字节流（上层负责序列化）
    pub payload: Vec<u8>,
}
```

### 2.5 新 Codec 实现

`PleiadesCodec` 不再使用 `bincode`/`serde`，改为直接读写 TLV 帧：

```rust
impl Codec for PleiadesCodec {
    type Protocol = StreamProtocol;
    type Request = DataRequest;
    type Response = DataResponse;

    // read_request:
    //   1. 读 1 字节 → DataType::From_U8()
    //   2. 读 8 字节 big-endian u64 → payload length
    //   3. 检查 length <= 2GB
    //   4. 读 length 字节 → payload
    //   5. 返回 DataRequest { data_type, payload }

    // read_response: 同 read_request 逻辑，返回 DataResponse

    // write_request:
    //   1. 写 1 字节 data_type as u8
    //   2. 写 8 字节 big-endian u64 payload length
    //   3. 写 payload 字节
    //   4. flush

    // write_response: 同 write_request 逻辑
}
```

**关键变化**：不再依赖 `bincode`、`serde`，Codec 只做字节搬运。

---

## 3. 对外接口改动

### 3.1 NodeHandle API 变更

| 旧接口 | 新接口 | 说明 |
|--------|--------|------|
| `Send_Command(peer, CommandRequest)` | **删除** | 合并到 `Send_Data` |
| `Send_File(peer, Path)` | **删除** | 合并到 `Send_Data` |
| `Send_Tensor(peer, TensorRequest)` | **删除** | 合并到 `Send_Data` |
| — | `Send_Data(peer, DataType, Vec<u8>)` | **新增**统一发送接口 |
| `Send_Response(channel, PleiadesResponse)` | `Send_Response(channel, DataType, Vec<u8>)` | 参数类型调整 |
| `Put_Record` / `Get_Record` / `Dial` / `Disconnect` / `Stop` | **保持不变** | DHT和连接管理不受影响 |

新的 `NodeHandle` 方法签名：

```rust
impl NodeHandle {
    /// 统一数据发送接口
    ///
    /// # Arguments
    /// * `peer` - 目标节点 ID
    /// * `data_type` - 数据类型标记（Command / Data / File）
    /// * `payload` - 已序列化的字节流（由上层负责序列化）
    pub async fn Send_Data(
        &self,
        peer: &PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// 发送响应
    pub async fn Send_Response(
        &self,
        channel: ResponseChannel<DataResponse>,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Box<dyn Error + Send + Sync>>;

    // Get_Local_Peer_Id, Put_Record, Get_Record, Dial, Disconnect, Stop 保持不变
}
```

### 3.2 NodeCommand 枚举变更

```rust
pub enum NodeCommand {
    // 旧的 SendCommand, SendFile, SendTensor 三个变体 → 合并为:
    SendData {
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    },

    // 旧的 SendResponse { channel, response: PleiadesResponse } → 改为:
    SendResponse {
        channel: ResponseChannel<DataResponse>,
        data_type: DataType,
        payload: Vec<u8>,
    },

    // 以下保持不变
    PutRecord { key: Vec<u8>, value: Vec<u8> },
    GetRecord { key: Vec<u8> },
    Dial { addr: Multiaddr },
    Disconnect { peer: PeerId },
    Stop,
}
```

### 3.3 NetworkEvent 枚举变更

```rust
pub enum NetworkEvent {
    PeerDiscovered(PeerId),        // 不变
    PeerLeft(PeerId),              // 不变
    ConnectionEstablished(PeerId), // 不变
    ConnectionClosed(PeerId),      // 不变

    // 旧的 CommandReceived, FileReceived, TensorReceived 三个变体 → 合并为:
    DataReceived {
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
        channel: ResponseChannel<DataResponse>,
    },

    RecordFound { key: Vec<u8>, value: Vec<u8> },   // 不变
    RecordNotFound { key: Vec<u8> },                  // 不变
}
```

---

## 4. 需要删除的内容

以下结构体/枚举/常量将从 `protocol.rs` 中**完全删除**：

| 删除项 | 类型 |
|--------|------|
| `COMMAND_PROTOCOL` | 常量 |
| `FILE_PROTOCOL` | 常量 |
| `TENSOR_PROTOCOL` | 常量 |
| `TensorDtype` | 枚举 |
| `CommandRequest` | 枚举 |
| `CommandResponse` | 枚举 |
| `FileRequest` | 枚举 |
| `FileResponse` | 枚举 |
| `TensorRequest` | 结构体 |
| `TensorResponse` | 枚举 |
| `PleiadesRequest` | 枚举 |
| `PleiadesResponse` | 枚举 |
| `FILE_CHUNK_SIZE` | 常量 |
| `TENSOR_CHUNK_SIZE` | 常量 |

**Cargo.toml 依赖变更**：
- `bincode` 依赖可以从 `[dependencies]` 中移除（网络层不再使用；如果其他模块也不使用则完全移除）
- `protocol.rs` 中不再需要 `use serde::{Deserialize, Serialize}`（serde 仍被 config 模块使用，保留在 Cargo.toml 中）

---

## 5. 需要修改的文件清单

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `Src/Network/protocol.rs` | **重写** | 删除旧协议结构，实现新的 `DataType` / `DataRequest` / `DataResponse` / `PleiadesCodec` |
| `Src/Network/node.rs` | **修改** | `NodeCommand` / `NetworkEvent` / `NodeHandle` / `Node` 使用新类型；`Handle_Command` 合并三种发送逻辑为一种；`Handle_Request_Response_Event` 简化为统一 `DataReceived` 事件 |
| `Src/Network/mod.rs` | **修改** | 更新 re-export 列表，导出新类型（`DataType`、`DataRequest`、`DataResponse`），移除旧类型导出 |
| `Src/lib.rs` | **修改** | 更新 `pub use network::` 导出列表，移除旧的 `CommandRequest`、`FileRequest` 等 |
| `Cargo.toml` | **修改** | 移除 `bincode` 依赖（如果没有其他模块使用） |

> **注意**：`tests/common/network_test.rs` 和 `tests/common/collaborative_test.rs` 暂不修改，后续单独完善测试代码。

---

## 6. 上层适配指引

### 6.1 发送命令（旧 Send_Command）

```rust
// 旧写法
handle.Send_Command(&peer, CommandRequest::Ping).await?;

// 新写法：上层自行序列化命令为字节
let payload = b"ping".to_vec(); // 或使用自定义序列化格式
handle.Send_Data(&peer, DataType::Command, payload).await?;
```

### 6.2 发送文件通知（旧 Send_File）

```rust
// 旧写法
handle.Send_File(&peer, Path::new("test.txt")).await?;

// 新写法：DataType::File 仅发送文件元数据通知，不发送文件内容
// 上层自行组装文件元数据 payload，如: [filename_len][filename][file_size_u64]
let filename = b"model.gguf";
let file_size: u64 = std::fs::metadata("model.gguf")?.len();
let mut payload = Vec::new();
payload.extend_from_slice(&(filename.len() as u32).to_be_bytes());
payload.extend_from_slice(filename);
payload.extend_from_slice(&file_size.to_be_bytes());
handle.Send_Data(&peer, DataType::File, payload).await?;

// 注意：实际文件内容的传输将通过未来实现的流式传输协议完成
// 当前 MVP 阶段暂不实现文件内容传输
```

### 6.3 发送张量（旧 Send_Tensor）

```rust
// 旧写法
let tensor_req = TensorRequest { request_id, tensor_data, shape, dtype };
handle.Send_Tensor(&peer, tensor_req).await?;

// 新写法：使用 GGUF_Tensor_Serialize 序列化后直接发送字节流
let packet = GGUF_Tensor_Packet::New(name, shape, dtype, raw_data);
let serialized = GGUF_Tensor_Serialize(&packet)?;
handle.Send_Data(&peer, DataType::Data, serialized).await?;
```

### 6.4 接收数据（旧 CommandReceived / FileReceived / TensorReceived）

```rust
// 旧写法
match event {
    NetworkEvent::CommandReceived { peer, request, channel } => { ... }
    NetworkEvent::FileReceived { peer, request, channel } => { ... }
    NetworkEvent::TensorReceived { peer, request, channel } => { ... }
}

// 新写法
match event {
    NetworkEvent::DataReceived { peer, data_type, payload, channel } => {
        match data_type {
            DataType::Command => {
                // 上层自行反序列化 payload
            }
            DataType::Data => {
                // 上层自行反序列化，如 GGUF_Tensor_Deserialize(&payload)
            }
            DataType::File => {
                // 上层自行解析文件元数据（文件名、大小等）
                // 实际文件内容传输将通过未来的流式传输协议完成
            }
        }
        // 发送响应
        handle.Send_Response(channel, DataType::Command, b"ack".to_vec()).await?;
    }
}
```

### 6.5 发送响应（旧 Send_Response）

```rust
// 旧写法
let ack = PleiadesResponse::Tensor(TensorResponse::Ack { request_id });
handle.Send_Response(channel, ack).await?;

// 新写法
handle.Send_Response(channel, DataType::Data, b"ack".to_vec()).await?;
```

---

## 7. node.rs 内部处理逻辑变更

### 7.1 Handle_Command 简化

旧实现中 `Handle_Command` 分别处理 `SendCommand`、`SendFile`（包含文件读取逻辑）、`SendTensor` 三个分支。新实现合并为一个：

```rust
NodeCommand::SendData { peer, data_type, payload } => {
    info!("发送数据到 {} | type={:?} | size={} bytes", peer, data_type, payload.len());
    let request = DataRequest { data_type, payload };
    self.swarm
        .behaviour_mut()
        .request_response
        .send_request(&peer, request);
}
```

**注意**：旧的 `SendFile` 分支中包含 `std::fs::read` 文件读取逻辑。该逻辑已移除。`DataType::File` 现在仅用于传输文件元数据通知（文件名、大小等），不传输文件内容，网络层不再负责文件 I/O。

### 7.2 Handle_Request_Response_Event 简化

旧实现对收到的 `PleiadesRequest` 做 `match` 分发为三种事件。新实现直接转发：

```rust
request_response::Message::Request { request, channel, .. } => {
    info!("收到数据请求 from {} | type={:?} | size={} bytes",
          peer, request.data_type, request.payload.len());
    let _ = self.event_sender.send(NetworkEvent::DataReceived {
        peer,
        data_type: request.data_type,
        payload: request.payload,
        channel,
    }).await;
}
```

### 7.3 Swarm Behaviour 注册

注册协议时使用新的 `DATA_PROTOCOL`：

```rust
let protocols = [(
    StreamProtocol::new(DATA_PROTOCOL),
    ProtocolSupport::Full,
)];
let request_response = request_response::Behaviour::<PleiadesCodec>::new(protocols, cfg);
```

---

## 8. 架构对比

### 旧架构

```
上层 (collaborative_test / network_test)
  │
  ├─ CommandRequest ─────┐
  ├─ FileRequest ────────┤  业务结构体 (serde)
  └─ TensorRequest ──────┘
          │
          ▼
     NodeHandle
  ├─ Send_Command()
  ├─ Send_File()      ← 网络层读取文件
  └─ Send_Tensor()
          │
          ▼
     PleiadesCodec
  [bincode serialize]  ← 网络层做序列化
          │
          ▼
     libp2p stream
```

### 新架构

```
上层 (调度层 / 控制层)
  │
  │  上层自行：
  │  - GGUF_Tensor_Serialize() 序列化张量
  │  - 自定义命令格式
  │  - 组装文件元数据通知
  │
  └─ Vec<u8> (原始字节) + DataType
          │
          ▼
     NodeHandle
  └─ Send_Data(peer, DataType, Vec<u8>)
          │
          ▼                              ┌────────────────────┐
     PleiadesCodec                       │  文件流式传输协议   │
  [1B type + 8B len + payload]           │  (未来实现)        │
  ← 只写帧头，不做业务序列化             │  libp2p-stream     │
          │                              └────────────────────┘
          ▼
     libp2p request-response stream
```

**说明**：
- `DataType::Command` 和 `DataType::Data` 通过 request-response 传输实际内容
- `DataType::File` 仅通过 request-response 传输文件元数据通知
- 实际文件内容传输将在未来通过独立的流式传输协议实现

---

## 9. 改动量评估

| 文件 | 预计改动行数 | 复杂度 |
|------|-------------|--------|
| `Src/Network/protocol.rs` | ~150 行（重写） | 中 |
| `Src/Network/node.rs` | ~100 行（修改） | 中 |
| `Src/Network/mod.rs` | ~10 行 | 低 |
| `Src/lib.rs` | ~10 行 | 低 |
| `Cargo.toml` | ~1 行 | 低 |

**总计约 270 行改动**，核心改动集中在 `protocol.rs` 和 `node.rs`。

> 测试文件（`network_test.rs`、`collaborative_test.rs`）暂不修改，后续单独完善。

---

## 10. 兼容性说明

- 此次改动是 **Breaking Change**，改动后旧版本节点与新版本节点**不兼容**（协议标识符不同）
- 由于当前项目处于 MVP 阶段，不需要考虑向后兼容
- 改动完成后需要重新运行 network_test 和 collaborative_test 验证功能正确性

---

## 11. 未来扩展：文件流式传输协议

当前 MVP 阶段，`DataType::File` 仅用于文件元数据通知。实际文件内容传输将在未来通过以下方案实现：

### 11.1 设计思路

采用"**通知 + 流式传输**"双协议模式：

```
发送方                               接收方
  │                                    │
  ├── Send_Data(File, metadata) ──────►│  1. 通知：文件名、大小、哈希
  │◄── Response(File, ack/reject) ─────┤  2. 接收方确认或拒绝
  │                                    │
  ├══ Stream Protocol ════════════════►│  3. 建立独立流式传输通道
  │    [chunk_1] [chunk_2] ... [end]   │  4. 分块传输文件内容
  │◄── Stream Complete ════════════════┤  5. 传输完成确认
```

### 11.2 优势

- **内存高效**：接收端不需要一次性分配整个文件大小的内存，边收边写磁盘
- **可中断恢复**：支持断点续传
- **进度追踪**：已知文件大小，可跟踪传输进度百分比
- **错误前置**：接收端可以在传输前检查磁盘空间等条件，提前拒绝

### 11.3 实现路径

使用 `libp2p-stream` 或在 libp2p 上自定义 streaming protocol，注册为独立协议（如 `/pleiades/file-stream/1.0.0`）。具体实现方案待文件传输需求明确后设计。
