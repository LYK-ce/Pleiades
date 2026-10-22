//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 前方障碍急停（task_31 §八.3）：与 `pleiades-ugv/src/ugv/emergency_stop.rs` **同语义**，
//! 代码独立、**不引用 ugv**（§五.2-1）。真机的 `stm32` 在本 crate 对应 `chassis: &dyn Chassis`。
//!
//! 判定（与真机一字不差）：
//! 1. 无 scan → `false`
//! 2. 前方 ±45° 扇区（`a < π/4 || a >= 7π/4`，先把 `angle` 归一到 `[0,2π)`）内
//!    取得最近的有效回波（`range >= 0.1`；世界侧无回波是 `0.0`，正好被这条过滤掉）
//! 3. 最近点 `range < 0.3` → `chassis.Stop()` + `goal_service.mark_obstacle(格)` + `true`
//!
//! **关键理解（决定实现对不对，task_31 §八.3）**：`mark_obstacle` 只是促使 D* 重算受影响邻居，
//! 本身**不改地图**；真正让 D* 改路的是 mapper 把那格写进 `grid` 并累积到 `Occupied`
//! （log-odds 命中 `+3`、`> +6` 才算占，约 3 帧 ≈ 0.3 s）。所以急停必须**每 tick 复核**
//! （本模块不做任何锁存），且**不造**「感知直连规划」的旁路。
//!
//! 日志限流 ≠ 判定限流（§八.7-3）：[`Warn_Throttled`] 只节流**日志文本**，
//! `Stop()` 与 `mark_obstacle` **每次调用都执行**。

use std::f32::consts::PI;
use std::sync::Arc;

use tokio::sync::Mutex;

use pleiades_base::robot::core::grid::CELL_RESOLUTION;
use pleiades_base::robot::core::state::RobotState;

use crate::sim::chassis::Chassis;
use crate::sim::goal::GoalService;
use crate::sim::sensors::lidar::mapper::HIT_EPS_M;
use crate::sim::sensors::lidar::mapping_task::Sensor_Pose;
use crate::sim::sensors::lidar::types::LaserScan;
use crate::sim::sensors::Warn_Throttled;
use crate::world_link::protocol::MountOut;

/// 前方障碍距离阈值（米）—— 真机同值（`pleiades-ugv/src/ugv/emergency_stop.rs:23`）
pub const OBSTACLE_THRESHOLD_M: f32 = 0.3;

/// 有效回波下限（米）：世界 `range == 0.0` 表示无回波，正好被这条过滤掉
pub const MIN_VALID_RANGE_M: f32 = 0.1;

/// 前方扇区左边界：`a < FRONT_HALF_ANGLE_RAD`（即 `π/4`）
pub const FRONT_HALF_ANGLE_RAD: f32 = PI / 4.0;

/// 前方扇区右边界：`a >= FRONT_SECTOR_START_RAD`（即 `7π/4`）
pub const FRONT_SECTOR_START_RAD: f32 = 7.0 * PI / 4.0;

/// 障碍点 → 网格坐标（与 mapper **同一套换算、同一** [`CELL_RESOLUTION`]、同一 [`HIT_EPS_M`]）
///
/// ## 为什么必须补外参（`mount`）
///
/// 世界侧 `Lidar2D.ScanOnce()` 的射线从**挂载点**起算，而 `rs.x/rs.y` 是**车体原点**。
/// 只用车体原点算命中点，每个障碍格会沿车头方向偏移 `mount.x`（本用例 0.1 m = 20% 格宽），
/// 于是落在**墙前那一格**（S3b 实测 `(100,183)`，而 wall_c 本体格是 `(100,184)`）。
/// 急停标的格与建图标出的格必须**同格**，否则 D\* 修补的是一条不存在的边。
///
/// ## 为什么必须外推 [`HIT_EPS_M`]（2 cm）
///
/// 墙面恰好落在格边界（`wall_c` 南面 `y=92.0` = 格 184 的下边界），命中点受浮点误差影响
/// 可能略小于 92.0，`floor` 会把它算进墙前格 183。外推量与建图那边**同值同式**，
/// 保证「急停标的格 = 建图标出的格」。
///
/// ## 偏差 D8（**未获人类批准的设计变更**，登记见 `Workbook/wb_31_sim_world_link.md` §偏差登记表）
///
/// §八.3 只写「车位置 + 回波」算格，没写外参与外推。补这两项是 Step 7 第 3 轮 prev#8 的修复；
/// 回退方法 = 把本函数退回 `yaw + angle` + `range` 的旧式（会让 B2 判据 FAIL，不许放松判据）。
pub fn Obstacle_Cell(
    x: f32,
    y: f32,
    yaw: f32,
    mount: &MountOut,
    angle: f32,
    range: f32,
) -> (i32, i32) {
    // 与 `mapping_task::Sensor_Pose` 同一个换算（车体 → 传感器挂载点）
    let p = Sensor_Pose(x, y, yaw, mount);
    let a = p.yaw + angle;
    // 与 `mapper::Update` 同式：命中点沿射线外推 HIT_EPS_M 再量化
    let wx = p.x + (range + HIT_EPS_M) * a.cos();
    let wy = p.y + (range + HIT_EPS_M) * a.sin();
    ((wx / CELL_RESOLUTION).floor() as i32, (wy / CELL_RESOLUTION).floor() as i32)
}

