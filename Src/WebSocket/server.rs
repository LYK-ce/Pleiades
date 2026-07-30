//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-07-22

//! WebSocket 服务器核心
//!
//! accept 循环 + handle_connection + pose/map/map_full 转发器。

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use std::sync::Arc;
use tokio::sync::RwLock;
use super::protocol::parse_command;
use crate::robot::core::command::{Command, ManualCmd};
use crate::robot::core::robot::{Pose, MapDelta};
use crate::robot::slam::OccupancyGrid;

pub async fn run(
    bind_addr: String,
    vehicle_id: String,
    robot_cmd_tx: mpsc::Sender<Command>,
    pose_rx: broadcast::Receiver<Pose>,
    map_rx: broadcast::Receiver<Vec<MapDelta>>,
    grid: Arc<RwLock<OccupancyGrid>>,
) {
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("[WS] 绑定 {bind_addr} 失败: {e}");
            return;
        }
    };
    tracing::info!("[WS] 遥控服务器已启动: ws://{bind_addr}");

    // 本地 broadcast：汇总 text 消息 (pose + map_delta)
    let (feed_tx, _) = broadcast::channel::<String>(32);

    // pose 转发
    let pose_feed = feed_tx.clone();
    let mut pose_rx2 = pose_rx.resubscribe();
    tokio::spawn(async move {
        loop {
            match pose_rx2.recv().await {
                Ok(p) => {
                    let json = serde_json::json!({
                        "type": "pose",
                        "ts": p.ts,
                        "x": p.x, "y": p.y, "z": p.z,
                        "yaw": p.yaw,
                        "vx": p.vx, "vy": p.vy,
                    }).to_string();
                    let _ = pose_feed.send(json);
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // map_delta 转发
    let map_feed = feed_tx.clone();
    let mut map_rx2 = map_rx.resubscribe();
    tokio::spawn(async move {
        loop {
            match map_rx2.recv().await {
                Ok(deltas) => {
                    let voxels: Vec<serde_json::Value> = deltas.iter().map(|d| {
                        serde_json::json!({"gx": d.gx, "gy": d.gy, "state": d.state})
                    }).collect();
                    let json = serde_json::json!({
                        "type": "map_delta",
                        "voxels": voxels,
                    }).to_string();
                    let _ = map_feed.send(json);
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("[WS] map 广播 lagged {n}，部分 voxel 丢失");
                    continue;
                }
            }
        }
    });

    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                tracing::info!("[WS] 新连接: {peer_addr}");
                let ws = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        warn!("[WS] 握手失败: {peer_addr} - {e}");
                        continue;
                    }
                };
                let tx = robot_cmd_tx.clone();
                let feed = feed_tx.subscribe();
                let vid = vehicle_id.clone();
                let addr = bind_addr.clone();
                let g = grid.clone();
                tokio::spawn(async move {
                    handle_connection(ws, tx, feed, vid, addr, peer_addr.to_string(), g).await;
                });
            }
            Err(e) => error!("[WS] accept 错误: {e}"),
        }
    }
}

async fn handle_connection(
    mut ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    robot_cmd_tx: mpsc::Sender<Command>,
    mut feed_rx: broadcast::Receiver<String>,
    vehicle_id: String,
    bind_addr: String,
    peer: String,
    grid: Arc<RwLock<OccupancyGrid>>,
) {
    let hello = serde_json::json!({
        "type": "hello",
        "vehicle_id": vehicle_id,
        "address": format!("ws://{bind_addr}")
    });
    let _ = ws.send(Message::Text(hello.to_string())).await;

    // 发送全量地图
    {
        let map_data = grid.read().await.build_map_full();
        info!("[WS] {peer} 发送 map_full: {} 字节", map_data.len());
        let _ = ws.send(Message::Binary(map_data.into())).await;
    }

    let (mut ws_tx, mut ws_rx) = ws.split();

    // 转发 text 消息到客户端（pose + map_delta）
    let feed_handle = tokio::spawn(async move {
        loop {
            match feed_rx.recv().await {
                Ok(json) => {
                    if ws_tx.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // 接收客户端命令
    info!("[WS] {peer} 已连接");
    while let Some(msg) = ws_rx.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                info!("[WS] {peer} 收到: {text}");
                if let Some(cmd) = parse_command(&text) {
                    let _ = robot_cmd_tx.send(cmd).await;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(e) => {
                warn!("[WS] {peer} 读取错误: {e}");
                break;
            }
        }
    }

    feed_handle.abort();
    if robot_cmd_tx.send(Command::Manual(ManualCmd::Stop)).await.is_err() {
        warn!("[WS] {peer} 断开时无法发送 Stop");
    }
    tracing::info!("[WS] {peer} 已断开，自动停车");
}
