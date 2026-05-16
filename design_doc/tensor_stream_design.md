# Tensor Stream 设计文档

Presented by KeJi
Date ： 2026-05-14

## 1. 模块概述

`Tensor_Stream` 是 `Network` 模块的子模块，负责管理分布式推理中的 **libp2p 张量流**。它替代原有的 `Tensor_IO` 独立模块，将流的管理回归 Network 内部。

### 核心定义

> **Tensor_Stream = 张量流的建立 + 交接 + 帧传输。**
> 提供 Rendezvous 双向匹配机制解决入站流从 Network Event Loop（tokio）到 ML Thread（OS 线程）的跨上下文交接问题。
> Core 不参与流路由，ML Thread Lua 直接管理 stream 的收发与生命周期。

### 模块结构

```
Network/Tensor_Stream/
├── mod.rs           ← 子模块声明 + pub use 导出
├── protocol.rs      ← 第一层：帧格式、Tensor_Buffer、Handshake
└── rendezvous.rs    ← 第二层：RendezvousMap 双向匹配

Network_Capability trait ← 第三层：组合 protocol + rendezvous
```

### 调用关系

```
ML Thread Lua (OS thread)              Network Event Loop (tokio)
         │                                      │
         │ open_tensor_stream(peer, id)          │
         │   → trait → control.open_stream       │
         │   → Write_Handshake                   │
         │                                      │
         │ accept_tensor_stream(id, tout)        │ 入站流到达
         │   → rendezvous.register_accept(id)    │   → Read_Handshake(id)
         │   → block_on(oneshot) ──等待──→       │   → rendezvous.insert_inbound(id, stream)
         │   ←──────── 收到 stream ──────────    │       → tx.send(stream) 唤醒
         │                                      │
         │ send_tensor(stream, frame_data)       │
         │ receive_tensor(stream) → frame_data   │
         │ close_tensor_stream(stream)           │
```

---

## 2. 数据结构

### 2.1 Tensor_Buffer — 预分配可复用缓冲区

```rust
pub struct Tensor_Buffer {
    buf: Vec<u8>,
    valid_len: usize,
}
```

| 方法 | 说明 |
|------|------|
| `New(capacity)` | 创建指定初始容量的缓冲区 |
| `As_Mut_Slice(len)` → `&mut [u8]` | 获取可写切片，自动扩容 |
| `As_Slice()` → `&[u8]` | 获取有效数据的只读切片 |
| `Len()` → `usize` | 有效数据长度 |
| `Is_Empty()` → `bool` | 是否为空 |
| `Clear()` | 清空有效标记，不释放内存 |

### 2.2 RendezvousMap — 双向匹配器

```rust
pub struct RendezvousMap {
    inner: std::sync::Mutex<RendezvousInner>,
}

struct RendezvousInner {
    pending_inbound: HashMap<u64, libp2p::Stream>,         // 流等人（入站流先到）
    pending_accept:  HashMap<u64, oneshot::Sender<Stream>>, // 人等流（accept 先调用）
}
```

| 方法 | 调用方 | 说明 |
|------|--------|------|
| `new()` | `Network_Service::Init()` | 创建空 Map |
| `insert_inbound(id, stream)` | Network Event Loop | 流到达 → 查 `pending_accept`，命中则 `tx.send` 唤醒，否则存入 `pending_inbound` |
| `register_accept(id)` → `Receiver<Stream>` | ML Thread（通过 Capability） | accept 调用 → 查 `pending_inbound`，命中则立即返回，否则创建 oneshot 存入 `pending_accept` |

**锁方案**：`std::sync::Mutex`。Network Event Loop（tokio）短期持锁，ML Thread（OS 线程）兼容 `block_on`。

**匹配逻辑**（同一把锁保证原子性）：

```
流先到:  pending_accept[id] 有 Tx?  Yes → tx.send(stream) 唤醒  No → 存入 pending_inbound[id]
人先到:  pending_inbound[id] 有流?  Yes → 立即返回            No → 创建 Tx 存入 pending_accept[id]
```

### 2.3 Network_Service_Capability 新增字段

```rust
pub struct Network_Service_Capability {
    node_handle: NodeHandle,
    file_stream_control: stream::Control,
    tensor_stream_control: stream::Control,
    tensor_rendezvous: Arc<RendezvousMap>,   // ← 新增
}
```

---

## 3. Trait 定义

### 3.1 Network_Capability 张量流方法

```rust
#[async_trait]
pub trait Network_Capability: Send + Sync {
    // ... 现有方法不变 ...

    /// 打开到目标节点的张量流（自动写入 handshake）
    ///
    /// 内部流程：
    /// 1. control.open_stream(peer, "/pleiades/tensor/1.0.0")
    /// 2. Write_Tensor_Stream_Handshake(stream, inference_id)
    async fn open_tensor_stream(
        &self, peer: PeerId, inference_id: u64,
    ) -> Result<libp2p::Stream, Network_Error>;

    /// 等待对端发来的张量流（rendezvous 匹配）
    ///
    /// 内部流程：
    /// 1. rendezvous.register_accept(inference_id) → Receiver
    /// 2. tokio::time::timeout(timeout, rx).await
    async fn accept_tensor_stream(
        &self, inference_id: u64, timeout_secs: u64,
    ) -> Result<libp2p::Stream, Network_Error>;
}
```

