# ML_VM 设计方案

## 1. 背景

当前 ML Engine 使用 `Register_File`（固定大小的寄存器组）和 `Loop { body }` 嵌套控制流。Vm_Base 模块已提供通用的槽位系统和 5 条公共指令的 handler，Orchestrator_VM 已率先完成基于 Vm_Base 的改造。

ML Engine 将被改造为基于 Vm_Base 的 `ML_VM`：
- `Register_File` → `Vm_Base::SlotFile`（基础类型） + `MlSlots`（领域类型）
- `Loop { body }` / `BreakIf` → `Jump` / `JumpIf`（VM 扁平控制流）
- `Set` / `CopyMeta` → `Const` / `Move`（Delegate 给 Vm_Base）

## 2. 目标

- 用 `Vm_Base::Vm` 替代 Session 内的 IP 管理和基础类型槽位
- 领域类型（`Vec<u32>`、`Candle Tensor`）放入独立的 `MlSlots`
- 指令枚举平铺，公共指令委托给 `Vm`
- 用 `Jump`/`JumpIf` 替代 `Loop`/`BreakIf`，实现扁平指令序列
- OS 线程模型保留不变

## 3. 目录结构

```
Src/ML_Engine/ML_VM/
├── mod.rs           # pub use + ML_VM 结构体
├── instruction.rs   # MlInstruction 枚举（平铺，含公共 5 条）
├── slots.rs         # MlSlotValue 枚举 + MlSlots
└── engine.rs        # step() / execute() 执行循环

programs/ml/
├── run.tmpl         # 更新：寄存器名 → 槽位名，Loop → Jump/JumpIf
├── relay.tmpl       # 更新：同上
└── coordinator.tmpl # 更新：同上
```

## 4. 槽位系统 (slots.rs)

### 4.1 设计原则

- 基础类型（`String`/`bool`/`f64`）→ `Vm_Base::SlotFile`（`vm.slots`）
- 领域类型（`Vec<u32>`/`Candle Tensor`）→ `MlSlots`
- 每个 `SlotId` 全局唯一，类型唯一

### 4.2 槽位编号约定

ML 槽位从 1000 开始，与 Orchestrator 槽位（0~303）不冲突：

| 类别 | 原寄存器 | 存储位置 | 槽位 ID | 类型 |
|------|---------|----------|---------|------|
| 文本 | TEXT1 | `vm.slots` | 1000 | `SlotValue::String` |
| | TEXT2 | `vm.slots` | 1001 | `SlotValue::String` |
| | TEXT3 | `vm.slots` | 1002 | `SlotValue::String` |
| | TEXT4 | `vm.slots` | 1003 | `SlotValue::String` |
| Token | TOKENID1 | `ml_slots` | 1010 | `MlSlotValue::TokenIds` |
| | TOKENID2 | `ml_slots` | 1011 | `MlSlotValue::TokenIds` |
| | TOKENID3 | `ml_slots` | 1012 | `MlSlotValue::TokenIds` |
| | TOKENID4 | `ml_slots` | 1013 | `MlSlotValue::TokenIds` |
| Tensor | TENSOR1 | `ml_slots` | 1020 | `MlSlotValue::Tensor` |
| | TENSOR2 | `ml_slots` | 1021 | `MlSlotValue::Tensor` |
| | TENSOR3 | `ml_slots` | 1022 | `MlSlotValue::Tensor` |
| | TENSOR4 | `ml_slots` | 1023 | `MlSlotValue::Tensor` |
| Flag | FLAG1 | `vm.slots` | 1030 | `SlotValue::Bool` |
| | FLAG2~4 | `vm.slots` | 1031~1033 | `SlotValue::Bool` |
| Meta | META1~META8 | `vm.slots` | 1040~1047 | `SlotValue::F64` |

