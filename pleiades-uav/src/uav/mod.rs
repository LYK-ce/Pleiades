//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! UAV（机）设备端模块
//!
//! - mavlink/：MAVLink 飞控驱动
//! - executor.rs / goal.rs / planning/：先复制车 2D 决策（后 3D 化，Task 23 C3）

pub mod mavlink;
pub mod executor;
pub mod goal;
pub mod planning;
pub mod robot_handler;
