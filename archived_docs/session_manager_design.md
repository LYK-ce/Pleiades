# SessionManager 设计文档

> ⚠️ **此文档为早期设计提案（2026-05-14），实际实现已有重大变更。**  
> 当前架构请参见 `Architecture/Pleiades_Architecture.md` §3.6 Orchestrator 和 `Task/task_6_v2_session_core.md`。
>
> 主要差异：SessionManager 不是 trait（改为纯数据结构 Arc<Mutex<>>）；Slot 不走 IoHandle 而是 mpsc 通道对；Chat 连接通过 allocate_slot 建立；远端 Chat 通过 Session 流协议接入。

---

Presented by KeJi
Date ： 2026-05-14

## 1. 模块概述

`SessionManager` 模块负责管理 Pleiades 的**推理会话生命周期**和**文本 IO 通道**。它不参与推理计算，不关心前端类型（TUI/HTTP/Network）——只做纯粹的**会话注册表 + 槽位分配 + 通道管理**。

### 核心定义

> **SessionManager = Session 注册表 + Slot 槽位分配 + Channel 文本通道。**
> 创建 Session 时分配槽位和通道对，前端通过 connect 申请槽位。SessionManager 不知道 ML Thread 的存在，两者仅通过 mpsc channel pair 耦合。

### 模块结构

```
Session/
├── session.rs          ← 数据结构定义（Session, SessionInfo）
├── slot.rs             ← 数据结构定义（Slot, SlotState）
├── capability.rs       ← Trait 定义（Session_Capability + Session_Error）
├── manager.rs          ← 核心实现（SessionManager: sessions Mutex<HashMap> + config）
└── mod.rs              ← 模块入口 + 集成测试
```

### 调用关系

```
TUI / HTTP / Network
         │
         ▼
  Arc<SessionManager>          ← manager.rs (sessions + slots + channels)
         │
         ├── create_session → 返回 (session_id, IoHandle_ml)
         │                     调用方自行将 IoHandle_ml 传给 ML Engine
         │
         ├── connect → 返回 (slot_id, IoFrontend)
         │              调用方自行决定如何使用（TUI / SSE / P2P 桥接）
         │
         ├── list_sessions()
         └── destroy_session()
         │
         ▼
   ML Engine (仅通过 IoHandle 通信，无模块依赖)
```

---

## 2. 数据结构

### 2.1 Session — 会话

```rust
struct Session {
    session_id: String,
    model_id: String,
    max_slots: usize,
    slots: Vec<Slot>,
    ml_handle: IoHandle,         // 通向 ML Thread 的唯一通道（SessionManager 持有 ml_side）
    slot_counter: u32,           // 已分配槽位计数
}
```

一个 Session 对应一个模型实例。SessionManager 不持有 ML Thread 句柄——只持有通向前端方向的通道管理权。

### 2.2 Slot — 槽位

```rust
struct Slot {
    slot_id: u32,
    state: SlotState,
}

enum SlotState {
    Vacant,
    Occupied { owner: String },  // 前端标识（如 "tui"、"http:8961"）
}
```

- 槽位总数由 `config.toml` `[Session].max_slots` 决定，默认 4
- `slot_id` 为服务序列号，`connect()` 时 `slot_counter` 递增分配
- v1 版本槽位固定不释放，`release_slot()` 空实现
- v2 版本支持释放：标记 Vacant，通知 ML Thread 清 KV cache

### 2.3 SessionInfo — 公开视图

```rust
struct SessionInfo {
    session_id: String,
    model_id: String,
    total_slots: usize,
    occupied_slots: usize,
}
```

通过 `list_sessions()` 返回，不暴露内部通道。

### 2.4 IoHandle / IoFrontend — 通道端点

```rust
/// ML 侧端点（持有者：ML Thread）
struct IoHandle {
    input_rx: mpsc::Receiver<String>,   // 收 prompt
    output_tx: mpsc::Sender<String>,    // 发 token
}

/// 前端端点（持有者：TUI / HTTP handler）
struct IoFrontend {
    input_tx: mpsc::Sender<String>,     // 发 prompt
    output_rx: mpsc::Receiver<String>,  // 收 token
}
```

