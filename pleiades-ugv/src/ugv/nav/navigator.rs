//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-10-29

//! 导航器（Task 36 §12.4.5）：目标 → 下一格，并维护「到达 / 被堵 / 无路」状态
//!
//! 与旧目标服务（`goal.rs`）的差别（§12.4.5 行为表）：
//! 1. **不再出队、不再做任务分派**（出队在 `main_loop`，分派在 `mission/`）；
//! 2. `status` 显式暴露，`Arrived` / `Blocked` **锁存**到下一次 `set_goal`；
//! 3. **无路不再放弃**：不清 goal、继续每拍重算；连续 `BLOCKED_AFTER_TICKS` 拍才判 `Blocked`；
//! 4. 障碍策略做成构造参数 `ObstacleMode`（三份副本逐字相同，差异只在这个参数）。
//! 5. **三态 + 时间预算**（Task 39 §二）：这拍没算完 → `NavStep::NotReady`，
//!    **不清 goal、不判失败、不增 `blocked_ticks`**（算不完 ≠ 没有路）。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, RwLock};
use tracing::{info, warn};

use pleiades_base::robot::core::cluster::ClusterInfoTable;
use pleiades_base::robot::core::grid::{world_to_grid, OccupancyGrid};
use pleiades_base::robot::core::map_delta::MapDelta;
use pleiades_base::robot::core::state::RobotState;

use super::cluster_obstacles::cluster_to_obstacle_cells;
use super::pathfinder::{DStarLite, StepResult};
// 枚举与端口定义在 `nav/mod.rs`（§12.4.4 分工），这里只取用
use super::{BlockedCause, NavPort, NavStatus, NavStep, ObstacleMode};

/// 到达判定阈值（米）：真机按**世界距离** 0.3 m（sim 侧判据不要求一致，§十一 #37）
pub const ARRIVAL_THRESHOLD_M: f32 = 0.3;

/// 连续无路的去抖拍数（1 s @ 20 Hz）：期间保持 `Navigating`，到点才判 `Blocked`
pub const BLOCKED_AFTER_TICKS: u32 = 20;

/// 规划时间预算的缺省值（Task 39 §二）= 三端 config 顶层键 `plan_budget_ms` 的缺省（20 ms）
///
/// 只在 `Navigator::new` 里兜底；装配期（handler）一定用 `set_plan_budget` 覆盖成 config 值。
pub const DEFAULT_PLAN_BUDGET: Duration = Duration::from_millis(20);

/// 连续 `NotReady` 多少拍打一条 `warn!`（5 拍 = 250 ms @ 20 Hz）
pub const NOT_READY_WARN_TICKS: u32 = 5;

