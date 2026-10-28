//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-27

//! UAV（机）设备端模块
//!
//! - mavlink/：MAVLink 飞控驱动
//! - nav/：导航层（navigator + pathfinder + cluster_obstacles，与 `pleiades-ugv` 逐字相同）
//! - mission/：任务层（Mission → 树分派 + Goto 树，与 `pleiades-ugv` 逐字相同）
//! - executor.rs：2D 决策三状态机（三份逐字节相同，后 3D 化，Task 23 C3）

pub mod mavlink;
pub mod executor;
// 机端既无 `slam/`（不弹建图增量）也无 2D 急停（不调 `Navigator::mark_obstacle`），
// 故 `nav/` 里的 `ObstacleMode::Respect` 与 `cluster_to_obstacle_cells` 的对外 re-export
// 在本 crate 没有消费者。四个 `nav/*.rs` 要求与 ugv **逐字相同**（§12.5.4），
// 所以这条容忍写在模块声明上，而不是去改副本。
#[allow(dead_code, unused_imports)]
pub mod nav;
pub mod mission;
pub mod robot_handler;
