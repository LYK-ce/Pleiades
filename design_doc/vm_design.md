# Vm 模块设计方案

## 1. 背景

当前 ML Engine 和 Orchestrator 各自实现了一套微型的指令执行引擎：

| | ML Engine | Orchestrator |
|---|---|---|
| 寄存器 | `Register_File` (固定列式) | `SlotFile` (HashMap) |
| 指令枚举 | `Instruction` (含 Loop/BreakIf 等 18 条) | `TaskInstruction` (15 条) |
| 执行方式 | `Execute()` 一次性 | `step()` 逐步泵 |
| 控制流 | `Loop` + `BreakIf` | `JumpIf` + label |

两者共享相同的执行模式（fetch → decode → execute → write back），且存在 5 条行为完全一致的基础指令（Const/Move/Add/Jump/JumpIf），但当前各自实现。另外 ML Engine 的 `Loop` + `BreakIf` 可拆解为 `Jump`/`JumpIf` 组合，`Set`/`CopyMeta` 可被 `Const`/`Move`/`Add` 替代。

## 2. 目标

- 提取共享的**槽位系统**和**基础指令 handler**到 `Vm/` 模块
- ML Engine 和 Orchestrator 各自持有领域指令，在自己的执行循环中 match
- Vm 不依赖任何领域模块

## 3. 模块结构

```
Src/
└── Vm/
    ├── mod.rs        # pub use slot::*, pub use vm::*
    ├── slot.rs       # SlotId, SlotValue, SlotFile
    └── vm.rs         # Vm { ip, slots, labels } + 基础 handler
```

## 4. 槽位系统 (slot.rs)

### 4.1 SlotId

```rust
pub struct SlotId(pub u32);
```

### 4.2 ConstValue — 常量值

```rust
#[derive(Debug, Clone)]
pub enum ConstValue {
    Nil,
    Bool(bool),
    U64(u64),
    F64(f64),
    String(String),
    PathBuf(PathBuf),
    Error(String),
}
```

`ConstValue` 和 `SlotValue` 的 From 转换放在 Vm/slot.rs 中。

### 4.3 SlotValue

只包含基础类型。领域类型由各模块在自身结构体上管理，不经过 Vm：

```rust
pub enum SlotValue {
    Nil,
    Bool(bool),
    U64(u64),
    F64(f64),
    String(String),
    PathBuf(PathBuf),
    Error(String),
}
```

### 4.4 SlotFile

基于 `HashMap<SlotId, SlotValue>`，提供类型化存取方法：

```rust
impl SlotFile {
    pub fn new() -> Self;
    pub fn set(&mut self, slot: SlotId, value: SlotValue);
    pub fn get(&self, slot: SlotId) -> Option<&SlotValue>;
    pub fn take(&mut self, slot: SlotId) -> Option<SlotValue>;
    pub fn remove(&mut self, slot: SlotId) -> Option<SlotValue>;
    pub fn get_bool(&self, slot: SlotId) -> Result<bool, String>;
    pub fn get_u64(&self, slot: SlotId) -> Result<u64, String>;
    pub fn get_f64(&self, slot: SlotId) -> Result<f64, String>;
    pub fn get_string(&self, slot: SlotId) -> Result<&String, String>;
    pub fn get_path(&self, slot: SlotId) -> Result<&PathBuf, String>;
    pub fn get_error(&self, slot: SlotId) -> Result<&String, String>;
}
```

领域类型通过组合挂载：

```rust
// ML Engine
struct MlSession {
    vm: Vm,
    tensor_slots: [Option<Tensor>; 4],   // 领域扩展字段
    backend: Model,
}

// Orchestrator
struct TaskEngine {
    vm: Vm,
    io_handles: HashMap<SlotId, IoHandle>,      // 领域扩展字段
    session_handles: HashMap<SlotId, SessionHandle>,
    pipeline_plan: Option<PipelinePlan>,
}
```

### 4.5 槽位编号约定

Vm 模块不规定槽位编号。SlotFile 只是一个通用容器，SlotId(0..N) 的语义由各领域模块自己定义：

- ML Engine 内部定义 TEXT1/TEXT2/.../TOKENID1/.../META8 等常量
- Orchestrator TaskEngine 内部定义自己的槽位常量

两边互不干扰，通过不同的 `SlotId` 值域自然隔离。 |

## 5. Vm 结构 (vm.rs)

### 5.1 结构体

```rust
pub struct Vm {
    pub ip: usize,
    pub slots: SlotFile,
}
```

### 5.2 基础指令 handler

五条公用指令，handler 实现在 Vm 中：

```rust
impl Vm {
    pub fn new() -> Self;

    /// Const: 常量写入槽位
    pub fn handle_const(&mut self, value: ConstValue, dst: SlotId) -> StepResult;

    /// Move: 槽位间拷贝（take 语义）
    pub fn handle_move(&mut self, src: SlotId, dst: SlotId) -> StepResult;

    /// Add: dst += value（仅 F64 槽位，替换 ML Engine 的 Increment_Meta）
    pub fn handle_add(&mut self, dst: SlotId, value: f64) -> StepResult;

    /// Jump: 无条件跳转到 target（IP 索引）
    pub fn handle_jump(&mut self, target: usize) -> StepResult;

    /// JumpIf: 若 condition 槽位为 true 则跳转到 target
    pub fn handle_jump_if(&mut self, condition: SlotId, target: usize) -> StepResult;
}
```

