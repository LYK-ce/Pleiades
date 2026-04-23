# ML Engine Reforge 设计文档

## 一、背景

原 ML Engine 的服务层（`ml_inference_service.rs`）设计时与 Control 层耦合：
- `Create_Session` 返回 `(Session_Handle, Receiver<Engine_Output>, Model_Info)`，由 Control 层持有 Handle 和输出通道
- Session 内部自建 `input_data_rx` / `output_data_tx` 通道对，用于 Control 层与 Session 之间传递文本
- `Engine_Input::Prompt(String)` 和 `Engine_Output::Text(String)` / `Engine_Output::End` 作为数据平面消息类型

现在 Orchestrator 取代 Control 层，同时 LLM_IO 模块提供了统一的前端⇄ML 文本通道（`IoFrontend` / `IoHandle`）。ML Engine 需要重构为 **Capability 模式**，与 LLM_IO、Storage 等模块保持一致的对外接口风格。

---

## 二、设计目标

1. **Capability 封装**：ML Engine 对外仅暴露 `ML_Engine_Capability` trait，所有内部类型（`Session_Handle`、`Session_Command` 等）对外不可见
2. **LLM_IO 集成**：文本 I/O 通道由 LLM_IO 提供的 `IoHandle` 替代原来的 `input_data_rx` / `output_data_tx`，消除 `Engine_Input` / `Engine_Output` 枚举
3. **Storage 集成**：模型文件通过 Storage 的 `file_id` 寻址，由 Storage 提供读锁保护；ML Engine 内部用 `std::fs::File::open(path)` 做同步 I/O（Candle 要求）
4. **Session 内部管理**：`ML_Engine_Service` 内部维护 Session 注册表，上层通过 `session_id` 操作，无需持有任何句柄
5. **职责清晰**：向上提供统一接口，向下管理 OS 线程和推理后端

---

## 三、架构总览

