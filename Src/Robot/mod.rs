//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Robot 模块
//!
//! - core/：Robot 核心（主循环、命令）
//! - state.rs：全局状态
//! - websocket/：WebSocket 遥控服务
//! - control/：底层设备驱动

pub mod core;
pub mod state;
pub mod control;
pub mod websocket;

pub use core::command::Command;
pub use core::robot::Robot;
pub use control::types::CarType;
pub use state::RobotState;
