# File Send Reforge — In-band Header 方案

## 1. 设计动机

当前文件传输采用 **三阶段协议**（Request-Response 元数据协商 → Stream 数据传输 → Request-Response 验证），涉及跨 select! 分支的 `pending_file_receives` 状态、两种协议混合、时序竞态风险。

而张量流（Tensor Stream）仅用 **stream + in-band handshake（8 字节）** 就完成了路由和传输，简洁可靠。

本方案将文件传输改造为与张量流同构的 **"in-band header + 1-byte ACK + stream body"** 模式。

### 核心设计哲学：职责分离

| 角色 | 职责 | 不关心 |
|------|------|--------|
| **发送方** | 传输文件数据 | 接收方是否校验成功 |
| **接收方** | 决定是否接收（ACK）+ 本地 checksum 校验 | 通知发送方校验结果 |

发送方只需知道对方"愿不愿意收"（通过 1-byte ACK），数据完整性校验完全由接收方负责。如果 checksum 不匹配，接收方自行删除损坏文件并通过 EventBus 报告，无需回传结果给发送方。这与旧方案 Phase 3 的 `VERIFY_FILE` 往返相比，消除了一次不必要的网络交互——接收方本地已经能确认数据是否正确，无须通过网络告知发送方。

---

## 2. 对比总览

| 维度 | 旧方案（三阶段） | 新方案（in-band） |
|------|------------------|-------------------|
| 网络交互 | 3 次（RR + Stream + RR） | 1 次（Stream only） |
| 协议混合 | Request-Response + Stream | Stream only |
| Core 跨分支状态 | `pending_file_receives: HashMap<PeerId, Vec<FileMetadata>>` | **消除** |
| B2 DataType::File 分支 | 需要处理 | **删除** |
| 时序竞态 | B2 元数据必须先于 B3 stream 到达 | **消除** |
| Phase 3 VERIFY_FILE | 额外 RR 往返 | **消除**（本地 checksum 足够） |
| `handle_network_file()` | 60+ 行 | **删除** |

---

## 3. 新协议帧格式

### 3.1 File Stream Header（发送方写入，接收方读取）

```
+-------------------+-------------------+-------------------+-------------------+
| name_len (2B BE)  | file_name (nB)    | file_size (8B LE) | checksum (32B)    |
+-------------------+-------------------+-------------------+-------------------+
```

| 字段 | 大小 | 编码 | 说明 |
|------|------|------|------|
| `name_len` | 2 字节 | big-endian u16 | 文件名 UTF-8 字节长度（最大 65535） |
| `file_name` | n 字节 | UTF-8 | 文件名（即 Storage file_id） |
| `file_size` | 8 字节 | little-endian u64 | 文件总大小 |
| `checksum` | 32 字节 | raw bytes | Blake3 校验和 |

### 3.2 ACK（接收方写入，发送方读取）

```
+-------------------+
| ACK (1B)          |
+-------------------+
```

| 值 | 含义 |
|----|------|
| `0x01` | ACCEPT — 空间足够，开始传输 |
| `0x00` | REJECT — 空间不足或其他拒绝原因 |

### 3.3 File Body（发送方写入，接收方读取）

仅在 ACK = `0x01` 时发送。与当前 `stream_protocol.rs` 中的纯 raw data 分块传输一致（64KB chunks）。

---

## 4. 新流程时序

```
Sender (handler_network.rs)              Receiver Core (core.rs B3)         Receiver Job
    │                                          │                               │
    │── open_file_stream(peer) ──────────────>│ FileStreamArrived              │
    │                                          │                               │
    │── [2B name_len][name][8B size][32B cksum]│                               │
    │                                          │ read header                    │
    │                                          │ check storage quota            │
    │                                          │                               │
    │<──────────── [1B ACK] ──────────────────│                               │
    │                                          │                               │
    │  if ACK=0x01:                            │ compile ReceiveFile Job        │
    │── raw file data (64KB chunks) ─────────>│──────────────────────────────>│
    │                                          │                               │ write to storage
    │                                          │                               │ verify checksum
    │  if ACK=0x00:                            │                               │
    │  Abort("peer rejected")                  │ (stream closed)               │
```