```
┌─────────────────────────────────────────────────────────┐
│                    Orchestrator                          │
│                                                         │
│  Core ──► Arc<Capabilities> ──► ml_engine 字段          │
│           │                     (Box<dyn ML_Engine_Capability>)
│           │                                             │
│  JobExecutor / TaskEngine                               │
│    └─ handler_inference.rs                              │
│       capabilities.ml_engine.Create_Session(config, io) │
│       capabilities.ml_engine.Run_Program(id, prog, ...) │
│       capabilities.ml_engine.Shutdown_Session(id)       │
└───────────────┬─────────────────────────────────────────┘
                │ (trait 接口)
                ▼
┌─────────────────────────────────────────────────────────┐
│              ML_Engine_Capability trait                   │
│              (Src/ML_Engine/capability.rs)                │
│                                                         │
│  Create_Session(config, io_handle) → Model_Info         │
│  Shutdown_Session(session_id)                            │
│  Run_Program(session_id, program, params, cancel)       │
│  Analyze_Model(file_id) → Model_Info                    │
│  Split_Model(source_id, start, end, output_id)          │
└───────────────┬─────────────────────────────────────────┘
                │ (impl)
                ▼
┌─────────────────────────────────────────────────────────┐
│              ML_Engine_Service                            │
│              (Src/ML_Engine/service.rs)                   │
│                                                         │
│  storage: Arc<StorageManager>                           │
│  sessions: Mutex<HashMap<String, SessionEntry>>         │
│                                                         │
│  SessionEntry {                                         │
│    handle: Session_Handle,                              │
│    _storage_guard: ReadGuard,  ← Storage 读锁守卫       │
│  }                                                      │
│                                                         │
│  Create_Session:                                        │
│    1. storage.acquire_read(file_id) → (PathBuf, Guard)  │
│    2. 创建 cmd 通道                                      │
│    3. spawn OS thread → Session_Thread(path, io, ...)   │
│    4. await ready → Model_Info                           │
│    5. 注册 SessionEntry(Handle + Guard) 到 sessions 表   │
│                                                         │
│  Run_Program:                                           │
│    1. 按 session_id 查表 → clone Session_Handle          │
│    2. 释放锁                                             │
│    3. handle.Run_Program(program, params, cancel)        │
│                                                         │
│  Shutdown_Session:                                      │
│    1. 按 session_id 查表 → remove SessionEntry           │
│    2. entry.handle.Shutdown()                            │
│    3. entry._storage_guard drop → 释放 Storage 读锁      │
└───────────────┬─────────────────────────────────────────┘
                │ (内部)
                ▼
┌─────────────────────────────────────────────────────────┐
│              Session_Handle (内部类型，不对外暴露)          │
│              (Src/ML_Engine/ml_thread_engine.rs)          │
│                                                         │
│  session_id: String                                     │
│  cmd_tx: mpsc::Sender<Session_Command>                  │
│                                                         │
│  Run_Program() → oneshot reply → Pipeline_Result        │
│  Shutdown() → 发送 Session_Command::Shutdown            │
└───────────────┬─────────────────────────────────────────┘
                │ (cmd 通道)
                ▼
┌─────────────────────────────────────────────────────────┐
│              Session_Thread (OS 线程)                     │
│              (Src/ML_Engine/ml_thread_engine.rs)          │
│                                                         │
│  Session {                                              │
│    id, cmd_rx,                                          │
│    io_handle: IoHandle,    ← LLM_IO 提供的文本 I/O      │
│    backend: Inference_Backend,                           │
│    config: Session_Config,                               │
│    tensor_io: Option<Tensor_IO_Handle>,                  │
│    register: Register_File,                              │
│  }                                                      │
│                                                         │
│  指令泵循环:                                              │
│    cmd_rx.recv() → Run_Program { program, params, ... }  │
│    Execute(program, session, params, cancel)              │
│                                                         │
│  指令执行时:                                              │
│    Input  → io_handle.input_rx.recv()  → String          │
│    Output → io_handle.output_tx.send() → String          │
│    Send/Receive → tensor_io                              │
└─────────────────────────────────────────────────────────┘
```

---

## 四、向上接口：ML_Engine_Capability

### 4.1 Trait 定义

```rust
#[async_trait]
pub trait ML_Engine_Capability: Send + Sync {
    /// 创建推理 Session
    ///
    /// 内部流程：
    /// 1. 通过 Storage 获取模型文件路径 + 读锁
    /// 2. 创建 cmd 通道
    /// 3. 启动 OS 线程，将物理路径和 io_handle 注入 Session_Thread
    /// 4. 等待 ready 信号，获取 Model_Info
    /// 5. 将 SessionEntry(Handle + ReadGuard) 注册到内部 sessions 表
    ///
    /// # 参数
    /// - config: Session 配置（model_file_id, layer_start, layer_end, device, tensor_io）
    /// - io_handle: LLM_IO 提供的 ML 侧文本通道端点
    ///
    /// # 返回
    /// - Model_Info: 模型信息（架构名称、层数、是否有 tokenizer 等）
    async fn Create_Session(
        &self,
        config: ML_Session_Config,
        io_handle: IoHandle,
    ) -> Result<Model_Info, ML_Engine_Error>;

    /// 关闭并移除指定 Session
    ///
    /// 内部流程：
    /// 1. 从 sessions 表中移除 SessionEntry
    /// 2. 通过 Handle 发送 Shutdown 命令
    /// 3. Session 线程退出，卸载模型
    /// 4. SessionEntry drop → ReadGuard drop → Storage 读锁释放
    ///
    /// 若 session_id 不存在，返回 SessionNotFound 错误。
    async fn Shutdown_Session(
        &self,
        session_id: &str,
    ) -> Result<(), ML_Engine_Error>;

    /// 提交指令序列到指定 Session 执行
    ///
    /// 内部流程：
    /// 1. 按 session_id 查 sessions 表，clone Handle（释放锁）
    /// 2. 通过 Handle 发送 Run_Program 命令
    /// 3. 等待 oneshot reply 获取 Pipeline_Result
    ///
    /// 执行期间，Session 线程会通过 io_handle 与前端交换文本。
    async fn Run_Program(
        &self,
        session_id: &str,
        program: Vec<Instruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<Pipeline_Result, ML_Engine_Error>;

    /// 分析模型文件结构（无需 Session，独立操作）
    ///
    /// 内部通过 Storage 获取临时读锁 + 路径，分析完自动释放。
    async fn Analyze_Model(
        &self,
        model_file_id: &str,
    ) -> Result<Model_Info, ML_Engine_Error>;

    /// 切分模型文件（无需 Session，独立操作）
    ///
    /// 内部通过 Storage 获取源文件读锁 + 输出文件写锁。
    async fn Split_Model(
        &self,
        source_file_id: &str,
        start: usize,
        end: usize,
        output_file_id: &str,
    ) -> Result<(), ML_Engine_Error>;
}
```

