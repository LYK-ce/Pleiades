//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 点云 → 占据栅格更新（task_31 §五.4.4）
//!
//! **语义照抄真机 `slam/lidar_mapper.rs`，代码是本 crate 自己的**（`pleiades-sim` 禁止引用
//! `pleiades-ugv`，§五.2-1）。流程：
//!
//! 1. 过滤无效距离（`< 0.1 m` 超近噪声 / `> 5.0 m` 精度差）→ 坐标转换（`a = yaw + angle`）
//! 2. 端点去重成 `HashSet`，限制在 `[0, CHUNK_SIZE)` 内
//! 3. **先 hit**：端点格 `+3`（跳过机器人自身格）
//! 4. **再 miss**：Bresenham 从机器人格到端点，遇端点格 `break`（射线截断），每帧每格最多 `−1`
//! 5. **最后动态障碍**：`masked` 每格 `decay(DYNAMIC_OBSTACLE_DECAY)`（抵消他车 `+3`）
//! 6. 收集 `delta != 0` 的 `Delta` 返回
//!
//! 坐标/概率底座全部来自 `pleiades-base`（§五.2-2）：`world_to_grid` / `CELL_RESOLUTION` /
//! `CHUNK_SIZE` / `DYNAMIC_OBSTACLE_DECAY` / `OccupancyGrid` / `Delta`。

use std::collections::HashSet;

use pleiades_base::robot::core::grid::{
    world_to_grid, Delta, OccupancyGrid, CELL_RESOLUTION, CHUNK_SIZE, DYNAMIC_OBSTACLE_DECAY,
};

/// 探测最大有效距离（米）：超过此距离噪声大、精度低，不入图（与真机一致，§五.7-4）
pub const LIDAR_MAX_RANGE: f32 = 5.0;
/// 探测最小有效距离（米）：超近距离反射噪声
pub const LIDAR_MIN_RANGE: f32 = 0.1;

/// 命中点沿射线**外推**的距离（米）—— 栅格量化稳健化（task_31 Step 7 实测修复）
///
/// 世界里的墙是 0.5 m 体素块（`World.gd::_fill_block_world`），墙面因此**恰好落在格边界上**
/// （`wall_c` 南面 y = 92.0 就是格 184 的下边界）。射线返回的碰撞点受浮点误差影响可能略小于
/// 92.0，`floor` 量化便把它算进**墙前那一格**（183）；下一帧又落回 184 —— 该格于是在
/// `Unknown ↔ Occupied` 之间以 ~1 Hz 翻面，D\* 的 `sub_target` 随之在「北」与「南」之间翻面，
/// 车在原地来回转向、**永久冻结**（S3 实测卡在 (36.25, 91.27) 120 s 不动）。
///
/// 命中点沿射线外推 2 cm（格宽 0.5 m 的 4%）后再量化，落点稳定在**被击中的那个体素**里。
/// 语义不受影响：最大/最小距离过滤在加外推**之前**做，2 cm 远小于格宽，不可能跨过薄障碍。
///
/// **偏差 D4（未获人类批准的设计变更）**：§五.4.4 只写「`(x + r·cos a, y + r·sin a)` 后 floor
/// 量化」，本实现多做了一步外推。登记见 `Workbook/wb_31_sim_world_link.md` 的「设计偏差登记表」
/// （现状 / 理由 / 实测证据 / 回退方法 / 待确认），并由单测
/// [`test_hit_epsilon_cannot_cross_thin_obstacle`] 锁住「2 cm 外推不跨薄障碍」。
///
/// `pub`：急停（`sim::emergency_stop`）的障碍格复用**同一个**外推量，保证两处标的格相同。
pub const HIT_EPS_M: f32 = 0.02;

/// 车辆位姿（sim 自己的类型，不 import 任何设备端类型）
#[derive(Debug, Clone, Copy)]
pub struct RobotPose {
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
}

