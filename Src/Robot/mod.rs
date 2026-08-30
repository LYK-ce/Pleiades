//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-30

//! Robot 模块
//!
//! - core/：Robot 核心（主循环、命令、模式、任务队列、执行器）
//! - core/state.rs：全局状态
//! - ugv/：车设备端（stm32/lidar/types）
//! - uav/：机设备端（mavlink）
//! - slam/：占据栅格建图（A3 拆到 ugv）
//! - util/：通用工具（serial）

pub mod core;
pub mod ugv;
pub mod uav;
pub mod util;

pub use core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
pub use core::mode::OpMode;
pub use core::robot::Robot;
pub use ugv::types::CarType;
pub use core::state::RobotState;
