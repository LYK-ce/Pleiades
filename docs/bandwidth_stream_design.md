# Bandwidth_Stream 设计文档

Presented by KeJi
Date ： 2026-05-16

## 1. 模块概述

`Bandwidth_Stream` 是 Network 模块内的**带宽测速传输子系统**，与 `File_Stream`、`Tensor_Stream` 并列，为 Network 提供第四种传输机制。

### 核心定义

> **Bandwidth_Stream = iperf 风格固定时长推流测速。**
> 替代当前借道 `request_response` + `DataType::BandwidthTest` + 三档取 max 的实现。
> 使用独立的 `/pleiades/bandwidth/1.0.0` 流协议：发送方推送固定时长（默认 3 秒），
> 接收方只计数字节、期满回传总量。**单次出结果，无需多档逼近。**

### 为什么不用 request_response

当前 `DataType::BandwidthTest` 占用了 `DataType` 枚举的一个槽位（5），混入了业务语义。`request_response` 的行为模型（发请求→等回复）本身不适合带宽测试——它要求对端把收到的数据再 `echo` 回来，测量的是往返时间而非单向上传带宽。

改用 libp2p Stream 后：

- 发送方只管推数据，不计时；接收方只计数不回传，**测量的是纯上传带宽**
- 一次测试、一个结果，不需要三档（1M/10M/50M）取 max
- 与 `File_Stream` / `Tensor_Stream` 完全同构，结构扁平
- `DataType` 枚举恢复为纯业务标签（Command/Data/File/Info），移除 `BandwidthTest`

### 模块结构

```
Network/Bandwidth_Stream/
├── mod.rs               ← 子模块入口 + re-export
└── protocol.rs          ← 协议常量 / Send_Bandwidth_Test / Receive_And_Count
```

### 调用关系

```
Orchestrator / Lua 脚本                       远端节点
─────                                         ─────
Capability::test_bandwidth(peer)
  │
  ├─ bandwidth_stream_control
  │   .open_stream(peer, "/pleiades/bandwidth/1.0.0")  ──→ accept 入站流
  │
  ├─ Run_Bandwidth_Test(stream, 3s)
  │   ├─ Send_Bandwidth_Test(stream, 3s)
  │   │   ├─ [2B: duration LE]                              ──→  读 duration
  │   │   ├─ 持续 3s 推送 64KB 全零 chunk                       →  循环接收 chunk 并累加
  │   │   └─ stream.close()                                 ──→  read()==0 → 停止
  │   │
  │   ├─ Read_Bandwidth_Result(stream)                      ←──  写 [8B: total_bytes LE]
  │   └─ Mbps = (total_bytes × 8) / 3s / 1e6
  │
  └─ 返回 Mbps（调用方自行决定是否写入 PeerManager）
```

出站测速走 `Capability` 直接操作 `stream::Control`，不经过 `Network_Service` 事件循环，不阻塞其他事件处理。与 `open_file_stream` / `open_tensor_stream` 调用路径完全一致。

---

## 2. 协议设计

### 2.1 协议标识符

```
/pleiades/bandwidth/1.0.0
```

### 2.2 线上帧格式

```
发送方 → 接收方:
+----------------------+----------------------------+--------+
|  duration_secs        |  64KB zero chunks...        |  EOF   |
|  2 bytes u16 LE      |  (持续推送固定时长)         | close  |
+----------------------+----------------------------+--------+

接收方 → 发送方:
+--------------------------+
|  total_bytes              |
|  8 bytes u64 LE          |
+--------------------------+
```

- `duration_secs`: 测试时长（秒），默认 3 秒，上限 10 秒
- `64KB chunks`: 全零填充的 64KB 数据块，循环推送直到时长到
- `EOF`: 发送方调用 `stream.close()` 关闭写端，接收方 `read() == 0` 结束
- `total_bytes`: 接收方实际收到的字节数（不含 2 字节 duration header）

### 2.3 与 File_Stream 的对比

| | File_Stream | Bandwidth_Stream |
|--|------------|-----------------|
| 协议标识 | `/pleiades/file-stream/1.0.0` | `/pleiades/bandwidth/1.0.0` |
| Header | 文件名+大小+校验和（变长） | duration（2 字节） |
| 数据体 | 真实文件内容 | 全零填充 |
| 握手 | Header ACK → 传数据 | 传数据 → 回传总量 |
| 结束方式 | 自然读完 | 发送方 close 写端 |
| 用途 | 文件传输 | 带宽测量 |

