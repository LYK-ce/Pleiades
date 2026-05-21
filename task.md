# Pleiades 任务列表

> Presented by KeJi
> Date: 2026-05-21

---

## Task 1: 文档清理与重组

> 背景：项目已完成 Lua 迁移重构（Vm_Base / ML_VM / Orchestrator_VM 删除，TOML→Lua，Src/Lua/→Src/VM/）。
> 代码库中存在大量引用已删除组件的过时文档，需要系统性清理。

### 1.1 清理 archived_docs/ 中已完全过时的文档（直接删除）

以下文件描述已删除的 VM 架构，无保留价值：

| 文件 | 过时原因 |
|------|---------|
| `archived_docs/orchestrator_vm_design.md` | 描述已删除的 Orchestrator_VM |
| `archived_docs/ml_vm_design.md` | 描述已删除的 ML_VM |
| `archived_docs/profiler_design.md` | 基于已删除 VM 的 profiler 设计 |
| `archived_docs/scheduler_design.md` | 已删除的 Scheduler 模块 |
| `archived_docs/ML_design.md` | 基于已删除 VM 的旧 ML 设计 |
| `archived_docs/core_instruction_handler.md` | 旧 TaskEngine 指令处理器 |
| `archived_docs/core_instruction.md` | 旧 TaskInstruction 枚举 |
| `archived_docs/ML_Engine_reforge.md` | 已完成的迁移文档 |
| `archived_docs/ML Engine new design.md` | 已被 Lua 方案取代 |
| `archived_docs/report_4.30.md` | 引用已删除的旧 executor |
| `archived_docs/中期总结.md` | 引用已删除的旧架构 |
| `archived_docs/instruction.md` | 旧指令设计 |
| `archived_docs/executor.md` | 旧 executor 设计 |
| `archived_docs/slot.md` | Vm_Base slot 系统 |
| `archived_docs/register.md` | 旧寄存器设计 |
| `archived_docs/control.md` | 旧控制流设计 |
| `archived_docs/Orchestrator.md` | 旧 Orchestrator 设计 |
| `archived_docs/Pleiades_Design_Document_v2.md` | 旧架构设计文档 |
| `archived_docs/Pleiades设计文档.md` | 旧架构设计文档（中文） |
| `archived_docs/problems.md` | 被 `problem.md` 取代 |
| `archived_docs/modification.md` | 旧修改记录 |
| `archived_docs/IO.md` | 旧 LLM_IO 设计 |
| `archived_docs/stream.md` | 旧流协议设计 |
| `archived_docs/portal.md` | 旧 portal 设计 |
| `archived_docs/传输层设计探讨 .md` | 已过时 |
| `archived_docs/优化编译方案.md` | 已过时 |
| `Src/Orchestrator/scheduler.md` | Scheduler 已删除 |

### 1.2 修正 docs/ 中 STALE 文档（路径更新）

以下文档内容仍然有效，但引用路径 `Src/Lua/` 需改为 `Src/VM/`：

| 文件 | 需要修正的内容 |
|------|---------------|
| `docs/code_review_lua_integration.md` | `Src/Lua/` → `Src/VM/`（engine.rs, registry.rs, capability_binding.rs） |
| `docs/capabilities_detail.md` | `Src/Lua/` → `Src/VM/`（network_stream.rs, capability_binding.rs） |
| `docs/lua_design.md` | `Src/Lua/` → `Src/VM/`（全局替换） |
| `docs/lua_integration_plan.md` | `Src/Lua/` → `Src/VM/`（全局替换） |
| `docs/summary.md` | 移除 Vm_Base 条目，修正 `Src/Lua` → `Src/VM` |
| `docs/capabilities.md` | 若含 `Src/Lua` 引用则修正 |
| `docs/event_bus_reforge.md` | 若含 `Src/Lua` 引用则修正 |

### 1.3 归档 STALE 历史文档（docs/ → archived_docs/）

以下文档是迁移前/中的历史记录，保留但移入 archived_docs：

