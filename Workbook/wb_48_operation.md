# wb_48_operation — 操作类任务：Face（到点朝向）+ Guard（值守警戒） 工作记录

> Presented by KeJi
> Created Date ： 2026-11-05
> Modified Date ： 2026-11-05
> 任务：Task/task_48_operation.md（操作类任务：Face（到点朝向）+ Guard（值守警戒））

## 执行引擎启动
- 开始时间：2026-11-05 11:01:30Z
- 引擎：tmp/task48_tools/execution_engine_task48.ts + .config.json（流程照搬 task_47）
- 运行标记：task_48 首轮
- 起始 step：1；计划：S1 → S2 → S3 → S4 → S5 → S6 → S7(mail)

## 执行引擎启动
- 开始时间：2026-11-05 11:05:09Z
- 引擎：tmp/task48_tools/execution_engine_task48.ts + .config.json（流程照搬 task_47）
- 运行标记：task_48 首轮
- 起始 step：1；计划：S1 → S2 → S3 → S4 → S5 → S6 → S7

## 执行引擎启动
- 开始：2026-11-05 11:07:29Z
- 引擎：tmp/task48_tools/（流程照搬 task_47）
- 标记：task_48 首轮
- 计划：S1 → S2 → S3 → S4 → S5 → S6 → S7

## 执行引擎启动
- 开始：2026-11-05 11:11:26Z
- 引擎：tmp/task48_tools/（流程照搬 task_47）
- 标记：task_48 首轮
- 计划：S1 → S2 → S3 → S4 → S5 → S6 → S7

## Step 1 — base：Face + Guard 两条协议（type=4/5）+ 四处 naive（task_48 首轮）
- 起止：2026-11-05 11:11:26Z → 2026-11-05 11:21:00Z
- 范围：base 的任务类型与 ORION 协议（element.rs / messages.rs / protocol/mod.rs）+ 四个 crate 的 naive 分派
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 6299a58
- 规划要点：计划已写入 [tmp/task48/plan.md](tmp/task48/plan.md)（534 行，源码零改动）。 要点：基线实测全绿（base lib + 三设备 crate 0 error，protocol 49 tests）。本步改 8 个文件、按行号锚定。两处校正：①「16 处构造点」实为 22 处（21 测试 + 1 生产 decode），逐条列了文件:行号；②`command_decode.rs:49-66` 的 type→`Mission` 映射 step.md 未点名但**必须改**，否则 type 4/5 帧被 `filter_map` 兜底静默丢弃（编译仍绿），已配「单发 Face」单测专防。四条金标串用独立脚本核过字节与长度（10/10/19/18）。验收：`cargo check -p pleiades-base --lib`、三设备 crate `--all-targets`、`cargo test -p pleiades-base --lib protocol`（49→54 passed），另给范围自查与 11 条风险规避（含不得加 fx/fy 校验、不得改 tests/、不得 fmt）。

## Step 2 — 叶子与树：Face 四端真叶子 + Guard 车端真树（UAV 保持 naive）（task_48 首轮）
- 起止：2026-11-05 11:21:00Z → 2026-11-05 11:40:05Z
- 范围：四个 crate 的 mission 目录（新增 face.rs / guard.rs，改 mod.rs 分派）
- 轮次：1 / 3
- 结果：PASS
- 提交：ok de35082
- 规划要点：计划已写入 [tmp/task48/plan.md](tmp/task48/plan.md)（824 行，源码零改动）。10 个文件：4 份 `face.rs` 整文件 `cp`（连 `use` 都相同）、2 份 `guard.rs` 用 `sed` 只差 `crate::` 路径、4 份 `mod.rs` 锚到行号（如 `ugv/mod.rs:26-28`、`88-104`）。关键设计：扇形朝向取 station→gaze 的世界系方位角（非车头 yaw）；Guard 显式传 `FollowParams{2.2,1.2}`；`HomeLeaf` 自实现不碰 `goto`；无 detector 走 `TargetSource::new(None)` 只值守。四处校正：uav 执行器死区是 5° 非 8°（故本地定义 `FACE_DEADBAND_DEG`）、「偏 5° Running」在 8° 下不成立（改 10°）、NaN 判据不能用 `!(a>b)`（改正向 `<=`）、60 拍收尾非 61。验收 5/8/5/10/8 passed，总数 201→214 / 175→180 / 475→493，范围自查 10 路径。
