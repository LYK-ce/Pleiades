//Presented by KeJi
//Date ： 2026-06-17

//! STM32 串口协议控制层
//!
//! 参照 ros_stm32_protocol.md 和 my_rossmaster.py 实现。
//! - protocol.rs: 帧构建 / 校验和 / 接收状态机 / 数据解析（纯函数）
//! - types.rs:   机器人状态、车型、运动方向等数据结构
//! - serial_io.rs: 串口后台接收任务

pub mod protocol;
pub mod types;
pub mod serial_io;
pub mod capability;
