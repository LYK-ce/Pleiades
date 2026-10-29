# task_39_executor_and_path — 行动路径与执行器优化

> Presented by KeJi
> Created Date ： 2026-10-29
> Modified Date ： 2026-10-29
> 状态：草案（Step 1a 方案已定：每拍重算 + 三态返回 + 停车等；Step 1b 待评审；Step 2 待讨论）

---

## 一、目标

优化行动路径与执行器。

1. **行动路径**：让路径更平顺——减少拐点、缩短路径长度（Step 1b：4 连通 → 8 连通）。
2. **执行器**：从"到格心 + 原地转向"的离散动作控制，改为支持连续运动的执行方式（如边走边转），使移动过程中能持续对准目标、不因转身丢失视野。（Step 2，待讨论）

两条共同遵守的原则：

- **先补"说法"，再补"行为"**：算不完 ≠ 没有路。这类区分要在接口上说清楚，不在下游打补丁。
- **不靠猜**：不去判断"有没有变化"。每拍照算，算不完就明说，让上层决定怎么办。

---

## 二、Step 1a — 规划预算与"没算完"

> 先做这一步，再做 1b（8 连通）。因为 8 连通会让扩展数上升，先把预算与返回状态理顺。

### 1 目标

1. 规划器能区分**"算完了、确实没有路"**与**"没算完、还不知道"**；
2. 预算按**时间**计、**可配置**（`plan_budget_ms`，默认 20），不按迭代次数；
3. 这一拍没算完 → **停车等**（不清目标、不判失败、下一拍接着算）。

### 2 要做什么

| # | 项 | 内容 |
|---|---|---|
| 1 | `next_step` 三态返回 | `Step(cell)` / `NoPath` / `NotReady`，取代 `Option<(i32,i32)>` |
| 2 | 预算按时间（可配） | 预算来自 config `plan_budget_ms`（默认 20 ms）；`compute_shortest_path` 循环里每 64 次查一次表，超预算返回 `BudgetExhausted` |
| 3 | `NoPath` 只在收敛后给 | 堆空 且 `rhs[start] = ∞` 才算"确实没有路" |
| 4 | 没算完就停车 | 叶子收到 `NotReady` → 写 `next_cell = None` → 执行器 `Stop`；`Status` 仍为 `Running` |
| 5 | 超时告警 | 连续 250 ms `NotReady` → 一条 `warn!`（给人看，不改变行为） |

### 3 涉及文件

| 文件 | 改什么 |
|---|---|
| `pleiades-ugv/src/ugv/nav/pathfinder.rs` | 新增 `StepResult` / 预算中断；`next_step` 签名（`next_step`:97、`compute_shortest_path`:257、`MAX_COMPUTE_ITERS`:18） |
| `pleiades-uav/src/uav/nav/pathfinder.rs` | 同上（与 ugv 逐字相同） |
| `pleiades-sim/src/sim/ugv_sim/nav/pathfinder.rs` | 同上（sim 变体，`next_step`:134） |
| `pleiades-sim/src/sim/uav_sim/nav/pathfinder.rs` | 同上（与 sim_ugv 逐字相同） |
| `pleiades-ugv/src/ugv/nav/navigator.rs` | `step`:190 透传三态 + 连续 NotReady 计数（`drain_map_deltas`:37、`plan_next`:70） |
| `pleiades-uav/src/uav/nav/navigator.rs` | 同上（与 ugv 逐字相同） |
| `pleiades-sim/src/sim/{ugv_sim,uav_sim}/nav/navigator.rs` | 同上（sim 变体） |
| `pleiades-{ugv,uav,sim}/src/config.rs` | 新增顶层键 `plan_budget_ms: Option<u64>`（缺省 20）+ `fill_*` 补全 + 单测 |
| `pleiades-ugv/src/ugv/mission/goto.rs` | `GoToLeaf::tick`:74 处理三态：`Step` 写格、`NotReady` 写 `None`、`NoPath` 维持现有 `Blocked` 去抖 |
| `pleiades-uav/src/uav/mission/goto.rs` / `pleiades-sim/src/sim/{ugv_sim,uav_sim}/mission/goto.rs` | 同上 |
| `Architecture/robot_arch.md` | 记录三态返回、时间预算 |

**不动**：执行器（Step 2 才动）、协议、`grid`、急停、`Mission`/队列。

### 4 详细实施步骤

#### 1.1 定义返回类型与预算（pathfinder）

