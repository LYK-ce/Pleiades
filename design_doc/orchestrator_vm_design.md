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

## 12. Phase 2 — 模板化程序 + 替换集成

### 12.1 原则

**硬编码 Compiler → 声明式 TOML 模板 + ProgramSelector。**
废弃 `compiler.rs` 中的编译方法，程序定义从前端 Rust 代码移入独立的 TOML 模板文件。新增 `program_selector.rs` 负责按需加载模板并生成指令。

### 12.2 模板目录结构

```
programs/
├── orchestrator/          # 编排层程序模板（每个 JobKind 一个文件）
│   ├── Run.tmpl
│   ├── Relay.tmpl
│   ├── Pipeline.tmpl
│   ├── Send.tmpl
│   └── ReceiveFile.tmpl
└── ml/                    # ML 引擎程序模板（每个 mode 一个文件）
    ├── run.tmpl
    ├── relay.tmpl
    └── coordinator.tmpl
```

### 12.3 模板格式

槽位用命名常量（如 `SLOT_SESSION`），变量用 `$var_name` 前缀：

```toml
[meta]
kind = "Run"

[[instructions]]
type = "Const"
value_type = "String"
value = "$model_path"
dst = "SLOT_MODEL"

[[instructions]]
type = "Const"
value_type = "String"
value = "$device"
dst = "SLOT_DEVICE"

[[instructions]]
type = "CreateSession"
model = "SLOT_MODEL"
device = "SLOT_DEVICE"
start = "SLOT_LAYER_START"
end = "SLOT_LAYER_END"
io = "SLOT_IO"
result = "SLOT_SESSION"

[[instructions]]
type = "RunProgram"
session = "SLOT_SESSION"
result = "SLOT_RESULT"
```

ML 模板支持 Loop 嵌套（TOML `[[表数组]]` 语法天然支持）：

```toml
[[instructions]]
type = "Loop"

[[instructions.body]]
type = "BreakIf"

[[instructions.body]]
type = "Inference"
input_type = "Tokens"
input = "TOKENID2"
```

### 12.4 ProgramSelector 设计

不再"编译"，只做两件事：**加载模板 + 变量替换**。

```rust
pub struct ProgramSelector;

impl ProgramSelector {
    /// 根据 JobKind 加载对应 orchestrator TOML 模板，替换变量，产出 TaskProgram
    pub fn select(kind: JobKind, vars: HashMap<String, String>)
        -> Result<TaskProgram, SelectorError>;

    /// 根据 ML mode 加载 ML 程序模板
    pub fn load_ml_program(mode: &str, params: &Pipeline_Params)
        -> Vec<Instruction>;
}
```

**编译期嵌入策略：** 模板文件通过 `include_str!()` 在编译期嵌入为字符串常量，运行时仅按需解析（调用 `select()` / `load_ml_program()` 时才做 TOML 反序列化 → 指令构造），避免分发时携带外部 `programs/` 目录。

在 `program_selector.rs` 中：
```rust
// 编译期嵌入：模板文件编译进二进制，无需运行时文件 IO，也无需分发 programs/ 目录。
// 运行时按需解析：仅在 select() / load_ml_program() 调用时做 TOML 反序列化。
const RUN_TMPL: &str = include_str!("../../../programs/orchestrator/Run.tmpl");
const RELAY_TMPL: &str = include_str!("../../../programs/orchestrator/Relay.tmpl");
// ...
const ML_RUN_TMPL: &str = include_str!("../../../programs/ml/run.tmpl");
// ...
```

**槽位常量查表：** 模板中槽位用命名常量（`SLOT_SESSION`），selector 内部维护一张映射表：
```rust
static SLOT_NAMES: phf::Map<&'static str, SlotId> = {
    // "SLOT_MODEL" → SlotId(1), etc.
};
```

**变量替换：** 模板中 `$var_name` 占位符在解析时用 `vars` HashMap 中的值替换。

### 12.5 集成替换点

