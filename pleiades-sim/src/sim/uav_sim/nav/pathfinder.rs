//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

#![allow(non_snake_case)]

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
///
/// task_31 Step 7 第 2 轮（prev_issues#7）：原值 `10_000` 在**空地图 + 远目标**下不够用。
/// 实测 `goto(36,110)`（起点格 (128,128) → 目标格 (72,220)，曼哈顿 148 格）首次规划时，
/// 4 连通 D* 需要扩展 ~2 万节点；10k 上限让 `compute_shortest_path` 提前 break，
/// `g/rhs` 只收敛了一半 → `rhs[start] = INF` → `[Goal] D* 不可达，跳过此任务`，
/// **首条远目标指令被静默丢弃**（S3 只因排在 S2 之后、地图已有路径才躲过）。
///
/// 取 60_000（评审建议 3~6 万的上界）：覆盖 256×256 全图的量级（65 536 格），实测首帧
/// 规划耗时仍远低于 50 ms tick 预算（见本文件单测 `test_far_goal_reachable_from_empty_grid`
/// 与 Workbook「Step 7 第 2 轮」记录的实测耗时）。超过上限仍会 break（有界终止，不挂死）。
const MAX_COMPUTE_ITERS: u32 = 60_000;

/// 等代价容差：两邻居的 `c(s,s')+g(s')` 相差 ≤ 它即视为「打平」，沿用上一步（防 4 连通并列解翻面）
const TIE_EPS: f32 = 1e-3;

