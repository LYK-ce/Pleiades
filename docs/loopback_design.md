# 自环 (Loopback) 实现方案

> Presented by KeJi
> Date: 2026-05-20

## 动机

用统一的 `send_data` 接口替代专用线程回调。任何内部线程（profile、推理等）需要向主流程投递数据时，
直接 `send_data(local_peer_id, DataType::xxx, payload)`，避免为每种场景定制输出通道。

## 现状

- `send_data` → `NodeHandle::Send_Data` → `NodeCommand::SendData` → `command_handler.rs` → libp2p `send_request`
- libp2p 层不支持向自己发 `send_request`（无自连接），当前会超时 30s 返回错误
- mDNS 层已有自环过滤 `if peer_id != self.local_peer_id`，但 send_data 路径无此检查

## 方案

在 `command_handler.rs` 的 `NodeCommand::SendData` 分支中增加自环检测，
`peer == self.local_peer_id` 时绕过 libp2p，本地模拟入站处理。

### 修改文件

仅 `Src/Network/command_handler.rs`

### 实现逻辑

```
NodeCommand::SendData { peer, data_type, payload, response_tx } =>
    if peer == self.local_peer_id:
        match data_type:
            Info   → 解析 payload "name|models_json" → peer_handle.Update_Peer_Name / Update_Supported_Models
            Data   → 直接回复 OK（通过 response_tx oneshot）
            Command/File → inbound_manager 转发给 Orchestrator（模拟外部入站）
        // 所有分支通过 response_tx 返回 Network_Data { data_type, payload: b"OK" }
    else:
        // 现有逻辑：libp2p send_request
```

### 典型用例

```
// profile 线程完成后自更新 PeerManager
let payload = format!("{}|{}", local_name, models_json).into_bytes();
network.send_data(local_peer_id, DataType::Info, payload).await;
// → command_handler 本地消化，更新 PeerManager，无需专用接口
```

## 不涉及

- libp2p 层不改动
- API 签名不变
- TUI/Orchestrator 无需适配

---

## Tensor Stream 自环

> 与 send_data 自环对称，线程间复用 Tensor Stream 协议通信

### 动机

推理服务需要两个线程：Prompt 收集线程 + ML 推理线程。两者之间通过 Tensor Stream
（send_tensor / recv_tensor / send_eof）通信。自环避免为同进程线程间通信单独设计通道。

### 方案

在 `capability.rs` 的 `open_tensor_stream` 和 `accept_tensor_stream` 中检测 `peer == local_peer_id`，
绕过 libp2p，用 `tokio::io::duplex()` 创建本地内存通道。

```
Prompt线程                               ML推理线程
  open_tensor_stream(self, id)             accept_tensor_stream(id)
       │                                        │
       ▼                                        ▼
  [duplex 本地通道]  ←── 内存对接 ──→  [duplex 本地通道]
       │                                        │
  Send_Tensor_Frame(buf)                Receive_Tensor_Frame(buf)
  Receive_Tensor_Frame(buf)             Send_Tensor_Frame(buf)
  Send_EOF()                            (收到 EOF 结束)
```

### 修改文件

仅 `Src/Network/capability.rs`

### 实现逻辑

```
open_tensor_stream(peer, inference_id):
    if peer == local_peer_id:
        (local, remote) = tokio::io::duplex(64KB)
        rendezvous_map.insert(inference_id, remote)  // 供 accept 取出
        return Ok(local)
    else:
        // 现有逻辑：libp2p stream::Control::open_stream

accept_tensor_stream(inference_id, timeout):
    if let Some(stream) = rendezvous_map.try_take(inference_id):
        return Ok(stream)
    else:
        // 现有逻辑：accept + handshake
```

### 协议层零改动（不准确，见下）

> 实际 `protocol.rs` 函数签名为 `&mut libp2p::Stream`，`duplex()` 返回的
> `DuplexStream` 类型不兼容。需要将函数改为 `S: AsyncRead + AsyncWrite + Unpin` 泛型，
> 或采用下方"独立本地流"方案完全避开此问题。