/// 取出建图增量里变化的格（非阻塞；同批去重）
///
/// `None` = 没订阅（ugv / uav）：返回空。`Lagged` 说明 tick 太慢漏了批次，`warn!` 一次。
fn drain_map_deltas(rx: &mut Option<broadcast::Receiver<Vec<MapDelta>>>) -> Vec<(i32, i32)> {
    let Some(rx) = rx.as_mut() else { return Vec::new(); };
    let mut out: Vec<(i32, i32)> = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(batch) => out.extend(batch.iter().map(|d| (d.gx, d.gy))),
            Err(broadcast::error::TryRecvError::Empty) => break,
            Err(broadcast::error::TryRecvError::Closed) => break,
            Err(broadcast::error::TryRecvError::Lagged(n)) => {
                warn!("[Nav] 建图增量积压，丢弃 {n} 批（D* 修补可能滞后一帧）");
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// 分类被堵原因（与 `step_result` 的「清空动态障碍再算一次」同一条路径）
fn classify_blocked(pf: &mut Option<DStarLite>, grid: &OccupancyGrid, budget: Duration) -> BlockedCause {
    match pf.as_mut() {
        Some(pf) => {
            pf.set_dynamic_obstacles(&[], grid);
            if matches!(pf.next_step(grid, budget), StepResult::Step(_)) {
                BlockedCause::Dynamic
            } else {
                BlockedCause::NoPath
            }
        }
        None => BlockedCause::NoPath,
    }
}

/// 寻路一拍：延迟建 D*（`start` 用本拍实时位姿）→ `move_to` → 动态障碍 → 建图增量 → `next_step`
///
/// `will_latch` = 本拍若仍无路就要判 `Blocked` → 顺带把原因算出来（只在那一拍多算一次）。
#[allow(clippy::too_many_arguments)]
fn plan_next(
    pf: &mut Option<DStarLite>,
    grid: &OccupancyGrid,
    current: (i32, i32),
    start_cell: (i32, i32),
    goal_cell: (i32, i32),
    dynamic: &[(i32, i32)],
    changed: &[(i32, i32)],
    will_latch: bool,
    budget: Duration,
) -> (NavStep, Option<BlockedCause>) {
    if pf.is_none() {
        *pf = Some(DStarLite::new(start_cell, goal_cell));
    }
    let step = match pf.as_mut() {
        Some(pf) => {
            pf.move_to(current);
            pf.set_dynamic_obstacles(dynamic, grid);
            // 建图增量逐格走「急停」同一条修补路径
            for cell in changed {
                pf.mark_obstacle(*cell, grid);
            }
            pf.next_step(grid, budget)
        }
        None => StepResult::NoPath,
    };
    // 只有「确实没有路」才算被堵原因；`NotReady` 不是无路，不去多算一次
    let cause = if matches!(step, StepResult::NoPath) && will_latch {
        Some(classify_blocked(pf, grid, budget))
    } else {
        None
    };
    (NavStep::from(step), cause)
}

/// 导航器（原目标服务）
pub struct Navigator {
    grid: Arc<RwLock<OccupancyGrid>>,
    cluster_table: Arc<ClusterInfoTable>,
    obstacle_mode: ObstacleMode,
    /// 建图增量订阅：`None` = 不接建图（ugv / uav），`Some` = sim（Step 6）
    map_rx: Option<broadcast::Receiver<Vec<MapDelta>>>,
    obstacle_inflation_radius: f32,
    arrival_threshold_m: f32,
    /// 最终目标（世界坐标，米）
    goal: Option<(f32, f32)>,
    pathfinder: Option<DStarLite>,
    /// 终态锁存到下一次 `set_goal`
    status: NavStatus,
    /// 连续无路拍数（去抖）
    blocked_ticks: u32,
    /// 单拍规划时间预算（Task 39 §二）：装配期由 `set_plan_budget` 用 config 值覆盖
    plan_budget: Duration,
    /// 连续 `NotReady` 拍数（算不完，不判失败；达 `NOT_READY_WARN_TICKS` 打一条 warn）
    not_ready_ticks: u32,
}

impl Navigator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        robot_state: Arc<RwLock<RobotState>>,
        grid: Arc<RwLock<OccupancyGrid>>,
        cluster_table: Arc<ClusterInfoTable>,
        obstacle_mode: ObstacleMode,
        map_rx: Option<broadcast::Receiver<Vec<MapDelta>>>,
        obstacle_inflation_radius: f32,
        arrival_threshold_m: f32,
    ) -> Self {
        // 位姿统一由 `step(rs)` 入参给（§12.4.5），不持第二份位姿来源；
        // 保留该参数只为三份 `navigator.rs` 构造签名一致（Step 5 / 6 逐字拷贝）。
        let _ = robot_state;
        Self {
            grid,
            cluster_table,
            obstacle_mode,
            map_rx,
            obstacle_inflation_radius,
            arrival_threshold_m,
            goal: None,
            pathfinder: None,
            status: NavStatus::Idle,
            blocked_ticks: 0,
            plan_budget: DEFAULT_PLAN_BUDGET,
            not_ready_ticks: 0,
        }
    }

    /// 设规划时间预算（Task 39 §二）：装配期由 handler 从 config 顶层键 `plan_budget_ms` 读入后调用
    ///
    /// 不放构造参数是为了不动 5 处既有 `Navigator::new` 调用点（其中 `mission/mod.rs` 的单测
    /// 与 `emergency_stop.rs` 的测试不在本 step 允许路径内）；字段缺省值与 config 缺省同值。
    pub fn set_plan_budget(&mut self, budget: Duration) {
        info!("[Nav] 规划时间预算 = {:?}", budget);
        self.plan_budget = budget;
        self.not_ready_ticks = 0;
    }

    /// 订阅建图增量（sim 用；ugv / uav 不调用 → `map_rx` 保持 `None`）
    // Step 4 的 ugv 不订阅；Step 6（sim）在 `start()` 里调用
    #[allow(dead_code)]
    pub fn subscribe_map(&mut self, map_tx: &broadcast::Sender<Vec<MapDelta>>) {
        self.map_rx = Some(map_tx.subscribe());
    }

    /// 设目标：清终态 + 记目标（D* 在本拍 `step_result` 用实时位姿建，见其注释）
    pub fn set_goal(&mut self, x: f32, y: f32) {
        info!("[Nav] 新目标 ({x:.2}, {y:.2})");
        self.goal = Some((x, y));
        self.pathfinder = None;
        self.blocked_ticks = 0;
        self.not_ready_ticks = 0;
        self.status = NavStatus::Navigating;
    }

    /// 清目标（`reset` / 终值 / 切模式）：状态回 `Idle`，下一次 `set_goal` 重新开始
    pub fn clear_goal(&mut self) {
        self.goal = None;
        self.pathfinder = None;
        self.blocked_ticks = 0;
        self.not_ready_ticks = 0;
        self.status = NavStatus::Idle;
    }

    /// 终态查询（`Arrived` / `Blocked` 锁存到下一次 `set_goal`）
    pub fn status(&self) -> NavStatus {
        self.status
    }

    /// 急停标记障碍（触发 D* 局部修补，**保留 goal**，让恢复后能继续走）
    pub async fn mark_obstacle(&mut self, cell: (i32, i32)) {
        let grid = self.grid.read().await;
        if let Some(pf) = self.pathfinder.as_mut() {
            pf.mark_obstacle(cell, &grid);
        }
    }

    /// 每拍推进：到达判定 → 格级兜底 → 动态障碍 + 建图增量 + `next_step`，产出**三态**结果
    ///
    /// `async` 的必要性：要读 `grid` 的 `RwLock`；同步签名只能 `try_read`，
    /// 锁被建图写者占用时会「假无路」，把 20 拍去抖打乱。
    pub async fn step_result(&mut self, rs: &RobotState) -> NavStep {
        // 终态锁存：Arrived / Blocked 之后不再推进（等下一次 set_goal）
        if matches!(self.status, NavStatus::Arrived | NavStatus::Blocked(_)) {
            return NavStep::NoPath;
        }
        let Some(goal) = self.goal else { return NavStep::NoPath; };
        let (wx, wy) = (rs.x, rs.y);

        // ① 到达检测（世界距离）
        let dist = ((wx - goal.0).powi(2) + (wy - goal.1).powi(2)).sqrt();
        if dist < self.arrival_threshold_m {
            info!("[Nav] 到达目标 ({:.2}, {:.2})", goal.0, goal.1);
            self.arrive();
            return NavStep::NoPath;
        }

        // ② 格级兜底：已进目标格 → 到达
        if world_to_grid(goal.0, goal.1) == world_to_grid(wx, wy) {
            info!("[Nav] 已在目标格，到达");
            self.arrive();
            return NavStep::NoPath;
        }

        // ③ 寻路：动态障碍 → 建图增量 → next_step
        let changed = drain_map_deltas(&mut self.map_rx);
        let dynamic_obstacles: Vec<(i32, i32)> = match self.obstacle_mode {
            ObstacleMode::Ignore => Vec::new(),
            ObstacleMode::Respect => {
                let others = self.cluster_table.snapshot().await;
                cluster_to_obstacle_cells(&others, self.obstacle_inflation_radius)
                    .into_iter()
                    .collect()
            }
        };
        let current = world_to_grid(wx, wy);
        let start_cell = current;
        let goal_cell = world_to_grid(goal.0, goal.1);

        let will_latch = self.blocked_ticks.saturating_add(1) >= BLOCKED_AFTER_TICKS;
        // 障碍策略决定喂哪张图（uav：空图；ugv/sim：真实栅格）。
        // 两条分支都只做「字段级借用」：`self.grid` 的读锁 + `self.pathfinder` 的可变借用互不重叠。
        let (step, cause) = match self.obstacle_mode {
            ObstacleMode::Ignore => {
                let empty = OccupancyGrid::new();
                plan_next(
                    &mut self.pathfinder, &empty, current, start_cell, goal_cell,
                    &dynamic_obstacles, &changed, will_latch, self.plan_budget,
                )
            }
            ObstacleMode::Respect => {
                let real = self.grid.read().await;
                plan_next(
                    &mut self.pathfinder, &real, current, start_cell, goal_cell,
                    &dynamic_obstacles, &changed, will_latch, self.plan_budget,
                )
            }
        };

        match step {
            NavStep::Step(cell) => {
                self.blocked_ticks = 0;
                self.not_ready_ticks = 0;
                if matches!(self.status, NavStatus::Idle) {
                    self.status = NavStatus::Navigating;
                }
                NavStep::Step(cell)
            }
            NavStep::NoPath => {
                self.not_ready_ticks = 0;
                // 无路：**不清 goal**，继续每拍重算；连续 BLOCKED_AFTER_TICKS 拍才判 Blocked
                self.blocked_ticks = self.blocked_ticks.saturating_add(1);
                if self.blocked_ticks >= BLOCKED_AFTER_TICKS {
                    let cause = cause.unwrap_or(BlockedCause::NoPath);
                    warn!(
                        "[Nav] 连续 {} 拍无路 → Blocked({:?})，目标保留",
                        self.blocked_ticks, cause
                    );
                    self.status = NavStatus::Blocked(cause);
                }
                NavStep::NoPath
            }
            NavStep::NotReady => {
                // 算不完 ≠ 没有路：不清 goal、不判失败、**blocked_ticks 不动**（这一拍没结论）
                self.not_ready_ticks = self.not_ready_ticks.saturating_add(1);
                if self.not_ready_ticks == NOT_READY_WARN_TICKS {
                    warn!(
                        "[Nav] 连续 {} 拍未算完（250 ms）：start=({},{}) goal=({},{}) plan_budget={}ms blocked_ticks={} → 停车等，目标保留",
                        self.not_ready_ticks,
                        start_cell.0, start_cell.1,
                        goal_cell.0, goal_cell.1,
                        self.plan_budget.as_millis(),
                        self.blocked_ticks,
                    );
                }
                NavStep::NotReady
            }
        }
    }

    /// 两态适配器（旧接口保留）：`NotReady` 与 `NoPath` 都映射成 `None`
    ///
    /// 只给「不区分三态」的既有调用方用；叶子走 [`Navigator::step_result`]。
    pub async fn step(&mut self, rs: &RobotState) -> Option<(i32, i32)> {
        match self.step_result(rs).await {
            NavStep::Step(cell) => Some(cell),
            NavStep::NoPath | NavStep::NotReady => None,
        }
    }

    /// 到达：清目标 + 锁存 `Arrived`
    fn arrive(&mut self) {
        self.goal = None;
        self.pathfinder = None;
        self.blocked_ticks = 0;
        self.not_ready_ticks = 0;
        self.status = NavStatus::Arrived;
    }
}

