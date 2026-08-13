//Presented by KeJi
//Created Date ： 2026-08-12
//Modified Date ： 2026-08-12

//! 任务分配 — 群发 Goto 分布式散布位置计算（Task 14）
//!
//! 确定性分配：所有车对同一输入（目标点 + members 列表）按同一规则计算，
//! 得到同一份位置列表 L，各自取自己序号对应槽位 → 零通信、零协商、零冲突。
//!
//! 规则（定稿 2026-08-12）：
//! - L[0] = 精确目标点（头车）；L[1..] = 棋盘同色格 `(gx+gy)%2==0` 的格中心
//! - 切比雪夫距离环由内向外枚举（环内固定顺序：正上方起顺时针）
//! - Occupied / OOB 格同等跳过（继续往后取，不换序）
//! - 目标格 Occupied → 不可达（不偏移）；环上限 10（≈5m）

use crate::robot::slam::grid::world_to_grid;
use crate::robot::slam::{CellState, OccupancyGrid, CELL_RESOLUTION, CHUNK_SIZE};

/// 分配错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignError {
    /// 目标格为障碍（Occupied）→ 不可达（不偏移）
    Unreachable,
    /// 目标点越界（超出地图范围）
    OutOfBounds,
    /// 自己的 peer_id 不在成员列表
    NotMember,
    /// 槽位不足（环上限内合法同色格不够）
    InsufficientSlots,
}

/// 最大枚举环数（r=1..=10 同色格共 220 + L[0] = 221 槽，≈5m 半径）
const MAX_RING: i32 = 10;

/// 网格坐标 → 世界坐标（格中心，与 executor `cell_center_world` 公式一致：(gx+0.5)*0.5）
fn cell_center_world(gx: i32, gy: i32) -> (f32, f32) {
    ((gx as f32 + 0.5) * CELL_RESOLUTION, (gy as f32 + 0.5) * CELL_RESOLUTION)
}

/// 切比雪夫环 r 的全部格，固定顺序：正上方 (0,-r) 起顺时针一周
///
/// 确定性是分配正确性的前提——禁止 HashMap / 排序不稳定枚举。
fn ring_cells(r: i32) -> Vec<(i32, i32)> {
    let mut cells = Vec::with_capacity(8 * r as usize);
    // 顶边右半（含右上角）：(0..=r, -r)
    for x in 0..=r {
        cells.push((x, -r));
    }
    // 右边（不含右上角，含右下角）：(r, -r+1..=r)
    for y in (-r + 1)..=r {
        cells.push((r, y));
    }
    // 底边（不含右下角，含左下角）：(r-1..=-r, r)
    for x in (0..r).rev() {
        cells.push((x, r));
    }
    for x in (-r..0).rev() {
        cells.push((x, r));
    }
    // 左边（不含左下角，含左上角）：(-r, r-1..=-r)
    for y in (0..r).rev() {
        cells.push((-r, y));
    }
    for y in (-r..0).rev() {
        cells.push((-r, y));
    }
    // 顶边左半（不含左上角）：(-r+1..=-1, -r)
    for x in (-r + 1..0).rev() {
        cells.push((x, -r));
    }
    cells
}

/// 构建散布位置列表 L
///
/// - L[0] = 精确目标点（头车，N=1 时即此）
/// - L[1..] = 同色格中心，环由内向外；Occupied / OOB 跳过
pub fn build_slots(
    target: (f32, f32),
    grid: &OccupancyGrid,
) -> Result<Vec<(f32, f32)>, AssignError> {
    let (tgx, tgy) = world_to_grid(target.0, target.1);
    // 目标点越界 → 提前失败
    if tgx < 0 || tgx >= CHUNK_SIZE as i32 || tgy < 0 || tgy >= CHUNK_SIZE as i32 {
        return Err(AssignError::OutOfBounds);
    }
    // 目标格为障碍 → 不可达（不偏移）
    if grid.state(tgx, tgy) == Some(CellState::Occupied as u8) {
        return Err(AssignError::Unreachable);
    }

    let mut slots = vec![target];
    for r in 1..=MAX_RING {
        for (dx, dy) in ring_cells(r) {
            let gx = tgx + dx;
            let gy = tgy + dy;
            // 棋盘同色格：(gx+gy) 为偶数
            if (gx + gy) % 2 != 0 {
                continue;
            }
            // 越界 / 障碍：同等跳过，继续往后取
            if gx < 0 || gx >= CHUNK_SIZE as i32 || gy < 0 || gy >= CHUNK_SIZE as i32 {
                continue;
            }
            if grid.state(gx, gy) == Some(CellState::Occupied as u8) {
                continue;
            }
            slots.push(cell_center_world(gx, gy));
        }
    }
    Ok(slots)
}

