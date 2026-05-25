//Presented by KeJi
//Date ： 2026-04-30

//! TUI 模块 - 终端图形界面
//!
//! 使用 ratatui + crossterm 实现终端 UI。
//! 通过 EventBus (broadcast) 接收系统事件，通过 mpsc 发送 UserCommand 给 Orchestrator Core。
//! 通过 IO Broker (Arc 共享) 获取推理会话的前端端点。
//!
//! ## 子模块
//! - app: 应用状态管理
//! - log_panel: Log 显示区
//! - network_panel: Network 显示区
//! - job_panel: Job 显示区
//! - command_panel: Command 显示区
//!
//! ## 布局（双输入框）
//! ```text
//! ┌─────────────────────────┬────────────────┐
//! │ Log (70%)               │ Network (30%)  │
//! ├─────────────────────────┴────────────────┤
//! │ Job (始终显示, 3行)                       │
//! ├──────────────────────────────────────────┤
//! │ Command Output (始终显示, 8行)            │
//! ├──────────────────────────────────────────┤
//! │ Prompt> (3行)                            │
//! ├──────────────────────────────────────────┤
//! │ pleiades> (3行, 命令输入)                 │
//! └──────────────────────────────────────────┘
//! ```

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

pub mod app;
pub mod command_panel;
pub mod job_panel;
pub mod log_panel;
pub mod network_panel;

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use tokio::sync::{broadcast, mpsc, oneshot};

use app::{App, Command_Output, InputFocus, Job_State, Transfer_Direction, View_Mode};

use crate::event_bus::Bus_Event;
use crate::orchestrator::command::UserCommand;
use crate::orchestrator::job::JobId;

// ============================================================
// TUI 主循环
// ============================================================

/// TUI 主循环入口
///
/// 在 `tokio::task::spawn_blocking` 中调用。
/// 初始化终端 → 事件循环（渲染 + 键盘 + Bus_Event + 推理输出轮询） → 恢复终端。
///
/// # 参数
/// - `event_rx`: 从 EventBus::Subscribe() 获得的 broadcast Receiver
/// - `user_cmd_tx`: 发送用户命令给 Orchestrator Core
pub fn TUI_Loop(
    mut event_rx: broadcast::Receiver<Bus_Event>,
    user_cmd_tx: mpsc::Sender<UserCommand>,
    prompt_tx: broadcast::Sender<String>,
) {
    // 1. 初始化终端
    let mut terminal = ratatui::init();

    // 启用鼠标捕获（滚轮等事件）
    crossterm::execute!(std::io::stdout(), EnableMouseCapture).ok();

    let mut app = App::New(prompt_tx);
    app.Add_Log("Pleiades TUI 已启动".to_string());

    // 2. 事件循环
    loop {
        // 2a. 渲染（&mut app 使 command_panel 能钳位 command_scroll）
        if terminal.draw(|frame| Render(frame, &mut app)).is_err() {
            break;
        }

        // 2b. 非阻塞接收所有待处理的 Bus_Event
        loop {
            match event_rx.try_recv() {
                Ok(event) => Handle_Bus_Event(&mut app, event),
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    app.Add_Log(format!("⚠ 丢失 {} 条事件", n));
                    // continue 继续接收后续事件
                }
                Err(broadcast::error::TryRecvError::Closed) => {
                    app.should_quit = true;
                    break;
                }
            }
        }

        // 2c. 处理输入事件（50ms 超时，约 20fps）
        if crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    Handle_Key_Event(&mut app, key.code, key.modifiers, &user_cmd_tx);
                }
                Ok(Event::Mouse(mouse)) => {
                    Handle_Mouse_Event(&mut app, mouse.kind, mouse.column, mouse.row);
                }
                _ => {}
            }
        }

        // 2d. 检查退出标志
        if app.should_quit {
            let (reply_tx, _reply_rx) = oneshot::channel();
            let _ = user_cmd_tx.blocking_send(UserCommand::Quit { reply: reply_tx });
            break;
        }
    }

    // 3. 恢复终端
    crossterm::execute!(std::io::stdout(), DisableMouseCapture).ok();
    ratatui::restore();
}

// ============================================================
// Bus_Event 处理
// ============================================================

