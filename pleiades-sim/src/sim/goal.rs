//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 目标服务（Task 22 步骤 5，Task 22_3 改造：get_path 移入 main_loop，不再维护 sub_target）
//!
//! `get_path()` = 完整目标服务：内部封装「到达检测 + 任务切换（pop + 群发分配 + 设 goal 进 D*）+ 寻路」。
//! 只返回「下一步格」或 None（无任务/到达/不可达）；sub_target 维护移入 `ExecuteState`（main_loop 单点写）。
//! 急停（前方障碍强制 stop）在 Rust 侧 main_loop，不在此处。
//!
//! Task 23 决策 D1：寻路（D* Lite）下沉设备端，由本服务直接持有（不再经 world）。

use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

use pleiades_base::robot::core::cluster::ClusterInfoTable;
use pleiades_base::robot::core::command::Mission;
use pleiades_base::robot::core::mission::MissionQueue;
use crate::sim::planning::pathfinder::DStarLite;
use crate::sim::planning::{assignment, cluster_to_obstacle_cells};
use pleiades_base::robot::core::state::RobotState;
use pleiades_base::robot::core::grid::{OccupancyGrid, CELL_RESOLUTION};
use pleiades_base::robot::core::map_delta::MapDelta;

/// 到达判定阈值（米，逐轴）—— §九.3 的通过判据就是逐轴 ≤ 0.3
pub const ARRIVAL_THRESHOLD_M: f32 = 0.3;

/// 到达判定的**逐轴**容差（米）：`CELL_RESOLUTION/2 + 0.01 = 0.26`
///
/// §九.3 的通过判据是**停稳后**逐轴 ≤ 0.3。取 0.26 的理由有两条：
/// 1. 目标点可能落在格**角**上，此时格心离目标点 0.25 m —— 容差小于 0.25 会让车永远判不了到达；
/// 2. 判定用的是**预测停点**（见 [`Predict_Stop`]），不是当前位置：把刹停滑行算进去后，
///    实际停点与目标的偏差 ≈ 容差 ± 预测误差，仍 ≤ 0.3。
pub const ARRIVAL_TOL_M: f32 = CELL_RESOLUTION / 2.0 + 0.01;

/// **预测停点**用的逐轴容差（米）：0.06
///
/// 判的是「预测停点」离目标多远，所以要**尽量小**：触发点 ≈ 目标 − 预测滑行量，
/// 实际停点 ≈ 目标 + (实际滑行 − 预测滑行)。容差越小，停得越准。
///
/// ## 余量推导（两档速度；数字来自实测，见 `RobotSim_TestEnv/results/conclusion.md`「判据余量表」）
///
/// 滑行量 `coast = v²/(2·BRAKE_DECEL_MPS2) + v·DECIDE_TICK_S`：
/// - **1.2 m/s**（第 2 轮实测档）：`coast = 0.24 + 0.06 = 0.30`；取 `tol = 0.06` =
///   一个 tick 的行程（1.2 × 0.05），保证窗口内至少 2 个 tick 采到 → 触发点 ≈ 目标 − 0.06，
///   实测停点误差 0.25（判据 0.3 → **余量 0.05 m**）。
/// - **§九.2 原值档**（`forward_speed=30` × 世界默认 `max_speed=1.5` = `0.45 m/s`）：
///   `coast = 0.034 + 0.0225 = 0.056`，一个 tick 行程 0.0225 → 窗口 ≈ 2.7 个 tick（仍 ≥2，
///   所以 0.06 在最慢档也够用）；实测余量由 Step 7 第 3 轮的「判据余量表」逐 run 填写。
///
/// 收不放宽的依据：`tol` 只能在窗口不足 2 个 tick 时才允许放大，且必须 ≥ 2×tick 行程；
/// 当前两档都满足，因此**常量值不动**（prev_issues#9② 的处置：记录余量，不静默改参）。
///
/// [`ARRIVAL_TOL_M`]（0.26）仍用于 ③ 分支的兜底（车已在目标格内、按**当前位置**判）。
const ARRIVAL_PREDICT_TOL_M: f32 = 0.06;

/// 制动减速度（m/s²）—— 世界侧 `ChassisModel.MAX_ACCEL_MPS2 = 3.0` 的**同值拷贝**
/// （`pleiades-sim` 不引用世界代码，语义对齐即可）。
const BRAKE_DECEL_MPS2: f32 = 3.0;

/// 一个决策 tick（秒）—— base 的 `auto_tick_ms = 50`
const DECIDE_TICK_S: f32 = 0.05;

