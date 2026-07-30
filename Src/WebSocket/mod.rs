//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-07-21

//! WebSocket 遥控通信层
//!
//! 订阅 Robot 的 pose_tx / map_tx 广播，序列化为 JSON 转发给客户端。
//! 接收 JSON 控制指令 → 转发给 Robot。
//! 与 Robot 平级，不依赖任何 Robot 内部 state。

pub mod server;
pub mod protocol;

use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use crate::robot::core::command::Command;
use crate::robot::core::robot::{Pose, MapDelta};
use crate::robot::slam::OccupancyGrid;

/// 启动 WebSocket 遥控服务器
pub fn start(
    bind_addr: &str,
    vehicle_id: &str,
    robot_cmd_tx: mpsc::Sender<Command>,
    pose_rx: broadcast::Receiver<Pose>,
    map_rx: broadcast::Receiver<Vec<MapDelta>>,
    grid: Arc<RwLock<OccupancyGrid>>,
) {
    let addr = bind_addr.to_string();
    let id = vehicle_id.to_string();
    tokio::spawn(async move {
        server::run(addr, id, robot_cmd_tx, pose_rx, map_rx, grid).await;
    });
}
