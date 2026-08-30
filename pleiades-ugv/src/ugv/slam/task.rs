//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-08-20

//! SLAM 建图 task（Task 22 步骤 4：从 robot.rs 移入，归雷达设备 spawn 内部）

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::select;
use tokio::sync::{broadcast, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use pleiades_base::network::{NodeHandle, TOPIC_ROBOT_MAP};
use pleiades_base::robot::core::cluster::ClusterInfoTable;
use crate::ugv::planning::cluster_to_obstacle_cells;
use pleiades_base::robot::core::protocol::{
    encode_frame, encode_map_delta, now_boot_ms, COMPID_ROBOT, MapDeltaEntry, MSGID_MAP_DELTA,
};
use pleiades_base::robot::core::state::RobotState;
use crate::ugv::lidar::LidarState;
use pleiades_base::robot::core::grid::{Delta, OccupancyGrid};
use pleiades_base::robot::core::map_delta::MapDelta;
use crate::ugv::slam::{RobotPose, update};



/// SLAM 建图 task 所需的上下文（由雷达设备 spawn 时打包传入）
pub struct SlamContext {
    pub grid: Arc<RwLock<OccupancyGrid>>,
    pub robot_state: Arc<RwLock<RobotState>>,
    pub cluster_table: Arc<ClusterInfoTable>,
    pub map_tx: broadcast::Sender<Vec<MapDelta>>,
    pub node_handle: Option<Arc<NodeHandle>>,
    pub local_peer_id: Option<Vec<u8>>,
    pub obstacle_inflation_radius: f32,
}

/// SLAM task — 地图更新 (200ms)
async fn slam_task(
    grid: Arc<RwLock<OccupancyGrid>>,
    robot_state: Arc<RwLock<RobotState>>,
    lidar_state: Arc<RwLock<LidarState>>,
    map_tx: broadcast::Sender<Vec<MapDelta>>,
    node_handle: Option<Arc<NodeHandle>>,
    local_peer_id: Option<Vec<u8>>,
    cluster_table: Arc<ClusterInfoTable>,
    obstacle_inflation_radius: f32,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(200));
    // Task 13_2：跨帧聚合缓冲（Δ≠0 才广播）+ 发送节流（模 5 = 1s 一次）
    // 聚合按格累加净变化（真实差分，不做 ±8 clamp——窗口内净变化上界 ±16 在 i8 内，
    // 且每 5 帧必 clear，不会无限增长；接收方应用时再 clamp ±8 完成精确重放）
    let mut frame_count: u32 = 0;
    let mut pending: HashMap<(i32, i32), i8> = HashMap::new();
    loop {
        select! {
            _ = interval.tick() => {
                // 同时读位姿和 LiDAR（两个独立锁，无死锁风险）
                let (pose, scan_points) = {
                    let rs = robot_state.read().await;
                    // 位姿 = 全局世界坐标（直读 RobotState，Task 9）
                    let pose = RobotPose {
                        x: rs.x,
                        y: rs.y,
                        yaw: rs.attitude.yaw,
                    };
                    let ls = lidar_state.read().await;
                    let pts: Vec<(f32, f32)> = match &ls.scan {
                        Some(s) => {
                            let mut v = Vec::with_capacity(s.points.len());
                            v.extend(s.points.iter().map(|p| (p.angle, p.range)));
                            v
                        }
                        None => Vec::new(),
                    };
                    (pose, pts)
                };

                if !scan_points.is_empty() {
                    // Task 15 C 节：锁外读集群表 → 他车格掩蔽集合（避免持 grid 写锁时再拿 cluster 锁）
                    let masked = {
                        let others = cluster_table.snapshot().await;
                        cluster_to_obstacle_cells(&others, obstacle_inflation_radius)
                    };
                    let mut g = grid.write().await;
                    let deltas = update(&mut *g, &pose, &scan_points, &masked);
                    // 聚合：每帧 Δ 累加进 pending（真实差分）
                    accumulate_pending(&mut pending, &deltas);
                }

                // 节流：每 5 帧（1s）发送一次，WS 与 gossip 两条链路一致
                frame_count = frame_count.wrapping_add(1);
                if frame_count % 5 == 0 {
                    let out = drain_pending(&mut pending);

                    if !out.is_empty() {
                        let _ = map_tx.send(out.clone());

                        // Task 9_2：地图增量广播到集群（Δ≠0 才发）
                        // Task 13 阶段一：帧身份 = 完整 peer_id（launch 时一次计算）
                        if let (Some(nh), Some(peer_id)) = (&node_handle, &local_peer_id) {
                            let entries: Vec<MapDeltaEntry> = out.iter().map(|d| MapDeltaEntry {
                                gx: d.gx, gy: d.gy, delta: d.delta,
                            }).collect();
                            // ORION 协议：地图增量帧广播（2026-08-07 协议统一，替代散装 JSON）
                            let payload = encode_map_delta(now_boot_ms(), &entries);
                            let frame = encode_frame(MSGID_MAP_DELTA, peer_id, COMPID_ROBOT, &payload);
                            if let Err(e) = nh.Gossipsub_Publish(TOPIC_ROBOT_MAP, frame).await {
                                warn!("[Robot] 地图增量广播失败: {e}");
                            }
                        }
                    }
                }
            }
            _ = cancel.cancelled() => {
                info!("SLAM task 退出");
                return;
            }
        }
    }
}

