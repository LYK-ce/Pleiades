# Session Manager 全链路 Code Review

> Presented by KeJi
> Date: 2026-05-22

---

本文档从 Session Manager 创建开始，到远端建立连接、提交 prompt、进入 ML Thread、返回 token 分发，**逐步追踪每条代码路径**。

---

## 目录

1. [启动：Session Manager 创建与 spawn](#1-启动session-manager-创建与-spawn)
2. [本地接入：open_slot + 发 prompt](#2-本地接入open_slot--发-prompt)
3. [远端接入：SessionStreamArrived → 桥接](#3-远端接入sessionstreamarrived--桥接)
4. [主循环：收 Prompt → tokenize → 攒 batch](#4-主循环收-prompt--tokenize--攒-batch)
5. [Flush：拼 batch 送 ML Thread](#5-flush拼-batch-送-ml-thread)
6. [ML Thread 接口：BatchRequest → forward → BatchResult](#6-ml-thread-接口batchrequest--forward--batchresult)
7. [分发：sample → decode → 写回流](#7-分发sample--decode--写回流)
8. [槽位释放：SlotHandle Drop](#8-槽位释放slothandle-drop)
9. [现存 Gap 清单](#9-现存-gap-清单)

---

## 1. 启动：Session Manager 创建与 spawn

**文件**: `Src/main.rs` Phase 3

```rust
// Phase 3: 创建基础组件
// 8. SessionManager
let max_slots = config.Session.as_ref()
    .and_then(|s| s.max_slots)
    .unwrap_or(4);
let session_mgr = SessionManager::new(max_slots as usize);
let session_handle = Arc::new(session_mgr.handle());
```

`SessionManager::new()` 创建了 **6 组通道**：

| 通道 | 方向 | 用途 |
|------|------|------|
| `shared_prompt_(tx/rx)` | unbounded | 所有 SlotHandle 发 prompt 到主循环 |
| `open_slot_(tx/rx)` | unbounded | 外部请求分配 slot（带 oneshot 回复） |
| `close_slot_(tx/rx)` | unbounded | SlotHandle Drop 时自动释放 |
| `batch_(tx/rx)` | bounded(1) | Session Manager → ML Thread |
| `logits_(tx/rx)` | bounded(1) | ML Thread → Session Manager |

`session_handle` 只持有 `open_slot_tx` 和 `logits_tx`，被放入 `Capabilities`。

**文件**: `Src/main.rs` Phase 6

```rust
// 14. 启动 SessionManager 主循环
tokio::spawn(async move {
    session_mgr.run().await;   // 消费 SessionManager，进入 select! loop
});

// 15. 启动 Network
tokio::spawn(async move { network_service.Start().await });

// 17. Core 主循环（阻塞）
core.run().await;
```

**关键点**: `run()` 接收 `mut self`，SessionManager 被移动到 tokio task 中，不再被外部持有。外部只能通过 `SessionManagerHandle` 的通道发送端与它通信。

---

## 2. 本地接入：open_slot + 发 prompt

**调用方**（TUI / Lua / 本地代码）：

```rust
// 1. 首先需要有一个 Session
session_mgr.create_session("qwen3");  // → "sess-1"

// 2. 申请一个 slot
let (reply_tx, reply_rx) = oneshot::channel();
handle.open_slot_tx.send(OpenSlotRequest {
    session_id: "sess-1".into(),
    reply_tx,
});
let slot_handle: SlotHandle = reply_rx.await.unwrap().unwrap();
```

**Session Manager 主循环分支 A — open_slot**:

```rust
// 文件: manager.rs run() → Branch A
Some(req) = self.open_slot_rx.recv() => {
    let result = self.allocate_slot(&req.session_id);  // 遍历找 Vacant → 分配
    req.reply_tx.send(result).ok();                    // 通过 oneshot 返回 SlotHandle
}
```

`allocate_slot()` 内部：

```rust
// 文件: manager.rs
pub fn allocate_slot(&mut self, session_id: &str) -> Result<SlotHandle, Session_Error> {
    let session = self.sessions.get_mut(session_id)
        .ok_or_else(|| Session_Error::SessionNotFound(...))?;

    let (token_tx, token_rx) = mpsc::unbounded_channel();  // 返回 token 的通道

    let slot_id = session.allocate(token_tx)
        .ok_or_else(|| Session_Error::SlotExhausted(...))?;

    // SlotHandle 持有 3 个通道端：
    Ok(SlotHandle::new(
        session_id.to_string(),
        slot_id,
        self.shared_prompt_tx.clone(),   // → 发 prompt 进主循环
        self.close_slot_tx.clone(),       // → drop 时自动释放
        token_rx,                         // → 收 token
    ))
}
```

**发 prompt**:

```rust
slot_handle.submit("你好".to_string());
// 内部: shared_prompt_tx.send(("sess-1", 0, "你好"))
// → 主循环 Branch B 收到
```

---

## 3. 远端接入：SessionStreamArrived → 桥接

**远端 Peer A** 发起连接：

```rust
// 远端代码 (伪代码)
let stream = network.open_session_stream(peer_B, "sess-7").await;
// 内部：libp2p 协商 "/pleiades/session/1.0.0"
//       → 写 handshake: [1B len][sess-7 UTF-8]
//       → 返回 stream
stream.write("你好\n".as_bytes());   // 发一整句
let mut buf = [0u8; 64];
stream.read(&mut buf);               // 读 token
```

**本端 Peer B** 接收：

```rust
// 文件: network_service.rs select! 分支
Some((peer_id, mut stream)) = incoming_session_streams.next() => {
    let session_id = Read_Session_Stream_Handshake(&mut stream).await;
    // 发送 Network_Inbound_Event::SessionStreamArrived { peer, stream, session_id }
}
```

**Core 路由**:

```rust
// 文件: branch_stream.rs
Network_Inbound_Event::SessionStreamArrived { peer, stream, session_id } => {
    // 1. 构造 OpenSlotRequest 发到 Session Manager
    let (reply_tx, reply_rx) = oneshot::channel();
    caps.session_manager.open_slot_tx.send(
        OpenSlotRequest { session_id, reply_tx }
    );

    // 2. 等待分配结果
    match reply_rx.await {
        Ok(Ok(slot_handle)) => {
            spawn_session_bridge(stream, slot_handle);  // 桥接
        }
        Ok(Err(e)) => { /* slot 满或 session 不存在 */ }
    }
}
```

**桥接协程**（完整的双向 select!）:

```rust
// 文件: branch_stream.rs
fn spawn_session_bridge(stream: libp2p::Stream, mut handle: SlotHandle) {
    let stream = Arc::new(Mutex::new(stream));

    tokio::spawn(async move {
        loop {
            tokio::select! {
                // 入站：远端发来一整句 prompt
                read_result = { s.read(&mut buf) } => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    handle.submit(text);            // → Branch B
                }
                // 出站：逐 token 流式写回远端
                Some(token) = handle.recv_token() => {
                    s.write_all(token.as_bytes());  // 写回 stream
                }
            }
        }
    });
}
```

**数据流路径**:

```
远端 "你好"  ──stream──► 桥接 read ──submit──► shared_prompt_tx
                                                    │
                                                    ▼
                                            Session Manager Branch B
                                                    │
                                              handle_prompt → token_buf
```

---

## 4. 主循环：收 Prompt → tokenize → 攒 batch

**文件**: `manager.rs` Branch B

```rust
Some((session_id, slot_id, text)) = self.shared_prompt_rx.recv() => {
    self.handle_prompt(&session_id, slot_id, &text);
}
```

`handle_prompt()`:

```rust
pub fn handle_prompt(&mut self, session_id: &str, slot_id: usize, text: &str) {
    let session = self.sessions.get_mut(session_id)?;
    let slot = session.get_slot_mut(slot_id)?;

    // 占位 tokenize：字节直接当 token ID（真实 Tokenizer 待接入）
    let tokens: Vec<u32> = text.bytes().map(|b| b as u32).collect();
    slot.token_buf.extend(tokens);
    slot.dirty = true;  // 标记：下一轮 flush 时处理
}
```

**关键设计**: `handle_prompt` 只 tokenize + 存，**不发 ML Thread**。发 ML Thread 是由定时器驱动的 Branch C 统一做的。

---

## 5. Flush：拼 batch 送 ML Thread

**文件**: `manager.rs` Branch C（100ms 定时器）

```rust
_ = flush_timer.tick() => {
    self.flush_and_dispatch().await;
}
```

`flush_and_dispatch()`:

```rust
async fn flush_and_dispatch(&mut self) {
    // 1. 遍历所有 Session，收集 dirty slot
    let mut dirty: Vec<(String, usize, Vec<u32>)> = Vec::new();
    for (sess_id, session) in self.sessions.iter() {
        for (slot_id, state) in session.slots.iter().enumerate() {
            if let SlotState::Occupied(ref slot) = state {
                if slot.dirty && !slot.token_buf.is_empty() {
                    dirty.push((sess_id.clone(), slot_id, slot.token_buf.clone()));
                }
            }
        }
    }

    // 2. 拼 batch（padding + slot_order 映射）
    if let Some(batch) = assemble_batch(&dirty) {
        // 3. 发送到 ML Thread
        self.batch_tx.send(batch).await.ok();
        // 4. 清空已发送的 token_buf
        for (sess_id, slot_id, _) in &dirty {
            session.get_slot_mut(*slot_id).token_buf.clear();
            slot.dirty = false;
        }
    }
}
```

`assemble_batch()` — 文件 `batch.rs`:

```rust
pub fn assemble_batch(dirty_slots: &[(String, usize, Vec<u32>)]) -> Option<BatchRequest> {
    let max_len = dirty_slots.iter().map(|(_, _, buf)| buf.len()).max()?;

    let mut token_batches = Vec::new();
    let mut slot_order = Vec::new();
    for (sess_id, slot_id, buf) in dirty_slots {
        let mut padded = buf.clone();
        padded.resize(max_len, 0);  // 0-padding
        token_batches.push(padded);
        slot_order.push((sess_id.clone(), *slot_id));
    }

    Some(BatchRequest { token_batches, slot_order })
}
```

**Batch 示例**:

```
输入:
  slot_0 (sess-1): [101, 204]              len=2
  slot_1 (sess-1): [10, 20, 30, 40]        len=4
  slot_2 (sess-2): [100]                    len=1

输出:
  token_batches: [[101,204,0,0], [10,20,30,40], [100,0,0,0]]
  slot_order:    [("sess-1",0), ("sess-1",1), ("sess-2",0)]
```

---

## 6. ML Thread 接口：BatchRequest → forward → BatchResult

**当前状态**: ML Thread 未实现，接口已定义。

```
Session Manager                              ML Thread (待实现)
─────────────                               ──────────────────
batch_tx.send(BatchRequest) ──────────────► batch_rx.recv()
                                                │
                                          model.forward([B, seqlen])
                                                │
logits_rx.recv() ◄─────────────────────── logits_tx.send(BatchResult)
    │
    ▼
dispatch_logits()
```

**BatchRequest** (Session Manager → ML Thread):
```rust
pub struct BatchRequest {
    pub token_batches: Vec<Vec<u32>>,       // [B, max_seqlen] 已 padding
    pub slot_order: Vec<(String, usize)>,   // batch index → slot 映射
}
```

**BatchResult** (ML Thread → Session Manager):
```rust
pub struct BatchResult {
    pub logits_batches: Vec<Vec<f32>>,      // [B, vocab_size]
    pub slot_order: Vec<(String, usize)>,   // 原样返回
}
```

> **GAP**: 当前用 `Vec<Vec<u32/f32>>` 而非 candle `Tensor`。ML Thread 实现时需对齐。

---

## 7. 分发：sample → decode → 写回流

**文件**: `manager.rs` Branch D

```rust
Some(result) = self.logits_rx.recv() => {
    self.dispatch_logits(result).await;
}
```

`dispatch_logits()`:

```rust
async fn dispatch_logits(&mut self, result: BatchResult) {
    // 1. 收集各 slot 的 temperature
    let mut temperatures = HashMap::new();
    for (sess_id, slot_id) in &result.slot_order {
        let slot = self.sessions.get(sess_id)?.get_slot(*slot_id)?;
        temperatures.insert((sess_id.clone(), *slot_id), slot.temperature);
    }

    // 2. 逐 slot sample
    let tokens = sample_batch(&result, &temperatures);
    // → Vec<((sess_id, slot_id), token_id)>

    // 3. decode + 分发
    for ((sess_id, slot_id), token) in tokens {
        let text = format!("[{}]", token);   // 占位 decode
        let slot = self.sessions.get_mut(&sess_id)?.get_slot_mut(slot_id)?;
        slot.token_tx.send(text);            // → 走 mpsc 到 SlotHandle.token_rx
        slot.token_buf.push(token);          // → 追加到历史（下一轮 prefill）
    }
}
```

`sample_logits()` — 文件 `batch.rs`:

```
算法: temperature scaling → softmax → categorical sample

logits = [0.1, 0.2, 5.0, 0.0], temp = 0.01
  → scaled = [10, 20, 500, 0]
  → softmax → [~0, ~0, ~1.0, ~0]
  → sample → 几乎一定是 index 2
```

**完整分发链**:

```
Session Manager dispatch_logits
    │
    ├─ slot_0.token_tx.send("[2]") ──mpsc──► SlotHandle.token_rx
    │                                            │
    │                                    ┌───────┴───────┐
    │                                    │ 本地: 调用方直接读
    │                                    │ 远端: 桥接写回流
    │                                    └───────────────┘
    │
    ├─ slot_1.token_tx.send("[0]") ──mpsc──► SlotHandle.token_rx
    │
    └─ slot_2.token_tx.send("[1]") ──mpsc──► SlotHandle.token_rx
```

---

## 8. 槽位释放：SlotHandle Drop

**文件**: `capability.rs`

```rust
impl Drop for SlotHandle {
    fn drop(&mut self) {
        if let Some(tx) = &self.close_slot_tx {
            tx.send((self.session_id.clone(), self.slot_id)).ok();
        }
    }
}
```

drop 时自动往 `close_slot_rx` 发 `(session_id, slot_id)` → Branch F 收到 → 释放槽位。

**特例 — `take_token_rx()`**: 桥接需要取出 `token_rx`。此时 `close_slot_tx` 被 `take()` 清空，Drop 不再自动释放。槽位由桥接协程结束时手动管理。

---

## 9. 现存 Gap 清单

| # | 严重度 | 描述 |
|---|--------|------|
| 1 | Medium | ACK 握手定义了但流程未用 — 接收方分配 slot 后没写 ACCEPT/REJECT |
| 3 | Medium | `handle_prompt` tokenize 是占位（字节→u32），需接真实 Tokenizer |
| 9 | Medium | `eos_token_id` 硬编码为 1，需从 Tokenizer 获取 |
| 10 | Medium | `BatchRequest` 用 `Vec<Vec<u32>>` 非 candle Tensor |
| 11 | Medium | `BatchResult` 用 `Vec<Vec<f32>>` 非 candle Tensor |
| 12 | Medium | Batch 拼合无 attention mask，padding 0 会被模型当真 token |
| 14 | Low | `fast_random()` 用系统纳秒当随机源，应换 PRNG |
| 15 | Low | `flush_interval` 硬编码 100ms，未读 config |

---

## 附录：通道全景图

```
                          SessionManagerHandle (Arc, 在 Capabilities)
                          ┌────────────────────────┐
                          │ open_slot_tx           │
                          │ logits_tx              │
                          └───────┬────────────────┘
                                  │
    ┌─────────────────────────────┼─────────────────────────────┐
    │                      SessionManager::run()                │
    │                      (tokio::select! loop)                │
    │                                                           │
    │  open_slot_rx ←─── A: open_slot  ───→ reply_tx           │
    │  shared_prompt_rx ← B: prompt   ───→ handle_prompt       │
    │                    C: flush_timer ───→ flush_and_dispatch │
    │                      batch_tx ────────→ ML Thread         │
    │                      logits_rx ◄──────── ML Thread        │
    │                    D: logits_rx ───→ dispatch_logits   │
    │  close_slot_rx ←─── F: close     ───→ release slot        │
    │                                                           │
    └───────────────────────────────────────────────────────────┘
```

---

> **测试**: 119 lib + 6 integration = 125 passed, 0 failed
> **分支**: `session-manager-reforge`
