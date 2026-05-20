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
