# Tensor Stream 重构方案

## 0. 核心定义

> **Tensor Stream = libp2p 张量流的建立与交接。**
> 删除 `Src/Tensor_IO/` 独立模块，将流的管理回归 `Src/Network/` 内部。
> Core 不再参与 tensor 流路由，ML Thread Lua 直接管理 stream 的收发与生命周期。

---

## 1. 问题分析

### 1.1 当前架构的问题

```
┌─ Tensor_IO（~465 行独立模块）─────┐
│ Tensor_Port_Switch               │
│ Pipeline_Entry                   │
│ Tensor_IO_Endpoint               │
│ FailureReport                    │
│ Tensor_IO_Error                  │
│ 分步注册 + 轮询匹配               │
└──────────────────────────────────┘
         ↑           ↑
    Core 操作     ML Thread 使用
```

| 问题 | 详情 |
|------|------|
| **热切换不可行** | `Create_Endpoint` 创建全新 `Arc`，与 Switch 侧的 `Pipeline_Entry` 断开——共享锁的设计前提被破坏 |
| **组件冗余** | v0.2 架构下 ML Thread 跑 Lua，Lua 应直管 Stream。Switch 的编排、偏移追踪、广播全部可移至 Lua |
| **Core 过度参与** | `Register_Inbound` / `Create_Endpoint` 路由逻辑在 Core 中，违背 "Lua=逻辑, Rust=纯API" |
| **接口复杂** | 分步注册三步，轮询等待，代码量大但实际只做一件事：将 stream 从 Network Event Loop 交给 ML Thread |

### 1.2 不可绕过的约束

```
入站流到达位置：Network Event Loop (tokio task)
流消费位置：      ML Thread (OS thread)
```

两者是**不同执行上下文**，必须有一个共享数据结构做交接。这不是设计选择，是多线程物理约束。

### 1.3 双流到达时序问题

```
Coordinator                              Relay
    │                                      │
    │── open_tensor_stream(Relay, id=X) ──→│────→ Relay.Inbound: rendezvous[X] 匹配 ✓
    │                                      │
    │←── open_tensor_stream(Coord, id=X) ──│──→ Coord.Inbound: rendezvous[X] 匹配 ✓
```

两边各开一条出站流，对方的入站事件到达时序不确定。Rendezvous 机制需要处理"谁先到谁等人"的双向匹配。

---

## 2. 新方案

### 2.1 架构

```
Network_Service 内部:
┌────────────────────────────────────────────┐
│  pending_inbound: Mutex<HashMap<u64, Stream>>│  ← 流先到，等人取
│  pending_accept: Mutex<HashMap<u64, Tx>>    │  ← 人先到，等流来
└────────────────────────────────────────────┘

ML Thread Lua（直接调用）:
  net.open_tensor_stream(peer, inference_id) → stream
  net.accept_tensor_stream(inference_id)      → stream  —— rendezvous 匹配
  net.send_tensor(stream, data)
  net.receive_tensor(stream)                  → data
```

### 2.2 流建立流程

```
1. Core 生成 inference_id（推理会话唯一标识，用户可见），spawn ML Thread(inference_id, script, model)
2. ML Thread Lua（全权负责调度与推理）:
     -- 选节点、通知 Relay（通过 caps:send_command）
     out = net.open_tensor_stream(relay_peer, X)   -- 出站
     inp = net.accept_tensor_stream(X, 30)         -- 入站，rendezvous 等待（30s 超时）
     ml.load_model("qwen3", ...)
     -- 推理循环
3. Relay Core 收到通知后 spawn Relay ML Thread(inference_id, script, model)
     -- Relay ML Thread Lua 同上
```

### 2.3 Lua API 设计

```lua
-- 出站：打开 tensor stream 到指定 peer，自动写入 8 字节 handshake(inference_id)
-- 返回：stream 句柄（userdata 或 integer）
stream = net.open_tensor_stream(peer_id, inference_id)

-- 入站：等待对端发来的 tensor stream（rendezvous 匹配）
-- 阻塞直到匹配或超时。返回 stream 句柄或 nil
stream = net.accept_tensor_stream(inference_id, timeout_sec)

-- 发送张量帧到 stream
-- frame_data: 帧内容格式待定（支持多 batch 独立 offset）
net.send_tensor(stream, frame_data)

-- 从 stream 接收一帧张量
-- 返回：frame_data 或 nil（超时/EOF/错误）
-- frame_data 格式待定（支持多 batch 独立 offset）
frame_data = net.receive_tensor(stream)

-- 关闭 tensor stream（先发 EOF 再关）
net.close_tensor_stream(stream)
```

