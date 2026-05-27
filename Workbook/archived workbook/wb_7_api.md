# wb_7 — API

> 2026-05-27 start
> branch: feature/session-api (from session-tokenizer)

## 讨论

- HTTP: axum
- 命令: `API <session_id>`
- 端口: 自动分配 8080+
- 端点: `/v1/models` + `/v1/chat/completions` (SSE stream)
- 内部: allocate_slot → prompt_tx/token_rx
- 对标 chat 实现，每 session 独立 server

## 进度

- [x] 创建分支 feature/session-api
- [x] axum + async-stream dep
- [x] Src/API/ mod (types, routes, server, mod)
- [x] UserCommand::Api
- [x] Core B1 handler
- [x] TUI cmd parse
- [x] help text

## 文件清单

- Cargo.toml — axum 0.7 + async-stream 0.3
- Src/API/types.rs — OpenAI 请求/响应 serde 类型
- Src/API/routes.rs — /v1/models, /v1/chat/completions (SSE + 非流式)
- Src/API/server.rs — spawn_api_server: 端口自动分配 + axum serve
- Src/API/mod.rs — pub mod
- Src/lib.rs — pub mod api
- Src/Orchestrator/command.rs — UserCommand::Api { session_id }
- Src/Orchestrator/core/branch_user.rs — Core B1 handler + BUILTIN_COMMANDS
- Src/TUI/mod.rs — api <session_id> cmd parse
