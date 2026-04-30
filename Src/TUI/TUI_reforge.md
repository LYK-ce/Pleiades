# TUI Reforge — 重新集成到 Orchestrator 系统

## 1. 背景

TUI 模块原本基于已删除的 **Control 层** 设计，依赖两个不存在的类型：
- `crate::control::ui_message::Ui_Message` — 已删除
- `crate::control::cli_command::CLI_Command` — 已被 `UserCommand` 取代

当前 `main.rs` 为空壳，TUI 模块无法编译。需要将 TUI 重新接入以 Orchestrator Core 为中心的新架构。

## 2. 新架构概览

```
┌────────────────────────────────────────────────────────────┐
│                    Orchestrator Core                        │
│                     (select! loop)                          │
│                                                            │
│  B1: user_cmd_rx ←── UserCommand ──── TUI                  │
│  B2: inbound_rx                                            │
│  B3: network_inbound_rx                                    │
│  B4: lifecycle_rx                                          │
│                                                            │
│  Publish(Bus_Event) ──→ EventBus ──→ TUI (try_recv)       │
└────────────────────────────────────────────────────────────┘
```

### 通信路径

| 方向 | 机制 | 类型 |
|------|------|------|
| Core/各模块 → TUI | `EventBus` broadcast，TUI 调用 `try_recv()` | `Bus_Event` |
| TUI → Core | `mpsc::Sender<UserCommand>`，TUI 调用 `blocking_send()` | `UserCommand` |

### 关键设计决策：TUI 直连 EventBus，无桥接层

`tokio::sync::broadcast::Receiver` 提供同步的 `try_recv()` 方法，可在 `spawn_blocking` 的同步上下文中直接调用。因此 **不需要** 额外的桥接任务（bridge task），TUI 直接持有 `broadcast::Receiver<Bus_Event>`。

优势：
- 零额外任务开销
- 事件延迟最小（1 次 try_recv 而非 broadcast → mpsc → try_recv 的 2 次）
- 代码更简洁

## 3. TUI_Loop 签名变更

旧签名：
```rust
pub fn TUI_Loop(
    mut ui_rx: mpsc::Receiver<Ui_Message>,
    cli_tx: mpsc::Sender<CLI_Command>,
)
```

新签名：
```rust
pub fn TUI_Loop(
    mut event_rx: broadcast::Receiver<Bus_Event>,
    user_cmd_tx: mpsc::Sender<UserCommand>,
)
```

事件接收循环从：
```rust
while let Ok(msg) = ui_rx.try_recv() {
    Handle_Ui_Message(&mut app, msg);
}
```

改为：
```rust
loop {
    match event_rx.try_recv() {
        Ok(event) => Handle_Bus_Event(&mut app, event),
        Err(broadcast::error::TryRecvError::Empty) => break,
        Err(broadcast::error::TryRecvError::Lagged(n)) => {
            app.Add_Log(format!("⚠ 丢失 {} 条事件", n));
        }
        Err(broadcast::error::TryRecvError::Closed) => {
            app.should_quit = true;
            break;
        }
    }
}
```

## 4. Bus_Event → App 状态映射

`Handle_Bus_Event()` 替代原 `Handle_Ui_Message()`，映射关系：

| Bus_Event | App 状态更新 |
|-----------|-------------|
| `Log { message }` | `app.Add_Log(message)` |
| `Error { message }` | `app.Add_Log(format!("[错误] {}", message))` |
| `Peer_Discovered { peer_id }` | `app.Update_Peer(peer_id, false)` + 日志 |
| `Peer_Left { peer_id }` | `app.Remove_Peer(&peer_id)` + 日志 |
| `Connection_Established { peer_id }` | `app.Update_Peer(peer_id, true)` + 日志 |
| `Connection_Closed { peer_id }` | `app.Update_Peer(peer_id, false)` + 日志 |
| `File_Progress { file_name, direction, peer, sent, total }` | `app.job = Job_State::File_Transfer { ... }` |
| `Inference_Started { model_name, device_count, layer_range, .. }` | `app.job = Job_State::Inference { ... }` |
| `Inference_Token { token, .. }` | `app.command_output.output_text += &token` + 自动滚动 |
| `Inference_Completed { text, tokens, tok_per_sec, total_secs, .. }` | `app.command_output` 完成状态 + `app.job = Idle` |
| `Job_Created { job_id, kind, model_name }` | `app.Add_Log(...)` |
| `Job_State_Changed { job_id, phase }` | `app.Add_Log(...)` |
| `Job_Completed { job_id, result }` | `app.job = Job_State::Idle` + 日志 |
| `Device_Changed { device }` | `app.device = device` + 日志 |

