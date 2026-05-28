# Task 9: API 重构与对话模板

> Presented by KeJi
> Date: 2026-05-28

## 描述

将系统从「TUI 聊天 + 服务端管历史」重构为「纯 API + 客户端管历史 + 正确对话模板」的标准化架构。

## 子任务总览

| # | 任务 | 涉及文件数 | 状态 |
|---|------|-----------|------|
| 9.0 | 创建 task9_api 分支 | - | ✅ |
| 9.1 | 移除 TUI chat 命令 | 6 | ✅ |
| 9.2 | API 透传完整 messages | 3 | ✅ |
| 9.3 | 对话模板正确化 (minijinja) | 2 | ✅ |
| 9.4 | Session 无状态化 | 1 | ✅ |
| 9.5 | API 完善 (max_tokens, system msg) | 3 | ✅ |

```
9.1 (移除chat) ──┐
                 ├──→ 9.4 (Session无状态) ──→ 9.5 (API完善)
9.2 (API透传)  ──┘
9.3 (对话模板)  ──→ 独立，可并行
```

---

## 详细实施计划

### 9.1 移除 TUI chat 命令

#### 9.1a — `Src/Orchestrator/command.rs`

- 删除 `UserCommand::Chat { session_id: u64 }` 变体
- 删除 `UserCommand::RemoteChat { peer_name: String, session_id: u64 }` 变体
- 保留 `UserCommand::Api { session_id: u64 }`

#### 9.1b — `Src/Orchestrator/core.rs`

- 删除 `prompt_tx: broadcast::Sender<String>` 字段
- 从 `Core::new()` 签名中删除 `prompt_tx` 参数
- 从 `Core::new()` 函数体内删除对应的字段赋值

#### 9.1c — `Src/Orchestrator/core/branch_user.rs`

- 删除 `UserCommand::Chat { session_id } => { ... }` 整个分支（约 60 行）
- 删除 `UserCommand::RemoteChat { peer_name, session_id } => { ... }` 整个分支（约 60 行）
- 删除该文件顶部不再需要的 `use tokio::sync::broadcast` (如果仅用于 chat)

#### 9.1d — `Src/TUI/app.rs`

- 删除 `prompt_tx: broadcast::Sender<String>` 字段
- 删除 `prompt_buffer: String` 字段
- 删除 `prompt_cursor: usize` 字段
- 删除 `InputFocus::Prompt` 枚举变体（简化 `InputFocus` 枚举）
- 删除 `Toggle_Focus()` 方法
- 从 `App::New()` 签名中删除 `prompt_tx` 参数
- 从 `App::New()` 函数体内删除对应字段初始化

#### 9.1e — `Src/TUI/mod.rs`

- 删除 `chat <session_id>` 命令处理块（约 30 行）
- 删除 `remote chat` 命令处理块（约 20 行）
- 删除 `InputFocus::Prompt` 相关的 Tab 切换逻辑
- 删除 `Handle_Prompt_Submit()` 函数
- 删除 prompt 输入框的渲染 `Render_Prompt()`
- 删除不再需要的 `broadcast` import

#### 9.1f — `Src/main.rs`

- 删除 `let (prompt_tx, _prompt_rx) = tokio::sync::broadcast::channel::<String>(16);`
- 从 `Core::new()` 调用中删除 `prompt_tx.clone()` 参数
- 从 `TUI_Loop()` 调用中删除 `prompt_tx` 参数

---

### 9.2 API 透传完整 messages

#### 9.2a — `Src/API/types.rs`

不需要改动（`ChatCompletionRequest` 已有 `messages: Vec<ChatMessage>`，`ChatMessage` 已有 `role` + `content`）。

#### 9.2b — `Src/API/routes.rs`

**改动 `ApiRequest` 结构体**（约第 30 行）：
```diff
- pub prompt: String,
+ pub messages: Vec<ChatMessage>,   // 完整对话历史，由前端传入
+ pub max_tokens: u32,
  pub stream: bool,
  pub reply_tx: oneshot::Sender<ApiResponse>,
```

