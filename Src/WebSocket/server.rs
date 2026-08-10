//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-07

//! WebSocket 服务器核心
//!
//! accept 循环 + handle_connection + pose/map/map_full 转发器。
//!
//! 2026-08-07 协议统一：WS 通道保留（Pictor 仍走 WS 连接），
//! 消息 payload 统一为 ORION 协议帧（二进制）；hello 连接握手保留（JSON，唯一不换的消息）。

use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use std::sync::Arc;
use tokio::sync::RwLock;
use super::protocol::parse_orion_frame;
use crate::robot::core::command::{Command, ManualCmd};
use crate::robot::core::protocol::{
    encode_frame, encode_map_delta, encode_map_full, encode_pose, now_boot_ms,
    COMPID_ROBOT, MSGID_MAP_DELTA, MSGID_MAP_FULL, MSGID_POSE, MapDeltaEntry, PoseData,
};
use crate::robot::core::robot::{MapDelta, Pose};
use crate::robot::slam::{CELL_RESOLUTION, CHUNK_SIZE, OccupancyGrid};

pub async fn run(
    bind_addr: String,
    vehicle_id: String,
    robot_cmd_tx: mpsc::Sender<Command>,
    pose_rx: broadcast::Receiver<Pose>,
    map_rx: broadcast::Receiver<Vec<MapDelta>>,
    grid: Arc<RwLock<OccupancyGrid>>,
    local_peer_id: Vec<u8>,
) {
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("[WS] 绑定 {bind_addr} 失败: {e}");
            return;
        }
    };
    tracing::info!("[WS] 遥控服务器已启动: ws://{bind_addr}");

    // 本地 broadcast：汇总 ORION 帧字节 (pose + map_delta)
    let (feed_tx, _) = broadcast::channel::<Vec<u8>>(32);

    // pose 转发（ORION_POSE 帧，Task 13 阶段一：sysid = 本车 peer_id）
    let pose_feed = feed_tx.clone();
    let mut pose_rx2 = pose_rx.resubscribe();
    let pose_peer = local_peer_id.clone();
    tokio::spawn(async move {
        loop {
            match pose_rx2.recv().await {
                Ok(p) => {
                    let pose = PoseData {
                        time_boot_ms: p.time_boot_ms,
                        x: p.x, y: p.y,
                        vx: p.vx, vy: p.vy,
                        yaw: p.yaw,
                    };
                    let frame = encode_frame(MSGID_POSE, &pose_peer, COMPID_ROBOT, &encode_pose(&pose));
                    let _ = pose_feed.send(frame);
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // map_delta 转发（ORION_MAP_DELTA 帧，Task 13 阶段一：sysid = 本车 peer_id）
    let map_feed = feed_tx.clone();
    let mut map_rx2 = map_rx.resubscribe();
    let map_peer = local_peer_id.clone();
    tokio::spawn(async move {
        loop {
            match map_rx2.recv().await {
                Ok(deltas) => {
                    let entries: Vec<MapDeltaEntry> = deltas.iter()
                        .map(|d| MapDeltaEntry { gx: d.gx, gy: d.gy, state: d.state })
                        .collect();
                    let payload = encode_map_delta(now_boot_ms(), &entries);
                    let frame = encode_frame(MSGID_MAP_DELTA, &map_peer, COMPID_ROBOT, &payload);
                    let _ = map_feed.send(frame);
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
                let conn_peer = local_peer_id.clone();
                tokio::spawn(async move {
                    handle_connection(ws, tx, feed, vid, addr, peer_addr.to_string(), g, conn_peer).await;
                });
            }
            Err(e) => error!("[WS] accept 错误: {e}"),
        }
    }
}

async fn handle_connection(
    mut ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    robot_cmd_tx: mpsc::Sender<Command>,
    mut feed_rx: broadcast::Receiver<Vec<u8>>,
    vehicle_id: String,
    bind_addr: String,
    peer: String,
    grid: Arc<RwLock<OccupancyGrid>>,
    local_peer_id: Vec<u8>,
) {
    // hello：连接握手（过渡期保留，唯一 JSON 消息）
    let hello = serde_json::json!({
        "type": "hello",
        "vehicle_id": vehicle_id,
        "address": format!("ws://{bind_addr}")
    });
    let _ = ws.send(Message::Text(hello.to_string())).await;

    // 发送全量地图（ORION_MAP_FULL 帧）
    {
        let g = grid.read().await;
        let data = g.chunk.state_bytes();
        let payload = encode_map_full(
            now_boot_ms(),
            g.chunk.origin_gx,
            g.chunk.origin_gy,
            CHUNK_SIZE as u16,
            CHUNK_SIZE as u16,
            CELL_RESOLUTION,
            data.as_ref(),
        );
        let frame = encode_frame(MSGID_MAP_FULL, &local_peer_id, COMPID_ROBOT, &payload);
        info!("[WS] {peer} 发送 map_full: {} 字节", frame.len());
        let _ = ws.send(Message::Binary(frame.into())).await;
    }

    let (mut ws_tx, mut ws_rx) = ws.split();

    // 转发 ORION 帧到客户端（pose + map_delta，二进制）
    let feed_handle = tokio::spawn(async move {
        loop {
            match feed_rx.recv().await {
                Ok(frame) => {
                    if ws_tx.send(Message::Binary(frame.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // 接收客户端命令（ORION 帧，二进制）
    info!("[WS] {peer} 已连接");
    while let Some(msg) = ws_rx.next().await {
        match msg {
            Ok(Message::Binary(bytes)) => {
                if let Some(cmd) = parse_orion_frame(&bytes) {
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
