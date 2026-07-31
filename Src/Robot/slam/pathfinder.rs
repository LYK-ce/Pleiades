//Presented by KeJi
//Created Date ： 2026-07-31
//Modified Date ： 2026-07-31

//! D* Lite 增量路径规划器
//!
//! 基于 Sven Koenig & Maxim Likhachev (2002) 算法。
//! 4 连通网格，Manhattan 距离启发式。
//! 障碍通过 mark_obstacle() 增量修补，无需每次从头搜索。

use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;

use crate::robot::slam::{OccupancyGrid, CHUNK_SIZE};

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
        // BinaryHeap 是最大堆，反转比较实现最小堆
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
        };
        slf.initialize();
        slf
    }

    // ─── 公有接口 ─────────────────────────────────

    /// 查询下一格方向
    ///
    /// 返回 4 连通邻居中 `cost + g` 最小的格子，或 None（不可达/已到达）。
    pub fn next_step(&mut self, grid: &OccupancyGrid) -> Option<(i32, i32)> {
        if self.start == self.goal {
            return None;
        }
        self.compute_shortest_path(grid);
        if self.rhs_val(self.start) >= f32::MAX / 2.0 {
            return None; // 不可达
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
        self.km += Self::heuristic(self.start, new_start);
        self.start = new_start;
    }

    /// 标记障碍格（会触发局部修补）
    pub fn mark_obstacle(&mut self, cell: (i32, i32), grid: &OccupancyGrid) {
        for n in Self::neighbors(cell) {
            if self.has_rhs(n) {
                self.update_vertex(n, grid);
            }
        }
        self.update_vertex(cell, grid);
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
        if to.0 < 0 || to.0 >= CHUNK_SIZE as i32
            || to.1 < 0 || to.1 >= CHUNK_SIZE as i32
        {
            return f32::MAX;
        }
        match grid.state(to.0, to.1) {
            Some(1) => f32::MAX,   // Occupied
            _ => 1.0,              // Free(0) / Unknown(2) → 可通过
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

    /// 更新节点的 rhs 和队列状态
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

    /// D* Lite 主循环：扩展节点直到达到一致状态
    fn compute_shortest_path(&mut self, grid: &OccupancyGrid) {
        loop {
            // 弹出有效条目（跳过过期）
            let (key, cell) = match self.pop_valid() {
                Some(v) => v,
                None => break,
            };

            let g = self.g_val(cell);
            let rhs = self.rhs_val(cell);

            // 一致节点，跳过
            if g == rhs {
                continue;
            }

            let start_key = self.calc_key(self.start, self.g_val(self.start), self.rhs_val(self.start));

            // 终止条件：队首 key ≥ start_key 且 start 一致
            if key >= start_key && self.g_val(self.start) == self.rhs_val(self.start) {
                self.u.push((key, cell));
                break;
            }

            if g > rhs {
                // 欠一致：降低 g
                self.g.insert(cell, rhs);
                for n in Self::neighbors(cell) {
                    if n != self.goal {
                        self.update_vertex(n, grid);
                    }
                }
            } else {
                // 过一致：提升 g (g < rhs)
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

    /// 从队列弹出有效条目，跳过 g/rhs 已变化的过期条目
    /// 仅比较 k2（min(g,rhs)），k1 因 km 技巧变化属正常，不比对
    fn pop_valid(&mut self) -> Option<(Key, (i32, i32))> {
        loop {
            let (key, cell) = self.u.pop()?;
            let g = self.g_val(cell);
            let r = self.rhs_val(cell);
            if key.k2 == g.min(r) {
                return Some((key, cell));
            }
        }
    }
}
