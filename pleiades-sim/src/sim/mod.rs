//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! SIM 设备端模块（无硬件）
//!
//! - sim_handler.rs：SimDeviceHandler（实现 base 的 DeviceHandler，动作只打印）
//! - goal.rs / executor.rs / planning/：复制车 2D 决策（寻路 + 三状态机）

pub mod goal;
pub mod executor;
pub mod planning;
pub mod sim_handler;