**改动 `chat_completions` handler**（约第 115-127 行）：
```diff
- // 只取最后一条 user 消息
- let prompt = req.messages.iter().rev()
-     .find(|m| m.role == "user")
-     .map(|m| m.content.clone())
-     .ok_or(StatusCode::BAD_REQUEST)?;
+ // 验证至少有一条消息
+ if req.messages.is_empty() {
+     return Err(StatusCode::BAD_REQUEST);
+ }

  state.request_tx.send(ApiRequest {
-     prompt,
+     messages: req.messages,
+     max_tokens: req.max_tokens,
      stream: req.stream,
      reply_tx,
  })
```

#### 9.2c — `Src/API/server.rs`

**改动 `spawn_slot_handler` 函数签名和实现**：将 `prompt_tx: mpsc::UnboundedSender<String>` 改为携带 messages 数组的通道类型，但考虑到与 Session 的接口变化在 9.4 才做，此处可以先保持兼容。

**实际方案**：9.2 先改 `ApiRequest` 结构体；9.4 再统一改 `spawn_slot_handler` → Session 的通道协议。

---

### 9.3 对话模板正确化 (minijinja)

#### 现状分析：数据管线已完整，只差最后一步

当前系统从 GGUF metadata 中读取 `chat_template` 的管线已完全就绪，**不需要修改 PGGUF 转换、存储或 `load_tokenizer` 流程**：

```
GGUF 原始文件
  │  metadata: { "tokenizer.chat_template": "{% for m in messages %}...", ... }
  │
  ▼  GGUF_Analyze_And_Convert（PGGUF 非 split，复制所有原始 metadata）
PGGUF 文件
  │  metadata: 完整保留 "tokenizer.chat_template"
  │  仅追加 "pleiades.model_id" + "pleiades.layer_bitmap"
  │
  ▼  MlSession::load_tokenizer(path)
  │  → GGUF_Analyze(path) → arch_info.chat_template
  │  → self.ctx.chat_template = arch_info.chat_template.clone()  ✅
  │
  ▼  apply_chat_template()
     ❌ 唯一缺失的环节：忽略 self.ctx.chat_template，硬编码 Qwen3
```

| 步骤 | 状态 | 说明 |
|------|------|------|
| GGUF → PGGUF 保留 `chat_template` | ✅ | 非 split PGGUF 完整保留 |
| `GGUF_Analyze` 读取 `"tokenizer.chat_template"` | ✅ | `Get_Metadata_String(&metadata, "tokenizer.chat_template")` |
| `load_tokenizer()` 存入 `MlContext` | ✅ | `self.ctx.chat_template = arch_info.chat_template.clone()` |
| `apply_chat_template()` 使用它 | ❌ | **唯一要改的地方** |

> ⚠️ **注意**：Split PGGUF（`GGUF_Split_Model`）会跳过 `chat_template`，但 split 模型是分布式推理分片，不用于 API 推理，不影响本次改造。

#### 9.3a — `Cargo.toml`

添加依赖：
```toml
# Jinja2 模板引擎 (GGUF chat_template 渲染)
minijinja = "2"
```

#### 9.3b — `Src/ML_Engine/context.rs`

**改动 `apply_chat_template` 方法**（当前位于约第 222-248 行）：

当前代码硬编码 Qwen3 格式，**忽略** `self.ctx.chat_template`。改为：

```rust
fn apply_chat_template(&self, messages: &[Message]) -> String {
    // 1. 优先使用 GGUF metadata 中的 chat_template（由 load_tokenizer 注入）
    if let Some(ref tmpl_str) = self.ctx.chat_template {
        match Self::render_with_minijinja(tmpl_str, messages) {
            Ok(result) => return result,
            Err(e) => tracing::warn!("chat_template render failed, fallback to qwen3: {}", e),
        }
    }
    // 2. fallback: 硬编码 Qwen3（兼容无 chat_template 的旧模型或 split PGGUF）
    Self::fallback_qwen3_template(messages)
}

/// minijinja 渲染 — 关联函数，不需要 &self
fn render_with_minijinja(tmpl_str: &str, messages: &[Message]) -> Result<String, String> {
    let mut env = minijinja::Environment::new();
    env.add_template("chat", tmpl_str)
        .map_err(|e| format!("parse template: {}", e))?;
    let tmpl = env.get_template("chat")
        .map_err(|e| format!("get template: {}", e))?;

    let msgs: Vec<minijinja::value::Value> = messages.iter().map(|m| {
        let mut map = std::collections::BTreeMap::new();
        map.insert("role".into(), m.role.clone().into());
        map.insert("content".into(), m.content.clone().into());
        minijinja::value::Value::from(map)
    }).collect();

    tmpl.render(minijinja::context! { messages => msgs })
        .map_err(|e| format!("render: {}", e))
}

/// fallback Qwen3 格式（无 chat_template 时使用）
fn fallback_qwen3_template(messages: &[Message]) -> String {
    let mut result = String::new();
    for msg in messages {
        result.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
    }
    result.push_str("<|im_start|>assistant\n");
    result
}
```

