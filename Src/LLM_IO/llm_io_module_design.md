# LLM_IO 模块设计文档（Phase 2）

## 一、设计概述

### 1.1 功能定位

LLM_IO 是系统的**大语言模型文本交互层**，负责为外部前端（TUI 或 API）与 ML Thread 之间建立**有状态的双向文本通道**。

- **输入**：外部前端发送的文本 Prompt（`String`）
- **输出**：ML Thread 返回的文本 Completion（`String`）
- **非目标**：Token 流推送、文件传输、多模态输入（由其他模块负责）

### 1.2 核心设计原则

1. **有状态会话**：先创建 Session，再持续多轮对话，避免每轮重复加载模型
2. **前端二选一**：同一时刻只存在一个输入源（TUI 模式 或 API 模式）
3. **通道由前端申请**：`LLM_IO_Broker` 分配通道后，前端保留外侧端点，将 ML 侧端点注入 Job
4. **零拷贝直传**：Prompt/Completion 不经过 Core 内核、不经过 JobExecutor 循环，直传 ML Thread
5. **不维护历史**：对话上下文由 ML Thread 内部通过 KV Cache 维护，LLM_IO 只负责单轮文本转发

### 1.3 用法概览

```rust
// 1. 创建 Broker
let broker = LLM_IO_Broker::new();

// 2. 为指定 Job 分配通道
let channels = broker.allocate(JobId(42)).await?;

// 3. 前端保留外侧端点
let frontend = channels.frontend;  // input_tx + output_rx

// 4. ML 侧端点随 Job 注入编排层
let program = compiler.compile_run(job_id, model_path, channels.ml_side);
```

---

## 二、文件组成

模块位于 `src/llm_io/` 目录，由 3 个源文件组成。所有单元测试均以 `#[cfg(test)]` 内联模块形式直接写在对应源文件底部。

```
src/llm_io/
├── mod.rs              # 模块入口：聚合导出
├── capability.rs       # 契约层：错误类型、通道结构、trait 定义
└── broker.rs           # 实现层：LLM_IO_Broker 结构体与全部业务逻辑
```

| 文件 | 职责 | 公开内容 |
|------|------|---------|
| `capability.rs` | 定义 `LLM_IO_Capability` trait、`LLM_IO_Error`、`IoChannels`、`IoFrontend`、`IoHandle` | 全部公开 |
| `broker.rs` | `LLM_IO_Broker` 结构体；通道分配、回收、状态查询 | `LLM_IO_Broker` |
| `mod.rs` | 通过 `pub use` 将上述类型提升到模块级；底部附模块级集成测试 | 模块聚合 |

---

## 三、Capabilities

### 3.1 Trait 定义

```rust
#[async_trait]
pub trait LLM_IO_Capability: Send + Sync {
    /// 为指定 Job 创建一组双向文本通道。
    /// 返回前端侧端点（IoFrontend）和 ML 侧端点（IoHandle）。
    async fn allocate(&self, job_id: JobId) -> Result<IoChannels, LLM_IO_Error>;

    /// 强制关闭指定 Job 的通道，清理内部索引。
    /// 若 job_id 不存在，幂等返回 Ok。
    async fn deallocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error>;

    /// 查询指定 Job 的通道是否仍存在于内部索引中。
    async fn is_active(&self, job_id: JobId) -> bool;
}
```

### 3.2 功能说明

| 方法 | 调用方 | 行为 |
|------|--------|------|
| `allocate` | Core / 前端 | 创建 `mpsc` 双向通道，注册到内部索引，返回两端句柄 |
| `deallocate` | Core / 前端 | 从索引移除条目，不强制关闭底层 `mpsc`（Drop 由所有者负责） |
| `is_active` | Core / 前端 | 检查内部索引中是否存在该 job_id 的通道记录 |

---

## 四、文件内容

### 4.1 capability.rs — 契约层

#### 公开类型

- **`LLM_IO_Error`**：错误枚举
  - `AllocationFailed(String)` — 通道创建失败（如内存不足）
  - `InvalidJobId(String)` — job_id 格式非法

- **`IoChannels`**：`allocate` 的返回类型
  - `frontend: IoFrontend`
  - `ml_side: IoHandle`

