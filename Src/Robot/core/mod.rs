//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-28

//! Robot 核心
//!
//! - command.rs：三层命令定义（Mode / Manual / Auto）
//! - state.rs：全局状态（RobotState + LidarState）
//! - mode.rs：运行模式（Manual / Auto）
//! - mission.rs：任务队列
//! - executor.rs：自动任务执行器（Idle / Turning / Moving）
//! - robot.rs：Robot::launch() + 主 select! 循环

pub mod command;
pub mod protocol;
pub mod state;
pub mod executor;
pub mod mission;
pub mod mode;
pub mod robot;
