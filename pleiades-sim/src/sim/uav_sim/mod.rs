//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-28

//! 机栈（Task 36 §12.6）：复刻 `pleiades-uav/src/uav/` 的 `nav/` + `mission/`
//!
//! 与真机的差别**只有两处**（登记见 §12.6.4 / `Architecture/robot_arch.md`）：
//! 1. 到达判据用 sim 的「预测停点 + 格内逼近」（§十一 #37）；
//! 2. 不订建图增量（`map_rx = None`：机喂空栅格、不看地面建图）。
//!
//! 本文件 = 真机 `CarDeviceHandler` 里「树实例表 + 建树 + 推进 + 收尾」那一段，
//! 抽出来只服务仿真；`Mission → 树` 的分派在 `mission::build`（编译期穷尽，§十一 #13）。

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
    /// 机栈独有（task_37 Step 4）：跟随的目标来源（车栈没有这个字段）
    detector: Option<Arc<crate::sim::detector::DetectorDevice>>,
    trees: Mutex<TreeSet>,
    /// 拍号（`Frame.tick_no`）
    tick_no: AtomicU64,
}

impl Stack {
    /// 建栈（`start()` 里一次）。`map_tx = Some` → 订阅建图增量（sim 独有）；
    /// `detector = Some` → `Mission::Follow` / `FollowStraight` 建真跟随树（`None` → 原地 Hold）
    pub fn new(
        grid: Arc<RwLock<OccupancyGrid>>,
        cluster_table: Arc<ClusterInfoTable>,
        robot_state: Arc<RwLock<RobotState>>,
        obstacle_inflation_radius: f32,
        map_tx: Option<&broadcast::Sender<Vec<MapDelta>>>,
        detector: Option<Arc<crate::sim::detector::DetectorDevice>>,
    ) -> Self {
        let mut nav = Navigator::new(
            robot_state.clone(),
            grid,
            cluster_table,
            ObstacleMode::Ignore, // 机：喂空栅格、不注入他车动态障碍（§12.5.2）
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
            detector,
            trees: Mutex::new(TreeSet::default()),
            tick_no: AtomicU64::new(0),
        }
    }