标签解析由编译器在加载程序前完成，Vm 不持有 `labels` HashMap。

`Set_Target` 和 `CopyMeta` 从 ML Engine 移除，由 `Const`/`Move`/`Add` 替代。

### 5.3 StepResult

```rust
pub enum StepResult {
    Continue,
    Ready,
    Done,
    Abort(String),
}
```

## 6. 使用方式

### 6.1 ML Engine

```rust
struct MlSession {
    vm: Vm,
    backend: InferenceBackend,
    io_handle: IoHandle,
    tensor_io: Option<TensorIOEndpoint>,
}

fn execute(&mut self, program: &[MlInstruction]) -> Result<()> {
    self.vm.ip = 0;
    loop {
        if self.vm.ip >= program.len() { break; }
        let inst = &program[self.vm.ip];
        self.vm.ip += 1;
        match inst {
            // 公共指令 → 委托给 Vm
            MlInstruction::Const { v, d }     => { self.vm.handle_const(v.clone(), *d); }
            MlInstruction::Move { s, d }      => { self.vm.handle_move(*s, *d); }
            MlInstruction::Add { dst, value } => { self.vm.handle_add(*dst, *value); }
            MlInstruction::Jump { target }    => { self.vm.handle_jump(*target); }
            MlInstruction::JumpIf { c, t }    => { self.vm.handle_jump_if(*c, *t); }
            // 领域指令
            MlInstruction::Input => self.handle_input()?,
            MlInstruction::Encode => self.handle_encode()?,
            MlInstruction::Decode => self.handle_decode()?,
            MlInstruction::Prefill { input } => self.handle_prefill(*input)?,
            MlInstruction::Inference { input } => self.handle_inference(input)?,
            MlInstruction::Sample { tensor_slot } => self.handle_sample(*tensor_slot)?,
            MlInstruction::Output => self.handle_output()?,
            MlInstruction::EndOutput => self.handle_end_output()?,
            MlInstruction::Send => self.handle_send()?,
            MlInstruction::Receive => self.handle_receive()?,
            MlInstruction::SendEOF => self.handle_send_eof()?,
        }
    }
    Ok(())
}
```

ML Engine 移除的指令及替代：

| 原指令 | 替代方案 |
|--------|----------|
| `Set { target }` | `Const` (写入) + `Add` (增量) |
| `CopyMeta { src, dst }` | `Move(src, dst)` |
| `Loop { body }` | label + `JumpIf`/`JumpNot` 组合 |
| `BreakIf` | 内联在 Loop body 末尾的 `JumpIf` |

### 6.2 Orchestrator (TaskEngine)

```rust
struct TaskEngine {
    vm: Vm,
    capabilities: Arc<Capabilities>,
}

async fn step(&mut self) -> StepResult {
    let inst = self.program[self.vm.ip].clone();
    self.vm.ip += 1;
    match inst {
        // 公共指令 → 委托给 Vm
        TaskInstruction::Const { v, d }     => self.vm.handle_const(v, d),
        TaskInstruction::Move { s, d }      => self.vm.handle_move(s, d),
        TaskInstruction::Add { dst, value } => self.vm.handle_add(dst, value),
        TaskInstruction::Jump { target }    => self.vm.handle_jump(target),
        TaskInstruction::JumpIf { c, t }    => self.vm.handle_jump_if(c, t),
        // 领域指令（含错误终止——handler 直接返回 StepResult::Abort）
        TaskInstruction::CreateSession { .. } => self.handle_create_session(..).await,
        TaskInstruction::SendFile { .. }      => self.handle_send_file(..).await,
        // ...
    }
}
```

## 7. 依赖关系

```
Vm/     → std only（零领域依赖）

ML Engine  → Vm + candle + LLM_IO + Tensor_IO
Orchestrator → Vm + 所有子系统
```

## 8. 实施步骤

### Phase 1 — 仅实现 Vm 模块（本次）

1. 创建 `Src/Vm/` 目录
2. 实现 `slot.rs`（SlotId / ConstValue / SlotValue / SlotFile + 测试）
3. 实现 `vm.rs`（Vm 结构体 + 5 个基础 handler + 测试）
4. 实现 `mod.rs`（pub use 导出）
5. 在 `lib.rs` 中注册 `pub mod vm;`
6. 运行 `cargo test --lib vm` 确保通过

### Phase 2 — 后续

- 改造 ML Engine：`Register_File` → `Vm`，指令集迁移
- 改造 Orchestrator TaskEngine：`SlotFile` → `Vm`

现有模块不动，Phase 2 待定。

## 9. 讨论项

- [x] 类型擦除方案：放弃 Erased。改为组合方案——领域类型挂载在各结构体字段上，SlotFile 只放基础类型
- [x] ML Engine 原有 Loop/BreakIf 控制流：拆解为 Jump/JumpIf 组合
- [x] `ConstValue` 枚举：移入 Vm/slot.rs
- [x] Abort 指令：移除，不由 Vm 统一