| 原调用 | 改为 |
|--------|------|
| `Core.compiler: Arc<Compiler>` | `Core.selector: Arc<ProgramSelector>` |
| `self.compiler.compile_run(job_id, model, device)` | `self.selector.select(JobKind::Run, vars)` |
| `self.compiler.compile_pipeline(job_id, id, model, device)` | `self.selector.select(JobKind::Pipeline, vars)` |
| `self.compiler.compile_relay(job_id, ...)` | `self.selector.select(JobKind::Relay, vars)` |
| `self.compiler.compile_send(job_id, ...)` | `self.selector.select(JobKind::Send, vars)` |
| `self.compiler.compile_receive_file(job_id, ...)` | `self.selector.select(JobKind::Receive, vars)` |
| `Compiler::build_run_ml_program(&params)` | `ProgramSelector::load_ml_program("run", &params)` |
| `Compiler::build_coordinator_ml_program(&params)` | `ProgramSelector::load_ml_program("coordinator", &params)` |
| `Compiler::build_relay_ml_program()` | `ProgramSelector::load_ml_program("relay", &params)` |

### 12.6 不做的事

- **Distribute** — 包含动态循环生成逻辑，暂不模板化，保留 Runtime 生成
- **CompileParams** — 砍掉，直接传 `HashMap<String, String>` 做变量替换
- **`compile()` 通用分发方法** — 不再需要，每种 JobKind 独立模板文件
- **Compensation 链** — 当前阶段不引入，引入补偿链过于复杂。模板中不含 `[[compensation]]` 节，`TaskProgram.compensation` 固定为空 Vec

### 12.7 实施步骤

**Phase 2a — 模板 + ProgramSelector**
1. 创建 `programs/orchestrator/` 目录，编写 Run/Relay/Pipeline/Send/ReceiveFile 五个 TOML 模板
2. 创建 `programs/ml/` 目录，编写 run/relay/coordinator 三个 TOML 模板
3. 新增 `Src/Orchestrator/program_selector.rs` — ProgramSelector 结构体 + `include_str!()` 嵌入 + TOML 解析 + 槽位常量查表 + 变量替换
4. 在 `Orchestrator/mod.rs` 中注册 `program_selector` 子模块
5. 编写单元测试
6. `cargo test` 验证

**Phase 2b — 集成替换**
7. `Core` — `compiler: Arc<Compiler>` → `selector: Arc<ProgramSelector>`
8. `branch_user.rs` / `branch_command.rs` / `branch_stream.rs` — 调用点从 `compile_xxx()` 切到 `selector.select()`
9. `inference_handler.rs` — `Compiler::build_xxx_ml_program()` → `ProgramSelector::load_ml_program()`
10. `compiler.rs` — 删除编译方法，仅保留槽位常量定义
 11. `cargo test` 全量通过


## 13. 实施计划

### 13.1 模板文件编写（8 个文件）

#### 13.1.1 Orchestrator 层

**`programs/orchestrator/Run.tmpl`**

```toml
[meta]
kind = "Run"

[[instructions]]
type = "Const"
value_type = "String"
value = "$model_path"
dst = "SLOT_MODEL"

[[instructions]]
type = "Const"
value_type = "String"
value = "$device"
dst = "SLOT_DEVICE"

[[instructions]]
type = "Const"
value_type = "U64"
value = "0"
dst = "SLOT_LAYER_START"

[[instructions]]
type = "Const"
value_type = "U64"
value = "18446744073709551615"
dst = "SLOT_LAYER_END"

[[instructions]]
type = "CreateSession"
model = "SLOT_MODEL"
device = "SLOT_DEVICE"
start = "SLOT_LAYER_START"
end = "SLOT_LAYER_END"
io = "SLOT_IO"
result = "SLOT_SESSION"

[[instructions]]
type = "RunProgram"
session = "SLOT_SESSION"
result = "SLOT_RESULT"
```

**`programs/orchestrator/Relay.tmpl`**

