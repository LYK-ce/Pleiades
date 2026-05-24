# Task 6 v2: Session 核心重构

> Presented by KeJi
> Date: 2026-05-24

---

## 设计变更

经过讨论，Session Manager 设计从「集中 event loop + mpsc 通道体系」改为「每 Session 独立 task + 直接方法调用」：

```
v1 (当前):                            v2 (新设计):
───────                              ────────
SessionManager::run()                Session.spawn() × N
  ├─ 5 分支 tokio::select!             └─ 2 分支 tokio::select!
  ├─ 6 组 mpsc channel                  （stream read + ml recv）
  ├─ SessionManagerHandle
  └─ 全局 batch 拼装/分发

外部通过 mpsc 通信                    外部通过 Arc<Mutex<>> 直接调方法
Session 间共享 batch KV cache        每 Session 独立 ML Thread
```

### 外对内的连接统一

**删除 Session Stream 协议。** 复用现有 Tensor Stream 作为统一的外部接入协议。

```
外部 (TUI / 远端) ── Tensor Stream ──► Session
  第一帧: [8B offset=0][session_id bytes]   ← 握手
  上行:   [8B offset=N][prompt UTF-8]
  下行:   [8B offset=N][token UTF-8]
```

libp2p 自环实现本地接入，远端走 TCP。两层同一条协议。

### Session 与 ML Thread 的连接

复用 `local_tensor_stream`（内存 DuplexStream + rendezvous）：

```
Session.open("ml-15")  ←──配对──  ML Thread.accept("ml-15", timeout)
```

Session 侧做 tokenize/tensorize/sample/decode，ML Thread 只做 forward。

---

## 新架构图

```
                    Tensor Stream (复用 /pleiades/tensor/1.0.0)
  ┌─────────────────────────────────────────────────────────────┐
  │  外部 (TUI / 远端)                                          │
  │                                                             │
  │  第一帧: session_id                                         │
  │  后续: prompt 文本字节                                       │
  │  返回: token 文本字节                                        │
  └──────────────────────┬──────────────────────────────────────┘
                         │
                         ▼
  ┌─────────────────────────────────────────────────────────────┐
  │  Session.spawn() — tokio task                               │
  │                                                             │
  │  loop {                                                     │
  │    tokio::select! {                                         │
  │      stream.read()  → prompt → tokenize → tensorize        │
  │                        → local_send_frame(ml_stream, tensor) │
  │                                                                 │
  │      ml_stream recv  → logits → deserialize → sample → decode │
  │                        → stream.write(token)                │
  │    }                                                        │
  │  }                                                          │
  └──────────────────────┬──────────────────────────────────────┘
                         │
                         │  local_tensor_stream (内存 duplex)
                         ▼
  ┌─────────────────────────────────────────────────────────────┐
  │  ML Thread (tokio::spawn)                                   │
  │                                                             │
  │  loop {                                                     │
  │    (tensor, offset) = recv_frame(ml_stream)                 │
  │    logits = model.forward(&tensor)                          │
  │    send_frame(ml_stream, logits, offset)                     │
  │  }                                                          │
  └─────────────────────────────────────────────────────────────┘
```

---

## SessionManager 退化为数据结构

```rust
// Arc<Mutex<SessionManager>> — 外部直接调用
pub struct SessionManager {
    sessions: HashMap<u64, Session>,
    slot_counter: u64,
    stream_hub: Arc<LocalStreamHub>,     // 用于 Session ↔ ML rendezvous
    max_slots: usize,
}

impl SessionManager {
    fn create_session(&mut self, model_id: &str) -> u64;
    fn create_ml_thread(&mut self, session_id: u64, model_path: &str);
    fn allocate_slot(&mut self, session_id: u64) -> Result<SlotHandle>;
    fn close_slot(&mut self, session_id: u64, slot_id: usize);
}
```

`SlotHandle` 退化为 `(DuplexStream, token_rx)` 或直接持有 stream。

---

## Session 结构

```rust
pub struct Session {
    session_id: u64,
    model_id: String,
    slots: Vec<SlotState>,
    ready: bool,              // ML Thread 是否已连接
    ml_stream: Option<DuplexStream>,  // 连 ML 的 local_tensor stream
}
```

