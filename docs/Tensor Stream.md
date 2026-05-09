# Tensor Stream 实现方案

## 背景

当前张量数据（`DataType::Data`）与控制命令（`DataType::Command`）共用 `data_protocol.rs` 的 request-response 协议。Pipeline 推理时每个张量都走完整的 request→ACK 流程，带来不必要的往返延迟，且大张量可能阻塞后续控制命令。

参照 `stream_protocol.rs` + `File_Transfer_Manager` 的设计模式，为张量引入专用的持久化 `libp2p::Stream` 通道，实现数据面与控制面的完全隔离。

## 整体架构

```
+-----------------------------+     +-------------------------------+     +-----------------------------+
| Control Plane (现有)         |     | Data Plane (新增)              |     | File Plane (现有)            |
| data_protocol.rs            |     | tensor_stream_protocol.rs     |     | stream_protocol.rs          |
| /pleiades/data/1.0.0        |     | /pleiades/tensor/1.0.0        |     | /pleiades/file-stream/1.0.0 |
| request-response 模式        |     | persistent stream 模式         |     | one-shot stream 模式         |
| WORK / LOAD / PIPELINE_FLOW |     | Hidden State Tensors          |     | Model Files                 |
+-----------------------------+     +-------------------------------+     +-----------------------------+
```

## 当前阶段目标

第一阶段只在 Network 层内部实现解耦的 tensor stream 组件，**不修改 Control 层和 ML Engine**。当前的 request-response 张量传输继续正常工作。

## 帧格式

```
张量帧格式（带长度前缀，fire-and-forget，无需 ACK）:
+-------------------+--------------------+---------------------+
|   Offset          |   Tensor Length    |   Raw Tensor Data   |
|   8 bytes u64 LE  |   8 bytes u64 LE  |   Length bytes      |
+-------------------+--------------------+---------------------+

EOF 哨兵帧（标记推理会话结束）:
+-------------------+--------------------+
|   u64::MAX        |   0u64             |
|   8 bytes         |   8 bytes          |
+-------------------+--------------------+
```

使用长度前缀（方案 A）而非固定长度，因为 Prefill 阶段（seq_len=512, ~8MB）和 Decode 阶段（seq_len=1, ~16KB）的张量大小差异很大。

接收方通过 `read_exact` 先读 16B header 获取 length，再 `read_exact(length)` 精确读取 payload，保证帧边界。

## 数据面与控制面分离设计

采用**双 Manager + IO Handle + 所有权转移**设计：

- **Tensor_Stream_Manager**（双实例）：Network_Service 持有，只负责流的**建立和接收**
- **Tensor_IO_Handle**：ML Engine worker 线程持有，直接操作 stream 做张量收发

### 生命周期

```
阶段 1: 建立                       阶段 2: 推理                    阶段 3: 结束
Network_Service 持有                ML Engine 持有                  Handle drop
  ┌──────────────────┐              ┌───────────────────┐
  │ Inbound_Manager  │──Take──→     │ Tensor_IO_Handle  │──→ drop ──→ TCP FIN
  │ Outbound_Manager │──Take──→     │ (直接 read/write)  │
  └──────────────────┘              └───────────────────┘
  Manager 建流/接收流                ML Engine 串行执行:
  Control 发 PIPELINE_FLOW           Receive → Inference → Send
```

### 关键设计点

1. **无共享内存**：ML Engine 串行执行 receive → inference → send，buffer 完全线程私有，不需要 `Arc<Mutex<>>`
2. **所有权转移**：stream 通过 `Take_Stream()` 从 Manager 移到 `Tensor_IO_Handle`，保证同一时刻只有一方操作 stream
3. **零中间环节**：推理阶段的张量收发不经过 Network_Service 事件循环、不经过 channel、不经过 Control 层
4. **缓冲区复用**：`Tensor_IO_Handle` 内部的 `Tensor_Buffer` 在 pipeline 生命周期内复用

## 新增文件

