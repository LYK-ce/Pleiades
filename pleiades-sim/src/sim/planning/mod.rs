//Presented by KeJi
//Created Date ： 2026-08-12
//Modified Date ： 2026-10-27

//! planning — 他车动态障碍（Task 15）
//!
//! ⚠️ Task 36 Step 6：`pathfinder.rs` 已迁入 `ugv_sim/nav/` 与 `uav_sim/nav/`（两栈各一份副本，
//! 与真机 `nav/` 结构一致）；本模块只剩 `cluster_obstacles`，调用方是
//! `sensors/lidar/mapping_task.rs`（他车 → 障碍格，**写 grid 的是传感器**）。

pub mod cluster_obstacles;

pub use cluster_obstacles::cluster_to_obstacle_cells;
