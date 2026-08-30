//Presented by KeJi
//Created Date ： 2026-08-10
//Modified Date ： 2026-08-20

//! 集群入站消费者（Task 13_1）
//!
//! 订阅 robot_bus → 解码 POSE 帧 → 过滤本车（gossipsub 自环回流）→ 写入 ClusterInfo 表。
//! 独立 task：数据面与 main_loop 控制面解耦（实时性互不拖累）。
//!
//! 本阶段只处理 POSE（msgid=1）；MAP_DELTA 留后续 CRDT 地图重构（人类指示挂起）。

use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::event_bus::{Bus_Event, EventBus};
use crate::robot::core::cluster::cluster_info::{ClusterInfo, ClusterInfoTable};
use crate::robot::core::protocol::{decode_frame, decode_map_delta, decode_pose, MSGID_MAP_DELTA, MSGID_POSE};
use crate::robot::core::grid::OccupancyGrid;
use tokio::sync::RwLock;

/// 入站 POSE 消费 task（launch 中 spawn）
pub async fn cluster_consumer(
    robot_bus: Option<Arc<EventBus>>,
    local_peer_id: Vec<u8>,
    table: Arc<ClusterInfoTable>,
    grid: Arc<RwLock<OccupancyGrid>>,
    cancel: CancellationToken,
) {
    let mut rx = robot_bus.as_ref().map(|b| b.Subscribe());
    loop {
        tokio::select! {
            ev = recv_bus(&mut rx) => {
                if let Some(Bus_Event::StreamRaw { payload }) = ev {
                    handle_frame(&payload, &local_peer_id, &table, &grid).await;
                }
            }
            _ = cancel.cancelled() => {
                debug!("cluster_consumer 退出");
                return;
            }
        }
    }
}

/// robot_bus 订阅接收（None 时永远 pending；Lagged 警告后继续）
async fn recv_bus(rx: &mut Option<tokio::sync::broadcast::Receiver<Bus_Event>>) -> Option<Bus_Event> {
    match rx.as_mut() {
        Some(rx) => match rx.recv().await {
            Ok(ev) => Some(ev),
            Err(RecvError::Lagged(n)) => {
                warn!("[Cluster] robot_bus 订阅落后 {n} 条");
                None
            }
            Err(RecvError::Closed) => None,
        },
        None => std::future::pending().await,
    }
}

/// 处理一帧入站数据（独立函数便于单测）
async fn handle_frame(
    payload: &[u8],
    local_peer_id: &[u8],
    table: &ClusterInfoTable,
    grid: &Arc<RwLock<OccupancyGrid>>,
) {
    let Some(frame) = decode_frame(payload) else {
        warn!("[Cluster] 收到无法解析的 ORION 帧 ({} 字节)", payload.len());
        return;
    };
    // 本车过滤：gossipsub 自环回流本机消息（Task 13 阶段一：sysid = 完整 peer_id）
    if frame.sysid == local_peer_id {
        return;
    }
    match frame.msgid {
        MSGID_POSE => {
            let Some(pose) = decode_pose(&frame.payload) else {
                warn!("[Cluster] POSE 帧解析失败 ({} 字节)", frame.payload.len());
                return;
            };
            let sub_target = if pose.valid { Some((pose.sub_gx, pose.sub_gy)) } else { None };
            table
                .upsert(ClusterInfo {
                    peer_id: frame.sysid.clone(),
                    x: pose.x,
                    y: pose.y,
                    z: pose.z,
                    yaw: pose.yaw,
                    vx: pose.vx,
                    vy: pose.vy,
                    time_boot_ms: pose.time_boot_ms,
                    last_seen: std::time::Instant::now(),
                    sub_target,
                })
                .await;
            debug!(
                "[Cluster] 远端车 {}: ({:.2}, {:.2}, {:.2}) yaw={:.2} sub={:?}",
                hex_short(&frame.sysid),
                pose.x,
                pose.y,
                pose.z,
                pose.yaw,
                sub_target
            );
        }
        // Task 13_2：入站 MAP_DELTA（车→车增量）→ merged 累加（只写 chunk，own 不碰）
        MSGID_MAP_DELTA => {
            let Some((_, entries)) = decode_map_delta(&frame.payload) else {
                warn!("[Cluster] MAP_DELTA 帧解析失败 ({} 字节)", frame.payload.len());
                return;
            };
            let mut g = grid.write().await;
            let (mut applied, mut skipped) = (0, 0);
            for e in &entries {
                if g.apply_delta(e.gx, e.gy, e.delta) {
                    applied += 1;
                } else {
                    skipped += 1;
                }
            }
            if skipped > 0 {
                warn!("[Cluster] MAP_DELTA 来自 {}: {} 条越界跳过", hex_short(&frame.sysid), skipped);
            }
            if applied > 0 {
                debug!("[Cluster] 远端车 {} 地图增量应用 {} 格", hex_short(&frame.sysid), applied);
            }
        }
        // 其他 msgid 忽略（gossip 通道不承载 MAP_FULL；未来如加入需同步处理）
        _ => {}
    }
}

