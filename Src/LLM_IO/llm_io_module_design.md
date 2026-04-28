# LLM_IO 模块设计文档（Phase 2，更新 2026-04-27）

## 一、设计概述

### 1.1 功能定位

LLM_IO 是系统的**大语言模型文本交互层**，负责为外部前端（TUI 或 API）与 ML Thread 之间建立**有状态的双向文本通道**。

- **输入**：外部前端发送的文本 Prompt（`String`）
- **输出**：ML Thread 返回的文本 Completion（`String`）
- **非目标**：Token 流推送、文件传输、多模态输入（由其他模块负责）

### 1.2 核心设计原则

1. **有状态会话**：先创建 Session，再持续多轮对话，避免每轮重复加载模型
2. **前端二选一**：同一时刻只存在一个输入源（TUI 模式 或 API 模式）
3. **Broker 托管分发**：编排层调用 `Allocate` 创建通道对并托管于 Broker，编排层取出 ML 侧端点注入 Job，前端通过 `Take_Frontend` 取出前端侧端点进行交互
4. **零拷贝直传**：Prompt/Completion 不经过 Core 内核、不经过 JobExecutor 循环，直传 ML Thread
5. **不维护历史**：对话上下文由 ML Thread 内部通过 KV Cache 维护，LLM_IO 只负责单轮文本转发

### 1.3 用法概览

```rust
// 1. 创建 Broker（通常在 Capabilities 中全局持有）
let broker = LLM_IO_Broker::New();

// 2. 编排层为指定 Job 分配通道（两端托管于 Broker）
broker.Allocate(job_id).await?;

// 3. 编排层取出 ML 侧端点，注入 JobExecutor
let io_handle = broker.Take_ML_Side(job_id).await?;
spawn_job(job_id, program, io_handle);

// 4. 前端取出前端侧端点，进行文本交互
let frontend = broker.Take_Frontend(job_id).await?;
frontend.input_tx.send("你好".to_string()).await?;
while let Some(token) = frontend.output_rx.recv().await {
    print!("{}", token);
}
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
| `capability.rs` | 定义 `LLM_IO_Capability` trait、`LLM_IO_Error`、`IoFrontend`、`IoHandle` | 全部公开 |
| `broker.rs` | `LLM_IO_Broker` 结构体；通道分配、托管、分发和回收 | `LLM_IO_Broker` |
| `mod.rs` | 通过 `pub use` 将上述类型提升到模块级；底部附模块级集成测试 | 模块聚合 |

---

## 三、Capabilities

### 3.1 Trait 定义

```rust
#[async_trait]
pub trait LLM_IO_Capability: Send + Sync {
    /// 为指定 Job 创建一组双向文本通道，两端由 Broker 内部托管。
    async fn Allocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error>;

    /// 取出指定 Job 的 ML 侧端点（IoHandle），take 语义，只能取一次。
    async fn Take_ML_Side(&self, job_id: JobId) -> Result<IoHandle, LLM_IO_Error>;

    /// 取出指定 Job 的前端侧端点（IoFrontend），take 语义，只能取一次。
    async fn Take_Frontend(&self, job_id: JobId) -> Result<IoFrontend, LLM_IO_Error>;

    /// 从内部索引中移除指定 Job 的通道条目。
    /// 仅清理内部索引，通道的实际生命周期由两端句柄的 Drop 决定。
    async fn Deallocate(&self, job_id: JobId) -> Result<(), LLM_IO_Error>;

