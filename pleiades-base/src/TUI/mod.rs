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
//! ## 布局（单输入框）
//! ```text
//! ┌─────────────────────────┬────────────────┐
//! │ Log (70%)               │ Network (30%)  │
//! ├─────────────────────────┴────────────────┤
//! │ Job (始终显示, 3行)                       │
//! ├──────────────────────────────────────────┤
//! │ Command Output (始终显示, 8行)            │
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
) {
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
        Bus_Event::StreamRaw { .. } => {
            // 二进制帧（ORION 协议）不由 TUI 消费，忽略
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
            let peer_name = v["peer_name"].as_str().unwrap_or("");
            app.Update_Peer(peer_id.to_string(), peer_name.to_string(), false, false);
            let label = if peer_name.is_empty() { peer_id } else { peer_name };
            app.Add_Log(format!("发现节点: {label}"));
        }
        Some("peer_left") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            app.Remove_Peer(peer_id);
            app.Add_Log(format!("节点离开: {peer_id}"));
        }
        Some("peer_info_updated") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            let peer_name = v["peer_name"].as_str().unwrap_or("");
            let is_local = v["is_local"].as_bool().unwrap_or(false);
            app.Update_Peer(peer_id.to_string(), peer_name.to_string(), is_local, true);
            let models = parse_models_json(v.get("models"));
            app.Update_Peer_Models(peer_id, models);
            let sessions = parse_sessions_json(v.get("sessions"));
            app.Update_Peer_Sessions(peer_id, sessions);
            if !peer_name.is_empty() {
                app.Add_Log(format!("节点信息: {peer_name}"));
            }
        }
        Some("peer_connected") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            let peer_name = v["peer_name"].as_str().unwrap_or("");
            app.Update_Peer(peer_id.to_string(), peer_name.to_string(), false, true);
            let label = if peer_name.is_empty() { peer_id } else { peer_name };
            app.Add_Log(format!("连接建立: {label}"));
        }
        Some("peer_disconnected") => {
            let peer_id = v["peer_id"].as_str().unwrap_or("?");
            let peer_name = v["peer_name"].as_str().unwrap_or("");
            app.Update_Peer(peer_id.to_string(), peer_name.to_string(), false, false);
            let label = if peer_name.is_empty() { peer_id } else { peer_name };
            app.Add_Log(format!("连接断开: {label}"));
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
        // 回车: 提交命令
        KeyCode::Enter => {
            let input = app.Take_Input();
            if !input.is_empty() {
                Handle_Command_Input(app, &input, user_cmd_tx);
            }
        }
        // Esc: 清空输入
        KeyCode::Esc => app.Clear_Input(),
        // Backspace: 删除字符
        KeyCode::Backspace => app.Delete_Char(),
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
        // 普通字符输入
        KeyCode::Char(c) => app.Input_Char(c),
        _ => {}
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
// ============================================================
// 命令解析（TUI + CLI 共用）
// ============================================================

/// 解析用户输入并构造 `UserCommand`，不依赖 TUI App 状态。
/// CLI 和 TUI 共用此函数。
///
/// 返回 `Ok(Some(cmd))` — 成功解析
/// 返回 `Ok(None)` — 本地命令（quit/clear 等），无需发送
/// 返回 `Err(msg)` — 解析失败，msg 为错误提示
pub fn parse_user_command(input: &str) -> Result<Option<UserCommand>, String> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Ok(None);
    }

    if trimmed == "quit" || trimmed == "exit" {
        return Ok(None); // 调用方自行处理（需构造 oneshot）
    }

    if trimmed == "clear" {
        return Ok(None); // CLI 忽略
    }

    if trimmed == "help" {
        return Ok(Some(UserCommand::Help));
    }

    // ---- session create <model> ----
    if let Some(model) = trimmed.strip_prefix("session create ") {
        let model = model.trim();
        if model.is_empty() {
            return Err("用法: session create <model_name>".to_string());
        }
        return Ok(Some(UserCommand::Session { model_id: model.to_string() }));
    }

    // ---- api <session_id> ----
    if trimmed.starts_with("api ") || trimmed.starts_with("API ") {
        let sid_str = trimmed
            .strip_prefix("api ").or_else(|| trimmed.strip_prefix("API ")).unwrap_or("").trim();
        match sid_str.parse::<u64>() {
            Ok(session_id) => return Ok(Some(UserCommand::Api { session_id })),
            Err(_) => return Err(format!("无效 session_id: '{}'\\n用法: api <session_id>", sid_str)),
        }
    }

    // ---- webui [port] ----
    if trimmed == "webui" || trimmed.starts_with("webui ") {
        let port_str = trimmed.strip_prefix("webui").unwrap_or("").trim();
        let port = if port_str.is_empty() {
            None
        } else {
            match port_str.parse::<u16>() {
                Ok(p) => Some(p),
                Err(_) => return Err(format!("无效端口: '{}'\n用法: webui [port]", port_str)),
            }
        };
        return Ok(Some(UserCommand::Webui { port }));
    }

    // ---- session inference [command] <session_id> <model_path> ----
    if let Some(rest) = trimmed.strip_prefix("session inference ") {
        let args: Vec<&str> = rest.split_whitespace().collect();
        if args.len() < 2 {
            return Err("用法: session inference [command] <session_id> <model_path>".to_string());
        }
        let (command, sid_str, model_path) = if args.len() >= 3 {
            (args[0].to_string(), args[1], args[2])
        } else {
            ("single_inf".to_string(), args[0], args[1])
        };
        match sid_str.parse::<u64>() {
            Ok(session_id) => return Ok(Some(UserCommand::SessionInference {
                command, session_id, model_path: model_path.to_string(),
            })),
            Err(_) => return Err(format!("无效 session_id: '{}'", sid_str)),
        }
    }

    // ---- run <model_path> ----
    if let Some(p) = trimmed.strip_prefix("run ") {
        let p = p.trim();
        if p.is_empty() { return Err("用法: run <model_path>".to_string()); }
        return Ok(Some(UserCommand::Run { script: String::new(), model_path: p.to_string() }));
    }

    // ---- cancel <job_id> ----
    if let Some(id_str) = trimmed.strip_prefix("cancel ") {
        match id_str.trim().parse::<u64>() {
            Ok(id) => return Ok(Some(UserCommand::Cancel { job_id: JobId(id) })),
            Err(_) => return Err(format!("无效 Job ID: '{}'", id_str.trim())),
        }
    }

    // ---- display-peer / dp ----
    if trimmed == "display-peer" || trimmed == "dp" {
        return Ok(Some(UserCommand::DisplayPeer));
    }

    // ---- set-device <cpu|cuda|cuda:N> ----
    if let Some(d) = trimmed.strip_prefix("set-device ") {
        let d = d.trim().to_lowercase();
        if d != "cpu" && d != "cuda" && !d.starts_with("cuda:") {
            return Err(format!("不支持的设备: '{}'\\n用法: set-device cpu|cuda|cuda:N", d));
        }
        return Ok(Some(UserCommand::SetDevice { device: d }));
    }

    // ---- ls ----
    if trimmed == "ls" {
        return Ok(Some(UserCommand::List));
    }

    // ---- flush ----
    if trimmed == "flush" {
        return Ok(Some(UserCommand::Flush));
    }

    // ---- reload ----
    if trimmed == "reload" {
        return Ok(Some(UserCommand::Reload));
    }

    // ---- set-name <name> ----
    if let Some(name) = trimmed.strip_prefix("set-name ") {
        let name = name.trim();
        if name.is_empty() { return Err("名称不能为空".to_string()); }
        return Ok(Some(UserCommand::SetName { name: name.to_string() }));
    }

    // ---- send <file> <peer> ----
    if let Some(rest) = trimmed.strip_prefix("send ") {
        let args: Vec<&str> = rest.split_whitespace().collect();
        if args.len() != 2 {
            return Err("用法: send <file> <peer_id>".to_string());
        }
        return Ok(Some(UserCommand::Send {
            file_path: args[0].to_string(), peer_id: args[1].to_string(),
        }));
    }

    // ---- distribute <model_path> <peer_id:start-end> ... ----
    if let Some(rest) = trimmed.strip_prefix("distribute ") {
        let args: Vec<&str> = rest.split_whitespace().collect();
        if args.len() < 2 {
            return Err("用法: distribute <model_path> <peer_id:start-end> ...".to_string());
        }
        let model_path = args[0].to_string();
        let mut peers: Vec<(String, usize, usize)> = Vec::new();
        for arg in &args[1..] {
            let assignment = Parse_Peer_Assignment(arg)
                .map_err(|e| format!("解析 '{}' 失败: {}", arg, e))?;
            peers.push(assignment);
        }
        return Ok(Some(UserCommand::DistributeModel { model_path, peers }));
    }

    // ---- pipeline <model_path> ----
    if let Some(rest) = trimmed.strip_prefix("pipeline ") {
        let model = rest.split_whitespace().next().unwrap_or("");
        if model.is_empty() {
            return Err("用法: pipeline <model_path>".to_string());
        }
        let mut params = std::collections::HashMap::new();
        params.insert("model_path".to_string(), model.to_string());
        return Ok(Some(UserCommand::Execute { command: "pipeline".to_string(), params }));
    }

    // ---- profile <model_id> ----
    if let Some(model_id) = trimmed.strip_prefix("profile ") {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err("用法: profile <model_id>".to_string());
        }
        return Ok(Some(UserCommand::Profile { model_id: model_id.to_string() }));
    }

    // ---- dial <multiaddr> ----
    if let Some(addr) = trimmed.strip_prefix("dial ") {
        let addr = addr.trim();
        if addr.is_empty() { return Err("用法: dial <multiaddr>".to_string()); }
        return Ok(Some(UserCommand::Dial { addr: addr.to_string() }));
    }

    // ---- exec <command> [key=value ...] ----
    if let Some(rest) = trimmed.strip_prefix("exec ") {
        let args: Vec<&str> = rest.split_whitespace().collect();
        let command = args.first().copied().unwrap_or("");
        if command.is_empty() {
            return Err("用法: exec <command> [key=value ...]".to_string());
        }
        let mut params = std::collections::HashMap::new();
        for arg in &args[1..] {
            let (k, v) = arg.split_once('=')
                .ok_or_else(|| format!("参数格式无效: '{}'", arg))?;
            params.insert(k.to_string(), v.to_string());
        }
        return Ok(Some(UserCommand::Execute { command: command.to_string(), params }));
    }

    // ---- rexec <peer> <command> [key=value ...] ----
    if let Some(rest) = trimmed.strip_prefix("rexec ") {
        let args: Vec<&str> = rest.split_whitespace().collect();
        if args.len() < 2 {
            return Err("用法: rexec <peer> <command> [key=value ...]".to_string());
        }
        let peer = args[0].to_string();
        let command = args[1].to_string();
        let mut params = std::collections::HashMap::new();
        for arg in &args[2..] {
            let (k, v) = arg.split_once('=')
                .ok_or_else(|| format!("参数格式无效: '{}'", arg))?;
            params.insert(k.to_string(), v.to_string());
        }
        return Ok(Some(UserCommand::ExecRemote { peer, command, params }));
    }

    Err(format!("未知命令: '{}'\\n输入 help 查看可用命令", trimmed))
}