- **`IoFrontend`**：前端持有的外侧端点
  - `input_tx: mpsc::Sender<String>` — 发送 Prompt
  - `output_rx: mpsc::Receiver<String>` — 接收 Completion

- **`IoHandle`**：注入 Job 的 ML 侧端点
  - `input_rx: mpsc::Receiver<String>` — 接收 Prompt
  - `output_tx: mpsc::Sender<String>` — 发送 Completion

- **`LLM_IO_Capability`**：trait 定义（见 3.1）

#### 内联测试（capability.rs 底部）

```rust
#[cfg(test)]
mod tests {
    // test_io_channels_construct_and_destructure
    //   验证 IoChannels 能正确构造，且 frontend/ml_side 字段可访问

    // test_io_frontend_clone_sender
    //   验证 input_tx 可以 Clone（多前端场景预留）

    // test_llm_io_error_display
    //   验证 LLM_IO_Error 的 Display 输出格式正确
}
```

### 4.2 broker.rs — 实现层

#### 内部结构

```rust
pub struct LLM_IO_Broker {
    channels: std::sync::Mutex<std::collections::HashMap<JobId, ChannelEntry>>,
}

struct ChannelEntry {
    frontend_input_tx: mpsc::Sender<String>,   // 保留引用用于 deallocate 清理
    ml_output_tx: mpsc::Sender<String>,        // 保留引用用于 deallocate 清理
}
```

#### 实现逻辑

- **`new()`**：创建空索引表
- **`allocate(job_id)`**：
  1. 创建 `input_tx/input_rx`（缓冲 64）
  2. 创建 `output_tx/output_rx`（缓冲 64）
  3. 组装 `IoFrontend { input_tx, output_rx }` 和 `IoHandle { input_rx, output_tx }`
  4. 在索引中插入 `ChannelEntry`（保留两份 Sender 用于后续查询/清理）
  5. 返回 `IoChannels`
- **`deallocate(job_id)`**：
  1. 获取索引写锁
  2. 移除对应条目（若存在）
  3. 返回 `Ok(())`
- **`is_active(job_id)`**：
  1. 获取索引读锁
  2. 检查条目是否存在

#### 内联测试（broker.rs 底部）

```rust
#[cfg(test)]
mod tests {
    // test_allocate_returns_valid_channels
    //   验证 allocate 返回的 IoChannels 两端非空，Sender/Receiver 可正常收发

    // test_allocate_registers_in_index
    //   验证 allocate 后 is_active(job_id) 返回 true

    // test_deallocate_removes_from_index
    //   验证 deallocate 后 is_active(job_id) 返回 false

    // test_deallocate_idempotent
    //   验证对不存在的 job_id 调用 deallocate 不 panic，返回 Ok

    // test_frontend_drop_closes_ml_input_rx
    //   验证前端 IoFrontend Drop 后，ML 侧的 input_rx.recv() 返回 None

    // test_ml_side_drop_closes_frontend_output_rx
    //   验证 IoHandle Drop 后，前端的 output_rx.recv() 返回 None

    // test_multiple_allocate_same_job_id_overwrites
    //   验证对同一 job_id 重复 allocate 时，旧条目被覆盖（或返回错误，视设计而定）
}
```

### 4.3 mod.rs — 模块入口

#### 聚合导出

```rust
pub use capability::{LLM_IO_Capability, LLM_IO_Error, IoChannels, IoFrontend, IoHandle};
pub use broker::LLM_IO_Broker;
```

#### 内联测试（mod.rs 底部）

```rust
#[cfg(test)]
mod tests {
    // test_module_imports_compile
    //   验证外部视角的 use 路径正确

    // test_end_to_end_allocate_chat_drop
    //   端到端测试：allocate → 前端发 Prompt → ML 侧收 → ML 侧发回复 → 前端收 → deallocate
}
```

---

## 五、测试说明

### 5.1 测试策略

| 层级 | 位置 | 验证目标 |
|------|------|---------|
| 结构测试 | `capability.rs` | 类型构造、字段访问、错误格式化 |
| 行为测试 | `broker.rs` | 分配/回收/查询的正确性、通道关闭语义、幂等性 |
| 集成测试 | `mod.rs` | 模块导入路径、端到端收发流程 |

