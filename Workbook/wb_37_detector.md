# wb_37_detector — task_37 工作记录

> Presented by KeJi
> Created Date ： 2026-10-28
> Modified Date ： 2026-10-28
> 任务：Task/task_37_detector.md（相机帧槽位 + detector 设备 + 位置估计 + sim 跟随，Step 1~4）

## 执行引擎启动
- 开始时间：2026-10-28 03:44:29Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：首轮（Step 1~4）（本次从 Step 0 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次
- 范围：相机槽位（Step 1）→ detector 设备（Step 2）→ 位置估计（Step 3）→ sim 跟随（Step 4）
- 前置：Task 36 已提交（79f463d），sim 双栈 / build_task / TickOutcome 就位

## Step 1 — 相机统一：常驻抓取 + 共享帧槽位（首轮（Step 1~4））
- 起止：2026-10-28 03:44:29Z → 2026-10-28 05:34:53Z
- 范围：pleiades-uav device/camera_{pc,uav} + config；pleiades-sim 相机 sensor
- 轮次：3 / 3
- 结果：FAIL（已中断）
- 规划要点：计划已写入 [tmp/task37/plan.md](tmp/task37/plan.md)（782 行，未改任何源码；`git status` 与入场时一致）。 **摘要（200 字内）** 第 1 轮留下半成品：`config.rs`、`camera_uav/{mod,frame,slot}.rs`、`camera_pc/slot.rs` 已完成，`camera_pc/mod.rs` 零改动且两处测试字面量缺新字段导致 `E0063`×2。本轮只做 9 步：先补 `camera_pc` 三处字面量恢复编译 → 挂 `pub mod slot;` → 加 `at_ms`/`slot` 字段 → worker 换 `Tick_Mode_Of`（常转 10 Hz/空闲无超时 `recv`）+ `Capture_And_Publish` → `capture_impl` 槽位优先 → 4 个单测 → 消 pc/uav 两条新 warning（`Describe_Stream` 接进 bootstrap、`Jpeg` 加 `allow`）→ sim 加 `CameraFrame::captured_at_ms()` 与 `latest_frame()` + 2 单测 → 文档 §8.6。验收 C1~C10：两侧 `cargo check --all-targets` 零 error、四项单测全绿、Lua 契约不回归、改动只在允许路径、两侧互不引用。
- 3 轮未通过的问题：
规划子 agent 连续失败（已重试）：Error: dispatchsubagent: subagent run ended with error

## 引擎结束
- 结束时间：2026-10-28 05:34:53Z
- 状态：ABORTED — Step 1（相机统一：常驻抓取 + 共享帧槽位）3 轮未通过
- git status（结束时）：
```
 M pleiades-sim/src/sim/sensors/camera/frame.rs
 M pleiades-sim/src/sim/sensors/camera/mod.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/config.rs
 M pleiades-uav/src/device/camera_pc/mod.rs
 M pleiades-uav/src/device/camera_uav/frame.rs
 M pleiades-uav/src/device/camera_uav/mod.rs
?? Task/task_37_detector.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task37.config.json
?? Tool/execution_engine_task37.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_37_detector.md
?? pleiades-uav/src/device/camera_pc/slot.rs
?? pleiades-uav/src/device/camera_uav/slot.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```

## 执行引擎启动
- 开始时间：2026-10-28 06:33:54Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：重跑 Step 1~4（上游中转站失败后）（本次从 Step 1 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 3 次（间隔 2s）
- 范围：相机槽位（Step 1）→ detector 设备（Step 2）→ 位置估计（Step 3）→ sim 跟随（Step 4）
- 说明：上一轮中断原因是上游中转站请求失败，非代码问题；Step 1 已有大半实现

## Step 1 — 相机统一：常驻抓取 + 共享帧槽位（重跑 Step 1~4（上游中转站失败后））
- 起止：2026-10-28 06:33:54Z → 2026-10-28 07:45:56Z
- 范围：pleiades-uav device/camera_{pc,uav} + config；pleiades-sim 相机 sensor
- 轮次：3 / 3
- 结果：FAIL（已中断）
- 规划要点：
- 3 轮未通过的问题：
规划子 agent 连续失败（已重试 3 次）：Error: dispatchsubagent: subagent run ended with error

## 引擎结束
- 结束时间：2026-10-28 07:45:56Z
- 状态：ABORTED — Step 1（相机统一：常驻抓取 + 共享帧槽位）3 轮未通过
- git status（结束时）：
```
 M pleiades-sim/src/sim/sensors/camera/frame.rs
 M pleiades-sim/src/sim/sensors/camera/mod.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/config.rs
 M pleiades-uav/src/device/camera_pc/mod.rs
 M pleiades-uav/src/device/camera_uav/frame.rs
 M pleiades-uav/src/device/camera_uav/mod.rs
?? Task/task_37_detector.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task37.config.json
?? Tool/execution_engine_task37.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_37_detector.md
?? pleiades-uav/src/device/camera_pc/slot.rs
?? pleiades-uav/src/device/camera_uav/slot.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```

