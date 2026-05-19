# EventBus 设计文档

Presented by KeJi
Date ： 2026-05-19

## 1. 模块概述

`EventBus` 是 Pleiades 分布式推理系统的 **全局事件总线**，负责在组件间传递通知型事件，实现生产者与消费者的完全解耦。

### 核心定义

> **EventBus = 4 种通用事件类型 + JSON payload + broadcast 广播。**
> 不感知 Peer / Job / ML 等任何领域概念。新增模块无需修改 EventBus 定义。
> 基于 `tokio::sync::broadcast` 实现多生产者多消费者。
> TUI 消费端通过 `"type"` 字段路由到具体 handler。

### 模块结构

```
EventBus/
├── mod.rs           ← 模块入口 + re-export
├── event.rs         ← Bus_Event 枚举 + NotifyLevel 枚举
└── event_bus.rs     ← EventBus 结构体（broadcast::Sender 薄封装）
```

### 事件流

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│ Orchestrator │  │   Network    │  │     Lua      │
│   branch_*   │  │ swarm_events │  │capability_bnd│
└──┬───┬───┬───┘  └──────┬───────┘  └──────┬───────┘
   │   │   │              │                  │
   ▼   ▼   ▼              ▼                  ▼
┌─────────────────────────────────────────────────┐
│     EventBus (tokio::sync::broadcast)           │
│     cap=1024, Arc<EventBus> 共享                │
└─────────────────────┬───────────────────────────┘
                      │  Subscribe()
                      ▼
              ┌───────────────┐
              │   TUI_Loop    │
              │ (spawn_blocking)│
              └───────────────┘
```

---

## 2. 数据结构

### 2.1 Bus_Event — 4 种通用事件类型

```rust
pub enum Bus_Event {
    Notify { level: NotifyLevel, message: String },
    State  { payload: String },
    Stream { payload: String },
    Output { payload: String },
}
```

| 类型 | 语义 | 对应旧 variant | TUI 消费面板 |
|------|------|---------------|-------------|
| `Notify` | 一次性通知 | Log / Error / Device_Changed | Log 面板 |
| `State` | 持久状态变更 | Peer_* / Job_* / Inference_* | Network + Job + Log 面板 |
| `Stream` | 高频流式推送 | Inference_Token / File_Progress | Command + Job 面板 |
| `Output` | 命令输出 | CommandResult / HelpInfo | Command 面板 |

### 2.2 NotifyLevel — 通知级别

```rust
pub enum NotifyLevel {
    Info,
    Warn,
    Error,
}
```

仅 `Notify` 事件使用。TUI 消费端按级别渲染：Info 直接显示、Warn 加 `[警告]` 前缀、Error 加 `[错误]` 前缀。

### 2.3 JSON Payload 格式

#### State

```json
// Peer 事件
{"type":"peer_discovered",   "peer_id":"12D3..."}
{"type":"peer_left",         "peer_id":"12D3..."}
{"type":"peer_connected",    "peer_id":"12D3..."}
{"type":"peer_disconnected", "peer_id":"12D3..."}

// Job 生命周期
{"type":"job_created",       "job_id":1, "kind":"Run", "model":"qwen3"}
{"type":"job_phase_changed", "job_id":1, "phase":"Executing"}
{"type":"job_completed",     "job_id":1, "result":"Success"}

// 推理
{"type":"inference_started",  "model":"qwen3", "devices":1, "layers":"0-15"}
{"type":"inference_completed","text":"Hello", "tokens":5, "tok_per_sec":30.5, "total_secs":0.16}

