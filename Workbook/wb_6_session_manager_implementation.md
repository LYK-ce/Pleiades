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
| v2.8 | 远端 Chat: Session 流协议 + remote chat + 空哨兵多轮 | 5914ac1..de9e247 |
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

