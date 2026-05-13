# Pleiades v0.2 升级方案

## 1. 目标

### 首要目标：替换 VM 为 Lua 脚本引擎

将当前自研的三层 VM 系统（Vm_Base → ML_VM + Orchestrator_VM）全部替换为 Lua 脚本引擎。Rust 侧退化为纯能力提供层，策略和推理流程由 Lua 脚本驱动。

**覆盖范围**：
- 用 Lua 脚本实现调度策略（uniform / weighted / 用户自定义）
- 用 Lua 脚本实现模型分配方案（层划分、拓扑编排）
- 用 Lua 脚本实现推理流程编排（CreateSession → RunProgram → ShutdownSession）
- 保留 ML 推理指令的执行能力（通过 Lua 调用 Rust 函数完成推理循环）

### 次要目标：规范化 API 实现

将当前拼凑式的模块接口整理为统一的、层次分明的 API 面：

- 消除模块间的松散耦合（如 SlotId 命名空间混乱）
- 统一错误处理和返回值格式
- 清理死代码和占位实现
- 为 Lua 侧提供类型稳定、语义清晰的函数签名

### 核心原则

**Lua 负责全部逻辑，Rust 只提供纯 API。** Capabilities 不包含任何业务判断：

```
┌──────────────────────────────────────────────────┐
│              Lua 脚本（全部逻辑）                   │
│  调度算法, 节点筛选, 层分配, 推理循环,             │
│  策略组合, 超时处理, 回退方案, 错误恢复            │
├──────────────────────────────────────────────────┤
│           Rust Capabilities（纯 API）              │
│  PeerManager:  返回节点列表                       │
│  ML Engine:    加载模型, 前向推理, 采样           │
│  Network:      建流, 发数据, 收数据               │
│  Storage:      读写文件, 校验和                   │
│  IoBroker:     分配文本通道                       │
│  EventBus:     发布事件                           │
│  TensorIO:     张量流管理                         │
└──────────────────────────────────────────────────┘
```

### 命令约定

- **内置命令**（无前缀）：`ls` `display peer` `set device` `quit` `cancel` `reload` — 由 Core 直接调用能力接口，不走 Lua
- **程序命令**（`/` 前缀）：`/pipeline` `/profile` `/hello` — 去 `programs/` 目录查找同名 `.lua` 脚本，创建一个 tokio task + Lua 实例来执行

## 2. 架构变更总览

### 2.1 删除的模块

| 模块 | 原因 |
|------|------|
| `Src/Vm_Base/` 整个目录 | Lua 原生提供变量存储、控制流、函数调用 |
| `Src/ML_Engine/ML_VM/instruction.rs` | MlInstruction 枚举不再需要 |
| `Src/ML_Engine/ML_VM/engine.rs` | step() 循环不再需要 |
| `Src/ML_Engine/ML_VM/slots.rs` | MlSlots 不再需要，状态由 ML Engine 内部管理 |
| `Src/Orchestrator/Orchestrator_VM/instruction.rs` | 指令枚举不再需要 |
| `Src/Orchestrator/Orchestrator_VM/engine.rs` | step() + dispatch 不再需要 |
| `Src/Orchestrator/Orchestrator_VM/slots.rs` | OrchestratorSlots 不再需要 |
| `Src/Orchestrator/program_selector.rs` | TOML 解析 + 槽位查表 + 指令转换全部删除 |
| `programs/orchestrator/*.tmpl` | 替换为 `programs/*.lua` |
| `programs/ml/*.tmpl` | 推理循环逻辑移至 Lua 脚本 |
| `Src/Scheduler/service.rs` | 调度逻辑由用户 Lua 脚本实现 |
| `Src/Scheduler/weighted.rs` | 同上 |
| `Src/Scheduler/capability.rs` union | Scheduler_Capability trait + Scheduler_Service + Scheduler_Weighted |
| `Scheduler_Strategy` 枚举 | 策略由 Lua 脚本决定，不再需要枚举 |

### 2.2 新增的模块

| 模块 | 职责 |
|------|------|
| `Src/Lua/` | Lua 集成模块 |
| ├── `mod.rs` | 模块声明 |
| ├── `engine.rs` | Lua 上下文工厂：创建实例、配置沙箱、注册能力函数 |
| ├── `registry.rs` | 脚本扫描与命令注册表 |
| └── `bindings/` | 能力函数绑定（按领域拆分） |
| &emsp; ├── `peer.rs` | get_available_peers 等 |
| &emsp; ├── `model.rs` | analyze_model / split_model / profile_model |
| &emsp; ├── `session.rs` | create_session / run_program / shutdown_session |
| &emsp; ├── `network.rs` | send_file / establish_streams / join_workers |
| &emsp; └── `ml.rs` | ml_input / ml_encode / ml_prefill / ml_inference / ... |
| `programs/*.lua` | 策略脚本（替代 TOML 模板） |
| `programs/schedulers/` | 参考调度脚本（uniform.lua / weighted.lua），用户可自行编写 |