| 文件 | 原因 |
|------|------|
| `docs/lua.md` | 原始 Lua 设计方案（561行），已被 `lua_design.md` 取代，但包含完整设计思路 |
| `docs/Pleiades_v0.2.md` | 迁移计划文档（已完成） |
| `docs/lua_migration_summary.md` | 迁移前基线记录 |
| `docs/orchestrator_reforge.md` | 已完成的重构计划 |
| `docs/ml_engine_code_review.md` | 旧 ML_VM 代码审查 |

### 1.4 处理 Pleiades_doc.md（根目录空文件）

`Pleiades_doc.md` 内容为空，`docs/design_doc/pleiades_overview.md` 已是最新总览。删除空文件，若有需要后续以符号链接或 README 替代。

### 1.5 最终验证

- [x] `grep_search` 全项目搜索 `Src/Lua/` 引用，确认为 0
- [x] `grep_search` 搜索 `Vm_Base` / `ML_VM` / `Orchestrator_VM` / `program_selector` 确认仅在历史归档中出现
- [x] 确认所有保留文档中的路径与实际代码结构一致

---

## Task 2: 清理 Src/ 目录中的文档文件

> 背景：`Src/` 目录下共 9 个 `.md` 文件，分为三类：
> - (A) 旧架构设计文档，引用已删除的 VM/compiler/JobExecutor 组件 — 移入 `archived_docs/` 归档
> - (B) 已完成的琐碎任务便条 — 直接删除
> - 清理后 `Src/` 不应包含任何 `.md` 文档

### 2.1 移入 archived_docs/（归档，保留历史参考）

| 文件 | 行数 | 过时原因 |
|------|------|---------|
| `Src/Orchestrator/orchestrator_implementation.md` | 769 | 旧 VM 架构（TaskEngine/SlotFile/instruction/handler_*），全部组件已删除 |
| `Src/Orchestrator/pipeline.md` | 597 | 旧 编译/指令/JobExecutor 分布式流水线设计 |
| `Src/Orchestrator/core_job.md` | 334 | 旧 Core select! 设计（compiler/SlotFile/JobExecutor） |
| `Src/Orchestrator/test_command.md` | 120 | 旧 handler_*/task_engine 测试命令 |
| `Src/Orchestrator/advance.md` | 210 | 旧架构演进讨论（JobExecutor/SlotFile/Handler） |
| `Src/TUI/TUI_reforge.md` | 415 | 旧 TUI 重设计（LLM_IO_Broker/IoFrontend/双输入框） |
| `Src/EventBus/Event_Bus_design.md` | 294 | 旧 EventBus 事件枚举（当前 4-type 简化版已不同） |

### 2.2 直接删除（无保留价值）

| 文件 | 原因 |
|------|------|
| `Src/TUI/task.md` | 3 行已完成任务便条 |
| `Src/EventBus/task.md` | 1 行已完成任务便条 |

### 2.3 最终验证

- [x] `file_search Src/**/*.md` 确认为 0

---

## Task 3: CPU/GPU 混合推理（单脚本顺序流水线）

> 目标：单 Lua 脚本中创建两个 `MlSession`（CPU + CUDA），加载同一模型的不同层范围，顺序执行。
> 核心思路：`cpu:forward() → to_device("cuda") → gpu:forward() → to_device("cpu") → sample`

### 背景分析

当前已有的基础设施：
- `MlSession::load_model(path, start, end)` — 支持加载指定层范围的模型 ✅
- `MlSession::forward(tensor, offset)` — 支持显式 offset，不自动递增 ✅
- `pipeline1.lua` / `pipeline2.lua` — 已验证的层拆分 forward 模式 ✅

唯一缺失的环节：
- `LuaTensor` 缺少 `to_device("cpu"|"cuda")` 方法 — 无法将 CPU forward 产出的 hidden tensor 搬到 GPU，反之亦然

### 3.1 Rust：LuaTensor 增加 to_device 方法

- [x] `Src/ML_Engine/lua_tensor.rs` — `LuaTensor` UserData 新增 `to_device(device_str)` 方法，调用 `candle_core::Tensor::to_device()`
- [x] 同步更新 `cargo test` 中相关测试

### 3.2 Lua 脚本：cpu_gpu_run.lua

