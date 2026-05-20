# Orchestrator 设计文档

**Date**: 2026-05-20

---

## 问题追踪

### P0-1: Lua 脚本执行阻塞 Core 主循环 ✅ 已解决

**问题**: `UserCommand::Execute` 和 `UserCommand::Send` 在 `route_user()` 中直接 `await execute_lua_script(...)`，该函数内部：
1. 同步 `std::fs::read_to_string` 阻塞 tokio 线程
2. Lua 脚本执行时长不可控
3. Core 的 `select!` 循环在此期间完全停滞，B2/B3/B4 事件无法处理

**方案**: fire-and-forget 独立 OS 线程。

```
┌─ Core (select!) ─────┐         ┌─ 独立线程 ────────────────────┐
│  Execute/Send → fire  │         │  rt.block_on(async {          │
│  立即返回，不等待       │         │    lua = LuaContext::new()     │
│                        │         │    注册 caps                  │
│                        │         │    execute(...)               │
│                        │         │  })                          │
│                        │         │  结果 → event_bus.Publish()   │
└───────────────────────┘         └───────────────────────────────┘
```

**实现**: `execute_lua_script()` (async) → `spawn_lua_script()` (`std::thread::spawn` + `new_current_thread` runtime)

**决策**:
- 纯 fire-and-forget，不追踪 Job 生命周期
- 不提供 cancel 机制（后续迭代考虑）
- 不通过 mpsc 回传结果（线程内直接 publish EventBus）
- `mlua::Lua: !Send` 不构成障碍 — 实例在线程内创建/销毁

**文件**: `Src/Orchestrator/core/branch_user.rs`

---

## 架构

（待补充）
