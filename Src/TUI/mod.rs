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
pub mod log_panel;
pub mod network_panel;
pub mod job_panel;
pub mod command_panel;

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind, EnableMouseCapture, DisableMouseCapture};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use tokio::sync::{broadcast, mpsc, oneshot};

use app::{App, View_Mode, Job_State, Command_Output, Transfer_Direction, InputFocus};

use crate::event_bus::Bus_Event;
use crate::llm_io::{LLM_IO_Broker, LLM_IO_Capability};
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
/// - `io_broker`: IO Broker 共享引用，用于 Take_Frontend 获取推理会话端点
pub fn TUI_Loop(mut event_rx: broadcast::Receiver<Bus_Event>, user_cmd_tx: mpsc::Sender<UserCommand>, io_broker: Arc<LLM_IO_Broker>) {
    // 1. 初始化终端
    let mut terminal = ratatui::init();

    // 启用鼠标捕获（滚轮等事件）
    crossterm::execute!(std::io::stdout(), EnableMouseCapture).ok();

    let mut app = App::New();
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

        // 2b-extra: 轮询推理输出（从 IoFrontend.output_rx 读取流式 token）
        if let Some(ref mut frontend) = app.active_frontend {
            let mut disconnected = false;
            loop {
                match frontend.output_rx.try_recv() {
                    Ok(token) => {
                        app.command_output.output_text.push_str(&token);
                        app.command_output.token_count += 1;
                        // 自动滚动到底部
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

        // 2c. 处理输入事件（50ms 超时，约 20fps）
        if crossterm::event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    Handle_Key_Event(&mut app, key.code, key.modifiers, &user_cmd_tx, &io_broker);
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
        Bus_Event::Log { message } => {
            app.Add_Log(message);
        }
        Bus_Event::Error { message } => {
            app.Add_Log(format!("[错误] {}", message));
        }
        Bus_Event::Peer_Discovered { peer_id } => {
            app.Update_Peer(peer_id.clone(), false);
            app.Add_Log(format!("发现节点: {}", peer_id));
        }
        Bus_Event::Peer_Left { peer_id } => {
            app.Remove_Peer(&peer_id);
            app.Add_Log(format!("节点离开: {}", peer_id));
        }
        Bus_Event::Connection_Established { peer_id } => {
            app.Update_Peer(peer_id.clone(), true);
            app.Add_Log(format!("连接建立: {}", peer_id));
        }
        Bus_Event::Connection_Closed { peer_id } => {
            app.Update_Peer(peer_id.clone(), false);
            app.Add_Log(format!("连接断开: {}", peer_id));
        }
        Bus_Event::File_Progress {
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
        Bus_Event::Inference_Started {
            job_id: _,
            model_name,
            device_count,
            layer_range,
        } => {
            app.job = Job_State::Inference {
                model_name: model_name.clone(),
                device_count,
                layer_range: layer_range.clone(),
                phase: "初始化".to_string(),
            };
            app.view_mode = View_Mode::Busy;
            app.Add_Log(format!("推理开始: {} [{}]", model_name, layer_range));
        }
        Bus_Event::Inference_Token { job_id: _, token } => {
            app.command_output.output_text.push_str(&token);
            app.command_output.token_count += 1;
            app.view_mode = View_Mode::Busy_Coordinator;
            // 自动滚动到底部，跟随推理输出
            let line_count = app.command_output.output_text.lines().count();
            app.command_scroll = line_count.saturating_sub(1);
        }
        Bus_Event::Inference_Completed {
            job_id: _,
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
        Bus_Event::Job_Created { job_id, kind, model_name } => {
            let msg = if model_name.is_empty() {
                format!("Job #{} 已创建 [{}]", job_id, kind)
            } else {
                format!("Job #{} 已创建 [{}] 模型: {}", job_id, kind, model_name)
            };
            app.Add_Log(msg);
        }
        Bus_Event::Job_State_Changed { job_id, phase } => {
            app.Add_Log(format!("Job #{} 阶段: {}", job_id, phase));
            // 更新 Inference 阶段（如果当前 Job 是推理类型）
            if let Job_State::Inference { phase: ref mut current_phase, .. } = app.job {
                *current_phase = phase;
            }
        }
        Bus_Event::Job_Completed { job_id, result } => {
            app.Add_Log(format!("Job #{} 完成: {}", job_id, result));
            // 仅当当前不在推理输出模式时才切回 Idle
            // （推理完成已由 Inference_Completed 处理）
            if app.view_mode != View_Mode::Busy_Coordinator {
                app.job = Job_State::Idle;
                app.view_mode = View_Mode::Idle;
            }
        }
        Bus_Event::Device_Changed { device } => {
            app.Add_Log(format!("设备已切换: {}", device.to_uppercase()));
            app.device = device;
        }
    }
}

// ============================================================
// 键盘事件处理
// ============================================================

/// 处理键盘事件
fn Handle_Key_Event(app: &mut App, key_code: KeyCode, modifiers: KeyModifiers, user_cmd_tx: &mpsc::Sender<UserCommand>, io_broker: &Arc<LLM_IO_Broker>) {
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
        KeyCode::Enter => {
            match app.focus {
                InputFocus::Command => {
                    let input = app.Take_Input();
                    if !input.is_empty() {
                        Handle_Command_Input(app, &input, user_cmd_tx, io_broker);
                    }
                }
                InputFocus::Prompt => {
                    Handle_Prompt_Submit(app);
                }
            }
        }
        // Esc: 清空当前焦点输入
        KeyCode::Esc => {
            match app.focus {
                InputFocus::Command => app.Clear_Input(),
                InputFocus::Prompt => app.Prompt_Clear(),
            }
        }
        // Backspace: 删除当前焦点字符
        KeyCode::Backspace => {
            match app.focus {
                InputFocus::Command => app.Delete_Char(),
                InputFocus::Prompt => app.Prompt_Delete_Char(),
            }
        }
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
        KeyCode::Char(c) => {
            match app.focus {
                InputFocus::Command => app.Input_Char(c),
                InputFocus::Prompt => app.Prompt_Input_Char(c),
            }
        }
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

    if let Some(ref frontend) = app.active_frontend {
        // 清空命令输出面板，准备显示新的推理输出
        app.command_output = Command_Output::New();
        app.command_scroll = 0;
        app.view_mode = View_Mode::Busy_Coordinator;

        match frontend.input_tx.blocking_send(prompt.clone()) {
            Ok(()) => {
                app.Add_Log(format!("已发送 Prompt: {}...", if prompt.len() > 20 { &prompt[..20] } else { &prompt }));
            }
            Err(_) => {
                app.Add_Log("[错误] Prompt 发送失败（会话可能已关闭）".to_string());
                app.active_frontend = None;
                app.active_job_id = None;
            }
        }
    } else {
        app.Add_Log("[提示] 无活跃推理会话，请先执行 run <model_path>".to_string());
    }
}

// ============================================================
// 鼠标事件处理
// ============================================================

/// 处理鼠标事件
///
/// 根据鼠标位置判断光标所在面板，将滚轮事件路由到对应的滚动方法。
fn Handle_Mouse_Event(app: &mut App, kind: MouseEventKind, _column: u16, row: u16) {
    let in_log = row >= app.log_area.y && row < app.log_area.y + app.log_area.height;
    let in_command = row >= app.command_area.y && row < app.command_area.y + app.command_area.height;

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
fn Handle_Command_Input(app: &mut App, input: &str, user_cmd_tx: &mpsc::Sender<UserCommand>, io_broker: &Arc<LLM_IO_Broker>) {
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
        app.command_output.output_text = [
            "可用命令:",
            "  run <model_path>         - 启动本地推理",
            "  pipeline <model_path>    - 启动分布式流水线推理",
            "  send <file> <peer>       - 向节点发送文件",
            "  cancel <job_id>          - 取消指定作业",
            "  display-peer / dp        - 查看节点列表",
            "  set-device cpu/cuda      - 切换计算设备",
            "  ls                       - 列出存储文件",
            "  distribute <model> <peer:start-end> ... - 分发模型分片",
            "  clear                    - 清空日志",
            "  quit / exit              - 退出",
            "  help                     - 显示此帮助",
        ].join("\n");
        app.command_output.completed = true;
        return;
    }

    // 每次新命令清空 Command 面板并重置滚动
    app.command_output = Command_Output::New();
    app.command_scroll = 0;

    // ---- run <model_path> ----

    if trimmed.starts_with("run ") {
        let model_path_str = trimmed.strip_prefix("run ").unwrap_or("").trim();

        if model_path_str.is_empty() {
            app.command_output.output_text = "错误: 缺少 model_path 参数\n用法: run <model_path>".to_string();
            app.command_output.completed = true;
            return;
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::Run {
            model_path: model_path_str.to_string(),
            reply: reply_tx,
        };

        app.Add_Log(format!("执行命令: run {}", model_path_str));
        app.command_output.output_text = "建立推理会话中...".to_string();

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        // 同步等待 Core 回复
        match reply_rx.blocking_recv() {
            Ok(Ok(job_id)) => {
                app.Add_Log(format!("推理 Job #{} 已创建", job_id.0));

                // 从 IO Broker 获取前端端点（会合点设计）
                let rt = tokio::runtime::Handle::current();
                match rt.block_on(io_broker.Take_Frontend(job_id)) {
                    Ok(frontend) => {
                        app.active_frontend = Some(frontend);
                        app.active_job_id = Some(job_id);
                        app.command_output.output_text = "会话已建立，请在 Prompt 框输入内容".to_string();
                        app.Add_Log(format!("IoFrontend 获取成功, Job #{}", job_id.0));
                    }
                    Err(e) => {
                        app.Add_Log(format!("[错误] 获取前端通道失败: {}", e));
                        app.command_output.output_text = format!("会话创建成功但通道获取失败: {}", e);
                        app.command_output.completed = true;
                    }
                }
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
        }
        return;
    }

    // ---- cancel <job_id> ----

    if trimmed.starts_with("cancel ") {
        let id_str = trimmed.strip_prefix("cancel ").unwrap_or("").trim();
        let job_id = match id_str.parse::<u64>() {
            Ok(id) => JobId(id),
            Err(_) => {
                app.command_output.output_text = format!("错误: 无效的 Job ID '{}'\n用法: cancel <job_id>", id_str);
                app.command_output.completed = true;
                return;
            }
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::Cancel {
            job_id,
            reply: reply_tx,
        };

        app.Add_Log(format!("执行命令: cancel {}", id_str));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        match reply_rx.blocking_recv() {
            Ok(Ok(())) => {
                app.command_output.output_text = format!("Job #{} 取消信号已发送", job_id.0);
                app.command_output.completed = true;
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
        }
        return;
    }

    // ---- display-peer / dp ----

    if trimmed == "display-peer" || trimmed == "dp" {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::DisplayPeer {
            reply: reply_tx,
        };

        app.Add_Log("执行命令: display-peer".to_string());

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        match reply_rx.blocking_recv() {
            Ok(Ok(peers)) => {
                if peers.is_empty() {
                    app.command_output.output_text = "当前无已知节点".to_string();
                } else {
                    app.command_output.output_text = peers.join("\n");
                }
                app.command_output.completed = true;
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
        }
        return;
    }

    // ---- set-device <cpu/cuda> ----

    if trimmed.starts_with("set-device ") {
        let device_str = trimmed.strip_prefix("set-device ").unwrap_or("").trim().to_lowercase();
        if device_str != "cpu" && device_str != "cuda" {
            app.command_output.output_text = format!("不支持的设备: '{}'\n用法: set-device cpu/cuda", device_str);
            app.command_output.completed = true;
            return;
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::SetDevice {
            device: device_str.clone(),
            reply: reply_tx,
        };

        app.Add_Log(format!("执行命令: set-device {}", device_str));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        match reply_rx.blocking_recv() {
            Ok(Ok(())) => {
                app.command_output.output_text = format!("设备已切换为: {}", device_str.to_uppercase());
                app.command_output.completed = true;
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
        }
        return;
    }

    // ---- ls (列出存储文件) ----

    if trimmed == "ls" {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::List {
            reply: reply_tx,
        };

        app.Add_Log("执行命令: ls".to_string());

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        match reply_rx.blocking_recv() {
            Ok(Ok(files)) => {
                if files.is_empty() {
                    app.command_output.output_text = "存储为空（无文件）".to_string();
                } else {
                    let mut output = format!("共 {} 个文件:\n", files.len());
                    for file_id in &files {
                        output.push_str(&format!("  {}\n", file_id));
                    }
                    app.command_output.output_text = output;
                }
                app.command_output.completed = true;
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
        }
        return;
    }

    // ---- send <file> <peer> ----

    if trimmed.starts_with("send ") {
        let args: Vec<&str> = trimmed.strip_prefix("send ").unwrap_or("").trim().split_whitespace().collect();

        if args.len() != 2 {
            app.command_output.output_text = "错误: 参数不正确\n用法: send <file> <peer_id>".to_string();
            app.command_output.completed = true;
            return;
        }

        let file_path = args[0].to_string();
        let peer_id = args[1].to_string();

        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::Send {
            file_path: file_path.clone(),
            peer_id: peer_id.clone(),
            reply: reply_tx,
        };

        app.Add_Log(format!("执行命令: send {} → {}", file_path, peer_id));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        match reply_rx.blocking_recv() {
            Ok(Ok(job_id)) => {
                app.command_output.output_text = format!("发送 Job #{} 已创建 ({} → {})", job_id.0, file_path, peer_id);
                app.command_output.completed = true;
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
        }
        return;
    }

    // ---- distribute <model_path> <peer_id:start-end> ... ----

    if trimmed.starts_with("distribute ") {
        let args: Vec<&str> = trimmed.strip_prefix("distribute ").unwrap_or("").trim().split_whitespace().collect();

        if args.len() < 2 {
            app.command_output.output_text = "错误: 参数不足\n用法: distribute <model_path> <peer_id:start-end> ...".to_string();
            app.command_output.completed = true;
            return;
        }

        let model_path = args[0].to_string();
        let mut peers: Vec<(String, usize, usize)> = Vec::new();

        for arg in &args[1..] {
            match Parse_Peer_Assignment(arg) {
                Ok(assignment) => peers.push(assignment),
                Err(e) => {
                    app.command_output.output_text = format!("错误: 解析 '{}' 失败: {}\n格式: peer_id:start-end", arg, e);
                    app.command_output.completed = true;
                    return;
                }
            }
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::DistributeModel {
            model_path: model_path.clone(),
            peers,
            reply: reply_tx,
        };

        app.Add_Log(format!("执行命令: distribute {}", model_path));

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        match reply_rx.blocking_recv() {
            Ok(Ok(job_id)) => {
                app.command_output.output_text = format!("分发 Job #{} 已创建", job_id.0);
                app.command_output.completed = true;
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
        }
        return;
    }

    // ---- pipeline <model_path> ----

    if trimmed.starts_with("pipeline ") {
        let model_path_str = trimmed.strip_prefix("pipeline ").unwrap_or("").trim();

        if model_path_str.is_empty() {
            app.command_output.output_text = "错误: 缺少 model_path 参数\n用法: pipeline <model_path>".to_string();
            app.command_output.completed = true;
            return;
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = UserCommand::Pipeline {
            model_path: model_path_str.to_string(),
            reply: reply_tx,
        };

        app.Add_Log(format!("执行命令: pipeline {}", model_path_str));
        app.command_output.output_text = "启动分布式流水线推理中...".to_string();

        if user_cmd_tx.blocking_send(cmd).is_err() {
            app.Add_Log("[错误] Orchestrator 已关闭".to_string());
            app.should_quit = true;
            return;
        }

        match reply_rx.blocking_recv() {
            Ok(Ok(job_id)) => {
                app.Add_Log(format!("Pipeline Job #{} 已创建", job_id.0));

                // 从 IO Broker 获取前端端点（会合点设计）
                let rt = tokio::runtime::Handle::current();
                match rt.block_on(io_broker.Take_Frontend(job_id)) {
                    Ok(frontend) => {
                        app.active_frontend = Some(frontend);
                        app.active_job_id = Some(job_id);
                        app.command_output.output_text = "流水线已建立，请在 Prompt 框输入内容".to_string();
                        app.Add_Log(format!("IoFrontend 获取成功, Pipeline Job #{}", job_id.0));
                    }
                    Err(e) => {
                        app.Add_Log(format!("[错误] 获取前端通道失败: {}", e));
                        app.command_output.output_text = format!("流水线建立成功但通道获取失败: {}", e);
                        app.command_output.completed = true;
                    }
                }
            }
            Ok(Err(e)) => {
                app.command_output.output_text = format!("错误: {}", e);
                app.command_output.completed = true;
            }
            Err(_) => {
                app.command_output.output_text = "Orchestrator 未响应".to_string();
                app.command_output.completed = true;
            }
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

    let start = range_parts[0].parse::<usize>()
        .map_err(|e| format!("start 解析失败: {}", e))?;
    let end = range_parts[1].parse::<usize>()
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
        Constraint::Min(6),       // Log + Network（自适应填满）
        Constraint::Length(3),    // Job（固定 3 行）
        Constraint::Length(8),    // Command Output（固定 8 行，推理输出）
        Constraint::Length(3),    // Prompt 输入栏（固定 3 行）
        Constraint::Length(3),    // 命令输入栏（固定 3 行，最底部）
    ];

    let rows = Layout::vertical(constraints).split(area);

    // Row 0: Log (70%) + Network (30%)
    let top = Layout::horizontal([
        Constraint::Percentage(70),
        Constraint::Percentage(30),
    ])
    .split(rows[0]);

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
    let has_session = app.Has_Active_Session();

    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if has_session {
        let input_text = Line::from(vec![
            Span::styled("Prompt> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(app.prompt_buffer.as_str(), Style::default().fg(Color::White)),
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
    } else {
        let hint_text = Line::from(vec![
            Span::styled("Prompt> ", Style::default().fg(Color::DarkGray)),
            Span::styled("无活跃会话 (先执行 run <model>)", Style::default().fg(Color::DarkGray)),
        ]);
        let paragraph = Paragraph::new(hint_text).block(block);
        frame.render_widget(paragraph, area);
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
        Span::styled("pleiades> ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
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
