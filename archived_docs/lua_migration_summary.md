# Pleiades 项目现状总结 — Lua 迁移前基线

> Presented by KeJi
> Date: 2026-05-16

---

## 一、项目概览

Pleiades 是一个基于 Rust 的边缘设备分布式 LLM 推理运行时框架。核心能力：

- **单机推理**：基于 candle + GGUF，支持 Qwen3 模型 CPU/CUDA 推理
- **分布式 Pipeline**：模型按层切分，多节点流水线并行，通过 libp2p P2P 网络通信
- **指令驱动架构**：两层 VM（Orchestrator_VM + ML_VM），TOML 模板编译为指令序列执行

---

## 二、当前策略层架构（迁移前）

```
用户命令 (TUI/CLI)
    │
    ▼
Core::route_user()  ─── branch_user.rs (280行, 10+ 硬编码分支)
    │
    ├─► ProgramSelector::select()  ─── 920行 TOML→指令转换
    │       │
    │       ├─► programs/orchestrator/*.tmpl (6个模板, include_str! 嵌入)
    │       └─► programs/ml/*.tmpl (4个模板)
    │
    ▼
JobExecutor ─── tokio::spawn
    │
    ├─► Orchestrator_VM::step() ─── 18条指令, match 分发
    │       ├─ 公共指令 → Vm_Base::Vm.handler (Const/Move/Add/Jump/JumpIf...)
    │       └─ 领域指令 → handler_*.rs (CreateSession/RunProgram/SendFile...)
    │
    └─► ML_VM::execute() ─── 17条指令, match 分发
            ├─ 公共指令 → Vm_Base::Vm.handler
            └─ 领域指令 → handle_input/encode/inference/sample...
```

### 2.1 各组件规模

| 组件 | 文件 | 行数 | 职责 |
|------|------|------|------|
| `Vm_Base` | slot.rs + vm.rs + mod.rs | ~780 | SlotFile 变量管理 + 7条基础指令 |
| `ML_VM` | instruction.rs + engine.rs + slots.rs + mod.rs | ~960 | ML 领域指令 + step() 循环 + MlSlots |
| `Orchestrator_VM` | instruction.rs + engine.rs + slots.rs + 3 handler + mod.rs | ~650 | 编排领域指令 + step() 循环 + 领域 handler |
| `program_selector.rs` | 单文件 | ~920 | TOML 反序列化 + 槽位名查表 + 指令转换 |
| `branch_user.rs` | 单文件 | ~280 | 10+ 命令分支 (Run/Pipeline/Send/Profile...) |
| TOML 模板 | 10个 .tmpl 文件 | ~300 | 编排模板(6) + ML模板(4) |
| **合计** | | **~3890** | |

### 2.2 当前指令集

**Orchestrator 级 (18条)**：Const, Move, Add, Jump, JumpIf, CreateSession, ShutdownSession, RunProgram, AnalyzeModel, SplitModel, SendFile, ReceiveFile, PlanPipeline, EstablishStreams, JoinWorkers, Profile, Abort

**ML 级 (17条)**：Const, Move, Add, Jump, JumpIf, Sub, Timer, Input, Encode, Decode, Prefill, Inference, Sample, Output, EndOutput, Send, Receive, SendEOF, FillTensor

### 2.3 核心流程：Pipeline 命令

```
1. route_user 收到 Pipeline 命令
2. 生成 inference_id + 分配 JobId
3. ProgramSelector::select(JobKind::Pipeline) → 12条 Orchestrator 指令
4. 分配 IO 通道 → spawn JobExecutor
5. Orchestrator_VM 逐步执行:
   Const(model_path) → Const(device) → AnalyzeModel → PlanPipeline
   → EstablishStreams → Const(layer_start) → Const(layer_end)
   → CreateSession(tensor_io) → RunProgram → ShutdownSession
   → JoinWorkers
6. RunProgram 内部: ProgramSelector::load_ml_program_vm("coordinator")
   → 24条 ML 指令 → ML_VM::execute() 循环执行
```

### 2.4 架构问题

- **手工实现的微型编程语言**：TOML + 指令集模拟变量/控制流，可读性差、无真正控制流
- **大量冗余 match 分发**：每条指令在两个 VM 中各有一个 match 分支 + handler
- **TOML 模板脆弱**：槽位名硬编码字符串查表，拼写错误编译期不可检测
- **Core 分支膨胀**：每个新命令需在 branch_user.rs 加 ~30 行重复模板代码
- **无热加载**：模板通过 include_str! 编译期嵌入，修改需重新编译

---

## 三、Lua 迁移目标架构

```
用户命令 (TUI/CLI)
    │
    ▼
Core::route_user()  ─── 统一 Execute 分支 (~10行)
    │
    ├─► ProgramRegistry::get(command) → 脚本路径
    │
    ▼
Lua 脚本执行 (programs/*.lua)
    │
    ├─ COMMAND / DESCRIPTION / execute(params, caps)
    │
    ├─► caps:analyze_model()     ─┐
    ├─► caps:plan_weighted()      │ Rust 能力函数
    ├─► caps:create_session()     │ (mlua create_async_function)
    ├─► caps:run_program()       ─┘
    │
    └─► 控制流 (if/while/for) 由 Lua 原生处理
```