/// 单圈激光扫描更新地图，返回发生变化的格子
///
/// `scan`：`(angle_rad, range_m)`，雷达坐标系（角度不上线，由 `Build_Scan` 按点数生成）
/// `masked`：动态障碍掩蔽格（他车所在格，由 sim 自己的 `cluster_to_obstacle_cells` 算）
pub fn Update(
    grid: &mut OccupancyGrid,
    pose: &RobotPose,
    scan: &[(f32, f32)],
    masked: &HashSet<(i32, i32)>,
) -> Vec<Delta> {
    let robot_gx = (pose.x / CELL_RESOLUTION).floor() as i32;
    let robot_gy = (pose.y / CELL_RESOLUTION).floor() as i32;

    // ---- 1) 坐标变换 + 端点去重（限制在 chunk 内）
    let mut endpoints: HashSet<(i32, i32)> = HashSet::new();
    for &(angle, range) in scan {
        if range < LIDAR_MIN_RANGE || range > LIDAR_MAX_RANGE {
            continue;
        }
        // 命中点沿射线外推 HIT_EPS_M 再量化：墙面落在格边界时不会把「墙前那一格」误标 Occupied
        let (wx, wy) = Laser_To_World(pose, angle, range + HIT_EPS_M);
        let (gx, gy) = world_to_grid(wx, wy);
        if gx >= 0 && gx < CHUNK_SIZE as i32 && gy >= 0 && gy < CHUNK_SIZE as i32 {
            endpoints.insert((gx, gy));
        }
    }
    let mut deltas: Vec<Delta> = Vec::new();
    let mut updated: HashSet<(i32, i32)> = HashSet::new();

    // ---- 2) 先 hit 全部端点（hit 优先于 miss）
    for &(egx, egy) in &endpoints {
        if egx == robot_gx && egy == robot_gy {
            continue; // 机器人自己的格子：超近距离反射噪声，不标占据
        }
        if let Some((_, _, delta)) = grid.update(egx, egy, true) {
            if delta != 0 {
                deltas.push(Delta { gx: egx, gy: egy, delta });
            }
        }
        updated.insert((egx, egy));
    }

    // ---- 3) 再 miss：逐端点画射线，遇端点格截断（被遮挡区域保持 Unknown）
    for &(egx, egy) in &endpoints {
        if egx == robot_gx && egy == robot_gy {
            continue;
        }
        for &(cgx, cgy) in &Ray_Cells(robot_gx, robot_gy, egx, egy) {
            if cgx == robot_gx && cgy == robot_gy {
                continue; // 机器人格不标 Free
            }
            if cgx == egx && cgy == egy {
                break; // 终点格：不标 miss
            }
            if endpoints.contains(&(cgx, cgy)) {
                break; // 遇到本帧其它端点格 → 射线截断（激光被障碍挡住）
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

    // ---- 4) 动态障碍掩蔽：对他车格做 -DYNAMIC_OBSTACLE_DECAY（中和 LiDAR 的 +3）
    for &(mgx, mgy) in masked {
        if let Some((_, _, delta)) = grid.decay(mgx, mgy, DYNAMIC_OBSTACLE_DECAY) {
            if delta != 0 {
                deltas.push(Delta { gx: mgx, gy: mgy, delta });
            }
        }
    }

    deltas
}

/// 雷达极坐标 → 世界笛卡尔坐标（`a = yaw + angle`）
fn Laser_To_World(pose: &RobotPose, angle: f32, range: f32) -> (f32, f32) {
    let a = pose.yaw + angle;
    (pose.x + range * a.cos(), pose.y + range * a.sin())
}

/// Bresenham 画线：返回从 start 到 end 的所有格子（**含两端**）
fn Ray_Cells(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
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
    use pleiades_base::robot::core::grid::CellState;

    fn pose_at(x: f32, y: f32, yaw: f32) -> RobotPose {
        RobotPose { x, y, yaw }
    }

    fn no_mask() -> HashSet<(i32, i32)> {
        HashSet::new()
    }

    /// 出生点（64,64）→ 机器人格 (128,128)；正前方 1 m → 端点格 (130,128)
    const ROBOT_GX: i32 = 128;
    const ROBOT_GY: i32 = 128;

    #[test]
    fn test_laser_to_world() {
        let pose = pose_at(50.0, 50.0, 0.0);
        let (x, y) = Laser_To_World(&pose, 0.0, 1.0);
        assert!((x - 51.0).abs() < 0.01);
        assert!((y - 50.0).abs() < 0.01);
        // yaw=0 时 angle=π/2 → 正右方
        let (x, y) = Laser_To_World(&pose, std::f32::consts::FRAC_PI_2, 1.0);
        assert!((x - 50.0).abs() < 0.01);
        assert!((y - 51.0).abs() < 0.01);
    }

    #[test]
    fn test_ray_cells_straight() {
        let cells = Ray_Cells(0, 0, 3, 0);
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[0], (0, 0));
        assert_eq!(cells[3], (3, 0));
        // 起终点重合只有一格
        assert_eq!(Ray_Cells(5, 5, 5, 5), vec![(5, 5)]);
    }

    #[test]
    fn test_hit_gives_plus_three() {
        let mut grid = OccupancyGrid::new();
        let deltas = Update(&mut grid, &pose_at(64.0, 64.0, 0.0), &[(0.0, 1.0)], &no_mask());
        // 端点格 Δ=+3（射线上的格另有 −1，这里只断言端点）
        assert!(
            deltas.iter().any(|d| d.gx == 130
                && d.gy == 128
                && d.delta == 3),
            "端点格应有 Δ=+3：{deltas:?}"
        );
    }

    #[test]
    fn test_ray_gives_minus_one() {
        let mut grid = OccupancyGrid::new();
        let deltas = Update(&mut grid, &pose_at(64.0, 64.0, 0.0), &[(0.0, 1.0)], &no_mask());
        // 途经格 (129,128)：一次 miss → Δ=−1
        assert!(
            deltas.iter().any(|d| d.gx == 129
                && d.gy == 128
                && d.delta == -1),
            "途经格应有 Δ=−1：{deltas:?}"
        );
        // 机器人自己的格子不标 Free
        assert!(!deltas.iter().any(|d| d.gx == ROBOT_GX && d.gy == ROBOT_GY));
    }

    #[test]
    fn test_out_of_chunk_dropped() {
        let mut grid = OccupancyGrid::new();
        // pose (0.5,0.5) yaw=π：angle=0 → 世界 (-0.5,0.5) → 格 (-1,1)，负坐标必须丢
        let deltas = Update(
            &mut grid,
            &pose_at(0.5, 0.5, std::f32::consts::PI),
            &[(0.0, 1.0)],
            &no_mask(),
        );
        assert!(deltas.is_empty(), "负坐标端点应整体丢弃：{deltas:?}");
        // 越界点不得产生任何 Δ（不靠 grid.update 的越界返回做"丢弃"证明）
        for d in &deltas {
            assert!(
                d.gx >= 0
                    && d.gx < CHUNK_SIZE as i32
                    && d.gy >= 0
                    && d.gy < CHUNK_SIZE as i32,
                "越界格进入 Δ：{d:?}"
            );
        }
    }

    // ─── 命中点外推（HIT_EPS_M）：墙面落在格边界时的量化稳健化 ───────────

    #[test]
    fn test_hit_on_cell_boundary_marks_wall_cell_not_gap() {
        let mut grid = OccupancyGrid::new();
        // pose (0.25,0.25) 朝东，墙面在 x=1.0（= 格 2 的下边界），命中点因浮点误差落在 0.9999
        let deltas = Update(&mut grid, &pose_at(0.25, 0.25, 0.0), &[(0.0, 0.7499)], &no_mask());
        // 墙那一格 (2,0) 必须 +3
        assert!(
            deltas.iter().any(|d| d.gx == 2 && d.gy == 0 && d.delta == 3),
            "墙体的格应有 Δ=+3：{deltas:?}"
        );
        // 墙**前**那一格 (1,0) 只能是 miss（−1），绝不能是 +3
        assert!(
            !deltas.iter().any(|d| d.gx == 1 && d.gy == 0 && d.delta == 3),
            "墙前一格不得被误标 Occupied：{deltas:?}"
        );
        assert!(deltas.iter().any(|d| d.gx == 1 && d.gy == 0 && d.delta == -1));
    }

    #[test]
    fn test_hit_inside_cell_unchanged_by_epsilon() {
        let mut grid = OccupancyGrid::new();
        // 命中点 1.25（格 2 正中）：外推 2 cm 不改变量化结果
        let deltas = Update(&mut grid, &pose_at(0.25, 0.25, 0.0), &[(0.0, 1.0)], &no_mask());
        assert!(deltas.iter().any(|d| d.gx == 2 && d.gy == 0 && d.delta == 3), "{deltas:?}");
        assert!(!deltas.iter().any(|d| d.gx == 3 && d.delta == 3), "不得跨格：{deltas:?}");
    }

    // ─── 偏差 D4 的边界锁定：2 cm 外推**不跨薄障碍**（0.5 m = 1 格）───────────

    #[test]
    fn test_hit_epsilon_cannot_cross_thin_obstacle() {
        // 薄墙 = 1 个体素：格 (2,0) 覆盖 x∈[1.0,1.5)（0.5 m 厚，与世界里 0.5 m 体素块同尺度）。
        // 两个方向各测一次：外推 2 cm 后命中格**必须仍是墙格 (2,0)**，不得越到墙后一格 (3,0)/(1,0)。
        // 若哪天有人把 HIT_EPS_M 调大到 ≥ 格宽，这条会立刻失败。
        let mut grid = OccupancyGrid::new();
        // ① 从西侧打：命中墙面 x=1.0（range 0.75 from x=0.25）；+2 cm → 1.02，仍在 (2,0)
        let deltas = Update(&mut grid, &pose_at(0.25, 0.25, 0.0), &[(0.0, 0.75)], &no_mask());
        assert!(
            deltas.iter().any(|d| d.gx == 2 && d.gy == 0 && d.delta == 3),
            "外推 2 cm 后应仍命中墙格 (2,0)：{deltas:?}"
        );
        assert!(
            !deltas.iter().any(|d| d.gx >= 3 && d.delta == 3),
            "外推不得穿透 0.5 m 薄墙（墙后又出现 +3）：{deltas:?}"
        );

        // ② 从东侧打（yaw=π 朝西）：命中墙背面 x=1.5（range 0.5 from x=2.0）；+2 cm → 1.48，仍在 (2,0)
        let mut grid = OccupancyGrid::new();
        let deltas = Update(
            &mut grid,
            &pose_at(2.0, 0.25, std::f32::consts::PI),
            &[(0.0, 0.5)],
            &no_mask(),
        );
        assert!(
            deltas.iter().any(|d| d.gx == 2 && d.gy == 0 && d.delta == 3),
            "从东侧打时外推 2 cm 也应仍落在墙格 (2,0)：{deltas:?}"
        );
        assert!(
            !deltas.iter().any(|d| d.gx <= 1 && d.delta == 3),
            "外推不得穿透薄墙（墙西侧出现 +3）：{deltas:?}"
        );
        // 外推量本身远小于格宽（把「不跨障碍」的依据写成断言，而非只写在注释里）
        assert!(HIT_EPS_M * 4.0 < CELL_RESOLUTION, "HIT_EPS_M 必须远小于格宽");
    }

    #[test]
    fn test_masked_cell_decays() {
        let mut grid = OccupancyGrid::new();
        let masked: HashSet<(i32, i32)> = [(130, 128)].into_iter().collect();
        for _ in 0..5 {
            let deltas = Update(&mut grid, &pose_at(64.0, 64.0, 0.0), &[(0.0, 1.0)], &masked);
            // 每个 tick 都有负 Δ（decay 生效）
            assert!(
                deltas.iter().any(|d| d.gx == 130 && d.gy == 128 && d.delta < 0),
                "masked 格应有负 Δ（decay）：{deltas:?}"
            );
        }
        // 他车格永远停在 Unknown（+3 被 −3 中和）
        assert_eq!(grid.state(130, 128), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_same_cell_once_per_scan() {
        let mut grid = OccupancyGrid::new();
        // 两个完全相同的点 → 端点集去重 → 端点只 +3 一次
        let deltas = Update(
            &mut grid,
            &pose_at(64.0, 64.0, 0.0),
            &[(0.0, 1.0), (0.0, 1.0)],
            &no_mask(),
        );
        let hits: Vec<_> = deltas.iter().filter(|d| d.gx == 130 && d.gy == 128).collect();
        assert_eq!(hits.len(), 1, "同格同帧只能更新一次：{deltas:?}");
        assert_eq!(hits[0].delta, 3);
    }

    #[test]
    fn test_filters_range_zero_and_beyond_five() {
        let mut grid = OccupancyGrid::new();
        // 0.0 = 无回波；0.05 < min；5.5 / 12.0 > max → 全部过滤，无 Δ
        let deltas = Update(
            &mut grid,
            &pose_at(64.0, 64.0, 0.0),
            &[(0.0, 0.0), (0.0, 0.05), (0.0, 5.5), (0.0, 12.0)],
            &no_mask(),
        );
        assert!(deltas.is_empty(), "越界/无效距离不应产生 Δ：{deltas:?}");
        // 5.0 恰好有效（边界包含）
        let deltas = Update(&mut grid, &pose_at(64.0, 64.0, 0.0), &[(0.0, 5.0)], &no_mask());
        assert!(
            deltas.iter().any(|d| d.gx == 138 && d.gy == 128 && d.delta == 3),
            "5.0 m 端点应有效：{deltas:?}"
        );
    }

    #[test]
    fn test_ray_truncated_by_near_endpoint() {
        let mut grid = OccupancyGrid::new();
        // 近端 1 m + 远端 3 m：远端射线被近端端点截断
        let points = [(0.0, 1.0), (0.0, 3.0)];
        for _ in 0..3 {
            Update(&mut grid, &pose_at(64.0, 64.0, 0.0), &points, &no_mask());
        }
        // 近端/远端都 3 次命中 → Occupied
        assert_eq!(grid.state(130, 128), Some(CellState::Occupied as u8));
        assert_eq!(grid.state(134, 128), Some(CellState::Occupied as u8));
        // 两端点之间被截断 → 保持 Unknown（没被穿行射线标成 Free）
        assert_eq!(grid.state(131, 128), Some(CellState::Unknown as u8));
        assert_eq!(grid.state(132, 128), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_three_hits_become_occupied() {
        let mut grid = OccupancyGrid::new();
        let points = [(0.0, 1.0)];
        // 第 1、2 圈：Unknown（+3 / +6）
        for i in 0..2 {
            Update(&mut grid, &pose_at(64.0, 64.0, 0.0), &points, &no_mask());
            assert_eq!(
                grid.state(130, 128),
                Some(CellState::Unknown as u8),
                "第 {} 圈不应 Occupied",
                i + 1
            );
        }
        // 第 3 圈：+8（clamp）> 6 → Occupied
        Update(&mut grid, &pose_at(64.0, 64.0, 0.0), &points, &no_mask());
        assert_eq!(grid.state(130, 128), Some(CellState::Occupied as u8));
    }
}