**同时删除**：
- 原有的 `render_template` 简易函数（约第 252-286 行，功能已被 minijinja 取代）
- 原有的硬编码 Qwen3 循环（`apply_chat_template` 旧实现）
- `apply_chat_template` 的 `&self` 参数保留用于访问 `self.ctx.chat_template`

---

### 9.4 Session 无状态化

#### 9.4a — `Src/Session_Manager/session.rs`

这是核心改动。当前 Session 的 spawn 内部维护 `let mut messages: Vec<Message>`，每次收到 prompt 字符串就 push 然后全量编码。

**改为**：Session 不再自管历史，从通道接收完整的 `Vec<Message>`，每次请求独立处理。

**改动点**：

1. **Slot 通道类型变更**：`SlotHandle.prompt_tx` 从 `mpsc::UnboundedSender<String>` 改为 `mpsc::UnboundedSender<Vec<Message>>`（或新的请求结构体）。

2. **session.rs 主循环**（约第 147-267 行）：
   - 删除 `let mut messages: Vec<Message> = Vec::new();`
   - 将 `prompt_rx.recv()` 收到的类型从 `String` 改为 `Vec<Message>`（或 `SessionRequest`）
   - 删除 `messages.push(Message { role: "user", ... })` 
   - 直接 `ml.encode_messages(&incoming_messages)` 编码客户端发来的 messages
   - 删除生成结束后 `messages.push(Message { role: "assistant", ... })` 

3. **影响范围检查**：需要同时修改使用 `SlotHandle` 的代码：
   - `branch_user.rs` 中 `UserCommand::Api` 分支（保留的）
   - `server.rs` 中 `spawn_slot_handler`

**新增 `SessionRequest` 结构体**（在 session.rs 或 slot.rs 中）：
```rust
pub struct SessionRequest {
    pub messages: Vec<Message>,
    pub max_tokens: u32,
}
```

#### 9.4b — `Src/Session_Manager/slot.rs`

`SlotHandle` 的 `prompt_tx` 类型需要更新。

---

### 9.5 API 完善

#### 9.5a — `Src/API/routes.rs`

- `chat_completions` handler 已通过 9.2 透传了 `messages` 和 `max_tokens`
- 确认 `ApiRequest` 结构体已包含所需字段

#### 9.5b — `Src/API/server.rs`

- `spawn_slot_handler` 改为发送 `SessionRequest { messages, max_tokens }` 而非 `String` prompt

#### 9.5c — `Src/Session_Manager/session.rs`

- 生成循环使用 `req.max_tokens` 而非硬编码 `for _ in 0..300`

---

## 数据流对比

### 改造前
```
Client → API{messages[]} → 丢弃只剩最后user → Session{自管历史Vec} → ML
```

### 改造后
```
Client → API{messages[]} → 完整透传 → Session{无状态} → ML{chat_template渲染→tokenize}
```

---

## 暂缓

- KV cache 复用 / 增量 prefill

## 备注

- 基分支: `reforge`
- 工作分支: `task9_api` ✅ 已创建
- 对话模板方案: `minijinja` crate (crates.io)
- **参考实现**: [Crane](https://github.com/lucasjinreal/Crane) — `rewrite_python_str_methods()` 函数
  - 将 GGUF chat_template 中的 Python 风格方法调用（`.split()`, `.startswith()`, `.rstrip()` 等）改写为 minijinja 过滤器语法
  - 处理 `.split(X)[0]` → `| split(X) | first` 和 `.split(X)[-1]` → `| split(X) | last`
  - 注册 `startswith`/`endswith`/`split`/`lstrip`/`rstrip`/`strip` 过滤器
  - 源文件: `crane-core/src/autotokenizer.rs`
