//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-31

//! 任务层（Task 36 §12.4.4）：任务 → 树的**编译期穷尽**分派 + 树依赖包
//!
//! 分派点落在设备端（§十一 #13/#26）：`Mission` 将来加变体，此处漏改 → **编译不过**。
//!
//! task_43 §五：车栈 `Follow` 与机栈**同构**（`follow.rs`，叶子不经 D\*，照抄 `uav_sim` 那份）；
//! task_43 §七：车栈 `Containment` 也是真树（`containment.rs`，照抄机栈那份，叶子不经 D\*）；
//! 无 detector 两者都退化为原地 Hold（`Running`，绝不一拍 `Success`）。

use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{Action, Frame, Node, Status};
use pleiades_base::robot::core::mission_queue::Mission;
use pleiades_base::robot::core::state::RobotState;

use crate::sim::ugv_sim::nav::Navigator;

pub mod containment;
pub mod follow;
pub mod goto;

/// 树叶子的依赖（导航端口 + 位姿快照 + 目标来源 + 协助请求）；加新任务类型时在此按需加字段
pub struct TaskDeps {
    pub nav: Arc<Mutex<Navigator>>,
    pub robot_state: Arc<RwLock<RobotState>>,
    /// 跟随的目标来源（task_43 §五）：`None` = 没启用 detector → Follow 建「原地 Hold」树
    pub detector: Option<Arc<crate::sim::detector::DetectorDevice>>,
    /// 协助请求发布者（task_43 §七，与机栈同形）：`None` = 不发（只判条件）；`Containment` 的 `InsideSquare` 消费
    pub assist: Option<Arc<dyn containment::AssistPublisher>>,
}

/// 任务 → 树（每次入队任务**新建实例**，不复用节点对象，§十一 #12）
pub fn build(mission: &Mission, deps: &TaskDeps) -> Result<Box<dyn Node>, String> {
    match mission {
        Mission::Goto { x, y } => Ok(goto::build(*x, *y, deps.nav.clone(), deps.robot_state.clone())),
        // task_43 §五：车栈 Follow 与机栈**同构**（照抄叶子，不经 D*）。
        // 无 detector 时退化为「原地 Hold」：`Action` 恒 `Running` + `next_cell = None` + 一条 warn。
        // **不得**一拍 `Success`（那会让队列把持续任务弹走，人就再也跟不上了），**不得**返回 `Err`
        // （Task 36 约定：建树失败 = 清队列 + 停车）。
        Mission::Follow { .. } => match deps.detector.as_ref() {
            Some(dev) => Ok(follow::build(
                deps.nav.clone(),
                deps.robot_state.clone(),
                Arc::new(follow::TargetSource::new(Some(dev.clone()))),
                follow::FollowParams::default(),
            )),
            None => {
                tracing::warn!(
                    "[Sim/ugv] Follow：未启用 detector（[detector].enabled=false 或无相机）→ 原地 Hold"
                );
                Ok(Box::new(Action::new(|f: &mut Frame<'_>| {
                    f.out.next_cell = None;
                    Status::Running
                })))
            }
        },
        // task_41 §3.2 + task_43 §七：sim 车栈真的封控（正方形内巡逻 + 跟随 + 出界回巡逻）
        // v2（task_41 §5.3）：`InsideSquare` 条件内发协助请求（`deps.assist = None` 时只判条件）
        Mission::Containment { x, y, half_side } => match deps.detector.as_ref() {
            Some(dev) => Ok(containment::build(
                deps.nav.clone(),
                deps.robot_state.clone(),
                Arc::new(follow::TargetSource::new(Some(dev.clone()))),
                (*x, *y),
                *half_side,
                deps.assist.clone(),
            )),
            None => {
                // 未启用 detector → **原地 Hold**（持续任务，不终值）。
                // `Success` 会让队列把这条弹走 → 任务静默消失（口径 #8 明确禁止）。
                tracing::warn!(
                    "[Sim/ugv] Containment：未启用 detector（[detector].enabled=false 或无相机）→ 原地 Hold"
                );
                Ok(Box::new(Action::new(|f: &mut Frame<'_>| {
                    f.out.next_cell = None;
                    Status::Running
                })))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pleiades_base::robot::core::behavior_tree::MotionOut;
    use pleiades_base::robot::core::cluster::ClusterInfoTable;
    use pleiades_base::robot::core::grid::OccupancyGrid;

    use crate::sim::ugv_sim::nav::{ObstacleMode, ARRIVAL_THRESHOLD_M};

    /// 造一套真 `TaskDeps`（`detector = None`：本 step 的建树路径不需要真模型）
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
        TaskDeps {
            nav: Arc::new(Mutex::new(nav)),
            robot_state: state,
            detector: None,
            assist: None,
        }
    }

    /// 无 detector：Follow 建「原地 Hold」树 —— 一拍 `Running`（**不终值**）、不动
    /// （task_43 §五 验收 3；机栈同款：`uav_sim/mission/mod.rs:112`）
    #[tokio::test]
    async fn test_follow_without_detector_is_hold_tree() {
        let d = deps();
        let mut tree = build(&Mission::Follow { x: 0.0, y: 0.0 }, &d).expect("Follow 不得返回 Err");
        let mut out = MotionOut::default();
        let mut f = Frame { dt_ms: 50, tick_no: 0, out: &mut out };
        for i in 0..3 {
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running（不终值）");
            assert_eq!(f.out.next_cell, None, "Hold 不得给出下一格");
        }
    }

    /// 无 detector：Containment 建「原地 Hold」树 —— 连 3 拍 `Running`（**不得一拍 Success**）、不动
    #[tokio::test]
    async fn test_containment_without_detector_is_hold_tree() {
        let d = deps();
        let mission = Mission::Containment { x: 64.0, y: 64.0, half_side: 8.0 };
        let mut tree = build(&mission, &d).expect("Containment 不得返回 Err");
        let mut out = MotionOut::default();
        let mut f = Frame { dt_ms: 50, tick_no: 0, out: &mut out };
        for i in 0..3 {
            assert_eq!(tree.tick(&mut f).await, Status::Running, "第 {i} 拍必须 Running（不终值）");
            assert_eq!(f.out.next_cell, None, "Hold 不得给出下一格");
        }
    }
}
