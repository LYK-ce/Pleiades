# ML_Engine 层设计文档

## 1. 概览

ML_Engine 是 Pleiades 的推理执行层，从最底层的 GGUF 模型文件解析到最顶层的 Capability trait 服务接口，构成了完整的六层架构：

```
┌──────────────────────────────────────────────────────────────────┐
│ Layer 5  Capability / Service / mod.rs                          │
│         ML_Engine_Capability trait → ML_Engine_Service          │
│         对外接口: Create_Session / Run_Program_VM / Shutdown_...│
├──────────────────────────────────────────────────────────────────┤
│ Layer 4  Session Runtime (session.rs)                           │
│         Session → Session_Handle → Session_Thread               │
│         OS 线程 + 命令泵 + ML_VM 执行                           │
├──────────────────────────────────────────────────────────────────┤
│ Layer 3  ML_VM 执行引擎 (ml_vm/)                                 │
│         MlInstruction → ML_VM.step() → Handlers                │
│         基于 Vm_Base + MlSlots 的指令驱动推理                  │
├──────────────────────────────────────────────────────────────────┤
│ Layer 2  模型抽象 (gguf_model.rs)                                │
│         GGUF_Model: 权重 + Tokenizer + Inference_Config         │
│         API: Load_Model / Unload_Model / Inference / Encode /   │
│         Decode                                                   │
├──────────────────────────────────────────────────────────────────┤
│ Layer 1  模型管理 (gguf_model_manager.rs)                        │
│         GGUF_Analyze / GGUF_Load_Layer / GGUF_Split_Model       │
│         文件 → 层权重 → 切分                                    │
├──────────────────────────────────────────────────────────────────┤
│ Layer 0  底层 (GGUF_Models/ + gguf_tensor.rs)                   │
│         Qwen3 权重定义 + Tensor 序列化                           │
│         candle_core / candle_transformers / shimmytok            │
└──────────────────────────────────────────────────────────────────┘
```

**依赖关系 (自底向上)**：
```
candle_core / candle_transformers / shimmytok  (第三方)
    └── GGUF_Models/qwen3.rs
           └── gguf_model_manager.rs
                  ├── gguf_tensor.rs  (独立, Tensor 网络传输)
                  └── gguf_model.rs
                         ├── pipeline.rs  (共享类型)
                         ├── ml_vm/      → Vm_Base
                         └── session.rs
                                └── capability.rs → service.rs
                                       └── mod.rs  (模块根)
```

---

## 2. Layer 0: 底层基础设施

### 2.1 第三方依赖

| 库 | 用途 |
|---|---|
| `candle_core` | 张量计算、设备抽象、GGUF 格式读取 |
| `candle_transformers` | `QMatMul` 量化矩阵乘法、`RmsNorm` |
| `candle_nn` | `Embedding`、`KV Cache`、`softmax` 等神经网络原语 |
| `shimmytok` | Qwen3 tokenizer (encode/decode) |

### 2.2 Qwen3 模型权重 (`GGUF_Models/qwen3.rs`)

定义 Qwen3 模型的结构组件：

```rust
// 注意力层
pub struct Attention_Weights {
    pub q_proj: QMatMul,
    pub k_proj: QMatMul,
    pub v_proj: QMatMul,
    pub o_proj: QMatMul,
    pub q_norm: RmsNorm,
    pub k_norm: RmsNorm,
}

// MLP 层
pub struct Mlp_Weights {
    pub gate_proj: QMatMul,
    pub up_proj: QMatMul,
    pub down_proj: QMatMul,
}

// 单层 Transformer Block
pub struct Layer_Weights {
    pub attention: Attention_Weights,
    pub mlp: Mlp_Weights,
    pub input_layernorm: RmsNorm,
    pub post_attention_layernorm: RmsNorm,
}

// 完整模型
pub struct Model_Weights {
    pub embed_tokens: Option<Embedding>,   // 可选的输入头
    pub layers: Vec<Layer_Weights>,        // Transformer Block 列表
    pub norm: Option<RmsNorm>,             // 可选的 output norm
    pub lm_head: Option<QMatMul>,          // 可选的输出头
    pub kv_cache: ConcatKvCache,
    pub device: Device,
}

// 旋转位置编码
pub struct Rotary_Embedding { ... }
```