### 4.2 配置与返回类型

```rust
/// Session 创建配置
pub struct ML_Session_Config {
    pub session_id: String,
    pub model_file_id: String,     // ← Storage 的 file_id，替代 model_path
    pub layer_start: usize,
    pub layer_end: usize,
    pub device: String,
    pub tensor_io: Option<Tensor_IO_Handle>,
}

/// Capability 错误类型
pub enum ML_Engine_Error {
    /// Session 创建失败（模型加载错误、设备不支持等）
    SessionCreationFailed(String),
    /// 指定 session_id 不存在
    SessionNotFound(String),
    /// 指令序列执行失败
    ProgramFailed(String),
    /// 模型分析失败
    ModelAnalysisFailed(String),
    /// 模型切分失败
    ModelSplitFailed(String),
}
```

### 4.3 上层使用示例

```rust
// Orchestrator handler_inference.rs 中的调用流程

// Step 1: LLM_IO 分配通道
let channels = capabilities.io_broker.Allocate(job_id).await?;
// channels.frontend → 交给前端（TUI）
// channels.ml_side  → 注入 ML Session

// Step 2: 创建 Session（使用 Storage file_id 而非直接路径）
let config = ML_Session_Config {
    session_id: "sess-001".to_string(),
    model_file_id: "qwen3-0.6b.gguf".to_string(),  // Storage file_id
    layer_start: 0,
    layer_end: 29,  // 全部层
    device: "cpu".to_string(),
    tensor_io: None, // 单机推理
};
let model_info = capabilities.ml_engine
    .Create_Session(config, channels.ml_side)
    .await?;

// Step 3: 提交推理程序
let program = vec![
    Instruction::Input,
    Instruction::Encode,
    Instruction::Set { target: Set_Target::Meta(META2, 120.0) },
    Instruction::Prefill { input: TOKENID3 },
    Instruction::CopyMeta { src: META5, dst: META1 },
    Instruction::Sample { tensor_reg: TENSOR2 },
    Instruction::Decode,
    Instruction::Output,
    Instruction::Loop {
        body: vec![
            Instruction::BreakIf { flag: FLAG1 },
            Instruction::Inference { input: Inference_Input::Tokens(TOKENID2) },
            Instruction::Sample { tensor_reg: TENSOR2 },
            Instruction::Decode,
            Instruction::Output,
        ],
    },
    Instruction::EndOutput,
];
let cancel = Arc::new(AtomicBool::new(false));
let result = capabilities.ml_engine
    .Run_Program("sess-001", program, params, cancel)
    .await?;

// Step 4: 清理
capabilities.ml_engine.Shutdown_Session("sess-001").await?;
capabilities.io_broker.Deallocate(job_id).await?;
```

---

## 五、向下接口：Session_Handle → Session_Thread

### 5.1 Session_Handle（内部类型）