/// 处理 Bus_Event，更新 App 状态
fn Handle_Bus_Event(app: &mut App, event: Bus_Event) {
    match event {
        Bus_Event::Notify { level, message } => {
            let msg = match level {
                crate::event_bus::NotifyLevel::Error => format!("[错误] {message}"),
                crate::event_bus::NotifyLevel::Warn => {
                    app.Add_Log(format!("[警告] {message}"));
                    return;
                }
                crate::event_bus::NotifyLevel::Info => message,
            };
            app.Add_Log(msg);
        }
        Bus_Event::State { payload } => {
            let v = match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(v) => v,
                Err(e) => {
                    app.Add_Log(format!("[EventBus] State JSON 解析失败: {e}"));
                    return;
                }
            };
            handle_state(app, &v);
        }
        Bus_Event::Stream { payload } => {
            let v = match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(v) => v,
                Err(e) => {
                    app.Add_Log(format!("[EventBus] Stream JSON 解析失败: {e}"));
                    return;
                }
            };
            handle_stream(app, &v);
        }
        Bus_Event::Output { payload } => {
            let v = match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(v) => v,
                Err(e) => {
                    app.Add_Log(format!("[EventBus] Output JSON 解析失败: {e}"));
                    return;
                }
            };
            handle_output(app, &v);
        }
    }
}

// ============================================================
// State 事件处理
// ============================================================

fn handle_state(app: &mut App, v: &serde_json::Value) {
    match v["type"].as_str() {
        Some("peer_discovered") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            app.Update_Peer(peer_id.to_string(), false);
            app.Add_Log(format!("发现节点: {peer_id}"));
        }
        Some("peer_left") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            app.Remove_Peer(peer_id);
            app.Add_Log(format!("节点离开: {peer_id}"));
        }
        Some("peer_connected") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            app.Update_Peer(peer_id.to_string(), true);
            app.Add_Log(format!("连接建立: {peer_id}"));
        }
        Some("peer_disconnected") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            app.Update_Peer(peer_id.to_string(), false);
            app.Add_Log(format!("连接断开: {peer_id}"));
        }
        Some("job_created") => {
            let job_id = v["job_id"].as_u64().unwrap_or(0);
            let kind = v["kind"].as_str().unwrap_or("?");
            let model = v["model"].as_str().unwrap_or("");
            let msg = if model.is_empty() {
                format!("Job #{job_id} 已创建 [{kind}]")
            } else {
                format!("Job #{job_id} 已创建 [{kind}] 模型: {model}")
            };
            app.Add_Log(msg);
        }
        Some("job_phase_changed") => {
            let job_id = v["job_id"].as_u64().unwrap_or(0);
            let phase = v["phase"].as_str().unwrap_or("?");
            app.Add_Log(format!("Job #{job_id} 阶段: {phase}"));
            if let Job_State::Inference {
                phase: ref mut current_phase,
                ..
            } = app.job
            {
                *current_phase = phase.to_string();
            }
        }
        Some("job_completed") => {
            let job_id = v["job_id"].as_u64().unwrap_or(0);
            let result = v["result"].as_str().unwrap_or("?");
            app.Add_Log(format!("Job #{job_id} 完成: {result}"));
            if app.view_mode != View_Mode::Busy_Coordinator {
                app.job = Job_State::Idle;
                app.view_mode = View_Mode::Idle;
            }
        }
        Some("inference_started") => {
            let model = v["model"].as_str().unwrap_or("?").to_string();
            let devices = v["devices"].as_u64().unwrap_or(1) as usize;
            let layers = v["layers"].as_str().unwrap_or("?").to_string();
            app.job = Job_State::Inference {
                model_name: model.clone(),
                device_count: devices,
                layer_range: layers.clone(),
                phase: "初始化".to_string(),
            };
            app.view_mode = View_Mode::Busy;
            app.Add_Log(format!("推理开始: {model} [{layers}]"));
        }
        Some("inference_completed") => {
            let text = v["text"].as_str().unwrap_or("").to_string();
            let tokens = v["tokens"].as_u64().unwrap_or(0) as usize;
            let tok_per_sec = v["tok_per_sec"].as_f64().unwrap_or(0.0);
            let total_secs = v["total_secs"].as_f64().unwrap_or(0.0);
            app.command_output.output_text = text;
            app.command_output.token_count = tokens;
            app.command_output.tok_per_sec = tok_per_sec;
            app.command_output.total_secs = total_secs;
            app.command_output.completed = true;
            app.job = Job_State::Idle;
            app.Add_Log(format!(
                "推理完成: {tokens} tokens, {tok_per_sec:.1} tok/s, {total_secs:.1}s"
            ));
        }
        Some("device_changed") => {
            let device = v["device"].as_str().unwrap_or("?").to_string();
            app.Add_Log(format!("设备已切换: {}", device.to_uppercase()));
            app.device = device;
        }
        _ => {}
    }
}

