# reforge 工作记录
> branch: reforge | start: 2026-05-20 | base: 89a1fee (pull from remote)

## Phase 1 状态 (远程已完成前5项)
- [x] Cargo.toml mlua 依赖
- [x] Lua 沙箱 (Src/VM/engine.rs)
- [x] ProgramRegistry (Src/VM/registry.rs)
- [x] 示范能力函数注册 (Src/VM/capability_binding.rs)
- [x] programs/builtin/pipeline.lua
- [ ] cargo test 全量

## 远程 reforge 重大变更 (44bf19c → 89a1fee)
- Src/Lua/ → Src/VM/ (含 capability_binding, network_stream, storage_handle)
- programs/ 重组: builtin/{list,run,send,pipeline}.lua + user/{hello,pipeline1,pipeline2,run}.lua
- ML_VM/, Orchestrator_VM/, Vm_Base/, Scheduler/ 全删
- program_selector.rs 删除, TOML 模板全删
- Orchestrator/core/ 重构 (branch_user/command/stream/lifecycle)

## reload 命令实现 (completed)
开始: 2026-05-20 | 结束: 2026-05-20
修改文件:
- Src/Orchestrator/command.rs: +UserCommand::Reload
- Src/Orchestrator/core/branch_user.rs: route_user 处理 Reload → program_registry.reload_user() → Bus_Event::Output 返回结果; BUILTIN_COMMANDS + ("reload", "重新加载用户 Lua 脚本")
- Src/TUI/mod.rs: Handle_Command_Input + "reload" 分支 → UserCommand::Reload
cargo check --no-default-features: 0 errors, 19 pre-existing warnings

## 独立本地流实现 (completed)
开始: 2026-05-20 | 结束: 2026-05-20
新增文件:
- Src/Orchestrator/local_tensor_stream/mod.rs: LocalStreamHub (open/accept 配对 + 2 tests)
- Src/Orchestrator/local_tensor_stream/frames.rs: 泛型帧函数 local_send_frame/recv_frame/send_eof + 2 tests
- Src/Orchestrator/local_tensor_stream/lua_binding.rs: LocalTensorStream userdata, register_local_stream_caps
- docs/design_doc/local_tensor_stream_design.md: 完整设计文档
修改文件:
- Src/Orchestrator/mod.rs: +pub mod local_tensor_stream
- Src/VM/capability_binding.rs: +register_local_stream_caps 导出
cargo check --no-default-features: 0 errors, 20 warnings (1 fixed: unused io import)
cargo test --lib local_tensor_stream: 4 passed, 0 failed
Lua API: local.open_stream(id), local.accept_stream(id, timeout), local.send_tensor(s, data, offset), local.recv_tensor(s), local.send_eof(s)

## 2026-05-21 会话初始化
- 切换 reforge 分支 (跟踪 origin/reforge, commit 19997f9)
- SSH 初始化: 权限 OK (600/644), 认证 OK (LYK-ce), remote → git@github.com:LYK-ce/Pleiades.git

## 2026-05-21 任务重构
- 清理旧 task.md（Lua 迁移 4 阶段 + Bug 修复 + 性能优化 全部移除）
- 新建 Task 1: 文档清理与重组
  - 1.1 删除 classified_docs/ 中 26 个完全过时文件（描述已删除 VM 架构）
  - 1.2 修正 docs/ 中 ~7 个 STALE 文件的路径引用（Src/Lua/ → Src/VM/）
  - 1.3 归档 5 个历史文档（docs/ → classified_docs/）
  - 1.4 删除空文件 Pleiades_doc.md
  - 1.5 最终 grep 验证无残留引用
