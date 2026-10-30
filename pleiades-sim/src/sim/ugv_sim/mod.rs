//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-29

//! 车栈（Task 36 §12.6）：复刻 `pleiades-ugv/src/ugv/` 的 `nav/` + `mission/`
//!
//! 与真机的差别**只有两处**（登记见 §12.6.4 / `Architecture/robot_arch.md`）：
//! 1. 到达判据用 sim 的「预测停点 + 格内逼近」（§十一 #37）；
//! 2. 建图增量真订阅（`map_rx = Some`，真机 `None`）。
//!
//! 本文件 = 真机 `CarDeviceHandler` 里「树实例表 + 建树 + 推进 + 收尾」那一段，
//! 抽出来只服务仿真；`Mission → 树` 的分派在 `mission::build`（编译期穷尽，§十一 #13）。
//!
//! 子模块 `executor`：车栈连续档位执行器（task_40 §二，独立文件，与 `uav_sim/executor.rs` 同构）。

pub mod executor;
pub mod mission;
pub mod nav;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::info;

use pleiades_base::robot::core::behavior_tree::{Frame, Node, Status};
use pleiades_base::robot::core::cluster::ClusterInfoTable;
use pleiades_base::robot::core::grid::OccupancyGrid;
use pleiades_base::robot::core::map_delta::MapDelta;
use pleiades_base::robot::core::mission_queue::{QueuedTask, TaskId};
use pleiades_base::robot::core::state::RobotState;

use self::mission::TaskDeps;
use self::nav::{Navigator, ObstacleMode, ARRIVAL_THRESHOLD_M};

/// 树实例表（§十一 #34）：懒创建、按 `TaskId` 存；`active` = 本拍要 tick 的那棵
#[derive(Default)]
struct TreeSet {
    trees: HashMap<TaskId, Box<dyn Node>>,
    /// 当前任务（`build` 写；`main_loop` 只认 `Terminated` 才出队）
    active: Option<TaskId>,
}

/// 一套任务栈 = 导航器 + 树实例表 + 拍号（与 `CarDeviceHandler` 的 `CarInner` 同形）
pub struct Stack {
    pub nav: Arc<Mutex<Navigator>>,
    robot_state: Arc<RwLock<RobotState>>,
    trees: Mutex<TreeSet>,
    /// 拍号（`Frame.tick_no`）
    tick_no: AtomicU64,
}

impl Stack {
    /// 建栈（`start()` 里一次）。`map_tx = Some` → 订阅建图增量（sim 独有）
    pub fn new(
        grid: Arc<RwLock<OccupancyGrid>>,
        cluster_table: Arc<ClusterInfoTable>,
        robot_state: Arc<RwLock<RobotState>>,
        obstacle_inflation_radius: f32,
        map_tx: Option<&broadcast::Sender<Vec<MapDelta>>>,
    ) -> Self {
        let mut nav = Navigator::new(
            robot_state.clone(),
            grid,
            cluster_table,
            ObstacleMode::Respect,
            None,
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        );
        if let Some(tx) = map_tx {
            nav.subscribe_map(tx);
        }
        Self {
            nav: Arc::new(Mutex::new(nav)),
            robot_state,
            trees: Mutex::new(TreeSet::default()),
            tick_no: AtomicU64::new(0),
        }
    }

    /// 建树（幂等：同一 `TaskId` 已有实例 → 只激活、不重建，§十一 #36）
    pub async fn build(&self, task: &QueuedTask) -> Result<(), String> {
        let mut ts = self.trees.lock().await;
        if ts.trees.contains_key(&task.id) {
            ts.active = Some(task.id);
            info!("[Sim/ugv] 任务 {:?} 已有树实例，只激活", task.id);
            return Ok(());
        }
        let deps = TaskDeps { nav: self.nav.clone(), robot_state: self.robot_state.clone() };
        let tree = mission::build(&task.mission, &deps)?;
        ts.trees.insert(task.id, tree);
        ts.active = Some(task.id);
        info!("[Sim/ugv] 任务 {:?} 建树完成（实例数 {}）", task.id, ts.trees.len());
        Ok(())
    }