```rust
/// Session 句柄 — ML_Engine 模块内部使用，不对外暴露
///
/// 仅包含命令通道发送端。文本 I/O 由 LLM_IO 的 IoHandle 直接注入 Session_Thread。
#[derive(Clone)]
pub(crate) struct Session_Handle {
    session_id: String,
    cmd_tx: mpsc::Sender<Session_Command>,
}
```

与旧设计对比：

| 字段 | 旧设计 | 新设计 |
|------|--------|--------|
| `session_id` | ✅ 保留 | ✅ 保留 |
| `cmd_tx` | ✅ 保留 | ✅ 保留 |
| `input_data_tx` | 存在 — 向 Session 发送 prompt | ❌ **删除** — 由 IoHandle 替代 |

删除的方法：

| 方法 | 原因 |
|------|------|
| `Send_Input()` | 前端通过 `IoFrontend.input_tx` 直接发送 prompt |

保留的方法：

| 方法 | 作用 |
|------|------|
| `Run_Program()` | 通过 cmd_tx 发送指令序列，await oneshot reply |
| `Shutdown()` | 通过 cmd_tx 发送 Shutdown 命令 |

### 5.2 Session_Command（命令平面）

```rust
/// 保持不变
pub(crate) enum Session_Command {
    Run_Program {
        program: Vec<Instruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<Pipeline_Result>>,
    },
    Shutdown,
}
```

### 5.3 Session（线程内部状态）

```rust
/// 推理会话容器 — 线程私有
pub(crate) struct Session {
    pub id: String,

    // 命令平面
    pub cmd_rx: mpsc::Receiver<Session_Command>,

    // 文本 I/O 平面（来自 LLM_IO）
    pub io_handle: IoHandle,

    // 推理后端
    pub backend: Inference_Backend,
    pub config: Session_Config,

    // 网络数据平面（分布式推理）
    pub tensor_io: Option<Tensor_IO_Handle>,

    // 寄存器组
    pub register: Register_File,
}
```

与旧设计对比：

| 字段 | 旧设计 | 新设计 |
|------|--------|--------|
| `cmd_rx` | ✅ 保留 | ✅ 保留 |
| `input_data_rx` | 存在 — 接收 Engine_Input | ❌ **删除** — 由 `io_handle.input_rx` 替代 |
| `output_data_tx` | 存在 — 发送 Engine_Output | ❌ **删除** — 由 `io_handle.output_tx` 替代 |
| `io_handle` | 不存在 | ✅ **新增** — LLM_IO 的 ML 侧端点 |
| `backend` | ✅ 保留 | ✅ 保留 |
| `config` | ✅ 保留 | ✅ 保留 |
| `tensor_io` | ✅ 保留 | ✅ 保留 |
| `register` | ✅ 保留 | ✅ 保留 |

### 5.4 Session_Thread 入口

```rust
pub(crate) fn Session_Thread(
    session_id: String,
    session_config: Session_Config,
    cmd_rx: mpsc::Receiver<Session_Command>,
    io_handle: IoHandle,                      // ← 替代 input_data_rx + output_data_tx
    tensor_io: Option<Tensor_IO_Handle>,
    ready_tx: oneshot::Sender<Result<Model_Info>>,
) {
    // 1. 阻塞加载模型
    // 2. 初始化寄存器
    // 3. 发送 ready 信号 (Model_Info)
    // 4. 创建 Session 实例（含 io_handle）
    // 5. 进入指令泵循环
}
```

### 5.5 指令执行变更

| 指令 | 旧行为 | 新行为 |
|------|--------|--------|
| `Input` | `input_data_rx.blocking_recv()` → `Engine_Input::Prompt(text)` → TEXT1 | `io_handle.input_rx.blocking_recv()` → `String` → TEXT1 |
| `Output` | TEXT2 → `output_data_tx.blocking_send(Engine_Output::Text(text))` | TEXT2 → `io_handle.output_tx.blocking_send(text)` |
| `EndOutput` | `output_data_tx.blocking_send(Engine_Output::End)` | drop `io_handle.output_tx` 或 noop（由 LLM_IO 通道生命周期管理） |

