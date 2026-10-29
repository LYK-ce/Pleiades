# task_39_executor_and_path — 行动路径与执行器优化

> Presented by KeJi
> Created Date ： 2026-10-29
> Modified Date ： 2026-10-29
> 状态：**已完成（2026-10-29）** —— 人类本地复验通过；遗留见 §七.3

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
> 代价是：`sub_target` 仍可能因为等代价解翻面而变，执行器会 `Stop + Idle`。**这一条本任务明确不解决**（人类 2026-10-29 决定），登记为已知限制：根因是"连续量被砍成格"+"执行器把格变当命令变"，与预算/8 连通无关。

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
- **不做"有没有变化"的缓存判断**（理由见 1.2）；
- **不解决"下一格在相邻两格之间翻面"**（见 1.2 的已知限制）。

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

## 四、Step 2 — 机栈执行器：连续速度控制（只做 sim 无人机）

> **定位：验证，不是替代。** 只额外添加一条 sim 机栈的连续执行路径：**不动 ORION 协议、不动手动遥控链、不动真机**。
> 协议层面的"速度指令取代 `MANUAL_CONTROL`"单开任务，见 §五。

### 1 目标

机栈执行器从"到格心 0.2 m + 原地转向"的离散动作，改成"每拍算一次 `(v, ω)`"：

- **目标点 = nav / 叶子给的下一格的格心**（方案 A，不做前瞻）；
- 偏角大 → `v=0` 只转；偏角小 → **匀速**直行（不做距离衰减）；
- 验证目标：切向通过不丢视野、不撞人、不原地打转。

### 2 范围

| 状态 | 文件 | 改什么 |
|---|---|---|
| 新增 | `pleiades-sim/src/sim/uav_sim/executor.rs` | 连续控制内核（复制自 `sim/executor.rs`，改内容） |
| 改 | `pleiades-sim/src/sim/sim_handler.rs` | 按栈分派：车 → 老执行器，机 → 新执行器 |
| 改 | `pleiades-sim/src/sim/chassis/mod.rs` | `Chassis` 加 `Apply_Velocity(v, ω)`，默认"不支持" |
| 改 | `pleiades-sim/src/sim/chassis/uav.rs` | 实现 `Apply_Velocity` → `MotionSender` |
| 改 | `pleiades-sim/src/world_link/{mod,protocol}.rs` | 新增出站速度指令（`v` / `omega`） |
| 改 | `orion-simulation-world/src/chassis/Quadrotor.gd` | `SetCommand` 认 `action="velocity"` |
| 按需 | `orion-simulation-world/src/net/*.gd` | 放行该消息/字段 |

**不动**：`pleiades-sim/src/sim/executor.rs`（车栈继续用）、`pleiades-base`（`ExecuteState` / `MotionAction` / 协议）、`pleiades-uav`（真机）、`pleiades-ugv`、`Pictor`、ORION 协议、手动遥控链。

> 车与机的执行器**本来就应该分开**（真机已经是两份）。sim 现在是共用一份，本步先把它们拆开：车保留离散，机新增连续。

### 3 控制律（2a 内核）

三个数都是定值，**没有比例、没有中间值**：`v ∈ {0, vel_fwd}`，`ω ∈ {−yaw_rate, 0, +yaw_rate}`。

> **2026-10-29 修订（人类决定）**：`v` / `ω` 是**物理量**（m/s、rad/s），来自 `[flight_ctrl]` 的
> `vel_fwd` / `yaw_rate_deg`——与 `pleiades-uav` **同名同单位**，不再借用车的 `[chassis]` 百分比档位
> （旧写法让 `forward_speed=100` 被世界 clamp 成满速、`turn_speed=30` 被当 30 rad/s 顶到 90°/s，
> 导致"调配置无效"）。

| 偏角 \|Δθ\| | v | ω | 车怎么动 |
|---|---|---|---|
| ≤ 5° | `vel_fwd`（m/s） | 0 | 直行 |
| 5° ~ 20° | `vel_fwd` | `±yaw_rate`（rad/s） | **边走边转（弧线）** |
| > 20° | 0 | `±yaw_rate` | 原地转 |

```text
每 50 ms：
  target = 叶子给的格（None → v = 0, ω = 0）
  Δθ = 归一化(指向格心的方位 − yaw)          // 顺时针为正

  if |Δθ| <= 5°:     v = vel_fwd, ω = 0
  elif |Δθ| <= 20°:  v = vel_fwd, ω = sign(Δθ) × yaw_rate
  else:              v = 0,       ω = sign(Δθ) × yaw_rate
```

- `vel_fwd`（m/s）/ `yaw_rate`（rad/s）来自 `[flight_ctrl]`（缺省 0.3 m/s、15°/s，与真机 UAV 默认一致）；
  sim 实例为验证设成 0.8 m/s、30°/s。世界侧 `clampf(v, ±max_speed)` / `clampf(omega, ±max_yaw_rate)`
  不再截断（2.0 m/s / 90°/s 是它的上限）。
