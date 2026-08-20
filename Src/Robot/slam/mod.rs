//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-20

//! SLAM — 占据栅格建图模块
//!
//! 单 Chunk 地图，LiDAR 点云驱动更新，广播 delta 给 WebSocket。

pub mod grid;
pub mod lidar_mapper;
pub mod odometry;
pub mod task;

pub use grid::{CellState, Delta, OccupancyGrid, CELL_RESOLUTION, CHUNK_SIZE, FREE_CLAMP, OCCUPIED_CLAMP};
pub use lidar_mapper::{update, RobotPose};
pub use task::{MapDelta, SlamContext};