// 设备
{"type":"device_changed",    "device":"cuda"}
```

#### Stream

```json
{"type":"token",         "text":"Hello", "count":1}
{"type":"file_progress", "name":"model.gguf", "dir":"send", "peer":"12D3...", "sent":1024, "total":4096}
```

#### Output

```json
{"type":"cmd_result",    "text":"共 3 个节点:\n  12D3...  ...", "completed":true}
{"type":"help",          "text":"[内置命令]\n  run <model>  启动本地推理\n  ..."}
```

### 2.4 EventBus — 结构体

```rust
pub struct EventBus {
    sender: broadcast::Sender<Bus_Event>,
}
```

`tokio::sync::broadcast` 的薄封装。提供 `New(capacity)`、`Publish(event)`、`Subscribe()` 三个方法。

---

## 3. 设计原则

### 3.1 零耦合

`Bus_Event` 不包含任何领域类型引用（如 `PeerId`、`JobKind`、`GGUF_Model`）。所有结构化数据通过 JSON payload 传递。新增模块（如 Auth、Monitoring）只需构造符合约定的 JSON，无需修改 EventBus。

### 3.2 4 种类型而非 14 种

旧设计 14 个 flat variant 随模块增长线性膨胀。新设计 4 种通用类型永不膨胀——`State` 和 `Stream` 通过 `"type"` 字段内部分发，新增"type" 只改 TUI handler。

### 3.3 Fire-and-Forget

生产者发布后不关心消费结果。`broadcast` 无订阅者时静默丢弃。命令/请求不走 EventBus（用 mpsc/oneshot）。

### 3.4 Clone 语义

`broadcast` 要求 `Bus_Event: Clone`。每条消息对每个订阅者 clone 一次。当前只有 TUI 一个订阅者，开销可控。

---

## 4. 生产者映射

| 生产者 | 旧（14 variant） | 新（4 type） | payload |
|--------|------------------|-------------|---------|
| **branch_command.rs** | `Log` ×2 | `Notify::Info` | — |
| **branch_stream.rs** | `Log` ×1 | `Notify::Info` | — |
| **branch_lifecycle.rs** | `Job_Completed` ×1 | `State` | `{"type":"job_completed",...}` |
| **branch_user.rs** | `CommandResult` ×12 + `HelpInfo` ×1 | `Output` | `{"type":"cmd_result/help",...}` |
| **swarm_events.rs** | `Peer_Discovered/Left/Connected/Disconnected` ×4 | `State` | `{"type":"peer_*",...}` |
| **capability_binding.rs** | `Log` ×1 | `Notify::Info` | — |

---

## 5. TUI 消费端

### 5.1 事件循环

```rust
// spawn_blocking 中运行，非阻塞接收
loop {
    match event_rx.try_recv() {
        Ok(event) => Handle_Bus_Event(&mut app, event),
        Err(Empty) => break,
        Err(Lagged(n)) => app.Add_Log(format!("⚠ 丢失 {n} 条事件")),
        Err(Closed) => break,
    }
}
```

### 5.2 路由逻辑

```
Handle_Bus_Event
├── Notify → app.Add_Log (按 NotifyLevel 决定前缀)
├── State → parse_json → match payload["type"]
│   ├── peer_*       → app.Update_Peer / Remove_Peer + Add_Log
│   ├── job_*        → app.Add_Log + Job 状态机
│   ├── inference_*  → app.job + app.command_output
│   └── device_changed → app.device + Add_Log
├── Stream → parse_json → match payload["type"]
│   ├── token        → command_output.push_str + scroll
│   └── file_progress → app.job = File_Transfer
└── Output → parse_json → match payload["type"]
    ├── cmd_result   → command_output = text/completed
    └── help         → command_output = text
```

---

## 6. 接口

```rust
impl EventBus {
    /// 创建新的 EventBus，capacity 为 broadcast 缓冲区大小
    pub fn New(capacity: usize) -> Self;

    /// 发布事件（同步方法，不阻塞）
    /// 无订阅者时静默丢弃
    pub fn Publish(&self, event: Bus_Event);

    /// 创建订阅者，仅接收 Subscribe 之后的事件
    pub fn Subscribe(&self) -> broadcast::Receiver<Bus_Event>;
}
```

---

## 7. 已知限制

### 7.1 单消费者

当前仅 TUI_Loop 一个订阅者。`broadcast` 对单消费者略重（每条消息 Clone），但保留扩展性。若确定永远单消费者，可替换为 `mpsc::unbounded`。

### 7.2 JSON 解析开销

每个 `State`/`Stream`/`Output` 事件在消费端需 `serde_json::from_str`。事件频率低（非热路径），开销可忽略。高频 `Stream::token` 事件需关注（推理时 ~50 tokens/s）。

### 7.3 无背压

生产者发布不等待消费者。cap=1024，若 TUI 阻塞 >20s（50 tokens/s）开始丢消息。Lagged 后消费者收到通知并恢复。

### 7.4 无类型安全

JSON `"type"` 字段拼写错误在运行时才暴露（消费者静默忽略未知 type）。`Notify` 保持编译期类型检查（`NotifyLevel` 枚举）。

---

## 8. 重构历史

| 变更 | 说明 |
|------|------|
| 初始实现 | 14 个领域耦合 flat variant（Peer_* ×4, Job_* ×3, Inference_* ×3, Log, Error, File_Progress, Device_Changed, CommandResult, HelpInfo） |
| EventBus Reforge | 14 variant → 4 通用类型（Notify/State/Stream/Output），JSON payload 解耦 |
| HelpEntry 本地化 | 从 event.rs 移除，移至 branch_user.rs |
| 新增 NotifyLevel | Log/Error/Device_Changed → Notify { level, message } |
| 新增 serde_json 依赖 | State/Stream/Output 生产与消费均需 JSON |

---

## 9. TODO

### 订阅者扩展

- 支持多消费者（GUI、日志收集器、Metrics 面板）
- 考虑消息持久化（Event Sourcing）

### 性能优化

- `Inference_Token` 高频场景考虑批处理或 `Arc<str>` 减少 String 分配
- 考虑 `serde_json::Value` 预解析以复用解析器

### 运维

- 添加事件计数统计（各 type 频率、Lagged 次数）
- 添加死信队列（未匹配 type 的审计日志）