// ============================================================
// Stream 事件处理
// ============================================================

fn handle_stream(app: &mut App, v: &serde_json::Value) {
    match v["type"].as_str() {
        Some("token") => {
            let text = v["text"].as_str().unwrap_or("");
            app.command_output.output_text.push_str(text);
            app.command_output.token_count += 1;
            app.view_mode = View_Mode::Busy_Coordinator;
            let line_count = app.command_output.output_text.lines().count();
            app.command_scroll = line_count.saturating_sub(1);
        }
        Some("file_progress") => {
            let file_name = v["name"].as_str().unwrap_or("?").to_string();
            let direction = v["dir"].as_str().unwrap_or("send");
            let peer = v["peer"].as_str().unwrap_or("?").to_string();
            let sent = v["sent"].as_u64().unwrap_or(0);
            let total = v["total"].as_u64().unwrap_or(0);
            app.job = Job_State::File_Transfer {
                direction: if direction == "send" {
                    Transfer_Direction::Send
                } else {
                    Transfer_Direction::Receive
                },
                file_name,
                peer,
                sent,
                total,
            };
        }
        _ => {}
    }
}

// ============================================================
// Output 事件处理
// ============================================================

fn handle_output(app: &mut App, v: &serde_json::Value) {
    match v["type"].as_str() {
        Some("cmd_result") => {
            let mut text = v["text"].as_str().unwrap_or("").to_string();
            // 解析 entries 中的 layer_bitmap，追加层范围信息
            if let Some(entries) = v["entries"].as_array() {
                for entry in entries {
                    if let (Some(name), Some(bitmap_str)) =
                        (entry["file_name"].as_str(), entry["layer_bitmap"].as_str())
                    {
                        if let Some(range) = bitmap_hex_to_layer_range(bitmap_str) {
                            text.push_str(&format!("\n  {} layers: {}", name, range));
                        }
                    }
                }
            }
            app.command_output.output_text = text;
            app.command_output.completed = v["completed"].as_bool().unwrap_or(true);
            app.command_scroll = 0;
        }
        Some("help") => {
            app.command_output.output_text =
                v["text"].as_str().unwrap_or("").to_string();
            app.command_output.completed = true;
        }
        _ => {}
    }
}

/// 将 64 字符 hex 位图解码为层范围字符串（如 "0-3,5,7-9"）
fn bitmap_hex_to_layer_range(hex: &str) -> Option<String> {
    if hex.len() < 64 {
        return None;
    }
    let mut layers: Vec<usize> = Vec::new();
    for i in 0..32 {
        let Ok(byte) = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) else {
            return None;
        };
        for bit in 0..8 {
            if byte & (1 << bit) != 0 {
                layers.push(i * 8 + bit);
            }
        }
    }
    if layers.is_empty() {
        return None;
    }
    let mut ranges = Vec::new();
    let mut start = layers[0];
    let mut end = layers[0];
    for &l in &layers[1..] {
        if l == end + 1 {
            end = l;
        } else {
            ranges.push(if start == end {
                format!("{}", start)
            } else {
                format!("{}-{}", start, end)
            });
            start = l;
            end = l;
        }
    }
    ranges.push(if start == end {
        format!("{}", start)
    } else {
        format!("{}-{}", start, end)
    });
    Some(ranges.join(","))
}

// ============================================================
// 键盘事件处理
// ============================================================