### 2.3 修改的模块

| 模块 | 变更 |
|------|------|
| `Src/Orchestrator/core/branch_user.rs` | 从 8 个硬编码 match 分支简化为 2 条：统一执行 + 网络特殊路由 |
| `Src/Orchestrator/core/job_executor.rs` | 删除 VM 循环，改为加载 Lua 脚本并调用 execute() |
| `Src/Orchestrator/job.rs` | JobKind 精简 |
| `Src/Orchestrator/Capabilities` | 移除不再需要的泛型/Stub/Scheduler 字段，整理为扁平结构 |
| `Src/Orchestrator/orchestrator_vm/` 目录 | 保留 handler 文件（inference_handler / network_handler），逻辑包装为 Lua-callable 函数 |
| `Src/ML_Engine/engine.rs` | 保留模型状态管理，删除指令分发和 MlInstruction 依赖；新增 MlContext struct |
| `Src/ML_Engine/capability.rs` | ML_Engine_Capability trait 新增原子方法（Prefill/Inference/Sample/...）替代 Run_Program_VM |
| `Src/TUI/mod.rs` | 命令补全数据源切换为 ProgramRegistry，新增 Reload 事件 |
| `Cargo.toml` | 新增 `mlua` 依赖，移除不再引用的 crate |
| `tests/` | 删除依赖 VM 指令枚举的测试，新增 Lua 集成测试 |

### 2.4 不变的模块

| 模块 | 原因 |
|------|------|
| `Src/Storage/` | 底层能力，接口不受影响 |
| `Src/Network/` | 底层能力，通过高层函数间接暴露给 Lua |
| `Src/PeerManagement/` | 底层能力，通过 get_available_peers 等封装暴露 |
| `Src/Scheduler/` 数据类型 | Scheduler_Input / Pipeline_Plan / Worker_Assignment 保留（establish_streams 等解析用） |
| `Src/LLM_IO/` | IO 通道管理不变 |
| `Src/Tensor_IO/` | 张量流管理不变 |
| `Src/EventBus/` | 事件总线不变 |
| `Src/Config/` | 配置管理不变 |
| `Src/ML_Engine/` 其余文件 | GGUF 加载/模型定义/Session 管理保留 |

## 3. 实施计划

**总原则：渐进而非一次到位。** 每个阶段完成后必须 `cargo test` 全量通过。

### Phase 1：Lua 基础设施搭建（不碰现有代码）

**目标**：Lua 能跑起来，注册 2-3 个示范函数，写 1 个最简单的脚本验证链路通。

| 步骤 | 内容 | 影响 |
|------|------|------|
| 1.1 | `Cargo.toml` 添加 `mlua` 依赖 | 仅新增依赖 |
| 1.2 | 创建 `Src/Lua/mod.rs` + `engine.rs` + `registry.rs` | 纯新增 |
| 1.3 | 实现 `LuaContext::new()`：创建实例、配置沙箱 | 纯新增 |
| 1.4 | 注册 2 个示范函数：`caps:list_peers()` / `caps:echo(msg)` | 纯新增 |
| 1.5 | 写 `programs/hello.lua`：打印节点列表 | 纯新增 |
| 1.6 | 加一个单元测试：`cargo test --lib lua` 验证脚本加载和执行 | 纯新增 |

**验证标准**：加载 `programs/hello.lua`，调用 `execute()`，日志输出节点数量。

### Phase 2：逐步添加 Lua 脚本（双轨并存）

**目标**：`programs/` 下同时存在 `.tmpl`（旧）和 `.lua`（新），Lua 脚本逐步覆盖现有功能。

能力函数分三组注册：

- **A 组（不改造 trait，纯绑定）**：`get_available_peers` `get_local_peer` `analyze_model` `split_model` `send_file` — 7 个函数
- **B 组（需在 ML_Engine_Capability 新增原子方法）**：`ml:input` `ml:encode` `ml:prefill` `ml:inference` `ml:sample` `ml:decode` `ml:output` `ml:end_output` `ml:is_done` `ml:send` `ml:receive` `ml:send_eof` `ml:timer_start/end` `ml:get_text/tokens` — 15 个函数
- **C 组（组合调用 A+B）**：`create_session` `run_program` `shutdown_session` `profile_model` `establish_streams` `join_workers` — 6 个函数

