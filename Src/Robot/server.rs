//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! 机器人 WebSocket 遥控服务器
//!
//! 接收 JSON 控制指令，通过 cmd_tx 发给 Robot。
//! 遥测数据推送到 EventBus。

use std::sync::Arc;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;

use crate::event_bus::{Bus_Event, EventBus};
use crate::robot::core::command::Command;
use crate::robot::state::RobotState;

/// 启动 WS 遥控服务（作为 tokio task，共享主 runtime）
pub fn spawn_robot_ws_server(
    port: u16,
    event_bus: Arc<EventBus>,
    cmd_tx: mpsc::Sender<Command>,
    state: Arc<tokio::sync::RwLock<RobotState>>,
) {
    tokio::spawn(async move {
        run_server(port, event_bus, cmd_tx, state).await;
    });
}

async fn run_server(
    port: u16,
    event_bus: Arc<EventBus>,
    cmd_tx: mpsc::Sender<Command>,
    state: Arc<tokio::sync::RwLock<RobotState>>,
) {
    let addr = format!("0.0.0.0:{port}");
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("[Robot WS] 绑定 {addr} 失败: {e}");
            return;
        }
    };
    tracing::info!("[Robot WS] 遥控服务器已启动: ws://{addr}");

    let telemetry_bus = event_bus.clone();
    let telemetry_state = state.clone();
    tokio::spawn(async move { telemetry_loop(telemetry_bus, telemetry_state).await });

    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                tracing::info!("[Robot WS] 新连接: {peer_addr}");
                let ws = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        tracing::warn!("[Robot WS] 握手失败: {peer_addr} - {e}");
                        continue;
                    }
                };
                let tx = cmd_tx.clone();
                tokio::spawn(async move {
                    handle_connection(ws, tx, peer_addr.to_string()).await;
                });
            }
            Err(e) => tracing::error!("[Robot WS] accept 错误: {e}"),
        }
    }
}

async fn handle_connection(
    mut ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    cmd_tx: mpsc::Sender<Command>,
    peer: String,
) {
    let welcome = serde_json::json!({"type":"welcome","message":"Robot WS connected"});
    let _ = ws.send(tokio_tungstenite::tungstenite::Message::Text(welcome.to_string())).await;

    while let Some(msg) = ws.next().await {
        match msg {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                if let Some(cmd) = parse_command(&text) {
                    let _ = cmd_tx.send(cmd).await;
                }
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("[Robot WS] {peer} 读取错误: {e}");
                break;
            }
        }
    }

    let _ = cmd_tx.send(Command::Stop).await;
    tracing::info!("[Robot WS] {peer} 已断开，自动停车");
}

fn parse_command(text: &str) -> Option<Command> {
    let v: Value = serde_json::from_str(text).ok()?;
    let cmd = v["cmd"].as_str()?;
    let speed = v["speed"].as_i64().unwrap_or(50) as i16;
    match cmd {
        "forward"    => Some(Command::Forward(speed)),
        "backward"   => Some(Command::Backward(speed)),
        "spin_left" | "left"   => Some(Command::SpinLeft(speed)),
        "spin_right" | "right" => Some(Command::SpinRight(speed)),
        "stop"       => Some(Command::Stop),
        "beep"       => Some(Command::Beep(v["ms"].as_u64().unwrap_or(200) as u16)),
        _ => { tracing::warn!("[Robot WS] 未知命令: {cmd}"); None }
    }
}

async fn telemetry_loop(
    event_bus: Arc<EventBus>,
    state: Arc<tokio::sync::RwLock<RobotState>>,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let s = state.read().await;
        let payload = serde_json::json!({
            "type": "robot_telemetry",
            "vx": s.vx, "vy": s.vy, "vz": s.vz,
            "battery": s.battery,
            "roll": s.attitude.roll,
            "pitch": s.attitude.pitch,
            "yaw": s.attitude.yaw,
            "encoders": s.encoders,
        });
        event_bus.Publish(Bus_Event::State { payload: payload.to_string() });
    }
}