/// 从候选里选下一步（task_31 Step 7 / D1：等代价迟滞）
///
/// `candidates` 必须已按 4 连通**固定顺序**给出且代价均有限（无穷代价由调用方滤掉）。
///
/// 1) 若 `last` 仍在候选里、且它的值与最优值相差 ≤ `eps` → 选 `last`（迟滞，避免 tie 每 tick 翻面）
/// 2) 否则选**第一个**最小值（与引入迟滞前的行为逐字一致）
///
/// 迟滞只在「完全打平」时生效：真有更优邻居（差 > eps）时照常切走，不会坚持走次优路径。
/// 调用方保证 `last` 是 `start` 的邻居；车走进新格后 `last == start`，不在候选里 → 自动失效、重新规划。
fn Choose_Step(candidates: &[((i32, i32), f32)], last: Option<(i32, i32)>, eps: f32) -> Option<(i32, i32)> {
    let best = candidates.iter().map(|(_, v)| *v).fold(f32::INFINITY, f32::min);
    if !best.is_finite() {
        return None;
    }
    if let Some(last) = last {
        if candidates.iter().any(|&(cell, v)| cell == last && v <= best + eps) {
            return Some(last);
        }
    }
    candidates.iter().find(|(_, v)| *v == best).map(|(cell, _)| *cell)
}

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
    /// 上次返回的下一步格（sub_target 变化时打日志，避免每 50ms 刷屏）
    last_step: Option<(i32, i32)>,
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
            last_step: None,
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
        // 找邻居中 c(s,s') + v(s') 最小的（先收集候选，再交给 Choose_Step 做等代价迟滞）
        //
        // 为什么需要迟滞（task_31 Step 7 实测 D1）：4 连通等距时最优邻居不唯一，
        // 每 tick 重新 compute_shortest_path 后 `HashMap` 遍历/浮点累加的末位差异会让 tie
        // 在两格之间翻面，`sub_target` 一变，executor 就 `Stop + Idle` 重新转向，车在墙前永久冻结。
        //
        // 为什么用 v = min(g, rhs) 而不是裸 g（Step 7 实测 D1-b）：D* Lite 的 `g` 只在节点被
        // **弹出并收敛**后才有值，没扩展过的邻居 `g = ∞`。拿它当"到目标的距离"会把
        // 「只是还没扩展、其实可通行」的邻居算成不可达（实测：西侧绕行格全 ∞，车只能在南北
        // 两格间来回摆动）。`rhs` 是**一步前瞻**（由邻居的 g 直接算出），对未收敛节点仍是可用
        // 估计；取 min 后已收敛的照旧用 g，未收敛的退化为一步前瞻，不会再出现"整片方向 ∞"。
        let mut cands: Vec<((i32, i32), f32)> = Vec::with_capacity(4);
        for n in Self::neighbors(self.start) {
            let cost = self.cost(grid, self.start, n);
            if cost >= f32::MAX / 2.0 { continue; }
            cands.push((n, cost + self.Value(n)));
        }
        let step = Choose_Step(&cands, self.last_step, TIE_EPS);
        // 仅在 sub_target 变化时打一条（车逐格移动，格子变了才打）
        if step != self.last_step {
            if let Some((gx, gy)) = step {
                // 候选值一起打：等代价迟滞失效时（D1 的第二形态）能一眼看出是哪个邻居在翻面
                let detail: Vec<String> =
                    cands.iter().map(|(c, v)| format!("({},{})={:.2}", c.0, c.1, v)).collect();
                info!("[D*] sub_target = ({gx}, {gy}) 候选: {}", detail.join(" "));
            }
            self.last_step = step;
        }
        step
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
        self.Notify_Changed(&[cell], grid);
    }

    /// 地图变化格 → 局部修补（**建图增量进 D\* 的唯一入口**）
    ///
    /// D* Lite 只在节点被弹出/被通知时重算 `rhs`；`cost()` 虽然现读 `grid`，但**没有任何事件**
    /// 告诉它"这些格的地图变了"。于是画好的地图（墙）对 `g/rhs` 不可见，规划沿旧值继续走，
    /// 表现为「目标在两格之间翻面、车原地打转」（task_31 Step 7 实测：S3 在 (36.28, 91.23)
    /// 与 (36.28, 90.74) 之间来回 150 s）。本函数由 `GoalService::get_path` 每 tick 用
    /// 建图增量喂进来，等价于标准 D* Lite 的"边代价变化"处理。
    ///
    /// 修补范围取**2 环**（自身 + 1 环 + 2 环）：`cost()` 的判据是"本格或贴邻格是 Occupied"
    /// （1 格膨胀），所以某格变为 Occupied 会影响到**距离 2** 的格子的可通行性。
    pub fn Notify_Changed(&mut self, cells: &[(i32, i32)], grid: &OccupancyGrid) {
        for &cell in cells {
            for n in Self::neighbors(cell) {
                for nn in Self::neighbors(n) {
                    if nn != cell && self.has_rhs(nn) {
                        self.update_vertex(nn, grid);
                    }
                }
                if self.has_rhs(n) {
                    self.update_vertex(n, grid);
                }
            }
            self.update_vertex(cell, grid);
        }
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

    /// 移动代价：Free/Unknown=1，Occupied（含**贴邻格**）=∞，出界=∞
    ///
    /// 贴邻格不可通行（task_31 Step 7 实测修复）：急停阈值 `OBSTACLE_THRESHOLD_M = 0.3 m`
    /// 比格心到墙面的距离（`CELL_RESOLUTION/2 = 0.25 m`）大 —— 只要规划把车放进**与
    /// Occupied 格 4 连通的格**，车停下时离墙必然 < 0.3 m → 急停每 tick 触发 → 车既不能
    /// 前进也不能转向（急停在决策之前 `Stop + return`）→ **永久冻结**。
    /// 实测：S3 停在 (40.23, 91.74) 120 s 不动、yaw 恒定，日志 94 条急停。
    /// 因此把「贴邻 Occupied」视为不可通行（等价于对地图做 1 格膨胀），让规划与急停自洽。
    fn cost(&self, grid: &OccupancyGrid, _from: (i32, i32), to: (i32, i32)) -> f32 {
        if self.dynamic_obstacles.contains(&to) {
            return f32::MAX;
        }
        if Self::Blocked(grid, to) {
            return f32::MAX;
        }
        1.0
    }

    /// 在界内且本格非 Occupied
    fn Passable(grid: &OccupancyGrid, cell: (i32, i32)) -> bool {
        Self::In_Bounds(cell)
            && !matches!(grid.state(cell.0, cell.1), Some(val) if val == CellState::Occupied as u8)
    }

    /// 本格 Occupied/出界，或**任一 4 连通邻居是 Occupied** → 不可通行（1 格膨胀，见 [`Self::cost`]）
    ///
    /// 膨胀半径取 1 格的理由：4 连通的规划格心到贴邻的 Occupied 格面只有 `0.25 m`，
    /// 而急停阈值是 `0.3 m`；再远一格（2 格）不需要，且会把 `wall_c` 与 `wall_d` 之间
    /// 仅 1 m 的通道彻底堵死（S3 的目标就在那条通道后面）。
    /// 注意：**出界只判自身**，不参与「邻居」膨胀（地图外沿不该再外扩一圈，
    /// 否则贴边的格全部不可通行，单测里以 (0,0) 为起点的用例会失去全部出口）。
    ///
    /// ## 偏差 D1-c（**未获人类批准的设计变更**，见 `Workbook/wb_31_sim_world_link.md` 偏差登记表）
    ///
    /// 与 `pleiades-ugv` 的 D\* `cost()`（只判本格）**不一致**：本函数额外把贴邻格判为不可通行。
    /// - 几何关系：急停阈值 [`crate::sim::emergency_stop::OBSTACLE_THRESHOLD_M`] = `0.3 m`
    ///   **大于** 格心到贴邻格面的 `CELL_RESOLUTION/2 = 0.25 m`。规划只要允许「与 Occupied 4 连通的格」，
    ///   车停在那里就必然 < 0.3 m → 急停每 tick 触发（`on_tick` 里急停在决策**之前** `Stop + return`）
    ///   → 车既不能前进也不能转向 → 永久冻结。第 1 轮实测：S3 卡 (40.23, 91.74) 120 s、94 条急停。
    /// - 去留：由 `RobotSim_TestEnv/tools/probe_noinflate.py` 的 A/B 决策实验按**事前定死**的规则裁决
    ///   （能删就删 → 恢复与 ugv 一致；必须留则登记为本偏差）。结论见 `results/conclusion.md`。
    /// - 影响的两条判据：Task §八 判据 8、§九.3 S3 的「急停出现过」——规划停在离墙 ≥ 0.5 m 处，
    ///   静态世界里走规划绕行的车永不进入 0.3 m 扇区。**需人类裁决**（结论「需人类决策」）。
    fn Blocked(grid: &OccupancyGrid, cell: (i32, i32)) -> bool {
        if !Self::Passable(grid, cell) {
            return true;
        }
        Self::neighbors(cell)
            .iter()
            .any(|n| Self::In_Bounds(*n) && !Self::Passable(grid, *n))
    }

    /// 地图边界内
    fn In_Bounds(cell: (i32, i32)) -> bool {
        cell.0 >= 0 && cell.0 < CHUNK_SIZE as i32 && cell.1 >= 0 && cell.1 < CHUNK_SIZE as i32
    }

    // ─── 值访问（不存在 = ∞）─────────────

    fn g_val(&self, cell: (i32, i32)) -> f32 {
        *self.g.get(&cell).unwrap_or(&f32::MAX)
    }

    fn rhs_val(&self, cell: (i32, i32)) -> f32 {
        *self.rhs.get(&cell).unwrap_or(&f32::MAX)
    }

    /// 估计值 `v = min(g, rhs)`（见 `next_step` 的注释：未收敛节点只有 `rhs` 可用）
    fn Value(&self, cell: (i32, i32)) -> f32 {
        self.g_val(cell).min(self.rhs_val(cell))
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
    fn test_cost_inflates_cells_adjacent_to_occupied() {
        let mut grid = OccupancyGrid::new();
        // (10,10) 连续命中 3 帧 → Occupied
        for _ in 0..3 {
            grid.update(10, 10, true);
        }
        assert_eq!(grid.state(10, 10), Some(CellState::Occupied as u8));
        let pf = DStarLite::new((0, 0), (20, 20));
        // 墙体本身不可通行
        assert_eq!(pf.cost(&grid, (9, 10), (10, 10)), f32::MAX);
        // 贴邻格（上下左右）也不可通行：否则车停下时离墙 < 急停阈值 0.3 m
        for n in [(9, 10), (11, 10), (10, 9), (10, 11)] {
            assert_eq!(pf.cost(&grid, (0, 0), n), f32::MAX, "贴邻格 {n:?} 应不可通行");
        }
        // 斜角格隔着一个空行/列（不贴邻）→ 可通行
        assert_eq!(pf.cost(&grid, (0, 0), (9, 9)), 1.0);
        assert_eq!(pf.cost(&grid, (0, 0), (8, 10)), 1.0);
    }

    #[test]
    fn test_next_step_steps_around_inflated_wall() {
        // 竖直墙 x=5（y=0..9），膨胀后 x=4/5/6 都不可通行；起点 (2,5)、目标 (8,5)
        let mut grid = OccupancyGrid::new();
        for y in 0..10 {
            for _ in 0..3 {
                grid.update(5, y, true);
            }
        }
        let mut pf = DStarLite::new((2, 5), (8, 5));
        let mut cur = (2, 5);
        let mut reached = false;
        for _ in 0..80 {
            let step = pf.next_step(&grid).expect("绕行路径应存在");
            assert_eq!(
                pf.cost(&grid, cur, step),
                1.0,
                "不得走进被膨胀的格：{cur:?} -> {step:?}"
            );
            pf.move_to(step);
            cur = step;
            if cur == (8, 5) {
                reached = true;
                break;
            }
        }
        assert!(reached, "沿膨胀后的可通行格应能到达 (8,5)，实际停在 {cur:?}");
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

    // ─── Choose_Step（task_31 Step 7 / D1：等代价迟滞）────────────────

    const A: (i32, i32) = (0, 1);
    const B: (i32, i32) = (1, 0);

    #[test]
    fn test_Choose_Step_hysteresis_prefers_last() {
        // 完全打平：上一步是 B → 继续 B（不翻面）
        let c = [(A, 10.0), (B, 10.0)];
        assert_eq!(Choose_Step(&c, Some(B), TIE_EPS), Some(B));
        // 上一步是 A → 继续 A
        assert_eq!(Choose_Step(&c, Some(A), TIE_EPS), Some(A));
    }

    #[test]
    fn test_Choose_Step_no_last_takes_first_min() {
        // 无上一步：固定顺序里第一个最小值（与引入迟滞前行为一致）
        let c = [(A, 10.0), (B, 10.0)];
        assert_eq!(Choose_Step(&c, None, TIE_EPS), Some(A));
    }

    #[test]
    fn test_Choose_Step_better_neighbor_wins_over_hysteresis() {
        // 有更优解（差 1.0 > eps）：不迟滞，切到 A
        let c = [(A, 10.0), (B, 11.0)];
        assert_eq!(Choose_Step(&c, Some(B), TIE_EPS), Some(A));
    }

    #[test]
    fn test_Choose_Step_within_tolerance_keeps_last() {
        // 上一步略优（差 5e-4 ≤ eps）：仍在容差内 → 保持 B
        let c = [(B, 10.0 - 5e-4), (A, 10.0)];
        assert_eq!(Choose_Step(&c, Some(B), TIE_EPS), Some(B));
    }

    #[test]
    fn test_Choose_Step_out_of_tolerance_switches() {
        // 上一步略差（差 2e-3 > eps）：容差外 → 切到 A
        let c = [(A, 10.0), (B, 10.0 + 2e-3)];
        assert_eq!(Choose_Step(&c, Some(B), TIE_EPS), Some(A));
    }

    #[test]
    fn test_Choose_Step_empty_candidates() {
        assert_eq!(Choose_Step(&[], Some((0, 0)), TIE_EPS), None);
    }

    #[test]
    fn test_Value_falls_back_to_rhs_for_unexpanded_cell() {
        // D1-b 回归：只有 rhs、没有 g 的节点（未扩展）不能被当成不可达
        let grid = OccupancyGrid::new();
        let pf = DStarLite::new((0, 0), (0, 1));
        // 新规划器：goal 的 rhs=0、g 还没被扩展（∞）
        assert_eq!(pf.g_val((0, 1)), f32::MAX);
        assert_eq!(pf.rhs_val((0, 1)), 0.0, "goal 的 rhs 应为 0");
        assert_eq!(pf.Value((0, 1)), 0.0, "未收敛节点应退化到 rhs，而不是 ∞");
        // 既没 g 也没 rhs 的格子仍是 ∞（不会凭空出现"可达"）
        assert_eq!(pf.Value((0, 2)), f32::MAX);
        // g 收敛后 Value 取 min(g, rhs) = g
        let mut pf = DStarLite::new((0, 0), (0, 2));
        assert_eq!(pf.next_step(&grid), Some((0, 1)));
        assert!(pf.g_val((0, 1)).is_finite(), "邻居应收敛出有限 g");
        assert_eq!(pf.Value((0, 1)), pf.g_val((0, 1)));
    }

    #[test]
    fn test_Choose_Step_last_not_in_candidates() {
        // 上一步已不是 start 的邻居（车走进了那一格）→ 迟滞失效，回到第一个最小值
        let c = [(A, 10.0), (B, 10.0)];
        assert_eq!(Choose_Step(&c, Some((9, 9)), TIE_EPS), Some(A));
    }

    #[test]
    fn test_next_step_is_stable_across_ticks() {
        // 端到端复现冻结场景：空地曼哈顿路径上等距邻居不唯一时，
        // 连续两次 next_step（同一 start、未 move_to）必须返回**同一格**，
        // 否则 executor 会因 sub_target 变化反复 Stop + Idle（D1）。
        let grid = OccupancyGrid::new();
        // start 与 goal 同列同行的对角线关系：两个邻居代价相等，tie 明显
        let mut pf = DStarLite::new((2, 2), (0, 0));
        let first = pf.next_step(&grid);
        assert!(first.is_some(), "空 grid 上应能找到下一步");
        for _ in 0..8 {
            assert_eq!(pf.next_step(&grid), first, "tie 不得每 tick 翻面（D1 冻结回归）");
        }
    }

    // ─── MAX_COMPUTE_ITERS（task_31 Step 7 第 2 轮 / prev_issues#7）────────────

    #[test]
    fn test_far_goal_reachable_from_empty_grid() {
        // 回归：空地图 + 远目标（= 第 1 轮 `goto(36,110)` 的首帧：起点格 (128,128) →
        // 目标格 (72,220)，曼哈顿 148 格）。10_000 上限时这里会「超迭代上限 → rhs[start]=INF」
        // → `[Goal] D* 不可达，跳过此任务` → 首条远目标指令被静默丢弃。
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((128, 128), (72, 220));
        let t0 = std::time::Instant::now();
        let step = pf.next_step(&grid);
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert!(step.is_some(), "空地图远目标必须有下一步（不得降级/丢任务）");
        // 注意：本实现用 `f32::MAX` 当"无穷"哨兵（不是 f32::INFINITY），所以判据是
        // `< f32::MAX / 2.0`（与 `next_step` 内部判不可达的阈值一致），不能用 is_finite()。
        assert!(
            pf.rhs_val((128, 128)) < f32::MAX / 2.0,
            "起点 rhs 必须收敛为有限代价（否则就是判定不可达：{}）",
            pf.rhs_val((128, 128))
        );
        assert!(
            pf.Value((128, 128)) < f32::MAX / 2.0,
            "起点估计值必须有限（rhs[start]=INF 就是不可达）"
        );
        // 打印实测耗时（`cargo test -- --nocapture` 可见）：60_000 上限是否撞 50 ms tick
        println!("[D*] 空地图远目标首帧规划耗时 {elapsed_ms:.2} ms");
    }

    #[test]
    fn test_enclosed_goal_terminates_with_none() {
        // 被围死的目标：4 邻居全 Occupied → 起点不可达。必须**有界终止**并返回 None
        // （不得因迭代上限之外的死循环挂死；也不得返回一个假的"下一步"）。
        let mut grid = OccupancyGrid::new();
        for cell in [(5, 4), (6, 5), (5, 6), (4, 5)] {
            for _ in 0..3 {
                grid.update(cell.0, cell.1, true);
            }
            assert_eq!(grid.state(cell.0, cell.1), Some(CellState::Occupied as u8));
        }
        let mut pf = DStarLite::new((0, 0), (5, 5));
        let t0 = std::time::Instant::now();
        let step = pf.next_step(&grid);
        assert_eq!(step, None, "被围死的目标必须返回 None");
        assert!(
            pf.rhs_val((0, 0)) >= f32::MAX / 2.0,
            "起点 rhs 应为 INF 哨兵（真不可达，而非降级）"
        );
        // 有界终止：远小于 tick 预算（若这里变慢说明有界性被破坏）
        assert!(
            t0.elapsed().as_secs_f64() < 1.0,
            "围死目标的搜索必须在 1 s 内终止，实际 {:?}",
            t0.elapsed()
        );
    }
}