#[async_trait::async_trait]
impl NavPort for Navigator {
    fn set_goal(&mut self, x: f32, y: f32) {
        Navigator::set_goal(self, x, y);
    }

    async fn step(&mut self, rs: &RobotState) -> Option<(i32, i32)> {
        Navigator::step(self, rs).await
    }

    async fn step_result(&mut self, rs: &RobotState) -> NavStep {
        Navigator::step_result(self, rs).await
    }

    fn status(&self) -> NavStatus {
        Navigator::status(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use pleiades_base::robot::core::cluster::ClusterInfo;

    /// 造一个 `Respect` 策略的导航器（位姿测试里另给，构造参数不落字段）
    fn nav_with(
        grid: Arc<RwLock<OccupancyGrid>>,
        table: Arc<ClusterInfoTable>,
        infl: f32,
    ) -> Navigator {
        Navigator::new(
            Arc::new(RwLock::new(RobotState::default())),
            grid,
            table,
            ObstacleMode::Respect,
            None,
            infl,
            ARRIVAL_THRESHOLD_M,
        )
    }

    /// 在 `(x, y)` 米处放一辆他车
    fn car_at(x: f32, y: f32) -> ClusterInfo {
        ClusterInfo {
            peer_id: vec![1, 2, 3],
            x,
            y,
            z: 0.0,
            yaw: 0.0,
            vx: 0.0,
            vy: 0.0,
            time_boot_ms: 0,
            last_seen: Instant::now(),
            sub_target: None,
        }
    }

    /// 标一格障碍（log-odds +3/次，`> 6` 才算 Occupied → 必须 3 次）
    async fn mark_occupied(grid: &Arc<RwLock<OccupancyGrid>>, cells: &[(i32, i32)]) {
        let mut g = grid.write().await;
        for &(gx, gy) in cells {
            for _ in 0..3 { g.update(gx, gy, true); }
        }
    }

    fn pose(x: f32, y: f32) -> RobotState {
        RobotState { x, y, ..Default::default() }
    }

    /// #1 世界距离到达（跨格：19 vs 20，确认走的是 ① 而非 ②），且锁存
    #[tokio::test]
    async fn test_arrive_by_world_distance() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let mut nav = nav_with(grid, Arc::new(ClusterInfoTable::new()), 0.2);
        nav.set_goal(10.28, 10.0); // 与 rs 跨格（19 vs 20），距离 0.29 < 0.3
        assert_eq!(nav.step(&pose(9.99, 10.0)).await, None);
        assert_eq!(nav.status(), NavStatus::Arrived);
        // 锁存：再拍仍 None、状态不变
        assert_eq!(nav.step(&pose(9.99, 10.0)).await, None);
        assert_eq!(nav.status(), NavStatus::Arrived);
    }

    /// #2 格级兜底（同格 20，距离 0.40 > 0.3 → 只能靠 ② 判到达）
    #[tokio::test]
    async fn test_arrive_by_goal_cell_fallback() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let mut nav = nav_with(grid, Arc::new(ClusterInfoTable::new()), 0.2);
        nav.set_goal(10.45, 10.0);
        assert_eq!(nav.step(&pose(10.05, 10.0)).await, None);
        assert_eq!(nav.status(), NavStatus::Arrived);
    }

