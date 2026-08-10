//Presented by KeJi
//Created Date ： 2026-08-10
//Modified Date ： 2026-08-10

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
use crate::robot::core::protocol::{decode_frame, decode_pose, MSGID_POSE};

/// 入站 POSE 消费 task（launch 中 spawn）
pub async fn cluster_consumer(
    robot_bus: Option<Arc<EventBus>>,
    local_peer_id: Vec<u8>,
    table: Arc<ClusterInfoTable>,
    cancel: CancellationToken,
) {
    let mut rx = robot_bus.as_ref().map(|b| b.Subscribe());
    loop {
        tokio::select! {
            ev = recv_bus(&mut rx) => {
                if let Some(Bus_Event::StreamRaw { payload }) = ev {
                    handle_frame(&payload, &local_peer_id, &table).await;
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
async fn handle_frame(payload: &[u8], local_peer_id: &[u8], table: &ClusterInfoTable) {
    let Some(frame) = decode_frame(payload) else {
        warn!("[Cluster] 收到无法解析的 ORION 帧 ({} 字节)", payload.len());
        return;
    };
    // 本车过滤：gossipsub 自环回流本机消息（Task 13 阶段一：sysid = 完整 peer_id）
    if frame.sysid == local_peer_id {
        return;
    }
    // 本阶段只处理 POSE；MAP_DELTA 留 CRDT 重构
    if frame.msgid != MSGID_POSE {
        return;
    }
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
            yaw: pose.yaw,
            vx: pose.vx,
            vy: pose.vy,
            time_boot_ms: pose.time_boot_ms,
            last_seen: std::time::Instant::now(),
            sub_target,
        })
        .await;
    debug!(
        "[Cluster] 远端车 {}: ({:.2}, {:.2}) yaw={:.2} sub={:?}",
        hex_short(&frame.sysid),
        pose.x,
        pose.y,
        pose.yaw,
        sub_target
    );
}

/// peer_id 前 8 字节 hex（日志短格式）
fn hex_short(bytes: &[u8]) -> String {
    bytes.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::core::protocol::{encode_frame, encode_pose, PoseData, COMPID_ROBOT};

    fn make_pose(valid: bool, gx: i32, gy: i32) -> Vec<u8> {
        let p = PoseData {
            time_boot_ms: 7,
            x: 10.0,
            y: 20.0,
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
        handle_frame(&frame, &local, &table).await;
        assert_eq!(table.len().await, 0);
    }

    #[tokio::test]
    async fn test_remote_pose_stored() {
        let table = ClusterInfoTable::new();
        let local = vec![0x01];
        let remote = vec![0xAA, 0xBB, 0xCC];
        let frame = encode_frame(MSGID_POSE, &remote, COMPID_ROBOT, &make_pose(true, 5, 6));
        handle_frame(&frame, &local, &table).await;
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
        handle_frame(&frame, &local, &table).await;
        let info = table.get(&remote).await.unwrap();
        assert_eq!(info.sub_target, None);
    }

    #[tokio::test]
    async fn test_non_pose_frame_ignored() {
        let table = ClusterInfoTable::new();
        let local = vec![0x01];
        let remote = vec![0xAA];
        // 非 POSE 消息（如 MAP_DELTA msgid=3 的假帧）不处理
        let frame = encode_frame(3, &remote, COMPID_ROBOT, &[0u8; 6]);
        handle_frame(&frame, &local, &table).await;
        assert_eq!(table.len().await, 0);
    }

    #[tokio::test]
    async fn test_garbage_frame_ignored() {
        let table = ClusterInfoTable::new();
        let local = vec![0x01];
        handle_frame(&[0xDE, 0xAD, 0xBE, 0xEF], &local, &table).await;
        assert_eq!(table.len().await, 0);
    }
}
