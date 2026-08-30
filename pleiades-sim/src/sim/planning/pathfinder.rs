//Presented by KeJi
//Created Date ： 2026-07-31
//Modified Date ： 2026-08-13

//! D* Lite 增量路径规划器
//!
//! 基于 Sven Koenig & Maxim Likhachev (2002) 算法。
//! 4 连通网格，Manhattan 距离启发式。
//! 障碍通过 mark_obstacle() 增量修补，无需每次从头搜索。

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::cmp::Ordering;
use tracing::{info, warn};

use pleiades_base::robot::core::grid::{CellState, OccupancyGrid, CHUNK_SIZE};

/// compute_shortest_path 最大迭代次数（P3：极端地图降级，避免拖垮 50ms tick）
const MAX_COMPUTE_ITERS: u32 = 10_000;

// ============================================================
// Key — D* Lite 优先队列排序键 (k1, k2)
// ============================================================

/// D* Lite 优先队列排序键
///
/// 按 (k1, k2) 字典序比较，值越小优先级越高。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    k1: f32,
    k2: f32,
}

impl Eq for Key {}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        other.k1.partial_cmp(&self.k1).unwrap_or(Ordering::Equal)
            .then_with(|| other.k2.partial_cmp(&self.k2).unwrap_or(Ordering::Equal))
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ============================================================
// DStarLite 规划器
// ============================================================

/// D* Lite 增量路径规划器
///
/// 生命周期：pop 新 Mission 时创建，到达 goal 或不可达时丢弃。
pub struct DStarLite {
    /// g-value: 已知到 goal 的最短距离
    g: HashMap<(i32, i32), f32>,
    /// rhs-value: 一步前瞻值
    rhs: HashMap<(i32, i32), f32>,
    /// 优先队列（最小堆）
    u: BinaryHeap<(Key, (i32, i32))>,
    /// 启发式偏移累积
    km: f32,
    /// 当前位置（网格坐标）
    start: (i32, i32),
    /// 目标位置（网格坐标）
    goal: (i32, i32),
    /// 他车动态障碍格（Task 15：每次寻路前由 GoalService 注入；footprint = 1 格）
    dynamic_obstacles: HashSet<(i32, i32)>,
}

impl DStarLite {
    /// 创建规划器，设定起点和目标
    pub fn new(start: (i32, i32), goal: (i32, i32)) -> Self {
        let mut slf = Self {
            g: HashMap::new(),
            rhs: HashMap::new(),
            u: BinaryHeap::new(),
            km: 0.0,
            start,
            goal,
            dynamic_obstacles: HashSet::new(),
        };
        slf.initialize();
        info!("[D*] 创建规划器: start=({},{}) goal=({},{})", start.0, start.1, goal.0, goal.1);
        slf
    }

    // ─── 公有接口 ─────────────────────────────────

    /// 查询下一格方向
    ///
    /// 返回 4 连通邻居中 `cost + g` 最小的格子，或 None（不可达/已到达）。
    pub fn next_step(&mut self, grid: &OccupancyGrid) -> Option<(i32, i32)> {
        info!("[D*] next_step: start=({},{}) goal=({},{}) km={:.1}",
            self.start.0, self.start.1, self.goal.0, self.goal.1, self.km);
        if self.start == self.goal {
            warn!("[D*] start==goal，返回 None");
            return None;
        }
        self.compute_shortest_path(grid);
        if self.rhs_val(self.start) >= f32::MAX / 2.0 {
            warn!("[D*] rhs[start]=INF，不可达");
            return None;
        }
        // 找邻居中 c(s,s')+g(s') 最小的
        let mut best: Option<((i32, i32), f32)> = None;
        for n in Self::neighbors(self.start) {
            let cost = self.cost(grid, self.start, n);
            if cost >= f32::MAX / 2.0 { continue; }
            let val = cost + self.g_val(n);
            if best.is_none() || val < best.unwrap().1 {
                best = Some((n, val));
            }
        }
        best.map(|(cell, _)| cell)
    }

    /// 更新起点（机器人移动了一步）
    pub fn move_to(&mut self, new_start: (i32, i32)) {
        let old = self.start;
        self.km += Self::heuristic(old, new_start);
        self.start = new_start;
        info!("[D*] move_to: ({},{})->({},{}) km={:.1}", old.0, old.1, new_start.0, new_start.1, self.km);
    }

    /// 标记障碍格（会触发局部修补）
    pub fn mark_obstacle(&mut self, cell: (i32, i32), grid: &OccupancyGrid) {
        info!("[D*] mark_obstacle: ({},{})", cell.0, cell.1);
        for n in Self::neighbors(cell) {
            if self.has_rhs(n) {
                self.update_vertex(n, grid);
            }
        }
        self.update_vertex(cell, grid);
    }

