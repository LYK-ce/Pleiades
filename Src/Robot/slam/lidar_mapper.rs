//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-15

//! LiDAR 点云 → 占据栅格更新
//!
//! 流程：坐标转换 → HashSet 去重终点 → 先 hit 后 miss（Cartographer 机制）→ 射线截断 → 收集 Delta
//!
//! 更新策略（对齐业界标准）：
//! - 端点格（hit）优先 +3；射线途经格（miss）-1
//! - 射线遇到本帧端点格即截断（激光被障碍挡住），被遮挡区域保持 Unknown
//! - 每帧每格最多更新一次

use std::collections::HashSet;

use super::grid::{
    world_to_grid, CellState, Delta, OccupancyGrid, CELL_RESOLUTION, CHUNK_SIZE,
    DYNAMIC_OBSTACLE_DECAY,
};

/// 车辆位姿
pub struct RobotPose {
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
}

/// 单圈 LiDAR 扫描更新地图，返回变化的格子列表
pub fn update(
    grid: &mut OccupancyGrid,
    pose: &RobotPose,
    scan_points: &[(f32, f32)], // (angle_rad, range_m)，雷达坐标系
    masked: &HashSet<(i32, i32)>, // 动态障碍掩蔽格（他车所在格，Task 15 C 节）
) -> Vec<Delta> {
    // 1. 坐标转换 + 去重终点格子
    let robot_gx = (pose.x / CELL_RESOLUTION).floor() as i32;
    let robot_gy = (pose.y / CELL_RESOLUTION).floor() as i32;

    let mut endpoints: HashSet<(i32, i32)> = HashSet::new();
    for &(angle, range) in scan_points {
        if range < 0.1 || range > 12.0 {
            // Tmini 有效量程 0.1~12m，过滤无效/噪声点
            continue;
        }
        let (wx, wy) = laser_to_world(pose, angle, range);
        let (gx, gy) = world_to_grid(wx, wy);
        // 限制在 chunk 范围内
        if gx >= 0 && gx < CHUNK_SIZE as i32 && gy >= 0 && gy < CHUNK_SIZE as i32 {
            endpoints.insert((gx, gy));
        }
    }

    // 2. 先 hit 全部端点（hit 优先于 miss，Cartographer 机制）
    let mut deltas: Vec<Delta> = Vec::new();
    let mut updated: HashSet<(i32, i32)> = HashSet::new();
    for &(egx, egy) in &endpoints {
        // 跳过机器人自己的格子（超近距离反射噪声）
        if egx == robot_gx && egy == robot_gy {
            continue;
        }
        if let Some((_, _, delta)) = grid.update(egx, egy, true) {
            if delta != 0 {
                deltas.push(Delta { gx: egx, gy: egy, delta });
            }
        }
        updated.insert((egx, egy));
    }

    // 3. miss：逐端点画射线，遇到本帧端点格即截断（被遮挡保持 Unknown）
    for &(egx, egy) in &endpoints {
        if egx == robot_gx && egy == robot_gy {
            continue;
        }
        let cells = bresenham(robot_gx, robot_gy, egx, egy);
        for &(cgx, cgy) in &cells {
            if cgx == robot_gx && cgy == robot_gy {
                continue; // 机器人格不标 Free
            }
            if cgx == egx && cgy == egy {
                break; // 终点格：不标 miss
            }
            if endpoints.contains(&(cgx, cgy)) {
                break; // 遇到本帧其他端点格 → 射线截断（激光被障碍挡住）
            }
            if !updated.insert((cgx, cgy)) {
                continue; // 每帧每格最多一次 miss
            }
            if let Some((_, _, delta)) = grid.update(cgx, cgy, false) {
                if delta != 0 {
                    deltas.push(Delta { gx: cgx, gy: cgy, delta });
                }
            }
        }
    }

    // 4. 动态障碍掩蔽（Task 15 C 节）：对「他车格」做 -DYNAMIC_OBSTACLE_DECAY 抵消
    //    - 中和 LiDAR 的 +3，他车格稳定在 Unknown（log ≤ 5），永不 Occupied；
    //    - 也能清历史残留（+8 被 clamp 顶住后 -3 真实降 3 → 5）。
    for &(mgx, mgy) in masked {
        if let Some((_, _, delta)) = grid.decay(mgx, mgy, DYNAMIC_OBSTACLE_DECAY) {
            if delta != 0 {
                deltas.push(Delta { gx: mgx, gy: mgy, delta });
            }
        }
    }

    deltas
}

/// 雷达极坐标 → 世界笛卡尔坐标
fn laser_to_world(pose: &RobotPose, angle: f32, range: f32) -> (f32, f32) {
    let a = pose.yaw + angle;
    (pose.x + range * a.cos(), pose.y + range * a.sin())
}