---

## 六、可删除的类型

| 类型 | 所在文件 | 删除原因 |
|------|----------|----------|
| `Engine_Input` | ml_thread_engine_instruction.rs | LLM_IO 直接传 `String` |
| `Engine_Output` | ml_thread_engine_instruction.rs | LLM_IO 直接传 `String`；`End` 由通道关闭替代；`Info` 由 Create_Session 返回 |
| `Session_Handle::Send_Input()` | ml_thread_engine.rs | 前端直接通过 IoFrontend.input_tx 发送 |
| `Session_Handle::input_data_tx` | ml_thread_engine.rs | 被 IoHandle.input_rx 替代 |
| `Session::input_data_rx` | ml_thread_engine.rs | 被 io_handle.input_rx 替代 |
| `Session::output_data_tx` | ml_thread_engine.rs | 被 io_handle.output_tx 替代 |

---

## 七、Storage 集成

### 7.1 设计原则

Storage 负责**锁管理 + 路径解析 + 文件注册表**，ML Engine 自行做同步 I/O。

详见 `Src/Storage/Storage_reforge.md`。

### 7.2 ML_Engine_Service 与 Storage 的关系

```rust
pub struct ML_Engine_Service {
    /// Storage 引用 — 用于路径解析和文件锁
    storage: Arc<StorageManager>,
    /// Session 注册表
    sessions: Mutex<HashMap<String, SessionEntry>>,
}

/// Session 注册条目
struct SessionEntry {
    /// 内部 Session 句柄
    handle: Session_Handle,
    /// Storage 读锁守卫 — Session 存活期间保护模型文件
    _storage_guard: ReadGuard,
}
```

### 7.3 文件访问流程

**Create_Session**：
```
ML_Engine_Service                    StorageManager
      │                                    │
      │─── acquire_read(file_id) ─────────►│
      │◄── (PathBuf, ReadGuard) ───────────│
      │                                    │
      │ spawn OS 线程, 传入 PathBuf         │
      │ 线程内: std::fs::File::open(path)  │
      │ 保存 ReadGuard 到注册表             │
```

**Shutdown_Session**：
```
ML_Engine_Service                    StorageManager
      │                                    │
      │ 移除 SessionEntry                   │
      │ entry.handle.Shutdown()             │
      │ entry._storage_guard drop ─────────►│ 读锁释放
```

**Analyze_Model**：
```
ML_Engine_Service                    StorageManager
      │                                    │
      │─── acquire_read(file_id) ─────────►│
      │◄── (PathBuf, ReadGuard) ───────────│
      │                                    │
      │ GGUF_Analyze(&path)                │
      │ ReadGuard drop ───────────────────►│ 读锁释放
```

**Split_Model**：
```
ML_Engine_Service                    StorageManager
      │                                    │
      │─── acquire_read(source_id) ───────►│
      │◄── (src_path, ReadGuard) ──────────│
      │─── acquire_write(output_id) ──────►│
      │◄── (out_path, WriteGuard) ─────────│
      │                                    │
      │ GGUF_Split_Model(&src, &out)       │
      │ Guards drop ──────────────────────►│ 锁释放
```

### 7.4 底层不变

`gguf_model_manager.rs` 内部的 `GGUF_Analyze`、`GGUF_Load_Layer`、`GGUF_Split_Model` 等函数
仍接收 `&Path` 参数，仍使用 `std::fs::File::open(path)` 做同步 I/O。
路径解析和锁管理由 `ML_Engine_Service` 在上层完成。

---

## 八、文件变更清单

### 新增文件

| 文件 | 职责 |
|------|------|
| `Src/ML_Engine/capability.rs` | ML_Engine_Capability trait、ML_Engine_Error、ML_Session_Config |
| `Src/ML_Engine/service.rs` | ML_Engine_Service 实现 ML_Engine_Capability，持有 Storage 引用 + Session 注册表 |

