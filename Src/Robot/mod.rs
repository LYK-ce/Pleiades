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

/// 全局 Robot 单例（Lua 绑定和 WS 服务器共享同一个实例）
static GLOBAL_ROBOT: std::sync::OnceLock<Arc<Robot>> = std::sync::OnceLock::new();

/// 获取全局 Robot 实例
pub fn get_robot() -> Arc<Robot> {
    GLOBAL_ROBOT.get_or_init(|| Arc::new(Robot::new())).clone()
}