```toml
[meta]
kind = "Relay"

[[instructions]]
type = "Const"
value_type = "String"
value = "$model_path"
dst = "SLOT_MODEL"

[[instructions]]
type = "Const"
value_type = "String"
value = "$device"
dst = "SLOT_DEVICE"

[[instructions]]
type = "Const"
value_type = "U64"
value = "$layer_start"
dst = "SLOT_LAYER_START"

[[instructions]]
type = "Const"
value_type = "U64"
value = "$layer_end"
dst = "SLOT_LAYER_END"

[[instructions]]
type = "Const"
value_type = "String"
value = "relay"
dst = "SLOT_ML_PROGRAM_MODE"

[[instructions]]
type = "CreateSession"
model = "SLOT_MODEL"
device = "SLOT_DEVICE"
start = "SLOT_LAYER_START"
end = "SLOT_LAYER_END"
io = "SLOT_IO"
tensor_io = "SLOT_TENSOR_IO"
result = "SLOT_SESSION"

[[instructions]]
type = "RunProgram"
session = "SLOT_SESSION"
result = "SLOT_RESULT"
```

**`programs/orchestrator/Pipeline.tmpl`**

```toml
[meta]
kind = "Pipeline"

[[instructions]]
type = "Const"
value_type = "String"
value = "$model_path"
dst = "SLOT_MODEL"

[[instructions]]
type = "Const"
value_type = "U64"
value = "$inference_id"
dst = "SLOT_INFERENCE_ID"

[[instructions]]
type = "AnalyzeModel"
model = "SLOT_MODEL"
result = "SLOT_MODEL_INFO"

[[instructions]]
type = "PlanPipeline"
model_info = "SLOT_MODEL_INFO"
inference_id = "SLOT_INFERENCE_ID"
result = "SLOT_PLAN"

[[instructions]]
type = "EstablishStreams"
plan = "SLOT_PLAN"
result = "SLOT_STREAMS_RESULT"

[[instructions]]
type = "JoinWorkers"
plan = "SLOT_PLAN"
result = "SLOT_WORKERS_RESULT"

[[instructions]]
type = "Const"
value_type = "String"
value = "$device"
dst = "SLOT_DEVICE"

[[instructions]]
type = "Const"
value_type = "String"
value = "coordinator"
dst = "SLOT_ML_PROGRAM_MODE"

[[instructions]]
type = "CreateSession"
model = "SLOT_MODEL"
device = "SLOT_DEVICE"
start = "SLOT_LAYER_START"
end = "SLOT_LAYER_END"
io = "SLOT_IO"
tensor_io = "SLOT_TENSOR_IO"
result = "SLOT_SESSION"

[[instructions]]
type = "RunProgram"
session = "SLOT_SESSION"
result = "SLOT_RESULT"
```

**`programs/orchestrator/Send.tmpl`**

```toml
[meta]
kind = "Send"

[[instructions]]
type = "Const"
value_type = "String"
value = "$file_path"
dst = "SLOT_SEND_FILE"

[[instructions]]
type = "Const"
value_type = "String"
value = "$peer_id"
dst = "SLOT_SEND_PEER"

[[instructions]]
type = "SendFile"
peer = "SLOT_SEND_PEER"
file = "SLOT_SEND_FILE"
```

**`programs/orchestrator/ReceiveFile.tmpl`**

```toml
[meta]
kind = "ReceiveFile"

[[instructions]]
type = "Const"
value_type = "String"
value = "$file_name"
dst = "SLOT_RECEIVE_FILE_NAME"

[[instructions]]
type = "Const"
value_type = "U64"
value = "$file_size"
dst = "SLOT_RECEIVE_FILE_SIZE"

[[instructions]]
type = "Const"
value_type = "String"
value = "$checksum"
dst = "SLOT_RECEIVE_CHECKSUM"

[[instructions]]
type = "ReceiveFile"
stream = "SLOT_RECEIVE_STREAM"
file_name = "SLOT_RECEIVE_FILE_NAME"
file_size = "SLOT_RECEIVE_FILE_SIZE"
checksum = "SLOT_RECEIVE_CHECKSUM"
result = "SLOT_RECEIVE_RESULT"
```

#### 13.1.2 ML 层

**`programs/ml/run.tmpl`**