```rust
pub enum StepResult {
    Step((i32, i32)),  // 有一格
    NoPath,            // 已收敛，确实没有路
    NotReady,          // 预算用完，还没算完（g/rhs 是半成品，不许读）
}

pub enum ComputeOutcome { Converged, BudgetExhausted }
```

- 预算**不写死**：`next_step(grid, budget)` 由调用方传入，值来自 config 的 `plan_budget_ms`（缺省 20 ms）。
- **配置位置**：放设备配置的**顶层键**（与 `obstacle_inflation_radius` 同级），**不放 `[chassis]`**——UAV 没有 `[chassis]` 段，而三端都有 nav。
- 三份 `config.rs` 各加 `pub plan_budget_ms: Option<u64>`；`fill_*` 缺键补 20；单测：缺键→20、已有值不被覆盖、幂等。
- `Navigator::new` 增加 `plan_budget: Duration` 参数（与 `obstacle_inflation_radius` 同一条装配路径），三处 bootstrap / handler 从 config 读并传入。
- `compute_shortest_path` 改为返回 `ComputeOutcome`：循环里 `iter % 64 == 0` 时检查 `Instant::now()`；超时**直接返回 `BudgetExhausted`**，不再继续读值。
- `next_step` 内部：`BudgetExhausted` → 立即返回 `StepResult::NotReady`（**不读 g/rhs**）；`Converged` 且 `rhs[start] = ∞` → `NoPath`；否则照现在的方式挑邻居 → `Step`。
- `start == goal` 一律 `NoPath`（正常路径不会走到，nav 早在到达判定里返回了）。

#### 1.2 navigator：三态透传 + 未算完计数

1. `Navigator` 新增一个字段：`not_ready_ticks: u32`（连续 `NotReady` 拍数）。
2. `step` 的流程（**每拍照算，不做"有没有变化"的判断**）：

   ```
   终态锁存 / 到达判定 / 格内逼近        （现状不变，命中就直接返回）
   drain 建图增量 → 取他车障碍集合 → plan_next 调 next_step(budget)
       Step(cell) → not_ready_ticks = 0，返回 Step(cell)
       NoPath     → 走现有 blocked_ticks 逻辑（20 拍才判 Blocked）
       NotReady   → not_ready_ticks += 1；>= 5 拍（250 ms）记一条 warn；返回 NotReady
   ```

3. `set_goal` / `clear_goal` / `arrive` 时清 `not_ready_ticks`。
4. `Navigator::step` 的返回类型携带三态（例如 `NavStep`），叶子据此分派。

> **为什么不做"无变化就不重算"**：车几乎一直在动（每 0.5 m 起点格就变），雷达画出来的边界每帧都在抖，建图增量几乎每拍都有 —— 这个判断在真机上大概率永远为假，只会多出一份常驻的无效缓存。**每拍重算，代价交给时间预算管。**
>
> 代价是：`sub_target` 仍可能因为等代价解翻面而变，执行器会 `Stop + Idle`。**这一点不在 1a 用迟滞去压**，留到 Step 2 从执行器侧解决（认"路径进度"，不认"格子相等"）。

#### 1.3 叶子：三种结果怎么写

| 结果 | `Frame.out.next_cell` | `Status` |
|---|---|---|
| `Step(cell)` | `Some(cell)` | `Running` |
| `NotReady` | `None`（→ 执行器 Stop，车停住等） | **`Running`**（不是 Failure） |
| `NoPath`（nav 累计 20 拍后置 `Blocked`） | `None` | `Failure(Blocked/Unreachable)`（现状不变） |

- **不清 goal、不出队、不重置 `sub_target`**：`NotReady` 只是"这一拍不发动作"。
- 连续 `NotReady` 到 250 ms 的那条 `warn!` 带上下文：起点格、目标格、已算拍数，便于排查。

#### 1.4 参数与实测

1. 首版 `plan_budget_ms = 20`（config 默认值；50 ms tick 里占 40%，实测再调）。
2. 实测两类场景并记录：空地图远目标（首次规划）、正常短任务（每拍重算）；记录单次耗时与 `NotReady` 出现次数。
3. 若"首次规划"经常连续 `NotReady` 超过 3 拍（150 ms），再考虑把预算调大或分帧，**不改这里的语义**。

#### 1.5 单测（每个变体各加 5 条）

1. **预算为 0**：`next_step` 返回 `NotReady`，**不是** `NoPath`。
2. **预算充足**：空地图目标返回 `Step`。
3. **收敛且无路**：目标被墙围死 → 返回 `NoPath`（不是 `NotReady`）。
4. **断点续算**：小预算连续调用多次，最终能返回 `Step`（中间可以多次 `NotReady`）。
5. **NotReady 不改变任务**：叶子收到 `NotReady` → `next_cell = None`、`Status::Running`、`nav.status()` 不变。