沿用现有定义，不变。`create_session()` 内部创建 `IoHandle` + 对应 `IoFrontend` 对。`IoHandle` 随 Session 存储（待 ML Engine 取走），`IoFrontend` 在 `connect()` 时发放。

### 2.5 SessionConfig — 配置

```rust
struct SessionConfig {
    max_slots: usize,    // 来自 config.toml [Session].max_slots
}
```

### 2.6 Session_Error — 错误类型

```rust
enum Session_Error {
    SessionNotFound(String),       // session_id 不存在
    SlotExhausted(String),         // 槽位已满
    SessionExists(String),         // session_id 已存在
    Internal(String),              // 内部错误
}
```

---

## 3. Trait 定义

### 3.1 Session_Capability

```rust
#[async_trait]
pub trait Session_Capability: Send + Sync {
    /// 创建 Session（分配槽位 + channel pair），返回 (session_id, IoHandle_ml)
    /// 调用方自行将 IoHandle_ml 传给 ML Engine 启动线程
    async fn create_session(&self, model_id: String, config: SessionConfig)
        -> Result<(String, IoHandle), Session_Error>;

    /// 销毁 Session，清理所有通道和槽位
    async fn destroy_session(&self, session_id: &str)
        -> Result<(), Session_Error>;

    /// 申请槽位，返回 (slot_id, IoFrontend)
    /// 调用方自行决定如何使用 IoFrontend（TUI / HTTP / Network）
    async fn connect(&self, session_id: &str)
        -> Result<(u32, IoFrontend), Session_Error>;

    /// 列出所有活跃 Session
    fn list_sessions(&self) -> Vec<SessionInfo>;

    /// 释放槽位（v1 空实现，预留接口）
    async fn release_slot(&self, session_id: &str, slot_id: u32)
        -> Result<(), Session_Error>;
}
```

---

## 4. 核心实现

### 4.1 SessionManager

```rust
pub struct SessionManager {
    sessions: Mutex<HashMap<String, Session>>,
    max_slots: usize,     // 来自 config.toml [Session].max_slots
}
```

**锁方案**：`tokio::sync::Mutex<HashMap>`。操作低频，简单互斥锁足够。

### 4.2 create_session()

```
create_session(model_id, config)
  1. 生成 session_id（UUID v4 或自增计数）
  2. 创建 max_slots 个 Slot（全部 Vacant）
  3. 创建 channel pair → 得 (IoHandle_ml, frontend_waiting)
     注意：此 frontend 对应 "batch_0" 位置，ML Thread 的输入输出
  4. 注册 Session 到 sessions 表
  5. 返回 (session_id, IoHandle_ml)
```

`frontend_waiting` 暂存于 Session 中，供后续 `connect()` 使用。

### 4.3 connect()

```
connect(session_id)
  1. 查 sessions 表
  2. slot_counter += 1，检查是否超过 max_slots
  3. 创建新的 channel pair → 得 (IoFrontend, frontend_side)
  4. Slot 标记为 Occupied
  5. 返回 (slot_id, IoFrontend)
```

注意：每个 slot 对应一个独立的 `IoFrontend`，ML Thread 侧需要知道哪个 slot 发了 prompt、token 回哪个 slot。v1 通过 `slot_id` 嵌入 prompt 前缀或使用独立的 channel per slot 实现。

### 4.4 destroy_session()

```
destroy_session(session_id)
  1. 从 sessions 表移除
  2. Session.drop → 所有 Sender.drop → 通道断开
  3. ML Thread 的 input_rx.recv() = None → 线程退出
```

### 4.5 list_sessions()

```
list_sessions()
  1. 遍历 sessions 表
  2. 统计 occupied_slots
  3. 返回 Vec<SessionInfo>
```

