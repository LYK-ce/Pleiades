# Alpha 分支工作记录

> Created: 2026-05-17
> Branch: alpha (基于 machine-learning-reforge @ 5f6f08f)
> 父分支: machine-learning-reforge

## 初始化
- 时间: 2026-05-17
- Git SSH: ✅ (ed25519, github.com/LYK-ce/Pleiades.git)
- Remote: git@github.com:LYK-ce/Pleiades.git

## 分支基线
- commit: 5f6f08f docs: summary 补充各模块功能说明和接口清单
- 已删除: ML_VM/, Orchestrator_VM/, Scheduler/, program_selector.rs, session.rs, service.rs, pipeline.rs
- 已有: context.rs (MlSession+MlContext+UserData), capability.rs (analyze/split)
- 待处理: forward/tensorize Lua 绑定占位, P2 问题 #6-#9

## 2026-05-17: 移除 Vm_Base
- 删除 Src/Vm_Base/ (slot.rs+vm.rs+mod.rs, ~780行, 47 tests)
- lib.rs: 移除 `pub mod vm_base` + 文档注释
- cargo check: ✅ 0 errors

## 2026-05-17: Lua 集成方案文档
- 创建 docs/lua_integration_plan.md
- 6 Phase 渐进式集成路线: Phase0(最小验证,纯测试) → Phase1(能力绑定) → Phase2(端到端集成测试) → Phase3(业务脚本) → Phase4(Core简化) → Phase5(TUI集成) → Phase6(遗留清理)
- Phase 0 目标: 在 Rust 测试中加载并执行 hello.lua 的 execute() 函数
- 关键设计决策: async函数统一用create_async_function, 张量不穿FFI边界, 业务级聚合暴露
- 新增 Phase 0 (设计文档): docs/lua_design.md (10章)
- 后续 Phase 重新编号: 1→7

## 2026-05-17: Lua 集成实施完成 (7 Phase)

### Phase 1: 最小验证
- engine.rs: +3 tests (load+execute hello.lua, missing execute, sandbox io)
- 6/6 lib tests passed

### Phase 2: 能力函数桥接
- 新建 capability_binding.rs: register_caps() + 5 tests
- 同步: echo/add/table_sum; 异步: ping
- Lua 调用语法: caps.method() (`.` 非 `:`)
- mod.rs: +pub mod capability_binding
- 11/11 lib tests passed

### Phase 3: 集成测试
- 新建 tests/t09_lua_integration.rs: 4 tests
- hello.lua 加载+执行, caps 函数调用, 异步 caps, 自定义脚本
- 4/4 integration tests passed

### Phase 4: 业务脚本
- 新建 programs/pipeline.lua (57行, Coordinator 分布式推理)
- 新建 programs/run.lua (25行, 单机推理)
- 新建 programs/list.lua (23行, 节点列表)

### Phase 5: Core 路由简化
- core.rs: 添加 ProgramRegistry 字段 + scan() 初始化
- branch_user.rs: Execute 分支改用 execute_lua_script()
- 添加 execute_lua_script() 辅助函数 (沙箱创建→caps注册→脚本加载→调用)
- 注意: Lua 非 Send, 不能 tokio::spawn, 改为直接 await

### Phase 6: TUI 命令补全
- app.rs: +available_commands: Vec<String> + set_available_commands()

### Phase 7: 遗留清理
- 删除 programs/orchestrator/*.tmpl (6个)
- 删除 programs/ml/*.tmpl (4个)
- program_selector.rs 已在上游分支删除

### 测试结果
- lib: 81 passed, 2 failed (storage已知: size非实时刷新)
- integration: t01(5) + t02(5) + t03(4) + t04(6) + t09(4) = 24/24 passed
- 新增测试: +3(engine) +5(capability) +4(integration) = +12 tests

