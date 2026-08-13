//Presented by KeJi
//Created Date ： 2026-08-13
//Modified Date ： 2026-08-13

//! 他车 → 动态障碍格转换（Task 15）
//!
//! footprint = 1 格（对方所在那一个格，不膨胀；小车直径 0.3m < 格宽 0.5m）。
//! 注入层不做在线/超时判断——表维护由 `cluster/maintenance.rs` 负责，
//! 这里无脑读全量快照。

use std::collections::HashSet;

use crate::robot::core::cluster::ClusterInfo;
use crate::robot::slam::grid::world_to_grid;

/// 把远端车世界坐标映射为障碍格集合（同格自动去重）
pub fn cluster_to_obstacle_cells(others: &[ClusterInfo]) -> HashSet<(i32, i32)> {
    others.iter().map(|info| world_to_grid(info.x, info.y)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn info_at(x: f32, y: f32) -> ClusterInfo {
        ClusterInfo {
            peer_id: vec![0u8],
            x,
            y,
            yaw: 0.0,
            vx: 0.0,
            vy: 0.0,
            time_boot_ms: 0,
            last_seen: Instant::now(),
            sub_target: None,
        }
    }

    #[test]
    fn test_empty_input() {
        assert!(cluster_to_obstacle_cells(&[]).is_empty());
    }

    #[test]
    fn test_single_floor() {
        // 世界 1.0,1.0 → 格 2,2（floor 语义）
        let cells = cluster_to_obstacle_cells(&[info_at(1.0, 1.0)]);
        assert_eq!(cells.len(), 1);
        assert!(cells.contains(&(2, 2)));
    }

    #[test]
    fn test_negative_floor() {
        let cells = cluster_to_obstacle_cells(&[info_at(-0.1, -0.1)]);
        assert!(cells.contains(&(-1, -1)));
    }

    #[test]
    fn test_boundary_half() {
        let cells = cluster_to_obstacle_cells(&[info_at(0.5, 0.5)]);
        assert!(cells.contains(&(1, 1)));
    }

    #[test]
    fn test_same_cell_dedup() {
        let cells = cluster_to_obstacle_cells(&[info_at(1.0, 1.0), info_at(1.2, 1.2)]);
        assert_eq!(cells.len(), 1);
    }

    #[test]
    fn test_multi_distinct() {
        let cells = cluster_to_obstacle_cells(&[info_at(1.0, 1.0), info_at(2.0, 2.0)]);
        assert_eq!(cells.len(), 2);
        assert!(cells.contains(&(2, 2)));
        assert!(cells.contains(&(4, 4)));
    }

    #[test]
    fn test_no_inflation() {
        // 只返回对方所在格，不含相邻格（footprint = 1 格）
        let cells = cluster_to_obstacle_cells(&[info_at(1.0, 1.0)]);
        assert_eq!(cells.len(), 1);
        assert!(cells.contains(&(2, 2)));
        assert!(!cells.contains(&(2, 3)));
        assert!(!cells.contains(&(3, 2)));
    }
}