```rust
// ─── 约定槽位常量 ─────────────────────────────────────────────

// Text
pub const SLOT_TEXT1: SlotId = SlotId(1000);
pub const SLOT_TEXT2: SlotId = SlotId(1001);
pub const SLOT_TEXT3: SlotId = SlotId(1002);
pub const SLOT_TEXT4: SlotId = SlotId(1003);

// Token IDs
pub const SLOT_TOKENS1: SlotId = SlotId(1010);
pub const SLOT_TOKENS2: SlotId = SlotId(1011);
pub const SLOT_TOKENS3: SlotId = SlotId(1012);
pub const SLOT_TOKENS4: SlotId = SlotId(1013);

// Tensor
pub const SLOT_TENSOR1: SlotId = SlotId(1020);
pub const SLOT_TENSOR2: SlotId = SlotId(1021);
pub const SLOT_TENSOR3: SlotId = SlotId(1022);
pub const SLOT_TENSOR4: SlotId = SlotId(1023);

// Flag
pub const SLOT_FLAG1: SlotId = SlotId(1030);
pub const SLOT_FLAG2: SlotId = SlotId(1031);
pub const SLOT_FLAG3: SlotId = SlotId(1032);
pub const SLOT_FLAG4: SlotId = SlotId(1033);

// Meta
pub const SLOT_META1: SlotId = SlotId(1040);
pub const SLOT_META2: SlotId = SlotId(1041);
pub const SLOT_META3: SlotId = SlotId(1042);
pub const SLOT_META4: SlotId = SlotId(1043);
pub const SLOT_META5: SlotId = SlotId(1044);
pub const SLOT_META6: SlotId = SlotId(1045);
pub const SLOT_META7: SlotId = SlotId(1046);
pub const SLOT_META8: SlotId = SlotId(1047);
```

### 4.3 MlSlotValue

```rust
pub enum MlSlotValue {
    TokenIds(Vec<u32>),
    Tensor(Tensor),          // Candle Tensor, Arc<Storage> 内部, clone 零拷贝
}
```

### 4.4 MlSlots

与 `OrchestratorSlots` 同构，`HashMap<SlotId, MlSlotValue>`：

```rust
pub struct MlSlots {
    slots: HashMap<SlotId, MlSlotValue>,
}

impl MlSlots {
    pub fn new() -> Self;
    pub fn set(&mut self, slot: SlotId, value: MlSlotValue);
    pub fn take(&mut self, slot: SlotId) -> Option<MlSlotValue>;

    // 类型化访问
    pub fn take_token_ids(&mut self, slot: SlotId) -> Option<Vec<u32>>;
    pub fn get_token_ids(&self, slot: SlotId) -> Option<&[u32]>;
    pub fn set_token_ids(&mut self, slot: SlotId, ids: Vec<u32>);
    pub fn append_token_id(&mut self, slot: SlotId, id: u32);      // 追加单个 token
    pub fn clear_token_ids(&mut self, slot: SlotId);

    pub fn take_tensor(&mut self, slot: SlotId) -> Option<Tensor>;
    pub fn get_tensor(&self, slot: SlotId) -> Option<&Tensor>;
    pub fn set_tensor(&mut self, slot: SlotId, tensor: Tensor);
    pub fn clear_tensor(&mut self, slot: SlotId);
}
```

## 5. 指令集 (instruction.rs)

### 5.1 MlInstruction 枚举（平铺）

原 `Set`、`CopyMeta`、`Loop`、`BreakIf` 被移除，由 Vm_Base 的 `Const`/`Move`/`Jump`/`JumpIf` 替代：

```rust
pub enum MlInstruction {
    // ─── 公共指令（委托给 Vm）──────────────────────
    Const { value: ConstValue, dst: SlotId },
    Move  { src: SlotId, dst: SlotId },
    Add   { dst: SlotId, delta: f64 },
    Jump  { target: usize },
    JumpIf { condition: SlotId, target: usize },

    // ─── 数据输入 ──────────────────────────────────
    /// io_handle.input_rx → SLOT_TEXT1
    Input,

    // ─── 编解码 ────────────────────────────────────
    /// SLOT_TEXT1 → tokenize → SLOT_TOKENS3, 清空 SLOT_TOKENS1
    Encode,
    /// SLOT_TOKENS2 → detokenize → SLOT_TEXT2
    Decode,

    // ─── 推理 ──────────────────────────────────────
    /// 批量前向: SLOT_TOKENS{input} → SLOT_TENSOR2, SLOT_META5=prompt_len
    Prefill { input: SlotId },
    /// 单步前向: 根据 input_type 从 SlotId 读取 → SLOT_TENSOR2
    Inference { input_type: InferenceInputType, input: SlotId },

    // ─── 采样 ──────────────────────────────────────
    /// tensor_slot → sampling → SLOT_TOKENS2, SLOT_TOKENS1追加, META2-=1, META4+=1, EOS/limit→FLAG1
    Sample { tensor_slot: SlotId },

    // ─── Control I/O ───────────────────────────────
    /// SLOT_TEXT2 → io_handle.output_tx
    Output,
    /// 输出结束（IoHandle Drop 管理通道关闭，此处 noop）
    EndOutput,

    // ─── 网络 I/O ──────────────────────────────────
    /// SLOT_TENSOR2 → 序列化 → tensor_io.Send(META1)
    Send,
    /// tensor_io.Receive() → SLOT_TENSOR1; EOF→FLAG1=true, offset→META6
    Receive,
    /// 发送 EOF 哨兵帧 (offset=u64::MAX)
    SendEOF,
}
```