**模型前向传播** (`Forward` 方法) 流程：
1. 若存在 `embed_tokens` → 输入 token IDs 经过 embedding
2. 逐层 transformer block 前向: Attention → MLP
3. KV cache 增量更新 (`offset` 控制位置编码)
4. 若存在 `norm` + `lm_head` → 输出 logits
5. 否则 → 输出 hidden states (relay 模式)

### 2.3 Tensor 网络传输 (`gguf_tensor.rs`)

ML Engine 的分布式推理依赖 tensor 跨节点传输。定义通用 Tensor 包装：

```rust
pub struct GGUF_Tensor_Packet {
    pub name: String,           // tensor 名称, 如 "hidden_state"
    pub shape: Vec<usize>,      // 形状
    pub dtype: GGUF_Dtype,      // 数据类型 (F32/F16/Q4_0/...)
    pub data: Vec<u8>,          // 原始字节
}
```

**序列化协议 (二进制布局)**：
```
[name_len: u32] [name: bytes] [ndim: u32] [shape: u64×ndim]
[dtype_len: u32] [dtype: bytes] [data_len: u64] [data: bytes]
```

提供两个核心函数：
- `GGUF_Tensor_Serialize(packet) → Vec<u8>` — 序列化
- `GGUF_Tensor_Deserialize(bytes) → GGUF_Tensor_Packet` — 反序列化

---

## 3. Layer 1: 模型管理 (`gguf_model_manager.rs`)

### 3.1 核心数据结构

```rust
pub struct Model_Arch_Info {
    pub architecture: String,        // "qwen3"
    pub num_layers: usize,           // Transformer Block 层数
    pub embedding_length: usize,     // 嵌入维度
    pub head_count: usize,           // 注意力头数
    pub head_count_kv: usize,        // KV 注意力头数 (GQA)
    pub head_dim: usize,             // 每头维度
    pub context_length: usize,       // 上下文长度
    pub rms_norm_eps: f64,           // LayerNorm epsilon
    pub rope_freq_base: f64,         // RoPE 频率基数
    pub vocab_size: usize,           // 词汇表大小
    pub eos_token_id: u32,           // EOS 标记 ID
    pub is_split: bool,              // 是否切分模型
    pub split_start: usize,          // 切分起始层
    pub split_end: usize,            // 切分结束层
    pub layers: Vec<Layer_Info>,     // 每层 tensor 详情
}

pub struct GGUF_Layer_Weights {
    pub layer_index: usize,
    pub tensors: HashMap<String, QTensor>,  // 量化 tensor 集合
}
```

### 3.2 层编号规则

```
层 0      → 输入层 (embedding)    token_embd.weight
层 1..=N  → Transformer Block    blk.0.* ~ blk.(N-1).*
层 N+1    → 输出层                 output_norm.weight + output.weight
```

### 3.3 API

**`GGUF_Analyze(path) → Model_Arch_Info`**
- 读取 GGUF 文件头 + metadata
- 提取架构名、层数、注意力参数
- 将所有 tensor 按 blk 编号分组
- 读取 EOS token ID 和 split 标记

**`GGUF_Load_Layer(path, layer_index, device) → GGUF_Layer_Weights`**
- 根据层索引加载指定层所有 tensor
- 层 0 → token_embd.weight
- 层 1..N → blk.{idx}.* 的所有 tensor
- 层 N+1 → output_norm.weight + output.weight

**`GGUF_Split_Model(path, start, end, output_dir)`**
- 输入: 完整 GGUF 文件 + 层范围
- 输出: `{stem}_split_{start}_{end}.pgguf`
- 复制并修改 metadata (追加 `pleiades.split.start/end`)
- 筛选选中的 tensor 写入新文件
- 处理 weight tying (output.weight 不存在时回退到 token_embd.weight)

---

## 4. Layer 2: 模型抽象 (`gguf_model.rs`)

### 4.1 GGUF_Model

```rust
pub struct GGUF_Model {
    pub model: Model_Weights,            // 组装好的权重
    pub tokenizer: Option<Tokenizer>,    // Qwen3 tokenizer (可空)
    pub inference_config: Inference_Config, // eos_token
    pub arch_info: Model_Arch_Info,      // 架构信息
    pub model_path: PathBuf,             // 文件路径
    pub device: Device,                  // 运行设备 (Cpu/Cuda)
    pub has_input_head: bool,            // 是否包含 embedding
    pub has_output_head: bool,           // 是否包含 lm_head
}

pub struct Inference_Config {
    pub eos_token: u32,                  // 唯一保留字段, 从模型 metadata 读取
}
```

