# Task 6 v2.7: Slot 化 — mpsc 通道对替代 local_tensor_stream

> Presented by KeJi
> Date: 2026-05-25

---

## 目标

Chat ↔ Session 的连接从 local_tensor_stream（DuplexStream + rendezvous）改为基于 slot 的 mpsc 通道对。本地 Chat 走纯 mpsc，远端 Chat 将来通过 relay 桥接 Tensor_Stream ↔ mpsc，Session 内部只看 mpsc。

---

## 设计（详见 v2 总文档演进设计节）

```
allocate_slot(session_id) → SlotHandle {
    prompt_tx: Sender<String>,   // Chat → Session
    token_rx:  Receiver<String>, // Session → Chat
}

Session.slots[slot_id] = Slot {
    prompt_rx: Receiver<String>,
    token_tx:  Sender<String>,
    context_len: usize,
}
```

Session.spawn() 通过 select! 监听所有 slot 的 `prompt_rx`，推理结果通过 `slot.token_tx.send(text)` 返回。

---

## 实施计划

### 改动的文件

| 文件 | 改动 |
|------|------|
| `Src/Session_Manager/slot.rs` | Slot 结构加 `prompt_rx`、`token_tx`、`context_len` |
| `Src/Session_Manager/capability.rs` | SlotHandle 改为 `(prompt_tx, token_rx)` 通道对，去掉 token_rx Option |
| `Src/Session_Manager/manager.rs` | allocate_slot 创建 mpsc 通道对，传入 Session.slots |
| `Src/Session_Manager/session.rs` | ① Session.spawn() 去掉 `accept_async("session-{id}")` 和 chat_stream；② select! 改为监听 slots[*].prompt_rx；③ token 输出走 `slots[*].token_tx.send()` 而非 EventBus::Stream |
| `Src/Orchestrator/core/branch_user.rs` | chat handler：调 allocate_slot 拿 SlotHandle → subscribe prompt → prompt_tx.send()；新增 relay task 收 token_rx → EventBus::Stream |
| `Src/TUI/command.rs` 或相关 | prompt 需带 slot_id（改用方案 B：HashMap 直接 send），或暂时保持 broadcast 但消息带 slot_id |

### 阶段 1: 改造 Slot / SlotHandle 数据结构

| 步骤 | 文件 |
|------|------|
| Slot 加字段：`prompt_rx: UnboundedReceiver<String>`, `token_tx: UnboundedSender<String>`, `context_len: usize` | `slot.rs` |
| SlotState::Occupied 初始化时持有新字段 | `slot.rs` |
| SlotHandle 改为 `{ session_id, slot_id, prompt_tx, token_rx }`，去掉 Option | `capability.rs` |
| allocate_slot：创建 `unbounded_channel()` → 分别存入 Slot 和 SlotHandle | `manager.rs` |

### 阶段 2: Session.spawn() 去掉 chat_stream

| 步骤 | 文件 |
|------|------|
| spawn() 签名去掉 `stream_hub` 依赖（不再 accept chat） | `session.rs` |
| 删除 `accept_async("session-{id}")` 和 chat_stream 变量 | `session.rs` |
| spawn() 需要拿 `Arc<Mutex<Vec<SlotState>>>` 或等效方式访问 slots | `session.rs` |
| select! 从 `chat_stream.recv()` 改为轮询 slots 的 prompt_rx | `session.rs` |
| token 输出从 `EventBus::Stream` 改为 `slots[slot_id].token_tx.send()` | `session.rs` |
| context_len 从 spawn 局部变量下沉到 Slot 字段 | `session.rs` |

### 阶段 3: Chat 链路适配

| 步骤 | 文件 |
|------|------|
| chat handler：`allocate_slot(session_id)` → 拿到 SlotHandle → spawn relay task | `branch_user.rs` |
| relay task：`prompt_rx.subscribe()` → prompt 文本 → `handle.prompt_tx.send()` | `branch_user.rs` |
| relay task：loop `handle.token_rx.recv()` → EventBus::Stream + Output 起止标记 | `branch_user.rs` |
| prompt 广播 payload 带 session_id + slot_id（或后续改为 HashMap 直发） | `branch_user.rs` |

### 阶段 4: 清理

| 步骤 | 文件 |
|------|------|
| 删除 Session 中 chat_stream 相关字段 | `session.rs` |
| SessionManager::create_session 不再传 stream_hub | `manager.rs` |
| LocalStreamHub 中 "session-{id}" 的 notifier 注册逻辑可删除（不再使用） | 相关 |

### 不在此范围

- 远端 Chat 的 relay 桥接（Tensor_Stream → mpsc）
- 多 slot 并发 select!（当前只用一个 slot）
- 多 slot 的 KV Cache 管理

---

## 人类评审

<!-- 在此区域写下评审意见 -->