/// 处理键盘事件
fn Handle_Key_Event(
    app: &mut App,
    key_code: KeyCode,
    modifiers: KeyModifiers,
    user_cmd_tx: &mpsc::Sender<UserCommand>,
) {
    // Ctrl+C: 强制退出
    if modifiers.contains(KeyModifiers::CONTROL) && key_code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    match key_code {
        // Tab: 切换输入焦点
        KeyCode::Tab => {
            app.Toggle_Focus();
        }
        // 回车: 根据焦点分发
        KeyCode::Enter => match app.focus {
            InputFocus::Command => {
                let input = app.Take_Input();
                if !input.is_empty() {
                    Handle_Command_Input(app, &input, user_cmd_tx);
                }
            }
            InputFocus::Prompt => {
                Handle_Prompt_Submit(app);
            }
        },
        // Esc: 清空当前焦点输入
        KeyCode::Esc => match app.focus {
            InputFocus::Command => app.Clear_Input(),
            InputFocus::Prompt => app.Prompt_Clear(),
        },
        // Backspace: 删除当前焦点字符
        KeyCode::Backspace => match app.focus {
            InputFocus::Command => app.Delete_Char(),
            InputFocus::Prompt => app.Prompt_Delete_Char(),
        },
        // ↑: 根据修饰键决定滚动目标
        KeyCode::Up => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                app.Scroll_Command_Up();
            } else {
                app.Scroll_Up();
            }
        }
        // ↓: 根据修饰键决定滚动目标
        KeyCode::Down => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                app.Scroll_Command_Down();
            } else {
                app.Scroll_Down();
            }
        }
        // PageUp: 命令面板向上滚动（快速）
        KeyCode::PageUp => {
            for _ in 0..5 {
                app.Scroll_Command_Up();
            }
        }
        // PageDown: 命令面板向下滚动（快速）
        KeyCode::PageDown => {
            for _ in 0..5 {
                app.Scroll_Command_Down();
            }
        }
        // 普通字符输入: 路由到当前焦点
        KeyCode::Char(c) => match app.focus {
            InputFocus::Command => app.Input_Char(c),
            InputFocus::Prompt => app.Prompt_Input_Char(c),
        },
        _ => {}
    }
}

/// 处理 Prompt 提交
///
/// 将 prompt_buffer 的内容通过 IoFrontend.input_tx 发送给 ML Session。
/// 仅在有活跃 session 且未在生成中时有效。
fn Handle_Prompt_Submit(app: &mut App) {
    let prompt = app.Take_Prompt();
    if prompt.is_empty() {
        return;
    }
    let _ = app.prompt_tx.send(prompt);
}

// ============================================================
// 鼠标事件处理
// ============================================================

/// 处理鼠标事件
///
/// 根据鼠标位置判断光标所在面板，将滚轮事件路由到对应的滚动方法。
fn Handle_Mouse_Event(app: &mut App, kind: MouseEventKind, _column: u16, row: u16) {
    let in_log = row >= app.log_area.y && row < app.log_area.y + app.log_area.height;
    let in_command =
        row >= app.command_area.y && row < app.command_area.y + app.command_area.height;

    match kind {
        MouseEventKind::ScrollUp => {
            if in_command {
                app.Scroll_Command_Up();
            } else if in_log {
                app.Scroll_Up();
            }
        }
        MouseEventKind::ScrollDown => {
            if in_command {
                app.Scroll_Command_Down();
            } else if in_log {
                app.Scroll_Down();
            }
        }
        _ => {}
    }
}

// ============================================================
// 命令输入处理
// ============================================================