## 执行引擎启动
- 开始时间：2026-10-28 07:56:11Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：分段重跑 Step 1（上游中转站失败后）（Step 1~1）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 4 次（间隔 5s）
- 说明：上游中转站不稳定，改为分段重跑；Step 1 已有大半实现

## Step 1 — 相机统一：常驻抓取 + 共享帧槽位（分段重跑 Step 1（上游中转站失败后））
- 起止：2026-10-28 07:56:11Z → 2026-10-28 11:01:39Z
- 范围：pleiades-uav device/camera_{pc,uav} + config；pleiades-sim 相机 sensor
- 轮次：2 / 3
- 结果：PASS
- 规划要点：7 条 issue 全在文档层，代码与单测已全绿（uav 66 / sim 19 / config 9）。本轮 7 处改动：`config.rs:3`、`bootstrap.rs:3` 补 `Modified Date 2026-10-28`；`camera_uav/mod.rs:14-17` 改写线程模型（worker 持 `Arc<Uav_Inner>` 自写槽位）并补 task_37 增补段；`camera_pc/mod.rs:14-16` 删「完全不变」「回传 Pc_Frame」；`sim/camera/frame.rs` 五字段补 `///`（世界时间 vs 本机单调毫秒、seq 可回退）；删 `camera_uav/frame.rs` 的 `Jpeg()` 与 allow，4 处单测改 `Into_Jpeg()`；`robot_arch.md:4` 登记 Task 37。零逻辑改动、禁 `cargo fmt`，验收 V1~V7（V6 判 diff 只含注释行）。

## 引擎结束
- 结束时间：2026-10-28 11:01:39Z
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M Architecture/robot_arch.md
 M pleiades-sim/src/sim/sensors/camera/frame.rs
 M pleiades-sim/src/sim/sensors/camera/mod.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/config.rs
 M pleiades-uav/src/device/camera_pc/mod.rs
 M pleiades-uav/src/device/camera_uav/frame.rs
 M pleiades-uav/src/device/camera_uav/mod.rs
?? Task/task_37_detector.md
?? Task/task_38_sim_world_refine.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task37.config.json
?? Tool/execution_engine_task37.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_37_detector.md
?? pleiades-uav/src/device/camera_pc/slot.rs
?? pleiades-uav/src/device/camera_uav/slot.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```

## 执行引擎启动
- 开始时间：2026-10-28 11:12:39Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：分段重跑 Step 2（detector 设备）（Step 2~2）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 4 次（间隔 5s）
- 说明：Step 1 已 PASS；本次推 Step 2（detector 设备，uav + sim）

## 执行引擎启动
- 开始时间：2026-10-28 12:15:16Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：重跑 Step 2（换用新中转站）（Step 2~2）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 4 次（间隔 5s）
- 说明：Step 1 已 PASS；旧中转站故障期间 Step 2 未产生代码，本次换新中转站重跑 Step 2

## Step 2 — detector 设备（uav + sim）（重跑 Step 2（换用新中转站））
- 起止：2026-10-28 12:15:16Z → 2026-10-28 12:33:47Z
- 范围：pleiades-uav + pleiades-sim（设备 + 配置 + 日志）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写好。核对了一遍仓库现状：没有 detector 代码，两侧相机都已具备用 `latest_frame()` 同步读取最新帧的接口，`Pleiades_Workspace/yolov8n.safetensors` 存在（所以单测必须绕开真实模型加载），且 `device/mod.rs` 不在允许改动清单内。计划把每个文件的完整代码、行号引用点、实施顺序和 12 条验收判据都写清楚了。 **摘要（200 字内，供 workbook）** plan.md 已写（零源码改动）。uav 新建 device/detector/mod.rs：FrameSource 薄适配 latest_frame、DetectorEngine 包 base Yolo_Detector、spawn + 100ms 轮询 + spawn_blocking + 覆盖槽位 + 7 单测；sim 同构仅差 4 处；两侧 config.rs 加 [detector] 5 键同默认；bootstrap push 前留 Arc<Uav_Camera> 传 handler；sim_handler 5.5 段接线，on_tick 零 diff；device/mod.rs 不许改→robot_handler 用 #[path]；验收 C1~C12。

## 引擎结束
- 结束时间：2026-10-28 12:33:47Z
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M Architecture/robot_arch.md
 M pleiades-sim/src/config.rs
 M pleiades-sim/src/sim/mod.rs
 M pleiades-sim/src/sim/sensors/camera/frame.rs
 M pleiades-sim/src/sim/sensors/camera/mod.rs
 M pleiades-sim/src/sim/sim_handler.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/config.rs
 M pleiades-uav/src/device/camera_pc/mod.rs
 M pleiades-uav/src/device/camera_uav/frame.rs
 M pleiades-uav/src/device/camera_uav/mod.rs
 M pleiades-uav/src/uav/robot_handler.rs
?? Task/task_37_detector.md
?? Task/task_38_sim_world_refine.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task37.config.json
?? Tool/execution_engine_task37.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_37_detector.md
?? pleiades-sim/src/sim/detector/mod.rs
?? pleiades-uav/src/device/camera_pc/slot.rs
?? pleiades-uav/src/device/camera_uav/slot.rs
?? pleiades-uav/src/device/detector/mod.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```