### 5.2 异步运行时

所有测试使用 `#[tokio::test]`。

### 5.3 并发安全

`LLM_IO_Broker` 内部使用 `std::sync::Mutex`（而非 `tokio::sync::Mutex`），因为锁内操作极短（HashMap 增删查），不会阻塞异步运行时。

---

## 六、开放接口汇总

### 6.1 核心 Trait

```rust
#[async_trait]
pub trait LLM_IO_Capability: Send + Sync {
    async fn allocate(&self, job_id: JobId) -> Result<IoChannels, LLM_IO_Error>;
    async fn deallocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error>;
    async fn is_active(&self, job_id: JobId) -> bool;
}
```

### 6.2 通道结构

```rust
pub struct IoChannels {
    pub frontend: IoFrontend,
    pub ml_side: IoHandle,
}

pub struct IoFrontend {
    pub input_tx: mpsc::Sender<String>,
    pub output_rx: mpsc::Receiver<String>,
}

pub struct IoHandle {
    pub input_rx: mpsc::Receiver<String>,
    pub output_tx: mpsc::Sender<String>,
}
```

### 6.3 错误类型

```rust
pub enum LLM_IO_Error {
    AllocationFailed(String),
    InvalidJobId(String),
}
```

---

## 七、接口用法示例

### 7.1 allocate — 创建会话通道

为指定 Job 创建双向文本通道。前端保留 `IoFrontend`，`IoHandle` 注入 JobExecutor。

```rust
use pleiades::llm_io::{LLM_IO_Broker, LLM_IO_Capability};
use pleiades::job::JobId;

let broker = LLM_IO_Broker::new();
let channels = broker.allocate(JobId(42)).await?;

// 前端保留外侧端点
let mut frontend = channels.frontend;

// ML 侧端点随 Job 注入编排层
let io_handle = channels.ml_side;
// ... 将 io_handle 通过 UserCommand::Run 传给 Core ...
```

### 7.2 前端发送 Prompt

```rust
frontend.input_tx.send("你好".to_string()).await?;
let completion = frontend.output_rx.recv().await?;
println!("模型回复: {}", completion);
```

### 7.3 ML Thread 消费与回复

```rust
// 在 InferenceCapability::create_session 内部
tokio::spawn(async move {
    while let Some(prompt) = io_handle.input_rx.recv().await {
        let reply = model.generate(&prompt).await; // 伪代码
        let _ = io_handle.output_tx.send(reply).await;
    }
    // input_rx 关闭时循环结束
});
```

### 7.4 deallocate — 清理通道索引

Job 结束或前端断开时，清理 Broker 内部索引（幂等）。

```rust
broker.deallocate(JobId(42)).await?;
assert!(!broker.is_active(JobId(42)).await);
```

### 7.5 Core 集成示例

```rust
fn route_user(&mut self, cmd: UserCommand) {
    match cmd {
        UserCommand::Run { model_path } => {
            let job_id = JobId(generate_id());
            let channels = self.llm_io.allocate(job_id).await.unwrap();

            // 前端（TUI）保留外侧端点
            self.active_sessions.insert(job_id, channels.frontend);

            // 编译 TaskProgram，将 ml_side 注入
            let program = self.compiler.compile_run(job_id, model_path, channels.ml_side);
            self.spawn_job(job_id, JobKind::Run, program);
        }
    }
}
```

---

## 八、Phase 2 范围

| 功能 | Phase 2 | Phase 3 |
|------|---------|---------|
| 文本 Prompt / Completion 通道 | ✅ 必须实现 | — |
| `LLM_IO_Broker` 分配与回收 | ✅ 必须实现 | — |
| `IoHandle` / `IoFrontend` 定义 | ✅ 必须实现 | — |
| 通道缓冲大小固定（64） | ✅ 必须实现 | ⏳ 可配置 |
| Token 流推送 | — | ⏳ 其他模块 |
| 多前端并发共享同一 Job | — | ⏳ 待设计 |
| 通道持久化/断线重连 | — | ⏳ 待设计 |