### 5.2 InferenceInputType

原 `Inference_Input` 枚举中的寄存器引用改为 `SlotId`：

```rust
#[derive(Debug, Clone)]
pub enum InferenceInputType {
    /// Token IDs → embedding → forward
    Tokens,
    /// Tensor → 直接 forward
    Tensor,
}
```

### 5.3 指令移除对照表

| 原指令 | 替代指令 | 说明 |
|--------|----------|------|
| `Set(Meta(META2, 120.0))` | `Const { value: F64(120.0), dst: SLOT_META2 }` | 常量写入 |
| `Set(Flag(FLAG1, false))` | `Const { value: Bool(false), dst: SLOT_FLAG1 }` | 标志初始化 |
| `CopyMeta(META5, META1)` | `Move { src: SLOT_META5, dst: SLOT_META1 }` | 寄存器间拷贝 |
| `Loop { body }` | `JumpIf` + body + `Jump` 组合 | 扁平化控制流 |
| `BreakIf` | body 末尾的 `JumpIf(FLAG1, after_loop)` | 条件退出检查内联 |

## 6. Loop → Jump/JumpIf 扁平化

### 6.1 转换模式

原始（嵌套）：
```
Loop [
    0: BreakIf            // if FLAG1 → break
    1: Inference(...)     // body
    2: Sample(...)
    3: Output
]
5: after_loop
```

转换后（扁平）：
```
0: JumpIf(SLOT_FLAG1, 5)  // if FLAG1 → jump to after_loop
1: Inference(...)         // body
2: Sample(...)
3: Output
4: Jump(0)                // unconditional loop back
5: after_loop
```

### 6.2 三种程序的 Loop 展开

#### Run (单机推理)

```
// Prefill 后的第一个 generate 步骤
Sample(TENSOR2)          // Prefill 结果采样
Decode                   // 解码
Output                   // 输出

// ── Loop: 0=check, 1~4=body, 5=jump_back ──
JumpIf(SLOT_FLAG1, after)
Inference(Tokens, TOKENS2)
Sample(TENSOR2)
Decode
Output
Jump(check)
after:
EndOutput
```

#### Coordinator (分布式协调者)

```
// Prefill 后的第一次 Send/Receive/Sample
Send                     // 发送 Prefill hidden state
Receive                  // 接收下游 Worker 结果
Sample(TENSOR1)          // 采样
Decode
Output

// ── Loop ──
JumpIf(SLOT_FLAG1, after)
Inference(Tokens, TOKENS2)
Send
Receive
Sample(TENSOR1)
Decode
Output
Jump(check)
after:
SendEOF
EndOutput
```

#### Relay (Worker)

```
// ── Loop ──
JumpIf(SLOT_FLAG1, after)
Receive                  // 阻塞接收 → TENSOR1; EOF→FLAG1=true
JumpIf(SLOT_FLAG1, after)// Receive 收到 EOF 时立即退出
Inference(Tensor, TENSOR1)
Send                     // 发送 TENSOR2
Jump(check)
after:
SendEOF
```

> **注意**：Relay 的 Loop 中，`Receive` 和 `Inference` 之间需要额外的 `JumpIf`（Receive 收到 EOF 时会设置 FLAG1），避免对空 Tensor 做 Inference。

## 7. 执行引擎 (engine.rs)

### 7.1 ML_VM 结构体

```rust
pub struct ML_VM {
    pub vm: Vm,                              // Vm_Base::Vm（IP + SlotFile）
    pub ml_slots: MlSlots,                   // 领域数据（TokenIds, Tensor）
    pub backend: Inference_Backend,          // 模型后端（GGUF）
    pub io_handle: IoHandle,                 // 文本 I/O
    pub tensor_io: Option<Tensor_IO_Endpoint>, // 网络张量 I/O
    program: Vec<MlInstruction>,             // 当前执行的指令序列
}
```

### 7.2 step() 分发