## 执行引擎启动
- 开始时间：2026-10-28 12:34:39Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：分段重跑 Step 3（位置估计）（Step 3~3）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 4 次（间隔 5s）
- 说明：Step 1 / Step 2 已 PASS；本次推 Step 3（像素 → 世界 (x,y)）

## Step 3 — 位置估计：像素 → 世界 (x,y)（分段重跑 Step 3（位置估计））
- 起止：2026-10-28 12:34:39Z → 2026-10-28 12:55:01Z
- 范围：uav + sim 的 detector（投影纯函数 + 配置）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task37/plan.md)（零源码改动，`git status` 与入场时一致）。 **摘要（194 字）** plan.md 已写（零源码改动）。两侧同一份 projection.rs：机体系基在 spawn 由 mount yaw/pitch 生成，pose.yaw→世界，与 z=ground_z 求交出 (x,y)+距离/方位，5 条拒答。9 键 [detector.camera] 进两侧 config.rs（sim:491 补 camera:None）；Run_Loop 只加日志+槽位。 **规划期已实测的两处关键结论**（替实现轮省掉返工） 1. `#[path = "../device/detector/mod.rs"]` 下再写 `pub mod projection;` 能解析到同目录 `projection.rs`（rustc 最小样例通过）。 2. 计划里的 `projection.rs` 原样抽进最小 crate 跑通：**14/14 测试通过、0 warning**，golden 数值（4.1965 m / 2.3440 m / ±1.0547 m 等）与手算一致；`fill_detector_camera` 的嵌套 `[detector.camera]` 渲染与幂等也实测通过。

## 引擎结束
- 结束时间：2026-10-28 12:55:01Z
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M Architecture/robot_arch.md
 M pleiades-sim/src/config.rs
 M pleiades-sim/src/sim/mod.rs
 M pleiades-sim/src/sim/sensors/camera/frame.rs
 M pleiades-sim/src/sim/sensors/camera/mod.rs
 M pleiades-sim/src/sim/sim_handler.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/config.rs
 M pleiades-uav/src/device/camera_pc/mod.rs
 M pleiades-uav/src/device/camera_uav/frame.rs
 M pleiades-uav/src/device/camera_uav/mod.rs
 M pleiades-uav/src/uav/robot_handler.rs