    /// 推进当前任务的树（无 active → `None`：这一拍没有树，执行器自然停车）
    pub async fn tick(&self, frame: &mut Frame<'_>) -> Option<(TaskId, Status)> {
        let mut ts = self.trees.lock().await;
        let id = ts.active?;
        let tree = ts.trees.get_mut(&id)?;
        Some((id, tree.tick(frame).await))
    }

    /// 终值收尾：halt + 丢实例 + 清 nav（下一次 `set_goal` 才能重新武装）
    pub async fn finish(&self, id: TaskId) {
        {
            let mut ts = self.trees.lock().await;
            if let Some(mut tree) = ts.trees.remove(&id) {
                tree.halt();
            }
            if ts.active == Some(id) {
                ts.active = None;
            }
        }
        self.nav.lock().await.clear_goal();
    }

    /// `reset`（§十一 #38：replace / 切模式 → halt 树 + 清实例表 + 清 nav；停车由调用方做）
    pub async fn reset(&self) {
        {
            let mut ts = self.trees.lock().await;
            for tree in ts.trees.values_mut() {
                tree.halt();
            }
            ts.trees.clear();
            ts.active = None;
        }
        self.nav.lock().await.clear_goal();
    }

    pub fn next_tick_no(&self) -> u64 {
        self.tick_no.fetch_add(1, Ordering::Relaxed)
    }

