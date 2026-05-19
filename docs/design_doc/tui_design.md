# TUI 设计文档

Presented by KeJi
Date ： 2026-05-19

## 1. 模块概述

`TUI` 是 Pleiades 分布式推理系统的**终端图形界面**，负责可视化系统状态并接收用户命令。

### 核心定义

> **TUI = ratatui + crossterm + EventBus 订阅 + mpsc 命令发送。**
> 基于 `tokio::sync::broadcast` 订阅 EventBus 事件更新 5 个面板。
> 通过 `std::sync::mpsc` 将用户命令发送给 Orchestrator Core。
> 双输入框设计（命令 + Prompt），Tab 键切换焦点。

### 模块结构

```
TUI/
├── mod.rs             ← 入口: TUI_Loop + Handle_Bus_Event + Render + 键盘/鼠标
├── app.rs             ← App 状态结构体 + 方法
├── log_panel.rs       ← Log 面板渲染
├── network_panel.rs   ← Network 面板渲染
├── job_panel.rs       ← Job 面板渲染
└── command_panel.rs   ← Command 面板渲染
```

### 数据流总览

```
                      ┌─────────────┐
                      │  EventBus   │
                      │ (broadcast) │
                      └──────┬──────┘
                             │ try_recv()
                             ▼
┌──────────────────────────────────────────────────┐
│                   TUI_Loop                        │
│                                                   │
│  ┌─────────────────┐     ┌───────────────────┐   │
│  │ Handle_Bus_Event │────▶│ App (状态)         │   │
│  └─────────────────┘     │  .logs             │   │
│                           │  .peers            │   │
│  ┌─────────────────┐     │  .job              │   │
│  │ Handle_Key_Event │     │  .command_output   │   │
│  └────────┬────────┘     └────────┬──────────┘   │
│           │                       │               │
│           │ user_cmd_tx           │ Render()      │
│           ▼                       ▼               │
│  ┌────────────────┐     ┌─────────────────────┐   │
│  │ UserCommand    │     │ ratatui Frame        │   │
│  │ → Orchestrator │     │ 5 行 6 面板          │   │
│  └────────────────┘     └─────────────────────┘   │
│                                                   │
│  crossterm::event::poll(50ms)                     │
└──────────────────────────────────────────────────┘
```


---

## 2. 布局设计

### 2.1 5 行布局

```
┌──────────────────────────────────────────────┐
│ Log 面板 (70%)          │ Network 面板 (30%)  │ Row 0: Min 6行
├──────────────────────────────────────────────┤
│ Job 面板                                     │ Row 1: 固定 3行
├──────────────────────────────────────────────┤
│ Command Output 面板                          │ Row 2: 固定 8行
├──────────────────────────────────────────────┤
│ Prompt 输入栏                                │ Row 3: 固定 3行
├──────────────────────────────────────────────┤
│ 命令输入栏                                   │ Row 4: 固定 3行
└──────────────────────────────────────────────┘
```

### 2.2 视图模式

| 模式 | 布局 | 触发条件 |
|------|------|---------|
| `Idle` | 5 行（全部可见） | 初始状态 / 推理结束 |
| `Busy` | 5 行（Job 面板显示任务信息） | 推理开始 |
| `Busy_Coordinator` | 5 行（Command 面板显示推理输出） | 第一个 token 到达 |

> **注意**：当前三种模式的布局约束相同（都是 5 行），区别仅在于 Job 面板和 Command 面板的**内容**变化。未来可为 Coordinator 模式增加额外面板行。

---

## 3. App 状态结构

```rust
pub struct App {
    pub view_mode: View_Mode,         // 视图模式
    pub device: String,               // 当前设备 ("cpu"/"cuda")

    // Log 面板
    pub logs: Vec<String>,            // 日志列表 [带时间戳]
    pub log_scroll: usize,            // 滚动偏移

    // Network 面板
    pub peers: Vec<Peer_Display>,     // 节点列表

    // Job 面板
    pub job: Job_State,               // 当前任务状态

    // Command 面板
    pub command_output: Command_Output, // 命令/推理输出
    pub command_scroll: usize,        // 滚动偏移

    // 输入
    pub input_buffer: String,         // 命令输入
    pub cursor_position: usize,
    pub prompt_buffer: String,        // Prompt 输入
    pub prompt_cursor: usize,
    pub focus: InputFocus,            // Command / Prompt

    // Lua 补全
    pub available_commands: Vec<String>,

    // 控制
    pub should_quit: bool,
    pub active_job_id: Option<JobId>,
    
    // 面板区域（鼠标命中检测）
    pub log_area: Rect,
    pub command_area: Rect,
}
```