**调度策略不注册为 Capability**，由 Lua 脚本直接实现。`programs/schedulers/` 下提供 `uniform.lua` 和 `weighted.lua` 参考脚本。

| 步骤 | 内容 | 影响 |
|------|------|------|
| 2.1 | 注册 A 组：节点查询 + 模型分析 + 文件传输 | 纯新增 |
| 2.2 | 实现 `LuaContext::new(caps: Arc<Capabilities>)`：接收能力集 | 修改 engine.rs |
| 2.3 | 写 `programs/schedulers/uniform.lua` 参考调度脚本 | 纯新增 |
| 2.4 | 写 `programs/pipeline.lua`：analyze → schedule → establish → create → run → shutdown → join | 纯新增 |
| 2.5 | `Core::route_user` 新增 `/` 前缀路由：输入以 `/` 开头 → 走 Lua 执行路径 | 修改 Core |
| 2.6 | B 组：ML_Engine_Capability trait 新增 15 个原子方法 + MlContext struct | 修改 ML Engine |
| 2.7 | 注册 B 组 + C 组到 Lua | 修改 engine.rs |
| 2.8 | 写 ML 推理 Lua 脚本（替代 run.tmpl / relay.tmpl / coordinator.tmpl） | 纯新增 |
| 2.9 | 手动对比 `/pipeline` 和 `pipeline` 命令行为一致性 | 验证 |

**验证标准**：Lua 脚本的输出结果与对应 `.tmpl` 模板一致。

### Phase 3：切换执行路径（旧代码变死代码）

**目标**：Core 的命令路由从旧 VM 路径切换到 Lua 路径，旧路径保留但不再走。

| 步骤 | 内容 | 影响 |
|------|------|------|
| 3.1 | `Core` 启动时扫描 `programs/`：分离 .tmpl（旧）和 .lua（新） | 纯新增扫描逻辑 |
| 3.2 | TUI 命令补全同时显示旧命令和新命令 | 修改 TUI |
| 3.3 | `route_user` 新增 `Execute` 分支：优先走 Lua，fallback 旧路径 | 修改 Core |
| 3.4 | 逐个将 UserCommand 变体改为默认走 Lua 路径 | 修改 Core |
| 3.5 | `pipeline` 命令切换到 Lua → 验证结果一致 → 确认稳定 | 切换 |
| 3.6 | `profile` 命令切换到 Lua → 验证 | 切换 |
| 3.7 | `run` 命令切换到 Lua → 验证 | 切换 |

**验证标准**：Lua 路径执行的所有 TUI 命令产出与旧路径一致。

### Phase 4：移除遗留代码

**目标**：删除旧 VM 相关的所有代码，`programs/` 下仅保留 `.lua` 文件。

| 步骤 | 内容 |
|------|------|
| 4.1 | 删除 `programs/orchestrator/*.tmpl` + `programs/ml/*.tmpl` |
| 4.2 | 删除 `Src/Orchestrator/program_selector.rs` |
| 4.3 | 删除 `Src/Vm_Base/` 整个目录 |
| 4.4 | 删除 `Src/ML_Engine/ML_VM/instruction.rs` + `engine.rs` + `slots.rs` |
| 4.5 | 删除 `Src/Orchestrator/Orchestrator_VM/instruction.rs` + `engine.rs` + `slots.rs` |
| 4.6 | 删除 `JobKind` 旧变体 + `UserCommand` 旧变体 |
| 4.7 | 删除 Core 中的旧路径 fallback 代码 |
| 4.8 | 更新 `tests/`：删除依赖旧 VM 的测试，更新为 Lua 版本 |
| 4.9 | `cargo test` 全量回归 |

### 阶段依赖关系

```
Phase 1 ──→ Phase 2 ──→ Phase 3 ──→ Phase 4
  │                      │
  └─ 不碰现有代码        └─ 仅此时开始替换
```

Phase 3 之前，现有系统完全不受影响。Phase 4 才做删除。

## 4. API 规范化

> 📋 暂留空。在 Lua 实施过程中，根据 Lua 脚本的实际调用需求反向推导 API 设计，届时再填充此节。

## 5. 风险与注意事项

| 风险 | 缓解 |
|------|------|
| 脚本运行时错误定位难 | Lua error → 包装为友好提示，包含行号和上下文字段 |
| 新旧路径行为不一致 | Phase 3 逐命令对比验证后再切换 |
| `Lua` 实例非 Send | 每个 Job 独立创建 `Lua` 实例，在各自 tokio task 中运行，天然隔离 |