    /// #3 去抖：无路 19 拍仍 `Navigating`，第 20 拍才 `Blocked`
    ///
    /// Task 39 起**围死目标格**（而不是围死起点格）：D* 证「起点不可达」必须弹空整堆
    /// （256×256 空图 ≈ 6.5 万次扩展 > `MAX_COMPUTE_ITERS`），会先连续 `NotReady`
    /// 若干拍（每拍续算一批）；围死目标则一次扩展就能收敛出 `NoPath`，去抖拍数才是纯粹的 20。
    #[tokio::test]
    async fn test_blocked_after_20_ticks_of_no_path() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        // 目标格 (24,24)（= 12.0 m）的四邻居全标占据 → 真·无路
        mark_occupied(&grid, &[(23, 24), (25, 24), (24, 23), (24, 25)]).await;
        let mut nav = nav_with(grid, Arc::new(ClusterInfoTable::new()), 0.2);
        nav.set_goal(12.0, 12.0);

        for i in 1..=19 {
            assert_eq!(nav.step(&pose(10.0, 10.0)).await, None, "第 {i} 拍应无路");
            assert_eq!(nav.status(), NavStatus::Navigating, "第 {i} 拍仍在去抖窗口内");
        }
        assert_eq!(nav.step(&pose(10.0, 10.0)).await, None);
        assert_eq!(nav.status(), NavStatus::Blocked(BlockedCause::NoPath));
    }

    /// #4 cause = Dynamic：清掉动态障碍后能走 → 被堵是他车挡的
    #[tokio::test]
    async fn test_blocked_cause_dynamic() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let table = Arc::new(ClusterInfoTable::new());
        // 他车压在**目标**上（膨胀 1 m → 目标及其周围一圈都是动态障碍）
        table.upsert(car_at(12.0, 12.0)).await;
        let mut nav = nav_with(grid, table, 1.0);
        nav.set_goal(12.0, 12.0);

        for _ in 0..19 {
            assert_eq!(nav.step(&pose(10.0, 10.0)).await, None);
            assert_eq!(nav.status(), NavStatus::Navigating);
        }
        assert_eq!(nav.step(&pose(10.0, 10.0)).await, None);
        assert_eq!(nav.status(), NavStatus::Blocked(BlockedCause::Dynamic));
    }

    /// #5 cause = NoPath：实心围栏 + 空集群表 → 清动态障碍后仍不行
    #[tokio::test]
    async fn test_blocked_cause_no_path() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        // 同 #3：围死目标格（收敛出 NoPath 不依赖迭代上限兜底）
        mark_occupied(&grid, &[(23, 24), (25, 24), (24, 23), (24, 25)]).await;
        let mut nav = nav_with(grid, Arc::new(ClusterInfoTable::new()), 0.2);
        nav.set_goal(12.0, 12.0);

        for _ in 0..19 {
            assert_eq!(nav.step(&pose(10.0, 10.0)).await, None);
        }
        assert_eq!(nav.step(&pose(10.0, 10.0)).await, None);
        assert_eq!(nav.status(), NavStatus::Blocked(BlockedCause::NoPath));
    }

    /// #6 Task 39 §二：`NotReady` 不清目标、不判失败；预算恢复后同一条 D* 链路上接着算
    #[tokio::test]
    async fn test_step_not_ready_keeps_goal_and_status() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let mut nav = nav_with(grid, Arc::new(ClusterInfoTable::new()), 0.2);
        nav.set_goal(40.0, 40.0);
        nav.set_plan_budget(Duration::ZERO);
        assert_eq!(nav.step_result(&pose(10.0, 10.0)).await, NavStep::NotReady);
        assert_eq!(nav.status(), NavStatus::Navigating, "NotReady 不得判失败/不清目标");
        // 预算恢复 → 同一条 D* 链路上接着算，立刻给出格（不是从无路里恢复）
        nav.set_plan_budget(Duration::from_secs(3));
        assert!(
            matches!(nav.step_result(&pose(10.0, 10.0)).await, NavStep::Step(_)),
            "预算恢复后应立刻给出下一格"
        );
        assert_eq!(nav.status(), NavStatus::Navigating);
    }
}