    /// 建树（幂等：同一 `TaskId` 已有实例 → 只激活、不重建，§十一 #36）
    pub async fn build(&self, task: &QueuedTask) -> Result<(), String> {
        let mut ts = self.trees.lock().await;
        if ts.trees.contains_key(&task.id) {
            ts.active = Some(task.id);
            info!("[Sim/uav] 任务 {:?} 已有树实例，只激活", task.id);
            return Ok(());
        }
        let deps = TaskDeps {
            nav: self.nav.clone(),
            robot_state: self.robot_state.clone(),
            detector: self.detector.clone(),
        };
        let tree = mission::build(&task.mission, &deps)?;
        ts.trees.insert(task.id, tree);
        ts.active = Some(task.id);
        info!("[Sim/uav] 任务 {:?} 建树完成（实例数 {}）", task.id, ts.trees.len());
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

    use pleiades_base::robot::core::behavior_tree::MotionOut;
    use pleiades_base::robot::core::mission_queue::Mission;
    use pleiades_base::robot::core::robot::AUTO_TICK_MS;

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

    /// §12.6.7 #2 + #4：机栈 Goto + `ObstacleMode::Ignore` 生效（他车压在机器人格上也不堵）；
    /// 同时覆盖机栈的「同一 `TaskId` 两次 `build` 只留一个实例」（抢占恢复幂等）
    #[tokio::test]
    async fn test_goto_tree_ignores_dynamic_obstacles() {
        use std::time::Instant;
        use pleiades_base::robot::core::cluster::ClusterInfo;

        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let table = Arc::new(ClusterInfoTable::new());
        table
            .upsert(ClusterInfo {
                peer_id: vec![1, 2, 3],
                x: 10.0,
                y: 10.0,
                z: 0.0,
                yaw: 0.0,
                vx: 0.0,
                vy: 0.0,
                time_boot_ms: 0,
                last_seen: Instant::now(),
                sub_target: None,
            })
            .await;
        let state = Arc::new(RwLock::new(RobotState::default()));
        // 膨胀 1 m：Respect 下 20 拍就会 Blocked，Ignore 下必须一直有路
        let s = Stack::new(grid, table, state.clone(), 1.0, None, None);
        s.build(&goto_task(1, 12.0, 12.0)).await.expect("建树");
        // §12.6.7 #4：建树后恰一个实例；同一 `TaskId` 再次 `build`（抢占恢复路径）只激活、不重建
        assert_eq!(s.instance_count().await, 1, "建树后应只有一个实例");
        s.build(&goto_task(1, 12.0, 12.0)).await.expect("再次建树（抢占恢复）");
        assert_eq!(s.instance_count().await, 1, "同一 TaskId 只能有一个实例");
        set_pose(&state, 10.0, 10.0).await;
        for n in 0..25u64 {
            let mut out = MotionOut::default();
            let got = s.tick(&mut frame(&mut out, n)).await.expect("有活动树");
            assert_eq!(got.1, Status::Running, "第 {n} 拍不该被判失败");
            assert!(out.next_cell.is_some(), "第 {n} 拍应始终有下一格");
        }
        assert_eq!(s.nav.lock().await.status(), nav::NavStatus::Navigating);
    }

    /// task_37 §五 / C10：`detector = None` 时建 `Follow` 树成功，且**一拍 `Running`、不动**
    /// （不终值 → 队列不会被弹走；`Follow` 只能靠 TASK_SET 替换或 `mission_count=0` 结束）
    #[tokio::test]
    async fn test_follow_tree_holds_without_detector() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let table = Arc::new(ClusterInfoTable::new());
        let state = Arc::new(RwLock::new(RobotState::default()));
        let s = Stack::new(grid, table, state.clone(), 0.2, None, None);
        set_pose(&state, 10.0, 10.0).await;

        s.build(&QueuedTask { id: TaskId(1), mission: Mission::Follow { x: 0.0, y: 0.0 } })
            .await
            .expect("Follow 建树不得 Err");
        assert_eq!(s.instance_count().await, 1, "建树后应恰一个实例");

        for n in 0..3u64 {
            let mut out = MotionOut::default();
            let (id, st) = s.tick(&mut frame(&mut out, n)).await.expect("有活动树");
            assert_eq!(id, TaskId(1));
            assert_eq!(st, Status::Running, "第 {n} 拍 Follow 不得终值");
            assert_eq!(out.next_cell, None, "无 detector → 原地 Hold，不给下一格");
        }
        assert_eq!(
            s.nav.lock().await.status(),
            nav::NavStatus::Idle,
            "Hold 分支必须把 nav 目标清干净（状态回 Idle）"
        );
    }

    /// task_37 §六 / C8：`Stack` 层建 `FollowStraight` 树成功、连拍 `Running`、`next_cell = None`
    /// （无 detector → 原地 Hold；`task_ticks` 由 `on_tick` 累加，此处只验树不终值）
    #[tokio::test]
    async fn test_follow_straight_tree_holds_without_detector() {
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let table = Arc::new(ClusterInfoTable::new());
        let state = Arc::new(RwLock::new(RobotState::default()));
        let s = Stack::new(grid, table, state.clone(), 0.2, None, None);
        set_pose(&state, 10.0, 10.0).await;

        s.build(&QueuedTask {
            id: TaskId(1),
            mission: Mission::FollowStraight { x: 0.0, y: 0.0 },
        })
        .await
        .expect("FollowStraight 建树不得 Err");
        assert_eq!(s.instance_count().await, 1, "建树后应恰一个实例");

        let mut yaw = state.write().await;
        yaw.attitude.yaw = 0.0;
        drop(yaw);

        for n in 0..3u64 {
            let mut out = MotionOut::default();
            let (id, st) = s.tick(&mut frame(&mut out, n)).await.expect("有活动树");
            assert_eq!(id, TaskId(1));
            assert_eq!(st, Status::Running, "第 {n} 拍不得终值");
            assert_eq!(out.next_cell, None, "无 detector → 原地 Hold，不给下一格");
        }
        assert_eq!(
            s.nav.lock().await.status(),
            nav::NavStatus::Idle,
            "直线跟随不设目标 → nav 恒 Idle"
        );
    }
}
