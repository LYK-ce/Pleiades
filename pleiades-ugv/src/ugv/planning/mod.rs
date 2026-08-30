//Presented by KeJi
//Created Date ： 2026-08-12
//Modified Date ： 2026-08-30

//! planning — 规划层（决策：去哪 + 怎么走）
//!
//! - assignment.rs：任务分配（群发 Goto 散布目标点计算，Task 14）
//! - pathfinder.rs：D* Lite 路径规划（从 slam/ 迁入，Task 14）
//! - cluster_obstacles.rs：他车 → 动态障碍格转换（Task 15）
//!
//! 分层语义：本层消费地图（只读 `core::grid`），不产生地图。

pub mod assignment;
pub mod pathfinder;
pub mod cluster_obstacles;

pub use pathfinder::DStarLite;
pub use cluster_obstacles::cluster_to_obstacle_cells;