```rust
impl ML_VM {
    pub fn new(
        backend: Inference_Backend,
        io_handle: IoHandle,
        tensor_io: Option<Tensor_IO_Endpoint>,
    ) -> Self;

    pub fn load(&mut self, program: Vec<MlInstruction>) {
        self.program = program;
        self.vm.ip = 0;
    }

    /// 执行单条指令，返回 StepResult。
    /// 由 execute() 循环调用，或由外部逐步调用。
    pub fn step(&mut self, cancel_flag: &AtomicBool) -> StepResult {
        if cancel_flag.load(Ordering::Relaxed) {
            return StepResult::Abort("cancelled".into());
        }
        if self.vm.ip >= self.program.len() {
            return StepResult::Done;
        }
        let inst = &self.program[self.vm.ip];
        self.vm.ip += 1;
        match inst {
            // 公共指令 → 委托给 Vm
            MlInstruction::Const { value, dst } => self.vm.handle_const(value.clone(), *dst),
            MlInstruction::Move { src, dst }    => self.vm.handle_move(*src, *dst),
            MlInstruction::Add { dst, delta }   => self.vm.handle_add(*dst, *delta),
            MlInstruction::Jump { target }      => self.vm.handle_jump(*target),
            MlInstruction::JumpIf { condition, target } => self.vm.handle_jump_if(*condition, *target),

            // 领域指令 → 各自 handler
            MlInstruction::Input              => self.handle_input(),
            MlInstruction::Encode             => self.handle_encode(),
            MlInstruction::Decode             => self.handle_decode(),
            MlInstruction::Prefill { input }  => self.handle_prefill(*input),
            MlInstruction::Inference { input_type, input } => self.handle_inference(*input_type, *input),
            MlInstruction::Sample { tensor_slot } => self.handle_sample(*tensor_slot),
            MlInstruction::Output             => self.handle_output(),
            MlInstruction::EndOutput          => self.handle_end_output(),
            MlInstruction::Send               => self.handle_send(),
            MlInstruction::Receive            => self.handle_receive(),
            MlInstruction::SendEOF            => self.handle_send_eof(),
        }
    }

    /// 执行全部指令，直到 Done/Abort/取消。
    pub fn execute(&mut self, program: Vec<MlInstruction>, cancel_flag: &AtomicBool) -> Result<(), String> {
        self.load(program);
        loop {
            match self.step(cancel_flag) {
                StepResult::Continue => continue,
                StepResult::Done => return Ok(()),
                StepResult::Abort(reason) => return Err(reason),
                StepResult::Ready => continue,
            }
        }
    }
}
```

### 7.3 Handler 清单

| Handler | 对应指令 | 依赖资源 | 说明 |
|---------|---------|---------|------|
| `handle_input` | Input | `io_handle.input_rx` | 阻塞读取 → SLOT_TEXT1 |
| `handle_encode` | Encode | `backend` (tokenizer) | SLOT_TEXT1 → tokenize → SLOT_TOKENS3 |
| `handle_decode` | Decode | `backend` (tokenizer) | SLOT_TOKENS2 → detokenize → SLOT_TEXT2 |
| `handle_prefill` | Prefill | `backend` (model) | SLOT_TOKENS[input] → forward(offset=0) → SLOT_TENSOR2, META5=prompt_len |
| `handle_inference` | Inference | `backend` (model) | 根据 input_type 读取 → forward → SLOT_TENSOR2, META1+=inc |
| `handle_sample` | Sample | `backend` (eos_token_id) | logits → sampling → SLOT_TOKENS2, META2/4 维护, FLAG1=EOS/limit |
| `handle_output` | Output | `io_handle.output_tx` | SLOT_TEXT2 → 发送 |
| `handle_end_output` | EndOutput | — | noop |
| `handle_send` | Send | `tensor_io` | SLOT_TENSOR2 → 序列化 → tensor_io.Send(META1) |
| `handle_receive` | Receive | `tensor_io` | tensor_io.Receive() → SLOT_TENSOR1; EOF→FLAG1=true |
| `handle_send_eof` | SendEOF | `tensor_io` | 发送 EOF 哨兵帧 |

### 7.4 与原 Execute() 的区别