---

## 4. 协议格式

### 4.1 Handshake 协议

```
发起方：open_tensor_stream → 写 [inference_id: u64 LE, 8 bytes] → flush
接收方：入站流到达 → Network Event Loop 读前 8 bytes → 得 inference_id → rendezvous 匹配
```

| 函数 | 说明 |
|------|------|
| `Write_Tensor_Stream_Handshake(stream, id)` | 写入 8 字节 inference_id |
| `Read_Tensor_Stream_Handshake(stream)` → `u64` | 读取 8 字节 inference_id |

### 4.2 数据帧格式（占位，待细化）

> ⚠️ 当前单 offset 格式 `[8B offset][8B len][data]` 不兼容多 batch 独立 offset。
> 后续需改为 `[num_batches][offsets...][total_len][data]`。

| 函数（占位签名） | 说明 |
|------------------|------|
| `Send_Tensor_Frame(stream, frame_data)` | 发送张量帧 |
| `Receive_Tensor_Frame(stream, buffer)` → frame_data | 接收张量帧 |
| `Send_EOF(stream)` | 发送 EOF 哨兵帧 |

`Send_Tensor_Frame` / `Receive_Tensor_Frame` / `Send_EOF` 不进 `Network_Capability` trait。它们是纯工具函数，ML Thread Lua 通过 `block_on` 直接调用。

---

## 5. 模块导出

```rust
// Src/Network/Tensor_Stream/mod.rs
pub mod protocol;
pub mod rendezvous;
```

```rust
// Src/Network/mod.rs
pub mod tensor_stream;
pub use tensor_stream::protocol::{
    TENSOR_STREAM_PROTOCOL, TENSOR_EOF_OFFSET,
    Tensor_Buffer,
    Send_Tensor_Frame, Receive_Tensor_Frame, Send_EOF,
    Write_Tensor_Stream_Handshake, Read_Tensor_Stream_Handshake,
};
pub use tensor_stream::rendezvous::RendezvousMap;
```

---

## 6. Lua API

ML Thread Lua 通过 `Network_Capability` trait 的 async 方法 + `block_on` 桥接调用：

```lua
stream = net.open_tensor_stream(peer_id, inference_id)      -- 出站 + handshake
stream = net.accept_tensor_stream(inference_id, timeout_sec) -- 入站 + 匹配
net.send_tensor(stream, frame_data)                          -- 发送（帧格式待定）
frame_data = net.receive_tensor(stream)                      -- 接收（帧格式待定）
net.close_tensor_stream(stream)                              -- 发送 EOF + 关闭
```

---

## 7. 协作关系

| 职责 | 负责方 | 说明 |
|------|--------|------|
| inference_id 生成 | Core | 推理会话唯一标识，作为参数传入 ML Thread |
| 流建立 | ML Thread Lua | open + accept 直管 stream 全生命周期 |
| 入站流路由 | Network_Service Event Loop | 读 handshake → rendezvous 匹配 |
| 编排/调度/容错 | ML Thread Lua | 全权由用户脚本决定 |
| 通知对端启动 | ML Thread Lua | 通过 `send_command` 或其他网络 API |

---

## 8. 已知风险

| 风险 | 说明 |
|------|------|
| 单 inference_id 多流 | 当前设计一 inbound + 一 outbound。若未来需冗余备份/多路传输，rendezvous 需支持一个 id 对应多条流 |
| 帧格式多 batch 兼容 | 当前单 offset 格式需改造为多 batch 独立 offset，protocol.rs 帧格式待细化 |
| `accept_tensor_stream` 超时后流泄漏 | `insert_inbound` 存入的 stream 若无人来取，不会自动清理 |

---

## 9. 与旧模块（Tensor_IO）的差异

| 方面 | Tensor_IO (旧) | Tensor_Stream (新) |
|------|---------------|-------------------|
| 模块位置 | `Src/Tensor_IO/` 独立模块 | `Src/Network/Tensor_Stream/` 子模块 |
| 流交接 | Switch 分步注册 + 外部轮询 | Rendezvous 双向匹配，自动交付 |
| 热切换 | 设计存在但实现断裂 | 由 Lua 脚本自行管理 |
| Core 参与 | Core 调 Register + Create Endpoint | Core 完全不参与 |
| 代码量 | ~465 行独立模块 | ~200 行 Network 内部 |
| Lua API | 无 | open/accept/send/receive/close |

---

## 10. TODO

### 帧格式多 batch 支持

当前 `Send_Tensor_Frame` / `Receive_Tensor_Frame` 使用单 offset 格式。需改造为支持多 batch 独立 offset 的新帧格式。

### accept_tensor_stream 流清理

`insert_inbound` 存入的 stream 若无匹配（accept 超时或永不调用），需要超时清理机制。