/// Bresenham 画线算法，返回从 start 到 end 的所有格子（含两端）
fn bresenham(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let mut cells = Vec::new();
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut x = x0;
    let mut y = y0;

    loop {
        cells.push((x, y));
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if x == x1 {
                break;
            }
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            if y == y1 {
                break;
            }
            err += dx;
            y += sy;
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_laser_to_world() {
        let pose = RobotPose { x: 50.0, y: 50.0, yaw: 0.0 };
        // 正前方 1m
        let (x, y) = laser_to_world(&pose, 0.0, 1.0);
        assert!((x - 51.0).abs() < 0.01);
        assert!((y - 50.0).abs() < 0.01);

        // 正右方 1m (yaw=0, angle=π/2)
        let (x, y) = laser_to_world(&pose, std::f32::consts::FRAC_PI_2, 1.0);
        assert!((x - 50.0).abs() < 0.01);
        assert!((y - 51.0).abs() < 0.01);
    }

    #[test]
    fn test_bresenham_straight() {
        let cells = bresenham(0, 0, 3, 0);
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[0], (0, 0));
        assert_eq!(cells[3], (3, 0));
    }

    #[test]
    fn test_bresenham_already_at_target() {
        let cells = bresenham(5, 5, 5, 5);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0], (5, 5));
    }

    #[test]
    fn test_ray_terminates_at_endpoint() {
        let mut grid = OccupancyGrid::new();
        let pose = RobotPose { x: 64.0, y: 64.0, yaw: 0.0 };
        // 近处端点 (130,128)（1m）挡住后方
        let points = vec![(0.0, 1.0)];
        for _ in 0..3 {
            update(&mut grid, &pose, &points, &std::collections::HashSet::new());
        }
        // 端点格 3 次命中 → Occupied
        assert_eq!(grid.state(130, 128), Some(CellState::Occupied as u8));
        // 射线截断后的格（131,128 起）从未更新 → 保持 Unknown
        assert_eq!(grid.state(131, 128), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_endpoint_not_erased_by_through_ray() {
        let mut grid = OccupancyGrid::new();
        let pose = RobotPose { x: 64.0, y: 64.0, yaw: 0.0 };
        // 近端点 (130,128)（1m）+ 远端点 (140,128)（6m）
        // 远射线 (128→140) 经过 (130,128)：旧代码会用它的 miss 抵消近端点（回归测试）
        let points = vec![(0.0, 1.0), (0.0, 6.0)];
        for _ in 0..3 {
            update(&mut grid, &pose, &points, &std::collections::HashSet::new());
        }
        // 近端点格：3 圈 +3×3=9 → Occupied（不被穿行射线抵消）
        assert_eq!(grid.state(130, 128), Some(CellState::Occupied as u8));
        // 远端点格：也是端点 → Occupied
        assert_eq!(grid.state(140, 128), Some(CellState::Occupied as u8));
        // 两端点之间的格：射线被 (130,128) 截断 → 保持 Unknown
        assert_eq!(grid.state(131, 128), Some(CellState::Unknown as u8));
        assert_eq!(grid.state(135, 128), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_update_basic() {
        let mut grid = OccupancyGrid::new();
        let pose = RobotPose { x: 64.0, y: 64.0, yaw: 0.0 };
        let points = vec![(0.0, 0.5)];

        let center_gx: i32 = 128;
        let center_gy: i32 = 128;

        // 初始：Unknown
        assert_eq!(grid.state(center_gx, center_gy), Some(CellState::Unknown as u8));


        // Δ 语义（Task 13_2）：每圈终点格都有 Δ=+3 → deltas 非空（原三态语义下前 2 圈为空）
        for i in 0..2 {
            let deltas = update(&mut grid, &pose, &points, &std::collections::HashSet::new());
            assert!(!deltas.is_empty(), "第 {} 圈终点应有 Δ", i + 1);
            // 终点格 Δ=+3，射线途经格 Δ=−1
            assert!(deltas.iter().any(|d| d.gx == center_gx + 1 && d.gy == center_gy && d.delta == 3));
        }

        // 第 3 圈：终点 +3×3=9 → 夹断 8 > 6 → Occupied；clamp 边界 Δ=+2
        let deltas = update(&mut grid, &pose, &points, &std::collections::HashSet::new());
        assert!(!deltas.is_empty());
        assert!(deltas.iter().any(|d| d.gx == center_gx + 1 && d.gy == center_gy && d.delta == 2));
        assert_eq!(grid.state(center_gx + 1, center_gy), Some(CellState::Occupied as u8));
    }

    #[test]
    fn test_masked_decay_neutralizes_hit() {
        let mut grid = OccupancyGrid::new();
        let pose = RobotPose { x: 64.0, y: 64.0, yaw: 0.0 };
        // 端点 (130,128)（1m）被 mask 为他车格
        let points = vec![(0.0, 1.0)];
        let masked: std::collections::HashSet<(i32, i32)> = [(130, 128)].into_iter().collect();
        // 多圈 hit + 每圈 -3 抵消：端点格永远不达 Occupied
        for _ in 0..5 {
            update(&mut grid, &pose, &points, &masked);
        }
        assert_eq!(grid.state(130, 128), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_masked_decay_clears_history_occupied() {
        let mut grid = OccupancyGrid::new();
        let pose = RobotPose { x: 64.0, y: 64.0, yaw: 0.0 };
        let points = vec![(0.0, 1.0)];
        // 先不加 mask，3 圈命中 → Occupied
        for _ in 0..3 {
            update(&mut grid, &pose, &points, &std::collections::HashSet::new());
        }
        assert_eq!(grid.state(130, 128), Some(CellState::Occupied as u8));
        // 加 mask 后，一帧 -3 降到 5（Unknown），清历史残留
        let masked: std::collections::HashSet<(i32, i32)> = [(130, 128)].into_iter().collect();
        update(&mut grid, &pose, &points, &masked);
        assert_eq!(grid.state(130, 128), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_masked_decay_does_not_affect_unmasked() {
        let mut grid = OccupancyGrid::new();
        let pose = RobotPose { x: 64.0, y: 64.0, yaw: 0.0 };
        let points = vec![(0.0, 1.0)];
        // masked 是无关格 (200,200)，端点 (130,128) 不受影响
        let masked: std::collections::HashSet<(i32, i32)> = [(200, 200)].into_iter().collect();
        for _ in 0..3 {
            update(&mut grid, &pose, &points, &masked);
        }
        // 端点格正常 3 圈 → Occupied（未被误掩蔽）
        assert_eq!(grid.state(130, 128), Some(CellState::Occupied as u8));
    }
}