| 原 Execute | 新 ML_VM |
|---|---|
| `fn Execute(program, session, params, cancel_flag) -> Result<Pipeline_Result>` | `fn execute(program, cancel_flag) -> Result<(), String>` |
| 参数通过 params 结构体传入，Register_File 在 session 中 | 参数通过 vm.slots 传入（META2=$max_tokens 等） |
| 结果从 Register_File 提取为 Pipeline_Result | 执行完成后调用方从 vm.slots + ml_slots 提取结果 |
| 递归嵌套（Loop { body }） | 扁平 IP 跳转（Jump/JumpIf） |
| 4 个辅助函数（Encode_With_Backend 等） | 内联为 handler 方法 |
| `Set_Error_Flags(FLAG4, FLAG1)` | `handle_const(Bool(true), FLAG4)` + `handle_const(Bool(true), FLAG1)` |

## 8. 模板文件更新

### 8.1 run.tmpl（单机推理）

```toml
[[instructions]]
type = "Input"

[[instructions]]
type = "Encode"

[[instructions]]
type = "Const"
value_type = "F64"
value = "$max_tokens"
dst = "META2"

[[instructions]]
type = "Const"
value_type = "Bool"
value = false
dst = "FLAG1"

[[instructions]]
type = "Prefill"
input = "TOKENS3"

[[instructions]]
type = "Move"
src = "META5"
dst = "META1"

[[instructions]]
type = "Sample"
tensor_slot = "TENSOR2"

[[instructions]]
type = "Decode"

[[instructions]]
type = "Output"

# ── 生成循环 ──
[[instructions]]
type = "JumpIf"
condition = "FLAG1"
target = 16

[[instructions]]
type = "Inference"
input_type = "Tokens"
input = "TOKENS2"

[[instructions]]
type = "Sample"
tensor_slot = "TENSOR2"

[[instructions]]
type = "Decode"

[[instructions]]
type = "Output"

[[instructions]]
type = "Jump"
target = 9

[[instructions]]
type = "EndOutput"
```

### 8.2 relay.tmpl（Worker）

```toml
# ── 接收-推理-发送 循环 ──
[[instructions]]
type = "JumpIf"
condition = "FLAG1"
target = 8

[[instructions]]
type = "Receive"

[[instructions]]
type = "JumpIf"
condition = "FLAG1"
target = 8

[[instructions]]
type = "Inference"
input_type = "Tensor"
input = "TENSOR1"

[[instructions]]
type = "Send"

[[instructions]]
type = "Jump"
target = 0

[[instructions]]
type = "SendEOF"
```

### 8.3 coordinator.tmpl（分布式协调者）

```toml
[[instructions]]
type = "Input"

[[instructions]]
type = "Encode"

[[instructions]]
type = "Const"
value_type = "F64"
value = "$max_tokens"
dst = "META2"

[[instructions]]
type = "Const"
value_type = "Bool"
value = false
dst = "FLAG1"

[[instructions]]
type = "Prefill"
input = "TOKENS3"

[[instructions]]
type = "Move"
src = "META5"
dst = "META1"

[[instructions]]
type = "Send"

[[instructions]]
type = "Receive"

[[instructions]]
type = "Sample"
tensor_slot = "TENSOR1"

[[instructions]]
type = "Decode"

[[instructions]]
type = "Output"

# ── 生成循环 ──
[[instructions]]
type = "JumpIf"
condition = "FLAG1"
target = 19

[[instructions]]
type = "Inference"
input_type = "Tokens"
input = "TOKENS2"

[[instructions]]
type = "Send"

[[instructions]]
type = "Receive"

[[instructions]]
type = "Sample"
tensor_slot = "TENSOR1"

[[instructions]]
type = "Decode"

[[instructions]]
type = "Output"

[[instructions]]
type = "Jump"
target = 12

[[instructions]]
type = "SendEOF"

[[instructions]]
type = "EndOutput"
```

## 9. 依赖关系

```
Vm_Base/         → std only

ML_VM/           → Vm_Base + candle_core + GGUF + LLM_IO + Tensor_IO
```

## 10. 与原 ml_thread_engine 的对应关系

| 原 ml_thread_engine | 新 ML_VM |
|---|---|
| `Session.id/backend/io_handle/tensor_io` | `ML_VM.backend/io_handle/tensor_io` |
| `Session.register: Register_File` | `ML_VM.vm.slots: SlotFile` + `ML_VM.ml_slots: MlSlots` |
| `Execute()` 函数 | `ML_VM.execute()` 方法 |
| `Execute_Instruction()` match 分支 | `ML_VM.step()` 中的 handler 调用 |
| `Execution_Context` (params/eos/cancel/should_break/rng) | `vm.slots`（参数、EOS）+ `step()` 参数（cancel_flag）+ IP 跳转（替代 should_break） |
| `Session_Thread` 指令泵循环 | 外层循环不变，内部改用 `ML_VM.execute()` |
| `Loop { body }` 递归执行 | flat 指令序列中的 `JumpIf`/`Jump` 组合 |

