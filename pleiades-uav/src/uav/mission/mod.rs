//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-11-03

//! 任务层（Task 36 §12.4.4）：任务 → 树的**编译期穷尽**分派 + 树依赖包
//!
//! 分派点落在设备端（§十一 #13/#26）：`Mission` 将来加变体，此处漏改 → **编译不过**。
//!
//! task_45 §二 Step 2：真机 Follow 建**真树**（`follow.rs`，与真车 / sim 车同构）；无 detector → 原地 Hold
//! （恒 `Running` + `next_cell` / `target_point` 都 `None`，**绝不一拍 `Success`**，**绝不** `Err`）。
//! `Containment` 仍平凡（本步不动）。

use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{Action, Frame, Node, Status};
use pleiades_base::robot::core::mission_queue::Mission;
use pleiades_base::robot::core::state::RobotState;

use crate::uav::nav::Navigator;

pub mod follow;
pub mod goto;

/// 树叶子的依赖（导航端口 + 位姿快照 + 目标来源）；加新任务类型时在此按需加字段
pub struct TaskDeps {
    pub nav: Arc<Mutex<Navigator>>,
    pub robot_state: Arc<RwLock<RobotState>>,
    /// 跟随的目标来源（task_45 §二 Step 2）：`None` = 没启用 detector → Follow 建「原地 Hold」树
    pub detector: Option<Arc<crate::uav::robot_handler::detector::DetectorDevice>>,
}

/// 任务 → 树（每次入队任务**新建实例**，不复用节点对象，§十一 #12）
pub fn build(mission: &Mission, deps: &TaskDeps) -> Result<Box<dyn Node>, String> {
    match mission {
        Mission::Goto { x, y } => Ok(goto::build(*x, *y, deps.nav.clone(), deps.robot_state.clone())),
        // task_45 §二 Step 2：真机 Follow 与真车 / sim 车同构（`follow.rs` 由 `ugv/mission/follow.rs`
        // 逐字复制，只差 `use` 与文件头）；无 detector（`[detector].enabled=false` / 无相机）→ 原地 Hold。
        // **不得**一拍 `Success`（队列会把持续任务弹走，人就再也跟不上），**不得**返回 `Err`。
        Mission::Follow { .. } => match deps.detector.as_ref() {
            Some(dev) => Ok(follow::build(
                deps.robot_state.clone(),
                Arc::new(follow::TargetSource::new(Some(dev.clone()))),
                follow::FollowParams::default(),
            )),
            None => {
                tracing::warn!(
                    "[Uav] Follow：未启用 detector（[detector].enabled=false 或无相机）→ 原地 Hold"
                );
                Ok(Box::new(Action::new(|f: &mut Frame<'_>| {
                    f.out.next_cell = None;
                    f.out.target_point = None;
                    Status::Running
                })))
            }
        },
        // task_41 §3.2：Containment（type=3）在三端只建**平凡树**（一拍 `Success`，队列进下一条）。
        // 真子树只在 sim 机栈（`uav_sim/mission/containment.rs`）；**不得**返回 `Err`（= 清队列 + 停车），
        // 也**不得**返回 `Running`（会永久占住队首）。
        Mission::Containment { .. } => {
            tracing::warn!("[Uav] Containment 未实现，跳过（平凡树：一拍 Success）");
            Ok(Box::new(Action::new(|f: &mut Frame<'_>| {
                f.out.next_cell = None;
                Status::Success
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pleiades_base::robot::core::behavior_tree::MotionOut;
    use pleiades_base::robot::core::cluster::ClusterInfoTable;
    use pleiades_base::robot::core::grid::OccupancyGrid;

    use crate::uav::nav::{ObstacleMode, ARRIVAL_THRESHOLD_M};

    /// 造一套真 `TaskDeps`（`detector = None`：走 Hold 分支，不加载模型）
    fn deps() -> TaskDeps {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let nav = Navigator::new(
            state.clone(),
            Arc::new(RwLock::new(OccupancyGrid::new())),
            Arc::new(ClusterInfoTable::new()),
            ObstacleMode::Ignore,
            None,
            0.2,
            ARRIVAL_THRESHOLD_M,
        );
        TaskDeps { nav: Arc::new(Mutex::new(nav)), robot_state: state, detector: None }
    }

    /// task_45 §二 Step 2 / §一 #8：无 detector → Follow 建「原地 Hold」树
    /// （连 3 拍 `Running`、`next_cell` / `target_point` 都 `None`；**不终值**、不 Err）
    #[tokio::test]
    async fn test_follow_without_detector_is_hold_tree() {
        let d = deps();
        let mut tree = build(&Mission::Follow { x: 0.0, y: 0.0 }, &d).expect("Follow 不得返回 Err");
        let mut out = MotionOut::default();
        let mut f = Frame { dt_ms: 50, tick_no: 0, out: &mut out };
        for i in 0..3 {
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running（不终值）");
            assert_eq!(f.out.next_cell, None, "Hold 不得给出下一格");
            assert_eq!(f.out.target_point, None, "Hold 不得给出目标点");
        }
        tree.halt();
        tree.halt();
    }

    /// task_41 §3.2：真机 Containment = 平凡树，**一拍 `Success`**、不动、不 Err
    #[tokio::test]
    async fn test_containment_builds_trivial_success_tree() {
        let d = deps();
        let mission = Mission::Containment { x: 64.0, y: 64.0, half_side: 8.0 };
        let mut tree = build(&mission, &d).expect("Containment 不得返回 Err");
        let mut out = MotionOut::default();
        let mut f = Frame { dt_ms: 50, tick_no: 0, out: &mut out };
        assert_eq!(
            tree.tick(&mut f).await,
            Status::Success,
            "Containment 必须一拍终值（Success，不得 Running）"
        );
        assert_eq!(out.next_cell, None, "平凡树不得给出下一格");
        // 队列会弹走这条 → 树被 halt；幂等、不 panic
        tree.halt();
        tree.halt();
    }
}