    /// 注入最新他车动态障碍格（Task 15）
    ///
    /// 对 old/new 集合做 diff，仅对变更格触发局部修补（先邻居、后自身，
    /// 复用 mark_obstacle 的修补机制）。集合未变时直接返回，零开销。
    pub fn set_dynamic_obstacles(&mut self, cells: &[(i32, i32)], grid: &OccupancyGrid) {
        let new: HashSet<(i32, i32)> = cells.iter().copied().collect();
        if new == self.dynamic_obstacles {
            return;
        }
        // 先置换为 new，使修补期间的 cost() 能看到最新障碍集合（否则新增格查不到、邻居 rhs 偏低）
        let old = std::mem::replace(&mut self.dynamic_obstacles, new);
        let changed: Vec<(i32, i32)> = old.symmetric_difference(&self.dynamic_obstacles).copied().collect();
        for cell in changed {
            for n in Self::neighbors(cell) {
                if self.has_rhs(n) {
                    self.update_vertex(n, grid);
                }
            }
            self.update_vertex(cell, grid);
        }
    }

    // ─── 私有核心算法 ─────────────────────────────

    /// 初始化：清空队列，goal rhs=0，入队
    fn initialize(&mut self) {
        self.u.clear();
        self.km = 0.0;
        self.rhs.clear();
        self.g.clear();
        self.rhs.insert(self.goal, 0.0);
        let key = self.calc_key(self.goal, 0.0, 0.0);
        self.u.push((key, self.goal));
    }

    /// 计算节点 key = (min(g, rhs) + h(start, cell) + km, min(g, rhs))
    fn calc_key(&self, cell: (i32, i32), g: f32, rhs: f32) -> Key {
        let m = g.min(rhs);
        let h = Self::heuristic(self.start, cell);
        Key { k1: m + h + self.km, k2: m }
    }

    /// Manhattan 距离
    fn heuristic(a: (i32, i32), b: (i32, i32)) -> f32 {
        ((a.0 - b.0).abs() + (a.1 - b.1).abs()) as f32
    }

    /// 4 连通邻居
    fn neighbors(cell: (i32, i32)) -> [(i32, i32); 4] {
        let (x, y) = cell;
        [(x, y - 1), (x + 1, y), (x, y + 1), (x - 1, y)]
    }

    /// 移动代价：Free/Unknown=1，Occupied=∞，出界=∞
    fn cost(&self, grid: &OccupancyGrid, _from: (i32, i32), to: (i32, i32)) -> f32 {
        if self.dynamic_obstacles.contains(&to) {
            return f32::MAX;
        }
        if to.0 < 0 || to.0 >= CHUNK_SIZE as i32
            || to.1 < 0 || to.1 >= CHUNK_SIZE as i32
        {
            return f32::MAX;
        }
        match grid.state(to.0, to.1) {
            Some(val) if val == CellState::Occupied as u8 => f32::MAX,
            _ => 1.0,
        }
    }

    // ─── 值访问（不存在 = ∞）─────────────

    fn g_val(&self, cell: (i32, i32)) -> f32 {
        *self.g.get(&cell).unwrap_or(&f32::MAX)
    }

    fn rhs_val(&self, cell: (i32, i32)) -> f32 {
        *self.rhs.get(&cell).unwrap_or(&f32::MAX)
    }

    fn has_rhs(&self, cell: (i32, i32)) -> bool {
        self.rhs.contains_key(&cell)
    }

    // ─── update_vertex ─────────────────────────

    fn update_vertex(&mut self, cell: (i32, i32), grid: &OccupancyGrid) {
        if cell != self.goal {
            let mut best = f32::MAX;
            for n in Self::neighbors(cell) {
                let c = self.cost(grid, cell, n);
                if c < f32::MAX / 2.0 {
                    best = best.min(c + self.g_val(n));
                }
            }
            self.rhs.insert(cell, best);
        }

        let g = self.g_val(cell);
        let rhs = self.rhs_val(cell);

        if g != rhs {
            let key = self.calc_key(cell, g, rhs);
            self.u.push((key, cell));
        }
    }

    // ─── compute_shortest_path ─────────────────