/// 处理用户输入的命令，通过 user_cmd_tx 发送 UserCommand 给 Orchestrator Core
fn Handle_Command_Input(app: &mut App, input: &str, user_cmd_tx: &mpsc::Sender<UserCommand>) {
    let trimmed = input.trim();

    // ---- 本地命令（不发给 Core） ----

    if trimmed == "quit" || trimmed == "exit" {
        app.should_quit = true;
        return;
    }

    if trimmed == "clear" {
        app.logs.clear();
        app.log_scroll = 0;
        return;
    }

    if trimmed == "help" {
        app.command_output = Command_Output::New();
        app.command_scroll = 0;
        app.command_output.output_text = "正在获取帮助信息...".to_string();
        if user_cmd_tx.blocking_send(UserCommand::Help).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // 每次新命令清空 Command 面板并重置滚动
    app.command_output = Command_Output::New();
    app.command_scroll = 0;

    // ---- session create <model> ----

    if trimmed.starts_with("session create ") {
        let model = trimmed.strip_prefix("session create ").unwrap_or("").trim();
        if model.is_empty() {
            app.command_output.output_text =
                "错误: 缺少 model 参数\n用法: session create <model_name>".to_string();
        } else {
            app.command_output.output_text = format!("正在创建 Session: {}...", model);
            let cmd = UserCommand::Session { model_id: model.to_string() };
            if user_cmd_tx.blocking_send(cmd).is_err() {
                app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            }
        }
        return;
    }

    // ---- chat <session_id> ----

    if trimmed.starts_with("chat ") {
        let sid_str = trimmed.strip_prefix("chat ").unwrap_or("").trim();
        let sid = sid_str.parse::<u64>();
        match sid {
            Ok(session_id) => {
                app.command_output.output_text =
                    format!("已连接到 Session {}，请切换到 Prompt 框输入", session_id);
                let cmd = UserCommand::Chat { session_id };
                if user_cmd_tx.blocking_send(cmd).is_err() {
                    app.Add_Log("[错误] Orchestrator 已关闭".to_string());
                }
            }
            Err(_) => {
                app.command_output.output_text =
                    format!("错误: '{}' 不是有效的 session_id\n用法: chat <session_id>", sid_str);
            }
        }
        return;
    }

    // ---- run <model_path> ----

    if trimmed.starts_with("run ") {
        let model_path_str = trimmed.strip_prefix("run ").unwrap_or("").trim();

        if model_path_str.is_empty() {
            app.command_output.output_text =
                "错误: 缺少 model_path 参数\n用法: run <model_path>".to_string();
            app.command_output.completed = true;
            return;
        }

        let cmd = UserCommand::Run {
            script: String::new(),
            model_path: model_path_str.to_string(),
        };

        app.Add_Log(format!("执行命令: run {}", model_path_str));
        app.command_output.output_text = "建立推理会话中...".to_string();

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- cancel <job_id> ----

    if trimmed.starts_with("cancel ") {
        let id_str = trimmed.strip_prefix("cancel ").unwrap_or("").trim();
        let job_id = match id_str.parse::<u64>() {
            Ok(id) => JobId(id),
            Err(_) => {
                app.command_output.output_text =
                    format!("错误: 无效的 Job ID '{}'\n用法: cancel <job_id>", id_str);
                app.command_output.completed = true;
                return;
            }
        };

        let cmd = UserCommand::Cancel { job_id };

        app.Add_Log(format!("执行命令: cancel {}", id_str));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- display-peer / dp ----

    if trimmed == "display-peer" || trimmed == "dp" {
        let cmd = UserCommand::DisplayPeer;

        app.Add_Log("执行命令: display-peer".to_string());

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- set-device <cpu/cuda> ----

    if trimmed.starts_with("set-device ") {
        let device_str = trimmed
            .strip_prefix("set-device ")
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if device_str != "cpu" && device_str != "cuda" {
            app.command_output.output_text =
                format!("不支持的设备: '{}'\n用法: set-device cpu/cuda", device_str);
            app.command_output.completed = true;
            return;
        }

        let cmd = UserCommand::SetDevice {
            device: device_str.clone(),
        };

        app.Add_Log(format!("执行命令: set-device {}", device_str));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- ls (列出存储文件) ----

    if trimmed == "ls" {
        let cmd = UserCommand::List;

        app.Add_Log("执行命令: ls".to_string());

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- flush (刷新存储索引) ----

    if trimmed == "flush" {
        let cmd = UserCommand::Flush;

        app.Add_Log("执行命令: flush".to_string());

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- reload (重新加载用户脚本) ----

    if trimmed == "reload" {
        let cmd = UserCommand::Reload;

        app.Add_Log("执行命令: reload".to_string());

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- set-name <name> ----

    if let Some(name) = trimmed.strip_prefix("set-name ") {
        let name = name.trim();
        if name.is_empty() {
            app.command_output.output_text =
                "错误: 名称不能为空\n用法: set-name <name>".to_string();
            app.command_output.completed = true;
            return;
        }
        let cmd = UserCommand::SetName { name: name.to_string() };
        app.Add_Log(format!("执行命令: set-name {}", name));
        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- send <file> <peer> ----

    if trimmed.starts_with("send ") {
        let args: Vec<&str> = trimmed
            .strip_prefix("send ")
            .unwrap_or("")
            .trim()
            .split_whitespace()
            .collect();

        if args.len() != 2 {
            app.command_output.output_text =
                "错误: 参数不正确\n用法: send <file> <peer_id>".to_string();
            app.command_output.completed = true;
            return;
        }

        let file_path = args[0].to_string();
        let peer_id = args[1].to_string();

        let cmd = UserCommand::Send {
            file_path: file_path.clone(),
            peer_id: peer_id.clone(),
        };

        app.Add_Log(format!("执行命令: send {} → {}", file_path, peer_id));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- distribute <model_path> <peer_id:start-end> ... ----

    if trimmed.starts_with("distribute ") {
        let args: Vec<&str> = trimmed
            .strip_prefix("distribute ")
            .unwrap_or("")
            .trim()
            .split_whitespace()
            .collect();

        if args.len() < 2 {
            app.command_output.output_text =
                "错误: 参数不足\n用法: distribute <model_path> <peer_id:start-end> ...".to_string();
            app.command_output.completed = true;
            return;
        }

        let model_path = args[0].to_string();
        let mut peers: Vec<(String, usize, usize)> = Vec::new();

        for arg in &args[1..] {
            match Parse_Peer_Assignment(arg) {
                Ok(assignment) => peers.push(assignment),
                Err(e) => {
                    app.command_output.output_text =
                        format!("错误: 解析 '{}' 失败: {}\n格式: peer_id:start-end", arg, e);
                    app.command_output.completed = true;
                    return;
                }
            }
        }

        let cmd = UserCommand::DistributeModel {
            model_path: model_path.clone(),
            peers,
        };

        app.Add_Log(format!("执行命令: distribute {}", model_path));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- pipeline <model_path> [strategy] ----

    if trimmed.starts_with("pipeline ") {
        let args: Vec<&str> = trimmed
            .strip_prefix("pipeline ")
            .unwrap_or("")
            .trim()
            .split_whitespace()
            .collect();
        let model_path_str = args.first().copied().unwrap_or("");

        if model_path_str.is_empty() {
            app.command_output.output_text =
                "错误: 缺少 model_path 参数\n用法: pipeline <model_path>".to_string();
            app.command_output.completed = true;
            return;
        }

        let mut params = std::collections::HashMap::new();
        params.insert("model_path".to_string(), model_path_str.to_string());
        let cmd = UserCommand::Execute {
            command: "pipeline".to_string(),
            params,
        };

        app.Add_Log(format!("执行命令: pipeline {}", model_path_str));
        app.command_output.output_text = "Pipeline 功能已移除，请使用 execute 命令".to_string();
        app.command_output.completed = true;

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    if trimmed.starts_with("profile ") {
        let model_id = trimmed[8..].trim();
        if model_id.is_empty() {
            app.command_output.output_text =
                "错误: 缺少 model_id 参数\n用法: profile <model_id>".to_string();
            app.command_output.completed = true;
            return;
        }

        let cmd = UserCommand::Profile {
            model_id: model_id.to_string(),
        };

        app.Add_Log(format!("执行命令: profile {}", model_id));
        app.command_output.output_text = "启动 Profile 中...".to_string();

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- exec <command> [key=value ...] ----

    if trimmed.starts_with("exec ") {
        let args: Vec<&str> = trimmed
            .strip_prefix("exec ")
            .unwrap_or("")
            .trim()
            .split_whitespace()
            .collect();
        let command = args.first().copied().unwrap_or("");
        if command.is_empty() {
            app.command_output.output_text =
                "错误: 缺少命令名\n用法: exec <command> [key=value ...]".to_string();
            app.command_output.completed = true;
            return;
        }

        let mut params = std::collections::HashMap::new();
        for arg in &args[1..] {
            if let Some((k, v)) = arg.split_once('=') {
                params.insert(k.to_string(), v.to_string());
            } else {
                app.command_output.output_text = format!(
                    "错误: 参数格式无效 '{}'\n用法: exec <command> [key=value ...]",
                    arg
                );
                app.command_output.completed = true;
                return;
            }
        }

        let cmd = UserCommand::Execute {
            command: command.to_string(),
            params,
        };

        app.Add_Log(format!("执行脚本: {}", command));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
        }
        return;
    }

    // ---- 未识别的命令 ----

    app.command_output.output_text = format!("未知命令: '{}'\n输入 help 查看可用命令", trimmed);
    app.command_output.completed = true;
}

/// 解析 peer 分配字符串，格式: `peer_id:start-end`
///
/// 例: `12D3KooW...abc:0-15` → `("12D3KooW...abc", 0, 15)`
fn Parse_Peer_Assignment(arg: &str) -> Result<(String, usize, usize), String> {
    let parts: Vec<&str> = arg.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return Err("缺少 ':' 分隔符".to_string());
    }
    let peer_id = parts[1].to_string();
    let range_str = parts[0];

    let range_parts: Vec<&str> = range_str.split('-').collect();
    if range_parts.len() != 2 {
        return Err("层范围格式错误，应为 start-end".to_string());
    }

    let start = range_parts[0]
        .parse::<usize>()
        .map_err(|e| format!("start 解析失败: {}", e))?;
    let end = range_parts[1]
        .parse::<usize>()
        .map_err(|e| format!("end 解析失败: {}", e))?;

    Ok((peer_id, start, end))
}

// ============================================================
// 总渲染函数
// ============================================================

/// 总渲染函数 — 计算布局并分发给各 panel 渲染（双输入框布局）
fn Render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // 5 行布局：Log+Network | Job | Command Output | Prompt 输入 | 命令输入
    let constraints = vec![
        Constraint::Min(6),    // Log + Network（自适应填满）
        Constraint::Length(3), // Job（固定 3 行）
        Constraint::Length(8), // Command Output（固定 8 行，推理输出）
        Constraint::Length(3), // Prompt 输入栏（固定 3 行）
        Constraint::Length(3), // 命令输入栏（固定 3 行，最底部）
    ];

    let rows = Layout::vertical(constraints).split(area);

    // Row 0: Log (70%) + Network (30%)
    let top =
        Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)]).split(rows[0]);

    // 记录面板区域，供鼠标滚轮事件命中检测
    app.log_area = top[0];
    app.command_area = rows[2];

    log_panel::Render(frame, top[0], app);
    network_panel::Render(frame, top[1], app);

    // Row 1: Job（始终显示）
    job_panel::Render(frame, rows[1], app);

    // Row 2: Command Output（始终显示）
    command_panel::Render(frame, rows[2], app);

    // Row 3: Prompt 输入栏
    Render_Prompt(frame, rows[3], app);

    // Row 4: 命令输入栏（最底部）
    Render_Input(frame, rows[4], app);
}

/// 渲染 Prompt 输入栏
///
/// 根据是否有活跃 session 显示不同状态：
/// - 有活跃 session：正常边框 + "Prompt>" 前缀
/// - 无活跃 session：灰色 + "无活跃会话" 提示
fn Render_Prompt(frame: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.focus == InputFocus::Prompt;

    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let input_text = Line::from(vec![
        Span::styled(
            "Prompt> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.prompt_buffer.as_str(),
            Style::default().fg(Color::White),
        ),
    ]);
    let paragraph = Paragraph::new(input_text).block(block);
    frame.render_widget(paragraph, area);

    // 设置光标位置（仅当 Prompt 获得焦点时）
    if is_focused {
        let cursor_x = area.x + 1 + "Prompt> ".len() as u16 + app.prompt_cursor as u16;
        let cursor_y = area.y + 1;
        if cursor_x < area.x + area.width - 1 {
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

/// 渲染命令输入栏（最底部）
fn Render_Input(frame: &mut Frame, area: Rect, app: &App) {
    let is_focused = app.focus == InputFocus::Command;

    let border_color = if is_focused {
        Color::Green
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let input_text = Line::from(vec![
        Span::styled(
            "pleiades> ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(app.input_buffer.as_str(), Style::default().fg(Color::White)),
    ]);

    let paragraph = Paragraph::new(input_text).block(block);
    frame.render_widget(paragraph, area);

    // 设置光标位置（仅当 Command 获得焦点时）
    if is_focused {
        let cursor_x = area.x + 1 + "pleiades> ".len() as u16 + app.cursor_position as u16;
        let cursor_y = area.y + 1;
        if cursor_x < area.x + area.width - 1 {
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}