/// 计算本车在群发任务中的目标点
///
/// - `members` 空 → 老单车语义：直接返回 target（不做 Occupied 预检，保持老行为）
/// - `members` 非空 → 排序（字节升序，**不信任帧序**）→ 序号 i → L[i]
pub fn group_goto_mission(
    target: (f32, f32),
    members: &[Vec<u8>],
    own_peer_id: &[u8],
    grid: &OccupancyGrid,
) -> Result<(f32, f32), AssignError> {
    if members.is_empty() {
        return Ok(target);
    }
    let mut sorted: Vec<&Vec<u8>> = members.iter().collect();
    sorted.sort();
    let idx = sorted
        .iter()
        .position(|m| m.as_slice() == own_peer_id)
        .ok_or(AssignError::NotMember)?;
    let slots = build_slots(target, grid)?;
    slots.get(idx).copied().ok_or(AssignError::InsufficientSlots)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造地图，指定格设为 Occupied（log-odds 8 > 阈值 6）
    fn grid_with_obstacles(obs: &[(i32, i32)]) -> OccupancyGrid {
        let mut g = OccupancyGrid::new();
        let mut data = Box::new([0i8; CHUNK_SIZE * CHUNK_SIZE]);
        for &(gx, gy) in obs {
            let idx = (gy * CHUNK_SIZE as i32 + gx) as usize;
            data[idx] = 8;
        }
        g.set_log_odds(&data[..]);
        g
    }

    #[test]
    fn test_ring_cells_count_and_unique() {
        for r in 1..=10 {
            let cells = ring_cells(r);
            assert_eq!(cells.len(), 8 * r as usize, "环 {r} 格数");
            assert!(cells.iter().all(|&(dx, dy)| dx.abs().max(dy.abs()) == r));
            // 无重复
            let mut set = std::collections::HashSet::new();
            for c in &cells {
                assert!(set.insert(*c), "环 {r} 重复格 {c:?}");
            }
            // 固定顺序（确定性）
            assert_eq!(ring_cells(r), cells);
        }
    }

    #[test]
    fn test_single_member_returns_exact_target() {
        // member=1：直接精确目标点（L[0]）
        let g = OccupancyGrid::new();
        let own = vec![1u8, 2, 3];
        let target = (64.0, 64.0);
        assert_eq!(
            group_goto_mission(target, &[own.clone()], &own, &g).unwrap(),
            target
        );
        // members 空（老单车语义）同样直接返回
        assert_eq!(group_goto_mission(target, &[], &own, &g).unwrap(), target);
    }

    #[test]
    fn test_three_members_assign_ring1() {
        // 目标格 (128,128)（世界 64.0,64.0），3 车 → L[1]、L[2] 为环 1 对角同色格中心
        let g = OccupancyGrid::new();
        let a = vec![1u8];
        let b = vec![2u8];
        let c = vec![3u8];
        let members = vec![a.clone(), b.clone(), c.clone()]; // 无序输入
        let target = (64.0, 64.0);
        let slots = build_slots(target, &g).unwrap();
        assert_eq!(slots.len() >= 3, true);
        // 头车精确
        assert_eq!(slots[0], target);
        // 环1 对角格中心（世界坐标）：
        // 格 (127,127) → (63.75, 63.75)；格 (129,127) → (64.75, 63.75)；(127,129)、(129,129)
        // 各车取自己的槽位，且全部同色
        for m in &members {
            let own = group_goto_mission(target, &members, m, &g).unwrap();
            let (gx, gy) = world_to_grid(own.0, own.1);
            assert_eq!((gx + gy) % 2, 0, "散布格必须同色");
        }
        // 排序后序号: a=1 → L[1], b=2 → L[2], c=3 → L[3]?（3 车序号 0,1,2 → L[0],L[1],L[2]）
        assert_eq!(group_goto_mission(target, &members, &a, &g).unwrap(), slots[0]);
        assert_eq!(group_goto_mission(target, &members, &b, &g).unwrap(), slots[1]);
        assert_eq!(group_goto_mission(target, &members, &c, &g).unwrap(), slots[2]);
    }

    #[test]
    fn test_obstacle_skip_keeps_order() {
        // 目标格 (128,128)；把环1 第一个同色格(127,127) 置障碍 → 跳过，L[1] 变为下一格
        let g = grid_with_obstacles(&[(127, 127)]);
        let target = (64.0, 64.0);
        let slots = build_slots(target, &g).unwrap();
        // 无障碍版本 L[1] = (127,127) 中心 = (63.75, 63.75)；有障碍版应跳过
        assert_eq!(slots[0], target);
        let blocked_center = ((127i32 as f32 + 0.5) * CELL_RESOLUTION, (127i32 as f32 + 0.5) * CELL_RESOLUTION);
        assert_ne!(slots[1], blocked_center, "障碍格必须被跳过");
        // 第二个同色格 (129,127) 应为新 L[1]
        let expect = ((129i32 as f32 + 0.5) * CELL_RESOLUTION, (127i32 as f32 + 0.5) * CELL_RESOLUTION);
        assert_eq!(slots[1], expect);
    }

    #[test]
    fn test_target_occupied_is_unreachable() {
        let g = grid_with_obstacles(&[(128, 128)]);
        let r = group_goto_mission((64.0, 64.0), &[vec![1u8]], &[1u8], &g);
        assert_eq!(r, Err(AssignError::Unreachable));
        assert_eq!(build_slots((64.0, 64.0), &g), Err(AssignError::Unreachable));
    }

    #[test]
    fn test_target_out_of_bounds() {
        let g = OccupancyGrid::new();
        // 世界坐标 200m → 格 400 > 255
        assert_eq!(build_slots((200.0, 200.0), &g), Err(AssignError::OutOfBounds));
    }

    #[test]
    fn test_insufficient_slots() {
        // 大量障碍占满环1~环10 的同色格 → slots 只有 L[0] → N=2 时 InsufficientSlots
        let mut obs = Vec::new();
        for r in 1..=10 {
            for (dx, dy) in ring_cells(r) {
                if (128 + dx + 128 + dy) % 2 == 0 {
                    obs.push((128 + dx, 128 + dy));
                }
            }
        }
        let g = grid_with_obstacles(&obs);
        let slots = build_slots((64.0, 64.0), &g).unwrap();
        assert_eq!(slots.len(), 1, "所有同色格被占 → 仅剩头车槽");
        let r = group_goto_mission(
            (64.0, 64.0),
            &[vec![1u8], vec![2u8]],
            &[2u8],
            &g,
        );
        assert_eq!(r, Err(AssignError::InsufficientSlots));
    }

    #[test]
    fn test_determinism() {
        let g = grid_with_obstacles(&[(127, 127), (129, 129)]);
        let members = vec![vec![9u8; 38], vec![3u8; 38], vec![7u8; 38], vec![1u8; 38]];
        let target = (64.0, 64.0);
        let a = group_goto_mission(target, &members, &vec![3u8; 38], &g).unwrap();
        let b = group_goto_mission(target, &members, &vec![3u8; 38], &g).unwrap();
        assert_eq!(a, b, "同输入必须得到相同结果");
    }

    #[test]
    fn test_not_member() {
        let g = OccupancyGrid::new();
        let members = vec![vec![1u8], vec![2u8]];
        let r = group_goto_mission((64.0, 64.0), &members, &[9u8], &g);
        assert_eq!(r, Err(AssignError::NotMember));
    }

    #[test]
    fn test_cell_center_matches_executor_formula() {
        // 与 executor::cell_center_world((gx+0.5)*0.5) 交叉断言
        for (gx, gy) in [(100, 100), (127, 127), (200, 50)] {
            let (wx, wy) = cell_center_world(gx, gy);
            assert_eq!(wx, (gx as f32 + 0.5) * 0.5);
            assert_eq!(wy, (gy as f32 + 0.5) * 0.5);
        }
    }
}
