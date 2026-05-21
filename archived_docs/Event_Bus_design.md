# EventBus 模块设计文档

## 概述

EventBus 是一个独立于任何组件之外的全局事件总线模块，用于在系统内部传递**通知型事件**。任何组件（Orchestrator、Network、ML Engine 等）可以向总线发布事件，任何订阅者（TUI、Web UI、日志系统等）可以同时接收事件副本。

EventBus **不处理命令/请求**（命令仍使用点对点 `mpsc` 通道），仅传递无需回复、无需所有权转移的状态通知。

## 架构位置

```text
┌─────────────┐  ┌───────────┐  ┌───────────┐  ┌──────────────┐
│ Orchestrator │  │  Network  │  │ ML Engine │  │ PeerManager  │
└──────┬───────┘  └─────┬─────┘  └─────┬─────┘  └──────┬───────┘
       │                │              │                │
       │  publish()     │  publish()   │  publish()     │ publish()
       ▼                ▼              ▼                ▼
  ┌─────────────────────────────────────────────────────────┐
  │                      EventBus                           │
  │              (tokio::sync::broadcast)                   │
  └──────┬─────────────────┬────────────────┬───────────────┘
         │  subscribe()    │  subscribe()   │  subscribe()
         ▼                 ▼                ▼
    ┌─────────┐     ┌───────────┐    ┌─────────────┐
    │   TUI   │     │  Web UI   │    │ File Logger │
    └─────────┘     └───────────┘    └─────────────┘
```

与命令通道的关系（非对称设计）：

```text
UI ──── mpsc: UserCommand ────→ Orchestrator    （命令方向：点对点）
任意组件 ─── broadcast: Bus_Event ──→ EventBus ──→ UI  （通知方向：广播）
```

## 模块结构

```text
Src/EventBus/
├── mod.rs              # 模块入口，导出 EventBus 和 Bus_Event
├── event_bus.rs        # EventBus 结构体实现
├── event.rs            # Bus_Event 枚举定义
└── Event_Bus_design.md # 本设计文档
```

## 核心类型

### EventBus 结构体

```rust
use tokio::sync::broadcast;

pub struct EventBus {
    sender: broadcast::Sender<Bus_Event>,
}
```

EventBus 是对 `tokio::sync::broadcast::Sender` 的薄封装。内部仅持有一个 `broadcast::Sender`，通过它实现多生产者多消费者的事件分发。

### Bus_Event 枚举

```rust
#[derive(Debug, Clone)]
pub enum Bus_Event {
    // ===== 网络/节点事件 =====
    Peer_Discovered { peer_id: String },
    Peer_Left { peer_id: String },
    Connection_Established { peer_id: String },
    Connection_Closed { peer_id: String },

    // ===== 作业生命周期事件 =====
    Job_Created { job_id: u64, kind: String, model_name: String },
    Job_State_Changed { job_id: u64, phase: String },
    Job_Completed { job_id: u64, result: String },

    // ===== 推理事件 =====
    Inference_Started { job_id: u64, model_name: String, device_count: usize, layer_range: String },
    Inference_Token { job_id: u64, token: String },
    Inference_Completed { job_id: u64, text: String, tokens: usize, tok_per_sec: f64, total_secs: f64 },

    // ===== 文件传输事件 =====
    File_Progress { file_name: String, direction: String, peer: String, sent: u64, total: u64 },

    // ===== 系统事件 =====
    Log { message: String },
    Error { message: String },
    Device_Changed { device: String },
}
```

**设计原则：**
- 所有字段使用纯数据类型（`String` / `u64` / `usize` / `f64`），不引入 `libp2p::PeerId`、`JobId` 等外部类型
- `Bus_Event` 必须实现 `Clone`（`broadcast` 要求）
- 带 `job_id` 以支持多 Job 并发场景
- 每个 variant 对应一次 UI 可渲染的信息单元

## 开放接口

### `EventBus::New(capacity: usize) -> Self`

创建新的 EventBus 实例。

| 参数 | 类型 | 说明 |
|------|------|------|
| `capacity` | `usize` | broadcast channel 的缓冲区大小，建议 1024 |

```rust
let event_bus = EventBus::New(1024);
```

### `EventBus::Publish(&self, event: Bus_Event)`

向总线发布一个事件。所有当前订阅者将收到此事件的克隆副本。

| 参数 | 类型 | 说明 |
|------|------|------|
| `event` | `Bus_Event` | 要发布的事件 |

- 无返回值
- 如果当前没有订阅者，事件被静默丢弃（不会 panic 或阻塞）
- 此方法是**同步的**（`broadcast::Sender::send` 不需要 `.await`），可以在同步和异步上下文中调用

```rust
event_bus.Publish(Bus_Event::Peer_Discovered {
    peer_id: "12D3KooW...".to_string(),
});
```

### `EventBus::Subscribe(&self) -> broadcast::Receiver<Bus_Event>`

创建一个新的订阅者（接收端）。每次调用返回一个独立的 `Receiver`，订阅者从**调用 Subscribe 之后**的事件开始接收。

| 返回值 | 类型 | 说明 |
|--------|------|------|
| receiver | `broadcast::Receiver<Bus_Event>` | 该订阅者的事件接收端 |