## 5. UserCommand 命令映射

`Handle_Command_Input()` 构造 `UserCommand` 而非旧的 `CLI_Command`：

| 用户输入 | 构造的 UserCommand | 回复类型 |
|----------|-------------------|---------|
| `run <model>` | `UserCommand::Run { model_path, reply }` | `Result<JobId, String>` |
| `quit` / `exit` | `UserCommand::Quit { reply }` | `()` |
| `display-peer` / `dp` | `UserCommand::DisplayPeer { reply }` | `Result<Vec<String>, String>` |
| `set-device cpu/cuda` | `UserCommand::SetDevice { device, reply }` | `Result<(), String>` |
| `ls` | `UserCommand::List { reply }` | `Result<Vec<String>, String>` |
| `distribute <model> <peers>` | `UserCommand::DistributeModel { model_path, peers, reply }` | `Result<JobId, String>` |
| `cancel <job_id>` | `UserCommand::Cancel { job_id, reply }` | `Result<(), String>` |
| `send <file> <peer>` | `UserCommand::Send { file_path, peer_id, reply }` | `Result<JobId, String>` |
| `clear` | 本地处理（清空日志） | — |

同步等待回复模式保持不变（oneshot channel + `blocking_recv()`）。

## 6. 悬置项：LLM_IO_Broker 的 IoFrontend 通道

~~用户输入 prompt → `IoFrontend.input_tx` → ML Engine 这条路径暂不实现。~~

**已解决：见第 9 节 — 双输入框设计。**

## 7. 文件改动清单

| 文件 | 改动内容 |
|------|---------|
| `Src/TUI/mod.rs` | 核心改造：移除 Control 依赖，接入 Bus_Event + UserCommand |
| `Src/TUI/app.rs` | **不动**（纯状态管理，无外部依赖） |
| `Src/TUI/log_panel.rs` | **不动** |
| `Src/TUI/network_panel.rs` | **不动** |
| `Src/TUI/job_panel.rs` | **不动** |
| `Src/TUI/command_panel.rs` | **不动** |
| `Src/lib.rs` | 添加 `pub mod tui;` |
| `Src/main.rs` | 重写：组装各模块 → Subscribe → spawn_blocking(TUI_Loop) → Core::run() |

## 8. main.rs 组装伪代码

```rust
#[tokio::main]
async fn main() {
    // 1. 加载配置
    let config = Read_Config(...);
    
    // 2. 初始化各模块
    let event_bus = Arc::new(EventBus::New(1024));
    let storage = StorageManager::New(...).await;
    let network = Network_Service::new(...);
    let ml_engine = ...;
    let peer_manager = ...;
    let io_broker = Arc::new(LLM_IO_Broker::New());  // Arc 共享
    let tensor_io_broker = Tensor_IO_Broker::New();
    
    // 3. 组装 Capabilities
    let capabilities = Arc::new(Capabilities {
        io_broker: io_broker.clone(),  // Arc clone
        ...
    });
    
    // 4. 创建通道
    let (user_cmd_tx, user_cmd_rx) = mpsc::channel::<UserCommand>(32);
    let event_rx = event_bus.Subscribe();
    
    // 5. 启动 TUI (spawn_blocking)
    let io_broker_for_tui = io_broker.clone();
    tokio::task::spawn_blocking(move || {
        TUI_Loop(event_rx, user_cmd_tx, io_broker_for_tui);
    });
    
    // 6. 启动 Core
    let compiler = Arc::new(Compiler);
    let core = Core::new(compiler, capabilities, config_path, user_cmd_rx, inbound_rx, net_inbound_rx);
    core.run().await;
}
```

---

## 9. 双输入框设计 — 推理交互集成

### 9.1 设计目标

让模型推理功能重新上线。当前 IO Broker 的 IoFrontend 端点悬置未被取走，导致 ML Session 在 `Input` 指令处阻塞。通过双输入框设计，TUI 直接从 IO Broker 取走 IoFrontend，建立与 ML Session 的文本通道。