/// 前方障碍急停检查（真机 `check_emergency_stop` 的同形实现）
///
/// 返回 `true` = 已触发（调用方必须：`Stop` + `ExecuteState = Idle` + **保留 goal** + return）
///
/// 参数形态与真机一致（`lidar.get_scan()` / `robot_state` / `stm32` / `&goal_service`），
/// 只是「雷达」按 task_31 §八.3 的签名换成**已经取出的** `Option<LaserScan>`，
/// 「底盘」用 [`Chassis`] trait 对象（真机是 `STM32Device`，§八.3 里写作 `chunk_move`）。
/// 第 5 个参数 `mount` 是传感器外参（[`LidarSensor::Mount`](crate::sim::sensors::lidar::LidarSensor::Mount)），
/// 只用于把回波换算成障碍格（[`Obstacle_Cell`]，偏差 D8）；判定阈值与四步流程不受它影响。
pub async fn Check_Emergency_Stop(
    scan: Option<LaserScan>,
    rs: &RobotState,
    chassis: &dyn Chassis,
    goal_service: &Arc<Mutex<GoalService>>,
    mount: &MountOut,
) -> bool {
    // ① 无 scan（无雷达 / 还没收到帧）→ 不急停（真机 `let Some(scan) = ... else { return false }`）
    let Some(scan) = scan else {
        return false;
    };

    // ② 前方 ±45° 扇区内的最近有效点（`range >= 0.1` 过滤无回波）
    let closest = scan
        .points
        .iter()
        .filter(|p| {
            let a = Normalize_Angle(p.angle);
            p.range >= MIN_VALID_RANGE_M && (a < FRONT_HALF_ANGLE_RAD || a >= FRONT_SECTOR_START_RAD)
        })
        .min_by(|a, b| a.range.partial_cmp(&b.range).unwrap_or(std::cmp::Ordering::Equal));
    let Some(p) = closest else {
        return false;
    };

    // ③ 阈值 0.3 m 未到 → 不急停
    if p.range >= OBSTACLE_THRESHOLD_M {
        return false;
    }

    // ④ 触发：日志（1 s 一条，**只节流日志**）→ 停车 → 标记障碍格
    //    格 = 车体位姿 + 外参 + 回波（与建图同一套换算/外推，偏差 D8）
    let cell = Obstacle_Cell(rs.x, rs.y, rs.attitude.yaw, mount, p.angle, p.range);
    Warn_Throttled("sim.emergency_stop", 1000, || {
        format!(
            "[Sim] 前方障碍 {:.2}m < {:.2}m，急停；mark_obstacle 格=({},{})",
            p.range, OBSTACLE_THRESHOLD_M, cell.0, cell.1
        )
    });
    // 真机 `stm32.stop()`；sim 走 `Chassis::Stop` → `Send_Stop_Priority`（G1，不静默丢）
    chassis.Stop();
    // 促 D* 局部修补（**不改地图**：真正改路靠 mapper 把该格推成 Occupied）
    goal_service.lock().await.mark_obstacle(cell).await;
    true
}