- 两个角度写常量：`DEADBAND_DEG = 5`、`SPIN_DEG = 20`。
- **相对今天唯一的行为变化**：多出"5°~20° 边走边转"这一档（`v` 与 `ω` 同时非零）。
- 到达不归执行器管：nav 判到达 → 叶子写 `None` → `v=ω=0`。
- 状态：机栈只写 `ExecuteState.sub_target`；`state` 留 `Idle`（没人读）。`POSE` 广播不受影响。
- 已知小毛病：`ω` 是满舵，车会在 5° 死区边界小幅左右摆（周期约 0.4 s）。先接受，之后调。

### 4 下发路径（2b）

1. `Chassis` trait 加：

   ```rust
   fn Apply_Velocity(&self, v: f32, omega: f32) -> bool { false }  // 默认不支持
   ```

2. 机栈实现：`MotionSender::Send_Velocity(v, omega)` → 出站 JSON，例如：

   ```json
   { "msg": "motion_cmd", "seq": 12, "action": "velocity", "v": 1.2, "omega": 0.35 }
   ```

3. 车栈不实现 → 走原来的 `Apply_Action`，一行不改。

### 5 世界侧（2c）

`Quadrotor.SetCommand` 增加 `"velocity"` 分支：

```gdscript
"velocity":
    _speed_target = clampf(v, -_max_speed, _max_speed)
    _yaw_target   = clampf(omega, -_max_yaw_rate, _max_yaw_rate)
```

- 原有的 `forward / backward / left / right / stop` 分支**保留不动**（保底兼容；车栈还在用）；
- 补一条自检断言（`check_quadrotor` 或 `check_ws_manager`）：发速度指令后 `Step()` 产出的 `velocity/yaw_rate` 符合预期。

### 6 验证

- 场景：sim 无人机 + 行人**切向通过**（人沿 y≈64.5 横穿，机在出生点附近），相机 `rate_hz=10`；
- 判据（初稿，跑完再校准）：
  1. 检测命中率不低于改前（同一段路的 `person` 帧占比）；
  2. **不撞人**：全程最小距离 ≥ 2.0 m；
  3. **不打转**：10 s 内水平位移 > 5 m；
  4. 日志里不再出现"每 100~200 ms 一次 Stop/Turning 交替"；
- 产物：trace + 结论写 `Workbook/wb_39_*.md`。

### 7 不做

- ORION 协议、手动遥控（W+A）、Pictor —— 单开任务（§五）；
- 真机 UAV、真车 UGV（UGV 等 `FUNC_MOTION` 实车验证）；
- 前瞻目标点（方案 B）、把 nav 接回跟随；
- 车栈执行器（保持离散）。

---

## 五、Step 3 — 验证（常规移动 + 跟随）

### 1 目标

在改完的 sim 机栈上验证两件事：**常规移动没被改坏**、**跟随能跟住且不丢视野**。

### 2 常规移动（回归）

| 用例 | 怎么跑 | 判据 |
|---|---|---|
| 直线 Goto | 控制口 `goto` 到正东约 20 m | 到达；无越界/撞墙；无急停 |
| L 形 Goto | 需要一次 90° 拐的目标 | 到达；拐角不反复停走 |
| 长路径 | 30~50 格 | 到达；日志里不出现"每 100~200 ms 一次 Stop/Turning" |

统一判据：nav 判 `Arrived`、无 `Blocked`、无急停、日志里没有连续的 `Stop + Idle` 交替。

### 3 跟随（重点）

| 用例 | 场景 | 判据（绝对标准，不跟改前比） |
|---|---|---|
| F1 直线跟随 | 人沿 y≈64.5 向东走 20 s | 10 s 内机位移 ≥ 5 m；不掉队（距离不持续拉大）；不撞人 |
| F2 **切线通过（重点）** | 人从机前方横向穿过：(72,60) → (72,68)，**速度 0.6 m/s**（约 13 s 走完） | ① 能持续检测到人：最长连续出画 ≤ 1 s；② 机跟着人走：10 s 内位移 ≥ 5 m，且方向与人的移动方向一致；③ 不是"停着不动"：`动作=Stop` ≤ 3 次；④ 最小距离 ≥ 2.0 m |
| F3 丢失 | 关掉行人客户端 | 800 ms 后机停住，不乱飞 |
| F4 不撞人 | 全程 | 最小距离 ≥ 2.0 m |

> 这一轮**不做"改前 vs 改后"的对比**：现状在切向场景下基本跟不了，没有可比性。只回答一个问题——**改完之后能不能跟住**。

### 4 指标怎么取

