# wb_46_robot_simulation_refine — Robot 仿真完善（Follow 停住朝人 + 行人常驻控制台 + 删预置行人 + 验证） 工作记录

> Presented by KeJi
> Created Date ： 2026-11-03
> Modified Date ： 2026-11-03
> 任务：Task/task_46_robot_simulation_refine.md

## 执行引擎启动
- 开始时间：2026-11-03 13:13:56Z
- 引擎：tmp/task46_tools/execution_engine_task46.ts + .config.json（流程照搬 task_45）
- 运行标记：task_46 首轮（Step 1~4 + 邮件）
- 起始 step：1；计划：S1 → S2 → S3 → S4(verify) → S5(mail)

## Step 1 — MotionTarget 加 Face + Follow 停住时朝人（task_46 首轮（Step 1~4 + 邮件））
- 起止：2026-11-03 13:13:56Z → 2026-11-03 13:30:15Z
- 范围：base 行为树输出 + 四份执行器 + 四份 Follow 树
- 轮次：1 / 3
- 结果：PASS
- 提交：ok d160fb3
- 规划要点：计划已完成并写入 [plan.md](tmp/task46/plan.md)（577 行，含改动清单与行号、最终代码、8 步顺序、12 条风险、10 组验收判据）。 **计划摘要（供 workbook）** 计划已写 tmp/task46/plan.md：base 加 `Face` 变体与 `face_point`，`clear()` 三清、`target()` 按 face>point>cell；四份执行器只在进门消解加 `rotate_only`（跳过身后直行档、v 恒 0、ω 走原符号链），Cell/Point 逐字不动；四份 follow.rs 的 `HoldLeaf` 改持快照（太近发朝向、丢失发 None），`StepLeaf` 带内停同样发朝向，树形状不变。实测基线 uav 有 2 条红单测（466、601，硬编码 1.7/1.95 落在 D_NEAR=2.6 外），比 step.md 多一条，统一用 MID_BAND 修；机栈 DEADBAND=5°，「8°→(0,0)」改用 DEADBAND−1。验收全静态：4 crate check + `--bins` 单测 + 三条 diff + `grep face_point` 落点。

## Step 2 — 行人常驻控制台 ped_console（ws 套 ws）（task_46 首轮（Step 1~4 + 邮件））
- 起止：2026-11-03 13:30:15Z → 2026-11-03 14:02:15Z
- 范围：orion-simulation-world/tool/pedestrian/ 新增 1 个 python 工具
- 轮次：2 / 3
- 结果：PASS
- 提交：ok 763be1f
- 规划要点：计划已写入 [plan.md](tmp/task46/plan.md)（259 行）。 **计划摘要（供 workbook，约 190 字）**：`prev_issues.md` 只有 4 行 `[object Object]`（引擎把对象数组 `join`、`review.json` 已删，不可恢复），故按 §二 逐条重审出 7 项定点改，只动 `tool/pedestrian/`：F1 控制 20 Hz + 发送 30 Hz 解耦（同时满足 §二 1 与 2）；F2 冻结窗只在连续 `v>0` 计时、补 `_finish_goal` 返回值；F3 `WsClosed`→`ProtocolError`，断链不出 traceback；F4 `--max-speed`/`--port` 校验；F5 pose 读取持 `_io`；F6 docstring 补三条；F7 README 补一行。验收全静态：`py_compile`、`--help`、无世界六条 `not_ready`、`quit` rc0、SIGINT 130、`git status` 仅 2 文件。 未改任何源码（`ped_console.py` 仍是 round 1 的未跟踪文件）。