---

## 5. 需要修改的文件

### 5.1 `Src/Network/stream_protocol.rs` — 新增 header 读写函数

**新增内容：**

```rust
/// File Stream Header 常量
pub const FILE_HEADER_ACCEPT: u8 = 0x01;
pub const FILE_HEADER_REJECT: u8 = 0x00;

/// 写入 File Stream Header
///
/// 发送方在 open_file_stream 后立即调用。
/// 格式: [2B name_len BE][name bytes][8B file_size LE][32B checksum]
pub async fn Write_File_Stream_Header(
    stream: &mut libp2p::Stream,
    file_name: &str,
    file_size: u64,
    checksum: &[u8; 32],
) -> io::Result<()>

/// 读取 File Stream Header
///
/// 接收方 Core 在 FileStreamArrived 时调用。
/// 返回 (file_name, file_size, checksum_bytes)
pub async fn Read_File_Stream_Header(
    stream: &mut libp2p::Stream,
) -> io::Result<(String, u64, [u8; 32])>

/// 写入 ACK 字节
pub async fn Write_File_Stream_Ack(
    stream: &mut libp2p::Stream,
    accepted: bool,
) -> io::Result<()>

/// 读取 ACK 字节
///
/// 返回 true = ACCEPT, false = REJECT
pub async fn Read_File_Stream_Ack(
    stream: &mut libp2p::Stream,
) -> io::Result<bool>
```

**不变内容：** `Send_File_Data()` 和 `Receive_File_Data()` 保持不变，仍负责 raw data 分块传输。

---

### 5.2 `Src/Orchestrator/core.rs` — 删除旧状态，重写 B3 FileStreamArrived

**删除：**
- `FileMetadata` 结构体（第 48-51 行）
- `pending_file_receives` 字段（第 86 行）
- `handle_network_file()` 方法（第 187-250 行）
- `handle_inbound_request()` 中 `DataType::File` 分支（第 159-161 行），B2 仅保留 `DataType::Command`

**修改：** `handle_network_inbound()` 中 `FileStreamArrived` 分支

```rust
Network_Inbound_Event::FileStreamArrived { peer, mut stream } => {
    // 1. 读取 in-band header
    let (file_name, file_size, checksum) = match Read_File_Stream_Header(&mut stream).await {
        Ok(h) => h,
        Err(e) => {
            warn!("读取文件流 header 失败 from {}: {}", peer, e);
            return;
        }
    };

    // 2. 检查存储空间
    let quota = self.capabilities.storage.quota_info().await;
    if quota.total > 0 && file_size > quota.available {
        warn!("存储空间不足: need {}, available {}", file_size, quota.available);
        let _ = Write_File_Stream_Ack(&mut stream, false).await;
        return;
    }

    // 3. 回复 ACCEPT
    if let Err(e) = Write_File_Stream_Ack(&mut stream, true).await {
        warn!("写入 ACK 失败: {}", e);
        return;
    }

    // 4. compile ReceiveFile Job，注入 stream + metadata → spawn
    // （将 stream、file_name、file_size、checksum 注入 SlotFile）
    let job_id = JobId(generate_id());
    let program = match self.compiler.compile_receive_file(
        job_id, file_name.clone(), file_size, checksum
    ) {
        Ok(p) => p,
        Err(e) => {
            warn!("编译接收作业失败: {:?}", e);
            return;
        }
    };
    // ... allocate IO, inject stream into slot, spawn_job
}
```

---

### 5.3 `Src/Orchestrator/executor/handler_network.rs` — 重写发送端

**修改 `handle_send_file()`：**

旧流程（3 阶段）→ 新流程（1 阶段 stream）：