```toml
[[instructions]]
type = "Input"

[[instructions]]
type = "Encode"

[[instructions]]
type = "Set"
target_type = "Meta"
target_reg = "META2"
target_value = "$max_tokens"

[[instructions]]
type = "Set"
target_type = "Flag"
target_reg = "FLAG1"
target_value = false

[[instructions]]
type = "Prefill"
input = "TOKENID3"

[[instructions]]
type = "CopyMeta"
src = "META5"
dst = "META1"

[[instructions]]
type = "Sample"
tensor_reg = "TENSOR2"

[[instructions]]
type = "Decode"

[[instructions]]
type = "Output"

[[instructions]]
type = "Loop"

[[instructions.body]]
type = "BreakIf"

[[instructions.body]]
type = "Inference"
input_type = "Tokens"
input = "TOKENID2"

[[instructions.body]]
type = "Sample"
tensor_reg = "TENSOR2"

[[instructions.body]]
type = "Decode"

[[instructions.body]]
type = "Output"

[[instructions]]
type = "EndOutput"
```

**`programs/ml/relay.tmpl`**

```toml
[[instructions]]
type = "Loop"

[[instructions.body]]
type = "Receive"

[[instructions.body]]
type = "BreakIf"

[[instructions.body]]
type = "Inference"
input_type = "Tensor"
input = "TENSOR1"

[[instructions.body]]
type = "Send"

[[instructions]]
type = "SendEOF"
```

**`programs/ml/coordinator.tmpl`**

```toml
[[instructions]]
type = "Input"

[[instructions]]
type = "Encode"

[[instructions]]
type = "Set"
target_type = "Meta"
target_reg = "META2"
target_value = "$max_tokens"

[[instructions]]
type = "Set"
target_type = "Flag"
target_reg = "FLAG1"
target_value = false

[[instructions]]
type = "Prefill"
input = "TOKENID3"

[[instructions]]
type = "CopyMeta"
src = "META5"
dst = "META1"

[[instructions]]
type = "Send"

[[instructions]]
type = "Receive"

[[instructions]]
type = "Sample"
tensor_reg = "TENSOR1"

[[instructions]]
type = "Decode"

[[instructions]]
type = "Output"

[[instructions]]
type = "Loop"

[[instructions.body]]
type = "BreakIf"

[[instructions.body]]
type = "Inference"
input_type = "Tokens"
input = "TOKENID2"

[[instructions.body]]
type = "Send"

[[instructions.body]]
type = "Receive"

[[instructions.body]]
type = "Sample"
tensor_reg = "TENSOR1"

[[instructions.body]]
type = "Decode"

[[instructions.body]]
type = "Output"

[[instructions]]
type = "SendEOF"

[[instructions]]
type = "EndOutput"
```

### 13.2 ProgramSelector 实现

`Src/Orchestrator/program_selector.rs`

```rust
// 编译期嵌入：模板文件编译进二进制，无需运行时文件 IO，也无需分发 programs/ 目录。
// 运行时按需解析：仅在 select() / load_ml_program() 调用时做 TOML 反序列化。

const RUN_TMPL: &str = include_str!("../../../programs/orchestrator/Run.tmpl");
const RELAY_TMPL: &str = include_str!("../../../programs/orchestrator/Relay.tmpl");
const PIPELINE_TMPL: &str = include_str!("../../../programs/orchestrator/Pipeline.tmpl");
const SEND_TMPL: &str = include_str!("../../../programs/orchestrator/Send.tmpl");
const RECEIVE_TMPL: &str = include_str!("../../../programs/orchestrator/ReceiveFile.tmpl");
const ML_RUN_TMPL: &str = include_str!("../../../programs/ml/run.tmpl");
const ML_RELAY_TMPL: &str = include_str!("../../../programs/ml/relay.tmpl");
const ML_COORDINATOR_TMPL: &str = include_str!("../../../programs/ml/coordinator.tmpl");
```

**核心结构：**

| 组件 | 职责 |
|------|------|
| `SlotNameMap` | 编译期常量：槽位名字符串 → `SlotId` 映射表（`SLOT_MODEL` → `SlotId(1)` 等） |
| `OrchestratorTemplate` enum | TOML 反序列化中间表示（`Meta` + `Vec<RawInstruction>`） |
| `MLTemplate` enum | ML TOML 反序列化中间表示（`Vec<RawMLInstruction>`，含 `body` 嵌套） |
| `ProgramSelector::select()` | JobKind → 选模板 → 变量替换 → 产出 `TaskProgram` |
| `ProgramSelector::load_ml_program()` | mode → 选 ML 模板 → 变量替换 → 产出 `Vec<Instruction>` |

