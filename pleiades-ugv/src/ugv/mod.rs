//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-27

//! UGV（车）设备端模块
//!
//! - types.rs：CarType 等车类型定义
//! - stm32/：STM32 底盘驱动
//! - lidar/：YDLIDAR 雷达驱动
//! - nav/：导航层（navigator + pathfinder + cluster_obstacles）
//! - mission/：任务层（Mission → 树分派 + Goto 树）

pub mod types;
pub mod stm32;
pub mod lidar;
pub mod slam;
pub mod executor;
pub mod nav;
pub mod mission;
pub mod emergency_stop;
pub mod robot_handler;