### 2.4 与 Tensor_Stream 的对比

| | Tensor_Stream | Bandwidth_Stream |
|--|--------------|-----------------|
| 流生命周期 | 长连接（推理会话） | 短连接（单次测速） |
| 帧数量 | 多帧（Prefill+多步Decode） | 一帧 |
| EOF | offset=u64::MAX 哨兵 | stream.close() |
| Handshake | inference_id（8 字节） | 无 handshake |

---

## 3. 常量与类型

### 3.1 常量

```rust
pub const BANDWIDTH_STREAM_PROTOCOL: &str = "/pleiades/bandwidth/1.0.0";

/// 分块大小（与 File_Stream 一致，64KB）
pub const CHUNK_SIZE: usize = 64 * 1024;

/// 默认测试时长（秒）
pub const DEFAULT_DURATION_SECS: u16 = 3;

/// 最大测试时长（秒），防止恶意节点指定超长时长
pub const MAX_DURATION_SECS: u16 = 10;
```

### 3.2 无自定义数据结构

与 File_Stream（有 FileHeader）和 Tensor_Stream（有 Tensor_Buffer）不同，Bandwidth_Stream 无自定义数据结构。协议数据仅为固定格式字节流，不需要结构体封装。

---

## 4. 核心实现

### 4.1 Send_Bandwidth_Test — 发送方推送数据

```rust
/// 发起带宽测试：写 duration，持续推送数据，关闭写端
///
/// 写入格式: [2B: duration LE][持续 64KB chunks][close]
///
/// # 参数
/// - stream: 已打开的出站流
/// - duration_secs: 测试时长（秒），调用方保证 ≤ MAX_DURATION_SECS
///
/// # 注意
/// - 推完 duration_secs 后调用 stream.flush() + stream.close() 通知对端结束
/// - 如果在推送过程中流断开，返回 io::Error
pub async fn Send_Bandwidth_Test(
    stream: &mut libp2p::Stream,
    duration_secs: u16,
) -> io::Result<()> {
    // 1. 写入 duration (2 bytes, u16 LE)
    stream.write_all(&duration_secs.to_le_bytes()).await?;
    stream.flush().await?;

    // 2. 持续 duration_secs 秒推送 64KB 全零 chunk
    let chunk = vec![0u8; CHUNK_SIZE];
    let deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_secs(duration_secs as u64);
    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        stream.write_all(&chunk).await?;
    }

    // 3. flush + close 写端，通知对端传输结束
    stream.flush().await?;
    stream.close().await?;
    Ok(())
}
```

### 4.2 Receive_And_Count — 接收方计数

```rust
/// 接收带宽测试数据并计数
///
/// 读取格式: [2B: duration LE]，然后持续读取 chunk 直到 read() == 0
///
/// # 返回
/// total_bytes（u64）—— 实际收到的数据字节数（不含 2 字节 header）
///
/// # 错误
/// - 如果 duration_secs > MAX_DURATION_SECS，返回 InvalidData 并立即关闭流
/// - 如果流提前异常关闭，返回 UnexpectedEof
pub async fn Receive_And_Count(
    stream: &mut libp2p::Stream,
) -> io::Result<u64> {
    // 1. 读取 duration (2 bytes, u16 LE)
    let mut dur_buf = [0u8; 2];
    stream.read_exact(&mut dur_buf).await?;
    let duration_secs = u16::from_le_bytes(dur_buf);

    // 2. 校验：duration 不能超过上限
    if duration_secs > MAX_DURATION_SECS {
        stream.close().await?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("duration {}s exceeds max {}", duration_secs, MAX_DURATION_SECS),
        ));
    }

    // 3. 循环读取 chunk 直到 EOF，累加字节数
    let mut total: u64 = 0;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break; // 对端关闭写端，传输结束
        }
        total += n as u64;
    }

    Ok(total)
}
```

### 4.3 Write_Bandwidth_Result / Read_Bandwidth_Result — 结果回传

