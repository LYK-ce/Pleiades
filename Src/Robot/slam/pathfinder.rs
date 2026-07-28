//Presented by KeJi
//Created Date ： 2026-07-28
//Modified Date ： 2026-07-28

//! D* Lite 路径规划器
//!
//! 输出下一格方向（网格坐标），不输出路点列表。
//! 查询时机：仅 sub_target 到达或障碍触发。
//! Unknown 格子视为 Free（乐观）。

use crate::robot::slam::{OccupancyGrid, CHUNK_SIZE};

/// D* Lite 规划器
pub struct DStarLite {
    // TODO: 实现 D* Lite 算法
    // - 维护 rhs/g 值
    // - priority queue（key 比较）
    // - compute_shortest_path()
    // - update_vertex() 局部修补
}

impl DStarLite {
    /// 创建规划器，绑定 OccupancyGrid
    pub fn new(_grid: &OccupancyGrid) -> Self {
        Self {}
    }

    /// 查询下一格方向
    ///
    /// - `start`: 当前位置（网格坐标）
    /// - `goal`: 目标位置（网格坐标）
    ///
    /// 返回：下一格网格坐标，或 None（不可达）
    pub fn next_step(&mut self, _start: (i32, i32), _goal: (i32, i32), _grid: &OccupancyGrid) -> Option<(i32, i32)> {
        // TODO: D* Lite 实现
        // 当前简化：直接返回 goal（直线走）
        Some(_goal)
    }

    /// 通知规划器某格被标记为 Occupied（障碍触发局部修补）
    pub fn mark_obstacle(&mut self, _gx: i32, _gy: i32) {
        // TODO: update_vertex() 局部修补
    }
}
