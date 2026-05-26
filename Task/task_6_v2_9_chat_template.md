# Task 6 v2.9: Chat Template 规范化 — 读取 GGUF 模板 + 过滤 think 块 + 多轮完整历史

> Presented by KeJi
> Date: 2026-05-26

---

## 目标

1. 从 GGUF metadata 读取 `tokenizer.chat_template` 替代硬编码 Qwen3 模板
2. 多轮对话改为每轮重新 tokenize 完整历史（messages 数组），而非增量拼接
3. 过滤 `<think>...</think>` 块，不将其保留在对话历史中

---

## 背景

### 当前问题

- **硬编码模板**：`context.rs` 写死 Qwen3 格式 `<|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n`，换模型就错
- **增量拼接**：每轮只 encode 新 prompt，历史靠 KV Cache。无法在 encode 层面对完整上下文做一致性处理
- **think 污染**：`<think>...</think>` 是模型内部推理过程，不应保留在对话历史中。当前随普通 token 一起进入 KV Cache，导致下一轮模型看到上一轮的 thinking 内容

### llama.cpp 的做法

```
每轮: messages = [
  {role: "system", content: "..."},
  {role: "user", content: "hello"},
  {role: "assistant", content: "Hi!"},  ← 不含 <think> 块
  {role: "user", content: "how are you"},
]
→ apply chat_template → tokenize 完整文本 → llama_decode
→ KV Cache 自动命中前缀 → 只算增量
```

---

## 实施计划

### 改动的文件

| 文件 | 改动 |
|------|------|
| `Src/ML_Engine/context.rs` | ① 新增 `chat_template: String` 字段（从 GGUF 读取）；② `encode()` 改为接受 messages 数组；③ 新增 `apply_template()` 方法 |
| `Src/Session_Manager/session.rs` | ① spawn() 中维护 `messages: Vec<Message>` 历史；② 每轮 encode 完整历史；③ 自回归后过滤 think 块，只保留 user+assistant 内容到 messages |
| `Src/ML_Engine/gguf_model_manager.rs` | `GGUF_Analyze` 或新增函数读取 `tokenizer.chat_template` metadata |

### 阶段 1: 读取 GGUF chat_template

| 步骤 | 文件 |
|------|------|
| `GGUF_Analyze` 返回新增 `chat_template: Option<String>` | `gguf_model_manager.rs` |
| `Model_Arch_Info` 加 `chat_template` 字段 | `gguf_model_manager.rs` |
| `MlSession::load_tokenizer()` 时保存 `chat_template` | `context.rs` |
| 默认 fallback：若 GGUF 无 chat_template，用 Qwen3 当前模板 | `context.rs` |

### 阶段 2: messages 数组 + 多轮 encode

| 步骤 | 文件 |
|------|------|
| 定义 `struct Message { role: String, content: String }` | `context.rs` |
| `MlSession::encode_messages(&self, messages: &[Message]) -> Vec<u32>` | `context.rs` |
| 内部实现: `apply_chat_template(messages) → 文本 → tokenize(parse_special=true)` | `context.rs` |
| 若 chat_template 不可用，fallback 为当前手动拼接格式 | `context.rs` |

### 阶段 3: Session 维护 messages 历史 + think 过滤

| 步骤 | 文件 |
|------|------|
| spawn() 中 `messages: Vec<Message>` 替代 `context_len` | `session.rs` |
| 收到 prompt: `messages.push(Message::user(prompt))` | `session.rs` |
| 推理前: `ml.encode_messages(&messages)` → prefill(offset=0) | `session.rs` |
| 自回归: 累计 `assistant_reply: String` | `session.rs` |
| 过滤: 去掉 `<think>...</think>` 后 push 到 messages | `session.rs` |
| prefill 改为 offset=0（因为每轮重新 tokenize 全部历史） | `session.rs` |

### 阶段 4: 清理

| 步骤 | 文件 |
|------|------|
| 删除旧的 `context_len` 机制（KV Cache offset 跟踪） | `session.rs` |
| 简化 spawn() 逻辑：去掉增量 prefill 相关代码 | `session.rs` |
| 旧的 `encode()` 方法保留或标记 deprecated | `context.rs` |

---

## 不在此范围

- Jinja 模板引擎（先支持 Qwen3 模板字符串替换 + fallback）
- 远端 Chat 的多轮适配（模板变更自动生效）
- model 切换时的 messages 清理

---

## 人类评审

<!-- 在此区域写下评审意见 -->