// ============================================================
// TUI 命令处理（含 App 状态更新）
// ============================================================

fn Handle_Command_Input(app: &mut App, input: &str, user_cmd_tx: &mpsc::Sender<UserCommand>) {
    let trimmed = input.trim();

    // 本地命令：quit / clear
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
    app.command_scroll = 0;

    if trimmed == "help" {
        app.command_output.output_text = "正在获取帮助信息...".to_string();
    }

    match parse_user_command(trimmed) {
        Ok(Some(cmd)) => {
            // 日志
            let label = match &cmd {
                UserCommand::Execute { command, .. } => format!("执行脚本: {}", command),
                UserCommand::ExecRemote { peer, command, .. } => format!("远程执行: rexec {} {}", peer, command),
                UserCommand::Session { model_id } => format!("创建 Session: {}", model_id),
                UserCommand::Api { session_id } => format!("启动 API Server -> Session {}", session_id),
                UserCommand::SessionInference { command, session_id, .. } =>
                    format!("ML Thread ({}) -> Session {}", command, session_id),
                UserCommand::Run { model_path, .. } => format!("执行命令: run {}", model_path),
                UserCommand::Cancel { job_id } => format!("执行命令: cancel {}", job_id.0),
                UserCommand::DisplayPeer => "执行命令: display-peer".to_string(),
                UserCommand::SetDevice { device } => format!("执行命令: set-device {}", device),
                UserCommand::List => "执行命令: ls".to_string(),
                UserCommand::Flush => "执行命令: flush".to_string(),
                UserCommand::Reload => "执行命令: reload".to_string(),
                UserCommand::SetName { name } => format!("执行命令: set-name {}", name),
                UserCommand::Send { file_path, peer_id } =>
                    format!("执行命令: send {} → {}", file_path, peer_id),
                UserCommand::DistributeModel { model_path, .. } =>
                    format!("执行命令: distribute {}", model_path),
                UserCommand::Profile { model_id } => format!("执行命令: profile {}", model_id),
                UserCommand::Dial { addr } => format!("执行命令: dial {}", addr),
                _ => String::new(),
            };
            if !label.is_empty() {
                app.Add_Log(label);
            }

            if user_cmd_tx.blocking_send(cmd).is_err() {
                app.Add_Log("[错误] Orchestrator 已关闭".to_string());
                app.should_quit = true;
            }
        }
        Ok(None) => {} // 空输入
        Err(msg) => {
            app.command_output.output_text = msg;
            app.command_output.completed = true;
        }
    }
}

