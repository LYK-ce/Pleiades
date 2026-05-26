# wb_6

> start: 2026-05-22
> end: 2026-05-25
> branch: session-tokenizer

## State
Task 6 v2.5 完成并验证通过。多轮对话：Session 维持 token 上下文 + 增量 prefill + KV Cache 复用。

```
session create model.pgguf → chat 1 → session inference 1 model.pgguf
  → Prompt> 你好 → 回复
  → Prompt> 你刚才说了什么？ → 基于上文回复 ✅
```

---

## v2 子任务记录

| 子任务 | 内容 | commit |
|--------|------|--------|
| v2 | Session 核心重构 (Phase 1-4) | 9073ba1..46f4cb2 |
| v2.1 | MlContext 拆分 model/tokenizer + load_tokenizer() | 6f6b147 |
| v2.2 | Session 集成 tokenizer (Storage + load_tokenizer) | dd19536 |
| v2.3 | Chat 持久连接 + Session loop + Prompt broadcast 通道 | de8f524 |
| v2.4 | ML Thread 接入 + 自回归 loop (max 120, temp=0.0, EOS) | 26ef191..ceb5028 |
| fix | B3 route_stream spawn 化 | fc30142 |
| fix | Core 设计原则写入 instructions.md | fc30142 |
| fix | Prompt 框渲染修复 | 20e686c |
| fix | tensor_to_bytes 支持 U32 dtype | fcb6d0d |
| v2.5 | 多轮对话: context_len + 增量 prefill + KV Cache 复用 + 4096 截断 | 8565f44 |
| v2.6 | reply 流式输出: Stream token → Command Output 区 + Output 起止标记 | 97244f8 |
| v2.7 | Slot 化: mpsc 通道对替代 local_tensor_stream (chat ↔ Session) | c768b07 |
| v2.8 | 远端 Chat: Session 流协议 + remote chat + yamux 两 task 修复 | 5914ac1..c4a9172 |
| v2.9 | Chat Template: messages数组 + think过滤 + KV Cache清理 + encode_messages | 7b669a3..4c6caf2 |

## 架构笔记（更新）

- Session.spawn(): load_tokenizer → accept_async("session-{id}") → accept_async("ml-{id}") → select!
- Chat 持久: broadcast::channel(16) → prompt_tx.subscribe() → loop → local_send_frame
- ML Thread: builtin/inference.lua → storage_acquire_read → load_model → open_stream("ml-{id}") → forward loop
- 自回归: chat 分支内 prefill + for 0..300 { recv ML → sample(0.0) → decode → EOS check → send next }
- 多轮: context_len 追踪 token 总数 → prefill offset=context_len → 每轮结束 context_len=offset
- 截断: context_len + new_tokens > 4096 → reset context_len=0
- ML Thread 无需改动，inference.lua 的 forward loop 已支持 offset 增量
- tensor 序列化: 1B dtype + header + data, dtype 0=F32 1=U32
- ML Thread side: Lua 脚本，通过 spawn_lua_script 在独立线程运行
- reply 流式: Bus_Event::Stream { type: token } → TUI handle_stream → command_output.push_str
- 起止标记: Bus_Event::Output { completed: false/true } 清空/标记
- Slot 连接: allocate_slot → (prompt_tx, token_rx) mpsc 通道对 → oneshot 通知 spawn task
- Session.spawn(): 去 chat_stream accept → slot_ready_rx.await → select! prompt_rx.recv()
- Chat relay: broadcast prompt → prompt_tx.send(); token_rx.recv() → EventBus::Stream
- Session ↔ ML: local_tensor_stream 保持（传 tensor/offset 语义匹配）
- 远端 Chat: /pleiades/session/1.0.0 流协议 → handshake(session_id) → [4B len][UTF-8] 帧
- 入站: Network_Inbound_Event::SessionStreamArrived → Core B3 → allocate_slot → bridge
- 出站: remote chat <peer> <id> → open_session_stream → prompt 上行/token 下行 串行
- 多轮: 空帧哨兵 (len=0) 作轮次分隔 → bridge 转发 → 远端退出 recv loop 回到 prompt 等待
- 死锁修复: Arc<Mutex<Stream>> → 单 task 串行读写
- slot_notify: oneshot → mpsc unbounded (支持多 slot 动态注册)
- messages 数组: 每轮 encode 完整历史 → prefill offset=0, ML Thread offset=0 时 reset KV Cache
- strip_think: 去掉 assistant_reply 中的 <think>...</think> 块再存入 messages
- chat_template: GGUF metadata 已读取但 fallback 硬编码 Qwen3 格式（Jinja 解析待后续）