### 不涉及

- libp2p 层不改动
- Lua 层需要适配（`NetworkStream` 需兼容 duplex）

---

## 独立本地流（推荐）

> 不碰 Tensor Stream，单独新建一套 Lua 能力用于同进程线程间通信。
> 两条流各自独立，互不约束。

### 动机

Tensor Stream 协议层绑定 `libp2p::Stream` 具体类型，要支持自环需改 `protocol.rs`
泛型化 + 改 `NetworkStream` userdata 枚举包装，改动面大且语义混乱。

不如拆分为两条流，各管各的：Tensor Stream 只管跨节点，新建 `local` 能力管线程间。

### 方案

Lua 层分两个命名空间：

```lua
-- === 网络推理（跨节点 / Tensor Stream）===
local s = network.open_tensor_stream(peer, id)
network.send_tensor(s, tensor)
network.recv_tensor(s, "cpu")
network.send_eof(s)

-- === 本地推理（同进程两个线程 / 本地流）===
local s = local.open_stream(id)      -- tokio::io::duplex() 内部对接
local.send_tensor(s, tensor)         -- 同帧格式
local.recv_tensor(s, "cpu")
local.send_eof(s)
```

底层实现：

```
Prompt线程                               ML推理线程
  local.open_stream("infer-1")             local.accept_stream("infer-1")
       │                                        │
       ▼                                        ▼
  [DuplexStream 左半]  ←── 内存对接 ──→  [DuplexStream 右半]
       │                                        │
  Send/Recv 本地帧                       Send/Recv 本地帧
```

### 修改文件

| 文件 | 改动 |
|------|------|
| `Src/VM/capability_binding.rs` | 新增 `register_local_stream_caps()` 注册 `local` 表 |
| `Src/VM/network_stream.rs` | 新增 `LocalStream` userdata，包 `Mutex<DuplexStream>` |

### 本地帧格式

与 Tensor Stream 帧格式**完全相同**——`[8B offset LE][8B length LE][data]` + EOF 哨兵。
复用 `Send_Tensor_Frame` / `Receive_Tensor_Frame` / `Send_EOF` 的函数**逻辑**，
但不复用函数签名（输入类型不同）。

### 对 Tensor Stream 零影响

- `protocol.rs` 不动
- `NetworkStream` 不动
- `network.open_tensor_stream` 不动
- `capability.rs` 不动

### 不涉及

- libp2p 层
- Tensor Stream 协议层
- 跨节点推理逻辑

---

## Spawn 线程生命周期管理

### 问题

`std::thread::spawn()` 创建的 OS 线程一旦进入 GPU 阻塞计算（`forward()`），
从外部无法主动终止。Rust 标准库不提供 `thread.kill()`，理由：
- 被 kill 线程可能持有 `Mutex` → 死锁
- `Arc` 引用计数未减 → 堆泄漏
- `unsafe` 代码可能正写裸指针 → UB

POSIX 的 `pthread_cancel` 存在但 Rust 不封装，暴力杀线程不保证资源清理。

### 现有手段

| 手段 | 机制 | 能否杀 GPU kernel |
|------|------|-------------------|
| `AtomicBool` 协作退出 | 共享 flag，线程在每步之间检查 | ❌ 单次 `forward()` 内部无解 |
| `CancellationToken` | 同上，Tokio 风格封装 | ❌ 同样依赖 await 点 |
| `JoinHandle::abort()` | tokio task 级强制取消 | ❌ 不适用于 `std::thread` |
| `pthread_cancel` (FFI) | OS 级杀线程 | ⚠️ 可能但不保证清理 |
| `process::exit(0)` | 杀整个进程 | ✅ 但同归于尽 |

### 建议

1. **短期**：`forward()` 内部不拆分的情况下接受"自然结束"，用 `AtomicBool` 加速空闲/等待阶段的退出
2. **中长期**：探索 CUDA stream callback (`cudaLaunchHostFunc`) 实现 GPU kernel 级的中断检查