### `Src/Network/tensor_stream_protocol.rs`

协议层：帧读写函数 + 共享缓冲区结构。

```rust
// 协议标识
pub const TENSOR_STREAM_PROTOCOL: &str = "/pleiades/tensor/1.0.0";

// 共享张量缓冲区
pub struct Tensor_Buffer {
    buf: Vec<u8>,
}

impl Tensor_Buffer {
    pub fn New(capacity: usize) -> Self;
    pub fn As_Mut_Slice(&mut self, len: usize) -> &mut [u8];
    pub fn As_Slice(&self) -> &[u8];
    pub fn Len(&self) -> usize;
}

// 发送张量帧：写入 [8B offset][8B length][data]
pub async fn Send_Tensor_Frame(
    stream: &mut libp2p::Stream,
    offset: u64,
    data: &[u8],
) -> io::Result<()>;

// 接收张量帧：读取帧头 → 读取数据到 buffer
pub async fn Receive_Tensor_Frame(
    stream: &mut libp2p::Stream,
    buffer: &mut Tensor_Buffer,
) -> io::Result<u64>;  // 返回 offset, u64::MAX 表示 EOF

// 发送 EOF 哨兵帧
pub async fn Send_EOF(stream: &mut libp2p::Stream) -> io::Result<()>;
```

### `Src/Network/tensor_stream_manager.rs`

包含两个组件：

**Tensor_Stream_Manager** — 流生命周期管理（Network_Service 持有）

```rust
pub struct Tensor_Stream_Manager {
    stream_control: stream::Control,
    stream: Option<libp2p::Stream>,
}

impl Tensor_Stream_Manager {
    pub fn New(stream_control: stream::Control) -> Self;
    pub fn Set_Stream(&mut self, peer: PeerId, stream: libp2p::Stream);
    pub async fn Open_Stream(&mut self, peer: PeerId) -> io::Result<()>;
    pub fn Take_Stream(&mut self) -> Option<libp2p::Stream>;  // 所有权转移
    pub fn Has_Stream(&self) -> bool;
}
```

**Tensor_IO_Handle** — 张量 I/O 句柄（ML Engine worker 线程持有）

```rust
pub struct Tensor_IO_Handle {
    inbound_stream: libp2p::Stream,
    outbound_stream: libp2p::Stream,
    buffer: Tensor_Buffer,           // 私有缓冲区，无需共享
    rt: tokio::runtime::Handle,
}

impl Tensor_IO_Handle {
    pub fn New(inbound: libp2p::Stream, outbound: libp2p::Stream, rt: Handle) -> Self;
    pub fn Receive(&mut self) -> io::Result<u64>;      // 阻塞 read，返回 offset
    pub fn Send(&mut self, offset: u64, data: &[u8]) -> io::Result<()>;  // 阻塞 write
    pub fn Send_EOF(&mut self) -> io::Result<()>;
    pub fn Get_Buffer(&self) -> &[u8];                 // 获取最近 Receive 的数据
}
```

## 修改文件

### `Src/Network/network_service.rs`

1. `Network_Service` 新增两个字段：
   - `inbound_tensor_manager: Option<Tensor_Stream_Manager>`
   - `outbound_tensor_manager: Option<Tensor_Stream_Manager>`
2. `Init()` 中为 tensor stream 创建 `stream::Control` 句柄
3. `Start()` 的 `select!` 新增 tensor incoming 分支
4. `Handle_Command` 新增：`CreateTensorStream`、`OpenTensorStream`、`TakeTensorStreams`、`CloseTensorStream`

### `Src/Network/node_handle.rs`

1. `NodeCommand` 新增：
   - `CreateTensorStream { reply }` — 创建两个 manager
   - `OpenTensorStream { peer, reply }` — 打开出站流
   - `TakeTensorStreams { reply }` — 从 manager 取出 stream（所有权转移）
   - `CloseTensorStream { reply }` — 清理 manager
