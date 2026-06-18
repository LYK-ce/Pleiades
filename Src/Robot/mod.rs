//Presented by KeJi
//Date ： 2026-06-17

//! Robot 模块 —— 机器人控制
//!
//! 基于 tokio-serial，实现 STM32 串口协议（Rosmaster 系列小车）。
//! control 子模块封装协议帧编解码与传感器数据解析。
//! server 子模块提供 WebSocket 遥控服务。

pub mod control;
pub mod server;