### 9.2 IO Broker 会合点设计

IO Broker 是独立的会合点（rendezvous point），两侧异步独立取走各自的端点：

```
                    ┌─────────────────────┐
Core:               │   LLM_IO_Broker     │
  Allocate(job_id)  │   HashMap<JobId,    │
  Take_ML_Side() ──→│     ChannelEntry>   │←── Take_Frontend()  :TUI
                    └─────────────────────┘
                         Arc 共享
```

- Core 负责 `Allocate` + `Take_ML_Side`（注入 Executor）
- TUI 负责 `Take_Frontend`（获取前端端点）
- 两侧互不感知，通过 job_id 索引同一通道对

### 9.3 前置修改：io_broker 改为 Arc 共享

```rust
// Capabilities 结构体
pub struct Capabilities {
    pub io_broker: Arc<LLM_IO_Broker>,  // 从 LLM_IO_Broker 改为 Arc<LLM_IO_Broker>
    // ...
}
```

```rust
// TUI_Loop 新签名
pub fn TUI_Loop(
    mut event_rx: broadcast::Receiver<Bus_Event>,
    user_cmd_tx: mpsc::Sender<UserCommand>,
    io_broker: Arc<LLM_IO_Broker>,  // 新增
)
```

### 9.4 新布局

```text
┌──────────────────────────┬───────────────┐
│ Log (70%)                │ Network (30%) │  ← Min(6) 自适应
├──────────────────────────┴───────────────┤
│ Job (固定 3 行)                           │  ← Length(3)
├──────────────────────────────────────────┤
│ Command Output (固定 8 行)                │  ← Length(8) 推理输出流式显示
├──────────────────────────────────────────┤
│ Prompt> 你好             (固定 3 行)      │  ← Length(3) Prompt 输入
├──────────────────────────────────────────┤
│ pleiades> run model.gguf (固定 3 行)      │  ← Length(3) 系统命令（最底部）
└──────────────────────────────────────────┘
```

布局 constraints：
```rust
let constraints = vec![
    Constraint::Min(6),       // Log + Network
    Constraint::Length(3),    // Job
    Constraint::Length(8),    // Command Output
    Constraint::Length(3),    // Prompt 输入
    Constraint::Length(3),    // 命令输入（最底部）
];
```

### 9.5 焦点管理

- **Tab 键** 切换焦点：Command 输入框 ↔ Prompt 输入框
- 当前焦点框以高亮边框（绿色）显示，非焦点框为灰色边框
- 默认焦点在 Command 输入框

### 9.6 Prompt 输入框状态机

| 状态 | 外观 | Enter 行为 |
|------|------|-----------|
| 无活跃 session | 灰色边框 + "无活跃会话" 提示 | 忽略 |
| 有活跃 session（空闲） | 正常边框 + "Prompt>" | `input_tx.blocking_send(text)` |
| 推理生成中 | 边框显示 "生成中..." | 忽略（等待输出结束） |

### 9.7 App 新增字段

```rust
pub struct App {
    // ... 现有字段 ...
    
    /// 当前输入焦点
    pub focus: InputFocus,
    /// Prompt 输入缓冲区（独立于 command 的 input_buffer）
    pub prompt_buffer: String,
    /// Prompt 光标位置
    pub prompt_cursor: usize,
    /// 当前活跃的 IoFrontend（单 session，暂不支持多 session）
    pub active_frontend: Option<IoFrontend>,
    /// 当前活跃的 job_id
    pub active_job_id: Option<JobId>,
}

/// 输入焦点枚举
pub enum InputFocus {
    Command,
    Prompt,
}
```

### 9.8 完整数据流

