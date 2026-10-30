//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-30

//! 任务层（Task 36 §12.4.4）：任务 → 树的**编译期穷尽**分派 + 树依赖包
//!
//! 分派点落在设备端（§十一 #13/#26）：`Mission` 将来加变体，此处漏改 → **编译不过**。

use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{Action, Frame, Node, Status};
use pleiades_base::robot::core::mission_queue::Mission;
use pleiades_base::robot::core::state::RobotState;

use crate::sim::ugv_sim::nav::Navigator;

pub mod goto;

/// 树叶子的依赖（导航端口 + 位姿快照）；加新任务类型时在此按需加字段
pub struct TaskDeps {
    pub nav: Arc<Mutex<Navigator>>,
    pub robot_state: Arc<RwLock<RobotState>>,
}

/// 任务 → 树（每次入队任务**新建实例**，不复用节点对象，§十一 #12）
pub fn build(mission: &Mission, deps: &TaskDeps) -> Result<Box<dyn Node>, String> {
    match mission {
        Mission::Goto { x, y } => Ok(goto::build(*x, *y, deps.nav.clone(), deps.robot_state.clone())),
        // Task 37 §五.2：**车不跟**（本 step 只在 `uav_sim` 实现）。平凡树 = 一拍 `Success`，
        // 让队列进下一条；**不得返回 `Err`**（Task 36 约定：建树失败 = 清队列 + 停车）。
        // 与 `uav_sim` 的持续树不对称是刻意的：车没有相机/detector，Hold 会永远占着队首（见 robot_arch.md D-S6）。
        Mission::Follow { .. } => {
            tracing::warn!("[Sim/ugv] Follow 未实现（车不跟），跳过（平凡树：一拍 Success）");
            Ok(Box::new(Action::new(|f: &mut Frame<'_>| {
                f.out.next_cell = None;
                Status::Success
            })))
        }
        // task_41 §3.2：Containment（type=3）在三端只建**平凡树**（一拍 `Success`，队列进下一条）。
        // 真子树只在 sim 机栈（`uav_sim/mission/containment.rs`）；**不得**返回 `Err`（= 清队列 + 停车），
        // 也**不得**返回 `Running`（会永久占住队首）。
        Mission::Containment { .. } => {
            tracing::warn!("[Sim/ugv] Containment 未实现，跳过（平凡树：一拍 Success）");
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

    use crate::sim::ugv_sim::nav::{ObstacleMode, ARRIVAL_THRESHOLD_M};

    /// 造一套真 `TaskDeps`（平凡树不读它们，只为签名完整）
    fn deps() -> TaskDeps {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let nav = Navigator::new(
            state.clone(),
            Arc::new(RwLock::new(OccupancyGrid::new())),
            Arc::new(ClusterInfoTable::new()),
            ObstacleMode::Respect,
            None,
            0.2,
            ARRIVAL_THRESHOLD_M,
        );
        TaskDeps { nav: Arc::new(Mutex::new(nav)), robot_state: state }
    }

    /// Task 37 §五.2 / step.md:12/18：sim 车栈 Follow = 平凡树，**一拍 `Success`**、不动、不 Err
    #[tokio::test]
    async fn test_follow_builds_trivial_success_tree() {
        let d = deps();
        let mut tree = build(&Mission::Follow { x: 0.0, y: 0.0 }, &d).expect("Follow 不得返回 Err");
        let mut out = MotionOut::default();
        let mut f = Frame { dt_ms: 50, tick_no: 0, out: &mut out };
        assert_eq!(tree.tick(&mut f).await, Status::Success, "Follow 必须一拍终值（Success）");
        assert_eq!(out.next_cell, None, "平凡树不得给出下一格");
        // 队列会弹走这条 → 树被 halt；幂等、不 panic
        tree.halt();
    }

    /// task_41 §3.2：sim 车栈 Containment = 平凡树，**一拍 `Success`**、不动、不 Err
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
