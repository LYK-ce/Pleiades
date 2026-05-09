# Orchestrator_VM 设计方案

## 1. 背景

Orchestrator 当前使用内嵌的 `TaskEngine` 执行编排指令，其中 `SlotFile` 混合了基础类型和领域类型（IoHandle/SessionHandle/Stream/TensorIo/PipelinePlan/ModelInfo），并且 `Const`/`Move`/`JumpIf` 等公共指令的 handler 实现在 Orchestrator 内部。

Vm_Base 模块提供了通用的槽位系统和 5 条公共指令的 handler，Orchestrator_VM 在此基础上扩展领域能力。

## 2. 目标

- 用 `Vm_Base::Vm` 替代 Orchestrator 内的 IP 管理和基础槽位
- 领域类型从 SlotFile 拆出，放入独立的 `OrchestratorSlots`
- 指令枚举平铺，不自嵌套，公共指令委托给 `Vm`

## 3. 目录结构

```
Src/Orchestrator/Orchestrator_VM/
├── mod.rs                  # pub use + Orchestrator_VM 结构体
├── instruction.rs          # OrchestratorInstruction 枚举（平铺，含公共 5 条）
├── slots.rs                # OrchestratorSlots（领域数据容器）
├── engine.rs               # step() 执行循环
├── inference_handler.rs    # CreateSession/ShutdownSession/RunProgram/AnalyzeModel/SplitModel
├── network_handler.rs      # SendFile/ReceiveFile/EstablishStreams/JoinWorkers
└── scheduler_handler.rs    # PlanPipeline
```
Src/Orchestrator/Orchestrator_VM/
├── mod.rs           # pub use + Orchestrator_VM 结构体
├── instruction.rs   # OrchestratorInstruction 枚举（平铺，含公共 5 条）
├── slots.rs         # OrchestratorSlots（领域数据容器）
└── engine.rs        # step() 执行循环（match 指令 → handler）
```

## 4. 指令集 (instruction.rs)

公共指令平铺进枚举，不通过 `Base(BaseInstruction)` 嵌套：

```rust
pub enum OrchestratorInstruction {
    // ─── 公共指令（委托给 Vm）──────────────────────
    Const { value: ConstValue, dst: SlotId },
    Move  { src: SlotId, dst: SlotId },
    Add   { dst: SlotId, delta: f64 },
    Jump  { target: usize },
    JumpIf { condition: SlotId, target: usize },

    // ─── 推理生命周期 ─────────────────────────────
    CreateSession {
        model: SlotId,
        device: SlotId,
        start: SlotId,
        end: SlotId,
        io: SlotId,
        tensor_io: Option<SlotId>,
        result: SlotId,
    },
    ShutdownSession { session: SlotId },
    RunProgram { session: SlotId, result: SlotId },
    AnalyzeModel { model: SlotId, result: SlotId },
    SplitModel { source: SlotId, start: SlotId, end: SlotId, output: SlotId },

    // ─── 网络操作 ─────────────────────────────────
    SendFile { peer: SlotId, file: SlotId },
    ReceiveFile {
        stream: SlotId,
        file_name: SlotId,
        file_size: SlotId,
        checksum: SlotId,
        result: SlotId,
    },

    // ─── Pipeline 规划 ────────────────────────────
    PlanPipeline { model_info: SlotId, inference_id: SlotId, result: SlotId },

    // ─── Pipeline 编排 ────────────────────────────
    EstablishStreams { plan: SlotId, result: SlotId },
    JoinWorkers { plan: SlotId, result: SlotId },
}
```

## 5. 领域槽位 (slots.rs)

与 `Vm_Base::SlotFile` 同构——一个 `HashMap<SlotId, T>`，每个 SlotId 全局唯一：

```rust
pub enum OrchestratorSlotValue {
    IoHandle(IoHandle),
    SessionHandle(SessionHandle),
    Stream(libp2p::Stream),
    TensorIO(TensorIOEndpoint),
    PipelinePlan(PipelinePlan),
    ModelInfo(ModelInfo),
}

pub struct OrchestratorSlots {
    pub slots: HashMap<SlotId, OrchestratorSlotValue>,
}

impl OrchestratorSlots {
    pub fn new() -> Self;
    pub fn set(&mut self, slot: SlotId, value: OrchestratorSlotValue);
    pub fn take_io_handle(&mut self, slot: SlotId) -> Option<IoHandle>;
    pub fn take_session(&mut self, slot: SlotId) -> Option<SessionHandle>;
    pub fn take_stream(&mut self, slot: SlotId) -> Option<libp2p::Stream>;
    pub fn take_tensor_io(&mut self, slot: SlotId) -> Option<TensorIOEndpoint>;
    pub fn take_pipeline_plan(&mut self, slot: SlotId) -> Option<PipelinePlan>;
    pub fn take_model_info(&mut self, slot: SlotId) -> Option<ModelInfo>;
}
```

每个 SlotId 类型唯一，不存在"同一个 SlotId 同时是 IoHandle 又不是"的情况。

## 6. 执行引擎 (engine.rs)

```rust
pub struct Orchestrator_VM {
    pub vm: Vm,                       // 基础槽位 + IP，来自 Vm_Base
    pub slots: OrchestratorSlots,     // 领域数据
    pub capabilities: Arc<Capabilities>,
    pub job_id: JobId,                // 作业标识，用于生成 session_id 等
}

