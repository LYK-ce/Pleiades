# Task 7: OpenAI 兼容 API

> Presented by KeJi
> Date: 2026-05-27

---

## 目标

为 Pleiades 实现 OpenAI 兼容的 HTTP API，使外部工具可以通过标准 REST API 调用本地 LLM 推理能力。

---

## 背景

当前 Pleiades 仅支持 TUI 内交互（`chat` / `remote chat`）。对外暴露标准 API 可以让第三方工具（如 Continue、Open WebUI、自定义前端）直接集成。

---

## 设计

### 总体思路

对标 `chat <session_id>` 命令的设计模式：`chat 1` 启动一对 mpsc 通道（prompt_tx / token_rx）连接到 Session，「API 1」同理——启动一个 HTTP server，通过 `allocate_slot` 拿到相同的通道对，将 HTTP 请求转换为 prompt，将 token 流转换为 HTTP 响应。

```
chat 1  → broadcast prompt_tx → slot.prompt_tx → Session → token_rx → EventBus → TUI
API 1   → HTTP POST /v1/chat/completions → slot.prompt_tx → Session → token_rx → SSE/JSON → HTTP Response
```

### 命令

```
API <session_id>
```

每个 session 独立启动一个 HTTP server（端口自动分配），互不干扰。

### HTTP 框架

axum（tokio 原生，与现有技术栈一致）

### 端口分配

从 8080 起递增自动分配，创建时打印端口号。

### API 端点

| Method | Path | 说明 |
|--------|------|------|
| `GET` | `/v1/models` | 列出当前 session 绑定的模型 |
| `POST` | `/v1/chat/completions` | Chat Completions（支持 `stream: true` SSE） |

### 请求/响应格式

**POST /v1/chat/completions 请求体**（OpenAI 子集）：
```json
{
  "model": "qwen3",
  "messages": [
    {"role": "user", "content": "Hello"}
  ],
  "stream": false,
  "max_tokens": 256
}
```

**非流式响应**：
```json
{
  "id": "chatcmpl-xxx",
  "object": "chat.completion",
  "created": 1716931200,
  "model": "qwen3",
  "choices": [{
    "index": 0,
    "message": {"role": "assistant", "content": "..."},
    "finish_reason": "stop"
  }]
}
```

**流式响应 (SSE)**：
```
data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
```

### 模块结构

新增 `Src/API/` 模块：

```
Src/API/
├── mod.rs          — 公开接口：spawn_api_server
├── server.rs       — axum HTTP server 启动逻辑
├── routes.rs       — /v1/models, /v1/chat/completions 路由
└── types.rs        — OpenAI 请求/响应 serde 类型
```

### 架构集成

API server 作为 `tokio::spawn` 任务运行，通过 `Arc<Mutex<SessionManager>>` 直接访问会话管理，不经过 Core 主循环路由。

- API server 持有：`Arc<Mutex<SessionManager>>`（用于 `allocate_slot`）
- 每个 API 命令创建一个新 server 实例，绑定独立端口
- Session 销毁时，对应的 API server 也随之关闭

### Core 集成

在 `UserCommand` 新增：
```rust
Api { session_id: u64 }
```

Core B1 处理：从 session_mgr 查找 session，spawn API server。

---

## 实施计划

0. 从当前分支 `session-tokenizer` 创建新分支 `feature/session-api`，所有改动在此分支上进行
1. 添加 `axum` 依赖到 Cargo.toml
2. 创建 `Src/API/` 模块，实现 OpenAI 类型定义
3. 实现 axum routes（`/v1/models`, `/v1/chat/completions`）
4. 实现 `spawn_api_server`：端口自动分配 + allocate_slot + prompt→token 流
5. 在 `UserCommand` 添加 `Api` 变体，Core B1 中处理
6. 在 TUI 命令解析中添加 `API <session_id>` 命令
7. 在 Workbook 中记录工作进度

---

## 人类评审

<!-- 在此区域写下评审意见 -->

---

## 已知问题

### 1. Slot 生命周期导致 Session 退出（已修复 ✅）

**现象**：第一个 HTTP 请求正常返回后，第二个请求导致 ML Thread 报 `early eof`，Session 退出。

**根因**：原实现每次请求 `allocate_slot` → 处理完 `close_slot` → `prompt_tx` 被 drop → Session 的 `prompt_rx` 返回 None → Session 退出 → local_tensor_stream 断开 → ML Thread 报 `early eof`。

**修复** (commit `b6f09b2`)：改为常驻 slot + 请求队列模式。API server 启动时 allocate slot 一次，后台 handler task 持有 `prompt_tx` + `token_rx`，HTTP 请求通过 mpsc → oneshot 串行处理，slot 永不断开。

### 2. Chat Template 处理粗糙（待解决）

**现象**：OpenCode 发出的请求带 system message + 多轮历史，但 API 只提取最后一条 `role: "user"` 的 content。导致 system prompt 丢失、多轮上下文丢失，模型可能直接生成 EOS 不输出内容。

**根因**：`Session` 接口只接受 `String`（单条 user prompt），内部自行维护 `Vec<Message>` 做追加。而 OpenAI API 语义是客户端每次请求自带完整 `messages[]` 数组，服务端无状态。

**影响**：
- system message 被丢弃 → 模型行为不符合预期
- 多轮 assistant 历史丢失 → 模型无法感知前文
- 某些 chat template 严格的模型（Qwen3）收到裸文本直接 EOS

### 3. 有状态 Session vs. 无状态 API 的架构矛盾（待决议）

**核心矛盾**：OpenAI API 是无状态的——客户端每次请求自带完整 messages，服务端不保存上下文。但当前 Pleiades 的 Session 设计是有状态的——维护消息历史，每轮追加。

**chat 与 API 的模式差异**：

| | chat 模式 | API 模式 |
|---|---|---|
| 状态持有者 | Session 服务端 | 客户端（OpenCode） |
| 消息传递 | 单条 prompt 追加 | 完整 messages[] 数组 |
| Session 复用 | 长期持有 | 每次请求可独立，或 Session 提供「替换」模式 |

**结论**：Session 需要支持两种模式——「追加」（chat 用）和「替换」（API 用）。API 侧应将完整 messages 传递给 Session，Session 端替换（而非追加）消息历史后再推理。

### 4. KV Cache 跨轮复用（非阻塞，通用优化项）

当前 chat 模式每轮都 `offset=0` + 完整 messages → 全量 prefill，KV Cache 只在同一轮 prefill → 自回归内生效。改为无状态 API 后不存在跨轮概念，因此无性能退化。真正的 KV Cache 跨轮复用是 chat 和 API 共有的远期优化项。

### 5. chat 命令的定位（待决议）

有了 API 之后，`chat` 命令的价值存疑——OpenCode / Continue 等前端能提供更好的交互体验。`chat` 可降级为调试入口或直接移除。

### 6. `remote api` 方向的可行性（已讨论，无阻塞）

原设想 `remote api <peer> <session_id>` 通过 session stream 提供远端 API 能力。技术上可行——只需让 Session 支持「替换」模式的 messages 传递，session stream 协议本身（字节流）无需改动。待本地 API 稳定后再推进。