## 已知问题

- logits 传输: 每 token ~600KB，应 ML Thread 侧 sample → 只回传 token_id (4B)
- 无 stop string 检测: temp=0 时 EOS OK，非 greedy 需兜底
- KV Cache: 每轮全量 prefill 清空重建，未利用历史前缀复用（因 think 过滤后前缀可能变化）
- slot 机制未启用: chat 一对一 session，多 slot 未挂接
- 远端 ML Thread 未实现

## 待做

- ML Thread 侧 sample（消除 logits 传输开销）
- 多 slot 支持
- KV Cache 滑动窗口截断（替代当前 reset 策略）
- 远端接入


---

## Code Review — 2026-05-26

> 审查范围: 全部 Task 6 Session Manager 代码 (session.rs, manager.rs, slot.rs, capability.rs, branch_user.rs, branch_stream.rs, context.rs, inference.lua, session_stream/protocol.rs, local_tensor_stream/*)
> 评审者: ZGent

### 🔴 Critical

#### CR-1: 多 Slot 并发时 prompt_rx 被覆盖，旧 slot 永久孤儿化

> **[Deferred]** 推迟至 Continuous Batching 阶段统一解决。Slot 并发管理将在 Continuous Batching 重构时与 KV Cache 共享策略一并实现。
**文件**: `Src/Session_Manager/session.rs:112-117`
```rust
while let Ok((new_id, new_rx, new_tx)) = slot_notify_rx.try_recv() {
    slot_tokens.insert(new_id, new_tx);
    slot_id = new_id;      // ← 覆盖
    prompt_rx = new_rx;    // ← 旧 prompt_rx 被 drop，永远收不到 prompt
}
```
`session.spawn()` 的 select! 循环只监听最后分配的 slot 的 `prompt_rx`。若先后创建 2 个 slot，第一个 slot 的 `prompt_tx.send()` 成功但 Session 永远不会 `recv`。**这意味着多用户 Chat 同一 Session 不可用。**

**建议**: 将 per-slot 状态（prompt_rx, context）结构化，select! 应轮询所有活跃 slot 或使用 `tokio::select!` 多分支。

#### CR-2: Session spawn task 崩溃后 SessionManager 无感知

> **[Deferred]** 与 CR-1 一并推迟至 Continuous Batching 阶段。届时 Session 生命周期管理将整体重构。
**文件**: `Src/Session_Manager/session.rs:spawn()`, `Src/Session_Manager/manager.rs`
Session 的 spawn task 在 tokenizer 加载失败、ML 连接超时等情况下直接 `return`。SessionManager 中的 `HashMap<u64, Session>` 仍保留该条目，`list_sessions()` 返回正常，`allocate_slot` 成功但 `slot_notify_tx.send()` 将失败（因为 `slot_notify_rx` 已被 move 进 spawn task 且 task 已退出）。

**建议**: spawn task 退出时应通过某种机制通知 SessionManager 标记 session 为 dead；或在 `allocate_slot` 时检测 `slot_notify_tx.is_closed()`。

#### CR-3: 多 Slot 共享 `messages` 历史导致对话上下文交叉污染

> **[Deferred]** 与 CR-1 同源，推迟至 Continuous Batching。per-slot 上下文隔离将与 slot 并发管理一起重构。
**文件**: `Src/Session_Manager/session.rs:131,145,215-216`
```rust
let mut messages: Vec<Message> = Vec::new();  // spawn task 局部变量
// ...
messages.push(Message { role: "user", ... });   // 任意 slot 的 prompt
// ...
messages.push(Message { role: "assistant", ... }); // reply 存入
```
虽然当前只有一个 slot 有实际效果（CR-1），但 `messages` 是全 Session 共享的。一旦 CR-1 修复，不同 slot 的 user/assistant 消息将混合在同一 `messages` 中 → 每个 slot 都看到其他 slot 的历史。

**建议**: messages 应 per-slot。改为 `HashMap<usize, Vec<Message>>` 或每个 slot 独立 Session instance。

### 🟠 High

#### CR-4: `messages` Vec 无限增长，长对话 OOM

> **[Deferred]** 推迟处理。当前单 slot 短对话场景不触发，后续与上下文窗口管理一并解决。
**文件**: `Src/Session_Manager/session.rs:131`
每轮 push user + assistant，永远不截断。100 轮 × 平均 500 tokens = 50K tokens，内存 OK。但无截断机制，长对话最终 OOM。

**建议**: 加滑动窗口（如保留最近 20 轮）或 token 计数截断（如 4096 上限）。

#### CR-5: `create_session` 传入 `eos_token_id: 1`，真实 EOS 从 tokenizer 加载

> **[Fixed]** `Session.spawn()` 中 `load_tokenizer` 成功后通过 `eos_token_id.store(ml.get_eos(), Relaxed)` 回写真实 EOS。字段类型改为 `Arc<AtomicU32>` 以支持跨线程写入。
**文件**: `Src/Session_Manager/manager.rs:48`
```rust
let session = Session::new(session_id, model_id.to_string(), self.max_slots, 1);
//                                                                           ↑ 硬编码 1
```
而 `Session.spawn()` 使用 `ml.load_tokenizer()` → `ml.get_eos()` 获取真实 EOS。`Session.eos_token_id` 字段从未被 `spawn()` 读取，是 dead code。Qwen3 真实 EOS 为 151645。虽不影响功能（因 spawn 走 MlSession），但字段存在造成混淆。

**建议**: 移除 `Session.eos_token_id` 字段，或让 `spawn()` 在 load_tokenizer 后回写。

#### CR-6: Remote Chat prompt 上行裸写帧，未复用 `write_session_frame`

> **[Fixed]** `write_session_frame` / `read_session_frame` 泛型化为 `AsyncWriteExt + Unpin` / `AsyncReadExt + Unpin`。RemoteChat 和 branch_stream bridge 统一调用。`branch_stream.rs` 中 `libp2p::Stream` 通过 `.compat()` 适配。
**文件**: `Src/Orchestrator/core/branch_user.rs:590-595`
```rust
stream_write.write_all(&(payload.len() as u32).to_be_bytes()).await?;
stream_write.write_all(payload).await?;
```
而 `Src/Network/Session_Stream/protocol.rs` 已定义 `write_session_frame()`。两处行为相同但重复实现，未来帧格式变更会导致不一致。

**建议**: Remote Chat 也使用 `write_session_frame()` / `read_session_frame()`。

### 🟡 Medium

#### CR-8: `slot_tokens` HashMap 只增不减

> **[Deferred]** 推迟至 Continuous Batching 阶段与 slot 生命周期管理一并解决。
**文件**: `Src/Session_Manager/session.rs:115,127`
`slot_tokens.insert(new_id, new_tx)` 添加后永不移除。Slot 关闭时 `token_tx` sender 残留。虽然 `UnboundedSender` 很轻量，但长期运行会造成微小内存泄漏。

**建议**: `close_slot` 时通知 spawn task 从 `slot_tokens` 移除对应条目。

#### CR-9: 自回归循环硬编码 max_tokens=300

> **[Deferred]** 暂时不管。
**文件**: `Src/Session_Manager/session.rs:171`
```rust
for _ in 0..300 {
```
300 tokens ≈ 600-800 中文字符，对大多数对话足够，但无配置。超长生成立即截断无提示。

**建议**: 从 config/params 读取 `max_tokens`。

#### CR-10: 采样温度硬编码 0.0 (greedy)

> **[By Design]** 故意设计，当前阶段只需 greedy decoding。
**文件**: `Src/Session_Manager/session.rs:183`
```rust
let token_id = match ml.sample(&logits, 0.0) {
```
只支持 greedy decoding，无法控制创造性和多样性。

**建议**: 从 config/params 读取 temperature，默认 0.8。

#### CR-11: `strip_think` 未闭合 `<think>` 标签处理有误

> **[Deferred]** 暂时不管。
**文件**: `Src/Session_Manager/session.rs:232-237`
```rust
if let Some(end) = after_start.find(think_end) {
    remaining = &after_start[end + think_end.len()..];
} else {
    result.push_str(remaining);  // 把含 <think> 的尾部全部保留
    return result;
}
```
注释说「跳过整段」，实际是保留未闭合标签及之后所有文本。若模型输出 `<think>some reasoning` 后截断（无 `</think>`），这段会直接暴露给用户。

**建议**: 未闭合 `<think>` 应整段丢弃（不 push 到 result）。

#### CR-12: Remote Chat bridge 退出时无 Session 端通知

> **[Deferred]** 暂时不需要处理。
**文件**: `Src/Orchestrator/core/branch_stream.rs:89-95`
Bridge 在 `token_rx.recv()` 返回 `None` 时 break，但 Session 的 spawn task 仍持有 `token_tx` sender 并不知道远端已断开，下次推理仍会 `send`（虽然会失败）。

**建议**: bridge 退出时通知 SessionManager 关闭对应 slot。

#### CR-13: `MlSession.forward()` 自动递增 offset 与显式传 offset 语义冗余

> **[Recorded]** 已写入 `docs/potential_risk.md` #13。当前不影响功能，后续移除自动模式。
**文件**: `Src/ML_Engine/context.rs:282-291`
```rust
if offset.is_none() {
    self.ctx.offset += seq_len;
}
```
Session 和 ML Thread 都显式传递 offset（从不用 None 路径），自动递增仅在旧 `run.lua` 脚本中用到。两种模式混用容易混淆。

**建议**: 明确区分 "auto-offset mode" vs "explicit mode"，或移除自动递增。

### 🟢 Low / Style

#### CR-14: `apply_chat_template` 参数命名 `_messages`

> **[Fixed]** 去掉下划线前缀，改为 `messages`。
**文件**: `Src/ML_Engine/context.rs:230`
下划线前缀表示"有意未使用"，但实际在循环中使用了。去掉下划线。

#### CR-15: `MlSession.encode()` 内部硬编码 Qwen3 chat 格式

> **[Fixed]** 添加 `#[deprecated(note = "use encode_messages() instead")]`。
**文件**: `Src/ML_Engine/context.rs:170`
```rust
let format_prompt = format!("<|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n");
```
`spawn()` 已改用 `encode_messages()`（支持多轮），此方法是旧代码。应标记 `#[deprecated]` 或删除。

#### CR-16: `Session` 使用 `RefCell` 持有 `slot_notify_rx`

> **[Fixed]** `Session` 去除 `slot_notify_rx` 字段。channel pair 在 `create_session` 中创建，tx 传 `Session::new()`，rx 传 `Session::spawn()`。不再需要 `RefCell`。
**文件**: `Src/Session_Manager/session.rs:25`
```rust
pub slot_notify_rx: std::cell::RefCell<Option<...>>
```
`spawn(&self)` 需要 `take()` rx，但 `Session` 在 `Arc<Mutex<SessionManager>>` 内，而 `spawn` 调用时持有 `Mutex` lock → `borrow_mut()` 不会 panic。但若不小心在无 lock 下调用会 panic。设计脆弱。

**建议**: `create_session` 直接构造 channel，将 tx 存 Session、rx 传给 spawn，避免 `RefCell`。

### ✅ Positive Findings

1. **架构遵循好**: Core B3 `route_stream` 纯路由+spawn，不阻塞主循环 ✅
2. **错误处理一致**: 所有错误通过 EventBus 上报 TUI ✅
3. **LocalStreamHub 设计优雅**: accept_async + notifier 模式避免 busy-wait ✅
4. **Slot 隔离设计**: Slot 作为独立单元持有独立 mpsc 通道对，概念清晰 ✅
5. **Session Stream 协议**: 帧格式清晰 (4B len + UTF-8)，有 64KB 上限保护 ✅
6. **集成测试覆盖**: session.rs/manager.rs 均有单元测试，涵盖 slot 分配/释放/耗尽边界 ✅
7. **KV Cache 自动清理**: inference.lua 在 offset=0 时 `reset_kv_cache`，Session 侧每轮 prefill offset=0 ✅
8. **think 过滤**: assistant reply 中 `<think>...</think>` 块正确过滤后再存 messages ✅
9. **lua_tensor 支持 U32 dtype**: token IDs 可高效序列化 ✅
10. **多轮 chat template**: 硬编码 Qwen3 格式覆盖 system/user/assistant 三种角色 ✅

### Summary

| 级别 | 数量 | 关键项 |
|------|------|--------|
| 🔴 Critical | 3 | 多 slot prompt_rx 覆盖, spawn 崩溃无感知, messages 交叉污染 — **推迟至 Continuous Batching** |
| 🟠 High | 3 | messages OOM (已推迟), eos dead code (已修复), remote chat 裸帧 (已修复) |
| 🟡 Medium | 5 | slot_tokens 泄漏, max_tokens/temp 硬编码, strip_think 缺陷, bridge 退出, offset 冗余 |
| 🟢 Low | 3 | 命名 (已修复), 废弃方法 (已修复), RefCell (已修复) |

**最优先修复**: CR-1 (slot 覆盖) 和 CR-3 (messages 污染) 是同一问题的一体两面 — 需要将 per-slot 状态（prompt_rx + messages）从 spawn task 的局部变量改造为结构化 per-slot 管理。