#### 1.6 文档

`Architecture/robot_arch.md` §1.3 / §2.1 补两条：`next_step` 三态、时间预算（并注明旧的迭代上限只作兜底、命中必须返回 `NotReady`）。

### 5 完成判据

1. `next_step` 只有三种返回；`NotReady` 在**任何**情况下都不会被当成 `NoPath` 处理。
2. 预算按时间计（旧的 `MAX_COMPUTE_ITERS` 命中时必须返回 `NotReady`，不再当作不可达）。
3. `NotReady` 期间：`Status = Running`、目标保留、车停住；连续 250 ms 打一条 warn。
4. config 缺键时自动补 `plan_budget_ms = 20`（幂等，已有值不被覆盖）；`Navigator::new` 的预算来自 config 而非硬编码。
5. 四份副本两两 `diff` 只剩文件头日期；原有单测不回归（真机 6、sim 19）。
6. 执行器 / 协议 / grid / 急停零改动。

### 6 不做

- 不改 8 连通（1b）；
- 不改执行器（Step 2）与急停；
- 不把 D* 改成后台任务、不改地图服务接口；
- **不做"有没有变化"的缓存判断**（理由见 1.2）。

---

## 三、Step 1b — 4 连通 → 8 连通（含回退 sim 膨胀）

### 1 目标

把 D* 的邻居集合从 4 连通扩到 8 连通：斜向路径从"阶梯"变"直线"（长度由 1.41× 逼近 1.0×）、拐点显著减少、单次转向角由 90° 降为 45°。

同时**把 sim 的可通行判据回退成与真机一致**（删掉 sim 单方面加的"1 格膨胀"）。

**不改**：格心语义、到达判据、执行器、nav 接口、协议、`CELL_RESOLUTION`、**急停逻辑**。

### 2 要做什么

| # | 项 | 内容 |
|---|---|---|
| 0 | 回退 sim 的 1 格膨胀 | sim 的 `cost()` 改回与真机逐字一致（只判本格 / 出界 / 动态障碍）；删除 `Blocked()` 的贴邻膨胀及对应单测 |
| 1 | 邻居集合 | `neighbors()` 4 → 8，固定顺序（决定 tie-break） |
| 2 | 边代价 | cardinal `1.0`；diagonal `√2` |
| 3 | 禁切角 | 对角边仅当两个相邻 cardinal 格都可通行时才存在 |
| 4 | 启发函数 | Manhattan → octile：`max(dx,dy) + (√2−1)·min(dx,dy)` |
| 5 | 修补范围 | `mark_obstacle` / `set_dynamic_obstacles` / `Notify_Changed` 按 8 邻域修补 |
| 6 | 选步 | 候选按固定 8 顺序收集；sim 的 `Choose_Step` / `TIE_EPS` 语义不变 |
| 7 | 迭代上限 | 由 Step 1a 的时间预算接管；旧常量保留为兜底（命中必须返回 `NotReady`） |

> 第 0 项是**语义对齐**，不是修 bug：急停"前方 0.3 m 内完全停住"是当前既定设计，本任务不动它。

### 3 涉及文件

| 文件 | 现状 |
|---|---|
| `pleiades-ugv/src/ugv/nav/pathfinder.rs` | 401 行（`neighbors`:197、`cost`:203、`heuristic`:192） |
| `pleiades-uav/src/uav/nav/pathfinder.rs` | 401 行，与 ugv 逐字相同 |
| `pleiades-sim/src/sim/ugv_sim/nav/pathfinder.rs` | 706 行（`Notify_Changed`:201、`cost`:278、`Blocked`:313） |
| `pleiades-sim/src/sim/uav_sim/nav/pathfinder.rs` | 706 行，与 sim_ugv 逐字相同 |
| `Architecture/robot_arch.md` | §2.1 与"关键常量速查" |

### 4 详细实施步骤

#### 4.1 回退 sim 的 1 格膨胀（先做，独立可验）

1. 删除 `Blocked()`（sim :313）及其"贴邻格不可通行"的整段理由注释。
2. `cost()`（sim :278）改回与真机逐字一致：动态障碍 / 出界 / 本格 `Occupied` → ∞，其余 `1.0`。
3. `Passable()` 若保留，语义改为**只看本格**，供禁切角使用。
4. `Notify_Changed()`（sim :201）修补环数 2 环 → 1 环（与真机 `mark_obstacle` 同口径）。
5. 删除单测：`test_cost_inflates_cells_adjacent_to_occupied`（:460）、`test_next_step_steps_around_inflated_wall`（:480）→ sim 由 19 条降为 17 条。
6. 补一条反向锁：**贴墙格可通行**（`cost(贴邻 Occupied 的空格) == 1.0`）。