/// 角度归一到 `[0, 2π)`（与真机一字不差：只处理负角）
///
/// 世界侧 `angle = TAU * i / n ∈ [0, 2π)`，不会出现 `>= 2π`，因此**不加**取模分支。
#[inline]
fn Normalize_Angle(angle: f32) -> f32 {
    if angle < 0.0 {
        angle + 2.0 * PI
    } else {
        angle
    }
}

// ================================================================ 单测（硬验收 2：7 条要求 → 9 个用例）

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use async_trait::async_trait;
    use tokio::sync::RwLock;

    use pleiades_base::robot::core::cluster::ClusterInfoTable;
    use pleiades_base::robot::core::command::ManualCmd;
    use pleiades_base::robot::core::grid::{world_to_grid, CellState, OccupancyGrid};
    use pleiades_base::robot::core::mission::MissionQueue;
    use pleiades_base::robot::core::state::MotionAction;

    use crate::sim::chassis::{PoseFields, SpawnFields};
    use crate::sim::goal::ARRIVAL_THRESHOLD_M;
    use crate::sim::sensors::lidar::mapper;
    use crate::sim::sensors::lidar::types::LaserPoint;

    /// 假底盘：只数 `Stop()` 次数（其余空实现）
    struct MockChassis {
        stops: Arc<AtomicUsize>,
    }

    impl MockChassis {
        fn new() -> Self {
            Self { stops: Arc::new(AtomicUsize::new(0)) }
        }
        fn stop_count(&self) -> usize {
            self.stops.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl Chassis for MockChassis {
        fn Name(&self) -> &str {
            "mock_chassis"
        }
        fn Apply_Action(&self, _action: MotionAction) {}
        fn Handle_Manual_Cmd(&self, _cmd: &ManualCmd) {}
        async fn On_Pose(&self, _p: &PoseFields) {}
        async fn On_Spawn(&self, _s: &SpawnFields) {}
        fn Pose_Age_Ms(&self) -> u64 {
            0
        }
        fn Pose_Age_Handle(&self) -> Arc<AtomicU64> {
            Arc::new(AtomicU64::new(0))
        }
        fn Pose_History(&self) -> Arc<crate::sim::chassis::pose_ingest::PoseHistory> {
            crate::sim::chassis::pose_ingest::PoseHistory::New()
        }
        fn Stop(&self) {
            self.stops.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 无 pathfinder 的 GoalService（`mark_obstacle` 为空操作，副作用不由单测断言，§八 R6）
    fn test_goal_service() -> Arc<Mutex<GoalService>> {
        Arc::new(Mutex::new(GoalService::new(
            Arc::new(RwLock::new(RobotState::default())),
            Arc::new(RwLock::new(MissionQueue::default())),
            b"self".to_vec(),
            Arc::new(RwLock::new(OccupancyGrid::new())),
            Arc::new(ClusterInfoTable::new()),
            0.2,
            ARRIVAL_THRESHOLD_M,
        )))
    }

    /// 一圈扫描（`intensity` 世界帧不上线 → 恒 0.0）
    fn scan_of(points: &[(f32, f32)]) -> LaserScan {
        LaserScan {
            points: points
                .iter()
                .map(|(angle, range)| LaserPoint { angle: *angle, range: *range, intensity: 0.0 })
                .collect(),
            stamp: 0,
            scan_freq: 10.0,
            scan_time: 0.1,
        }
    }

    /// 车在 (64, 64)、yaw = 0（障碍格示例即来自这个位姿）
    fn robot_at_64() -> RobotState {
        RobotState { x: 64.0, y: 64.0, ..Default::default() }
    }

    /// 用例里的传感器外参（= §九.2 的 `[sim_world.sensor.mount]`）
    fn mount_02() -> MountOut {
        MountOut { x: 0.1, y: 0.0, z: 0.2, yaw: 0.0 }
    }

    /// 零外参（判定阈值/扇区类用例只关心「触不触发」，与格换算无关）
    async fn check(scan: Option<LaserScan>, rs: &RobotState, chassis: &MockChassis) -> bool {
        check_with(scan, rs, chassis, &MountOut::default()).await
    }

    async fn check_with(
        scan: Option<LaserScan>,
        rs: &RobotState,
        chassis: &MockChassis,
        mount: &MountOut,
    ) -> bool {
        let gs = test_goal_service();
        Check_Emergency_Stop(scan, rs, chassis, &gs, mount).await
    }

    // 1) 前方 0.2 m → true
    #[tokio::test]
    async fn test_emergency_stop_front_02m_triggers() {
        let ch = MockChassis::new();
        assert!(check(Some(scan_of(&[(0.0, 0.2)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 1, "触发必须调 Stop() 一次");
    }

    // 2) 前方 0.5 m → false（阈值 0.3）
    #[tokio::test]
    async fn test_emergency_stop_front_05m_no_trigger() {
        let ch = MockChassis::new();
        assert!(!check(Some(scan_of(&[(0.0, 0.5)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 0);
    }

    // 3) 侧方 90° 0.2 m → false（±45° 外）
    #[tokio::test]
    async fn test_emergency_stop_side_90deg_no_trigger() {
        let ch = MockChassis::new();
        assert!(!check(Some(scan_of(&[(PI / 2.0, 0.2)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 0);
    }

    // 4) 全 0.0 无回波 → false（`range >= 0.1` 过滤）
    #[tokio::test]
    async fn test_emergency_stop_all_no_echo() {
        let ch = MockChassis::new();
        let scan = scan_of(&[
            (0.0, 0.0),
            (-PI / 2.0, 0.0),
            (PI / 2.0, 0.0),
            (3.0 * PI / 2.0, 0.0),
            (PI, 0.0),
        ]);
        assert!(!check(Some(scan), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 0);
    }

    // 5) scan = None → false
    #[tokio::test]
    async fn test_emergency_stop_none_scan() {
        let ch = MockChassis::new();
        assert!(!check(None, &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 0);
    }

    // 6) 扇区边界：π/4 − ε 触发 / π/4 + ε 不触发 / 7π/4 触发
    #[tokio::test]
    async fn test_emergency_stop_sector_edges() {
        // ε 远大于 f32 精度、又远小于扇区宽度（§八 R7）
        const EPS: f32 = 1e-3;
        // 边界用与实现**同一表达式**生成，避免 f32 舍入抖动
        assert_eq!(FRONT_HALF_ANGLE_RAD, std::f32::consts::PI / 4.0);
        assert_eq!(FRONT_SECTOR_START_RAD, 7.0 * std::f32::consts::PI / 4.0);

        let ch = MockChassis::new();
        // π/4 − ε：还在前方扇区内 → true
        assert!(check(Some(scan_of(&[(FRONT_HALF_ANGLE_RAD - EPS, 0.2)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 1);
        // π/4 + ε：已出扇区 → false
        assert!(!check(Some(scan_of(&[(FRONT_HALF_ANGLE_RAD + EPS, 0.2)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 1, "未触发不该再 Stop()");
        // 7π/4：扇区右边界（闭区间）→ true
        assert!(check(Some(scan_of(&[(FRONT_SECTOR_START_RAD, 0.2)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 2);
    }

    // 7) 障碍格坐标：x=64,y=64,yaw=0,mount.x=0.1,angle=0,range=0.2 → (128,128)
    #[test]
    fn test_obstacle_cell_doc_example() {
        // mount.x=0.1 → 传感器在 64.1；再外推 HIT_EPS_M=0.02 → 64.32（/0.5 = 128.64 → 128）
        let cell = Obstacle_Cell(64.0, 64.0, 0.0, &mount_02(), 0.0, 0.2);
        assert_eq!(cell, (128, 128), "64.32 / 0.5 = 128.64 → floor 128");
        // 与 mapper 同公式（同一个 CELL_RESOLUTION + floor + HIT_EPS_M）
        assert_eq!(cell, world_to_grid(64.0 + 0.1 + 0.2 + HIT_EPS_M, 64.0));
        // yaw 参与：yaw=π 时传感器在 63.9、障碍落在 −X 方向（63.9 − 0.22 → 63.68 → 格 127）
        let cell = Obstacle_Cell(64.0, 64.0, PI, &mount_02(), 0.0, 0.2);
        assert_eq!(cell, (127, 128));
        assert_eq!(cell, world_to_grid(64.0 - 0.1 - 0.2 - HIT_EPS_M, 64.0));
    }

    // 10) 外参 + 外推的实证用例（prev#8）：墙前 0.27 m、朝北、mount.x=0.1 → wall_c 本体格 (100,184)
    #[test]
    fn test_obstacle_cell_mount_and_hit_eps() {
        // S3b 实测：车 y=91.630、range=0.27、yaw≈π/2、mount.x=0.1
        let cell = Obstacle_Cell(50.0, 91.63, PI / 2.0, &mount_02(), 0.0, 0.27);
        // 旧式（无外参、无外推）：91.63 + 0.27 = 91.90 → 格 183（墙前自由格）—— 这就是 prev#8 的缺陷
        assert_eq!(world_to_grid(50.0, 91.63 + 0.27), (100, 183), "旧式确实落在 183");
        // 新式：91.73 + 0.29 = 92.02 → 格 184（wall_c 本体）
        assert_eq!(cell, (100, 184), "新式应落在 wall_c 本体格");

        // 与 mapper 同格：手工跑 mapper::Update（同一位姿 + 同一单点扫描），三次命中后该格 Occupied
        let mut grid = OccupancyGrid::new();
        let pose = Sensor_Pose(50.0, 91.63, PI / 2.0, &mount_02());
        for _ in 0..3 {
            mapper::Update(&mut grid, &pose, &[(0.0, 0.27)], &HashSet::new());
        }
        assert_eq!(
            grid.state(cell.0, cell.1),
            Some(CellState::Occupied as u8),
            "急停标的格必须与建图标出的格相同（{cell:?}）"
        );
    }

    // 11) 零外参 + 常规距离：新旧式只差 HIT_EPS_M，锁住「外推不会跨格」（边界命中除外，见末尾）
    #[test]
    fn test_obstacle_cell_zero_mount_matches_old_formula() {
        let m = MountOut::default();
        for (x, y, yaw, angle, range) in [
            (64.0f32, 64.0f32, 0.0f32, 0.0f32, 1.0f32),
            (64.0, 64.0, 0.0, 0.0, 0.2),
            (64.0, 64.0, PI, 0.0, 0.3),
            (50.0, 91.63, PI / 2.0, 0.0, 0.27),
        ] {
            // 旧式（无外参、无外推）
            let a = yaw + angle;
            let old = ((x + range * a.cos()) / CELL_RESOLUTION).floor() as i32;
            let oldy = ((y + range * a.sin()) / CELL_RESOLUTION).floor() as i32;
            let new = Obstacle_Cell(x, y, yaw, &m, angle, range);
            assert_eq!(new, (old, oldy), "零外参下外推 2 cm 不应跨格：{new:?} vs ({old},{oldy})");
        }
        // 外推量确实是 HIT_EPS_M（同式、非零）
        assert!(HIT_EPS_M > 0.0);

        // 唯一的例外：命中点**恰好落在格边界**（x=63.5 = 格 126/127 的交界）时，旧式留在边界内侧那一格、
        // 新式被推进被击中的体素（126）—— 这正是外推的目的（D4：墙在边界上时稳定在墙格）
        assert_eq!(Obstacle_Cell(64.0, 64.0, PI, &m, 0.0, 0.5), (126, 128));
        assert_eq!(world_to_grid(63.5, 64.0), (127, 128));
    }

    // 8) 触发 / 未触发各自的 Stop 计数（日志限流不影响动作，§八 R5）
    #[tokio::test]
    async fn test_emergency_stop_stops_chassis() {
        let ch = MockChassis::new();
        assert!(check(Some(scan_of(&[(0.0, 0.2)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 1, "触发 → Stop() 恰好一次");
        assert!(!check(Some(scan_of(&[(0.0, 0.5)])), &robot_at_64(), &ch).await);
        assert_eq!(ch.stop_count(), 1, "未触发 → 不增加");
    }

    // 9) 每 tick 复核：同一 scan 连续两次都 true、Stop 计数 2（**无锁存**，§八.6-10）
    #[tokio::test]
    async fn test_emergency_stop_rechecks_every_call() {
        let ch = MockChassis::new();
        let gs = test_goal_service();
        let rs = robot_at_64();
        let scan = Some(scan_of(&[(0.0, 0.2)]));

        assert!(Check_Emergency_Stop(scan.clone(), &rs, &ch, &gs, &MountOut::default()).await);
        assert!(Check_Emergency_Stop(scan, &rs, &ch, &gs, &MountOut::default()).await);
        assert_eq!(ch.stop_count(), 2, "第 2 次调用必须再停一次（不能锁存）");
    }
}
