//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! 通用串口设备抽象
//!
//! `spawn_port` 启动 TX + RX 两个 tokio task，
//! RX 方向通过回调直接处理，无需中间事件通道。

pub mod port;