    fn compute_shortest_path(&mut self, grid: &OccupancyGrid) {
        let mut iter = 0u32;
        loop {
            // P3：迭代上限——极端地图降级，避免拖垮 50ms tick（get_path 在 main_loop 同步执行）
            if iter >= MAX_COMPUTE_ITERS {
                warn!("[D*] compute_shortest_path 超过迭代上限 {MAX_COMPUTE_ITERS}，降级（路径可能不完整）");
                break;
            }
            let (key, cell) = match self.pop_valid() {
                Some(v) => v,
                None => {
                    info!("[D*] compute: heap empty after {} iters", iter);
                    break;
                }
            };
            iter += 1;

            let g = self.g_val(cell);
            let rhs = self.rhs_val(cell);

            if g == rhs {
                continue;
            }

            let start_key = self.calc_key(self.start, self.g_val(self.start), self.rhs_val(self.start));

            if (key.k1 > start_key.k1 || (key.k1 == start_key.k1 && key.k2 >= start_key.k2))
                && self.g_val(self.start) == self.rhs_val(self.start) {
                self.u.push((key, cell));
                info!("[D*] compute: converged after {} iters", iter);
                break;
            }

            if g > rhs {
                self.g.insert(cell, rhs);
                for n in Self::neighbors(cell) {
                    if n != self.goal {
                        self.update_vertex(n, grid);
                    }
                }
            } else {
                self.g.insert(cell, f32::MAX);
                self.update_vertex(cell, grid);
                for n in Self::neighbors(cell) {
                    if n != self.goal {
                        self.update_vertex(n, grid);
                    }
                }
            }
        }
    }

    /// 从队列弹出有效条目
    ///
    /// 存储 key 与当前重算 key 失配时按当前 key **重插**而非丢弃（标准 D* Lite 的 re-key）：
    /// `move_to` 会累积 `km`，使堆内旧条目 key 失配；若直接丢弃，开阔地移动（障碍不变、
    /// `set_dynamic_obstacles` 早退不补种）时堆被清空、`g/rhs` 冻结、寻路退化。
    fn pop_valid(&mut self) -> Option<(Key, (i32, i32))> {
        loop {
            let (key, cell) = self.u.pop()?;
            let g = self.g_val(cell);
            let r = self.rhs_val(cell);
            let expected = self.calc_key(cell, g, r);
            if key.k1 == expected.k1 && key.k2 == expected.k2 {
                return Some((key, cell));
            }
            self.u.push((expected, cell));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_hits_dynamic_obstacle() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (5, 5));
        // 初始无动态障碍，Free/Unknown 格代价 1.0
        assert_eq!(pf.cost(&grid, (0, 0), (2, 2)), 1.0);
        // 注入障碍后，该格代价为 ∞
        pf.set_dynamic_obstacles(&[(2, 2)], &grid);
        assert_eq!(pf.cost(&grid, (0, 0), (2, 2)), f32::MAX);
        // 非障碍格不受影响
        assert_eq!(pf.cost(&grid, (0, 0), (3, 3)), 1.0);
    }

    #[test]
    fn test_set_dynamic_obstacles_remove_restores() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (5, 5));
        pf.set_dynamic_obstacles(&[(2, 2)], &grid);
        assert_eq!(pf.cost(&grid, (0, 0), (2, 2)), f32::MAX);
        // 移除后恢复为普通格
        pf.set_dynamic_obstacles(&[], &grid);
        assert_eq!(pf.cost(&grid, (0, 0), (2, 2)), 1.0);
    }

    #[test]
    fn test_set_dynamic_obstacles_idempotent() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (5, 5));
        pf.set_dynamic_obstacles(&[(2, 2)], &grid);
        // 相同集合重复调用不 panic，结果不变
        pf.set_dynamic_obstacles(&[(2, 2)], &grid);
        assert_eq!(pf.cost(&grid, (0, 0), (2, 2)), f32::MAX);
    }

    #[test]
    fn test_next_step_avoids_injected_obstacle() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (0, 2));
        // 无障碍时必经格 (0,1)
        assert_eq!(pf.next_step(&grid), Some((0, 1)));
        // 注入 (0,1) 障碍后，不应再返回 (0,1)
        pf.set_dynamic_obstacles(&[(0, 1)], &grid);
        let step = pf.next_step(&grid);
        assert_ne!(step, Some((0, 1)), "注入障碍后不应返回障碍格");
        assert!(step.is_some(), "绕行路径应存在");
    }

    #[test]
    fn test_next_step_avoids_obstacle_after_move_to() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (0, 3));
        // 走一步到 (0,1)（模拟车移动，更新 km/start）
        pf.move_to((0, 1));
        // 注入下一必经格 (0,2) 障碍
        pf.set_dynamic_obstacles(&[(0, 2)], &grid);
        let step = pf.next_step(&grid);
        assert_ne!(step, Some((0, 2)), "move_to 后注入障碍应被绕开");
        assert!(step.is_some(), "绕行路径应存在");
    }

    #[test]
    fn test_next_step_after_move_to_without_obstacle_change() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (0, 3));
        // 走一步到 (0,1)，模拟车跨格移动（km 变化）；不注入障碍（回归：pop_valid 不应清空堆）
        pf.move_to((0, 1));
        let step = pf.next_step(&grid);
        assert_eq!(step, Some((0, 2)), "move_to 后（无障碍变化）寻路不应退化");
    }
}
