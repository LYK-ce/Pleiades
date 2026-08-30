//Presented by KeJi
//Created Date ： 2026-08-13
//Modified Date ： 2026-08-16

//! 他车 → 动态障碍格转换（Task 15 → Task 17 圆形几何膨胀）
//!
//! footprint = 以他车精确位置为圆心、半径 `radius`（config 缺省 0.2m）的圆盘，
//! 标记所有与圆盘相交的格（格中心 1 格 / 格边 2 格 / 格角最多 4 格）。
//! 注入层不做在线/超时判断——表维护由 `cluster/maintenance.rs` 负责，
//! 这里无脑读全量快照。

use std::collections::HashSet;

use pleiades_base::robot::core::cluster::ClusterInfo;
use pleiades_base::robot::core::grid::{world_to_grid, CELL_RESOLUTION};

/// 把远端车世界坐标映射为障碍格集合（圆盘膨胀，同格自动去重）
///
/// `radius`：膨胀半径（米），config `[Robot].obstacle_inflation_radius`，缺省 0.2。
pub fn cluster_to_obstacle_cells(others: &[ClusterInfo], radius: f32) -> HashSet<(i32, i32)> {
    // 圆盘最多伸入相邻格数：R=0.2 < 格宽 0.5 → reach=1 → 3×3 候选
    let reach = (radius / CELL_RESOLUTION).ceil() as i32;
    let r2 = radius * radius;
    let mut cells = HashSet::new();

    for info in others {
        let (cx, cy) = world_to_grid(info.x, info.y);
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let gx = cx + dx;
                let gy = cy + dy;
                // 格 (gx,gy) 覆盖世界矩形 [x0,x1) × [y0,y1)
                let x0 = gx as f32 * CELL_RESOLUTION;
                let x1 = (gx + 1) as f32 * CELL_RESOLUTION;
                let y0 = gy as f32 * CELL_RESOLUTION;
                let y1 = (gy + 1) as f32 * CELL_RESOLUTION;
                // 圆心到矩形最近距离（分量夹取）
                let px = info.x.clamp(x0, x1);
                let py = info.y.clamp(y0, y1);
                let ddx = info.x - px;
                let ddy = info.y - py;
                if ddx * ddx + ddy * ddy <= r2 {
                    cells.insert((gx, gy));
                }
            }
        }
    }

    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const R: f32 = 0.2;

    fn info_at(x: f32, y: f32) -> ClusterInfo {
        ClusterInfo {
            peer_id: vec![0u8],
            x,
            y,
            z: 0.0,
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
        assert!(cluster_to_obstacle_cells(&[], R).is_empty());
    }

    #[test]
    fn test_center_single_cell() {
        // 车在格中心 (0.25,0.25) → 圆盘 [0.05,0.45]² 完全在格内 → 1 格
        let cells = cluster_to_obstacle_cells(&[info_at(0.25, 0.25)], R);
        assert_eq!(cells, HashSet::from([(0, 0)]));
    }

    #[test]
    fn test_edge_two_cells() {
        // 车靠格边 (0.45,0.25) → 圆盘 x∈[0.25,0.65] 伸入格 (1,0) → 2 格
        let cells = cluster_to_obstacle_cells(&[info_at(0.45, 0.25)], R);
        assert_eq!(cells, HashSet::from([(0, 0), (1, 0)]));
    }

    #[test]
    fn test_corner_four_cells() {
        // 车靠格角 (0.45,0.45) → 圆盘伸入 (1,0)(0,1)(1,1) → 4 格
        let cells = cluster_to_obstacle_cells(&[info_at(0.45, 0.45)], R);
        assert_eq!(cells, HashSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]));
    }

    #[test]
    fn test_tangent_straddling_boundary() {
        // 两车圆心距 30cm 相切、跨边界 0.5 → 两格均被标，巡路绕行无空隙
        let cells = cluster_to_obstacle_cells(&[info_at(0.25, 0.25), info_at(0.55, 0.25)], R);
        assert!(cells.contains(&(0, 0)));
        assert!(cells.contains(&(1, 0)));
    }

    #[test]
    fn test_negative_floor() {
        // 圆心 (-0.1,-0.1) 在格 (-1,-1)，圆盘 [-0.3,0.1]² 伸入 (0,-1)(-1,0)(0,0)
        let cells = cluster_to_obstacle_cells(&[info_at(-0.1, -0.1)], R);
        assert!(cells.contains(&(-1, -1)));
        assert!(cells.contains(&(0, -1)));
        assert!(cells.contains(&(-1, 0)));
        assert!(cells.contains(&(0, 0)));
    }

    #[test]
    fn test_same_cell_dedup() {
        let cells = cluster_to_obstacle_cells(&[info_at(0.25, 0.25), info_at(0.25, 0.25)], R);
        assert_eq!(cells.len(), 1);
    }

    #[test]
    fn test_multi_distinct() {
        // 两车分处远格 → 各自独立膨胀
        let cells = cluster_to_obstacle_cells(&[info_at(0.25, 0.25), info_at(1.25, 1.25)], R);
        assert!(cells.contains(&(0, 0)));
        assert!(cells.contains(&(2, 2)));
    }
}