> **已知后果（接受，不在本步处理）**：回退后 sim 可能再次出现"规划进贴墙格 → 急停每拍触发 → 冻结"——这正是真机当前行为。急停策略由后续任务统一处理。
>
> **待确认**：sim 还有 `Choose_Step` 等代价迟滞与 `Value()=min(g,rhs)`（D-S1）。1a 之后仍是每拍重算、每拍重挑，等代价翻面可能继续存在；是否保留迟滞，等 Step 2 落地（执行器认路径进度）后再看。

#### 4.2 定 8 连通契约（写进两变体的 module 注释）

1. **邻居顺序（固定，顺时针，从正北起）**：`N → NE → E → SE → S → SW → W → NW`
2. **边代价**：cardinal `1.0`，diagonal `SQRT_2`。
3. **禁切角**：`s → t` 对角边存在 ⟺ `passable(s.x+dx, s.y)` 且 `passable(s.x, s.y+dy)` 且 `passable(t)`。
4. **`passable(c)`（回退膨胀后两变体一致）**：在界内 && 本格 != `Occupied` && 不在动态障碍集合。

#### 4.3 真机版（401 行）

1. `neighbors()`（:197）返回 8 个（固定顺序）。
2. `cost()`（:203）用 `from` 判断对角；对角时检查两个 cardinal 中间格；返回 `1.0` / `SQRT_2`。
3. `heuristic()`（:192）改 octile。
4. `mark_obstacle()` / `set_dynamic_obstacles()`：修补邻居用 8 邻域。
5. `next_step()`（:97）：候选按 8 顺序遍历，保留"第一个严格更小"。
6. 文件头注释同步（"4 连通，Manhattan"）。

#### 4.4 sim 版（706 行）

1. 先做 4.1，再做 4.3 的 1~6。
2. `next_step()`（:134）候选容量 4 → 8。
3. `Choose_Step()` / `TIE_EPS` 不改。
4. `Notify_Changed()` 环数按 4.1 第 4 条。

#### 4.5 同步与单测

1. `diff`：`ugv ≡ uav`、`sim_ugv ≡ sim_uav`（只允许文件头日期不同）。
2. 每个变体各加 6 条单测：
   1. 对角生效（首步为对角格）；
   2. 禁切角（左）；
   3. 禁切角（右）；
   4. 对角代价 `√2`、cardinal `1.0`；
   5. octile 数值（`(0,0)→(3,4)` ≈ 5.2426）；
   6. L 形障碍下仍能绕行（`next_step` 非 `NoPath`）。

#### 4.6 文档

`robot_arch.md` §2.1 改为"8 连通（禁切角）、octile 启发、对角代价 √2"；"关键常量速查"加"对角步长 √2 × 0.5 m ≈ 0.707 m"；删除"1 格膨胀"相关表述。

### 5 完成判据

1. sim 与真机的 `cost()` / 可通行判据逐字一致。
2. 四个文件两两 `diff` 只剩文件头日期。
3. 新增单测全绿；原有不回归（真机 6、sim 17）。
4. 开阔地图斜向目标：路径长度 ≤ 1.05 × 欧氏（按 0.5 m 量化）；首步为对角。
5. 任一 cardinal 被占 → 对应对角边不存在。
6. `cargo check --all-targets` 零 error、新增代码零 warning。
7. `git diff` 只含 4 个 `.rs` + `robot_arch.md`。

### 6 不做

- 不做路径平滑 / 前瞻 / string-pulling、不换算法（RRT/A*/Theta*）；
- 不碰急停；不引 Unknown 代价；不做对角边的多车预留。

---

## 四、Step 2 — 执行器（待讨论）

目标：从"到格心 + 原地转向"的离散动作，改为支持连续运动（边走边转）的执行方式。

已知输入（前序讨论）：

- 命令需要二维（`(v, ω)`），现有 `MotionAction` 五枚举表达不了；
- UAV 的 MAVLink 链天然支持 `(vx, vy, vz, yaw_rate)`，改我们侧的映射即可；
- 仿真世界只认 `forward/backward/left/right/stop`，要扩指令集；
- 真车 UGV 有一条未验证的 `FUNC_MOTION`（vx/vy/vz）；
- 二维设定值 = 周期刷新语义，三端都需要超时兜底。

方案与步骤待讨论后补。