2. `NodeHandle` 新增 async API：
   - `Create_Tensor_Stream()`
   - `Open_Tensor_Stream(peer)`
   - `Take_Tensor_Streams()` → `(libp2p::Stream, libp2p::Stream)`
   - `Close_Tensor_Stream()`

### `Src/Network/mod.rs`

导出 `tensor_stream_protocol` 和 `tensor_stream_manager` 模块。

## Pipeline 建流时序（将来 Control 层集成时）

```
Coordinator                     Peer 1                      Peer 2
    |                              |                           |
    |-- PIPELINE_FLOW(next=P2) -->|                           |
    |                              |-- Open Tensor Stream --> |
    |                              |                           |
    |<--------- OK ---------------|                           |
    |                                                          |
    |-- PIPELINE_FLOW(next=Coord) --------------------------->|
    |                              |                           |
    |                              |    Open Tensor Stream <---|
    |<------------- OK ----------------------------------------|
    |                                                          |
    |-- Open Tensor Stream --->|                               |
    |                              |                           |
    |  此时每个节点持有:                                         |
    |  outbound_stream → next_peer                             |
    |  inbound_stream  ← prev_peer                             |
```

每个节点在收到 `PIPELINE_FLOW` 命令时，主动向自己的 `next_peer` 发起 tensor stream 连接。协调者发完所有 `PIPELINE_FLOW` 后，也打开到 `peers[0]` 的流。入站流由 Network 层 `select!` 自动接收并交给 `Tensor_Stream_Manager` 保存。

## ML 执行引擎（ML Thread Engine）

### 设计思路

将 ML Engine 重构为**指令驱动的执行引擎**：所有操作（包括模型加载/卸载、推理、编解码）统一建模为指令序列（Program），由 ML Engine worker 线程作为纯粹的指令执行器逐条执行。Control 层只负责编排指令序列并提交，推理期间完全由 ML Engine 线程自驱动。

这种设计参考了 ONNX Runtime（算子执行计划）、vLLM（调度步骤）等工业化 ML 推理框架的模式。

### 文件结构

执行引擎从 `ml_inference_service.rs` 中分离为两个独立文件，实现关注点分离：

```
Src/ML_Engine/
├── mod.rs                                # 模块导出
├── ml_inference_service.rs               # 现有：Handle + Worker Loop（极简调度层）
├── ml_thread_engine.rs            (新增)  # 执行器 + 执行上下文 + 全部执行逻辑
├── ml_thread_engine_instruction.rs (新增) # 指令集 + 条件枚举 + Pipeline_Result
├── gguf_model.rs                         # 现有：GGUF 模型抽象
├── gguf_model_manager.rs                 # 现有：模型解析/加载
├── gguf_tensor.rs                        # 现有：张量序列化
├── error.rs                              # 现有：错误类型
└── GGUF_Models/                          # 现有：具体模型实现
```

**依赖链**（单向）：

```
ml_thread_engine_instruction.rs    ← 纯数据类型，无外部依赖
         ↑
ml_thread_engine.rs                ← 依赖 instruction + gguf_model + tensor_io
         ↑
ml_inference_service.rs            ← 依赖 engine（Worker Loop 调 Execute）
         ↑
Control 层                          ← 依赖 instruction（编排程序）+ service（提交执行）
```

### 指令集（`ml_thread_engine_instruction.rs`）