### 3.1 预期可删除代码

| 删除项 | 行数 |
|--------|------|
| `Vm_Base/` 全部 (vm.rs + slot.rs + mod.rs) | ~780 |
| `ML_VM/` instruction.rs + engine.rs + slots.rs | ~930 |
| `Orchestrator_VM/` instruction.rs + engine.rs + slots.rs + handler_*.rs | ~650 |
| `program_selector.rs` | ~920 |
| TOML 模板 (10个 .tmpl) | ~300 |
| `branch_user.rs` 大幅简化 | ~250 |
| **合计** | **~3830** |

---

## 四、Lua 模块实现现状

### 4.1 已完成

| 项 | 状态 | 详情 |
|----|------|------|
| mlua 依赖 | ✅ | `Cargo.toml`: mlua 0.12.0-rc.1, features: lua54+vendored+async |
| `Src/Lua/` 模块 | ✅ | mod.rs + engine.rs + registry.rs，已在 lib.rs 注册 |
| Lua 沙箱 | ✅ | `LuaContext::new()` — 禁用 os/io/require/dofile/loadfile，保留 string/table/math |
| ProgramRegistry | ✅ | `scan()` 扫描 `programs/` 目录，读取 COMMAND/DESCRIPTION 顶层变量 |
| 演示脚本 | ✅ | `programs/hello.lua` — 验证 Lua 引擎启动 |
| 单元测试 | ✅ | engine.rs 2个测试，registry.rs 1个测试 |

### 4.2 未完成