    #[cfg(test)]
    pub async fn instance_count(&self) -> usize {
        self.trees.lock().await.trees.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pleiades_base::robot::core::behavior_tree::{FailCode, MotionOut};
    use pleiades_base::robot::core::mission_queue::Mission;
    use pleiades_base::robot::core::robot::AUTO_TICK_MS;

    /// 空地图 + 空集群表的车栈（单测不需要建图增量）
    fn stack() -> (Stack, Arc<RwLock<RobotState>>) {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let s = Stack::new(
            Arc::new(RwLock::new(OccupancyGrid::new())),
            Arc::new(ClusterInfoTable::new()),
            state.clone(),
            0.2,
            None,
        );
        (s, state)
    }

    fn goto_task(id: u64, x: f32, y: f32) -> QueuedTask {
        QueuedTask { id: TaskId(id), mission: Mission::Goto { x, y } }
    }

    async fn set_pose(state: &Arc<RwLock<RobotState>>, x: f32, y: f32) {
        let mut rs = state.write().await;
        rs.x = x;
        rs.y = y;
    }

    fn frame<'a>(out: &'a mut MotionOut, n: u64) -> Frame<'a> {
        Frame { dt_ms: AUTO_TICK_MS as u32, tick_no: n, out }
    }

    /// §12.6.7 #1：车栈 Goto → 未到格心给下一格，到格心后 `Success` 且清输出
    #[tokio::test]
    async fn test_goto_tree_reaches_goal_and_succeeds() {
        let (s, state) = stack();
        s.build(&goto_task(1, 10.0, 10.0)).await.expect("建树");
        set_pose(&state, 10.0, 10.4).await;
        let mut out = MotionOut::default();
        let got = s.tick(&mut frame(&mut out, 0)).await.expect("有活动树");
        assert_eq!(got, (TaskId(1), Status::Running));
        assert_eq!(out.next_cell, Some((20, 20)), "未到格心 → 走目标格自身");
        set_pose(&state, 10.0, 10.02).await;
        let mut out = MotionOut::default();
        let got = s.tick(&mut frame(&mut out, 1)).await.expect("有活动树");
        assert_eq!(got, (TaskId(1), Status::Success));
        assert_eq!(out.next_cell, None, "成功那拍必须清输出");
    }

    /// §12.6.7 #4：同一 `TaskId` 两次 `build` → 只有一个实例（抢占恢复幂等）
    #[tokio::test]
    async fn test_build_task_is_idempotent_for_same_id() {
        let (s, _state) = stack();
        s.build(&goto_task(7, 1.0, 1.0)).await.expect("首次建树");
        assert_eq!(s.instance_count().await, 1);
        s.build(&goto_task(7, 1.0, 1.0)).await.expect("再次建树（抢占恢复）");
        assert_eq!(s.instance_count().await, 1, "同一 TaskId 只能有一个实例");
        s.build(&goto_task(8, 2.0, 2.0)).await.expect("新任务");
        assert_eq!(s.instance_count().await, 2);
    }

    /// §12.6.8 #2（替身）：第一条 `Success` → `finish` 出表 → 第二条是新实例
    #[tokio::test]
    async fn test_two_gotos_run_in_sequence() {
        let (s, state) = stack();
        s.build(&goto_task(1, 10.0, 10.0)).await.expect("建树 1");
        set_pose(&state, 10.0, 10.0).await;
        let mut out = MotionOut::default();
        assert_eq!(
            s.tick(&mut frame(&mut out, 0)).await,
            Some((TaskId(1), Status::Success))
        );
        s.finish(TaskId(1)).await;
        assert_eq!(s.instance_count().await, 0, "终值后实例必须出表");
        assert_eq!(s.nav.lock().await.status(), nav::NavStatus::Idle, "终值后 nav 必须清目标");
        s.build(&goto_task(2, 10.0, 10.0)).await.expect("建树 2");
        assert_eq!(s.instance_count().await, 1, "第二条必须是新实例");
        let mut out = MotionOut::default();
        assert_eq!(
            s.tick(&mut frame(&mut out, 1)).await,
            Some((TaskId(2), Status::Success))
        );
    }

    /// §12.6.8 #3（替身）：围死的目标 → 恢复路走完 → `Failure(Unreachable)`（通用码 3）
    #[tokio::test]
    async fn test_unreachable_target_fails_with_unreachable_code() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        {
            let mut g = grid.write().await;
            for &(gx, gy) in &[(19, 20), (21, 20), (20, 19), (20, 21)] {
                for _ in 0..3 {
                    g.update(gx, gy, true);
                }
            }
        }
        let state = Arc::new(RwLock::new(RobotState::default()));
        let s = Stack::new(grid, Arc::new(ClusterInfoTable::new()), state.clone(), 0.2, None);
        s.build(&goto_task(1, 12.0, 12.0)).await.expect("建树");
        set_pose(&state, 10.0, 10.0).await;

        let mut last = None;
        for n in 0..400u64 {
            let mut out = MotionOut::default();
            if let Some((_, st)) = s.tick(&mut frame(&mut out, n)).await {
                if !matches!(st, Status::Running) {
                    last = Some(st);
                    break;
                }
            }
        }
        assert_eq!(
            last,
            Some(Status::Failure(FailCode::Unreachable)),
            "围死后应 Failure(Unreachable)（20 拍去抖 + 3 s Wait + 再次 20 拍）"
        );
    }

    /// §12.6.8 #4（替身）：`reset`（= replace 路径）→ 无活动树、无输出（这一拍就停车）
    #[tokio::test]
    async fn test_reset_halts_tree_and_clears_output() {
        let (s, state) = stack();
        s.build(&goto_task(1, 10.0, 10.0)).await.expect("建树");
        set_pose(&state, 10.0, 11.0).await;
        let mut out = MotionOut::default();
        assert_eq!(
            s.tick(&mut frame(&mut out, 0)).await,
            Some((TaskId(1), Status::Running))
        );
        assert!(out.next_cell.is_some(), "在途任务本拍应有下一格");
        s.reset().await;
        assert_eq!(s.instance_count().await, 0, "reset 必须清实例表");
        assert_eq!(s.nav.lock().await.status(), nav::NavStatus::Idle, "reset 必须清 nav");
        let mut out = MotionOut::default();
        assert_eq!(s.tick(&mut frame(&mut out, 1)).await, None, "reset 后无活动树");
        assert_eq!(out.next_cell, None, "无输出 → 执行器自然停车");
    }

    /// §12.6.8 #5（替身）：无任务 → 零输出、零副作用
    #[tokio::test]
    async fn test_no_task_yields_no_output() {
        let (s, _state) = stack();
        let mut out = MotionOut::default();
        assert_eq!(s.tick(&mut frame(&mut out, 0)).await, None);
        assert_eq!(out.next_cell, None);
    }
}