```rust
let mut rx = event_bus.Subscribe();
loop {
    match rx.recv().await {
        Ok(event) => { /* 处理事件 */ }
        Err(broadcast::error::RecvError::Lagged(n)) => {
            // 消费过慢，丢失了 n 条事件
            tracing::warn!("EventBus consumer lagged, missed {} events", n);
        }
        Err(broadcast::error::RecvError::Closed) => break,
    }
}
```

## 使用方式

### 1. 启动时创建并分发

在应用启动时（`main.rs`）创建 `EventBus`，用 `Arc` 包装后分发给各组件：

```rust
use std::sync::Arc;

let event_bus = Arc::new(EventBus::New(1024));

// 传给 Orchestrator（放入 Capabilities 中）
let capabilities = Capabilities {
    // ...
    event_bus: event_bus.clone(),
};

// 传给 TUI
let tui_rx = event_bus.Subscribe();
tokio::task::spawn_blocking(move || {
    TUI_Loop(tui_rx, user_cmd_tx);
});

// 传给 Network 层
let network_bus = event_bus.clone();
```

### 2. 生产者侧（发布事件）

任何持有 `Arc<EventBus>` 的组件直接调用 `Publish`：

```rust
// Orchestrator 发布作业创建事件
self.capabilities.event_bus.Publish(Bus_Event::Job_Created {
    job_id: job_id.0,
    kind: format!("{:?}", job_kind),
    model_name: model_path.clone(),
});

// Network 层发布节点发现事件
network_bus.Publish(Bus_Event::Peer_Discovered {
    peer_id: peer_id.to_string(),
});

// 推理过程中发布 token
event_bus.Publish(Bus_Event::Inference_Token {
    job_id: job_id.0,
    token: token_text.clone(),
});
```

### 3. 消费者侧（接收事件）

TUI 或其他 UI 在事件循环中消费事件：

```rust
// TUI 消费者示例
fn TUI_Loop(mut event_rx: broadcast::Receiver<Bus_Event>, cmd_tx: mpsc::Sender<UserCommand>) {
    let mut app = App::New();
    loop {
        // 非阻塞接收所有待处理事件
        while let Ok(event) = event_rx.try_recv() {
            Handle_Bus_Event(&mut app, event);
        }
        // 渲染 + 键盘处理...
    }
}

fn Handle_Bus_Event(app: &mut App, event: Bus_Event) {
    match event {
        Bus_Event::Peer_Discovered { peer_id } => {
            app.Update_Peer(peer_id.clone(), false);
            app.Add_Log(format!("发现节点: {}", peer_id));
        }
        Bus_Event::Inference_Token { job_id, token } => {
            app.command_output.output_text.push_str(&token);
            app.command_output.token_count += 1;
        }
        Bus_Event::Log { message } => {
            app.Add_Log(message);
        }
        // ... 其他事件处理
        _ => {}
    }
}
```

## 与现有组件的集成方式

### 替换 Orchestrator 中的 UiCapability

当前 `Orchestrator/mod.rs` 中的 `UiCapability` 占位符将被 `Arc<EventBus>` 替代：

```rust
// 修改前
pub struct Capabilities {
    pub ui: UiCapability,  // 占位符 todo!()
    // ...
}

// 修改后
pub struct Capabilities {
    pub event_bus: Arc<EventBus>,
    // ...
}
```

### 替换 Control 层的 ui_tx 模式

旧 Control 层的 `ui_tx: mpsc::Sender<Ui_Message>` 参数传递模式不再需要。各组件不再需要接收 `ui_tx` 参数，而是通过 `Arc<EventBus>` 自行发布事件。

| 旧模式 | 新模式 |
|--------|--------|
| `Send_Ui(&ui_tx, Ui_Message::Log(...)).await` | `event_bus.Publish(Bus_Event::Log { ... })` |
| `fn foo(ui_tx: &mpsc::Sender<Ui_Message>)` | `fn foo(event_bus: &EventBus)` 或从 Capabilities 获取 |
| 单消费者（TUI 独占 `ui_rx`） | 多消费者（任意数量 `Subscribe()`） |

## 注意事项

### 慢消费者处理

`tokio::sync::broadcast` 在缓冲区满时，最慢的消费者会收到 `RecvError::Lagged(n)` 错误，表示丢失了 n 条消息。对于 `Inference_Token` 这类需要严格有序的事件流：

- 缓冲区设为 1024 以降低 Lagged 风险
- 消费者端收到 `Lagged` 后记录警告日志，不应 panic
- 如果未来发现 1024 不够，可以在消费者内部增加 `mpsc` 二级缓冲

### 不适合走 EventBus 的场景

以下场景**不应**使用 EventBus，仍需使用点对点 channel：

| 场景 | 原因 |
|------|------|
| `UserCommand`（UI → Orchestrator） | 命令需要路由到特定目标，可能需要回复 |
| `NetworkCommand`（Network → Orchestrator） | 同上 |
| `Network_Inbound_Event`（含 `libp2p::Stream`） | 包含不可 Clone 的所有权类型 |
| `LifecycleEvent`（Executor → Core） | 内部生命周期管理，无需对外广播 |

### Event 类型不引入外部依赖

`Bus_Event` 的所有字段使用 `String` / `u64` / `f64` 等基本类型。这确保：
- EventBus 模块不依赖 `libp2p`、`orchestrator` 等重量级 crate
- 任何 UI 实现可以直接消费事件，无需引入领域层依赖
- 序列化友好（未来可直接 JSON 序列化传给 Web UI）
