# Workbook: Task 9

> Branch: task9_api
> Base: reforge

## Timeline

- 2026-05-28: 任务开始，创建 task9_api 分支
- 2026-05-28: 全部子任务完成，编译通过

## Progress

| # | 子任务 | 状态 | 开始 | 结束 | 备注 |
|---|--------|------|------|------|------|
| 9.0 | 创建 task9_api 分支 | completed | 2026-05-28 | 2026-05-28 | 基于 reforge 创建 task9_api 分支 |
| 9.1 | 移除 TUI chat 命令 | completed | 2026-05-28 | 2026-05-28 | 6 files: command.rs/core.rs/branch_user.rs/app.rs/mod.rs/main.rs |
| 9.2 | API 透传完整 messages | completed | 2026-05-28 | 2026-05-28 | ApiRequest.messages 替代 prompt；ChatMessage→Message 转换 |
| 9.3 | 对话模板正确化 | completed | 2026-05-28 | 2026-05-28 | Cargo.toml+minijinja；apply_chat_template 重写；删除 render_template |
| 9.4 | Session 无状态化 | completed | 2026-05-28 | 2026-05-28 | SessionRequest 结构体；Vec<Message> 移除；SlotHandle 通道类型变更 |
| 9.5 | API 完善 | completed | 2026-05-28 | 2026-05-28 | max_tokens 生效；system msg 支持；strip_think 不再使用 |

## Key Changes

- `SessionRequest { messages, max_tokens }` 替代裸 String prompt
- 对话历史完全由前端管理，Session 变为无状态
- minijinja 渲染 GGUF chat_template，fallback Qwen3 硬编码
- TUI 双输入框简化为单输入框，删除 chat/remote chat 命令
