//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-28

//! 任务层（Task 36 §12.4.4）：任务 → 树的**编译期穷尽**分派 + 树依赖包
//!
//! 分派点落在设备端（§十一 #13/#26）：`Mission` 将来加变体，此处漏改 → **编译不过**。
//!
//! task_37 §五：机栈是**唯一真跟**的地方（`Follow` → `follow::build`）；真机与车栈只建平凡树。
//! task_37 §六：`follow.rs` 的叶子**不经 D\***（不设目标、不取下一步），`next_cell` 由几何算出。

use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::{Action, Frame, Node, Status};
use pleiades_base::robot::core::mission_queue::Mission;
use pleiades_base::robot::core::state::RobotState;

use crate::sim::uav_sim::nav::Navigator;

pub mod follow;
pub mod goto;

/// 树叶子的依赖（导航端口 + 位姿快照）；加新任务类型时在此按需加字段
pub struct TaskDeps {
    pub nav: Arc<Mutex<Navigator>>,
    pub robot_state: Arc<RwLock<RobotState>>,
    /// 跟随的目标来源（task_37 Step 4）：`None` = 没启用 detector → Follow 建「原地 Hold」树
    pub detector: Option<Arc<crate::sim::detector::DetectorDevice>>,
}

/// 任务 → 树（每次入队任务**新建实例**，不复用节点对象，§十一 #12）
pub fn build(mission: &Mission, deps: &TaskDeps) -> Result<Box<dyn Node>, String> {
    match mission {
        Mission::Goto { x, y } => Ok(goto::build(*x, *y, deps.nav.clone(), deps.robot_state.clone())),
        // Task 37 §五：sim 机栈才真的跟；无 detector 时退化为「原地 Hold」
        // （持续任务，**不终值**：`Success` 会让队列把 Follow 弹走，人就再也跟不上了）
        Mission::Follow { .. } => match deps.detector.as_ref() {
            Some(dev) => Ok(follow::build(
                deps.nav.clone(),
                deps.robot_state.clone(),
                Arc::new(follow::TargetSource::new(Some(dev.clone()))),
                follow::FollowParams::default(),
            )),
            None => {
                tracing::warn!(
                    "[Sim/uav] Follow：未启用 detector（[detector].enabled=false 或无相机）→ 原地 Hold"
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

    use crate::sim::uav_sim::nav::{ObstacleMode, ARRIVAL_THRESHOLD_M};

    /// 造一套 `TaskDeps`（`detector = None`：本 step 的建树路径不需要真模型）
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

    /// 无 detector：Follow 建「原地 Hold」树 —— 一拍 `Running`（**不终值**）、不动
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

    // 有 detector 的建树路径由 `follow::tests` 用 `FakeFeed` 覆盖
    // （真 `DetectorDevice` 需要权重文件，单测不加载模型 → 此处只验无 detector 的退化路径）。
}
