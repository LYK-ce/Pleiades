//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-11-05

//! 任务层（Task 36 §12.4.4）：任务 → 树的**编译期穷尽**分派 + 树依赖包
//!
//! 分派点落在设备端（§十一 #13/#26）：`Mission` 将来加变体，此处漏改 → **编译不过**。
//!
//! task_44 §二 Step 7：真车 `Follow` / `Containment` 都是**真树**（`follow.rs` / `containment.rs`，
//! 照抄 sim 车栈那两份，口径 #8 各 crate 自带一份）；无 detector 两者退化为**原地 Hold**
//! （恒 `Running` + `next_cell` / `target_point` 都 `None`，**绝不一拍 `Success`**，**绝不** `Err`）。
//!
//! task_45 §二 Step 1：`Follow` 的叶子改给**人的世界坐标**（`target_point`），`build()` 不再收 `nav`
//! （「清目标」的薄 trait 与其清目标方法一并删，§一 #6）；`Containment` 的巡逻分支仍 `goto::build(nav)`。
//!
//! task_48 §二 Step 2：`Face` 四端真叶子（`face.rs`）；`Guard` 车端真树（`guard.rs`），机端仍 naive。

use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{Action, Frame, Node, Status};
use pleiades_base::robot::core::mission_queue::Mission;
use pleiades_base::robot::core::state::RobotState;

use crate::ugv::nav::Navigator;

pub mod containment;
pub mod face;
pub mod follow;
pub mod goto;
pub mod guard;

/// 树叶子的依赖（导航端口 + 位姿快照 + 目标来源 + 协助请求）；加新任务类型时在此按需加字段
pub struct TaskDeps {
    pub nav: Arc<Mutex<Navigator>>,
    pub robot_state: Arc<RwLock<RobotState>>,
    /// 跟随的目标来源：`None` = 没启用 detector → Follow / Containment 建「原地 Hold」树
    pub detector: Option<Arc<crate::device::detector::DetectorDevice>>,
    /// 协助请求发布者：`None` = 不发（只判条件）；`Containment` 的 `InsideSquare` 消费
    pub assist: Option<Arc<dyn containment::AssistPublisher>>,
}

/// 任务 → 树（每次入队任务**新建实例**，不复用节点对象，§十一 #12）
pub fn build(mission: &Mission, deps: &TaskDeps) -> Result<Box<dyn Node>, String> {
    match mission {
        Mission::Goto { x, y } => Ok(goto::build(*x, *y, deps.nav.clone(), deps.robot_state.clone())),
        // task_44 §二 Step 7 + task_45 §二 Step 1：真车 Follow 与 sim 车栈**同构**（照抄叶子，不经 D*）。
        // 无 detector 时退化为「原地 Hold」：`Action` 恒 `Running` + `next_cell` / `target_point` 都 `None` + 一条 warn。
        // **不得**一拍 `Success`（那会让队列把持续任务弹走，人就再也跟不上了），**不得**返回 `Err`
        // （Task 36 约定：建树失败 = 清队列 + 停车）。
        Mission::Follow { .. } => match deps.detector.as_ref() {
            Some(dev) => Ok(follow::build(
                deps.robot_state.clone(),
                Arc::new(follow::TargetSource::new(Some(dev.clone()))),
                follow::FollowParams::default(),
            )),
            None => {
                tracing::warn!(
                    "[Ugv] Follow：未启用 detector（[detector].enabled=false 或无相机）→ 原地 Hold"
                );
                Ok(Box::new(Action::new(|f: &mut Frame<'_>| {
                    f.out.next_cell = None;
                    f.out.target_point = None;
                    Status::Running
                })))
            }
        },
        // task_44 §二 Step 7：真车真的封控（正方形内巡逻 + 跟随 + 出界回巡逻），照抄 sim 车栈那份。
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
                // `Success` 会让队列把这条弹走 → 任务静默消失（step.md 明令禁止）。
                tracing::warn!(
                    "[Ugv] Containment：未启用 detector（[detector].enabled=false 或无相机）→ 原地 Hold"
                );
                Ok(Box::new(Action::new(|f: &mut Frame<'_>| {
                    f.out.next_cell = None;
                    f.out.target_point = None;
                    Status::Running
                })))
            }
        },
        // task_48 §二 Step 2：Face 真叶子（四个栈都真做，口径 #3）+ Guard 真树（只车端，口径 #9）。
        // Face 只依赖自身位姿，四端同一份 `face.rs`（逐字）；Guard 的 detector=None 走
        // `TargetSource::new(None)`（`sample` 恒 None ⇒ 门恒失败 ⇒ 只值守不追，口径 #16），
        // 故它**不是一个 naive 分支**，而是同一棵树：`Success` 会让队列把守卫弹走，绝不这么写。
        Mission::Face { x, y } => Ok(face::build(deps.robot_state.clone(), (*x, *y))),
        Mission::Guard { x, y, fx, fy } => {
            let source = match deps.detector.as_ref() {
                // 与 Follow / Containment 同款：握 `DetectorDevice`，`sample` 走它（`follow.rs:152-156`）
                Some(dev) => Arc::new(follow::TargetSource::new(Some(dev.clone()))),
                None => {
                    tracing::warn!(
                        "[Ugv] Guard：未启用 detector（[detector].enabled=false 或无相机）→ 只值守不追"
                    );
                    Arc::new(follow::TargetSource::new(None))
                }
            };
            Ok(guard::build(
                deps.nav.clone(),
                deps.robot_state.clone(),
                source,
                (*x, *y),
                (*fx, *fy),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use pleiades_base::robot::core::behavior_tree::MotionOut;
    use pleiades_base::robot::core::cluster::ClusterInfoTable;
    use pleiades_base::robot::core::grid::OccupancyGrid;

    use crate::ugv::nav::{ObstacleMode, ARRIVAL_THRESHOLD_M};

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

    /// 无 detector：Follow 建「原地 Hold」树 —— 连 3 拍 `Running`（**不终值**）、不动
    /// （task_44 §二 Step 7 验收 5；与 sim 车栈同款：`ugv_sim/mission/mod.rs`）
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
        // 幂等、不 panic
        tree.halt();
        tree.halt();
    }
}