```rust
enum Instruction {
    // ===== 模型生命周期 =====
    Load_Model { path: PathBuf, start: usize, end: usize, device: String },
    Unload_Model,
    Analyze_Model { path: PathBuf },
    Split_Model { path: PathBuf, start: usize, end: usize, output: PathBuf },

    // ===== 推理原子指令 =====
    Encode,              // text → token_ids → input_bytes
    Inference,           // input_bytes → model.forward → output
    Send,                // output → tensor_io.Send()
    Receive,             // tensor_io.Receive() → input_bytes
    Sample,              // output(logits) → next_token → input_bytes
    Decode,              // generated_tokens → result_text

    // ===== 复合指令（常用模式优化，跳过 context 中转） =====
    Relay,               // Receive → Inference → Send (Worker 核心操作)
    Inference_Round,     // Inference → Send → Receive → Sample (Coordinator decode 步骤)

    // ===== 控制流 =====
    Loop { body: Vec<Instruction> },
    Break_If { condition: Condition },

    // ===== 系统 =====
    Send_EOF,            // 发送 EOF 哨兵帧
}

enum Condition {
    EOS,          // next_token == eos_token_id
    EOF,          // 最近一次 Receive 收到 EOF
    Max_Rounds,   // generated_tokens.len() >= max_rounds
}
```

#### 关于模型生命周期指令

`Load_Model`、`Unload_Model` 等也是指令而非独立命令，这使得 `Service_Command` 简化到只有两个变体（`Run_Program` + `Shutdown`）。模型加载和推理分两次 `Run_Program` 调用执行，因为 Control 层需要 `Load_Model` 的返回信息（`Model_Load_Info`）来编排后续推理指令序列：

```
// 阶段 1：加载模型
let load_result = ml_service.Run_Program([Load_Model{...}], params).await?;
let model_info = load_result.model_info.unwrap();

// 阶段 2：基于 model_info 编排推理程序
let program = Build_Coordinator_Program(model_info.eos_token_id, ...);
let inference_result = ml_service.Run_Program(program, params, tensor_io).await?;
```

`backend` 状态在 Worker 线程中跨调用持久化（`Worker_Loop` 的局部变量），不随 Program 结束销毁。

#### 复合指令的 EOF 处理

`Relay` 内部 Receive 收到 EOF 时，必须跳过后续的 Inference 和 Send，避免用无效数据推理：

```
Relay 内部伪代码:
    Receive()
    if eof_received { return Ok(()) }   // 跳过后续，由 Break_If(EOF) 处理退出
    Inference()
    Send()
```

### 执行上下文（`ml_thread_engine.rs`）

`Pipeline_Context` 是执行器的内部状态，由 Worker 线程内部构造，不暴露给 Control 层。Control 层通过 `Pipeline_Params` 传递必要参数。

```rust
/// Control 层提交的参数（跨层传递）
struct Pipeline_Params {
    prompt: String,
    max_rounds: usize,
    temperature: f64,
    seed: u64,
    // ... 其他配置
}

/// 执行器内部上下文（Worker 线程私有）
struct Pipeline_Context {
    // I/O
    tensor_io: Option<Tensor_IO_Handle>,

    // 缓冲区
    input_bytes: Vec<u8>,
    output: Option<Tensor>,

    // Token 状态
    prompt: String,
    token_ids: Vec<u32>,
    generated_tokens: Vec<u32>,
    next_token: u32,
    offset: usize,

    // 配置
    eos_token_id: u32,
    max_rounds: usize,

    // 采样器
    sampler: LogitsProcessor,

    // 控制流
    should_break: bool,
    eof_received: bool,

    // 取消标志（与 Control 层共享）
    cancel_flag: Arc<AtomicBool>,

    // 结果
    result_text: String,
    model_info: Option<Model_Load_Info>,   // Load_Model 指令填充
}
```

### 执行器（`ml_thread_engine.rs`）

递归执行指令序列。Loop 通过 `should_break` 标记实现 break 语义。每步检查 `cancel_flag` 支持中途取消。

