//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-30

//! Robot 模块（base：共享底座）
//!
//! - core/：Robot 核心（主循环、命令、模式、任务队列、协议、集群、世界、栅格）
//! - util/：通用工具（serial）
//!
//! 设备端（ugv 车 / uav 机 / terminal 地面站）已拆到各自 crate（Task 23 C1-C4）。

pub mod core;
pub mod util;

pub use core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
pub use core::mode::OpMode;
pub use core::robot::Robot;
pub use core::state::RobotState;
