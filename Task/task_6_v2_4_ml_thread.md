# Task 6 v2.4: ML Thread 接入 — 自回归推理闭环 ✅

> Presented by KeJi
> Date: 2026-05-25

---

## 状态：已完成

端到端推理链路全部闭合并验证通过。105 tests pass。

---

## 最终链路

```
pleiades> session create test.pgguf
  → Session.spawn() → load_tokenizer → accept_async("session-{id}") + accept_async("ml-{id}")

pleiades> chat 1
  → chat task → subscribe(prompt_tx) → open("session-1") → loop { send prompt }

pleiades> session inference 1 test.pgguf
  → Core 查 builtin/inference.lua → spawn_lua_script
  → storage_acquire_read → load_model → open_stream("ml-1") → forward loop

[Tab] Prompt> 你好
  → prompt_tx.send → chat task → local_send_frame → Session
  → Session: encode → tensorize (U32) → tensor_to_bytes → send (prefill, offset=0)
  → ML Thread: recv → bytes_to_tensor (U32) → forward → tensor_to_bytes (F32) → send
  → Session: recv → bytes_to_tensor (F32) → sample (temp=0.0) → decode → EventBus
  → loop 0..120: send next token → recv logits → sample → decode → EOS? → break
```

## 实际改动

| 文件 | 改动 |
|------|------|
| `session.rs` | spawn() 加 accept_async("ml-{id}") + chat 分支自回归 loop (max 120 tokens, EOS 检测) |
| `programs/builtin/inference.lua` | 新建：Storage 读模型 → load_model → open_stream → forward loop |
| `command.rs` | 新增 `UserCommand::SessionInference` |
| `TUI/mod.rs` | 新增 `session inference <id> <path>` 命令解析 |
| `branch_user.rs` | SessionInference handler：查 builtin/inference.lua → spawn_lua_script |
| `lua_tensor.rs` | tensor_to_bytes/bytes_to_tensor 加 dtype 支持 (0=F32, 1=U32) |

## 已知问题

| 问题 | 说明 | 优先级 |
|------|------|:--:|
| logits 传输开销 | 每 token 传输 600KB logits（整个词表），应改为 ML Thread 侧 sample，只回传 token_id | 高 |
| 无 stop string 检测 | temperature=0 时 EOS 正常工作，但非 greedy 时需要 stop string 检测兜底 | 中 |
| Core 设计原则 | 已写入 instructions.md | ✅ |
| B3 route_stream 阻塞 | 已修复为 spawn 模式 | ✅ |

## 后续（未做）

- ML Thread 侧 sample（解决 600KB/logits 传输问题）
- stop string 检测 (`<|im_end|>` 等)
- reply 通道：token 返回 chat task 显示在 Command Output 区
- 多 slot 支持
- 远端 ML Thread 接入

---

## 人类评审

<!-- 在此区域写下评审意见 -->

