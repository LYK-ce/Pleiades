//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 激光建图 task（task_31 §五.4.5）：200 ms 更新 + 每 5 帧（1 s）聚合净变化后广播
//!
//! 归属：**`LidarSensor` 私有**（[`crate::sim::sensors::lidar::LidarSensor::Spawn_Mapping`] 拉起），
//! 与真机 `LidarDevice::spawn(…, Some(SlamContext))` 内部 `spawn_slam` 同构（§五.1）。
//!
//! 每 tick 的固定顺序（**锁绝不跨 `await`**，真机踩过 `robot.rs:216`）：
//!
//! ```text
//! 1. 读位姿快照 { x, y, yaw } → 立刻 drop robot_state 读锁
//! 2. Age_Of(pose_age) > 1000 ms → 本周期跳过（装配门闩）+ 限流 warning
//! 3. lidar.Get_Scan().await（拿不到 → 本周期不建图）
//! 4. 锁外算 masked：cluster_table.snapshot().await → cluster_to_obstacle_cells（sim 自己的副本）
//! 5. grid.write() → mapper::Update → accumulate_pending（写锁只裹这一步，块内无 await）
//! 6. 每 5 帧：drain_pending → map_tx.send + gossip TOPIC_ROBOT_MAP（失败仅 warn）
//! ```

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, RwLock};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use pleiades_base::network::{NodeHandle, TOPIC_ROBOT_MAP};
use pleiades_base::robot::core::cluster::ClusterInfoTable;
use pleiades_base::robot::core::grid::{Delta, OccupancyGrid};
use pleiades_base::robot::core::map_delta::MapDelta;
use pleiades_base::robot::core::protocol::{
    encode_frame, encode_map_delta, now_boot_ms, MapDeltaEntry, COMPID_ROBOT, MSGID_MAP_DELTA,
};
use pleiades_base::robot::core::state::RobotState;

use crate::sim::planning::cluster_to_obstacle_cells;
use crate::sim::chassis::pose_ingest::{Age_Of, PoseHistory, POSE_STALE_MS};
use crate::sim::sensors::lidar::mapper::{self, RobotPose};
use crate::sim::sensors::lidar::LidarSensor;
use crate::sim::sensors::Warn_Throttled;
use crate::world_link::protocol::MountOut;

/// 建图更新周期（毫秒）：与真机 `slam/task.rs` 一致
pub const MAPPING_INTERVAL_MS: u64 = 200;
/// 每 N 帧广播一次（5 × 200 ms = 1 s）
pub const BROADCAST_EVERY: u32 = 5;
/// 按扫描 `stamp` 取位姿的容差（秒）：1.5 个 10 Hz 周期；超出则退回当前位姿
pub const POSE_AT_SCAN_TOL_S: f64 = 0.15;

/// 建图 task 所需的上下文（由 `start()` 打包，每个传感器 clone 一份）
///
/// ## 偏差 D5 / D6（**未获人类批准的设计变更**，见 `Workbook/wb_31_sim_world_link.md` 偏差登记表）
///
/// §五.4.5 写的是「读 `RobotState` 当前位姿」，本实现多做两件事：①用传感器**外参位姿**
/// （`mount`，D5）；②按扫描帧 `stamp` 在 3.2 s 环形历史里取**扫描时刻**的位姿（`pose_history`，D6）。
/// 两者都是为了让建图与「世界侧雷达从挂载点起算、且扫描帧与位姿不同步」对齐：
/// 不补 `mount.x` 会把每个命中点沿车头平移 0.1 m（20% 格宽，S3 车在墙前冻结）；
/// 不按 `stamp` 取位姿会因 ~80 ms 位姿-扫描差把掠射点转偏 0.10 m（转弯时最明显）。
/// 结论与证据见 `RobotSim_TestEnv/results/conclusion.md`「需人类决策」。
#[derive(Clone)]
pub struct MappingCtx {
    /// 公共世界状态：本 task 是唯一写者（§五.1）
    pub grid: Arc<RwLock<OccupancyGrid>>,
    pub robot_state: Arc<RwLock<RobotState>>,
    pub cluster_table: Arc<ClusterInfoTable>,
    pub map_tx: broadcast::Sender<Vec<MapDelta>>,
    pub node_handle: Option<Arc<NodeHandle>>,
    pub local_peer_id: Option<Vec<u8>>,
    pub obstacle_inflation_radius: f32,
    /// 上次 pose 的单调毫秒时间戳（**0 = 从未收到**）；与 `PoseIngest.last_ms` 是同一个 `Arc`
    pub pose_age: Arc<AtomicU64>,
    /// 位姿短历史：按**扫描帧的 world_time** 取当时的位姿（否则转向时的位姿-扫描
    /// 时间差会把命中点转偏 `r·cosθ·Δyaw`，实测 r=4 m 掠射点偏 0.10 m）
    pub pose_history: Arc<PoseHistory>,
    /// 本传感器的外参（`[sim_world.sensor.mount]`，世界坐标：x/y 水平、yaw 弧度）
    ///
    /// 世界侧 `Lidar2D.ScanOnce()` 从**挂载点**起算 `global_position` 与 `global_basis`，
    /// 而 `RobotState` 只有车体原点 —— 不做这次换算，建图会把每个命中点沿车头方向平移
    /// `mount.x`（本用例 0.1 m = 20% 格宽），墙面因此被写偏一格并在格边界上反复翻面
    /// （task_31 Step 7 实测：S3 车在墙前永久冻结）。
    pub mount: MountOut,
}