```text
用户操作                               系统内部
─────────                             ─────────
① [Command框] "run model.gguf" Enter
                                      → UserCommand::Run → Core
                                      → Core: Allocate(job_id) + Take_ML_Side → 注入 Executor
                                      → spawn Job → reply Ok(job_id)
② TUI 收到 job_id
   Handle::block_on(io_broker.Take_Frontend(job_id))
   app.active_frontend = Some(frontend)
   Prompt 框激活
                                      → ML Session_Thread 启动
                                      → Execute() → Input 指令：阻塞等待 io_handle.input_rx

③ [Prompt框] "你好" Enter
   frontend.input_tx.blocking_send("你好")
                                      → Session: 收到 prompt
                                      → Encode → Prefill → CopyMeta → Sample → Decode → Output
                                      → output_tx.blocking_send(token)

④ TUI 每帧轮询:
   while let Ok(token) = frontend.output_rx.try_recv() {
       app.command_output.output_text.push_str(&token);
   }
                                      → Session: Loop [BreakIf, Inference, Sample, Decode, Output]
                                      → 持续 send tokens

⑤ 推理结束：
   frontend.output_rx.try_recv() → Err(TryRecvError::Disconnected)
   app.active_frontend = None
   Prompt 框恢复灰色
```

### 9.9 TUI 中调用异步 Broker 方法

TUI 运行在 `spawn_blocking` 线程中，`Take_Frontend` 是 async 方法。使用 `Handle::block_on()` 桥接：

```rust
let rt = tokio::runtime::Handle::current();
match rt.block_on(io_broker.Take_Frontend(job_id)) {
    Ok(frontend) => {
        app.active_frontend = Some(frontend);
        app.active_job_id = Some(job_id);
        app.Add_Log(format!("会话已建立, Job #{}", job_id.0));
    }
    Err(e) => {
        app.Add_Log(format!("[错误] 获取前端通道失败: {}", e));
    }
}
```

### 9.10 推理输出轮询

在 TUI 事件循环的 `2b` 位置（Bus_Event 处理之后）增加推理输出轮询：

```rust
// 2b-extra: 轮询推理输出
if let Some(ref mut frontend) = app.active_frontend {
    let mut disconnected = false;
    loop {
        match frontend.output_rx.try_recv() {
            Ok(token) => {
                app.command_output.output_text.push_str(&token);
                app.command_output.token_count += 1;
                // 自动滚动
                let line_count = app.command_output.output_text.lines().count();
                app.command_scroll = line_count.saturating_sub(1);
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                disconnected = true;
                break;
            }
        }
    }
    if disconnected {
        app.active_frontend = None;
        app.active_job_id = None;
        app.command_output.completed = true;
        app.Add_Log("推理会话已结束".to_string());
    }
}
```

### 9.11 约束：暂时单 session

- 一次只允许一个活跃推理 session
- 如果已有活跃 session，`run` 命令返回错误提示 "已有活跃会话，请等待完成或 cancel"
- 后续扩展多 session 管理时再调整

---

## 10. Bug 修复：Log 面板文本自动换行

当前 Log 面板的 `Paragraph` 未启用 `wrap`，导致长文本超出面板宽度时被截断不可见。

### 修复方式

在 `log_panel.rs` 的 `Render` 函数中，为 `Paragraph` 添加 `.wrap(Wrap { trim: false })`：

```rust
use ratatui::widgets::Wrap;

let paragraph = Paragraph::new(lines)
    .block(block)
    .wrap(Wrap { trim: false });  // 启用自动换行，不裁剪空白
```

---

## 11. 改动清单（第二阶段）

| 文件 | 改动内容 |
|------|---------|
| `Src/Orchestrator/mod.rs` | `Capabilities.io_broker` 类型从 `LLM_IO_Broker` 改为 `Arc<LLM_IO_Broker>` |
| `Src/main.rs` | `io_broker` 创建为 Arc，clone 给 Capabilities 和 TUI |
| `Src/TUI/mod.rs` | 1. `TUI_Loop` 增加 `io_broker` 参数<br>2. 增加 Prompt 输入框渲染<br>3. 增加推理输出轮询逻辑<br>4. 增加 Tab 焦点切换<br>5. 增加 Prompt Enter 处理 |
| `Src/TUI/app.rs` | 新增 `InputFocus`、`prompt_buffer`、`prompt_cursor`、`active_frontend`、`active_job_id` 字段 |
| `Src/TUI/log_panel.rs` | 添加 `.wrap(Wrap { trim: false })` 启用自动换行 |
| `Src/Orchestrator/core.rs` | `self.capabilities.io_broker.Xxx()` 调用无需改动（Arc 实现了 Deref） |
| 所有使用 `capabilities.io_broker` 的测试 | 将 `LLM_IO_Broker::New()` 改为 `Arc::new(LLM_IO_Broker::New())` |