**`select()` 流程：**
1. 根据 `kind` 选择对应 `include_str!()` 常量
2. `toml::from_str::<OrchestratorTemplate>()` 反序列化
3. 遍历每条 `RawInstruction`，将 `$var_name` 替换为 `vars` 中的值
4. 将 `"SLOT_XXX"` 名称通过 `SlotNameMap` 查表转为 `SlotId`
5. 构造 `Vec<OrchestratorInstruction>`，包装为 `TaskProgram`

**`load_ml_program()` 流程：**
1. 根据 `mode`（"run"|"relay"|"coordinator"）选择 ML 模板
2. `toml::from_str::<MLTemplate>()` 反序列化
3. 遍历 `RawMLInstruction`，递归处理 `Loop.body` 嵌套
4. 将 ML 寄存器名（`TOKENID2`/`TENSOR1`/`META5` 等）通过 ML 常量查表转为实际值
5. 产出 `Vec<Instruction>`

### 13.3 集成改动清单

| 步骤 | 文件 | 改动内容 |
|------|------|---------|
| S1 | `programs/orchestrator/*.tmpl` (5 个) | 新建，内容见 13.1.1 |
| S2 | `programs/ml/*.tmpl` (3 个) | 新建，内容见 13.1.2 |
| S3 | `Src/Orchestrator/program_selector.rs` | 新建，内容见 13.2 |
| S4 | `Src/Orchestrator/mod.rs` | 添加 `pub mod program_selector;` |
| S5 | `Src/Orchestrator/core.rs` | `compiler: Arc<Compiler>` → `selector: Arc<ProgramSelector>` |
| S6 | `Src/Orchestrator/core/branch_user.rs` | 各 `compile_xxx()` → `selector.select(kind, vars)` |
| S7 | `Src/Orchestrator/core/branch_command.rs` | 如有 Compiler 调用，同样替换 |
| S8 | `Src/Orchestrator/core/branch_stream.rs` | `compile_receive_file()` → `selector.select(Receive, vars)` |
| S9 | `Src/Orchestrator/Orchestrator_VM/inference_handler.rs` | `Compiler::build_xxx_ml_program()` → `selector.load_ml_program()` |
| S10 | `Src/Orchestrator/compiler.rs` | 删除所有 `compile_xxx()` 和 `build_xxx_ml_program()` 方法，仅保留槽位常量定义 |
| S11 | `Src/Orchestrator/instruction.rs` | 标记 `compensation` 为 TODO，固定为空（不移除字段，保持兼容） |
| S12 | `cargo test` | 全量测试通过 |

## 14. Phase 3 — 清除旧 executor/instruction/slot 冗余

### 14.1 背景

Phase 1/2 完成后，Orchestrator_VM 和 ProgramSelector 已全面接管编排执行和程序生成，但以下旧模块仍留存：

| 旧模块 | 状态 | 替代者 |
|--------|------|--------|
| `executor/task_engine.rs` + 5 个 `handler_*.rs` | 死代码，仅 `executor/mod.rs` 中 `mod` 声明 | `Orchestrator_VM/` |
| `executor/*.md`（5 个设计/测试文档） | 旧设计文档 | — |
| `instruction.rs`（TaskInstruction/TaskProgram） | 仅被 `executor/mod.rs` 的 `convert_instruction()` 死函数引用 | `OrchestratorInstruction`（平铺枚举） |
| `slot.rs`（SlotId/SlotValue/SlotFile） | 4 处引用（compiler/branch_command/branch_stream/OVM::to_vm_slot） | `vm_base::SlotId` + `OrchestratorSlotValue` |
| `compiler.rs` | `compile_distribute()` 被 `branch_user.rs` 的 `DistributeModel` 命令调用；槽位常量可迁移 | `ProgramSelector::select()` + `resolve_slot()` 查表 |
| `executor/mod.rs`（JobExecutor） | 仍活跃，被 Core 各分支使用 | 移至 `core/job_executor.rs` |