/// 拉起建图 task（200 ms；`cancel` 由 `LidarSensor` 持有，`Shutdown` 时 cancel）
pub fn Spawn_Mapping(ctx: MappingCtx, lidar: Arc<LidarSensor>, cancel: CancellationToken) {
    tokio::spawn(async move { Mapping_Loop(ctx, lidar, cancel).await });
}

/// 建图主循环（帧率无关：用 `interval`，不按帧计数驱动角度）
async fn Mapping_Loop(ctx: MappingCtx, lidar: Arc<LidarSensor>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_millis(MAPPING_INTERVAL_MS));
    // 卡顿后不补跑（§五.7-7：帧率无关）
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    // 跨帧聚合缓冲（Δ≠0 才广播）+ 发送节流（模 5 = 1 s 一次）
    let mut frame_count: u32 = 0;
    let mut pending: HashMap<(i32, i32), i8> = HashMap::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                debug!("[mapping] slot={} 建图 task 退出", lidar.Slot());
                return;
            }
            _ = interval.tick() => {}
        }

        // ---- 1) 位姿快照：读完立刻 drop 读锁（R3）
        //      加上外参 → **传感器位姿**（世界侧雷达从挂载点起算；见 `MappingCtx::mount`）
        let now_pose = {
            let rs = ctx.robot_state.read().await;
            Sensor_Pose(rs.x, rs.y, rs.attitude.yaw, &ctx.mount)
        };
        let mut pose = now_pose;

        // ---- 2) 位姿陈旧 → 本周期跳过（未装配/世界未起天然走到这里 = 装配门闩）
        let age = Age_Of(&ctx.pose_age);
        if age > POSE_STALE_MS {
            Warn_Throttled("mapping.stale_pose", 5000, || {
                format!(
                    "[mapping] slot={} 位姿陈旧（age={} > {} ms），本周期跳过建图",
                    lidar.Slot(),
                    Age_Text(age),
                    POSE_STALE_MS
                )
            });
        } else if let Some(scan) = lidar.Get_Scan().await {
            // ---- 3) 最新一圈（拿不到就只跳过本周期建图，广播节奏不变）
            //      位姿按**扫描时刻**取：历史里找不到（刚起/刚重连）才退回"当前位姿"
            if let Some(sample) = ctx.pose_history.At(scan.stamp as f64 / 1e9, POSE_AT_SCAN_TOL_S) {
                pose = Sensor_Pose(sample.x, sample.y, sample.yaw, &ctx.mount);
            }
            let scan_points: Vec<(f32, f32)> = scan
                .points
                .iter()
                .map(|p| (p.angle, p.range))
                .collect();

            if !scan_points.is_empty() {
                // ---- 4) 锁外算 masked（避免持 grid 写锁时再拿 cluster 锁）
                let masked = {
                    let others = ctx.cluster_table.snapshot().await;
                    cluster_to_obstacle_cells(&others, ctx.obstacle_inflation_radius)
                };

                // ---- 5) 写 grid：写锁只裹住同步的 mapper::Update（块内无 await）
                {
                    let mut g = ctx.grid.write().await;
                    let deltas = mapper::Update(&mut *g, &pose, &scan_points, &masked);
                    accumulate_pending(&mut pending, &deltas);
                }
            }
        }

        // ---- 6) 每 5 帧（1 s）聚合广播一次
        frame_count = frame_count.wrapping_add(1);
        if frame_count % BROADCAST_EVERY == 0 {
            let out = drain_pending(&mut pending);
            if !out.is_empty() {
                // 接收方可能一个都没有（无订阅者时 send 返回 Err）：与真机一致忽略（R20）
                let _ = ctx.map_tx.send(out.clone());

                if let (Some(nh), Some(peer_id)) = (&ctx.node_handle, &ctx.local_peer_id) {
                    let entries: Vec<MapDeltaEntry> = out
                        .iter()
                        .map(|d| MapDeltaEntry { gx: d.gx, gy: d.gy, delta: d.delta })
                        .collect();
                    let payload = encode_map_delta(now_boot_ms(), &entries);
                    let frame = encode_frame(MSGID_MAP_DELTA, peer_id, COMPID_ROBOT, &payload);
                    if let Err(e) = nh.Gossipsub_Publish(TOPIC_ROBOT_MAP, frame).await {
                        warn!("[mapping] 地图增量广播失败：{e}");
                    }
                }
            }
        }
    }
}

