//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-30

//! UGV SLAM — 占据栅格建图模块（车端感知）
//!
//! 单 Chunk 地图，LiDAR 点云驱动更新，广播 delta。
//! 地图表示（OccupancyGrid）已迁底座 core/grid.rs（决策 D1）。

pub mod lidar_mapper;
pub mod odometry;
pub mod task;

pub use lidar_mapper::{update, RobotPose};
pub use task::SlamContext;
