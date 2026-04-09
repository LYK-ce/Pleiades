//Presented by KeJi
//Date ： 2026-04-09

//! TUI 模块 - 终端图形界面
//!
//! 使用 ratatui + crossterm 实现终端 UI，替代纯文本 CLI。
//! 通过 mpsc channel 与 Control 层解耦通信。
//!
//! ## 子模块
//! - app: 应用状态管理
//! - log_panel: Log 显示区
//! - network_panel: Network 显示区
//! - job_panel: Job 显示区
//! - command_panel: Command 显示区
//!
//! ## 布局
//! ```text
//! ┌─────────────────────────┬────────────────┐
//! │ Log (70%)               │ Network (30%)  │
//! ├─────────────────────────┴────────────────┤
//! │ Job (始终显示, 3行)                       │
//! ├──────────────────────────────────────────┤
//! │ Command (按需弹出)                       │
//! ├──────────────────────────────────────────┤
//! │ 输入栏 (3行)                             │
//! └──────────────────────────────────────────┘
//! ```

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

pub mod app;
pub mod log_panel;
pub mod network_panel;
pub mod job_panel;
pub mod command_panel;

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use tokio::sync::{mpsc, oneshot};

use app::{App, View_Mode, Job_State, Command_Output, Transfer_Direction};

// Ui_Message 定义在 Control 层（crate::control::ui_message），
// 任何 UI 实现都通过引用此类型获取显示数据。
use crate::control::ui_message::Ui_Message;
use crate::control::cli::CLI_Command;

// ============================================================
// TUI 主循环
// ============================================================

/// TUI 主循环入口
///
/// 在 `tokio::task::spawn_blocking` 中调用。
/// 初始化终端 → 事件循环（渲染 + 键盘 + UI消息） → 恢复终端。
///
/// # 参数
/// - `ui_rx`: 接收 Control 层发来的 UI 消息
/// - `cli_tx`: 发送用户命令给 Control 层
pub fn TUI_Loop(mut ui_rx: mpsc::Receiver<Ui_Message>, cli_tx: mpsc::Sender<CLI_Command>) {
    // 1. 初始化终端
    let mut terminal = match ratatui::init() {
        terminal => terminal,
    };

    let mut app = App::New();
    app.Add_Log("Pleiades TUI 已启动".to_string());

    // 2. 事件循环
    loop {
        // 2a. 渲染
        if terminal.draw(|frame| Render(frame, &app)).is_err() {
            break;
        }

        // 2b. 非阻塞接收所有待处理的 UI 消息
        while let Ok(msg) = ui_rx.try_recv() {
            Handle_Ui_Message(&mut app, msg);
        }

        // 2c. 处理键盘事件（50ms 超时，约 20fps）
        if crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                // 只处理按下事件（避免重复）
                if key.kind == KeyEventKind::Press {
                    Handle_Key_Event(&mut app, key.code, key.modifiers, &cli_tx);
                }
            }
        }

        // 2d. 检查退出标志
        if app.should_quit {
            let _ = cli_tx.blocking_send(CLI_Command::Quit);
            break;
        }
    }

    // 3. 恢复终端
    ratatui::restore();
}

// ============================================================
// UI 消息处理
// ============================================================

/// 处理 UI 消息，更新 App 状态
fn Handle_Ui_Message(app: &mut App, msg: Ui_Message) {
    match msg {
        Ui_Message::Log(text) => {
            app.Add_Log(text);
        }
        Ui_Message::State_Change(state) => {
            match state.as_str() {
                "Idle" => {
                    app.view_mode = View_Mode::Idle;
                    app.job = Job_State::Idle;
                }
                "Busy" => {
                    app.view_mode = View_Mode::Busy;
                }
                _ => {}
            }
            app.Add_Log(format!("状态变更: {}", state));
        }
        Ui_Message::Peer_Discovered(peer_id) => {
            app.Update_Peer(peer_id.clone(), false);
            app.Add_Log(format!("发现节点: {}", peer_id));
        }
        Ui_Message::Peer_Left(peer_id) => {
            app.Remove_Peer(&peer_id);
            app.Add_Log(format!("节点离开: {}", peer_id));
        }
        Ui_Message::Connection_Established(peer_id) => {
            app.Update_Peer(peer_id.clone(), true);
            app.Add_Log(format!("连接建立: {}", peer_id));
        }
        Ui_Message::Connection_Closed(peer_id) => {
            app.Update_Peer(peer_id.clone(), false);
            app.Add_Log(format!("连接断开: {}", peer_id));
        }
        Ui_Message::File_Progress {
            file_name,
            direction,
            peer,
            sent,
            total,
        } => {
            let dir = if direction == "send" {
                Transfer_Direction::Send
            } else {
                Transfer_Direction::Receive
            };
            app.job = Job_State::File_Transfer {
                direction: dir,
                file_name,
                peer,
                sent,
                total,
            };
        }
        Ui_Message::Job_Inference {
            model_name,
            device_count,
            layer_range,
            phase,
        } => {
            app.job = Job_State::Inference {
                model_name,
                device_count,
                layer_range,
                phase,
            };
        }
        Ui_Message::Job_Idle => {
            app.job = Job_State::Idle;
            app.view_mode = View_Mode::Idle;
        }
        Ui_Message::Inference_Token(token) => {
            app.command_output.output_text.push_str(&token);
            app.command_output.token_count += 1;
            app.view_mode = View_Mode::Busy_Coordinator;
        }
        Ui_Message::Inference_Complete {
            text,
            tokens,
            tok_per_sec,
            total_secs,
        } => {
            app.command_output.output_text = text;
            app.command_output.token_count = tokens;
            app.command_output.tok_per_sec = tok_per_sec;
            app.command_output.total_secs = total_secs;
            app.command_output.completed = true;
            app.job = Job_State::Idle;
            app.Add_Log(format!(
                "推理完成: {} tokens, {:.1} tok/s, {:.1}s",
                tokens, tok_per_sec, total_secs
            ));
        }
        Ui_Message::Error(text) => {
            app.Add_Log(format!("[错误] {}", text));
        }
        Ui_Message::Device_Change(device) => {
            app.Add_Log(format!("设备已切换: {}", device.to_uppercase()));
            app.device = device;
        }
    }
}