- [x] `programs/user/cpu_gpu_run.lua` — 单脚本实现：
  ```lua
  cpu = ml.new("cpu"); gpu = ml.new("cuda")
  cpu:load_model(path, 0, mid); gpu:load_model(path, mid+1, total)
  
  -- prefill
  t = cpu:tensorize(tokens)
  hidden = cpu:forward(t, 0)
  hidden_gpu = hidden:to_device("cuda")
  logits = gpu:forward(hidden_gpu, 0)
  logits_cpu = logits:to_device("cpu")
  tok = cpu:sample(logits_cpu, temp)
  
  -- autoregressive loop
  for i = 2, max_tokens do
      t = cpu:tensorize({tok})
      hidden = cpu:forward(t, offset)
      hidden_gpu = hidden:to_device("cuda")
      logits = gpu:forward(hidden_gpu, offset)
      logits_cpu = logits:to_device("cpu")
      tok = cpu:sample(logits_cpu, temp)
      if tok == eos then break end
      cpu:decode(tok) → print
      offset = offset + 1
  end
  ```

### 3.3 文档更新

- [x] `docs/capabilities.md` — 更新 `ml` 表，补充 `LuaTensor:to_device()` 方法说明和 CPU/GPU 混合推理示例

### 3.4 端到端验证

- [x] `cargo test` 全量通过 (103 passed, 0 failed)
- [x] 在 CPU-only 设备上验证 `cpu_gpu_run.lua`（两个 session 都用 CPU）
- [x] 在有 GPU 设备上验证 CPU+CUDA 混合推理

---

## Task 4: 验证 LocalTensorStream（本地双线程流水线）

> 目标：用两个 `exec` 命令启动两个 Lua 线程，通过 `LocalStreamHub` 传递张量，验证本地流通道。
> 模式：类似 pipeline1/pipeline2，但用 `local_tensor` API 替代 `caps.network`。

### 背景分析

已有基础设施：
- `LocalStreamHub` + `local_tensor_stream` 模块 ✅
- `register_local_stream_caps()` 绑定函数 ✅
- pipeline1/pipeline2 已验证的双流协同模式 ✅

缺失：
1. `LocalStreamHub` 未注入 `Capabilities`，`spawn_lua_script` 未注册 `local_tensor` API
2. `lua_binding.rs` 放错位置 — 应在 `VM/` 而非 `Orchestrator/local_tensor_stream/`
3. 无配套 Lua 脚本

### 4.1 重构：lua_binding 移入 VM/

- [ ] 新建 `Src/VM/local_stream.rs` — 内容从 `Src/Orchestrator/local_tensor_stream/lua_binding.rs` 迁移
- [ ] `Src/VM/mod.rs` — 新增 `pub mod local_stream`
- [ ] 删除 `Src/Orchestrator/local_tensor_stream/lua_binding.rs`
- [ ] `Src/Orchestrator/local_tensor_stream/mod.rs` — 移除 `pub mod lua_binding`
- [ ] 所有 `use crate::orchestrator::local_tensor_stream::lua_binding` → `use crate::vm::local_stream`

### 4.2 Rust：LocalStreamHub 注入 Capabilities

- [ ] `Src/Orchestrator/mod.rs` — `Capabilities` 新增 `local_stream_hub: Arc<LocalStreamHub>` 字段
- [ ] `Src/main.rs` — 创建 `Arc<LocalStreamHub>` 注入 `Capabilities`
- [ ] `Src/Orchestrator/core/branch_user.rs` — `spawn_lua_script()` 调用 `register_local_stream_caps(&lua, caps.local_stream_hub.clone())`，import 使用 `crate::vm::local_stream`

### 4.3 Lua 脚本

- [ ] `programs/user/local_coord.lua` — 协调端：open_stream("fwd") + accept_stream("bwd") → 同 pipeline1 逻辑
- [ ] `programs/user/local_work.lua` — 工作端：accept_stream("fwd") + open_stream("bwd") → 同 pipeline2 逻辑

### 4.4 文档

- [ ] `docs/capabilities.md` — 补充 `local_tensor` API 参考（当前缺失）

### 4.5 验证

- [ ] `cargo test` 全量通过
- [ ] 单设备 `exec local_coord` + `exec local_work` 端到端推理

---

## 人类评审

<!-- 在此区域写下评审意见 -->