### 3.1 子结构体

#### Peer_Display — Network 面板项

```rust
pub struct Peer_Display {
    pub peer_id: String,   // 截断显示 (12D3Koo...h7gT)
    pub connected: bool,   // 绿● / 红○
}
```

> ⚠️ **已知不足**：`Peer_Display` 仅含 peer_id + 连接状态，缺失 IP 地址、带宽、内存、持有的模型列表等关键信息。这些数据在 `PeerInfo` 中已存在但 EventBus `peer_*` 事件的 JSON payload 未携带。

#### Job_State — Job 面板内容

```rust
pub enum Job_State {
    Idle,
    File_Transfer {            // 文件传输进度
        direction: Transfer_Direction,
        file_name: String,
        peer: String,
        sent: u64,
        total: u64,
    },
    Inference {                // 推理任务
        model_name: String,
        device_count: usize,
        layer_range: String,
        phase: String,         // "初始化"/"加载模型"/"推理中"
    },
}
```

#### Command_Output — Command 面板内容

```rust
pub struct Command_Output {
    pub output_text: String,   // 输出文本
    pub token_count: usize,    // 已生成 token 数
    pub tok_per_sec: f64,      // 生成速度
    pub total_secs: f64,       // 总耗时
    pub completed: bool,       // 是否已完成
}
```

#### InputFocus — 焦点切换

```rust
pub enum InputFocus {
    Command,  // 底部命令输入栏
    Prompt,   // Command Output 下方的 Prompt 栏
}
```

---

## 4. 事件循环

### 4.1 主循环结构

```
loop {
    // ① 渲染 (每次循环都绘制)
    terminal.draw(|frame| Render(frame, &mut app))

    // ② 排空 EventBus (非阻塞，一次性处理所有积压事件)
    loop {
        match event_rx.try_recv() {
            Ok(event)  → Handle_Bus_Event(&mut app, event)
            Err(Empty) → break
            Err(Lagged) → Add_Log + continue
            Err(Closed) → should_quit = true
        }
    }

    // ③ 处理键盘/鼠标 (50ms 轮询，≈20fps)
    if poll(50ms) {
        match read() {
            Key  → Handle_Key_Event(…)
            Mouse → Handle_Mouse_Event(…)
        }
    }

    // ④ 退出检查
    if should_quit {
        send(UserCommand::Quit) → break
    }
}
```

### 4.2 运行时

TUI_Loop 在 **`tokio::task::spawn_blocking`** 中运行：

```rust
// main.rs
tokio::task::spawn_blocking(move || {
    TUI::TUI_Loop(event_rx, user_cmd_tx);
});
```

> **原因**：`ratatui` 和 `crossterm` 不是 async 的，需要在阻塞线程中运行。EventBus 接收使用 `try_recv()`（非阻塞），命令发送使用 `blocking_send()`。

---

## 5. 面板详解

### 5.1 Log 面板 (`log_panel.rs`)

| 属性 | 值 |
|------|-----|
| 位置 | 左上 70% |
| 标题 | `📋 Log` |
| 数据源 | `app.logs: Vec<String>` |
| 事件 | `Notify` (全部), `State` (peer_* / job_* / inference_* 附带日志) |

**渲染格式**：
```
📋 Log ──────────────────────────────
[14:32:01] Pleiades TUI 已启动
[14:32:05] 发现节点: 12D3KooW...abc1
[14:32:06] 连接建立: 12D3KooW...abc1
[14:32:10] Job #1 已创建 [Run] 模型: qwen3
[14:32:12] [错误] 连接断开: 12D3KooW...abc1
```

> Notify 按级别加前缀：`Info` 无前缀，`Warn` 加 `[警告]`，`Error` 加 `[错误]`。

### 5.2 Network 面板 (`network_panel.rs`)

| 属性 | 值 |
|------|-----|
| 位置 | 右上 30% |
| 标题 | `📡 Network` |
| 数据源 | `app.peers: Vec<Peer_Display>` |
| 事件 | `State` (peer_discovered / peer_left / peer_connected / peer_disconnected) |

**渲染格式**：
```
📡 Network ────────────────
  ● 12D3Koo...h7gT  已连接
  ○ 12D3Koo...a2xP  已断开
```

- 绿 `●` = 已连接，红 `○` = 已断开
- PeerID 截断为 `前8位...后4位`
- 空状态显示 `"  等待节点连接..."`

> ⚠️ **改进方向**：应展示 IP:port、带宽、延迟、持有模型列表等 `PeerInfo` 中的丰富信息。

