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