/// 预测「此刻发 Stop 之后，车最终停在哪」（task_31 Step 7 第 2 轮实测修复）
///
/// 为什么必须预测：判定到达 → 清 goal → 执行器下一 tick 发 `Stop` → 世界侧按 3 m/s² 减速，
/// 车还会滑行 `v²/(2a) + v·tick`。1.2 m/s 时 ≈ 0.3 m —— 拿**当前位置**判到达，停稳后单轴
/// 误差实测可到 **0.39**（S2 第 2 轮 FAIL：到达点 (40.247, 80.243)，停稳在 x=40.394）。
/// 按预测停点判，则 1.2 m/s 时车会在离目标 ~0.55 m 处开始刹车、停在 0.2~0.26 m 内。
///
/// 速度来自 `RobotState.vx/vy`（世界 `pose` 帧自带，10 Hz）。
fn Predict_Stop(wx: f32, wy: f32, vx: f32, vy: f32) -> (f32, f32) {
    let v = (vx * vx + vy * vy).sqrt();
    if v < 1e-3 {
        return (wx, wy);
    }
    let coast = v * v / (2.0 * BRAKE_DECEL_MPS2) + v * DECIDE_TICK_S;
    (wx + vx / v * coast, wy + vy / v * coast)
}

/// 网格级到达的逐轴容差（米）
///
/// 格心到格内任意点的逐轴距离恒 ≤ `CELL_RESOLUTION/2 = 0.25`，取 `0.26` 留 1 cm 余量：
/// 按本函数判「到达」时，逐轴误差必然 ≤ 0.26 m（§九.3 的判据是逐轴 ≤ 0.3 m）。
/// 旧逻辑「入目标格即到达」在目标点正好落在格边界上时实测停在 `x=40.45`（误差 0.45 m > 0.3）。
fn Grid_Arrived(goal: (f32, f32), wx: f32, wy: f32) -> bool {
    (wx - goal.0).abs() <= ARRIVAL_TOL_M && (wy - goal.1).abs() <= ARRIVAL_TOL_M
}

/// 目标点所在格（网格坐标，与 `world_to_grid` 的 floor 语义一致）
fn Goal_Cell(goal: (f32, f32)) -> (i32, i32) {
    ((goal.0 / CELL_RESOLUTION).floor() as i32, (goal.1 / CELL_RESOLUTION).floor() as i32)
}

/// 当前位置是否已在目标点所在格
fn In_Goal_Cell(goal: (f32, f32), wx: f32, wy: f32) -> bool {
    let (gx, gy) = Goal_Cell(goal);
    gx == (wx / CELL_RESOLUTION).floor() as i32 && gy == (wy / CELL_RESOLUTION).floor() as i32
}

