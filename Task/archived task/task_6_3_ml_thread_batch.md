# Task 6.3: ML Thread Batch 接口设计

> Presented by KeJi
> Date: 2026-05-22

---

## 目标

定义 Session Manager 与 ML Thread 之间的接口协议。ML Thread 只负责 batch forward，sample 留在 Session Manager 侧，各 Slot 独立控制 temperature 等参数。

本任务只定接口，不实现 ML Thread 侧改动。

---

## 职责边界

```
Session Manager                    ML Thread
─────────────────                 ──────────
• tokenize / decode               • forward([B, seqlen]) → logits
• 拼 batch tensor + mask          • 维护 KV cache
• sample(logits) per slot         （纯计算，不碰 tokenizer）
• 分发结果
```

---

## 接口

### 通道

```rust
// Session Manager → ML Thread: batch tensor
let (batch_tx, mut batch_rx) = mpsc::channel::<BatchRequest>(1);

// ML Thread → Session Manager: logits
let (logits_tx, mut logits_rx) = mpsc::channel::<BatchResult>(1);
```

### BatchRequest

```rust
struct BatchRequest {
    tensor: Tensor,              // [B, max_seqlen]，已 padding + attention mask 嵌入
    slot_order: Vec<(SessionId, SlotId)>,  // batch index → slot 映射
}
```

### BatchResult

```rust
struct BatchResult {
    logits: Tensor,              // [B, vocab]
    slot_order: Vec<(SessionId, SlotId)>,  // 原样返回
}
```

---

## 主循环分支

### 分支 C — Flush (Session Manager)

```
flush_timer.tick()
    │
    ▼
  收集所有 dirty Slot 的 token_buf
  拼 [B, max_seqlen] tensor + attention_mask
  batch_tx.send(BatchRequest { tensor, slot_order })
  清空 token_buf, dirty = false
```

### 分支 D — 收 Logits + Sample (Session Manager)

```
Some(BatchResult { logits, slot_order }) = logits_rx.recv()
    │
    ▼
  for each batch_idx:
    logits_row = logits.get(batch_idx)              // [vocab]
    (sess_id, slot_id) = slot_order[batch_idx]
    slot = sessions[sess_id].slots[slot_id]
    token = sample(logits_row, slot.temperature)    // → u32
    text = tokenizer.decode(token)
    slot.token_tx.send(text)
    slot.token_buf.push(token)                      // 追加到历史
```

### ML Thread 内部（伪代码，本任务不实现）

```
loop {
    BatchRequest { tensor, slot_order } = batch_rx.recv()
    logits = model.forward(&tensor)    // [B, vocab]
    logits_tx.send(BatchResult { logits, slot_order })
}
```

---

## Sample 策略（Session Manager）

每个 Slot 独立参数：

```rust
struct Slot {
    // ...
    temperature: f64,      // 默认 0.8
    top_p: Option<f64>,    // 暂不实现
    top_k: Option<usize>,  // 暂不实现
}
```

sample 直接复现当前 `MlSession::sample` 逻辑，从 `[vocab]` logits row 做 temperature scaling + categorical sampling。

---

## 对比当前架构

| 项目 | 当前 MlSession | 新设计 |
|------|---------------|--------|
| forward | `forward(tensor, offset)` 单序列 | forward `[B, seqlen]` batch |
| sample | ML Thread 内 sample → 返回 1 token | Session Manager sample → 返回 B tokens |
| tokenize | ML Thread 持 tokenizer | Session Manager 持 tokenizer |
| KV cache | offset 手动管理 | ConcatKvCache batch dim 自动隔离 |

---

## 不实现的内容

- ML Thread 侧 batch forward 实现（后续子任务）
- 增量 KV cache（第一版每轮完整 prefill）
- 混合 prefill/decode batch（第一版统一 prefill）

---

## 人类评审

<!-- 在此区域写下评审意见 -->