| 项 | 状态 | 阻塞 |
|----|------|------|
| 能力函数注册 (capability_binding) | ❌ | 核心缺失：无 Rust→Lua 函数桥接 |
| programs/*.lua 业务脚本 | ❌ | pipeline.lua / run.lua / profile.lua 等均未编写 |
| Core 路由简化 | ❌ | branch_user.rs 仍为 280 行硬编码分支 |
| TUI 命令补全切换 | ❌ | 仍硬编码命令列表，未接入 ProgramRegistry |
| reload 热加载命令 | ❌ | 未实现 |
| 遗留代码删除 | ❌ | Vm_Base / ML_VM / Orchestrator_VM / program_selector 全部未动 |

### 4.3 关键差距分析

```
当前状态:  ┌─────────────┐    ┌──────────────────┐
           │ Lua 沙箱就绪 │    │ TOML+VM 体系全量运行 │
           │ hello.lua   │    │ 10个模板 + 3层VM    │
           └─────────────┘    └──────────────────┘
           完全隔离，未集成        生产路径

目标状态:  ┌──────────────────────────────────────┐
           │ Lua 策略脚本 (programs/*.lua)         │
           │ caps:analyze_model / create_session … │
           │ 控制流由 Lua 原生处理                  │
           └──────────────────────────────────────┘
           统一路径，VM/TOML 全部删除
```

---

## 五、迁移实施路线

### Phase 1：能力函数注册（当前待做）

1. 新建 `Src/Lua/capability_binding.rs`
2. 注册 A 级能力（无需模型）：`get_available_peers`, `get_local_peer`, `send_file`, `allocate_io`, `publish_event`
3. 注册 B 级能力（模型操作）：`analyze_model`, `split_model`, `profile_model`
4. 注册 C 级能力（Session 生命周期）：`create_session`, `run_program`, `shutdown_session`
5. 注册 D 级能力（ML 推理指令）：`ml_input`, `ml_encode`, `ml_prefill`, `ml_inference`, `ml_sample`, `ml_decode`, `ml_output`, `ml_end_output`, `ml_send`, `ml_receive`, `ml_send_eof`
6. 每个能力函数需要：Rust async fn → `lua.create_async_function` → Lua Table 参数/返回值转换

### Phase 2：脚本迁移

7. 编写 `programs/pipeline.lua` → 替代 Pipeline.tmpl + coordinator.tmpl
8. 编写 `programs/run.lua` → 替代 Run.tmpl + run.tmpl
9. 编写 `programs/profile.lua` → 替代 Profile.tmpl + profile.tmpl
10. 编写 `programs/send.lua` → 替代 Send.tmpl
11. 编写 `programs/list.lua` → 替代 DisplayPeer 硬编码

### Phase 3：Core 简化

12. `branch_user.rs` 精简为统一 Execute 分支
13. `UserCommand` 新增 `Execute { command, params, reply }` 变体
14. `JobKind` 精简（仅保留 Send/Receive 网络特殊路由）

### Phase 4：遗留代码删除

15. 删除 `Vm_Base/`
16. 删除 `ML_VM/` instruction.rs + engine.rs + slots.rs
17. 删除 `Orchestrator_VM/` instruction.rs + engine.rs + slots.rs + handler_*.rs
18. 删除 `program_selector.rs`
19. 删除 10 个 .tmpl 文件
20. 整理 `Capabilities` 结构体，移除不再需要的字段

### Phase 5：TUI 集成

21. 命令补全数据源 → `ProgramRegistry::command_names()`
22. `reload` 命令实现（运行时重扫 programs/）
23. `Commands_Reloaded` 事件通知 TUI

---

## 六、当前代码关键文件索引

### Lua 模块
| 文件 | 行数 | 说明 |
|------|------|------|
| `Src/Lua/mod.rs` | 9 | 模块声明 |
| `Src/Lua/engine.rs` | 58 | LuaContext::new() 沙箱创建 |
| `Src/Lua/registry.rs` | 104 | ProgramRegistry::scan() |

### VM 体系（待删除）
| 文件 | 行数 | 说明 |
|------|------|------|
| `Src/Vm_Base/mod.rs` | 8 | |
| `Src/Vm_Base/slot.rs` | 316 | SlotId, ConstValue, SlotValue, SlotFile |
| `Src/Vm_Base/vm.rs` | 447 | BaseInstruction(7条), Vm(ip+slots), StepResult |
| `Src/ML_Engine/ML_VM/mod.rs` | 12 | |
| `Src/ML_Engine/ML_VM/instruction.rs` | 127 | MlInstruction(17条), InferenceInputType |
| `Src/ML_Engine/ML_VM/engine.rs` | 576 | ML_VM: step() + execute() + 15 handler |
| `Src/ML_Engine/ML_VM/slots.rs` | 242 | MlSlotValue(TokenIds/Tensor), MlSlots |
| `Src/Orchestrator/Orchestrator_VM/mod.rs` | 15 | |
| `Src/Orchestrator/Orchestrator_VM/instruction.rs` | 101 | OrchestratorInstruction(18条) |
| `Src/Orchestrator/Orchestrator_VM/engine.rs` | 212 | Orchestrator_VM: step() async dispatch |
| `Src/Orchestrator/Orchestrator_VM/slots.rs` | — | OrchestratorSlots |
| `Src/Orchestrator/Orchestrator_VM/inference_handler.rs` | — | CreateSession/ShutdownSession/RunProgram… |
| `Src/Orchestrator/Orchestrator_VM/network_handler.rs` | — | SendFile/ReceiveFile |
| `Src/Orchestrator/Orchestrator_VM/scheduler_handler.rs` | — | PlanPipeline/EstablishStreams/JoinWorkers |

### 编排器核心
| 文件 | 行数 | 说明 |
|------|------|------|
| `Src/Orchestrator/mod.rs` | 134 | Capabilities 结构体 + test_utils |
| `Src/Orchestrator/core.rs` | 389 | Core: select! 4分支 + spawn_job + registry |
| `Src/Orchestrator/core/branch_user.rs` | 280 | 10+ 命令分支路由 |
| `Src/Orchestrator/core/branch_command.rs` | — | 入站 Request-Response |
| `Src/Orchestrator/core/branch_stream.rs` | — | Stream 入站事件 |
| `Src/Orchestrator/core/branch_lifecycle.rs` | — | Job 生命周期 |
| `Src/Orchestrator/core/job_executor.rs` | — | JobExecutor: Orchestrator_VM 执行循环 |
| `Src/Orchestrator/command.rs` | 405 | UserCommand + NetworkProtocol |
| `Src/Orchestrator/program_selector.rs` | 920 | TOML→指令转换 |

### TOML 模板
| 文件 | 说明 |
|------|------|
| `programs/orchestrator/Run.tmpl` | 7条指令: Const×4 + CreateSession + RunProgram + ShutdownSession |
| `programs/orchestrator/Relay.tmpl` | Worker 推理编排 |
| `programs/orchestrator/Pipeline.tmpl` | 12条指令: Pipeline 端到端编排 |
| `programs/orchestrator/Send.tmpl` | 文件发送 |
| `programs/orchestrator/ReceiveFile.tmpl` | 文件接收 |
| `programs/orchestrator/Profile.tmpl` | 性能测试 |
| `programs/ml/run.tmpl` | 19条指令: 单机推理完整流程 |
| `programs/ml/relay.tmpl` | 7条指令: Relay 推理循环 |
| `programs/ml/coordinator.tmpl` | 24条指令: Coordinator 推理循环 |
| `programs/ml/profile.tmpl` | Profile 推理 |

---

## 七、结论

Lua 迁移处于 **Phase 1 早期阶段**：基础设施（沙箱 + 注册表扫描）已就绪，但能力函数桥接、业务脚本、Core 简化、遗留代码删除均未开始。当前系统仍完全运行在 TOML + 双层 VM 架构上。

迁移的核心挑战在于 **capability_binding.rs**：需要将 20+ 个 Rust async 能力函数注册为 Lua 可调用函数，并处理 Lua Table ↔ Rust struct 的双向数据转换。这是 Phase 1 剩余工作中最关键也是最复杂的一步。