?? Task/task_37_detector.md
?? Task/task_38_sim_world_refine.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task37.config.json
?? Tool/execution_engine_task37.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_37_detector.md
?? pleiades-sim/src/sim/detector/mod.rs
?? pleiades-sim/src/sim/detector/projection.rs
?? pleiades-uav/src/device/camera_pc/slot.rs
?? pleiades-uav/src/device/camera_uav/slot.rs
?? pleiades-uav/src/device/detector/mod.rs
?? pleiades-uav/src/device/detector/projection.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```

## 执行引擎启动
- 开始时间：2026-10-28 12:55:26Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：分段重跑 Step 4（sim 跟随）（Step 4~4）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 4 次（间隔 5s）
- 说明：Step 1 / 2 / 3 已 PASS；本次推 Step 4（Mission::Follow + sim 跟随树）

## Step 4 — sim 无人机跟随（Mission::Follow）（分段重跑 Step 4（sim 跟随））
- 起止：2026-10-28 12:55:26Z → 2026-10-28 13:18:24Z
- 范围：base 协议 + 真机平凡树 + sim 跟随树
- 轮次：1 / 3
- 结果：PASS
- 规划要点：`tmp/task37/plan.md` 已写好，共 812 行，本次未改动任何源码。 **摘要（约 199 字）：** plan.md 已写（零改动）。type=2→Follow；真机 uav/ugv 用平凡树，一拍 Success+warn，禁用 Err 返回；sim 新建 follow.rs：judge 四态纯函数 + ReactiveSequence/HoldLeaf，外层用 ReactiveFallback（本仓库 Fallback 有记忆，Hold 返回 Running 会一直停在该分支）；人作为动态障碍否决 next_cell；控制口加 follow；验收 C1~C13。

## 引擎结束
- 结束时间：2026-10-28 13:18:24Z
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M Architecture/robot_arch.md
 M docs/design_doc/orion_protocol.md
 M pleiades-base/src/Robot/core/mission_queue/element.rs
 M pleiades-base/src/Robot/core/protocol/command_decode.rs
 M pleiades-base/src/Robot/core/protocol/messages.rs
 M pleiades-base/src/Robot/core/protocol/mod.rs
 M pleiades-sim/src/config.rs
 M pleiades-sim/src/sim/control.rs
 M pleiades-sim/src/sim/mod.rs
 M pleiades-sim/src/sim/sensors/camera/frame.rs
 M pleiades-sim/src/sim/sensors/camera/mod.rs
 M pleiades-sim/src/sim/sim_handler.rs
 M pleiades-sim/src/sim/uav_sim/mission/mod.rs
 M pleiades-sim/src/sim/uav_sim/mod.rs
 M pleiades-sim/src/sim/ugv_sim/mission/mod.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/config.rs
 M pleiades-uav/src/device/camera_pc/mod.rs
 M pleiades-uav/src/device/camera_uav/frame.rs
 M pleiades-uav/src/device/camera_uav/mod.rs
 M pleiades-uav/src/uav/mission/mod.rs
 M pleiades-uav/src/uav/robot_handler.rs
 M pleiades-ugv/src/ugv/mission/mod.rs
?? Task/task_37_detector.md
?? Task/task_38_sim_world_refine.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task37.config.json
?? Tool/execution_engine_task37.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_37_detector.md
?? pleiades-sim/src/sim/detector/mod.rs
?? pleiades-sim/src/sim/detector/projection.rs
?? pleiades-sim/src/sim/uav_sim/mission/follow.rs
?? pleiades-uav/src/device/camera_pc/slot.rs
?? pleiades-uav/src/device/camera_uav/slot.rs
?? pleiades-uav/src/device/detector/mod.rs
?? pleiades-uav/src/device/detector/projection.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```

## 执行引擎启动
- 开始时间：2026-10-28 13:35:45Z
- 引擎：Tool/execution_engine_task37.ts + Tool/execution_engine_task37.config.json
- 运行标记：Step 5（sim 直线跟随）（Step 5~5）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 4 次（间隔 5s）
- 说明：Step 1~4 已 PASS；本次只推 Step 5（直线跟随），禁止改 executor/nav/协议解码

## Step 5 — sim 直线跟随（人往前走，飞机往前飞）（Step 5（sim 直线跟随））
- 起止：2026-10-28 13:35:45Z → 2026-10-28 13:51:22Z
- 范围：sim：Mission::FollowStraight + 控制口 + uav_sim 直线跟随树；真机平凡树
- 轮次：1 / 3
- 结果：PASS
- 规划要点：**计划摘要（供 workbook）** plan.md 已写（964 行，未改任何源码）。`FollowStraight` 加入 base，并为 4 处 `match` 补齐穷尽分支；sim 机栈新建 `follow_straight.rs` 单叶：复用 Step 4 的 `TargetSource`，`decide` 按三档判定（d>4 前进、d≤2.5 停、带内沿用上拍），`front_cell = world_to_grid(me+0.5·dir)`，全程不碰 nav，因此 `delta≈0`，只发 MoveForward；真机与两个车栈建平凡 Success 树。控制口新增 `follow_straight`，并同步 3 处已有测试（help 11→12、未知 cmd 文本）。规划期已用桩 crate 实测：9/9 测试通过，build 分支的强制类型转换能通过编译。验收 V1~V14，包含范围与只读红线 diff 判据。 [plan.md](tmp/task37/plan.md)

## 引擎结束
- 结束时间：2026-10-28 13:51:22Z
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M Architecture/robot_arch.md
 M Task/task_37_detector.md
 M Tool/execution_engine_task37.config.json
 M Tool/execution_engine_task37.ts
 M Workbook/wb_37_detector.md
 M pleiades-base/src/Robot/core/mission_queue/element.rs
 M pleiades-sim/src/sim/control.rs
 M pleiades-sim/src/sim/emergency_stop.rs
 M pleiades-sim/src/sim/sensors/lidar/mapping_task.rs
 M pleiades-sim/src/sim/uav_sim/mission/mod.rs
 M pleiades-sim/src/sim/uav_sim/mod.rs
 M pleiades-sim/src/sim/ugv_sim/mission/mod.rs
 M pleiades-sim/src/world_link/config.rs
 M pleiades-sim/src/world_link/parts.rs
 M pleiades-sim/src/world_link/protocol.rs
 M pleiades-uav/src/uav/mission/mod.rs
 M pleiades-ugv/src/ugv/mission/mod.rs
?? Task/task_38_sim_world_refine.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? pleiades-sim/src/sim/uav_sim/mission/follow_straight.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```
