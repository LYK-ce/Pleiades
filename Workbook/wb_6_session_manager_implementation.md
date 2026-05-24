# wb_6

> start: 2026-05-22
> end: 2026-05-24
> branch: session-manager-reforge

## State
ALL DONE. Tasks 6.1~6.4 + 6.1.i complete. 125 tests pass.

## Task 6.1.i: Session Stream ACK 补全 + 死代码清理 (2026-05-24)

### 问题
Session Stream 协议定义了 ACK 握手但未使用。
远端不知道 slot 分配成功还是失败。
TensorStreamArrived 变体从未被构造，纯死代码。

### 修复
- `open_session_stream`: handshake 后读 ACK，REJECT→Err
- `branch_stream.rs`: slot 分配后写 ACCEPT/REJECT
- 删除 `TensorStreamArrived` 变体（3 处：enum/macth/test）

### 讨论：Network Stream 架构
讨论了是否要统一四种流的处理方式。
结论：不改。File 和 Session 走 Core（需要业务能力），
Bandwidth 和 Tensor 在 Network 层自闭环（纯网络机制）。
只有 2 种走 Core，不值得抽 StreamHandler trait。

## Gaps resolved

### Fixed (post-review)
- GAP 1: ACK handshake → FIXED: open_session_stream 读 ACK, Core 写 ACK
- GAP 6: SlotHandle Drop → FIXED: auto-send CloseSlotRequest on drop
- GAP 16: Bridge unidirectional → FIXED: bidirectional select! (入站 submit + 出站 recv_token)
- GAP 17: No ACK on slot allocation → FIXED: same as GAP 1

### Still open
- GAP 3: No Tokenizer → placeholder byte-to-u32
- GAP 9: eos_token_id hardcoded → placeholder
- GAP 10/11: Vec<Vec<>> not candle Tensor
- GAP 12: No attention mask
- GAP 14: fast_random bad RNG
- GAP 15: flush_interval hardcoded

## Architecture notes
- SessionManager.run() owns &mut self → single-threaded, no locks
- SessionManagerHandle holds cloned sender halves → safe for Arc sharing
- Network sessions: SessionStreamArrived → Core → open_slot_tx → Main loop Branch A
- Local sessions: any component → open_slot_tx → Main loop Branch A
- ACK flow: open→handshake→ACK(ACCEPT)→stream ready / ACK(REJECT)→error
- 4 network streams, 2 go to Core (File+Session), 2 stay in Network (Bandwidth+Tensor)
- 6 tokio channels: 3 unbounded (prompt/open/close), 2 bounded(1) (batch/logits for ML Thread)