---

## 实施计划

### 阶段 1: 删旧代码

| 操作 | 文件 |
|------|------|
| 删除 `SessionManager::run()` | `manager.rs` |
| 删除 `shared_prompt_(tx/rx)` | `manager.rs` |
| 删除 `open_slot_(tx/rx)` | `manager.rs` |
| 删除 `close_slot_(tx/rx)` | `manager.rs` |
| 删除 `SessionManagerHandle` | `manager.rs` |
| 删除 `OpenSlotRequest` 等消息结构 | `manager.rs` |
| 删除 `BatchRequest / BatchResult` | `batch.rs` |
| 删除 `assemble_batch / sample_batch` | `batch.rs` |
| 删除 `Network/Session_Stream/` | 整个目录 |
| 删除 `Network_Inbound_Event::SessionStreamArrived` | `capability.rs` |
| 删除 `open_session_stream` trait 方法 + impl | `capability.rs` |
| 删除 `branch_stream.rs` 中 Session 路由 | `branch_stream.rs` |
| 删除 `network_service.rs` 中 session accept/select! | `network_service.rs` |
| 删除 `Network_Capability` 中 `open_session_stream` | `capability.rs` |
| 更新 `main.rs` 移除相关初始化 | `main.rs` |
| 删除 `Capabilities.session_manager` 字段 | `orchestrator/mod.rs` |

### 阶段 2: 建新 Session

| 操作 | 文件 |
|------|------|
| 重写 `Session` struct（加 ml_stream, ready） | `session.rs` |
| 新增 `Session::spawn(stream, ml_stream)` — 启动 select! task | `session.rs` |
| 新增 `SessionManager::create_session` — 创建 + spawn | `manager.rs` |
| 新增 `SessionManager::create_ml_thread` — 加载模型 + connect local_tensor | `manager.rs` |
| `SessionManager` 加 `Arc<Mutex<>>` 包裹 | `manager.rs` |
| `allocate_slot` 改为直接创建 DuplexStream + 返回 | `manager.rs` |

### 阶段 3: Tensor Stream 适配

| 操作 | 文件 |
|------|------|
| `RendezvousMap` 新增 `register_notify(id, tx)` — 通知模式 | `Src/Network/Tensor_Stream/rendezvous.rs` |
| `RendezvousMap::insert_inbound` 加优先级分发：notifier → oneshot → pending | 同上 |
| Tensor Stream 到达后，读第一帧 (offset=0) 判断是 session_id 还是推理 tensor | 分析现有流程 |
| 新增自环 dial 支持（libp2p loopback） | 后续 task |

**RendezvousMap 改动详情：**

当前 `RendezvousMap` 只支持「流先到等人取」(`pending_inbound`) 和「人先到等流」(`pending_accept` + oneshot) 两种被动模式。新增第三种：**通知模式**。

```rust
// 新增字段
notifiers: HashMap<u64, mpsc::UnboundedSender<libp2p::Stream>>,

// Session 侧注册通知
pub fn register_notify(&self, id: u64, tx: mpsc::UnboundedSender<libp2p::Stream>);

// insert_inbound 改为三级优先级分发
pub fn insert_inbound(&self, id: u64, stream: libp2p::Stream) {
    // 1. 优先: 通知模式 — Session.select! 等着
    if let Some(tx) = self.notifiers.remove(&id) { tx.send(stream); return; }
    // 2. 其次: oneshot 模式 — Pipeline accept 阻塞等
    if let Some(tx) = self.pending_accept.remove(&id) { tx.send(stream); return; }
    // 3. 兜底: 存货架 — 等后续 accept 来取
    self.pending_inbound.insert(id, stream);
}
```

三种模式共存，Pipeline 不受影响。

### 阶段 4: 测试

| 操作 |
|------|
| 单 Session 创建 → open_slot → prompt → token 返回 |
| 多 Session 并发 |

---

## 不在此阶段

- ML Thread 实现（后续 Task）
- libp2p loopback 实现（后续 Task）
- 跨 Session batch KV cache（v0 不做）
- 远端接入验证（后续 Task）

---

## 人类评审

<!-- 在此区域写下评审意见 -->