### 14.2 目标

- 删除 `executor/` 目录
- 删除 `instruction.rs`
- 删除 `slot.rs`
- 删除 `compiler.rs`
- `DistributeModel` 命令改为占位符（返回 "not yet implemented"），`UserCommand` 枚举中保留以兼容前端
- 槽位常量迁移至 `program_selector.rs`
- `JobExecutor` 移至 `core/job_executor.rs`
- 所有槽位引用统一为 `vm_base::SlotId`（基础类型）和 `OrchestratorSlotValue`（领域类型）
- `Core.compiler` 字段移除

### 14.3 实施步骤

#### Step 1 — 迁移 slot.rs + compiler.rs 内容

| 步骤 | 文件 | 改动内容 |
|------|------|---------|
| S1a | `program_selector.rs` | 将 `compiler.rs` 中的槽位常量迁入，类型改为 `vm_base::SlotId` |
| S1b | `compiler.rs` | 删除整个文件 |
| S1c | `branch_user.rs` | `DistributeModel` 分支 → 返回 `reply.send(Err("not yet implemented".into()))`；移除 `self.compiler` 引用 |
| S1d | `branch_command.rs` | `SlotValue::TensorIo(...)` → `OrchestratorSlotValue::TensorIO(...)`；调整 `inject_slot` 调用 |
| S1e | `branch_stream.rs` | `SlotValue::Stream(...)` → `OrchestratorSlotValue::Stream(...)` |
| S1f | `executor/mod.rs` | `inject_slot()` 参数 `SlotValue` → `OrchestratorSlotValue`，移除 OLD→NEW 转换桥接 |
| S1g | `Orchestrator_VM/mod.rs` | 移除 `to_vm_slot()` |
| S1h | `core.rs` | 移除 `compiler: Arc<Compiler>` 字段及构造参数 |
| S1i | `orchestrator/mod.rs` | 移除 `pub mod compiler;` `pub mod slot;` |
| S1j | `cargo test` | 全量通过 |

#### Step 2 — 创建 job_executor.rs + 移除冗余文件

| 步骤 | 文件 | 改动内容 |
|------|------|---------|
| S2a | `core/job_executor.rs`（新建） | 从 `executor/mod.rs` 迁移 JobExecutor + ExitReason + convert_basic_slot（移除 convert_instruction、旧 handler mod 声明、TaskProgram pub use、对 `super::slot` 的引用） |
| S2b | `core.rs` | `use super::executor::JobExecutor` → `use job_executor::JobExecutor`；添加子模块声明 |
| S2c | `branch_user.rs` / `branch_command.rs` / `branch_stream.rs` | `use crate::orchestrator::executor::JobExecutor` → `use crate::orchestrator::core::job_executor::JobExecutor`（或通过 re-export） |
| S2d | `orchestrator/mod.rs` | 移除 `pub mod executor;` `pub mod instruction;` |
| S2e | 删除文件 | `executor/` 目录（task_engine.rs + 5 handler + 5 md）、`instruction.rs`、`slot.rs` |
| S2f | `cargo test` | 全量通过 |

### 14.4 迁移前后对比

| 组件 | 迁移前 | 迁移后 |
|------|--------|--------|
| 基础槽位类型 | `orchestrator::slot::SlotId` | `vm_base::SlotId` |
| 领域槽位类型 | `orchestrator::slot::SlotValue`（混合） | `OrchestratorSlotValue` |
| 槽位常量位置 | `compiler.rs` | `program_selector.rs` |
| Program 生成 | `Compiler::compile_xxx()` + `compile_distribute()` | `ProgramSelector::select()`（Distribute 占位） |
| inject_slot 参数 | `slot::SlotValue` | `OrchestratorSlotValue` |
| JobExecutor 位置 | `executor/mod.rs` | `core/job_executor.rs` |
| Core 字段 | `compiler: Arc<Compiler>` + `selector: Arc<ProgramSelector>` | `selector: Arc<ProgramSelector>` |
| Orchestrator 子模块 | 9 个 | 6 个（core, job, command, inference_id, program_selector, orchestrator_vm） |

