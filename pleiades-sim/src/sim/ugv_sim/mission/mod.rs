//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 任务层（Task 36 §12.4.4）：任务 → 树的**编译期穷尽**分派 + 树依赖包
//!
//! 分派点落在设备端（§十一 #13/#26）：`Mission` 将来加变体，此处漏改 → **编译不过**。

use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use pleiades_base::robot::core::behavior_tree::Node;
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
    }
}