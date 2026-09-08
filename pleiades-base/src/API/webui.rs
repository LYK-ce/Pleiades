//Presented by KeJi
//Created Date ： 2026-09-08

//! YOLO 检测结果 WebSocket 展示服务
//!
//! - `spawn_webui_server(port)`：起 WebSocket 服务（绑 0.0.0.0），返回端口
//! - `webui_publish(jpeg, dets_json)`：打包一帧并广播；服务未启动时静默丢弃
//! - 每个浏览器连接订阅广播通道，服务把每帧以二进制消息转发
//!
//! 消息格式（一条二进制消息 = 一帧）：
//! `[4B header_len LE][header JSON: {dets:[...]}][JPEG 原始字节]`

use std::net::SocketAddr;
use std::sync::OnceLock;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use tokio::sync::broadcast;

/// 全局广播通道（webui 未启动时为 None，`webui_publish` 静默丢弃）
static FRAME_TX: OnceLock<broadcast::Sender<Vec<u8>>> = OnceLock::new();

/// 启动 WebSocket 展示服务，返回实际绑定端口。
pub async fn spawn_webui_server(port: u16) -> Result<u16, String> {
    // 幂等初始化广播通道（已启动则复用）
    if FRAME_TX.get().is_none() {
        let (tx, _rx) = broadcast::channel(16);
        let _ = FRAME_TX.set(tx);
    }

    let app = Router::new().route("/ws", get(ws_handler));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;

    tracing::info!("WebUI server listening on ws://{addr}");

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("WebUI server exited: {e}");
        }
    });

    Ok(port)
}

/// 打包一帧（`[4B header_len][JSON][JPEG]`）并广播给所有浏览器连接。
/// 服务未启动时静默丢弃。
pub fn webui_publish(jpeg: &[u8], dets_json: &str) {
    let Some(tx) = FRAME_TX.get() else {
        return;
    };
    let mut payload = Vec::with_capacity(4 + dets_json.len() + jpeg.len());
    payload.extend_from_slice(&(dets_json.len() as u32).to_le_bytes());
    payload.extend_from_slice(dets_json.as_bytes());
    payload.extend_from_slice(jpeg);
    let _ = tx.send(payload);
}

/// WebSocket 升级入口。
async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

/// 每连接转发 task：订阅广播，把每帧以二进制消息发给浏览器。
async fn handle_socket(mut socket: WebSocket) {
    let Some(tx) = FRAME_TX.get() else {
        return;
    };
    let mut rx = tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(payload) => {
                if socket.send(Message::Binary(payload)).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue, // 浏览器慢，丢帧
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