### 4.2 API

**`GGUF_Load_Model(start, end, path, device) → GGUF_Model`**
1. `GGUF_Analyze` 解析架构
2. 校验架构 (目前仅支持 qwen3)
3. 范围校验: start/end 必须在有效范围内
4. 构建 `Rotary_Embedding`
5. 若 `start==0` → 加载 embedding + tokenizer
6. 加载 `[max(start,1)..=min(end,N)]` 的 transformer blocks
7. 若 `end==N+1` → 加载 output_norm + lm_head (处理 weight tying)
8. 组装 `Model_Weights`
9. 设置 `eos_token_id`

**`GGUF_Unload_Model(model) → true`**
- Drop 模型释放资源

**`GGUF_Model_Inference(model, input, offset) → Tensor`**
- 调用 `Model_Weights::Forward(input, offset)`
- 单次前向, 不自回归循环

**`GGUF_Encode(model, text) → Vec<u32>`**
- 添加 Qwen3 对话模板: `<|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n`
- 通过 tokenizer 编码为 token IDs

**`GGUF_Decode(model, token_ids) → String`**
- 移除末尾 EOS token
- 解码为文本

---

## 5. Layer 3: ML_VM 执行引擎 (`ml_vm/`)

### 5.1 设计理念

ML_VM 是 ML 推理的执行引擎，基于 Vm_Base 构建。将原有的 `Register_File` + `Loop/BreakIf` 嵌套执行模型，替换为 `SlotId` + `Jump/JumpIf` 扁平控制流。

```
原: Register_File { text[4], token[4], tensor[4], flag[4], meta[8] }
    Loop { body: [BreakIf, Inference, Sample, Decode, Output] }

新: Vm.slots: SlotFile<SlotId, SlotValue>    # 基础类型
    MlSlots:  HashMap<SlotId, MlSlotValue>   # 领域类型
    JumpIf + body + Jump                     # 扁平控制流
```

### 5.2 槽位系统 (`slots.rs`)

#### 槽位编号约定 (1000~1047)

| 类别 | 名称 | 类型 | 存储位置 | SlotId |
|------|------|------|----------|--------|
| 文本 | TEXT1~4 | String | `vm.slots` | 1000~1003 |
| Token | TOKENS1~4 | `Vec<u32>` | `ml_slots` | 1010~1013 |
| Tensor | TENSOR1~4 | `candle_core::Tensor` | `ml_slots` | 1020~1023 |
| Flag | FLAG1~4 | bool | `vm.slots` | 1030~1033 |
| Meta | META1~8 | f64 | `vm.slots` | 1040~1047 |

#### Meta 寄存器语义

| 槽位 | 含义 | 写入者 |
|------|------|--------|
| META1 | KV cache 位置偏移 | Encode(=0), Inference(+=增量), Prefill 后=prompt_len |
| META2 | 剩余生成 token 数 | Const(初始化), Sample(-=1) |
| META4 | 已生成步数 | Sample(+=1) |
| META5 | prompt token 数 | Prefill |
| META6 | 最后接收 offset | Receive |

#### 领域槽位类型

```rust
pub enum MlSlotValue {
    TokenIds(Vec<u32>),
    Tensor(Tensor),  // candle_core Tensor, Arc<Storage> 内部, clone 零拷贝
}

pub struct MlSlots {
    slots: HashMap<SlotId, MlSlotValue>,
}
```

提供类型化访问方法: `set_token_ids` / `take_token_ids` / `get_token_ids` / `append_token_id` / `clear_token_ids` / `set_tensor` / `take_tensor` / `get_tensor` / `clear_tensor` / `reset_all`

### 5.3 指令集 (`instruction.rs`)

