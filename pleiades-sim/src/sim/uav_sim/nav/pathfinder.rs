//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-29

#![allow(non_snake_case)]

//! D* Lite 增量路径规划器
//!
//! 基于 Sven Koenig & Maxim Likhachev (2002) 算法。
//! **8 连通（禁切角）**，octile 距离启发式。
//! 障碍通过 mark_obstacle() 增量修补，无需每次从头搜索。
//!
//! ## 8 连通契约（Task 39 §三）
//!
//! 1. **邻居顺序（固定，顺时针，从正北起）**：`N → NE → E → SE → S → SW → W → NW`
//!    —— `neighbors()` 的数组顺序决定 tie-break（`next_step` 取**第一个严格更小**的候选），**不许重排**。
//! 2. **边代价**：cardinal `1.0`，diagonal `SQRT_2`。
//! 3. **禁切角**：`s → t` 对角边存在 ⟺ `passable(s.x+dx, s.y)` 且 `passable(s.x, s.y+dy)` 且 `passable(t)`。
//! 4. **`passable(c)`**（两变体同口径）：在界内 && 本格 != `Occupied` && 不在动态障碍集合。
//! 5. **修补范围 1 环**：`rhs(s)` 只依赖 `s` 的 8 条边，而边只涉及 `t` 与两个 cardinal 中间格，
//!    全在 `s` 的 8 邻域内 —— 所以 `mark_obstacle` / `set_dynamic_obstacles` / `Notify_Changed`
//!    按 **8 邻域**修补即可（sim 的 2 环随 1 格膨胀一起回退）。

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::cmp::Ordering;
use std::time::{Duration, Instant};
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
///
/// Task 39 起它**只是兜底**：命中时返回 `ComputeOutcome::BudgetExhausted` → `StepResult::NotReady`
/// （=「没算完」），**不再当作不可达**。正常出口是时间预算（config `plan_budget_ms`，缺省 20 ms）。
const MAX_COMPUTE_ITERS: u32 = 60_000;

/// 规划时间预算的检查粒度：每 N 次迭代查一次表（Task 39 §二）
///
/// 一次迭代是一个 `pop_valid` + `update_vertex`（微秒级），64 次一查既不会让
/// 单拍超支太多，也不会被 `Instant::now()` 自身的开销拖慢。
const BUDGET_CHECK_EVERY: u32 = 64;

/// 对角步长（= √2 格 = √2 × 0.5 m ≈ 0.707 m），Task 39 §三
const SQRT_2: f32 = std::f32::consts::SQRT_2;

// ============================================================
// 返回类型（Task 39 §二：先补"说法"，再补"行为"）
// ============================================================

/// `next_step` 的三种结果：**"没算完" 不等于 "没有路"**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    /// 有一格（8 邻居里 `cost + Value` 最小的）
    Step((i32, i32)),
    /// 已收敛，确实没有路（堆空 且 `rhs[start] = ∞`）
    NoPath,
    /// 时间预算（或旧迭代上限）用完，`g/rhs` 是半成品 —— **不许读、不许当不可达**
    NotReady,
}

/// `compute_shortest_path` 的两种结果（Task 39 §二：预算按**时间**计）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeOutcome {
    /// 堆空或已收敛（起点值已定、堆顶 key 已过起点）
    Converged,
    /// 预算（或旧迭代上限）用完，搜索没做完
    BudgetExhausted,
}

/// 等代价容差：两邻居的 `c(s,s')+g(s')` 相差 ≤ 它即视为「打平」，沿用上一步（防 8 连通并列解翻面）
const TIE_EPS: f32 = 1e-3;

