//Presented by KeJi
//Date ： 2026-06-17

//! Robot 模块 —— 机器人控制
//!
//! 基于 tokio-serial，实现 STM32 串口协议（Rosmaster 系列小车）。
//! control 子模块封装协议帧编解码与传感器数据解析。
//! server 子模块提供 WebSocket 遥控服务。

pub mod control;
pub mod server;

use std::sync::Arc;

pub use control::capability::Robot;
pub use control::capability::RobotCapability;

/// 全局 Robot 实例（由 main.rs 在启动时初始化）
static GLOBAL_ROBOT: std::sync::OnceLock<Arc<Robot>> = std::sync::OnceLock::new();

/// 初始化全局 Robot 实例（只在 main.rs 调用一次）
pub fn init_robot(robot: Arc<Robot>) {
    let _ = GLOBAL_ROBOT.set(robot);
}

/// 获取全局 Robot 实例
pub fn get_robot() -> Arc<Robot> {
    GLOBAL_ROBOT.get().expect("Robot 未初始化").clone()
}
