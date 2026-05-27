# Task 6 v2.5: 多轮对话

> Presented by KeJi
> Date: 2026-05-25

---

## 目标

Session 支持多轮对话：维持 token 上下文，每轮新 prompt 基于之前的对话历史进行增量推理。

---

## 前置审查结论

### KV Cache 已自动支持增量

`MlSession::forward(&mut self, tensor, offset)` 调用 `GGUF_Model_Inference(model, tensor, offset)`，后者调用 `model.model.Forward(input, offset)`。candle 的 Model_Weights 内部维护 KV Cache：

- `offset=0`：input 所有 tokens 走完整 forward，写入 KV Cache
- `offset=N`：前 N 个位置从 KV Cache 读，只 forward input 的新 tokens，追加到 Cache

ML Thread 的 `inference.lua` 是纯 forward loop（`while true { recv → forward → send }`），不区分轮次，无需改动。

### Chat Template 每轮独立工作

`encode()` 每轮都会加 `<|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n`。每轮 prefill 只发新 tokens（不含历史），offset 设为历史长度，模型从 Cache 读前面、算后面。

### EOS 和 token_count 的精确时序

```
自回归循环每次迭代：
  recv logits        ← 收到上一步发送 token 的 forward 结果
  sample → token_id
  if EOS → break     ← EOS 不发回 ML（不计入 cache）
  send token to ML   ← 此 token 被 ML forward，进入 KV Cache
  offset += 1
```

因此 `context_len` 跟踪所有已发送给 ML 的 token 数（prefill + 自回归发送的），EOS 不计入。每轮结束时 `context_len = offset`（offset 指向下一个空位，即当前 cache 长度）。

---

## 实施计划

### 唯一改动文件：`Src/Session_Manager/session.rs`

| 改动点 | 当前 | 改为 |
|--------|------|------|
| 变量声明 | 无 | select! loop 前加 `let mut context_len: usize = 0;` |
| prefill offset | `send_frame(..., 0, &data)` | `send_frame(..., context_len as u64, &data)` |
| prefill 后计数 | 无 | `context_len += new_tokens.len()` |
| 自回归 offset 初始 | `let mut offset = token_ids.len()` | `let mut offset = context_len` |
| 自回归结束后回写 | 无 | `context_len = offset` |
| Context 截断 | 无 | round 开始前若 `context_len + new_tokens.len() > 4096`，做截断处理 |

### 截断策略

当新 round 的 tokens 会超出 4096 时，保留最后轮次的 tokens（简单折中）：

```
if context_len + new_tokens.len() > 4096 {
    context_len = 0;        // 从零开始，丢弃旧 KV Cache
    // 本轮的 prefill offset = 0（模型重新构建 KV）
}
```

> 后续可优化为滑动窗口：截断旧 tokens 并通过 `model.kv_cache_remove(0..N)` 清理旧 KV。当前先跑通。

### 不在此范围

- ML Thread 改动（无）
- `inference.lua` 改动（无）
- Session close / 清理命令
- reply 通道定向到 Command Output
- KV Cache 滑动窗口截断

---

## 人类评审

<!-- 在此区域写下评审意见 -->

