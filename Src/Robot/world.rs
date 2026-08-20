//Presented by KeJi
//Created Date ： 2026-08-20
//Modified Date ： 2026-08-20

//! 世界模块（Task 22 步骤 3）
//!
//! 三子模块：
//! - **static**（静态地图）= `grid`（OccupancyGrid）→ `get_cell`
//! - **dynamic**（动态设备）= `cluster`（ClusterInfoTable）→ `get_agents`
//! - **pathfinding**（寻路）= D* Lite（`Mutex<Option<DStarLite>>`）→ `get_path`
//!
//! 寻路目标由 Rust 侧任务管理「设进 D*」（`set_goal`）；`get_path` 无目标时返回 None。
//! 邻居 4 连通固定（真 3D 的 26 连通留后续，不暴露 get_neighbors）。

use std::sync::{Arc, Mutex};

use tokio::sync::RwLock;

use crate::robot::core::cluster::cluster_info::{ClusterInfo, ClusterInfoTable};
use crate::robot::core::planning::pathfinder::DStarLite;
use crate::robot::slam::{OccupancyGrid, CELL_RESOLUTION};

/// 世界模块：静态地图 + 动态设备 + 寻路
pub struct World {
    /// 静态地图（merged 占据栅格）
    grid: Arc<RwLock<OccupancyGrid>>,
    /// 动态设备（其他车状态）
    cluster: Arc<ClusterInfoTable>,
    /// D* Lite 路径规划器（Task 22：独立锁，跨线程可被 Lua caps 调用）
    pathfinder: Mutex<Option<DStarLite>>,
}

impl World {
    pub fn new(grid: Arc<RwLock<OccupancyGrid>>, cluster: Arc<ClusterInfoTable>) -> Self {
        Self {
            grid,
            cluster,
            pathfinder: Mutex::new(None),
        }
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

    /// 设定寻路目标（网格坐标）：起点 + 目标（任务切换时由 Rust 侧调用）
    pub fn set_goal(&self, start: (i32, i32), goal: (i32, i32)) {
        *self.pathfinder.lock().unwrap() = Some(DStarLite::new(start, goal));
    }

    /// 清空寻路目标（到达/取消时调用）
    pub fn clear_goal(&self) {
        *self.pathfinder.lock().unwrap() = None;
    }

    /// 是否已设置寻路目标
    pub fn has_goal(&self) -> bool {
        self.pathfinder.lock().unwrap().is_some()
    }

    /// 寻路：move_to 当前位置 → 注入动态障碍 → next_step，返回下一格或 None
    ///
    /// 先读 grid（await）再锁 D*（同步），避免 std Mutex 守卫跨 await（!Send）。
    /// 注意：grid 读锁会持有到 next_step 结束（含 D* compute_shortest_path 重算，可能数 ms），
    /// 期间会阻塞 SLAM/MAP_DELTA 的 grid 写锁——单调用者 + 50ms 节拍下可接受。
    pub async fn get_path(
        &self,
        wx: f32,
        wy: f32,
        dynamic_obstacles: &[(i32, i32)],
    ) -> Option<(i32, i32)> {
        let current_gx = (wx / CELL_RESOLUTION).floor() as i32;
        let current_gy = (wy / CELL_RESOLUTION).floor() as i32;
        let grid = self.grid.read().await;
        let mut guard = self.pathfinder.lock().unwrap();
        let pf = guard.as_mut()?;
        pf.move_to((current_gx, current_gy));
        pf.set_dynamic_obstacles(dynamic_obstacles, &grid);
        pf.next_step(&grid)
    }

    /// 急停标记障碍（触发 D* 局部修补）
    pub async fn mark_obstacle(&self, cell: (i32, i32)) {
        let grid = self.grid.read().await;
        if let Some(pf) = self.pathfinder.lock().unwrap().as_mut() {
            pf.mark_obstacle(cell, &grid);
        }
    }
}
