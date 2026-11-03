# task_45_follow_logic — Follow 直接跟人（不再走格子）

> Presented by KeJi
> Created Date ： 2026-11-03
> Modified Date ： 2026-11-03
> 状态：Step 1 / Step 2 / Step 3 待人类确认。

## 一、目标

真车跟人时车头左右摆，摆到把人甩出画面。病根：Follow 不朝人开，朝**格子中心**开。

```
现在：StepLeaf 给一格 → executor 取格心 → 格心与人的真实方位最多差 35°  → 超过 12° 进「原地转」档 → 摆
改成：StepLeaf 直接给 (x, y) → executor 直接用 → 没有量化
```

做完之后：

1. **Follow 直接跟人**：目标是人当前的世界坐标，格与格心不再进入转向环。
2. **执行器只加类型，不改算法**：五档顺序、`8°`/`12°`/`0.5 m` 三个阈值、`ω` 的符号链一个字不动，只是目标从「格」变成「格或点」。
3. **四份 Follow 树同形**：`pleiades-ugv`、`pleiades-uav`、`sim/ugv_sim`、`sim/uav_sim`，两两只允许差 `use` 与文件头。
4. **真机补上真 Follow 树**：`pleiades-uav` 现在 `Follow` 是一拍 `Success` 的平凡树，本次建成与车同构的真树。
5. **Goto / 巡逻不受影响**：仍走格 + D\*，`nav/` 一行不改。

已定口径：

| # | 项 |
|---|---|
| 1 | 点优先，格其次，都没有 = 停；`MotionOut` 保留 `next_cell`，新增 `target_point` |
| 2 | 执行器只有一个入口 `decide(Option<MotionTarget>, …)`，不再有第二个函数 |
| 3 | 四份执行器两两保持「只差文件头」：真车 ≡ sim 车、真机 ≡ sim 机 |
| 4 | 四份 Follow 树统一切到点，并统一 `D_FAR_M = 2.2` / `D_NEAR_M = 1.2` |
| 5 | `step_cell` / `guard_cell` / `person_cells` 删除（永远不触发） |
| 6 | Follow 不再持 nav（`FollowNav` / `clear_goal` 一并删）；巡逻分支仍用 `goto::build(nav)` |
| 7 | 丢目标就丢：不做搜索、不做记忆转圈、不加滤波 |
| 8 | 真机无 detector / 无相机 → **原地 Hold**（`Running` + 不动），不得一拍 `Success` |
| 9 | 不改协议、不改急停、不改 `0x12`、不改 Pictor、不新增阈值 |
| 10 | 单测不作验收判据；用例与口径冲突时改用例，禁止为迁就用例改生产代码 |

---

## 二、Step

### Step 1 — Follow 与执行器改动（真车 / sim 车 / sim 机）

**小目标**：Follow 的叶子把人的坐标交给执行器；执行器用类型区分「格」与「点」，算法不变；现有三份 Follow 树统一切换到点。

**涉及文件**

| 层 | 文件 |
|---|---|
| base | `pleiades-base/src/Robot/core/behavior_tree/frame.rs` |
| 执行器 ×4 | `pleiades-ugv/src/ugv/executor.rs`、`pleiades-sim/src/sim/ugv_sim/executor.rs`、`pleiades-sim/src/sim/uav_sim/executor.rs`、`pleiades-uav/src/uav/executor.rs` |
| 调用点 ×4 | `pleiades-ugv/src/ugv/robot_handler.rs`、`pleiades-sim/src/sim/sim_handler.rs`（两栈各一处）、`pleiades-uav/src/uav/robot_handler.rs` |
| Follow 树 ×3 | `pleiades-ugv/src/ugv/mission/follow.rs`、`pleiades-sim/src/sim/ugv_sim/mission/follow.rs`、`pleiades-sim/src/sim/uav_sim/mission/follow.rs` |
| 树的分派 ×3 | `pleiades-ugv/src/ugv/mission/mod.rs`、`pleiades-sim/src/sim/ugv_sim/mission/mod.rs`、`pleiades-sim/src/sim/uav_sim/mission/mod.rs`（+ 各自的 `containment.rs` 若传参变化） |

**实施步骤**

1. base `frame.rs`
   - 新增 `pub enum MotionTarget { Cell((i32, i32)), Point((f32, f32)) }`（`Copy + Clone + Debug + PartialEq`）。
   - `MotionOut` 新增 `pub target_point: Option<(f32, f32)>`。
   - 新增 `impl MotionOut { pub fn target(&self) -> Option<MotionTarget> }`：点优先，其次格，都没有 → `None`。

2. 执行器 ×4
   - 签名改为 `decide(&self, target: Option<MotionTarget>, x: f32, y: f32, yaw: f32) -> VelocityCmd`。
   - 进门先 `match` 出 `(tx, ty)`：`Cell` → `cell_center`；`Point` → 原值。
   - 其后（`d` / `delta` / 五档判断 / `omega` 符号）**原样保留**。
   - 四份文件两两保持只差文件头。

3. 调用点 ×4
   - `d.executor.decide(out.next_cell, …)` → `d.executor.decide(out.target(), …)`。

4. Follow 树 ×3
   - `Advance` 分支改成：`f.out.target_point = Some((s.x, s.y));`、`f.out.next_cell = None;`。
   - 其余分支两者都 `None`（现在是 `next_cell = None`）。
   - 删 `step_cell` / `guard_cell` / `person_cells` 及其引用。
   - Follow 不再持 nav：删 `FollowNav` trait、树里的 `nav` 字段、`clear_goal()` 调用；`build()` 不再收 `nav` 参数。
   - `D_FAR_M` / `D_NEAR_M` 三份统一为 `2.2` / `1.2`。