```rust
pub enum MlInstruction {
    // 公共指令 (委托 Vm_Base)
    Const { value: ConstValue, dst: SlotId },
    Move  { src: SlotId, dst: SlotId },
    Add   { dst: SlotId, delta: f64 },
    Jump  { target: usize },
    JumpIf { condition: SlotId, target: usize },

    // 领域指令
    Input,                                                  // io_handle.blocking_recv → TEXT1
    Encode,                                                 // TEXT1 → tokenize → TOKENS3
    Decode,                                                 // TOKENS2 → detokenize → TEXT2
    Prefill { input: SlotId },                              // TOKENS → forward(offset=0) → TENSOR2
    Inference { input_type: InferenceInputType, input: SlotId }, // Token/Tensor → forward → TENSOR2
    Sample { tensor_slot: SlotId },                         // logits → sampling → TOKENS2 + FLAG1/META更新
    Output,                                                 // TEXT2 → io_handle.blocking_send
    EndOutput,                                              // 输出结束 noop
    Send,                                                   // TENSOR2 → 序列化 → tensor_io.Send
    Receive,                                                // tensor_io.Receive → TENSOR1
    SendEOF,                                                // 发送 EOF 哨兵
}

pub enum InferenceInputType {
    Tokens,   // META1 += 1
    Tensor,   // META1 += seq_len
}
```

### 5.4 控制流: Loop → Jump/JumpIf

**原始 (嵌套)**:
```
Loop [
    BreakIf            // if FLAG1 → break
    Inference(...)
    Sample(...)
    Output
]
```

**扁平化后**:
```
0: JumpIf(FLAG1, 5)   // check → exit
1: Inference(...)      // body
2: Sample(...)
3: Output
4: Jump(0)             // loop back
5: ...                 // after loop
```

### 5.5 ML_VM 结构体与执行 (`engine.rs`)

```rust
pub struct ML_VM<'a> {
    pub vm: Vm,                                      // Vm_Base (IP + SlotFile)
    pub ml_slots: MlSlots,                           // 领域槽位
    backend: &'a mut GGUF_Model,                     // 模型引用
    io_handle: &'a mut IoHandle,                     // 文本 I/O
    tensor_io: &'a mut Option<Tensor_IO_Endpoint>,   // 网络 I/O
    program: Vec<MlInstruction>,                     // 指令磁带
    pub temperature: f64,   rng_state: u64,  eos_token_id: u32,
}

impl ML_VM {
    pub fn execute_with_params(program, params, cancel_flag) → Result<Pipeline_Result, ML_Engine_Error>
    pub fn execute(program, cancel_flag) → Result<(), ML_Engine_Error>
    pub fn step(cancel_flag) → StepResult
}
```

**执行流程**:
1. `Session_Thread` 收到 `Run_Program_VM` 命令
2. 创建 `ML_VM` (借用 session 的 backend/io_handle/tensor_io)
3. `execute_with_params()`: 设置 temperature/seed/eos → `reset_state()` (初始化所有 Flag=false, Meta=0.0) → `execute()` 主循环
4. `execute()`: 循环调用 `step()`, 直到 `Done`/`Abort`/取消
5. `step()`: 取 `program[ip++]`, match 指令类型, 委托 Vm_Base 或调用 handler

### 5.6 Handler 清单

| Handler | 依赖资源 | 说明 |
|---------|---------|------|
| `handle_input` | `io_handle.input_rx` | 阻塞读 → `SLOT_TEXT1` |
| `handle_encode` | `backend` (tokenizer) | `SLOT_TEXT1` → tokenize → `SLOT_TOKENS3` |
| `handle_decode` | `backend` (tokenizer) | `SLOT_TOKENS2` → detokenize → `SLOT_TEXT2` |
| `handle_prefill` | `backend` (model) | `SLOT_TOKENS[input]` → forward(offset=0) → `SLOT_TENSOR2`, `META5` |
| `handle_inference` | `backend` (model) | 根据 input_type → forward → `SLOT_TENSOR2`, `META1` 递增 |
| `handle_sample` | `eos_token_id` | logits → LCG 采样 → `SLOT_TOKENS2`, `SLOT_TOKENS1.append`, `META2/META4/FLAG1` 维护 |
| `handle_output` | `io_handle.output_tx` | `SLOT_TEXT2` → 发送 |
| `handle_send` | `tensor_io` | `SLOT_TENSOR2` → 序列化 → `tensor_io.Send(offset, bytes)` |
| `handle_receive` | `tensor_io` | `tensor_io.Receive()` → `SLOT_TENSOR1`; EOF→`FLAG1=true` |
| `handle_send_eof` | `tensor_io` | 发送 EOF 哨兵帧 |