/// 从候选里选下一步（task_31 Step 7 / D1：等代价迟滞）
///
/// `candidates` 必须已按 8 连通**固定顺序**给出且代价均有限（无穷代价由调用方滤掉）。
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

    /// 查询下一格方向（Task 39 §二：三态返回）
    ///
    /// 返回 8 邻居中 `cost + Value` 最小的格子；这一拍没算完 → [`StepResult::NotReady`]，
    /// **绝不**降级成 `NoPath`（算不完 ≠ 没有路）。
    ///
    /// `budget` 由调用方传入（值来自 config 顶层键 `plan_budget_ms`，缺省 20 ms），
    /// 本函数自身不写入任何缺省值。
    pub fn next_step(&mut self, grid: &OccupancyGrid, budget: Duration) -> StepResult {
        info!("[D*] next_step: start=({},{}) goal=({},{}) km={:.1} budget={:?}",
            self.start.0, self.start.1, self.goal.0, self.goal.1, self.km, budget);
        if self.start == self.goal {
            warn!("[D*] start==goal，返回 NoPath");
            return StepResult::NoPath;
        }
        // 预算用完 → 立即 NotReady：从这里往下**一行都不许读 g/rhs**（半成品）
        if self.compute_shortest_path(grid, budget) == ComputeOutcome::BudgetExhausted {
            return StepResult::NotReady;
        }
        if self.rhs_val(self.start) >= f32::MAX / 2.0 {
            warn!("[D*] rhs[start]=INF，不可达");
            return StepResult::NoPath;
        }
        // 找邻居中 c(s,s') + v(s') 最小的（先收集候选，再交给 Choose_Step 做等代价迟滞）
        //
        // 为什么需要迟滞（task_31 Step 7 实测 D1）：8 连通等距时最优邻居不唯一，
        // 每 tick 重新 compute_shortest_path 后 `HashMap` 遍历/浮点累加的末位差异会让 tie
        // 在两格之间翻面，`sub_target` 一变，executor 就 `Stop + Idle` 重新转向，车在墙前永久冻结。
        //
        // 为什么用 v = min(g, rhs) 而不是裸 g（Step 7 实测 D1-b）：D* Lite 的 `g` 只在节点被
        // **弹出并收敛**后才有值，没扩展过的邻居 `g = ∞`。拿它当"到目标的距离"会把
        // 「只是还没扩展、其实可通行」的邻居算成不可达（实测：西侧绕行格全 ∞，车只能在南北
        // 两格间来回摆动）。`rhs` 是**一步前瞻**（由邻居的 g 直接算出），对未收敛节点仍是可用
        // 估计；取 min 后已收敛的照旧用 g，未收敛的退化为一步前瞻，不会再出现"整片方向 ∞"。
        let mut cands: Vec<((i32, i32), f32)> = Vec::with_capacity(8);
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
        match step {
            Some(cell) => StepResult::Step(cell),
            // 收敛了却挑不出候选（数学上不会发生）：宁可说没有路，也**绝不给假格**
            None => StepResult::NoPath,
        }
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
    /// 修补范围取**1 环**（自身 + `neighbors()` 一圈，与真机 `mark_obstacle` 同口径）：
    /// `rhs(s)` 只依赖 `s` 的 8 条边，而边只涉及 `t` 与两个 cardinal 中间格，全在 `s` 的
    /// 8 邻域内 —— 所以按 **8 邻域**修补即可。sim 的 2 环修补已随可通行判据回退一起回退
    /// （Task 39 §三：与真机同口径）。
    pub fn Notify_Changed(&mut self, cells: &[(i32, i32)], grid: &OccupancyGrid) {
        for &cell in cells {
            for n in Self::neighbors(cell) {
                if self.has_rhs(n) {
                    self.update_vertex(n, grid);
                }
            }
            self.update_vertex(cell, grid);
        }
    }

    /// 注入最新他车动态障碍格（Task 15）
    ///
    /// 对 old/new 集合做 diff，仅对变更格触发局部修补（先 **8 邻域**、后自身，
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

    /// octile 距离（8 连通下的精确步数代价）：`max(dx,dy) + (√2−1)·min(dx,dy)`
    fn heuristic(a: (i32, i32), b: (i32, i32)) -> f32 {
        let dx = (a.0 - b.0).abs() as f32;
        let dy = (a.1 - b.1).abs() as f32;
        dx.max(dy) + (SQRT_2 - 1.0) * dx.min(dy)
    }

    /// 8 连通邻居，顺序固定（顺时针、从正北起）：`N → NE → E → SE → S → SW → W → NW`
    ///
    /// 顺序即 tie-break：`next_step` 取**第一个严格更小**的候选，**不许重排**（Task 39 §三）。
    fn neighbors(cell: (i32, i32)) -> [(i32, i32); 8] {
        let (x, y) = cell;
        [
            (x, y - 1),
            (x + 1, y - 1),
            (x + 1, y),
            (x + 1, y + 1),
            (x, y + 1),
            (x - 1, y + 1),
            (x - 1, y),
            (x - 1, y - 1),
        ]
    }

    /// 边代价：cardinal `1.0`、diagonal `SQRT_2`；不可通行 / 切角 → ∞
    ///
    /// **禁切角**：`s → t` 对角边存在 ⟺ 两个相邻 cardinal 中间格可通行，且 `t` 可通行。
    /// `from`/`to` 只有 8 邻居才有"边"语义；非邻居输入按 cardinal 处理（只有单测拿它查单格可通行性）。
    fn cost(&self, grid: &OccupancyGrid, from: (i32, i32), to: (i32, i32)) -> f32 {
        let dx = to.0 - from.0;
        let dy = to.1 - from.1;
        let diagonal = dx.abs() == 1 && dy.abs() == 1;
        if diagonal
            && (!self.Passable(grid, (from.0 + dx, from.1))
                || !self.Passable(grid, (from.0, from.1 + dy)))
        {
            return f32::MAX;
        }
        if !self.Passable(grid, to) {
            return f32::MAX;
        }
        if diagonal { SQRT_2 } else { 1.0 }
    }

    /// 可通行判据（只判**本格**，Task 39 §三：sim 的 1 格膨胀已回退，与真机同口径）：
    /// 在界内 && 本格 != Occupied && 不在动态障碍集合
    fn Passable(&self, grid: &OccupancyGrid, cell: (i32, i32)) -> bool {
        Self::In_Bounds(cell)
            && !matches!(grid.state(cell.0, cell.1), Some(val) if val == CellState::Occupied as u8)
            && !self.dynamic_obstacles.contains(&cell)
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

    fn compute_shortest_path(&mut self, grid: &OccupancyGrid, budget: Duration) -> ComputeOutcome {
        // 预算 0 → 本拍一次都不算（确定性：不依赖墙钟，单测可断言）
        if budget.is_zero() {
            info!("[D*] compute: 预算为 0，本拍不算（NotReady）");
            return ComputeOutcome::BudgetExhausted;
        }
        let t0 = Instant::now();
        let mut iter = 0u32;
        loop {
            // P3：迭代上限——极端地图降级，避免拖垮 50ms tick（Task 39 起命中 → NotReady）
            if iter >= MAX_COMPUTE_ITERS {
                warn!("[D*] compute_shortest_path 超过迭代上限 {MAX_COMPUTE_ITERS}，返回 NotReady（兜底，不算不可达）");
                return ComputeOutcome::BudgetExhausted;
            }
            // 时间预算：每 BUDGET_CHECK_EVERY 次迭代查一次表。
            // `iter > 0` 保证**每拍至少推进一批** —— 极小预算（如 1 ns）也能断点续算，
            // 不会因"每次都超时且在起点就超时"造成永久零进展的死循环。
            //
            // 这一条用 `info!`（不是 `warn!`）：预算用完是**正常工况**，`NotReady` 连拍时
            // 每拍都会命中，`warn!` 会刷屏。给人看的"连续 5 拍一条 warn"在 `Navigator` 里。
            if iter > 0 && iter % BUDGET_CHECK_EVERY == 0 && t0.elapsed() >= budget {
                info!("[D*] compute: 时间预算 {budget:?} 用完（{iter} iters），本拍返回 NotReady");
                return ComputeOutcome::BudgetExhausted;
            }
            let (key, cell) = match self.pop_valid() {
                Some(v) => v,
                None => {
                    info!("[D*] compute: heap empty after {} iters", iter);
                    return ComputeOutcome::Converged;
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
                return ComputeOutcome::Converged;
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
        assert_eq!(pf.next_step(&grid, Duration::from_secs(3)), StepResult::Step((0, 1)));
        // 注入 (0,1) 障碍后，不应再返回 (0,1)
        pf.set_dynamic_obstacles(&[(0, 1)], &grid);
        let step = pf.next_step(&grid, Duration::from_secs(3));
        assert_ne!(step, StepResult::Step((0, 1)), "注入障碍后不应返回障碍格");
        assert!(matches!(step, StepResult::Step(_)), "绕行路径应存在");
    }

    #[test]
    fn test_next_step_avoids_obstacle_after_move_to() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (0, 3));
        // 走一步到 (0,1)（模拟车移动，更新 km/start）
        pf.move_to((0, 1));
        // 注入下一必经格 (0,2) 障碍
        pf.set_dynamic_obstacles(&[(0, 2)], &grid);
        let step = pf.next_step(&grid, Duration::from_secs(3));
        assert_ne!(step, StepResult::Step((0, 2)), "move_to 后注入障碍应被绕开");
        assert!(matches!(step, StepResult::Step(_)), "绕行路径应存在");
    }

    #[test]
    fn test_next_step_after_move_to_without_obstacle_change() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (0, 3));
        // 走一步到 (0,1)，模拟车跨格移动（km 变化）；不注入障碍（回归：pop_valid 不应清空堆）
        pf.move_to((0, 1));
        let step = pf.next_step(&grid, Duration::from_secs(3));
        assert_eq!(step, StepResult::Step((0, 2)), "move_to 后（无障碍变化）寻路不应退化");
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
        assert_eq!(pf.next_step(&grid, Duration::from_secs(3)), StepResult::Step((0, 1)));
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
        // 端到端复现冻结场景：空地 8 连通路径上等距邻居不唯一时，
        // 连续两次 next_step（同一 start、未 move_to）必须返回**同一格**，
        // 否则 executor 会因 sub_target 变化反复 Stop + Idle（D1）。
        let grid = OccupancyGrid::new();
        // start 与 goal 同列同行的对角线关系：两个邻居代价相等，tie 明显
        let mut pf = DStarLite::new((2, 2), (0, 0));
        let first = pf.next_step(&grid, Duration::from_secs(3));
        assert!(matches!(first, StepResult::Step(_)), "空 grid 上应能找到下一步");
        for _ in 0..8 {
            assert_eq!(
                pf.next_step(&grid, Duration::from_secs(3)),
                first,
                "tie 不得每 tick 翻面（D1 冻结回归）"
            );
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
        let step = pf.next_step(&grid, Duration::from_secs(3));
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert!(
            matches!(step, StepResult::Step(_)),
            "空地图远目标必须有下一步（不得降级/丢任务），实际 {step:?}"
        );
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
    fn test_enclosed_goal_returns_no_path() {
        // 被围死的目标：4 邻居全 Occupied → 起点不可达。必须**有界终止**并返回 NoPath
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
        // 3 s 预算：足够走完「收敛」这条路，所以这一条必须给出 NoPath 而不是 NotReady
        let step = pf.next_step(&grid, Duration::from_secs(3));
        assert_eq!(step, StepResult::NoPath, "被围死的目标必须返回 NoPath（收敛后）");
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

    // ─── Task 39 §二：三态返回 + 时间预算 ─────────────

    /// 预算 0 → 本拍不算 → `NotReady`，**不是** `NoPath`（算不完 ≠ 没有路）
    #[test]
    fn test_next_step_zero_budget_is_not_ready() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (5, 5));
        assert_eq!(pf.next_step(&grid, Duration::ZERO), StepResult::NotReady);
    }

    /// 预算充足 → 空地图直达 `Step`（第一格必为沿目标方向的邻居 `(0,1)`）
    #[test]
    fn test_next_step_converged_with_path_returns_step() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (0, 2));
        assert_eq!(
            pf.next_step(&grid, Duration::from_secs(3)),
            StepResult::Step((0, 1))
        );
    }

    /// 断点续算：1 ns 预算每拍只推一批 64 次迭代 → 多次调用最终必须给出 `Step`
    ///
    /// 目标取 (100,100)：8 连通 + octile 在空地图上是**精确**启发，搜索只沿最优对角走廊
    /// 扩展（每次迭代一格），距离必须长过一批 64 次迭代，才可能先出 `NotReady`。
    #[test]
    fn test_next_step_resumes_across_small_budgets() {
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (100, 100));
        let mut not_ready = 0u32;
        let mut out = StepResult::NotReady;
        for _ in 0..2_000 {
            out = pf.next_step(&grid, Duration::from_nanos(1));
            if out == StepResult::NotReady {
                not_ready += 1;
                continue;
            }
            break;
        }
        assert!(not_ready > 0, "1 ns 预算下应先出现 NotReady（每拍只推一批）");
        assert!(
            matches!(out, StepResult::Step(_)),
            "多次续算后必须收敛出 Step，实际 {out:?}"
        );
    }

    // ─── Task 39 §三：回退 1 格膨胀（反向锁）─────────────

    /// 反向锁（Task 39 §三.4.1-6）：**贴邻 `Occupied` 的空格代价必须是 1.0** —— sim 不再有 1 格膨胀
    #[test]
    fn test_cost_does_not_inflate_cells_next_to_occupied() {
        let mut grid = OccupancyGrid::new();
        for _ in 0..3 {
            grid.update(10, 10, true);
        }
        assert_eq!(grid.state(10, 10), Some(CellState::Occupied as u8));
        let pf = DStarLite::new((0, 0), (20, 20));
        // 墙体本身不可通行（含对角进墙）
        assert_eq!(pf.cost(&grid, (9, 10), (10, 10)), f32::MAX);
        assert_eq!(pf.cost(&grid, (9, 9), (10, 10)), f32::MAX);
        // 贴邻 Occupied 的空格**可通行**：代价 1.0（与真机同口径，无膨胀）
        for n in [(9, 10), (11, 10), (10, 9), (10, 11)] {
            assert_eq!(pf.cost(&grid, (0, 0), n), 1.0, "贴邻格 {n:?} 应可通行（无 1 格膨胀）");
        }
        // 斜角隔一行的格子也不受影响
        assert_eq!(pf.cost(&grid, (0, 0), (9, 9)), 1.0);
    }

    // ─── Task 39 §三：8 连通（禁切角）+ octile ─────────────

    /// 邻居顺序固定 `N,NE,E,SE,S,SW,W,NW`（决定 tie-break）；空地图斜向目标首步必为对角格
    #[test]
    fn test_neighbor_order_and_first_step_is_diagonal() {
        assert_eq!(
            DStarLite::neighbors((5, 5)),
            [(5, 4), (6, 4), (6, 5), (6, 6), (5, 6), (4, 6), (4, 5), (4, 4)],
            "邻居顺序必须是 N,NE,E,SE,S,SW,W,NW"
        );
        let grid = OccupancyGrid::new();
        let mut pf = DStarLite::new((0, 0), (2, 2));
        assert_eq!(
            pf.next_step(&grid, Duration::from_secs(3)),
            StepResult::Step((1, 1)),
            "斜向目标首步必须是对角格（4 连通时会是 (1,0) 或 (0,1)）"
        );
    }

    /// 禁切角（左）：对角边 (0,0)→(1,1) 的**北侧**中间格 (0,1) 被占 → 该对角边不存在
    #[test]
    fn test_no_corner_cutting_left() {
        let mut grid = OccupancyGrid::new();
        for _ in 0..3 {
            grid.update(0, 1, true);
        }
        assert_eq!(grid.state(0, 1), Some(CellState::Occupied as u8));
        let mut pf = DStarLite::new((0, 0), (2, 2));
        assert_eq!(
            pf.cost(&grid, (0, 0), (1, 1)),
            f32::MAX,
            "北侧中间格被占 → 不得切角"
        );
        assert_eq!(
            pf.next_step(&grid, Duration::from_secs(3)),
            StepResult::Step((1, 0)),
            "切角被禁后只能先走 E 绕"
        );
    }

    /// 禁切角（右）：对角边 (0,0)→(1,1) 的**东侧**中间格 (1,0) 被占 → 该对角边不存在
    #[test]
    fn test_no_corner_cutting_right() {
        let mut grid = OccupancyGrid::new();
        for _ in 0..3 {
            grid.update(1, 0, true);
        }
        assert_eq!(grid.state(1, 0), Some(CellState::Occupied as u8));
        let mut pf = DStarLite::new((0, 0), (2, 2));
        assert_eq!(
            pf.cost(&grid, (0, 0), (1, 1)),
            f32::MAX,
            "东侧中间格被占 → 不得切角"
        );
        assert_eq!(
            pf.next_step(&grid, Duration::from_secs(3)),
            StepResult::Step((0, 1)),
            "切角被禁后只能先走 S 绕"
        );
    }

    /// 边代价：cardinal 1.0、diagonal √2
    #[test]
    fn test_edge_costs_cardinal_one_diagonal_sqrt2() {
        let grid = OccupancyGrid::new();
        let pf = DStarLite::new((0, 0), (3, 3));
        assert_eq!(pf.cost(&grid, (0, 0), (1, 0)), 1.0, "E cardinal = 1.0");
        assert_eq!(pf.cost(&grid, (0, 0), (0, 1)), 1.0, "S cardinal = 1.0");
        assert_eq!(pf.cost(&grid, (0, 0), (1, 1)), SQRT_2, "SE diagonal = √2");
        assert_eq!(pf.cost(&grid, (1, 1), (0, 0)), SQRT_2, "反向对角同价");
        assert!(
            (pf.cost(&grid, (0, 0), (1, 1)) - std::f32::consts::SQRT_2).abs() < 1e-6,
            "对角代价必须等于 √2"
        );
    }

    /// octile：(0,0)→(3,4) = 4 + (√2−1)×3 ≈ 5.2426（Manhattan 会是 7.0）
    #[test]
    fn test_heuristic_is_octile() {
        let h = DStarLite::heuristic((0, 0), (3, 4));
        assert!((h - 5.2426).abs() < 1e-4, "octile(3,4) 应 ≈5.2426，实际 {h}");
        assert_eq!(h, DStarLite::heuristic((3, 4), (0, 0)), "启发必须对称");
        assert_eq!(DStarLite::heuristic((7, 7), (7, 7)), 0.0, "同格距离为 0");
        assert!(h < 7.0, "octile 必须小于 Manhattan(7.0)");
    }

    /// L 形墙仍能绕行：首步非 `NoPath`，且逐格移动能走到目标
    #[test]
    fn test_l_shaped_wall_detours() {
        // L 形墙：竖墙 x=5（y=0..=9）+ 横墙 y=9（x=5..=12）；起点 (2,5) 在 L 内侧，目标 (10,4)
        let mut grid = OccupancyGrid::new();
        for y in 0..=9 {
            for _ in 0..3 {
                grid.update(5, y, true);
            }
        }
        for x in 5..=12 {
            for _ in 0..3 {
                grid.update(x, 9, true);
            }
        }
        assert_eq!(grid.state(5, 5), Some(CellState::Occupied as u8));
        assert_eq!(grid.state(12, 9), Some(CellState::Occupied as u8));
        assert_eq!(
            grid.state(10, 4),
            Some(CellState::Unknown as u8),
            "目标格必须可通行"
        );

        let mut pf = DStarLite::new((2, 5), (10, 4));
        assert_eq!(
            pf.cost(&grid, (4, 5), (5, 5)),
            f32::MAX,
            "竖墙挡住直行"
        );
        let mut cur = (2, 5);
        let mut reached = false;
        for _ in 0..300 {
            let step = match pf.next_step(&grid, Duration::from_secs(3)) {
                StepResult::Step(c) => c,
                other => panic!("L 形墙必须能绕行，实际 {other:?}（当前 {cur:?}）"),
            };
            assert!(
                pf.cost(&grid, cur, step) < f32::MAX / 2.0,
                "不得走进不可通行格：{cur:?} -> {step:?}"
            );
            pf.move_to(step);
            cur = step;
            if cur == (10, 4) {
                reached = true;
                break;
            }
        }
        assert!(reached, "L 形墙绕行必须到达 (10,4)，实际停在 {cur:?}");
    }
}
