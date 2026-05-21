# TUI 模块设计文档

## 概述

TUI（Terminal User Interface）模块为 Pleiades 提供终端图形界面，替代原来的纯文本 CLI 交互。  
采用 `ratatui` + `crossterm` 实现，通过 `mpsc` channel 与 Control 层解耦通信。

## 架构

```
┌──────────────┐    ui_tx: UiMessage     ┌──────────────┐
│  Control 层  │ ──────────────────────→ │   TUI 模块    │
│  control.rs  │                         │  (渲染+输入)  │
│              │ ←────────────────────── │              │
└──────────────┘    cli_tx: CLI_Command  └──────────────┘
```

TUI 模块**不依赖** Network、ML_Engine 的任何类型，只依赖：
- `UiMessage`（自定义枚举）
- `CLI_Command`（来自 Control 模块，简单枚举）
- `ratatui` / `crossterm`（TUI 框架）

## 通信协议

### UiMessage（Control → TUI）

```rust
pub enum UiMessage {
    // 日志
    Log(String),
    // 状态变化
    StateChange(String),          // "Idle" / "Busy"
    // 网络事件
    PeerDiscovered(String),
    PeerLeft(String),
    ConnectionEstablished(String),
    ConnectionClosed(String),
    // Job 状态
    JobUpdate(JobState),
    // 文件传输进度
    FileProgress { file_name: String, direction: String, peer: String, sent: u64, total: u64 },
    // 推理输出（逐 token）
    InferenceToken(String),
    // 推理完成
    InferenceComplete { text: String, tokens: usize, tok_per_sec: f64, total_secs: f64 },
    // 错误
    Error(String),
}
```

### CLI_Command（TUI → Control）

复用已有的 `CLI_Command` 枚举（Run / Quit）。

## 布局设计

### 四大显示区

| 区域 | 名称 | 位置 | 可见性 |
|------|------|------|--------|
| Log | 日志显示区 | 左上 70% | 始终 |
| Network | 网络显示区 | 右上 30% | 始终 |
| Job | 任务显示区 | 中间窄条(3行) | 始终（空闲显示"空闲"） |
| Command | 命令输出区 | Job下方 | 按需弹出（run 命令后出现） |
| 输入栏 | — | 最底部(3行) | 始终 |

### Idle 状态（3层）

```
┌──────────────────────────────────────────┬───────────────────┐
│ 📋 Log                                   │ 📡 Network        │
│  [时间] 日志内容...                       │  ● PeerId...      │
│                                          │    已连接 2ms     │
├──────────────────────────────────────────┴───────────────────┤
│ ⚙️ Job: 空闲                                                 │
├──────────────────────────────────────────────────────────────┤
│ pleiades> _                                                  │
└──────────────────────────────────────────────────────────────┘
```

### Busy 状态 — 工作节点（3层，Job 内容变化）

```
├──────────────────────────────────────────────────────────────┤
│ ⚙️ Job: Qwen3-0.6B │ 本机:层10-19 │ 阶段: 推理中            │
├──────────────────────────────────────────────────────────────┤
```

### Busy 状态 — 文件传输（3层，Job 显示进度条）

```
├──────────────────────────────────────────────────────────────┤
│ ⚙️ Job: 📤 发送 Qwen3-split.pgguf ██████░░░ 65%             │
├──────────────────────────────────────────────────────────────┤
```

### Busy 状态 — 协调者推理中（4层，弹出 Command 区）

```
├──────────────────────────────────────────────────────────────┤
│ ⚙️ Job: Qwen3-0.6B │ 3台设备 │ 本机:层0-9 │ 阶段: Decode    │
├──────────────────────────────────────────────────────────────┤
│ 💬 Command                                                   │
│  推理输出文本...█                                             │
│  45 tok │ 12.3 tok/s │ 3.7s                                 │
├──────────────────────────────────────────────────────────────┤
│ pleiades> run model.gguf "你好"                              │
└──────────────────────────────────────────────────────────────┘
```

## 文件结构

```
Src/TUI/
├── mod.rs              ← 模块入口：UiMessage + TUI_Loop + 总渲染
├── app.rs              ← App 状态结构体
├── log_panel.rs        ← Log 显示区渲染
├── network_panel.rs    ← Network 显示区渲染
├── job_panel.rs        ← Job 显示区渲染
└── command_panel.rs    ← Command 显示区渲染
```

## 键盘交互

| 按键 | 功能 |
|------|------|
| Enter | 提交命令 |
| Esc | 清空输入 |
| ↑↓ | 滚动日志 |
| Backspace | 删除字符 |
| Ctrl+C | 强制退出 |

## 依赖

- `ratatui = "0.29"` — TUI 渲染框架
- `crossterm = "0.28"` — 终端操作（raw mode, 键盘事件）

## 运行方式

在 `tokio::task::spawn_blocking` 中运行（替代当前 CLI_Loop 的位置）：

```rust
let (cli_tx, cli_rx) = mpsc::channel::<CLI_Command>(32);
let (ui_tx, ui_rx) = mpsc::channel::<UiMessage>(256);
tokio::task::spawn_blocking(move || { TUI_Loop(cli_tx, ui_rx); });
```
