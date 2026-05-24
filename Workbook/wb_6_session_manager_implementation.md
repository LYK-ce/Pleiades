# wb_6

> start: 2026-05-22
> end: 2026-05-24
> branch: session-manager-reforge

## State
Task 6 v2 Phase 1-4 完成。104 tests pass。端到端验证通过（session create + chat → EventBus 打印）。

---

## v2 设计总结

### 架构变更
v1 (集中式) → v2 (分布式):
- 删除 SessionManager::run() 全局 loop + 6 组 mpsc channel
- 删除 SessionManagerHandle、BatchRequest/BatchResult、assemble_batch/sample_batch
- 删除 Session Stream 协议（/pleiades/session/1.0.0）及其全部代码
- SessionManager 退化为 `Arc<Mutex<HashMap<u64, Session>>>` + 简单方法
- 每 Session 独立 spawn task，通过 LocalStreamHub 等待连接

### 连接方案
- **本地**: TUI → LocalStreamHub (accept_async + notifier) → Session
- **远端**: 待后续 Task（用 RendezvousMap register_notify + Tensor Stream）
- libp2p 拒绝自环（相同 PeerId），因此本地不走 Tensor Stream

### LocalStreamHub 增强
- 新增 `accept_async()` — 用 oneshot + tokio::time::timeout，不阻塞 tokio
- 新增 `notifiers` 字段 — open 时优先通知异步等的人
- 旧 `accept()` 保持同步兼容

### 已验证链路
```
session create qwen3 → Session 1 spawned → accept_async waiting
chat 1 你好 → hub.open → notifier paired → send_frame → 
  Session recv_frame → EventBus "Session 1: hello"
```

---

## v2 实施记录

| 阶段 | 内容 | commit |
|------|------|--------|
| 1 | 删旧代码: Session_Stream, batch.rs, run(), mpsc, SessionManagerHandle | 9073ba1 |
| 2 | SessionManager Arc<Mutex<>> | 9073ba1 |
| 3 | RendezvousMap register_notify | 6129025 |
| 4 | Session.spawn() + TUI 命令 session/chat | f2a35f9, 3ea8272, 46f4cb2 |
| fix | chat 从 Tensor Stream 改 LocalStreamHub + accept_async | 46f4cb2 |

---

## v1 遗留（已弃用）
- GAP 1: ACK handshake — Session Stream 协议已删除，无此 gap
- GAP 6: SlotHandle Drop — SlotHandle 已大幅简化，v2 不自动释放
- GAP 16: Bridge unidirectional — Session Stream 已删除
- GAP 17: ACK on slot — 同上

## v2 待做
- Session↔ML Thread 连接（local_tensor_stream rendezvous）
- 远端接入（RendezvousMap register_notify + Tensor Stream）
- Tokenizer 接入
- EOS 检测
- 多 Session 支持验证

---

## 架构笔记
- SessionManager: Arc<Mutex<>> — 共享同一个 LocalStreamHub
- Session.spawn(): tokio::spawn → accept_async → recv_frame → EventBus
- LocalStreamHub: open() 优先 notifier → 否则 pending
- accept_async(): 先查 pending → 注册 oneshot → tokio::time::timeout
- accept(): 保持同步，用于 Lua 绑定等旧代码
- RendezvousMap.register_notify: 保留，远端接入时使用
