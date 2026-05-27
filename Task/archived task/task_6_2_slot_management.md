# Task 6.2: Slot 管理

> Presented by KeJi
> Date: 2026-05-22

---

## 目标

实现 Session/Slot 分配、录入、分发、销毁的完整生命周期。Slot 是对话级隔离单元，Session 内的多个 Slot 共享同一个模型权重和 KV cache batch。

---

## 数据结构

### SlotState

```rust
enum SlotState {
    Vacant,
    Occupied(Slot),
}

struct Slot {
    id: usize,
    token_buf: Vec<u32>,         // 累积的 token 历史（下一轮 flush 送入 ML）
    token_tx: UnboundedSender<String>,   // 返回 token 给接入方
    dirty: bool,                 // token_buf 有新内容，等待 flush
}
```

### Session

```rust
struct Session {
    session_id: String,
    model_id: String,
    max_slots: usize,
    slots: [SlotState; N],       // 固定大小数组
    tokenizer: Tokenizer,
    eos_token_id: u32,
}
```

### SlotHandle（返回给接入方）

```rust
struct SlotHandle {
    prompt_tx: UnboundedSender<(usize, String)>,  // 发 prompt 到共享通道
    token_rx: UnboundedReceiver<String>,           // 收 token
}
```

---

## 接口

### Session Manager 暴露

```rust
impl SessionManager {
    /// 创建 Session（加载 tokenizer，初始化槽位数组）
    async fn create_session(model_id: &str, model_path: &Path) -> Result<String>;
    
    /// 销毁 Session，回收全部 Slot
    async fn destroy_session(session_id: &str);
    
    /// 申请槽位 — 遍历找 Vacant → 分配 → 返回 SlotHandle
    /// 全满则返回 SlotExhausted
    fn open_slot(session_id: &str) -> Result<SlotHandle, SessionError>;
    
    /// 释放槽位 → Vacant
    fn close_slot(session_id: &str, slot_id: usize);
}
```

### SlotHandle 暴露

```rust
impl SlotHandle {
    /// 发送 prompt（自动带 slot_id，投递到共享 prompt_rx）
    fn submit(text: String);
    
    /// 读取 token（接入方直接 await）
    async fn recv_token() -> Option<String>;
}
```

---

## 主循环分支

### 分支 B — 收 Prompt

```
Some((slot_id, text)) = shared_prompt_rx.recv()
    │
    ▼
  session = lookup slot_id → which session
  tokens = session.tokenizer.encode(text)
  session.slots[slot_id].token_buf.extend(tokens)
  session.slots[slot_id].dirty = true
```

### 分支 C — Flush Batch

```
flush_timer.tick()
    │
    ▼
  收集所有 Session 下所有 dirty Slot 的 token_buf
  拼成 [B, max_seqlen] batch tensor + attention_mask
  记录 slot_order: [(session_id, slot_id), ...]  ← batch index 映射
  ml_input_tx.send((batch_tensor, slot_order))
  清空各 token_buf, dirty = false
```

### 分支 D — 分发 Token

```
Some((tokens: [B], slot_order)) = ml_output_rx.recv()
    │
    ▼
  for each batch_idx:
    token = tokens[batch_idx]
    (sess_id, slot_id) = slot_order[batch_idx]
    text = session.tokenizer.decode(token)
    session.slots[slot_id].token_tx.send(text)
    session.slots[slot_id].token_buf.push(token)   ← 追加到历史
```

### 分支 A — 本地 open_slot

```
Some((session_id, prompt_rx, token_tx)) = open_slot_rx.recv()
    │
    ▼
  allocate_slot(session_id)
    → 返回 SlotHandle(shared_prompt_tx.clone(), token_rx)
```

### 分支 E — 网络 open_slot

```
SessionStreamArrived { peer, stream, session_id }
    │
    ▼
  allocate_slot(session_id)
    → 成功 → spawn 桥接(stream, slot_handle)
    → 失败 → stream.write("FULL"), stream.close()
```

### 分支 F — 销毁

```
Some((session_id, slot_id)) = close_slot_rx.recv()
    │
    ▼
  slots[slot_id] = Vacant
  清理 token_buf，drop 通道
```

---

## 补充

- `SlotHandle` 实现 Drop：drop 时自动调用 `close_slot` 释放槽位
- `dirty` 标记优化：避免空 slot 被 flush 浪费计算

---

## 人类评审

<!-- 在此区域写下评审意见 -->