// ============================================================
// 键盘事件处理
// ============================================================

/// 处理键盘事件
fn Handle_Key_Event(app: &mut App, key_code: KeyCode, modifiers: KeyModifiers, cli_tx: &mpsc::Sender<CLI_Command>) {
    // Ctrl+C: 强制退出
    if modifiers.contains(KeyModifiers::CONTROL) && key_code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    match key_code {
        // 回车: 提交命令
        KeyCode::Enter => {
            let input = app.Take_Input();
            if !input.is_empty() {
                Handle_Command_Input(app, &input, cli_tx);
            }
        }
        // Esc: 清空输入
        KeyCode::Esc => {
            app.Clear_Input();
        }
        // Backspace: 删除字符
        KeyCode::Backspace => {
            app.Delete_Char();
        }
        // ↑: 日志向上滚动
        KeyCode::Up => {
            app.Scroll_Up();
        }
        // ↓: 日志向下滚动
        KeyCode::Down => {
            app.Scroll_Down();
        }
        // 普通字符输入
        KeyCode::Char(c) => {
            app.Input_Char(c);
        }
        _ => {}
    }
}

/// 处理用户输入的命令，通过 cli_tx 发送给 Control 层
fn Handle_Command_Input(app: &mut App, input: &str, cli_tx: &mpsc::Sender<CLI_Command>) {
    let trimmed = input.trim();

    if trimmed == "quit" || trimmed == "exit" {
        app.should_quit = true;
        return;
    }

    if trimmed == "clear" {
        app.logs.clear();
        app.log_scroll = 0;
        return;
    }

    // 每次新命令清空 Command 面板
    app.command_output = Command_Output::New();

    if trimmed == "ls" {
        // 列出 Pleiades_Workspace 下的文件
        app.Add_Log("执行命令: ls".to_string());
        let workspace_dir = std::path::Path::new("Pleiades_Workspace");
        if workspace_dir.exists() {
            let mut file_list = String::new();
            match std::fs::read_dir(workspace_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let metadata = entry.metadata();
                        let size_str = match metadata {
                            Ok(m) => {
                                let size = m.len();
                                if size > 1_073_741_824 {
                                    format!("{:.2} GB", size as f64 / 1_073_741_824.0)
                                } else if size > 1_048_576 {
                                    format!("{:.2} MB", size as f64 / 1_048_576.0)
                                } else if size > 1024 {
                                    format!("{:.1} KB", size as f64 / 1024.0)
                                } else {
                                    format!("{} B", size)
                                }
                            }
                            Err(_) => "???".to_string(),
                        };
                        file_list.push_str(&format!("  {} ({})\n", name, size_str));
                    }
                    if file_list.is_empty() {
                        file_list = "  (空目录)".to_string();
                    }
                }
                Err(e) => {
                    file_list = format!("  读取目录失败: {}", e);
                }
            }
            app.command_output.output_text = format!("Pleiades_Workspace/\n{}", file_list);
            app.command_output.completed = true;
        } else {
            app.command_output.output_text = "Pleiades_Workspace/ 目录不存在".to_string();
            app.command_output.completed = true;
        }
        return;
    }

    if trimmed.starts_with("set-device ") {
        let device_str = trimmed.strip_prefix("set-device ").unwrap_or("").trim().to_lowercase();
        if device_str == "cpu" || device_str == "cuda" {
            let (reply_tx, reply_rx) = oneshot::channel();
            let cmd = CLI_Command::SetDevice {
                device: device_str.clone(),
                reply: reply_tx,
            };

            app.Add_Log(format!("执行命令: set-device {}", device_str));

            if cli_tx.blocking_send(cmd).is_err() {
                app.Add_Log("[错误] Control 层已关闭".to_string());
                app.should_quit = true;
                return;
            }

            // 同步等待 Control 层的回复
            match reply_rx.blocking_recv() {
                Ok(Ok(msg)) => {
                    app.command_output.output_text = msg;
                    app.command_output.completed = true;
                }
                Ok(Err(e)) => {
                    app.command_output.output_text = format!("错误: {}", e);
                    app.command_output.completed = true;
                }
                Err(_) => {
                    app.command_output.output_text = "Control 层未响应".to_string();
                    app.command_output.completed = true;
                }
            }
        } else {
            app.command_output.output_text = format!("不支持的设备: '{}'\n用法: set-device cpu/cuda", device_str);
            app.command_output.completed = true;
        }
        return;
    }

    if trimmed.starts_with("run ") {
        // 解析 run 命令: run <model_path> <prompt>
        let rest = trimmed.strip_prefix("run ").unwrap_or("").trim();
        if let Some(space_idx) = rest.find(' ') {
            let model_path_str = &rest[..space_idx];
            let prompt = rest[space_idx + 1..].trim();

            // 去掉 prompt 两端引号
            let prompt = if (prompt.starts_with('"') && prompt.ends_with('"'))
                || (prompt.starts_with('\'') && prompt.ends_with('\''))
            {
                &prompt[1..prompt.len() - 1]
            } else {
                prompt
            };

            if prompt.is_empty() {
                app.command_output.output_text = "错误: 缺少 prompt 参数\n用法: run <model_path> <prompt>".to_string();
                app.command_output.completed = true;
                return;
            }

            let (reply_tx, _reply_rx) = oneshot::channel();
            let cmd = CLI_Command::Run {
                model_path: PathBuf::from(model_path_str),
                prompt: prompt.to_string(),
                reply: reply_tx,
            };

            app.Add_Log(format!("执行命令: run {} {}", model_path_str, prompt));
            app.command_output.output_text = "等待推理输出...".to_string();

            if cli_tx.blocking_send(cmd).is_err() {
                app.Add_Log("[错误] Control 层已关闭".to_string());
                app.should_quit = true;
            }
        } else {
            app.command_output.output_text = "用法: run <model_path> <prompt>".to_string();
            app.command_output.completed = true;
        }
        return;
    }

    // 未知命令
    app.command_output.output_text = format!(
        "未知命令: '{}'\n\n可用命令:\n  run <model_path> <prompt>  运行推理\n  set-device cpu/cuda       切换计算设备\n  ls                        列出工作区文件\n  clear                     清空日志\n  quit                      退出",
        trimmed
    );
    app.command_output.completed = true;
    app.Add_Log(format!("未知命令: '{}'", trimmed));
}