### 5.3 Job 面板 (`job_panel.rs`)

| 属性 | 值 |
|------|-----|
| 位置 | Row 1（全宽） |
| 标题 | `🔧 Job` |
| 数据源 | `app.job: Job_State` |
| 事件 | `State` (job_phase_changed / inference_started / inference_completed), `Stream` (file_progress) |

**三种渲染状态**：

```
Idle:
🔧 Job ───────────────────────────
  就绪

File_Transfer:
🔧 Job ───────────────────────────
  发送 model.gguf → 12D3Koo...h7gT
  ████████████░░░░░░  75% (768/1024 MB)

Inference:
🔧 Job ───────────────────────────
  模型: qwen3  设备: CUDA (1)  层: 0-15
  阶段: 推理中
```

### 5.4 Command Output 面板 (`command_panel.rs`)

| 属性 | 值 |
|------|-----|
| 位置 | Row 2（全宽） |
| 标题 | `💻 Command` |
| 数据源 | `app.command_output: Command_Output` |
| 事件 | `Output` (cmd_result / help), `Stream` (token), `State` (inference_completed) |

**两种内容模式**：

```
命令结果 (Output::cmd_result):
💻 Command ─────────────────────────
共 3 个节点:
  12D3KooW...abc1 [mem=4096MB latency=2ms]
  12D3KooW...def2 [mem=8192MB latency=5ms]
  12D3KooW...ghi3 [mem=2048MB latency=1ms]
                                    ← completed 时不显示底部状态

推理输出 (Stream::token 追加):
💻 Command ─────────────────────────
你好，这是一个测试。今天天气很好。
适合出去散步。
                                    ← 流式输出时逐 token 追加
  tokens: 15 | 30.5 tok/s | 0.49s  ← 推理完成后显示统计
```

### 5.5 Prompt 输入栏

| 属性 | 值 |
|------|-----|
| 位置 | Row 3（全宽） |
| 标题 | `💬 Prompt` |
| 操作 | Enter → `Handle_Prompt_Submit` → TODO（当前未发送任何命令） |

> ⚠️ **未实现**：Prompt 输入栏的提交逻辑为 stub，`Handle_Prompt_Submit` 函数体目前是 `todo!()`。

### 5.6 命令输入栏

| 属性 | 值 |
|------|-----|
| 位置 | Row 4（全宽，最底部） |
| 标题 | `>_ 输入` |
| 操作 | Enter → `Handle_Command_Input` → 解析 → `user_cmd_tx.blocking_send(UserCommand)` |

**支持的命令**（全部发送给 Orchestrator Core，结果通过 EventBus Output 返回）：

| 命令 | 别名 | UserCommand 变体 |
|------|------|-----------------|
| `display-peer` | `dp` | `DisplayPeer` |
| `set-device <cpu/cuda>` | — | `SetDevice { device }` |
| `run <script> <model>` | — | `Run { script, model_path }` |
| `pipeline <model> [strategy]` | — | `Pipeline { model_path, strategy }` |
| `distribute <model> <peer=start-end>...` | — | `DistributeModel { model_path, peers }` |
| `list` | `ls` | `List` |
| `send <file> <peer>` | — | `Send { file_path, peer_id }` |
| `profile <model>` | — | `Profile { model_id }` |
| `cancel <job_id>` | — | `Cancel { job_id }` |
| `quit` | `q` | `Quit { reply }` |
| `help` | `?` | → EventBus 输出内置命令列表 |
| `exec <command> [k=v ...]` | — | `Execute { command, params }` |

---

## 6. Bus_Event → Panel 映射

```
Handle_Bus_Event(app, event)
│
├── Notify { level, message }
│   └── Log 面板  ← app.Add_Log
│
├── State { payload: JSON }
│   ├── "peer_discovered"    → Network面板(新增) + Log
│   ├── "peer_left"          → Network面板(移除) + Log
│   ├── "peer_connected"     → Network面板(connected=true) + Log
│   ├── "peer_disconnected"  → Network面板(connected=false) + Log
│   ├── "job_created"        → Log
│   ├── "job_phase_changed"  → Log + Job面板(phase)
│   ├── "job_completed"      → Log + Job面板(→Idle) + view(→Idle)
│   ├── "inference_started"  → Log + Job面板(→Inference) + view(→Busy)
│   ├── "inference_completed"→ Log + Command面板(text/stats) + Job面板(→Idle)
│   └── "device_changed"     → Log + app.device
│
├── Stream { payload: JSON }
│   ├── "token"              → Command面板(output_text追加+scroll)
│   └── "file_progress"      → Job面板(→File_Transfer)
│
└── Output { payload: JSON }
    ├── "cmd_result"         → Command面板(output_text替换+completed)
    └── "help"               → Command面板(output_text替换)
```