```rust
fn Execute(
    program: &[Instruction],
    ctx: &mut Pipeline_Context,
    backend: &mut Option<Inference_Backend>,
) -> Result<()> {
    for inst in program {
        if ctx.should_break { break; }
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow!("Program cancelled"));
        }
        match inst {
            // 模型生命周期
            Instruction::Load_Model { path, start, end, device } => {
                /* 加载模型 → backend, 结果存入 ctx.model_info */
            }
            Instruction::Unload_Model => { /* 卸载模型 → backend = None */ }
            Instruction::Analyze_Model { path } => { /* 分析模型 → ctx.model_info */ }
            Instruction::Split_Model { .. } => { /* 切分模型文件 */ }

            // 推理原子指令
            Instruction::Encode => { /* text → token_ids → input_bytes */ }
            Instruction::Inference => { /* model.forward(input_bytes, offset) → output */ }
            Instruction::Send => { /* tensor_to_bytes(output) → tensor_io.Send() */ }
            Instruction::Receive => { /* tensor_io.Receive() → input_bytes (or set eof_received) */ }
            Instruction::Sample => { /* logits → sample → next_token → input_bytes */ }
            Instruction::Decode => { /* all_tokens → decode → result_text */ }

            // 复合指令
            Instruction::Relay => {
                // Receive → (if !eof) Inference → Send
            }
            Instruction::Inference_Round => {
                // Inference → Send → Receive → Sample
            }

            // 控制流
            Instruction::Loop { body } => {
                loop {
                    Execute(body, ctx, backend)?;
                    if ctx.should_break {
                        ctx.should_break = false;
                        break;
                    }
                }
            }
            Instruction::Break_If { condition } => {
                if check_condition(condition, ctx) {
                    ctx.should_break = true;
                    return Ok(());
                }
            }

            // 系统
            Instruction::Send_EOF => { /* tensor_io.Send_EOF() */ }
        }
    }
    Ok(())
}
```

### 三种场景的程序编排

**协调者（有 pipeline）**：
```
Encode
Inference                           ← prefill（本机 layers）
Send                                ← 发给 pipeline 下游
Receive                             ← 从 pipeline 最后一个节点收回 logits
Sample                              ← 采样第一个 token
Loop [
    Break_If(EOS)
    Break_If(Max_Rounds)
    Inference_Round                  ← 复合指令: Inference → Send → Receive → Sample
]
Send_EOF                            ← 通知所有节点结束
Decode
```

**Worker**：
```
Loop [
    Relay                            ← 复合指令: Receive → Inference → Send
    Break_If(EOF)
]
```

**单机（无 pipeline）**：
```
Encode
Inference                            ← prefill
Sample
Loop [
    Break_If(EOS)
    Break_If(Max_Rounds)
    Inference                        ← decode step
    Sample
]
Decode
```

### 与现有架构的集成

#### ML Service 层（`ml_inference_service.rs`）

`Service_Command` 简化为仅两个变体，Worker Loop 变为纯粹的指令泵：

```rust
enum Service_Command {
    Run_Program {
        program: Vec<Instruction>,
        params: Pipeline_Params,
        tensor_io: Option<Tensor_IO_Handle>,
        cancel_flag: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<Pipeline_Result>>,
    },
    Shutdown,
}

fn Worker_Loop(mut cmd_rx: mpsc::Receiver<Service_Command>) {
    let mut backend: Option<Inference_Backend> = None;
    loop {
        match cmd_rx.blocking_recv() {
            Some(Service_Command::Run_Program { program, params, tensor_io, cancel_flag, reply }) => {
                let mut ctx = Pipeline_Context::New(params, tensor_io, cancel_flag);
                let result = Execute(&program, &mut ctx, &mut backend);
                let _ = reply.send(result.map(|_| ctx.Into_Result()));
            }
            Some(Service_Command::Shutdown) | None => break,
        }
    }
}
```

现有的 `Inference`、`Encode`、`Decode` 等独立命令全部移除，统一走 `Run_Program`。

`ML_Service_Handle` 对外只暴露：
- `Run_Program(program, params, tensor_io)` — 统一执行入口
- `Shutdown()` — 关闭服务

#### Control 层：两层 Queue 架构

Control 层维护一个**显式 Program Queue**，mpsc channel 作为隐式通道连接到 Worker。用户命令先翻译为 Program 入队，Control 逐个 dispatch 到 Worker。