### 2.4 Rendezvous 匹配逻辑

```
Network Event Loop (tokio)            ML Thread Lua (OS thread)
│                                    │
│ 入站流到达, 读 handshake(id=X)      │ net.accept_tensor_stream(X)
│   ↓                                │   ↓
│ pending_accept[X] 有 Tx?          │ pending_inbound[X] 有 stream?
│   Yes → send(stream) → 删除Tx      │   Yes → 返回 stream
│   No  → 存入 pending_inbound[X]     │   No  → 创建 oneshot → 存入 pending_accept[X]
│                                     │          └→ block_on(rx) 等待
```

- 两端都用 `std::sync::Mutex`（Network Event Loop 短期持锁，ML Thread `block_on` 兼容）
- `inference_id` 不保证全局唯一（不同节点可能生成相同 id），但同一个节点内 rendezvous 只匹配本地

### 2.5 Handshake 协议保留

```
发起方：open_tensor_stream → 写 [inference_id: u64 LE, 8 bytes] → flush
接收方：入站流到达 → 读前 8 bytes → 得 inference_id → rendezvous 匹配
```

当前帧格式/Handshake 函数从 `tensor_stream_protocol.rs` 迁移至 `Src/Network/Tensor_Stream/protocol.rs`，调用方从 Core 改为 Network_Service 内部。

### 2.6 模块内部结构

```
Src/Network/Tensor_Stream/
├── mod.rs           -- pub mod protocol; pub mod rendezvous;
├── protocol.rs      -- 第一层：纯函数，无状态
└── rendezvous.rs    -- 第二层：有状态，双向匹配

Network_Capability trait（第三层：组合 protocol + rendezvous）
```

#### 第一层：protocol.rs（纯函数）

> ⚠️ 帧格式待定：当前单 offset `[8B offset][8B len][data]` 不兼容多 batch 独立 offset。
> 后续需改为 `[num_batches][offsets...][total_len][data]`。以下函数签名以占位符表示，暂不细化。

| 导出项 | 类型 | 说明 |
|--------|------|------|
| `TENSOR_STREAM_PROTOCOL` | `&str` | 协议标识符 `/pleiades/tensor/1.0.0` |
| `TENSOR_EOF_OFFSET` | `u64` | EOF 标记值 `u64::MAX` |
| `Tensor_Buffer` | struct | 预分配可复用缓冲区：`New(cap)` / `As_Slice()` / `As_Mut_Slice(len)` / `Len()` / `Is_Empty()` / `Clear()` |
| `Send_Tensor_Frame(stream, frame_data)` | async fn | （占位）发送张量帧 |
| `Receive_Tensor_Frame(stream, buffer)` → frame_data | async fn | （占位）接收张量帧到预分配 buffer |
| `Send_EOF(stream)` | async fn | （占位）发送 EOF 哨兵帧 |
| `Write_Tensor_Stream_Handshake(stream, id)` | async fn | 写入 8 字节 inference_id |
| `Read_Tensor_Stream_Handshake(stream)` → id | async fn | 读取 8 字节 inference_id |

#### 第二层：rendezvous.rs（双向匹配）

```rust
pub struct RendezvousMap { inner: std::sync::Mutex<RendezvousInner> }

struct RendezvousInner {
    pending_inbound: HashMap<u64, libp2p::Stream>,        // 流等人
    pending_accept:  HashMap<u64, oneshot::Sender<Stream>>, // 人等流
}
```

| 方法 | 调用方 | 说明 |
|------|--------|------|
| `new()` | Network_Service::Init | 创建空 Map |
| `insert_inbound(id, stream)` | Network Event Loop | 入站流到达 → 查 `pending_accept`，有人等则 `tx.send` 唤醒，否则存入 `pending_inbound` |
| `register_accept(id)` → `Receiver<Stream>` | ML Thread（通过 Capability） | accept 调用 → 查 `pending_inbound`，有流则立即返回，否则创建 oneshot 存入 `pending_accept` |

同一把 `std::sync::Mutex` 保护两个 HashMap，保证检查+插入原子。

#### 第三层：Network_Capability trait（组合 protocol + rendezvous）