### 修改文件

| 文件 | 变更内容 |
|------|----------|
| `Src/ML_Engine/mod.rs` | 添加 `pub mod capability; pub mod service;` 导出；移除 Engine_Input/Output 导出 |
| `Src/ML_Engine/ml_thread_engine.rs` | Session 结构体用 io_handle 替代 input_data_rx/output_data_tx；Session_Handle 删除 input_data_tx 和 Send_Input；降低可见性为 pub(crate) |
| `Src/ML_Engine/ml_thread_engine_instruction.rs` | 删除 Engine_Input 和 Engine_Output 枚举 |
| `Src/ML_Engine/ml_inference_service.rs` | 逻辑搬入 service.rs，此文件可删除或降级为内部辅助 |
| `Src/Storage/guard.rs` | 新增 ReadGuard / WriteGuard（替代 handle.rs） |
| `Src/Storage/capability.rs` | `open_read` → `acquire_read`，`open_write` → `acquire_write` |
| `Src/Storage/manager.rs` | impl 改为返回 `(PathBuf, Guard)` |
| `Src/Storage/handle.rs` | 删除（被 guard.rs 替代） |
| `Src/Orchestrator/mod.rs` | Capabilities 中 `inference` 字段替换为 `ml_engine: Box<dyn ML_Engine_Capability>` |
| `Src/Orchestrator/executor/mod.rs` | 删除 InferenceCapability trait |
| `Src/Orchestrator/executor/handler_inference.rs` | 改为调用 capabilities.ml_engine 的方法 |

---

## 九、数据流对比

### 旧数据流（Control 层时代）

```
Frontend → Control → input_data_tx ──→ input_data_rx → Session
Session → output_data_tx ──→ output_data_rx → Control → Frontend
```

- Control 中转所有文本数据
- Engine_Input / Engine_Output 作为通道消息类型

### 新数据流（Orchestrator + LLM_IO）

```
Frontend (IoFrontend)                           Session (IoHandle)
  input_tx  ───────── mpsc ─────────→  input_rx    (String)
  output_rx ←──────── mpsc ─────────  output_tx    (String)
```

- Orchestrator 仅负责 Session 生命周期编排，不经手文本数据
- 文本通过 LLM_IO 通道直连前端与 Session
- 通道由 LLM_IO_Broker.Allocate() 创建，IoHandle 在 Create_Session 时注入

---

## 十、Session 管理状态机

```
                  Create_Session(config, io_handle)
                           │
                           ▼
                   ┌──────────────┐
                   │   Loading    │  OS 线程加载模型
                   └──────┬───────┘
                          │ ready (Model_Info)
                          ▼
                   ┌──────────────┐
                   │    Idle      │  等待命令
                   └──────┬───────┘
                          │ Run_Program
                          ▼
                   ┌──────────────┐
                   │  Executing   │  执行指令序列（读写 IoHandle）
                   └──────┬───────┘
                          │ 完成 / 错误
                          ▼
                   ┌──────────────┐
                   │    Idle      │  等待下一个命令
                   └──────┬───────┘
                          │ Shutdown_Session
                          ▼
                   ┌──────────────┐
                   │  Terminated  │  线程退出，模型卸载
                   └──────────────┘
```

ML_Engine_Service 的 sessions 注册表仅在 **Idle** 和 **Executing** 阶段包含该 Session。Shutdown_Session 从注册表移除并发送关闭命令。

---

## 十一、实施顺序

> **前置条件**：Storage Reforge 已完成（guard.rs / acquire_read / acquire_write），LLM_IO 模块已完成（IoHandle / IoFrontend / LLM_IO_Broker）。

### 步骤 1：新增 ML Engine Capability 层（纯新文件，不破坏现有编译）

- **新建** `Src/ML_Engine/capability.rs`
  - 定义 `ML_Engine_Capability` trait（5 个异步方法）
  - 定义 `ML_Engine_Error` 错误枚举
  - 定义 `ML_Session_Config` 配置结构体