impl Orchestrator_VM {
    pub fn new(job_id: JobId, capabilities: Arc<Capabilities>) -> Self { .. }

    pub async fn step(&mut self, program: &[OrchestratorInstruction]) -> StepResult {
        let inst = &program[self.vm.ip];
        self.vm.ip += 1;
        match inst {
            // 公共指令 → 委托给 Vm
            OrchestratorInstruction::Const { value, dst } => self.vm.handle_const(value.clone(), *dst),
            OrchestratorInstruction::Move { src, dst }    => self.vm.handle_move(*src, *dst),
            OrchestratorInstruction::Add { dst, delta }   => self.vm.handle_add(*dst, *delta),
            OrchestratorInstruction::Jump { target }      => self.vm.handle_jump(*target),
            OrchestratorInstruction::JumpIf { c, t }      => self.vm.handle_jump_if(*c, *t),
            // 领域指令 → 各自 handler（实现在 inference/network/scheduler handler 文件）
            OrchestratorInstruction::CreateSession { .. }   => self.handle_create_session(..).await,
            OrchestratorInstruction::ShutdownSession { .. } => self.handle_shutdown_session(..).await,
            OrchestratorInstruction::RunProgram { .. }      => self.handle_run_program(..).await,
            OrchestratorInstruction::AnalyzeModel { .. }    => self.handle_analyze_model(..).await,
            OrchestratorInstruction::SplitModel { .. }      => self.handle_split_model(..).await,
            OrchestratorInstruction::SendFile { .. }        => self.handle_send_file(..).await,
            OrchestratorInstruction::ReceiveFile { .. }     => self.handle_receive_file(..).await,
            OrchestratorInstruction::PlanPipeline { .. }    => self.handle_plan_pipeline(..).await,
            OrchestratorInstruction::EstablishStreams { .. } => self.handle_establish_streams(..).await,
            OrchestratorInstruction::JoinWorkers { .. }     => self.handle_join_workers(..).await,
        }
    }
        }
    }
}
```

### 6.1 Handler 分组

领域 handler 按职责分到三个文件：

| 文件 | 指令 | 说明 |
|------|------|------|
| `inference_handler.rs` | CreateSession, ShutdownSession, RunProgram, AnalyzeModel, SplitModel | 推理生命周期 |
| `network_handler.rs` | SendFile, ReceiveFile, EstablishStreams, JoinWorkers | 网络 + 流水线编排 |
| `scheduler_handler.rs` | PlanPipeline | 拓扑规划 |

每个 handler 接收 `&mut self` (Orchestrator_VM) 和对应指令参数，返回 `StepResult`。

## 7. 依赖关系

```
Vm_Base/         → std only

Orchestrator_VM/ → Vm_Base + Capabilities + ML_Engine + Scheduler + LLM_IO + Tensor_IO + Network + Storage
```

## 8. 与原 TaskEngine 的对应关系

| 原 TaskEngine | 新 Orchestrator_VM |
|---|---|
| `self.ip` | `self.vm.ip` |
| `self.slots: SlotFile` (含领域类型) | `self.vm.slots: SlotFile` (仅基础类型) + `self.slots: OrchestratorSlots` |
| `handle_const/Move/Add/Jump/JumpIf` | `self.vm.handle_xxx()` |

## 9. 不去掉的内容

- 现有 `TaskEngine` 不动，Phase 2 再迁移
- Orchestrator_VM 作为独立模块实现和测试
- `Capabilities` 保持现有结构不变

## 10. 讨论项

- [x] `OrchestratorSlots`：单个 `HashMap<SlotId, OrchestratorSlotValue>`，与 Vm_Base::SlotFile 同构
- [x] step() 的 `StepResult`：直接用 `vm_base::StepResult`。Continue/Abort(reason)/Done/Ready 四条路径覆盖所有场景
- [x] 补偿链：MVP 阶段不加，保留 `StepResult::Abort` 作为口子，后续在 Abort 分支中切补偿程序即可

## 11. 实施步骤

1. 创建 `Src/Orchestrator/Orchestrator_VM/` 目录
2. 实现 `instruction.rs` — `OrchestratorInstruction` 枚举（平铺，含公共 5 条 + 领域 10 条）
3. 实现 `slots.rs` — `OrchestratorSlotValue` 枚举 + `OrchestratorSlots`
4. 实现 `inference_handler.rs` — CreateSession/ShutdownSession/RunProgram/AnalyzeModel/SplitModel，参考 executor/handler_inference.rs
5. 实现 `network_handler.rs` — SendFile/ReceiveFile/EstablishStreams/JoinWorkers，参考 executor/handler_network.rs
6. 实现 `scheduler_handler.rs` — PlanPipeline，参考 executor/handler_scheduler.rs
7. 实现 `engine.rs` — `Orchestrator_VM` 结构体 + `step()` 分发
8. 实现 `mod.rs` — pub use 导出
9. 在 Orchestrator/mod.rs 中注册子模块
10. 编写单元测试
11. `cargo test` 验证

现有 `TaskEngine` 不动，后续 Phase 再替换集成。
