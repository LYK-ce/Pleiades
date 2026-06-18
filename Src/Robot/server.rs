//Presented by KeJi
//Date ： 2026-06-18

//! 机器人 WebSocket 遥控服务器
//!
//! 独立线程运行，监听 TCP 端口，接收 JSON 控制指令，
//! 直接调用 Robot API，不经过 Lua。

use std::sync::Arc;
use std::thread;

use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

use crate::event_bus::{Bus_Event, EventBus};
use crate::robot::control::capability::RobotCapability;
use crate::robot::control::types::RobotState;
use crate::robot::{get_robot, Robot};

// ============================================================
// 公开入口
// ============================================================

/// 在独立线程中启动 Robot WebSocket 遥控服务器。
///
/// # 参数
/// - `port`: 监听端口
/// - `event_bus`: 用于推送遥测数据到 TUI
pub fn spawn_robot_ws_server(port: u16, event_bus: Arc<EventBus>) {
    thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("[Robot WS] 创建 tokio runtime 失败: {}", e);
                return;
            }
        };

        rt.block_on(async {
            run_server(port, event_bus).await;
        });
    });
}

// ============================================================
// 服务器主循环
// ============================================================

async fn run_server(port: u16, event_bus: Arc<EventBus>) {
    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("[Robot WS] 绑定 {} 失败: {}", addr, e);
            return;
        }
    };

    tracing::info!("[Robot WS] 遥控服务器已启动: ws://{}", addr);
    event_bus.Publish(Bus_Event::Notify {
        level: crate::event_bus::NotifyLevel::Info,
        message: format!("Robot WS 服务器已启动: ws://0.0.0.0:{}", port),
    });

    // 启动遥测定时器
    let telemetry_bus = event_bus.clone();
    tokio::spawn(async move {
        telemetry_loop(telemetry_bus).await;
    });

    // 接受连接循环
    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                tracing::info!("[Robot WS] 新连接: {}", peer_addr);
                event_bus.Publish(Bus_Event::Notify {
                    level: crate::event_bus::NotifyLevel::Info,
                    message: format!("Robot WS 客户端已连接: {}", peer_addr),
                });

                let ws = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        tracing::warn!("[Robot WS] WebSocket 握手失败: {} - {}", peer_addr, e);
                        continue;
                    }
                };

                tokio::spawn(async move {
                    handle_connection(ws, peer_addr.to_string()).await;
                });
            }
            Err(e) => {
                tracing::error!("[Robot WS] accept 错误: {}", e);
            }
        }
    }
}

// ============================================================
// 单个连接处理
// ============================================================

async fn handle_connection(
    mut ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    peer: String,
) {
    let robot = get_robot();

    // 连接时发一个欢迎消息
    let welcome = serde_json::json!({
        "type": "welcome",
        "message": "Robot WS connected"
    });
    let _ = ws
        .send(tokio_tungstenite::tungstenite::Message::Text(
            welcome.to_string(),
        ))
        .await;

    while let Some(msg) = ws.next().await {
        match msg {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                tracing::debug!("[Robot WS] {} → {}", peer, text);
                handle_command(&robot, &text).await;
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                tracing::info!("[Robot WS] {} 断开连接", peer);
                break;
            }
            Ok(_) => {} // 忽略 Binary/Ping/Pong
            Err(e) => {
                tracing::warn!("[Robot WS] {} 读取错误: {}", peer, e);
                break;
            }
        }
    }

    // 断开时自动停车
    tracing::info!("[Robot WS] {} 已断开，自动停车", peer);
    let _ = robot.stop().await;
}

// ============================================================
// 命令解析
// ============================================================

async fn handle_command(robot: &Robot, text: &str) {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[Robot WS] JSON 解析失败: {} - {}", text, e);
            return;
        }
    };

    let cmd = match v["cmd"].as_str() {
        Some(c) => c,
        None => {
            tracing::warn!("[Robot WS] 缺少 cmd 字段: {}", text);
            return;
        }
    };

    let speed = v["speed"].as_i64().unwrap_or(50) as i16;

    let result = match cmd {
        "forward" => robot.forward(speed).await,
        "backward" => robot.backward(speed).await,
        "spin_left" | "left" => robot.spin_left(speed).await,
        "spin_right" | "right" => robot.spin_right(speed).await,
        "stop" => robot.stop().await,
        "beep" => {
            let ms = v["ms"].as_u64().unwrap_or(200) as u16;
            robot.beep(ms).await
        }
        "open" => {
            let port = v["port"].as_str().unwrap_or("/dev/myserial");
            let baudrate = v["baudrate"].as_u64().unwrap_or(115200) as u32;
            let car_type = v["car_type"].as_u64().unwrap_or(2) as u8;
            let ct = match crate::robot::control::types::CarType::from_u8(car_type) {
                Some(c) => c,
                None => {
                    tracing::warn!("[Robot WS] 无效车型: {}", car_type);
                    return;
                }
            };
            robot.open(port, baudrate, ct).await
        }
        "close" => robot.close().await,
        _ => {
            tracing::warn!("[Robot WS] 未知命令: {}", cmd);
            return;
        }
    };

    if let Err(e) = result {
        tracing::warn!("[Robot WS] 命令 {} 执行失败: {}", cmd, e);
    }
}

// ============================================================
// 遥测定时器
// ============================================================

async fn telemetry_loop(event_bus: Arc<EventBus>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let robot = get_robot();
        if !robot.is_open() {
            continue;
        }

        let state = robot.get_state().await;
        push_telemetry(&event_bus, &state);
    }
}

fn push_telemetry(event_bus: &EventBus, state: &RobotState) {
    let payload = serde_json::json!({
        "type": "robot_telemetry",
        "vx": state.vx,
        "vy": state.vy,
        "vz": state.vz,
        "battery": state.battery,
        "roll": state.attitude.roll,
        "pitch": state.attitude.pitch,
        "yaw": state.attitude.yaw,
        "encoders": state.encoders,
    });
    event_bus.Publish(Bus_Event::State {
        payload: payload.to_string(),
    });
}