/// peer_id 前 8 字节 hex（日志短格式）
fn hex_short(bytes: &[u8]) -> String {
    bytes.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::core::protocol::{
        encode_frame, encode_map_delta, encode_pose, MapDeltaEntry, PoseData, COMPID_ROBOT, MSGID_MAP_DELTA,
    };

    fn make_grid() -> Arc<RwLock<OccupancyGrid>> {
        Arc::new(RwLock::new(OccupancyGrid::new()))
    }

    fn make_pose(valid: bool, gx: i32, gy: i32) -> Vec<u8> {
        let p = PoseData {
            time_boot_ms: 7,
            x: 10.0,
            y: 20.0,
            z: 0.0,
            vx: 0.5,
            vy: 0.0,
            yaw: 1.0,
            valid,
            sub_gx: gx,
            sub_gy: gy,
        };
        encode_pose(&p)
    }

    #[tokio::test]
    async fn test_self_frame_filtered() {
        // gossipsub 自环：本车 peer_id 的帧必须被过滤
        let table = ClusterInfoTable::new();
        let local = vec![0x01, 0x02];
        let frame = encode_frame(MSGID_POSE, &local, COMPID_ROBOT, &make_pose(true, 3, 4));
        handle_frame(&frame, &local, &table, &make_grid()).await;
        assert_eq!(table.len().await, 0);
    }

    #[tokio::test]
    async fn test_remote_pose_stored() {
        let table = ClusterInfoTable::new();
        let local = vec![0x01];
        let remote = vec![0xAA, 0xBB, 0xCC];
        let frame = encode_frame(MSGID_POSE, &remote, COMPID_ROBOT, &make_pose(true, 5, 6));
        handle_frame(&frame, &local, &table, &make_grid()).await;
        let info = table.get(&remote).await.unwrap();
        assert_eq!(info.x, 10.0);
        assert_eq!(info.y, 20.0);
        assert_eq!(info.time_boot_ms, 7);
        assert_eq!(info.sub_target, Some((5, 6)));
    }

    #[tokio::test]
    async fn test_no_intent_stored_as_none() {
        let table = ClusterInfoTable::new();
        let local = vec![0x01];
        let remote = vec![0xAA];
        // valid=false：sub 坐标忽略，表中 sub_target = None
        let frame = encode_frame(MSGID_POSE, &remote, COMPID_ROBOT, &make_pose(false, 9, 9));
        handle_frame(&frame, &local, &table, &make_grid()).await;
        let info = table.get(&remote).await.unwrap();
        assert_eq!(info.sub_target, None);
    }

    #[tokio::test]
    async fn test_remote_map_delta_applied() {
        // Task 13_2：远端 MAP_DELTA → merged 累加，own 不变
        let table = ClusterInfoTable::new();
        let grid = make_grid();
        let local = vec![0x01];
        let remote = vec![0xAA];
        let entries = vec![MapDeltaEntry { gx: 10, gy: 10, delta: 3 }];
        let frame = encode_frame(MSGID_MAP_DELTA, &remote, COMPID_ROBOT, &encode_map_delta(1, &entries));
        handle_frame(&frame, &local, &table, &grid).await;
        {
            let g = grid.read().await;
            assert_eq!(g.chunk.get(10, 10).unwrap(), 3, "merged 应累加远端 Δ");
            assert_eq!(g.own.get(10, 10).unwrap(), 0, "own 不得被远端增量污染");
        }
        // POSE 表不受影响
        assert_eq!(table.len().await, 0);
    }

    #[tokio::test]
    async fn test_unknown_msgid_ignored() {
        // 非 POSE/MAP_DELTA msgid（如 9）不处理
        let table = ClusterInfoTable::new();
        let grid = make_grid();
        let local = vec![0x01];
        let remote = vec![0xAA];
        let frame = encode_frame(9, &remote, COMPID_ROBOT, &[0u8; 6]);
        handle_frame(&frame, &local, &table, &grid).await;
        assert_eq!(table.len().await, 0);
        assert_eq!(grid.read().await.chunk.get(0, 0).unwrap(), 0);
    }

    #[tokio::test]
    async fn test_garbage_frame_ignored() {
        let table = ClusterInfoTable::new();
        let local = vec![0x01];
        handle_frame(&[0xDE, 0xAD, 0xBE, 0xEF], &local, &table, &make_grid()).await;
        assert_eq!(table.len().await, 0);
    }
}