### 5.7 三种推理程序

#### run (单机推理)

```
Input → Encode → Const(META2=$max_tokens) → Const(FLAG1=false) → Prefill(TOKENS3) → Move(META5→META1)
  → Sample(TENSOR2) → Decode → Output
  → [JumpIf(FLAG1, end) → Inference(Tokens, TOKENS2) → Sample(TENSOR2) → Decode → Output → Jump(check)]
  → EndOutput
```

#### relay (Worker/中继)

```
[JumpIf(FLAG1, end) → Receive → JumpIf(FLAG1, end)       ← 双重 FLAG1 检查
  → Inference(Tensor, TENSOR1) → Send → Jump(check)]
  → SendEOF
```

#### coordinator (分布式协调者)

```
Input → Encode → Const(META2) → Const(FLAG1=false) → Prefill → Move(META5→META1)
  → Send → Receive → Sample(TENSOR1) → Decode → Output
  → [JumpIf(FLAG1, end) → Inference(Tokens, TOKENS2) → Send → Receive
     → Sample(TENSOR1) → Decode → Output → Jump(check)]
  → SendEOF → EndOutput
```

---

## 6. Layer 4: Session 运行时 (`session.rs`)

### 6.1 架构

每个 Session 是一个独立的 OS 线程，持有完整的模型实例。通过通道与外部通信：

```
   上层 (Orchestrator)
        │                    │                    │
   Session_Handle       IoHandle            Tensor_IO_Endpoint
   (命令平面)          (文本 I/O)          (网络数据平面)
        │                    │                    │
   [cmd_tx/cmd_rx]    [input_rx/output_tx]   [Send/Receive]
        │                    │                    │
        └────────────────────┼────────────────────┘
                             │
                      Session_Thread
                      (OS Thread)
                             │
                          ML_VM
```

### 6.2 核心类型

```rust
pub struct Session_Config {
    pub model_path: PathBuf,
    pub layer_start: usize,
    pub layer_end: usize,
    pub device: String,          // "cpu" / "cuda"
}

pub enum Session_Command {
    Run_Program_VM {
        program: Vec<MlInstruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<Pipeline_Result>>,
    },
    Shutdown,
}

pub struct Session {
    pub id: String,
    pub(crate) cmd_rx: Receiver<Session_Command>,
    pub(crate) io_handle: IoHandle,
    pub(crate) backend: GGUF_Model,
    pub(crate) config: Session_Config,
    pub(crate) tensor_io: Option<Tensor_IO_Endpoint>,
}

pub struct Session_Handle {
    session_id: String,
    cmd_tx: Sender<Session_Command>,
}
```

### 6.3 线程生命周期

`Session_Thread(session_id, config, cmd_rx, io_handle, tensor_io, ready_tx)`:

1. **设备初始化**: 解析 device 字符串 → `Device::Cpu` / `Device::Cuda`
2. **模型加载**: `GGUF_Load_Model(layer_start, layer_end, path, device)`
3. **就绪通知**: 构造 `Model_Info` → 通过 `ready_tx` 发送
4. **指令泵循环**:
   - `Run_Program_VM` → 创建 `ML_VM` → `execute_with_params()` → 通过 oneshot reply 返回结果
   - `Shutdown` → 退出循环
5. **卸载**: `GGUF_Unload_Model(model)`

---

## 7. Layer 5: Capability 与服务层

### 7.1 Pipeline 类型 (`pipeline.rs`)

```rust
pub struct Pipeline_Params {
    pub max_tokens: usize,    // 默认 120
    pub temperature: f64,     // 默认 0.8
    pub seed: u64,            // 默认 299792458
    pub eos_token_id: Option<u32>,  // None → 使用模型内置值
}

pub struct Pipeline_Result {
    pub result_text: String,
    pub generated_tokens: Vec<u32>,
    pub prompt_tokens: Vec<u32>,
    pub model_info: Option<Model_Info>,
    pub duration: Duration,
    pub total_steps: usize,
}

pub struct Model_Info {
    pub architecture: String,
    pub num_layers: usize,
    pub has_input_head: bool,
    pub has_output_head: bool,
    pub has_tokenizer: bool,
    pub eos_token_id: u32,
}
```

