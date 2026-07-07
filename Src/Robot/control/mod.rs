//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Robot 控制层
//!
//! - serial/：通用串口抽象（ProtocolUnpack trait, spawn_port）
//! - device/：各设备驱动实现（stm32, ...）
//! - types.rs：共用数据类型

pub mod types;
pub mod serial;
pub mod device;
