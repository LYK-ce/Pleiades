//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-08-30

//! 世界模块（纯地图信息，Task 23 决策 D1）
//!
//! 两个子模块：
//! - **static**（静态地图）= `grid`（OccupancyGrid）→ `get_cell`
//! - **dynamic**（动态设备）= `cluster`（ClusterInfoTable）→ `get_agents`
//!
//! 寻路（D* Lite）已下沉设备端，由 `GoalService` 直接持有（不再经 world）。

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::robot::core::cluster::cluster_info::{ClusterInfo, ClusterInfoTable};
use crate::robot::core::grid::{OccupancyGrid, CELL_RESOLUTION};

/// 世界模块：静态地图 + 动态设备（纯地图只读接口）
pub struct World {
    /// 静态地图（merged 占据栅格）
    grid: Arc<RwLock<OccupancyGrid>>,
    /// 动态设备（其他车状态）
    cluster: Arc<ClusterInfoTable>,
}

impl World {
    pub fn new(grid: Arc<RwLock<OccupancyGrid>>, cluster: Arc<ClusterInfoTable>) -> Self {
        Self { grid, cluster }
    }

    /// 静态地图查询（不含动态障碍）：`0`=Free / `100`=Occupied / `255`=Unknown，越界 None
    pub async fn get_cell(&self, x: f32, y: f32) -> Option<u8> {
        let gx = (x / CELL_RESOLUTION).floor() as i32;
        let gy = (y / CELL_RESOLUTION).floor() as i32;
        self.grid.read().await.state(gx, gy)
    }

    /// 动态设备快照（其他设备位置/速度）
    pub async fn get_agents(&self) -> Vec<ClusterInfo> {
        self.cluster.snapshot().await
    }
}