```rust
#[async_trait]
pub trait Network_Capability: Send + Sync {
    // ... 现有方法不变 ...

    /// 打开到目标节点的张量流（自动写入 handshake）
    ///
    /// 内部：control.open_stream(peer, "/pleiades/tensor/1.0.0") + Write_Handshake
    async fn open_tensor_stream(
        &self, peer: PeerId, inference_id: u64,
    ) -> Result<libp2p::Stream, Network_Error>;

    /// 等待对端发来的张量流（rendezvous 匹配）
    ///
    /// 内部：rendezvous.register_accept(id) + timeout + oneshot 等待
    async fn accept_tensor_stream(
        &self, inference_id: u64, timeout_secs: u64,
    ) -> Result<libp2p::Stream, Network_Error>;
}
```

`Network_Service_Capability` 新增字段：

```rust
pub struct Network_Service_Capability {
    node_handle: NodeHandle,
    file_stream_control: stream::Control,
    tensor_stream_control: stream::Control,
    tensor_rendezvous: Arc<RendezvousMap>,   // ← 新增
}
```

`Send_Tensor_Frame` / `Receive_Tensor_Frame` / `Send_EOF` **不进 trait**。它们是纯工具函数，ML Thread Lua 通过 `block_on` 直接调用。具体帧格式由 `protocol.rs` 内部决定（当前为占位，待多 batch 支持细化）。

#### ML Thread Lua 侧的调用链路

```
ML Thread Lua (OS thread):
  stream = net.open_tensor_stream(peer, id)
    → block_on → Network_Capability::open_tensor_stream(peer, id)
      → control.open_stream + Write_Handshake

  stream = net.accept_tensor_stream(id, 30)
    → block_on → Network_Capability::accept_tensor_stream(id, 30)
      → rendezvous.register_accept(id) + timeout

  net.send_tensor(stream, frame_data)
    → block_on → Send_Tensor_Frame(stream, frame_data)   ← 帧格式待定

  frame_data = net.receive_tensor(stream)
    → block_on → Receive_Tensor_Frame(stream, buffer) → frame_data
    → 返回给 Lua                                              ← 帧格式待定
```

---

## 3. 影响范围（本次仅限以下文件，不动其他模块）

> ⚠️ 核心原则：本次只删 `Tensor_IO/`、建 `Tensor_Stream/`（含 `capability.rs` + `network_service.rs` 集成 rendezvous）、改 `mod.rs` 和 `lib.rs`。
> Core / Orchestrator / main.rs 等即使引用旧类型导致编译报错，也暂不处理。

### 3.1 删除

| 文件/目录 | 说明 |
|-----------|------|
| `Src/Tensor_IO/` 整个目录 | `mod.rs` + `tensor_port_switch.rs` + `task.md` + `tensor_io_design.md` |
| `Src/Network/tensor_stream_protocol.rs` | 整文件删除（内容迁入 `Tensor_Stream/protocol.rs`） |

### 3.2 新建：`Src/Network/Tensor_Stream/` 子目录

```
Src/Network/Tensor_Stream/
├── mod.rs           -- 子模块声明 + pub use 统一导出
├── protocol.rs      -- 帧格式、Tensor_Buffer、Handshake（从 tensor_stream_protocol.rs 迁移，删除 `Tensor_IO_Handle`）
└── rendezvous.rs    -- RendezvousMap（双向匹配，同一把锁）
```

| 文件 | 说明 |
|------|------|
| `Src/Network/Tensor_Stream/mod.rs` | 🆕 声明 `pub mod protocol; pub mod rendezvous;` 导出 |
| `Src/Network/Tensor_Stream/protocol.rs` | 📦 从 `tensor_stream_protocol.rs` 整体迁移，内容不变 |
| `Src/Network/Tensor_Stream/rendezvous.rs` | 🆕 ~60 行：`RendezvousMap` 结构体 |

### 3.3 修改

| 文件 | 操作 | 说明 |
|------|------|------|
| `Src/Network/mod.rs` | ✏️ | `mod tensor_stream_protocol` → `mod tensor_stream`；更新所有重导出路径 |
| `Src/Network/capability.rs` | ✏️ | import 路径更新；trait 新增 `open_tensor_stream(peer, id)` 签名变更 + `accept_tensor_stream(id, timeout)` 新方法；`Network_Service_Capability` 新增 `tensor_rendezvous: Arc<RendezvousMap>` 字段 |
| `Src/Network/network_service.rs` | ✏️ | 添加 `rendezvous: Arc<RendezvousMap>`；入站 tensor stream 处理改为读 handshake → rendezvous 匹配；import 路径更新 |
| `Src/lib.rs` | ✏️ | 移除 `pub mod tensor_io` 及相关 export；更新 `pub use network::tensor_stream_protocol` → `pub use network::tensor_stream` |