```rust
pub(super) async fn handle_send_file(&mut self, peer: SlotId, file: SlotId) -> StepResult {
    // 1. 解析 PeerId + file_id（不变）

    // 2. 从 Storage 获取 path + read_guard（不变）

    // 3. 获取 file_size + 计算 checksum（不变）

    // ═══ 新流程：Stream in-band ═══

    // 4. 打开文件流
    let mut stream = self.capabilities.network.open_file_stream(peer_id).await?;

    // 5. 写入 in-band header
    Write_File_Stream_Header(&mut stream, &file_id, file_size, &checksum_bytes).await?;

    // 6. 读取 ACK
    let accepted = Read_File_Stream_Ack(&mut stream).await?;
    if !accepted {
        return StepResult::Abort("SendFile: peer rejected file transfer".into());
    }

    // 7. 发送文件数据（复用现有 send_file_data）
    self.capabilities.network.send_file_data(&mut stream, &path).await?;

    // 8. 释放读锁
    drop(read_guard);

    StepResult::Continue
    // Phase 3 VERIFY_FILE 已删除
}
```

**`handle_receive_file()` 基本不变**，仍然从 slot 取出 stream/file_name/file_size/checksum 执行接收。

---

### 5.4 `Src/Network/data_protocol.rs` — 可选清理

`DataType::File` 不再被 Request-Response 使用。可选择：
- **保留**：不动，未来可能有其他用途
- **删除**：从 enum 中移除，同时清理 `From_U8` 的 `2 =>` 分支

建议：**保留但标记为 deprecated 注释**

---

### 5.5 `Src/Network/capability.rs` — 微调

`Network_Capability` trait：
- `open_file_stream()` — 不变
- `send_file_data()` / `receive_file_data()` — 不变
- 无需新增 trait 方法（header 读写通过独立函数完成，不经过 trait）

`Network_Inbound_Event::FileStreamArrived` — 不变（仍携带 `peer` + `stream`）

---

### 5.6 `Src/Network/inbound_manager.rs` — 无需改动

Inbound_Manager 仅处理 Request-Response 入站。文件传输改为纯 Stream 后不再经过此组件。

---

### 5.7 `Src/Orchestrator/command.rs` — 可选清理

如果 `NetworkProtocol::Verify_File` 仅用于文件验证，可删除该变体及其解析逻辑。

---

### 5.8 `Src/Storage/capability.rs` — 新增 checksum 转换辅助

当前 `checksum()` 返回 `String`（hex），新方案 header 需要 32 字节 raw bytes。可新增：

```rust
/// 计算 checksum 并返回原始字节（32 bytes Blake3）
async fn checksum_bytes(&self, file_id: &str) -> Result<[u8; 32], StorageError>;
```

或在 handler 端做 hex → bytes 转换。

---

## 6. 改动汇总表

| 文件 | 操作 | 改动量 |
|------|------|--------|
| `Src/Network/stream_protocol.rs` | 新增 4 个函数 | ~60 行 |
| `Src/Orchestrator/core.rs` | 删除 FileMetadata + pending_file_receives + handle_network_file；重写 FileStreamArrived 分支 | 净减 ~40 行 |
| `Src/Orchestrator/executor/handler_network.rs` | 重写 handle_send_file（删 Phase 1 RR + Phase 3 验证） | 净减 ~30 行 |
| `Src/Network/data_protocol.rs` | 可选：DataType::File 标记 deprecated | ~2 行 |
| `Src/Orchestrator/command.rs` | 可选：删除 Verify_File 变体 | ~10 行 |
| `Src/Storage/capability.rs` | 可选：新增 checksum_bytes 方法 | ~10 行 |
| `Src/Network/inbound_manager.rs` | 无需改动 | 0 |
| `Src/Network/capability.rs` | 无需改动 | 0 |

**净效果**：删除 ~70 行旧代码，新增 ~60 行协议函数，消除跨分支状态和时序竞态。
