# EventBus 设计文档

Presented by KeJi
Created Date ： 2026-05-19
Modified Date ： 2026-06-15

---

## 目录

- [1. 模块概述](#1-模块概述)
- [2. 结构体定义](#2-结构体定义)
  - [Bus_Event](#bus_event)
  - [NotifyLevel](#notifylevel)
  - [EventBus](#eventbus)
- [3. 模块方法](#3-模块方法)
  - [New](#new)
  - [Publish](#publish)
  - [Subscribe](#subscribe)
- [4. 使用示例](#4-使用示例)
- [5. 已知限制](#5-已知限制)

---

## 1. 模块概述

**模组等级：Level 0** — 不调用任何其他项目模块，仅依赖第三方 crate（`tokio::sync::broadcast`）。为全系统最底层基础设施。

EventBus 是全局事件总线，对 `tokio::sync::broadcast` 的薄封装。不感知任何领域概念，通过 4 种通用事件类型 + JSON payload 实现生产者与消费者完全解耦。

---

## 2. 结构体定义

### Bus_Event

4 种通用事件类型。`Notify` 使用具名字段，其余 3 种使用 JSON 字符串 payload 以解耦领域概念。

```rust
pub enum Bus_Event {
    /// 一次性通知（日志、错误、设备切换等瞬态消息）
    /// level 决定消息级别，message 为人类可读文本
    Notify { level: NotifyLevel, message: String },

    /// 持久状态变更（节点上下线、Job 生命周期、模型列表更新等）
    /// payload 为 JSON 字符串，消费者解析 "type" 字段二次路由
    State  { payload: String },

    /// 高频流式推送（逐 token 输出、文件传输进度等）
    /// payload 为 JSON 字符串，消费者解析 "type" 字段二次路由
    Stream { payload: String },

    /// 命令执行结果（flush、ls、help 等命令的输出）
    /// payload 为 JSON 字符串，消费者解析 "type" 字段二次路由
    Output { payload: String },
}
```

| 变体 | 语义 | payload | 消费方式 |
|------|------|---------|----------|
| `Notify` | 一次性通知 | 纯文本 `message: String` | 直接读取 `level` + `message`，无需解析 |
| `State` | 持久状态变更 | JSON `{"type": "peer_discovered", ...}` | `serde_json::from_str` → 按 `type` 分发 |
| `Stream` | 高频流式推送 | JSON `{"type": "token", ...}` | `serde_json::from_str` → 按 `type` 分发 |
| `Output` | 命令执行结果 | JSON `{"type": "cmd_result", ...}` | `serde_json::from_str` → 按 `type` 分发 |

> **设计说明**：`Notify` 不走 JSON 是因为消费端只需要「根据 level 加前缀，输出 message」，结构化字段比 JSON 字符串更直接高效。其余三个变体需要二级路由（同一变体内有多种子类型），JSON 的 `"type"` 字段承担此职责。

### NotifyLevel

`Notify` 事件的级别枚举，与 Rust 生态 `log`/`tracing` crate 的命名惯例一致。

```rust
pub enum NotifyLevel {
    /// 常规信息，直接展示
    Info,
    /// 警告，消费端通常加 [WARN] 前缀
    Warn,
    /// 错误，消费端通常加 [ERROR] 前缀
    Error,
}
```

仅 3 个级别，对应 `tracing::Level` 的 `INFO`/`WARN`/`ERROR`（不含 `TRACE` 和 `DEBUG`，因为 EventBus 的消费者是人类而非开发者，不需要调试级日志）。

### EventBus

对 `tokio::sync::broadcast::Sender<Bus_Event>` 的薄封装。内部仅持有一个字段：

```rust
pub struct EventBus {
    /// tokio broadcast 发送端，支持 Clone + Send + Sync
    /// 通过 Arc<EventBus> 在多个组件间共享
    sender: broadcast::Sender<Bus_Event>,
}
```

所有方法均直接委托给 `sender`，无额外状态或逻辑。`sender` 自身线程安全，`EventBus` 天然满足 `Send + Sync`。

---

## 3. 模块方法

### New

```rust
pub fn New(capacity: usize) -> Self;
```

| | 说明 |
|------|------|
| **输入** | `capacity: usize` — broadcast 通道缓冲区大小（建议 1024） |
| **输出** | `Self` — 新的 EventBus 实例 |
| **内部逻辑** | 调用 `tokio::sync::broadcast::channel(capacity)` 创建通道，丢弃返回的 Receiver（首个 Receiver 通过 `Subscribe()` 按需创建），仅保留 Sender |

---

### Publish

```rust
pub fn Publish(&self, event: Bus_Event);
```

| | 说明 |
|------|------|
| **输入** | `event: Bus_Event` — 要广播的事件，所有订阅者将收到此事件的 Clone 副本 |
| **输出** | 无返回值 |
| **内部逻辑** | 调用 `self.sender.send(event)`。<br>• 有订阅者 → 事件推入每个订阅者的缓冲区，返回 `Ok(已发送数)` → 被 `let _ =` 忽略<br>• 零订阅者 → 返回 `Err(事件)` → 被 `let _ =` 静默丢弃 |

> 同步方法，非阻塞。`broadcast::send` 仅在缓冲区满时等待，但 `EventBus::Publish` 不检查返回值，因此对调用方总是瞬间返回。

---

### Subscribe

```rust
pub fn Subscribe(&self) -> broadcast::Receiver<Bus_Event>;
```

| | 说明 |
|------|------|
| **输入** | 无 |
| **输出** | `broadcast::Receiver<Bus_Event>` — 新的订阅者接收端 |
| **内部逻辑** | 调用 `self.sender.subscribe()`，tokio 内部为此 Receiver 分配独立的缓冲区。仅接收 Subscribe 调用**之后**发布的事件，历史事件不可见 |

> 每次调用产生独立 Receiver，支持多个消费者同时订阅。各消费者独立接收，互不干扰。

---

## 4. 使用示例

```rust
use std::sync::Arc;
use pleiades::event_bus::{EventBus, Bus_Event, NotifyLevel};

// 创建
let bus = Arc::new(EventBus::New(1024));

// 订阅
let mut rx = bus.Subscribe();

// 发布
bus.Publish(Bus_Event::Notify {
    level: NotifyLevel::Info,
    message: "模型加载完成".to_string(),
});
bus.Publish(Bus_Event::State {
    payload: serde_json::json!({"type": "peer_discovered", "peer_id": "12D3..."}).to_string(),
});

// 消费
tokio::spawn(async move {
    loop {
        match rx.recv().await {
            Ok(event) => { /* 按变体分发处理 */ }
            Err(Lagged(n)) => eprintln!("丢失 {n} 条事件"),
            Err(Closed) => break,
        }
    }
});
```

---

## 5. 已知限制

| 限制 | 说明 |
|------|------|
| 无背压 | 零订阅者时事件静默丢弃；订阅者处理慢时缓冲区满后旧事件被覆盖，消费者收到 `Lagged` 通知 |
| 无类型安全 | `State`/`Stream`/`Output` 的 JSON `"type"` 字段拼写错误在运行时才暴露，消费者静默忽略未知 type |
| 无持久化 | 事件仅存在于内存，无历史回放。消费者 Subscribe 之后才能收到事件 |