/// 取出建图增量里变化的格（非阻塞；同一批去重）
///
/// 退回 `None`（没订阅）时返回空；`Lagged` 说明 tick 太慢漏了批次，用 `warn!` 提醒
/// 一次（漏掉的格会在后续帧再变时被补上；不重初始化 D*，代价太大）。
fn Drain_Map_Deltas(rx: &mut Option<broadcast::Receiver<Vec<MapDelta>>>) -> Vec<(i32, i32)> {
    let Some(rx) = rx.as_mut() else {
        return Vec::new();
    };
    let mut out: Vec<(i32, i32)> = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(batch) => out.extend(batch.iter().map(|d| (d.gx, d.gy))),
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => break,
            Err(broadcast::error::TryRecvError::Lagged(n)) => {
                warn!("[Goal] 建图增量积压，丢弃 {n} 批（D* 修补可能滞后一帧）");
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// 目标服务：任务管理 + 寻路（Rust 侧「通用任务执行框架」）
pub struct GoalService {
    robot_state: Arc<RwLock<RobotState>>,
    mission_queue: Arc<RwLock<MissionQueue>>,
    own_peer_id: Vec<u8>,
    grid: Arc<RwLock<OccupancyGrid>>,
    cluster_table: Arc<ClusterInfoTable>,
    obstacle_inflation_radius: f32,
    arrival_threshold_m: f32,
    /// 当前 Mission 最终目标（世界坐标，米）
    goal: Option<(f32, f32)>,
    /// D* Lite 路径规划器（决策 D1：下沉设备端，GoalService 直接持有）
    pathfinder: Option<DStarLite>,
    /// 建图增量订阅（mapper → D\* 修补，见 [`DStarLite::Notify_Changed`]）
    ///
    /// `None` = 没接建图（单测/无雷达）：此时 D* 只能靠 `mark_obstacle`（急停）被动修补。
    map_rx: Option<broadcast::Receiver<Vec<MapDelta>>>,
}

impl GoalService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        robot_state: Arc<RwLock<RobotState>>,
        mission_queue: Arc<RwLock<MissionQueue>>,
        own_peer_id: Vec<u8>,
        grid: Arc<RwLock<OccupancyGrid>>,
        cluster_table: Arc<ClusterInfoTable>,
        obstacle_inflation_radius: f32,
        arrival_threshold_m: f32,
    ) -> Self {
        Self {
            robot_state,
            mission_queue,
            own_peer_id,
            grid,
            cluster_table,
            obstacle_inflation_radius,
            arrival_threshold_m,
            goal: None,
            pathfinder: None,
            map_rx: None,
        }
    }

    /// 订阅建图增量（`robot.map_tx`）：D* 的 `g/rhs` 只有被通知才会重新计算
    ///
    /// 单独一个方法而不是 `new` 的参数：不改既有的 7 参签名（单测/急停测试都在用）。
    pub fn Subscribe_Map(&mut self, map_tx: &broadcast::Sender<Vec<MapDelta>>) {
        self.map_rx = Some(map_tx.subscribe());
    }

    /// 完整目标服务：到达检测 + 任务切换 + 寻路，返回下一步格或 None（无任务/到达/不可达）
    pub async fn get_path(&mut self) -> Option<(i32, i32)> {
        let (wx, wy, vx, vy) = {
            let rs = self.robot_state.read().await;
            (rs.x, rs.y, rs.vx, rs.vy)
        };
        // 预测停点：判到达要用「刹停之后停在哪」，而不是「现在在哪」（见 [`Predict_Stop`]）
        let (px, py) = Predict_Stop(wx, wy, vx, vy);

        // ① 到达检测（**预测停点 + 逐轴容差**）
        //    容差 = min(配置阈值, 预测停点容差)：配置阈值（默认 0.3）比几何容差更宽时以几何容差为准
        let tol = self.arrival_threshold_m.min(ARRIVAL_PREDICT_TOL_M);
        if let Some((gx, gy)) = self.goal {
            let arrived = (px - gx).abs() <= tol && (py - gy).abs() <= tol;
            if arrived {
                info!("[Goal] 到达目标 ({:.2}, {:.2})", gx, gy);
                self.goal = None;
                self.pathfinder = None;
                return None;
            }
        }

        // ② 任务切换：无 goal → pop 下一个 Mission + 群发分配 + 设 goal 进 D*
        if self.goal.is_none() {
            let mission = self.mission_queue.write().await.pop_next();
            match mission {
                Some(Mission::Goto { x, y, members }) => {
                    info!("[Goal] 新任务 Goto({:.2}, {:.2})", x, y);
                    let goal = {
                        let g = self.grid.read().await;
                        assignment::group_goto_mission((x, y), &members, &self.own_peer_id, &g)
                    };
                    match goal {
                        Ok(goal) => self.Start_Goal(goal, wx, wy),
                        Err(e) => {
                            warn!("[Goal] 群发任务分配失败: {e:?}，跳过此任务");
                            return None;
                        }
                    }
                }
                Some(Mission::Circle { x, y, members }) => {
                    info!("[Goal] 新任务 Circle({:.2}, {:.2})", x, y);
                    let goal = {
                        let g = self.grid.read().await;
                        assignment::group_circle_mission((x, y), &members, &self.own_peer_id, &g)
                    };
                    match goal {
                        Ok(goal) => self.Start_Goal(goal, wx, wy),
                        Err(e) => {
                            warn!("[Goal] 围圈任务分配失败: {e:?}，跳过此任务");
                            return None;
                        }
                    }
                }
                None => {
                    // 无任务 → 无下一步
                    return None;
                }
            }
        }

        // ③ 网格级到达：入目标格后**必须**再逼近到格心，否则会停在格边界上（逐轴误差可能 > 0.3 m，D2）
        //    - 已入格且逐轴 ≤ 0.26 m → 到达
        //    - 已入格但还差一点 → 把**目标格自身**当下一步，让 executor 朝格心收敛（这次不再走 D*）
        if let Some(goal) = self.goal {
            if In_Goal_Cell(goal, wx, wy) {
                if Grid_Arrived(goal, wx, wy) {
                    info!(
                        "[Goal] 已在目标格且逐轴误差 <= {:.2} m，到达 ({:.2}, {:.2})",
                        ARRIVAL_TOL_M,
                        goal.0,
                        goal.1
                    );
                    self.goal = None;
                    self.pathfinder = None;
                    return None;
                }
                return Some(Goal_Cell(goal));
            }
        }

        // ④ 寻路：move_to 当前位置 → 注入动态障碍 → **应用建图增量修补** → next_step
        //
        // 建图增量必须在 `next_step` 之前喂给 D*：否则画出来的墙对 `g/rhs` 不可见，
        // 规划沿旧值继续走（目标在两格之间翻面、车原地打转，task_31 Step 7 实测）。
        let changed = Drain_Map_Deltas(&mut self.map_rx);
        let dynamic_obstacles: Vec<(i32, i32)> = {
            let others = self.cluster_table.snapshot().await;
            cluster_to_obstacle_cells(&others, self.obstacle_inflation_radius).into_iter().collect()
        };
        let current_gx = (wx / CELL_RESOLUTION).floor() as i32;
        let current_gy = (wy / CELL_RESOLUTION).floor() as i32;
        let grid = self.grid.read().await;
        let next = match self.pathfinder.as_mut() {
            Some(pf) => {
                pf.move_to((current_gx, current_gy));
                pf.set_dynamic_obstacles(&dynamic_obstacles, &grid);
                if !changed.is_empty() {
                    pf.Notify_Changed(&changed, &grid);
                }
                pf.next_step(&grid)
            }
            None => None,
        };
        match next {
            Some(cell) => Some(cell),
            None => {
                warn!("[Goal] D* 不可达，跳过此任务");
                self.goal = None;
                self.pathfinder = None;
                None
            }
        }
    }

    /// 外部切换模式时重置
    pub fn reset(&mut self) {
        self.goal = None;
        self.pathfinder = None;
    }

    /// 急停标记障碍（触发 D* 局部修补；决策 D1：寻路下沉后由 GoalService 提供）
    pub async fn mark_obstacle(&mut self, cell: (i32, i32)) {
        let grid = self.grid.read().await;
        if let Some(pf) = self.pathfinder.as_mut() {
            pf.mark_obstacle(cell, &grid);
        }
    }

    /// 装载新目标：设 goal + 建 D* 路径（Goto / Circle 共用）
    fn Start_Goal(&mut self, goal: (f32, f32), wx: f32, wy: f32) {
        self.goal = Some(goal);
        let start_gx = (wx / CELL_RESOLUTION).floor() as i32;
        let start_gy = (wy / CELL_RESOLUTION).floor() as i32;
        let goal_gx = (goal.0 / CELL_RESOLUTION).floor() as i32;
        let goal_gy = (goal.1 / CELL_RESOLUTION).floor() as i32;
        self.pathfinder = Some(DStarLite::new((start_gx, start_gy), (goal_gx, goal_gy)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── 纯函数：Grid_Arrived / Goal_Cell / In_Goal_Cell（task_31 Step 7 / D2）───

    #[test]
    fn test_Grid_Arrived_rejects_cell_boundary_stop() {
        // D2 现场：旧逻辑「入目标格即到达」实测停在 x=40.45（误差 0.45 > 0.3）
        assert!(!Grid_Arrived((40.0, 80.0), 40.45, 80.22));
    }

    #[test]
    fn test_Grid_Arrived_accepts_cell_center() {
        // 格心 (40.25, 80.25)：逐轴 0.25 ≤ 0.26
        assert!(Grid_Arrived((40.0, 80.0), 40.25, 80.25));
        assert!(Grid_Arrived((36.0, 110.0), 36.25, 110.25));
    }

    #[test]
    fn test_Grid_Arrived_rejects_far_axis() {
        assert!(!Grid_Arrived((36.0, 110.0), 36.40, 110.10));
    }

    // ─── Predict_Stop（task_31 Step 7 第 2 轮：到达判定必须算刹停滑行）───

    #[test]
    fn test_predict_stop_at_rest_is_current_position() {
        assert_eq!(Predict_Stop(40.0, 80.0, 0.0, 0.0), (40.0, 80.0));
        // 极小速度视作静止（防噪声把预测点推走）
        let (px, py) = Predict_Stop(40.0, 80.0, 1e-4, 0.0);
        assert_eq!((px, py), (40.0, 80.0));
    }

    #[test]
    fn test_predict_stop_matches_braking_distance() {
        // 1.2 m/s（= forward_speed 30% × max_speed 4.0）朝 +x：滑行 ≈ v²/(2a) + v·tick
        let v: f32 = 1.2;
        let (px, _py) = Predict_Stop(39.6, 80.0, v, 0.0);
        let coast = px - 39.6;
        let expect = v * v / (2.0 * 3.0) + v * 0.05;
        assert!((coast - expect).abs() < 1e-5, "预测滑行 {coast} 应等于 {expect}");
        // 预测停点 (39.6+0.30=39.90) 离目标 40 只有 0.10 → 这时判到达，停稳后 ≈0.10~0.30，仍 ≤0.3
        assert!((px - 40.0).abs() <= ARRIVAL_TOL_M, "预测停点应落在容差内：px={px}");
        // 反方向（-x）预测点也在运动方向一侧（不会凭空拉回到目标）
        let (px, _) = Predict_Stop(40.4, 80.0, -v, 0.0);
        assert!(px < 40.4 && (px - 40.0).abs() <= ARRIVAL_TOL_M);
    }

    #[test]
    fn test_Goal_Cell_floor_semantics() {
        assert_eq!(Goal_Cell((40.0, 80.0)), (80, 160));
        assert_eq!(Goal_Cell((36.0, 110.0)), (72, 220));
    }

    #[test]
    fn test_In_Goal_Cell() {
        assert!(In_Goal_Cell((40.0, 80.0), 40.45, 80.22));
        assert!(!In_Goal_Cell((40.0, 80.0), 39.99, 80.22));
        assert!(!In_Goal_Cell((40.0, 80.0), 40.45, 80.51));
    }

    // ─── 分支：GoalService::get_path 的 ③「入格但未到格心」───

    async fn svc() -> (GoalService, Arc<RwLock<RobotState>>) {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let svc = GoalService::new(
            state.clone(),
            Arc::new(RwLock::new(MissionQueue::default())),
            Vec::new(),
            Arc::new(RwLock::new(OccupancyGrid::new())),
            Arc::new(ClusterInfoTable::new()),
            0.2,
            ARRIVAL_THRESHOLD_M,
        );
        (svc, state)
    }

    async fn set_pose(state: &Arc<RwLock<RobotState>>, x: f32, y: f32) {
        let mut rs = state.write().await;
        rs.x = x;
        rs.y = y;
    }

    #[tokio::test]
    async fn test_get_path_keeps_goal_cell_until_centered() {
        let (mut svc, state) = svc().await;
        // 起点 (64,64) 装载目标 (40,80)
        svc.Start_Goal((40.0, 80.0), 64.0, 64.0);
        // 已入目标格但逐轴 0.45 m：不再判到达，而是把目标格自身当下一步（朝格心收敛）
        set_pose(&state, 40.45, 80.22).await;
        assert_eq!(svc.get_path().await, Some((80, 160)));
        assert_eq!(svc.goal, Some((40.0, 80.0)), "未到格心前 goal 必须保留");
        // 到达格心附近（逐轴 ≤ 0.26）：判到达，goal 清空
        set_pose(&state, 40.25, 80.25).await;
        assert_eq!(svc.get_path().await, None);
        assert_eq!(svc.goal, None);
    }

    #[tokio::test]
    async fn test_get_path_world_threshold_arrival_still_first() {
        let (mut svc, state) = svc().await;
        svc.Start_Goal((40.0, 80.0), 64.0, 64.0);
        // ① 世界距离 < 0.3 先判到达（与顺序保持一致的回归）
        set_pose(&state, 40.1, 80.05).await;
        assert_eq!(svc.get_path().await, None);
        assert_eq!(svc.goal, None);
    }

    #[tokio::test]
    async fn test_get_path_in_goal_cell_same_cell_as_start_does_not_report_unreachable() {
        // 目标格 == 起点格（D* 会 start==goal，④ 会报「不可达」）：③ 必须先兜住
        let (mut svc, state) = svc().await;
        // (40.4,80.4) 就在目标格 (80,160) 内但逐轴差 0.4 > 0.26 → 应返回目标格自身，而不是 None
        set_pose(&state, 40.4, 80.4).await;
        svc.Start_Goal((40.0, 80.0), 40.4, 80.4);
        assert_eq!(svc.get_path().await, Some((80, 160)));
        assert_eq!(svc.goal, Some((40.0, 80.0)));
        assert!(svc.pathfinder.is_some(), "不应走到 ④ 把 pathfinder 丢掉");
    }
}
