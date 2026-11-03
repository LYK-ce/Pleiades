# wb_45_follow_logic — task_45 工作记录

> Presented by KeJi
> Created Date ： 2026-11-03
> Modified Date ： 2026-11-03
> 任务：Task/task_45_follow_logic.md（Follow 直接跟人，不再走格子）

## 执行引擎启动
- 开始时间：2026-11-03 02:46:54Z
- 引擎：tmp/task45_tools/execution_engine_task45.ts + .config.json（流程照搬 task_44）
- 运行标记：task_45 首轮（Step 1~3 + 邮件）
- 计划：Step 1 / 2 最多 3 轮（规划 → 实现 → 评审）；Step 3 验证**只跑 1 轮**，失败记录后继续；Step 4 由引擎发邮件
- 范围：Follow 走点 + 执行器加类型（1）→ 真机真树（2）→ sim 验证 ugv/uav 的 Follow/Containment（3）→ 邮件（4）

## Step 1 — Follow 直接跟人 + 执行器加 MotionTarget（真车 / sim 车 / sim 机）（task_45 首轮（Step 1~3 + 邮件））
- 起止：2026-11-03 02:46:54Z → 2026-11-03 03:07:53Z
- 范围：base 行为树输出 + 四份执行器签名 + 四个调用点 + 三份 Follow 树 + 三处分派
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 74382c3
- 规划要点：计划已写入 [plan.md](tmp/task45/plan.md)（469 行，源码零改动，`git status` 与进场基线一致）。 **摘要（<200 字，可写入 workbook）** Step 1 计划：18 个文件，均在白名单。① `frame.rs` 加 `MotionTarget{Cell,Point}` + `MotionOut.target_point` + `target()`（点优先），必须去掉 `Eq`（f32 不可 Eq，全仓无依赖）。② 四份执行器只加一次 `match` 消解格心/点，五档、`omega` 符号链零改动；按「先改车→复制到 ugv_sim，改机→复制到 uav_sim」保证两两 diff 为空。③ 四调用点改 `decide(out.target(),…)`，`sub_target` 保留 `out.next_cell`（已知副作用不修）。④ 三份 Follow 树改 `target_point=Some((s.x,s.y))`，删 `step_cell/guard_cell/person_cells/FollowNav/nav/clear_goal` 及死参数 `person_inflate_m`；与 nav 解耦后 `containment` 的 bound 降为 `P: NavPort`。⑤ 三份树统一 2.2/1.2，先做母本再整份复制只补 `use` 与文件头。验收：逐 crate `check --all-targets` + `test --bins --no-run`（只编译不判单测）+ 四条 grep/diff 自证 + `git status` 恰 18 文件。首轮无上轮 issues。
