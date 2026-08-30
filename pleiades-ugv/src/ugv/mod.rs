//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! UGV（车）设备端模块
//!
//! - types.rs：CarType 等车类型定义
//! - stm32/：STM32 底盘驱动
//! - lidar/：YDLIDAR 雷达驱动

pub mod types;
pub mod stm32;
pub mod lidar;
pub mod slam;
pub mod executor;
pub mod goal;
pub mod emergency_stop;
pub mod planning;
pub mod robot_handler;
