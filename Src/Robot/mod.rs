//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-20

//! Robot 模块
//!
//! - core/：Robot 核心（主循环、命令、模式、任务队列、执行器）
//! - core/state.rs：全局状态
//! - control/：底层设备驱动
//! - slam/：占据栅格建图 + 路径规划

pub mod core;
pub mod control;
pub mod device;
pub mod slam;
pub mod world;

pub use core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
pub use core::mode::OpMode;
pub use core::robot::Robot;
pub use control::types::CarType;
pub use core::state::RobotState;
