# wb_6

> start: 2026-05-22
> end: 2026-05-25
> branch: session-tokenizer

## State
Task 6 v2 全部子任务完成。端到端推理闭环验证通过。105 tests pass。

```
session create test.pgguf → chat 1 → session inference 1 test.pgguf
  → Prompt> 你好 → encode → prefill → forward loop (0..120)
  → sample (temp=0.0) → decode → EventBus → EOS stop
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

## 架构笔记（更新）

- Session.spawn(): load_tokenizer → accept_async("session-{id}") → accept_async("ml-{id}") → select!
- Chat 持久: broadcast::channel(16) → prompt_tx.subscribe() → loop → local_send_frame
- ML Thread: builtin/inference.lua → storage_acquire_read → load_model → open_stream("ml-{id}") → forward loop
- 自回归: chat 分支内 prefll + for 0..120 { recv ML → sample(0.0) → decode → EOS check → send next }
- tensor 序列化: 1B dtype + header + data, dtype 0=F32 1=U32
- ML Thread side: Lua 脚本，通过 spawn_lua_script 在独立线程运行

## 已知问题

- logits 传输: 每 token ~600KB，应 ML Thread 侧 sample → 只回传 token_id (4B)
- 无 stop string 检测: temp=0 时 EOS OK，非 greedy 需兜底
- slot 机制未启用: chat 一对一 session，多 slot 未挂接
- 远端 ML Thread 未实现

## 待做

- ML Thread 侧 sample（消除 logits 传输开销）
- reply 通道（TUI Command Output 区显示生成文本）
- 多 slot 支持
- 远端接入