- **新建** `Src/ML_Engine/service.rs`
  - 实现 `ML_Engine_Service` 结构体（持有 `Arc<StorageManager>` + `Mutex<HashMap<String, SessionEntry>>`）
  - `SessionEntry` = `Session_Handle` + `ReadGuard`
  - impl `ML_Engine_Capability` for `ML_Engine_Service`（先用占位符）

### 步骤 2：修改 Session 内部接入 IoHandle（ML Engine 核心变更）

- **修改** `Src/ML_Engine/ml_thread_engine.rs`
  - `Session` 结构体：删除 `input_data_rx: Receiver<Engine_Input>` + `output_data_tx: Sender<Engine_Output>`，新增 `io_handle: IoHandle`
  - `Session_Handle`：删除 `input_data_tx` 字段和 `Send_Input()` 方法；降可见性为 `pub(crate)`
  - `Session_Thread()`：参数 `input_data_rx` + `output_data_tx` 替换为 `io_handle: IoHandle`
  - `Session_Config`：`model_path: PathBuf` 保留（物理路径由 service 层通过 Storage 解析后传入）

### 步骤 3：删除旧通道消息类型

- **修改** `Src/ML_Engine/ml_thread_engine_instruction.rs`
  - 删除 `Engine_Input` 枚举
  - 删除 `Engine_Output` 枚举
- **修改** `Src/ML_Engine/ml_thread_engine.rs` 中的指令执行器
  - `Input` 指令：`input_data_rx.blocking_recv()` → `io_handle.input_rx.blocking_recv()` → `String`
  - `Output` 指令：`output_data_tx.blocking_send(Engine_Output::Text(...))` → `io_handle.output_tx.blocking_send(text)`
  - `EndOutput` 指令：`Engine_Output::End` → noop 或 drop output_tx

### 步骤 4：迁移服务层逻辑

- **修改** `Src/ML_Engine/service.rs`
  - 将 `ml_inference_service.rs` 中 `Create_Session` 的逻辑搬入 `ML_Engine_Service::Create_Session`（使用 Storage acquire_read + IoHandle）
  - 将 `Split_Model` / `Analyze_Model` 搬入并接入 Storage（acquire_read / acquire_write）
- **删除或降级** `Src/ML_Engine/ml_inference_service.rs`

### 步骤 5：更新 ML Engine 模块导出

- **修改** `Src/ML_Engine/mod.rs`
  - 新增 `pub mod capability; pub mod service;`
  - 移除 `Engine_Input` / `Engine_Output` 导出
  - 将 `Session` / `Session_Handle` / `Session_Command` 降为 `pub(crate)`（不对外暴露）
  - 新增 `ML_Engine_Capability` / `ML_Engine_Error` / `ML_Session_Config` / `ML_Engine_Service` 导出

### 步骤 6：Orchestrator 侧集成

- **修改** `Src/Orchestrator/mod.rs`
  - `Capabilities` 中 `inference: Box<dyn InferenceCapability>` → `ml_engine: Box<dyn ML_Engine_Capability>`
- **修改** `Src/Orchestrator/executor/mod.rs`
  - 删除 `InferenceCapability` trait
- **修改** `Src/Orchestrator/executor/handler_inference.rs`
  - 改为调用 `self.capabilities.ml_engine.Create_Session(config, io)` 等方法
- **修改** 所有 Orchestrator 测试中的 `stub_caps()`
  - Stub `ML_Engine_Capability` 替代 Stub `InferenceCapability`

### 步骤 7：更新 lib.rs 重导出 + 最终验证

- **修改** `Src/lib.rs`
  - 移除 `Engine_Input` / `Engine_Output` / `Session` / `Session_Handle` 等旧导出
  - 新增 `ML_Engine_Capability` / `ML_Engine_Error` / `ML_Session_Config` / `ML_Engine_Service` 导出
- **编译验证** + **全量测试**