---

## 7. 用户输入处理

### 7.1 键盘事件

| 键 | 作用 |
|----|------|
| **Enter** | 提交当前焦点输入框 |
| **Tab** | 切换焦点（Command ↔ Prompt） |
| **Backspace** | 删除光标前字符 |
| **← →** | 移动光标 |
| **Home / End** | 跳到行首/行尾 |
| **PageUp / PageDown** | Command/Log 面板上下翻页 |
| **Esc** | 清空当前输入框 |
| **Ctrl+C** | 退出 |
| **Ctrl+L** | 清空 Log |

### 7.2 鼠标事件

| 事件 | 目标区域 | 效果 |
|------|---------|------|
| 滚轮上/下 | Log 面板 | 日志滚动 |
| 滚轮上/下 | Command 面板 | 输出滚动 |
| 点击 | 输入框 | 设置光标位置 |

---

## 8. 已知限制

### 8.1 Network 面板信息不足

`Peer_Display` 仅含 `peer_id` + `connected`。EventBus 的 `peer_*` JSON payload 未携带 `PeerInfo` 中的 IP 地址、带宽、内存、模型列表等关键信息。用户需通过 `display-peer` 命令额外查询（且 `display-peer` 当前也只返回 mem+latency）。

**改进方向**：扩充 `peer_*` 事件的 JSON payload，或在 Network 面板中通过 `Peer_Management_Capability` 接口主动拉取完整信息。

### 8.2 Prompt 输入栏未实现

`Handle_Prompt_Submit` 函数体为 TODO。Prompt 输入栏的 UI 已就绪但无实际功能。设计意图是作为推理 Prompt 的输入口（类似 LLM chat 界面），需要等待 ML Engine 推理路径打通。

### 8.3 命令通过 EventBus 异步返回

`UserCommand` 大部分变体采用 fire-and-forget 模式（无 `reply` oneshot），Core 处理完后通过 `Bus_Event::Output::cmd_result` 返回结果。这种设计的好处是不阻塞 TUI 线程，但缺点是：
- 无法关联请求和响应
- 高并发命令时输出可能交错

### 8.4 无测试覆盖

TUI 模块没有自动化测试。所有功能依赖手动验证。

### 8.5 命令补全未接入 ProgramRegistry

`app.available_commands` 字段和 `set_available_commands()` 方法已定义，但当前命令补全仍使用硬编码列表，未从 `ProgramRegistry` 动态获取 Lua 脚本命令。

### 8.6 单消费者瓶颈

EventBus 使用 `broadcast` 但只有 TUI 一个订阅者。`broadcast` 对单消费者过重（每条消息 Clone）。若确定永远单消费者，可替换为 `mpsc::unbounded`。

---

## 9. TODO

### 高优先级

- [ ] Network 面板增强：展示 IP:port、带宽、延迟、持有模型列表
- [ ] `peer_*` EventBus JSON payload 扩充为完整 `PeerInfo` 信息
- [ ] 命令补全接入 `ProgramRegistry::command_names()`
- [ ] `Commands_Reloaded` 事件监听 → 动态刷新命令补全

### 中优先级

- [ ] Prompt 输入栏功能实现（推理 Prompt 输入 → ML Engine 流程）
- [ ] 命令请求-响应关联（如给每个 Output 带 `correlation_id`）
- [ ] `display-peer` 命令输出增强（展示全部 10 项 PeerInfo 字段）

### 低优先级

- [ ] TUI 单元测试 / 集成测试
- [ ] EventBus 替换为 mpsc（单消费者优化）
- [ ] 多消费者支持（HTTP 状态页、日志持久化）
- [ ] 命令历史记录（↑↓ 翻历史）
- [ ] 支持 resize 动态调整面板比例

---

## 10. 重构历史

| 变更 | 说明 |
|------|------|
| 初始实现 | 14 variant Bus_Event + 对应 14 个 handler 分支 |
| EventBus Reforge | Handle_Bus_Event 14→4 分支 + JSON 解析 + handle_state/handle_stream/handle_output 分派 |
| Orchestrator Reforge | UserCommand 变体精简，Remove reply 改为 EventBus Output |
| IO 迁移 | `llm_io`→`session`，`Pipeline`→`Execute` |
| 双输入框 | 新增 Prompt 输入栏 + InputFocus 焦点切换 |
| Lua 补全预留 | App 新增 `available_commands` 字段 + `set_available_commands()` |

