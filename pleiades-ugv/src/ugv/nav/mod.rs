//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-10-29

//! 导航层（Task 36 §12.4.4：原 `goal.rs` + 原规划层合并为 `nav/`）
//!
//! - `navigator.rs`：`Navigator`（目标 → 下一格 + 到达/被堵/无路 状态；**不碰队列**）
//! - `pathfinder.rs`：D* Lite（原样搬入）
//! - `cluster_obstacles.rs`：他车 → 动态障碍格（原样搬入）
//!
//! 分层语义：本层只读 `core::grid`，不产生地图；**出队只在 `main_loop`**，本层不做出队。

pub mod cluster_obstacles;
pub mod navigator;
pub mod pathfinder;

pub use cluster_obstacles::cluster_to_obstacle_cells;
// `BLOCKED_AFTER_TICKS` / `DStarLite` 目前只有 `nav/` 内部在用；对外 re-export 是留给
// Step 5（uav）调参与 Step 6（sim）复用（三份副本接口一致），先在 bin 里忽略 unused。
#[allow(unused_imports)]
pub use navigator::{Navigator, ARRIVAL_THRESHOLD_M, BLOCKED_AFTER_TICKS};
#[allow(unused_imports)]
pub use pathfinder::{DStarLite, StepResult};

use pleiades_base::robot::core::state::RobotState;

/// 被堵原因：失败那一拍清空动态障碍再算一次来分类（§12.4.5）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedCause {
    /// 他车 / 急停补丁挡的：清掉动态障碍就能走
    Dynamic,
    /// 地图上确实没有路
    NoPath,
}

/// 导航状态（`Arrived` / `Blocked` 锁存到下一次 `set_goal`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavStatus {
    /// 没有目标（未武装 / 被清）
    Idle,
    Navigating,
    Arrived,
    Blocked(BlockedCause),
}

/// 障碍策略：三份 `navigator.rs` 逐字相同，差异只走这个参数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObstacleMode {
    /// 尊重地面障碍：注入他车动态障碍 + 读真实栅格（ugv / sim）
    Respect,
    /// 忽略地面障碍：喂空栅格、不注入动态障碍（uav，Task 27）
    // Step 4 只有 ugv 传 `Respect`；`Ignore` 在 Step 5（uav）接入
    #[allow(dead_code)]
    Ignore,
}

/// 导航一拍的三种结果（Task 39 §二）：**"这拍没算完" 与 "确实没有路" 必须分开**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavStep {
    /// 有一格：叶子写 `Frame.out.next_cell = Some(cell)`
    Step((i32, i32)),
    /// 确实没有路：走 `blocked_ticks` 去抖，到点判 `Blocked`
    NoPath,
    /// 这拍没算完：叶子写 `next_cell = None`（执行器 `Stop`），但 `Status` 仍是 `Running`
    NotReady,
}

impl From<StepResult> for NavStep {
    fn from(r: StepResult) -> Self {
        match r {
            StepResult::Step(cell) => NavStep::Step(cell),
            StepResult::NoPath => NavStep::NoPath,
            StepResult::NotReady => NavStep::NotReady,
        }
    }
}

/// 叶子依赖的导航端口（`mission::goto` 只认这些方法 → 单测可用假端口，§12.4.9 #6~#8）
#[async_trait::async_trait]
pub trait NavPort: Send {
    fn set_goal(&mut self, x: f32, y: f32);
    async fn step(&mut self, rs: &RobotState) -> Option<(i32, i32)>;
    /// 三态版本（Task 39 §二）：默认实现把两态 `step()` 映射为 `Step` / `NoPath`。
    ///
    /// 带默认实现是为了**只实现两态的端口零改动、行为不变**（例如 `uav_sim` 跟随叶子的假端口）；
    /// `Navigator` 覆写成真实三态（多出一个 `NotReady`）。
    async fn step_result(&mut self, rs: &RobotState) -> NavStep {
        match self.step(rs).await {
            Some(cell) => NavStep::Step(cell),
            None => NavStep::NoPath,
        }
    }
    fn status(&self) -> NavStatus;
}
