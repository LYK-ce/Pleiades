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