---

## 5. 模块导出

```rust
// mod.rs
pub use capability::{Session_Capability, Session_Error};
pub use session::{SessionInfo};
pub use slot::{Slot, SlotState};
pub use manager::SessionManager;

// 保留旧模块的 IoHandle / IoFrontend 导出（兼容现有依赖）
mod io_types;
pub use io_types::{IoHandle, IoFrontend};
```

---

## 6. 协作关系

### 6.1 与 ML Engine 的协作

| 职责 | 负责方 | 说明 |
|------|--------|------|
| 通道创建 | SessionManager | create_session 创建 channel pair |
| 线程启动 | Core / ML Engine | 收到 IoHandle_ml 后启动推理线程 |
| 推理执行 | ML Thread | 从 IoHandle.input_rx 读 prompt，往 IoHandle.output_tx 写 token |
| 通道回收 | SessionManager | destroy_session 时 drop Sender，线程退出 |

SessionManager 与 ML Engine **无代码级依赖**，仅通过 `IoHandle` (mpsc channel pair) 通信。

### 6.2 与 Core 的协作

Core（后续阶段）调用流程：

```
run <model>:
  (session_id, io_handle) = session_manager.create_session(model, config)
  ml_engine.spawn(io_handle, model_config)

chat <session_id>:
  (slot_id, frontend) = session_manager.connect(session_id)
  启动 ChatService(slot_id, frontend)

api <session_id>:
  (slot_id, frontend) = session_manager.connect(session_id)
  启动 ApiService(slot_id, frontend)

stop <session_id>:
  session_manager.destroy_session(session_id)
```

### 6.3 与 Config 的协作

`SessionManager::new()` 接收 `SessionConfig`，来自 `config.toml` 的 `[Session]` 段：

```toml
[Session]
max_slots = 4
```

---

## 7. 已知风险

### 7.1 槽位固定不释放

v1 版本 `release_slot()` 为空实现。槽位一旦分配永不回收。对于长时运行且频繁 connect/disconnect 的场景，槽位数可能迅速耗尽。

- **当前状态**：可接受，请求量不大
- **未来优化**：实现 `release_slot()`，通知 ML Thread 清理 KV cache

### 7.2 多槽位 token 路由

当多个 slot 同时活动时，ML Thread 的输出需要按 `slot_id` 解复用，路由到正确的 `IoFrontend`。当前设计通过独立 channel per slot 实现，但 ML Thread 侧需要知道 "这次推理结果属于 slot X"。

- **当前状态**：v1 通过独立 mpsc per slot 实现，ML Thread 需配合改造
- **后续细化**：在 ML Thread 接口设计时确定 token→slot 的路由协议

### 7.3 create_session 返回的 IoHandle

`create_session` 返回的 `IoHandle_ml` 需要由调用方（Core）传递给 ML Engine。如果调用方未传递或忽略，ML Thread 永远不启动，SessionManager 中留下僵尸 Session。

- **缓解**：SessionManager 可设置超时（后续版本），未启动的 Session 自动清理

---

## 8. 与旧模块（LLM_IO）的差异

| 方面 | LLM_IO (旧) | SessionManager (新) |
|------|-------------|-------------------|
| key | JobId | SessionId（UUID/自增） |
| 分配接口 | Allocate + Take_ML_Side + Take_Frontend | create_session + connect |
| 通道数 | 1 per Job | 1 (ML) + N (frontends) |
| 生命周期 | 随 Job | 独立于 Job，显式 destroy |
| ML Engine 感知 | 无（通过 IoHandle） | 无（通过 IoHandle），不变 |
| 槽位 | 无 | 固定槽位，序列号递增 |

---

## 9. TODO

### release_slot 实现

`release_slot()` 目前为空实现。正式实现需要：
- 标记 Slot 为 Vacant
- 通过独立命令通道通知 ML Thread 清理对应 `slot_id` 的 KV cache
- 关闭该槽位的 `IoFrontend` 通道