### 7.2 Capability Trait (`capability.rs`)

对外接口，所有内部类型对外不可见：

```rust
pub struct ML_Session_Config {
    pub session_id: String,
    pub model_file_id: String,    // Storage 中的文件 ID
    pub layer_start: usize,
    pub layer_end: usize,
    pub device: String,
    pub tensor_io: Option<Tensor_IO_Endpoint>,
}

pub enum ML_Engine_Error {
    SessionCreationFailed(String),
    SessionNotFound(String),
    ProgramFailed(String),
    ModelAnalysisFailed(String),
    ModelSplitFailed(String),
}

#[async_trait]
pub trait ML_Engine_Capability: Send + Sync {
    async fn Create_Session(config, io_handle) → Result<Model_Info, ML_Engine_Error>;
    async fn Shutdown_Session(session_id) → Result<(), ML_Engine_Error>;
    async fn Run_Program_VM(session_id, program, params, cancel_flag) → Result<Pipeline_Result, ML_Engine_Error>;
    async fn Analyze_Model(model_file_id) → Result<Model_Info, ML_Engine_Error>;
    async fn Split_Model(source, start, end, output) → Result<(), ML_Engine_Error>;
}
```

### 7.3 Service 实现 (`service.rs`)

`ML_Engine_Service` 持有 `StorageManager` + `HashMap<String, SessionEntry>`：

- `Create_Session`: Storage 获取读锁 → `Session_Config` 构建 → `Session_Thread` 启动 → 注册 handle
- `Shutdown_Session`: 发送 Shutdown 命令 + join 线程 + Storage 释放读锁
- `Run_Program_VM`: 查表获取 handle → `handle.Run_Program_VM()` → 等待回复
- `Analyze_Model`: Storage 临时读锁 → `GGUF_Analyze`
- `Split_Model`: Storage 读锁(源) + 写锁(输出) → `GGUF_Split_Model`

---

## 8. 程序加载: ProgramSelector (`program_selector.rs`)

### 8.1 ML 程序模板

ML 程序定义在 `programs/ml/*.tmpl` 文件中，编译期通过 `include_str!()` 嵌入二进制：

```rust
const ML_RUN_TMPL: &str         = include_str!("../../../programs/ml/run.tmpl");
const ML_RELAY_TMPL: &str       = include_str!("../../../programs/ml/relay.tmpl");
const ML_COORDINATOR_TMPL: &str = include_str!("../../../programs/ml/coordinator.tmpl");
```

### 8.2 load_ml_program_vm

```rust
pub fn load_ml_program_vm(mode, params) → Result<Vec<MlInstruction>, SelectorError> {
    1. 选模板常量 (run/relay/coordinator)
    2. toml::from_str → RawMLTemplate
    3. 遍历 raw instructions → convert_ml_instruction_vm(raw, params)
    4. 返回 Vec<MlInstruction>
}
```

转换逻辑 `convert_ml_instruction_vm`: 将 TOML 的 `type/input_type/value/dst/src/condition/target` 字段转换为 `MlInstruction` 枚举变体。`$max_tokens` 变量由 `Pipeline_Params.max_tokens` 替换。槽位名 (`TOKENS3`/`META2`/`FLAG1` 等) 通过 `resolve_ml_slot()` 查表转换为 `SlotId`。

---

## 9. Vm_Base 依赖

ML_VM 依赖 `Vm_Base` 模块提供通用 VM 骨架：

```rust
pub struct SlotId(pub u32);
pub type SlotFile = HashMap<SlotId, SlotValue>;

pub enum ConstValue { U64(u64), F64(f64), Bool(bool), String(String) }
pub enum SlotValue { Nil, U64(u64), F64(f64), Bool(bool), String(String) }

pub struct Vm {
    pub slots: SlotFile,
    pub ip: usize,
}

impl Vm {
    // 5 条基础指令
    pub fn handle_const(&mut self, value, dst) → StepResult;
    pub fn handle_move(&mut self, src, dst) → StepResult;
    pub fn handle_add(&mut self, dst, delta) → StepResult;
    pub fn handle_jump(&mut self, target) → StepResult;
    pub fn handle_jump_if(&mut self, condition, target) → StepResult;
}

pub enum StepResult {
    Continue,
    Done,
    Abort(String),
    Ready,
}
```