```rust
/// 写入测速结果（接收方调用）
pub async fn Write_Bandwidth_Result(
    stream: &mut libp2p::Stream,
    total_bytes: u64,
) -> io::Result<()> {
    stream.write_all(&total_bytes.to_le_bytes()).await?;
    stream.flush().await
}

/// 读取测速结果（发送方调用）
pub async fn Read_Bandwidth_Result(
    stream: &mut libp2p::Stream,
) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    stream.read_exact(&mut buf).await?;
    Ok(u64::from_le_bytes(buf))
}
```

### 4.4 Run_Bandwidth_Test — 完整单次测速（Bandwidth_Stream 协议层）

```rust
/// 执行一次完整带宽测试并返回 Mbps
///
/// 打开流已由调用方完成，本函数负责推送数据 → 读取结果 → 计算 Mbps。
pub async fn Run_Bandwidth_Test(
    stream: &mut libp2p::Stream,
    duration_secs: u16,
) -> io::Result<u64> {
    Send_Bandwidth_Test(stream, duration_secs).await?;
    let total_bytes = Read_Bandwidth_Result(stream).await?;
    let mbps = (total_bytes as f64 * 8.0) / duration_secs as f64 / 1_000_000.0;
    Ok(mbps as u64)
}
```

### 4.5 test_bandwidth — Capability 对外接口

```rust
// Network_Capability trait
async fn test_bandwidth(&self, peer: PeerId) -> Result<u64, Network_Error>;

// Network_Service_Capability 实现
async fn test_bandwidth(&self, peer: PeerId) -> Result<u64, Network_Error> {
    let mut stream = self.bandwidth_stream_control
        .clone()
        .open_stream(peer, StreamProtocol::new(BANDWIDTH_STREAM_PROTOCOL))
        .await?;

    Run_Bandwidth_Test(&mut stream, DEFAULT_DURATION_SECS).await
        .map_err(|e| Network_Error::StreamIoError(format!("带宽测试失败: {}", e)))
}
```

不经过 `Network_Service` 事件循环，不阻塞其他事件。调用方（Orchestrator / Lua）拿到 Mbps 后自行决定是否写回 PeerManager。

---

## 5. 模块导出

```rust
// Bandwidth_Stream/mod.rs
pub mod protocol;

pub use protocol::{
    BANDWIDTH_STREAM_PROTOCOL, CHUNK_SIZE,
    DEFAULT_DURATION_SECS, MAX_DURATION_SECS,
    Send_Bandwidth_Test, Receive_And_Count,
    Write_Bandwidth_Result, Read_Bandwidth_Result,
};
```

Network 层重新导出（`Network/mod.rs`）：
```rust
pub mod bandwidth_stream;
```

与 File_Stream 导出方式完全对称。

---

## 6. 与 Network_Service 的协作

### 6.1 Network_Service 结构体新增字段

```rust
pub struct Network_Service {
    // ... 现有字段 ...

    /// 带宽测试流控制 — accept 入站测试流（被动接收）
    bandwidth_accept_control: stream::Control,
}
```

出站测速流控制（`bandwidth_stream_control`）由 `Network_Service_Capability` 独占，`Network_Service` 不持有。

### 6.2 Init() 新增

```rust
let bandwidth_accept_control = node_swarm.behaviour().stream.new_control();
let bandwidth_stream_control = node_swarm.behaviour().stream.new_control();  // → Capability
```

`bandwidth_stream_control` 直接移交给 `Network_Service_Capability::New()`，不存于 `Network_Service`。

### 6.3 Start() 新增

```rust
let mut incoming_bandwidth_streams = self.bandwidth_accept_control
    .accept(StreamProtocol::new(BANDWIDTH_STREAM_PROTOCOL))
    .expect("带宽测试流协议注册失败");
```

select! 分支处理入站流（被动接收端）：
```rust
Some((peer_id, mut stream)) = incoming_bandwidth_streams.next() => {
    match Receive_And_Count(&mut stream).await {
        Ok(total_bytes) => {
            if let Err(e) = Write_Bandwidth_Result(&mut stream, total_bytes).await {
                warn!("带宽测试结果回传失败 from {}: {}", peer_id, e);
            }
        }
        Err(e) => warn!("带宽测试入站失败 from {}: {}", peer_id, e),
    }
}
```

### 6.4 出站测速走 Capability

出站测速（主动发起）不经过 `Handle_Command`，由 `Network_Service_Capability::test_bandwidth` 直接操作 `stream::Control`：