// ============================================================
// 总渲染函数
// ============================================================

/// 总渲染函数 — 计算布局并分发给各 panel 渲染
fn Render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Command 面板始终可见
    let constraints = vec![
        Constraint::Min(8),       // Log + Network（自适应填满）
        Constraint::Length(3),    // Job（固定 3 行）
        Constraint::Length(8),    // Command（固定 8 行，始终可见）
        Constraint::Length(3),    // 输入栏（固定 3 行）
    ];

    let rows = Layout::vertical(constraints).split(area);

    // Row 0: Log (70%) + Network (30%)
    let top = Layout::horizontal([
        Constraint::Percentage(70),
        Constraint::Percentage(30),
    ])
    .split(rows[0]);
    log_panel::Render(frame, top[0], app);
    network_panel::Render(frame, top[1], app);

    // Row 1: Job（始终显示）
    job_panel::Render(frame, rows[1], app);

    // Row 2: Command（始终显示）
    command_panel::Render(frame, rows[2], app);

    // Row 3: 输入栏
    Render_Input(frame, rows[3], app);
}

/// 渲染输入栏
fn Render_Input(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let input_text = Line::from(vec![
        Span::styled("pleiades> ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(app.input_buffer.as_str(), Style::default().fg(Color::White)),
    ]);

    let paragraph = Paragraph::new(input_text).block(block);
    frame.render_widget(paragraph, area);

    // 设置光标位置
    let cursor_x = area.x + 1 + "pleiades> ".len() as u16 + app.cursor_position as u16;
    let cursor_y = area.y + 1;
    if cursor_x < area.x + area.width - 1 {
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}