/// 从 EventBus JSON 中解析模型列表
fn parse_models_json(models_val: Option<&serde_json::Value>) -> Vec<app::ModelDisplay> {
    let Some(val) = models_val else { return Vec::new() };
    let arr = match val {
        serde_json::Value::Array(a) => a,
        serde_json::Value::String(s) => {
            match serde_json::from_str::<Vec<serde_json::Value>>(s) {
                Ok(a) => return a.iter().filter_map(|v| parse_one_model(v)).collect(),
                Err(_) => return Vec::new(),
            }
        }
        _ => return Vec::new(),
    };
    arr.iter().filter_map(|v| parse_one_model(v)).collect()
}

fn parse_one_model(v: &serde_json::Value) -> Option<app::ModelDisplay> {
    let file_name = v.get("file_name")?.as_str()?.to_string();
    let layer_range = v.get("layer_range")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();
    Some(app::ModelDisplay { file_name, layer_range })
}

/// 从 EventBus JSON 中解析 session 列表
fn parse_sessions_json(sessions_val: Option<&serde_json::Value>) -> Vec<app::SessionDisplay> {
    let Some(val) = sessions_val else { return Vec::new() };
    let arr = match val {
        serde_json::Value::Array(a) => a,
        _ => return Vec::new(),
    };
    arr.iter().filter_map(|v| {
        Some(app::SessionDisplay {
            session_id: v.get("session_id")?.as_u64()?,
            model_id: v.get("model_id")?.as_str()?.to_string(),
            slots: format!("{}/{}",
                v.get("occupied_slots")?.as_u64()?,
                v.get("total_slots")?.as_u64()?),
        })
    }).collect()
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

/// 总渲染函数 — 计算布局并分发给各 panel 渲染（单输入框布局）
fn Render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // 4 行布局：Log+Network | Job | Command Output | 命令输入
    let constraints = vec![
        Constraint::Min(6),    // Log + Network（自适应填满）
        Constraint::Length(3), // Job（固定 3 行）
        Constraint::Length(8), // Command Output（固定 8 行，推理输出）
        Constraint::Length(3), // 命令输入栏（固定 3 行，最底部）
    ];

    let rows = Layout::vertical(constraints).split(area);

    // Row 0: Log (60%) + Network (40%)
    let top =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(rows[0]);

    // 记录面板区域，供鼠标滚轮事件命中检测
    app.log_area = top[0];
    app.command_area = rows[2];

    log_panel::Render(frame, top[0], app);
    network_panel::Render(frame, top[1], app);

    // Row 1: Job（始终显示）
    job_panel::Render(frame, rows[1], app);

    // Row 2: Command Output（始终显示）
    command_panel::Render(frame, rows[2], app);

    // Row 3: 命令输入栏（最底部）
    Render_Input(frame, rows[3], app);
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