- **检测命中率** = 有 `person` 的帧数 / 总帧数（从 `[detector]` 日志统计）；
- **位移** = 控制口 `status` 的 `x/y` 序列；
- **最小距离** = detector 日志里的 `距离=`；
- **停车次数** = 日志里 `动作=Stop` 的条数；
- **出画时长** = 连续无 `person` 的最长间隔。

### 5 产物

`RobotSim_TestEnv/results/task39_<UTC时间戳>/`：`trace.csv`、截图、`world.log` / `sim.log` / `ped.log`、`verdict.md`；结论写 `Workbook/wb_39_*.md`。

### 6 不做

- 真机 UAV / 真车 UGV；
- 多目标跟随、前瞻目标点；
- 协议层（ORION 速度消息）——另开任务。

---

## 六、后续任务（不在本任务范围）

1. **协议重置**：把"速度指令"提升为 ORION 消息（如 `ORION_VELOCITY_CMD`：`vx / vy / omega`），取代 `MANUAL_CONTROL` 的运动分支；按"并存 → 切默认 → 删旧的"三步走，Pictor 需同步。
2. **真机 UAV 跟随**：把 `Mission::Follow` 在真机上从平凡树接成真树（需要 detector 消费者 + 安全距离约束）。
3. **UGV 连续运动**：实车验证 `FUNC_MOTION`（`set_motion(vx, vy, vz)`），通过后再决定车端是否也走连续执行。
4. **"下一格在相邻两格间翻面"**：连续量砍成格 + 执行器把格变当命令变；本任务登记为已知限制。

---

## 七、收尾（2026-10-29）

### 1 交付

- 提交：**`4276bf8`**（36 个文件，+7817 / −602），已推送 `origin/robot_yolo`。
- 内容：
  - Step 1a：`next_step` 三态（`Step/NoPath/NotReady`）+ 时间预算 + `plan_budget_ms`（20 ms）；
  - Step 1b：回退 sim 的 1 格膨胀；4 连通 → 8 连通（octile、禁切角、对角 √2）；
  - Step 2：新增 `sim/uav_sim/executor.rs`（三档全有全无）+ `Chassis::Apply_Velocity` + 世界侧 `velocity` 帧；
  - 口径对齐：`v/ω` 改物理量（m/s、rad/s），取 `[flight_ctrl] vel_fwd / yaw_rate_deg`，与真机 UAV 同名同单位。

### 2 验证结论

| 来源 | 结论 |
|---|---|
| 自动验证（`RobotSim_TestEnv/results/task39_20261029T100655Z/verdict.md`） | 28 条判据 **PASS 23 / FAIL 5** |
| 人类本地复验（2026-10-29） | **通过** |

自动验证的关键结果：

- **M1/M2/M3 全部到达**（27.2 s / 29.2 s / 31.2 s，末位逐轴误差 ≤ 4.3 cm）——上一轮的"原地打转、60 s 超时"极限环消失；
- **F3（丢失）PASS**：关客户端前在动、关后停住，不再是"真空 PASS"；
- **F4（不撞人）PASS**：三窗最小距离 5.24 m；
- FAIL：`F1-2`、`F2-1/2/3`、`L9b`（原因见 §7.3）。

### 3 已知遗留（不阻塞收尾）

| # | 现象 | 根因 | 建议 |
|---|---|---|---|
| 1 | `F1-2` 末距离 23.7 m | 机速上限 0.8 m/s **< 人速 1.26 m/s**，算术上必然被拉开 | 提高 `vel_fwd`，或把行人降速 |
| 2 | `F2-1/2/3` 切向跟不住 | 检测链 3.34 Hz（单帧推理 300 ms）+ 跟随目标取 **0.5 m 格心** + "净位移方向"判据对切向场景不自洽 | 前瞻目标点；判据改口径或改场景 |
| 3 | `L9b` M2 拐角 `omega` 翻转 29 次 | `v/ω = 0.8 / 0.5236 ≈ 1.53 m`，转弯半径 **≫ 0.5 m 格** | 拐角降速，或提高 `yaw_rate` |
| 4 | detector 幻影 `person` 框（20 帧） | 单帧误检 + `follow.rs` 按 `range_m` 最近选目标 | 置信度 / 多帧一致性过滤 |
| 5 | sim 与真机三项 D-S1 差异（迭代上限 / 等代价迟滞 / `Value()`） | 历史补丁只落在 sim | 另开任务反哺（真机先用日志确认是否存在） |
| 6 | ORION 速度消息、手动 W+A | 本轮只做验证，未触及协议 | 见 §六 后续任务 |

### 4 备注

- 急停策略（前方 0.3 m 内完全停住）按人类要求**未改动**；
- "下一格在相邻两格之间翻面"按人类决定登记为**已知限制**（§二 1.2），本任务不解决；
- follow 行为树本轮未调整（仍为几何直奔、不经 nav）。