### 3.4 不变

| 模块 | 说明 |
|------|------|
| `Tensor_Stream/protocol.rs` 帧格式 | `Send_Tensor_Frame` / `Receive_Tensor_Frame` / `Send_EOF` / `Tensor_Buffer` 保留，**帧内容格式用占位符，待多 batch offset 细化** |
| `Src/Orchestrator/` | 暂不修改（即使依赖旧类型导致编译报错） |
| `Src/main.rs` | 暂不修改 |

---

## 4. 实施计划（本次）

### Phase 1：删除 Tensor_IO + 新建 Tensor_Stream

| # | 文件 | 操作 | 说明 |
|---|------|------|------|
| 1.1 | `Src/Tensor_IO/` | 🗑️ 删除 | 整个目录 |
| 1.2 | `Src/Network/Tensor_Stream/mod.rs` | 🆕 新建 | `pub mod protocol; pub mod rendezvous;` |
| 1.3 | `Src/Network/Tensor_Stream/protocol.rs` | 📦 迁入 | `tensor_stream_protocol.rs` 内容整体复制，无修改 |
| 1.4 | `Src/Network/Tensor_Stream/rendezvous.rs` | 🆕 新建 | `RendezvousMap`：`new()` / `insert_inbound(id, stream)` / `register_accept(id)` → `oneshot::Receiver` |
| 1.5 | `Src/Network/tensor_stream_protocol.rs` | 🗑️ 删除 | 已迁入 `Tensor_Stream/protocol.rs` |
| 1.6 | `Src/Network/mod.rs` | ✏️ | `mod tensor_stream_protocol` → `mod tensor_stream`；更新所有重导出路径 |
| 1.7 | `Src/Network/capability.rs` | ✏️ | import 路径更新；`open_tensor_stream` 签名加 `inference_id` 参数 + 写 handshake；新增 `accept_tensor_stream(id, timeout)`；`Network_Service_Capability` 加 `tensor_rendezvous` 字段 |
| 1.8 | `Src/Network/network_service.rs` | ✏️ | 添加 `rendezvous: Arc<RendezvousMap>`；入站 tensor stream 改为读 handshake → rendezvous 匹配；import 路径更新 |
| 1.9 | `Src/lib.rs` | ✏️ | 移除 `pub mod tensor_io` 及旧导出；`pub use network::tensor_stream_protocol` → `pub use network::tensor_stream` |

**验证**：Phase 1 代码变更务必编译通过

### Phase 2：编写正式设计文档

| # | 文件 | 操作 | 说明 |
|---|------|------|------|
| 2.1 | `design_doc/tensor_stream_design.md` | 🆕 新建 | 参考 `storage_design.md` 格式，将本文档第 2 节设计内容整理为正式设计文档（模块概述、数据结构、Trait、协议格式、Lua API、协作关系、已知风险、TODO）

---

## 5. 讨论项

| # | 问题 | 结论 |
|---|------|------|
| 1 | `inference_id` 由谁生成？ | ✅ Core 生成，作为参数传入 ML Thread。推理会话唯一标识，用户可见 |
| 2 | `accept_tensor_stream` 超时行为？ | ✅ 接受 `timeout_sec` 参数，超时返 nil。超时后行为由用户 Lua 脚本自行决定 |
| 3 | 通知对端启动机制？ | ✅ 通过 `caps:send_command` 发送网络消息通知 Relay 的 Core 启动 ML Thread |
| 4 | `receive_tensor` / `send_tensor` 数据格式？ | ⚠️ 帧格式占位。当前单 offset 不兼容多 batch，后续需支持 `[num_batches][offsets...][total_len][data]`。Lua API 签名暂定 `send(stream, frame_data)` / `receive(stream) → frame_data`，具体内容待帧格式确定后再细化 |
| 5 | 是否需要 `send_eof`？ | ✅ 保留，`net.close_tensor_stream(stream)` 先发 EOF 再关 |

### 已知风险（同步至 `design_doc/potential_risk.md`）

| 风险 | 说明 |
|------|------|
| 单 inference_id 多流 | 当前设计一 inference_id 对应一 inbound + 一 outbound。若未来需冗余备份/多路传输，rendezvous 需支持一个 id 对应多条流 |