## 11. 实施计划

### Phase 1 — 创建 ML_VM 模块（不动现有引擎）

1. **创建目录** `Src/ML_Engine/ML_VM/`
2. **实现 `slots.rs`** — `MlSlotValue` 枚举 + `MlSlots` + 槽位常量
3. **实现 `instruction.rs`** — `MlInstruction` 枚举（平铺，含公共 5 条 + 领域 11 条）+ `InferenceInputType`
4. **实现 `engine.rs`** — `ML_VM` 结构体 + handler 方法 + `step()` + `execute()`，处理器逻辑从原 `ml_thread_engine::Execute_Instruction` 迁移（改为读写槽位而非寄存器）
5. **实现 `mod.rs`** — pub use 导出
6. **在 `ML_Engine/mod.rs` 中注册子模块** — `pub mod ml_vm;`
7. **编写单元测试** — 各 handler 独立测试 + Loop 模拟测试 + 三种 program 逐指令执行测试
8. **`cargo test --lib ml_vm`** 验证

### Phase 2 — 更新程序模板 + ProgramSelector

9. **更新 `programs/ml/run.tmpl`** — Loop→Jump/JumpIf, 寄存器名→槽位名
10. **更新 `programs/ml/relay.tmpl`** — 同上
11. **更新 `programs/ml/coordinator.tmpl`** — 同上
12. **更新 `program_selector.rs`** — `load_ml_program()` 返回 `Vec<MlInstruction>` 替代 `Vec<Instruction>`，更新 `convert_ml_instruction()` 和 TOML 解析
13. **`cargo test --lib program_selector`** 验证模板正确加载

### Phase 3 — ML_Engine_Capability 层接入

14. **新增 `load_ml_program_vm()` 或双轨函数** — `ML_Engine_Capability` 支持 `Run_Program_VM(program: Vec<MlInstruction>, ...)`
15. **在 `Session_Thread` 中支持 ML_VM 模式** — 新增 `Session_Command::Run_Program_VM`，调用 `ML_VM.execute()`
16. **保留旧 `Run_Program`** — 原 `Vec<Instruction>` 路径暂时保留，双轨并行
17. **Orchestrator_VM 切换调用** — `handle_run_program` 改为使用 `Run_Program_VM`

### Phase 4 — 清除旧引擎（后续）

18. 删除 `ml_thread_register.rs`（Register_File 及相关类型）
19. 删除 `ml_thread_engine_instruction.rs` 中不再使用的 `Set_Target`、原 `Instruction` 枚举
20. 删除 `ml_thread_engine.rs` 中 `Execute()`/`Execute_Instruction`/辅助函数
21. 删除 `program_selector.rs` 中的旧 `load_ml_program()` 和旧 TOML 转换逻辑
22. `cargo test` 全量通过

### Phase 5 — 原有 TOML 模板清理（后续）

23. 删除 `programs/ml/` 目录中的旧模板文件内容
24. 或保留备查

## 12. 不去掉的内容

- 现有 `ml_thread_engine.rs`（`Session`/`Session_Handle`/`Session_Thread`）不动，Phase 4 再清理
- `ML_Engine_Service` 保持现有结构不变
- `GGUF_Model`/`GGUF_Model_Inference` 等后端保持不变
- `LLM_IO`/`Tensor_IO` 接口保持不变
- `Pipeline_Params`/`Pipeline_Result`/`Model_Info` 保留，由调用方在 VM 执行前后构造/提取

## 13. 讨论项

- [x] 槽位系统：基础类型走 `Vm_Base::SlotFile`，TokenIds/Tensor 走 `MlSlots`
- [x] Loop 控制流：`JumpIf` + body + `Jump` 扁平化，`BreakIf` 合并为 body 末尾的 `JumpIf`
- [x] OS 线程模型保留：`ML_VM` 在 `Session_Thread` 内部构造和执行
- [ ] `Inference` 指令中 Token/Tensor 的 META1 递增策略沿用原逻辑（Tokens→+1, Tensor→+seq_len）
- [ ] `Sample` 的 eos_token_id 从 backend 获取（同原逻辑）