5. 树的分派 ×3
   - `mission/mod.rs` 的 `Follow` 分支改调新签名。
   - `Containment` 的巡逻分支仍 `goto::build(nav)`；若 `containment.rs` 里复用了 `follow::build`，跟着改。

6. 收尾核对
   - `grep -rn 'step_cell\|guard_cell\|person_cells\|FollowNav' pleiades-*/src` 零命中。
   - 四份执行器两两 `diff` 只剩文件头；三份 Follow 树两两 `diff` 只剩 `use` 与文件头。
   - `apply_velocity` / 急停 / 协议 / Pictor 零改动（`git diff --stat` 自证）。

---

### Step 2 — 真机 Follow：把平凡树换成真树

**小目标**：`pleiades-uav` 收到 `Follow` 时真的跟人（与车同构的树），而不是一拍 `Success` 就消失。

**涉及文件**

| 层 | 文件 |
|---|---|
| 新建 | `pleiades-uav/src/uav/mission/follow.rs`（以 `ugv/mission/follow.rs` 为母本，只差 `use` 与文件头） |
| 分派 | `pleiades-uav/src/uav/mission/mod.rs`（`Follow` 分支 → 真树 / 无 detector → 原地 Hold） |
| 装配 | `pleiades-uav/src/uav/robot_handler.rs`（detector 取用、`TaskDeps`、`build_task` 传目标源） |
| 设备 | `pleiades-uav/src/device/detector/mod.rs`（`Target` / `latest_targets()` 若缺字段就补齐；已存在则不动） |
| 配置 | `pleiades-uav/src/config.rs`（`[detector]` / `[detector.camera]` 已有则不动） |

**实施步骤**

1. 新建 `uav/mission/follow.rs`
   - 从 `pleiades-ugv/src/ugv/mission/follow.rs` 复制，只改 `use` 路径与文件头。
   - 依赖：`TargetFeed` / `TargetSource` / `Target` / `Sample` 全部照搬；`decide` 的五档与 `D_FAR_M` / `D_NEAR_M` 与车完全一致。
   - 因为 Step 1 已经删掉 nav 依赖，真机这份天然不需要 nav。

2. `uav/mission/mod.rs`
   - `Mission::Follow` 分支：有 detector → `follow::build(robot_state, source, params)`；无 detector / 无相机 → 原地 Hold（`Action` 恒 `Running` + `target_point = None`），**不得**一拍 `Success`、**不得**返回 `Err`。
   - `Containment` 保持现状（真机仍平凡），本次不动。

3. `uav/robot_handler.rs`
   - 把 detector 设备交给任务层：构造 `TargetSource::new(Some(detector))`，与 `Arc<TargetSource>` 共享（与车同款：一份源、一处 age 追踪）。
   - `build_task` 分派时把目标源传进 `uav/mission/mod.rs`。
   - detector 未启用（`[detector].enabled = false`）→ 目标源为 `None` → 上面的就地 Hold 分支生效。
   - `on_tick` 的决策调用沿用 Step 1 的 `decide(out.target(), …)`。

4. 真机核对（静态）
   - `grep -n 'Follow' pleiades-uav/src/uav/mission/mod.rs`：不应再出现"一拍 Success"的实现。
   - `diff` 真机与真车的 `follow.rs`：只剩 `use` 与文件头。
   - `[detector].enabled` 缺省仍是 `false`：不配置时真机 `Follow` 表现为原地不动（这是期望行为，不是 bug）。

> 说明：真机 Follow 依赖 `camera_uav`（或 `camera_pc`）真实出帧 + `yolov8n.safetensors` 在 workspace 里；缺任一条件时走 Hold。

---

### Step 3 — sim 验证（ugv + uav 的 Follow / Containment）

**小目标**：在仿真里分别看到 **sim ugv** 与 **sim uav** 的 **Follow** 与 **Containment** 现象出现。

**涉及文件**：**不改源码**。只起仿真并用现成控制口注入任务；产物落 `RobotSim_TestEnv/results/task45_s3_<ts>/`。

**实施步骤**

1. 起环境：世界（`RobotSim_TestEnv/world_fast` 或现成副本）+ Pictor（headless）+ 1 台 sim ugv（带 `camera_front` + `[detector].enabled = true`）+ 1 台 sim uav（同）。
2. 目标人：世界侧放 1 个行人，摆在设备前方 3~5 m。
3. **sim ugv — Follow**：注入 Follow，观察
   - 车朝人靠近（而不是朝某个格子中心）；
   - 车头不再出现「走一步、摆一下」；
   - 到 `1.2 m` 左右停住。
4. **sim ugv — Containment**：注入 Containment，观察
   - 出现方形巡逻（四个角顺时针）；
   - 人进入方框后切到跟随；
   - 人离开方框后回到巡逻。
5. **sim uav**：重复 3、4（同一套判据）。
6. 记录：每个 case 一段日志摘录（`[Sim] UGV/UAV 位置=…`、`[Nav] 新目标 …`、`[Robot] 开始任务 …`）+ 一句话结论。

**判据（重要）**

- **只判「现象是否出现」**：Follow 有没有跟、Containment 有没有巡逻/切分支。
- **不设任何数值门槛**：不判到位误差、不判摆幅、不判耗时、不判 tick 频率、不判弧线半径。数值只记录，不参与 PASS/FAIL。
- 出现「车完全不动」「任务不进队」「日志报错」才算 FAIL。

**收尾**：关世界与所有进程，端口/test 进程无残留；`git status` 与进场一致（本步不改代码）。