```rust
// 调用方（Orchestrator / Lua）
let mbps = network_capability.test_bandwidth(peer).await?;
// 自行决定是否写入 PeerManager
```

与 `open_file_stream` 调用路径完全一致，不阻塞事件循环。

### 6.5 删除项

| 删除的内容 | 位置 | 原因 |
|-----------|------|------|
| `DataType::BandwidthTest = 5` | `Request_Response/codec.rs` | 不再通过 request_response 测速 |
| `Handle_Bandwidth_Test_Inbound()` | `network_service.rs` | 被 Start() 中的 stream accept 替代 |
| `DataType::BandwidthTest` match 分支 | `network_service.rs` | 不再有此类型入站请求 |
| `test_single_bandwidth()` | `network_service.rs` | 被 `Run_Bandwidth_Test` 替代 |
| `Test_Bandwidth()` | `network_service.rs` | 出站测速走 Capability，不在事件循环 |
| `bandwidth_open_control` 字段 | `network_service.rs` | 仅 Capability 需要 stream::Control |
| `NodeCommand::UpdateInfo` 变体 | `node_handle.rs` | 带宽测试不再走命令通道 |
| `NodeHandle::Update_Info()` 方法 | `node_handle.rs` | 同上 |
| `test_sizes` 数组 `[1M, 10M, 50M]` | `network_service.rs` | 不再需要 |

---

## 7. 已知风险

### 7.1 单向上传偏差

当前方案测量的是**上传带宽**。在非对称网络中（如大多数家庭宽带，下行远大于上行），测得的值不能代表下行能力。对于推理场景，模型层分发主要是下载，目前用上传带宽近似，可能有偏差。

- **当前状态**：局域网环境通常对称，影响很小
- **未来优化**：双向测速——接收方在 `Receive_And_Count` 中同时计时自己的接收窗口，回传 `(total_bytes, elapsed_ms)` 让发送方也能得到接收方的视角

### 7.2 stream.close() 尾部延迟

`Send_Bandwidth_Test` 中 `flush()` + `close()` 会等待远端确认关闭。在网络延迟较高时，close 本身耗时会被计入 buffer 中未发数据的 flush 时间。对 3 秒测试影响约 1-5%。

### 7.3 入站流 accept 积压

多个节点同时发起带宽测试时，入站流在 `incoming_bandwidth_streams` 通道中排队。`tokio::select!` 公平调度，但 `Receive_And_Count` 是持续 3-10 秒的阻塞异步操作，会短暂阻塞 Swarm 事件循环中的其他分支。当前出站测速已走 Capability 直接操作 `stream::Control`，不经过事件循环。如果未来并发测速场景增多，可将接收逻辑 spawn 到独立 task。

### 7.4 与旧协议的不兼容

此改动移除 `DataType::BandwidthTest`，旧版节点发送的 `BandwidthTest` 类型请求将在 `DataType::From_U8` 中返回错误。这是可接受的破坏性变更——`DataType` 成员值改变属于内部协议演进，不涉及流协议升级。

---

## 8. 重构历史

| 变更 | 说明 |
|------|------|
| 新建 `Bandwidth_Stream/` 子模块 | 从 `network_service.rs` 中提取带宽测试协议 |
| 删除 `DataType::BandwidthTest` | 不再占用 request_response 枚举槽位 |
| 删除 `Handle_Bandwidth_Test_Inbound` | 入站处理迁移到 Start() stream accept |
| 协议从"固定量"改为"固定时长" | 三档取 max → iperf 单次出结果 |
| 新增 `Run_Bandwidth_Test` | 完整单次测速（发送数据+读取结果+计算），纯协议函数 |
| Network_Service 新增 `bandwidth_accept_control` | 仅持有入站 accept 控制，不持有出站 |
| Capability 新增 `bandwidth_stream_control` | 出站测速走 Capability 直接操作 stream::Control |
| 删除 `Test_Bandwidth` | 出站测速不在事件循环，走 Capability 接口 |
| 删除 `NodeCommand::UpdateInfo` | 带宽测试不再走命令通道 |
| 删除 `NodeHandle::Update_Info` | 对外接口为 `Capability::test_bandwidth` |

---

## 9. TODO

暂无。