ML_VM 将 `Const`/`Move`/`Add`/`Jump`/`JumpIf` 直接委托给 `self.vm.handle_xxx()`，领域指令由 `ML_VM.step()` 分发到自身 handler。

---

## 10. 完整数据流

### 10.1 单机推理 (Run)

```
Orchestrator                          ML_Engine_Service               Session_Thread
    │                                       │                              │
    │ Create_Session(config, io_handle)      │                              │
    │──────────────────────────────────────→│                              │
    │                                       │ spawn Session_Thread         │
    │                                       │─────────────────────────────→│
    │                                       │                      GGUF_Load_Model
    │                                       │                          tokenize 就绪
    │                                       │←─ ready_tx: Model_Info ────│
    │←── Ok(Model_Info) ────────────────────│                              │
    │                                       │                              │
    │ Run_Program_VM(program, params)       │                              │
    │──────────────────────────────────────→│                              │
    │                                       │ Run_Program_VM               │
    │                                       │─────────────────────────────→│
    │                                       │            ML_VM::execute()  │
    │  ← ←  io_handle.output_tx  ← ← ← ← ← │← ← ← Output handler ← ← ← ← ← ←←│
    │                                       │         ┌─────────────────┐  │
    │                                       │         │ Input           │  │
    │                                       │         │ Encode (tokenize)│  │
    │                                       │         │ Prefill (forward)│  │
    │                                       │         │ Sample → Decode │  │
    │                                       │         │ [loop: Inference│  │
    │                                       │         │  Sample Decode  │  │
    │                                       │         │  Output]        │  │
    │                                       │         │ EndOutput       │  │
    │                                       │         └─────────────────┘  │
    │                                       │←─ reply: Pipeline_Result ───│
    │←── Pipeline_Result ───────────────────│                              │
    │                                       │                              │
    │ Shutdown_Session(id)                  │                              │
    │──────────────────────────────────────→│ Shutdown                    │
    │                                       │─────────────────────────────→│
    │                                       │               GGUF_Unload    │
```

### 10.2 分布式推理 (Coordinator + Worker)

```
Coordinator Session                    Worker Sessions (×N)
      │                                      │
      │ Input → Encode → Prefill             │
      │ META5 → META1                        │
      │                                      │
      │ Send(TENSOR2) ──── tensor_io ────→   │ Receive → TENSOR1
      │                                      │ Inference(Tensor, TENSOR1)
      │                                      │ Send(TENSOR2)
      │ ←─── tensor_io ←─── Receive ────     │
      │ Sample(TENSOR1)                      │
      │ Decode → Output                      │
      │                                      │
      │ [Loop: Inference → Send              │ [Loop: Receive
      │        ← Receive ← Sample            │        Inference → Send]
      │        Decode Output]                │
      │                                      │
      │ SendEOF ──── tensor_io ────────→     │ Receive → EOF → FLAG1=true
      │ EndOutput                            │ SendEOF
```

---

## 11. 目录结构总览

```
Src/ML_Engine/
├── mod.rs                       # 模块根 + re-export
├── pipeline.rs                  # Pipeline_Params / Pipeline_Result / Model_Info
├── session.rs                   # Session / Session_Handle / Session_Thread
├── capability.rs                # ML_Engine_Capability trait + ML_Engine_Error
├── service.rs                   # ML_Engine_Service (Storage + Session 注册表)
├── gguf_model.rs                # GGUF_Model 抽象 (加载/卸载/推理/编解码)
├── gguf_model_manager.rs        # GGUF 文件分析/层加载/模型切分
├── gguf_tensor.rs               # Tensor 网络传输序列化
├── ml_vm/
│   ├── mod.rs
│   ├── instruction.rs           # MlInstruction + InferenceInputType
│   ├── slots.rs                 # MlSlots + MlSlotValue
│   └── engine.rs                # ML_VM 执行引擎
└── GGUF_Models/
    ├── mod.rs
    └── qwen3.rs                 # Qwen3 权重定义 (candle 量化)

programs/ml/
├── run.tmpl                     # 单机推理程序模板
├── relay.tmpl                   # Worker 中继程序模板
└── coordinator.tmpl             # 分布式协调者程序模板
```