```
用户 CLI → Control 翻译 → [显式 Program Queue] → Dispatch → mpsc(隐式) → Worker → Execute
                              ↑ 可查看/取消/重排              ↑ 不可撤回
```

```rust
struct Program_Queue {
    queue: VecDeque<Program_Job>,
    current_job: Option<u64>,           // 当前执行中的 job_id
    cancel_flag: Arc<AtomicBool>,       // 取消正在执行的 program
}

struct Program_Job {
    id: u64,
    program: Vec<Instruction>,
    params: Pipeline_Params,
    tensor_io: Option<Tensor_IO_Handle>,
}

impl Program_Queue {
    fn Submit(&mut self, job: Program_Job) {
        self.queue.push_back(job);
    }

    fn Cancel(&mut self, job_id: u64) -> bool {
        // 排队中 → 直接从 VecDeque 移除
        if let Some(pos) = self.queue.iter().position(|j| j.id == job_id) {
            self.queue.remove(pos);
            return true;
        }
        // 执行中 → 设置 cancel flag，Execute 每步检查
        if self.current_job == Some(job_id) {
            self.cancel_flag.store(true, Ordering::Relaxed);
            return true;
        }
        false
    }

    async fn Dispatch_Next(&mut self, ml_service: &ML_Service_Handle) -> Option<Pipeline_Result> {
        if let Some(job) = self.queue.pop_front() {
            self.current_job = Some(job.id);
            self.cancel_flag.store(false, Ordering::Relaxed);

            let result = ml_service.Run_Program(
                job.program,
                job.params,
                job.tensor_io,
                self.cancel_flag.clone(),
            ).await;

            self.current_job = None;
            return result.ok();
        }
        None
    }
}
```

取消机制的三种场景：

| 场景 | Job 状态 | 处理方式 |
|------|----------|----------|
| 排队中 | 在 Control 的 `VecDeque` 里 | 直接 `remove(job_id)`，不会进 mpsc |
| 已提交 | 在 mpsc 缓冲区（窗口极短） | Worker 会立即取走执行，几乎不需处理 |
| 执行中 | Worker 正在 `Execute()` | `cancel_flag` 设为 true，Execute 每步检查并提前返回错误 |

由于 Control 层采用逐个 dispatch 模式（前一个 await reply 完成后才 dispatch 下一个），mpsc channel 中最多只有 1 个 program。

### 数据流（推理阶段）

```
ML Engine Worker Thread (直接操作 stream，不经 Network_Service loop)

Coordinator                     Peer 1                      Peer 2
    │                              │                           │
    │  Inference(layers 0-18)      │                           │
    │  Send ──→  stream ──→       │                           │
    │                              │  Receive                  │
    │                              │  Inference(layers 19-28)  │
    │                              │  Send ──→ stream ──→     │
    │                              │                           │  Receive
    │                              │                           │  Inference(layers 29-37)
    │                   stream ←── │                           │  Send
    │  Receive                     │                           │
    │  Sample → next token         │                           │
    │  (下一轮...)                  │                           │
```

所有张量通过专用 tensor stream 传输（fire-and-forget），控制命令继续走现有的 request-response 通道。

### 会话结束协议

- **正常结束**：协调者 Sample 到 EOS token 后，执行 `Send_EOF` 指令，向 outbound stream 发送 EOF 帧（`offset=u64::MAX, length=0`）。Worker 的 `Relay` 指令内部 Receive 收到 EOF 后设置 `eof_received=true`，`Break_If(EOF)` 触发退出循环。
- **异常结束**：节点崩溃时 TCP 断开，`read_exact` 返回 `UnexpectedEof`，执行器 propagate 错误到 `Run_Program` 的 reply；节点存活但推理出错时，通过控制通道（request-response）发送 ERROR 命令。
- **用户取消**：Control 层调用 `Program_Queue::Cancel(job_id)`，cancel_flag 被设置，执行器在下一步检查时返回 `Err("Program cancelled")`，结果通过 reply 返回给 Control 层。
