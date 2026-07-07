//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! 机器人 WebSocket 遥控服务器
//!
//! 独立线程运行，监听 TCP 端口，接收 JSON 控制指令，
//! 直接调用 STM32Device API。

use std::sync::Arc;
use std::thread;

use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

use crate::event_bus::{Bus_Event, EventBus};
use crate::robot::state::RobotState;
use crate::robot::get_stm32;

pub fn spawn_robot_ws_server(port: u16, event_bus: Arc<EventBus>) {
    thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all().build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("[Robot WS] 创建 runtime 失败: {e}");
                return;
            }
        };
        rt.block_on(async { run_server(port, event_bus).await });
    });
}

async fn run_server(port: u16, event_bus: Arc<EventBus>) {
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
    tokio::spawn(async move { telemetry_loop(telemetry_bus).await });

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
                tokio::spawn(async move {
                    handle_connection(ws, peer_addr.to_string()).await;
                });
            }
            Err(e) => tracing::error!("[Robot WS] accept 错误: {e}"),
        }
    }
}

async fn handle_connection(
    mut ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    peer: String,
) {
    let welcome = serde_json::json!({"type":"welcome","message":"Robot WS connected"});
    let _ = ws.send(tokio_tungstenite::tungstenite::Message::Text(welcome.to_string())).await;

    while let Some(msg) = ws.next().await {
        match msg {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                handle_command(&text).await;
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("[Robot WS] {peer} 读取错误: {e}");
                break;
            }
        }
    }

    // 断开自动停车
    if let Some(dev) = get_stm32() {
        let _ = dev.stop().await;
    }
    tracing::info!("[Robot WS] {peer} 已断开，自动停车");
}

async fn handle_command(text: &str) {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => { tracing::warn!("[Robot WS] JSON 解析失败: {e}"); return; }
    };
    let cmd = match v["cmd"].as_str() {
        Some(c) => c,
        None => { tracing::warn!("[Robot WS] 缺少 cmd 字段"); return; }
    };

    let dev = match get_stm32() {
        Some(d) => d,
        None => { tracing::warn!("[Robot WS] STM32 设备未初始化"); return; }
    };

    let speed = v["speed"].as_i64().unwrap_or(50) as i16;
    let result = match cmd {
        "forward" => dev.forward(speed).await,
        "backward" => dev.backward(speed).await,
        "spin_left" | "left" => dev.spin_left(speed).await,
        "spin_right" | "right" => dev.spin_right(speed).await,
        "stop" => dev.stop().await,
        "beep" => {
            let ms = v["ms"].as_u64().unwrap_or(200) as u16;
            dev.beep(ms).await
        }
        _ => { tracing::warn!("[Robot WS] 未知命令: {cmd}"); return; }
    };
    if let Err(e) = result {
        tracing::warn!("[Robot WS] 命令 {cmd} 失败: {e}");
    }
}

async fn telemetry_loop(event_bus: Arc<EventBus>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let Some(dev) = get_stm32() else { continue };
        let state = dev.get_state().await;
        let payload = serde_json::json!({
            "type": "robot_telemetry",
            "vx": state.vx, "vy": state.vy, "vz": state.vz,
            "battery": state.battery,
            "roll": state.attitude.roll,
            "pitch": state.attitude.pitch,
            "yaw": state.attitude.yaw,
            "encoders": state.encoders,
        });
        event_bus.Publish(Bus_Event::State { payload: payload.to_string() });
    }
}