/// 车体位姿 + 外参 → **传感器位姿**（水平面：世界坐标 Z-up，yaw 绕 +z）
///
/// `mount` = `[sim_world.sensor.mount]`（世界侧 `Vehicle.SetSensorMount` 用的同一份数值）。
/// 只做水平二维换算（建图是 2D 的，`mount.z` 不参与）。
///
/// `pub(crate)`：急停（`sim::emergency_stop`）复用**同一个**换算，避免第二份外参公式 ——
/// 这是「急停标的格 = 建图标出的格」的前提（Step 7 第 3 轮，偏差 D8）。
pub(crate) fn Sensor_Pose(x: f32, y: f32, yaw: f32, mount: &MountOut) -> RobotPose {
    let (c, s) = (yaw.cos(), yaw.sin());
    let mx = mount.x;
    let my = mount.y;
    RobotPose {
        x: x + mx * c - my * s,
        y: y + mx * s + my * c,
        yaw: yaw + mount.yaw,
    }
}

/// 日志用：`u64::MAX` 表示"从未收到 pose"
fn Age_Text(age: u64) -> String {
    if age == u64::MAX {
        "从未收到".to_string()
    } else {
        format!("{age} ms")
    }
}

/// 聚合纯函数：把一帧的 Δ 累加进 pending（**真实差分，不做 ±8 clamp**）
///
/// 窗口（5 帧）内净变化上界 ±16（own 从 −8 冲到 +8），在 i8 内；每 5 帧必 drain，不会无限增长。
/// 接收方应用时再 clamp ±8，完成精确重放。
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

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::Ordering;

    use pleiades_base::robot::core::grid::CellState;

    use crate::sim::chassis::pose_ingest::{PoseFields, PoseIngest};
    use crate::sim::sensors::lidar::frame::Test_Frame;
    use crate::sim::sensors::lidar::LidarParams;

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
    fn test_accumulate_exact_differential() {
        // 回归锁定：own 从 −8 连续命中 5 帧 → 窗口净变化 +15
        // 必须保留真实差分（不能被 clamp 成 +8），接收方才能精确重放
        let mut pending = HashMap::new();
        let deltas: Vec<Delta> = (0..5).map(|_| Delta { gx: 9, gy: 9, delta: 3 }).collect();
        accumulate_pending(&mut pending, &deltas);
        let out = drain_pending(&mut pending);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].gx, 9);
        assert_eq!(out[0].gy, 9);
        assert_eq!(out[0].delta, 15, "窗口净变化必须精确到 +15（不 clamp）");
    }

    #[test]
    fn test_accumulate_net_zero_filtered() {
        // +3 与 −3 抵消 → 净 0 → 不广播
        let mut pending = HashMap::new();
        accumulate_pending(&mut pending, &[Delta { gx: 5, gy: 5, delta: 3 }]);
        accumulate_pending(&mut pending, &[Delta { gx: 5, gy: 5, delta: -3 }]);
        let out = drain_pending(&mut pending);
        assert!(out.is_empty(), "净变化为 0 的格子不应广播");
        assert!(pending.is_empty());
    }

    // ─── Sensor_Pose：车体位姿 + 外参 → 传感器位姿（task_31 Step 7 实测修复）───

    #[test]
    fn test_sensor_pose_applies_mount_offset() {
        use std::f32::consts::FRAC_PI_2;
        let mount = MountOut { x: 0.1, y: 0.0, z: 0.2, yaw: 0.0 };
        // 朝东（yaw=0）：挂载点 0.1 m 在车头方向 → x +0.1
        let p = Sensor_Pose(1.0, 2.0, 0.0, &mount);
        assert!((p.x - 1.1).abs() < 1e-6, "{p:?}");
        assert!((p.y - 2.0).abs() < 1e-6, "{p:?}");
        // 朝北（yaw=π/2）：车头方向 +y → y +0.1（而不是 x）
        let p = Sensor_Pose(1.0, 2.0, FRAC_PI_2, &mount);
        assert!((p.x - 1.0).abs() < 1e-6, "{p:?}");
        assert!((p.y - 2.1).abs() < 1e-6, "{p:?}");
        // mount.yaw 直接叠加
        let yawed = MountOut { x: 0.0, y: 0.0, z: 0.0, yaw: 0.25 };
        assert!((Sensor_Pose(0.0, 0.0, 0.5, &yawed).yaw - 0.75).abs() < 1e-6);
        // 无外参 = 车体位姿原样
        let zero = MountOut { x: 0.0, y: 0.0, z: 0.0, yaw: 0.0 };
        let p = Sensor_Pose(3.0, 4.0, 0.7, &zero);
        assert_eq!((p.x, p.y, p.yaw), (3.0, 4.0, 0.7));
    }

    #[test]
    fn test_age_text() {
        assert_eq!(Age_Text(u64::MAX), "从未收到");
        assert_eq!(Age_Text(1200), "1200 ms");
    }

    fn test_ctx(
        grid: Arc<RwLock<OccupancyGrid>>,
        state: Arc<RwLock<RobotState>>,
        pose_age: Arc<AtomicU64>,
        map_tx: broadcast::Sender<Vec<MapDelta>>,
    ) -> MappingCtx {
        MappingCtx {
            grid,
            robot_state: state,
            cluster_table: Arc::new(ClusterInfoTable::new()),
            map_tx,
            node_handle: None,
            local_peer_id: None,
            obstacle_inflation_radius: 0.2,
            pose_age,
            pose_history: crate::sim::chassis::pose_ingest::PoseHistory::New(),
            mount: MountOut { x: 0.0, y: 0.0, z: 0.0, yaw: 0.0 },
        }
    }

    #[tokio::test]
    async fn test_mapping_loop_writes_grid_and_broadcasts() {
        // 离线闭环（R1）：新鲜位姿 + 一帧正前方 1 m 的点 → ~1 s 内 map_tx 收到广播、
        // grid 端点格 (130,128) == Occupied
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let state = Arc::new(RwLock::new(RobotState::default()));
        let pose = PoseIngest::New(state.clone());
        pose.On_Pose(&PoseFields { world_time: 1.0, x: 64.0, y: 64.0, z: 0.0, yaw: 0.0, vx: 0.0, vy: 0.0 })
            .await;

        let (map_tx, _) = broadcast::channel::<Vec<MapDelta>>(8);
        let mut map_rx = map_tx.subscribe();

        let lidar = Arc::new(LidarSensor::New("lidar_front", LidarParams::default()));
        // 360 点里只有第 0 点有回波（1 m 正前方），其余 = 0.0（无回波，mapper 过滤）
        let mut ranges = vec![0.0f32; 360];
        ranges[0] = 1.0;
        lidar.On_Binary(&Test_Frame(1, 1, 0.5, &ranges));
        assert!(lidar.Get_Scan().await.is_some(), "应已有一圈扫描");

        let ctx = test_ctx(grid.clone(), state, pose.Age_Handle(), map_tx);
        let cancel = CancellationToken::new();
        Spawn_Mapping(ctx, lidar.clone(), cancel.clone());

        let out = tokio::time::timeout(Duration::from_secs(3), map_rx.recv())
            .await
            .expect("~1 s 内应有 map_tx 广播")
            .expect("broadcast 通道应活着");
        assert!(
            out.iter().any(|d| d.gx == 130 && d.gy == 128 && d.delta > 0),
            "广播里应含端点格 (130,128) 的正 Δ：{out:?}"
        );

        // 5 帧后端点格已 3 次以上命中 → Occupied
        assert_eq!(
            grid.read().await.state(130, 128),
            Some(CellState::Occupied as u8),
            "端点格应被建图写成 Occupied"
        );

        cancel.cancel();
        // 给 task 一个 tick 窗口退出（不 join，只是不给后续用例留噪音）
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    #[tokio::test]
    async fn test_mapping_loop_skips_when_pose_stale() {
        // 从没收到过 pose → Age = MAX → 每周期跳过；不写 grid、不广播
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (map_tx, _) = broadcast::channel::<Vec<MapDelta>>(8);
        let mut map_rx = map_tx.subscribe();
        let pose_age = Arc::new(AtomicU64::new(0));

        let lidar = Arc::new(LidarSensor::New("lidar_front", LidarParams::default()));
        let mut ranges = vec![0.0f32; 360];
        ranges[0] = 1.0;
        lidar.On_Binary(&Test_Frame(1, 1, 0.5, &ranges));

        let ctx = test_ctx(grid.clone(), state, pose_age.clone(), map_tx);
        let cancel = CancellationToken::new();
        Spawn_Mapping(ctx, lidar.clone(), cancel.clone());

        // 1.3 s（> 5 帧）内不应有任何广播
        let got = tokio::time::timeout(Duration::from_millis(1300), map_rx.recv()).await;
        assert!(got.is_err(), "位姿陈旧时不应广播地图增量");
        assert_eq!(grid.read().await.state(130, 128), Some(CellState::Unknown as u8));
        assert_eq!(pose_age.load(Ordering::Relaxed), 0);

        cancel.cancel();
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