    /// 查询指定 Job 的通道是否仍存在于内部索引中。
    async fn Is_Active(&self, job_id: JobId) -> bool;
}
```

### 3.2 功能说明

| 方法 | 调用方 | 行为 |
|------|--------|------|
| `Allocate` | 编排层 (Core) | 创建 `mpsc` 双向通道，两端存入内部索引，不返回端点 |
| `Take_ML_Side` | 编排层 (Core) | 从索引取出 ML 侧端点（take 语义），注入 JobExecutor |
| `Take_Frontend` | 前端 / 测试 | 从索引取出前端侧端点（take 语义），用于文本交互 |
| `Deallocate` | 编排层 (Core) | 从索引移除条目，不强制关闭底层 `mpsc`（Drop 由所有者负责） |
| `Is_Active` | 编排层 / 前端 | 检查内部索引中是否存在该 job_id 的通道记录 |

---

## 四、文件内容

### 4.1 capability.rs — 契约层

#### 公开类型

- **`LLM_IO_Error`**：错误枚举
  - `AllocationFailed(String)` — 通道创建失败（如 job_id 已存在）
  - `NotFound(String)` — 指定 job_id 未找到或端点已被取走

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
    // test_io_endpoints_construct
    //   验证 IoFrontend 和 IoHandle 能正确构造

    // test_io_frontend_clone_sender
    //   验证 input_tx 可以 Clone（多前端场景预留）

    // test_llm_io_error_display
    //   验证 LLM_IO_Error 的 Display 输出格式正确（含 NotFound）
}
```

### 4.2 broker.rs — 实现层

#### 内部结构

```rust
pub struct LLM_IO_Broker {
    channels: tokio::sync::Mutex<HashMap<JobId, ChannelEntry>>,
}

struct ChannelEntry {
    _frontend_input_tx: mpsc::Sender<String>,   // 保留 Sender 克隆用于 Deallocate 时加速通道关闭
    _ml_output_tx: mpsc::Sender<String>,        // 保留 Sender 克隆用于 Deallocate 时加速通道关闭
    ml_side: Option<IoHandle>,                   // Take_ML_Side 后变为 None
    frontend: Option<IoFrontend>,                // Take_Frontend 后变为 None
}
```

#### 实现逻辑

- **`New()`**：创建空索引表
- **`Allocate(job_id)`**：
  1. 若 job_id 已存在，返回 `AllocationFailed` 错误
  2. 创建 `input_tx/input_rx`（缓冲 64）
  3. 创建 `output_tx/output_rx`（缓冲 64）
  4. 组装 `IoFrontend { input_tx.clone(), output_rx }` 和 `IoHandle { input_rx, output_tx.clone() }`
  5. 在索引中插入 `ChannelEntry`（Sender 克隆 + 两端 `Option::Some`）
  6. 返回 `Ok(())`
- **`Take_ML_Side(job_id)`**：
  1. 查找条目，不存在返回 `NotFound`
  2. `Option::take()` 取出 `IoHandle`，已被取走返回 `NotFound`
- **`Take_Frontend(job_id)`**：
  1. 查找条目，不存在返回 `NotFound`
  2. `Option::take()` 取出 `IoFrontend`，已被取走返回 `NotFound`
- **`Deallocate(job_id)`**：
  1. 移除对应条目（若存在），幂等返回 `Ok(())`
  2. 仅清理内部索引，通道的实际生命周期由两端句柄的 Drop 决定
- **`Is_Active(job_id)`**：
  1. 检查条目是否存在

#### 内联测试（broker.rs 底部，12 个）

```rust
#[cfg(test)]
mod tests {
    // test_allocate_and_take_channels — Allocate + Take 两端后可正常收发
    // test_allocate_registers_in_index — Allocate 后 Is_Active 返回 true
    // test_deallocate_removes_from_index — Deallocate 后 Is_Active 返回 false
    // test_deallocate_idempotent — 对不存在的 job_id Deallocate 不 panic
    // test_frontend_drop_closes_ml_input_rx — Take + Deallocate + drop frontend → ML recv None
    // test_ml_side_drop_closes_frontend_output_rx — Take + Deallocate + drop ml → frontend recv None
    // test_duplicate_allocate_returns_error — 重复 Allocate 返回 AllocationFailed
    // test_reallocate_after_deallocate — Deallocate 后可重新 Allocate
    // test_take_ml_side_not_found — 不存在的 job_id Take 返回 NotFound
    // test_take_frontend_not_found — 不存在的 job_id Take 返回 NotFound
    // test_take_ml_side_already_taken — 重复 Take 返回 NotFound（已被取走）
    // test_take_frontend_already_taken — 重复 Take 返回 NotFound（已被取走）
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