/// 聚合纯函数：把一帧的 deltas 累加进 pending（Task 13_2）
///
/// 累加**真实差分**（不做 ±8 clamp）：窗口（5 帧）内净变化上界 ±16（own 从 −8 冲到 +8），
/// 在 i8 范围内；且 pending 每 5 帧 drain 清空，不会无限增长。
/// 接收方应用时再 clamp ±8，完成"发送方 own 轨迹精确重放"。
fn accumulate_pending(pending: &mut HashMap<(i32, i32), i8>, deltas: &[Delta]) {
    for d in deltas {
        let v = pending.entry((d.gx, d.gy)).or_insert(0);
        *v = v.saturating_add(d.delta);
    }
}

/// 收集纯函数：取出 Δ≠0 项（净变化为 0 的格子不广播），并清空 pending
fn drain_pending(pending: &mut HashMap<(i32, i32), i8>) -> Vec<MapDelta> {
    let out: Vec<MapDelta> = pending
        .iter()
        .filter(|(_, &delta)| delta != 0)
        .map(|(&(gx, gy), &delta)| MapDelta { gx, gy, delta })
        .collect();
    pending.clear();
    out
}

/// 在雷达设备内 spawn SLAM 建图 task（Task 22：SLAM 归雷达设备）
pub fn spawn_slam(ctx: SlamContext, lidar_state: Arc<RwLock<LidarState>>, cancel: CancellationToken) {
    let SlamContext {
        grid,
        robot_state,
        cluster_table,
        map_tx,
        node_handle,
        local_peer_id,
        obstacle_inflation_radius,
    } = ctx;
    tokio::spawn(async move {
        slam_task(
            grid,
            robot_state,
            lidar_state,
            map_tx,
            node_handle,
            local_peer_id,
            cluster_table,
            obstacle_inflation_radius,
            cancel,
        ).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accumulate_drain_basic() {
        let mut pending = HashMap::new();
        let deltas = vec![
            Delta { gx: 1, gy: 1, delta: 3 },
            Delta { gx: 2, gy: 2, delta: -1 },
        ];
        accumulate_pending(&mut pending, &deltas);
        let out = drain_pending(&mut pending);
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|d| d.gx == 1 && d.delta == 3));
        assert!(out.iter().any(|d| d.gx == 2 && d.delta == -1));
        assert!(pending.is_empty(), "drain 后必须清空");
    }

    #[test]
    fn test_accumulate_preserves_exact_differential() {
        // 回归锁定：own 从 −8 连续命中 5 帧 → 窗口净变化 +15
        // 必须保留真实差分（不能被 clamp 截断成 +8），接收方才能精确重放
        let mut pending = HashMap::new();
        let deltas: Vec<Delta> = (0..5).map(|_| Delta { gx: 9, gy: 9, delta: 3 }).collect();
        accumulate_pending(&mut pending, &deltas);
        let out = drain_pending(&mut pending);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].delta, 15, "窗口净变化 +15 必须保留");
    }

    #[test]
    fn test_accumulate_net_zero_filtered() {
        // 窗口内 +3 与 −1×3 抵消 → 净 0 → 不广播
        let mut pending = HashMap::new();
        let deltas = vec![
            Delta { gx: 5, gy: 5, delta: 3 },
            Delta { gx: 5, gy: 5, delta: -1 },
            Delta { gx: 5, gy: 5, delta: -1 },
            Delta { gx: 5, gy: 5, delta: -1 },
        ];
        accumulate_pending(&mut pending, &deltas);
        let out = drain_pending(&mut pending);
        assert!(out.is_empty(), "净变化 0 的格子不应广播");
        assert!(pending.is_empty());
    }

    #[test]
    fn test_accumulate_negative_saturation_safe() {
        // 窗口反向净变化 −5 也保留（不截断）
        let mut pending = HashMap::new();
        let deltas: Vec<Delta> = (0..5).map(|_| Delta { gx: 7, gy: 7, delta: -1 }).collect();
        accumulate_pending(&mut pending, &deltas);
        let out = drain_pending(&mut pending);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].delta, -5);
    }
}
