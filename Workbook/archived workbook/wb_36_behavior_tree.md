# wb_36_behavior_tree — task_36 工作记录

> Presented by KeJi
> Created Date ： 2026-10-27
> Modified Date ： 2026-10-27
> 任务：Task/task_36_behavior_tree.md（任务层改造：mission queue 重构 + 行为树）

## 开工前状态（2026-10-27）

- **设计讨论已完成**：结论见 task 文档 §三（#1~#10）+ §十一（#11~#38），冲突以 §十一 为准
- **实施计划**：§十二 Step 1~8 全部定稿
- **关键决策速查**：
  - 一任务一树；树结构与叶子在设备端，引擎在 base（#11/#19）
  - 引擎非泛型 + async + 失败码；`Frame{dt_ms, tick_no, out}`；无 Obs / 无黑板 / 无兜底时限（#20~#24/#31~#33）
  - 队列三语义 preempt/append/replace；执行中不离队；抢占从暂停点恢复（#15~#17）
  - 协同上移指挥层；协议去 `member_count`/`members`；`mission_count==0` 即取消（#27/#28）
  - 算路在叶子里；驱动只读 `Frame.out.next_cell`（#25/#32）
  - `reset` 与 `suspend` 分开（#38）
- **依赖**：
  - base：`mission_queue` / `behavior_tree` / 协议 / `main_loop`
  - 设备端：ugv（Step 4）、uav（Step 5）、sim（Step 6）
  - Pictor：已 clone 到 `Pictor/`（`godot-rust` 分支），已被 `.gitignore` 排除
- **环境注意**：`Pictor/bin/` 需自行放 `libpleiades_terminal.so`（由 `cargo build -p pleiades-terminal` 产出）

## 步骤一览（开工前计划，执行结果见下方分节）

| Step | 内容 | 状态 |
|---|---|---|
| 1 | mission_queue（base） | 待执行 |
| 2 | behavior_tree 引擎（base） | 待执行 |
| 3 | 协议升版 + 全仓删协同 | 待执行 |
| 4 | ugv 迁移 + base 接线 | 待执行 |
| 5 | uav 迁移（同构） | 待执行 |
| 6 | sim 迁移（ugv_sim / uav_sim 双栈） | 待执行 |
| 7 | Pictor（协议同步 + 逐车下发） | 待执行 |
| 8 | sim 端到端验证（队列语义 + 双栈） | 待执行 |

## 执行引擎启动
- 开始时间：2026-10-27 04:53:58Z
- 引擎：Tool/execution_engine_task36.ts + Tool/execution_engine_task36.config.json
- 计划：Step 1~8 串行，每 step 最多 3 轮（规划 → 实现 → 评审）
- 范围：base（队列 / 引擎 / 协议 / 接线）→ ugv → uav → sim（双栈）→ Pictor → sim 端到端验证

## Step 1 — mission queue 重构
- 起止：2026-10-27 04:53:58Z → 2026-10-27 05:30:20Z
- 范围：pleiades-base（mission_queue）+ 4 处 import
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划写入 [plan.md](tmp/task36/plan.md)：新建 mission_queue/ 三文件（Mission 逐字搬、TaskId/QueuedTask、八方法、10 单测已验证全绿）；删 mission.rs；改 8 处 import；三处 goal.rs 的 pop_next 补 .map(|t|t.mission)（已登记）。验收 check --workspace + test --lib。 补充两点供实现/评审取用：三份新文件已用 `rustc --test` 独立编译跑过 10/10 通过（产物在 `tmp/task36/verify/`，已被 .gitignore 忽略）；基线 `cargo test -p pleiades-base`（整包）本就因 `tests/t01_storage_integration.rs` 编译失败而红、`--lib` 也有 2 个既有 `vm::engine` 失败，故判据一律用 `--lib` + 过滤串（期望 10/12/174 passed）。

## Step 2 — behavior_tree 引擎
- 起止：2026-10-27 05:30:20Z → 2026-10-27 06:05:57Z
- 范围：pleiades-base（behavior_tree）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写好并完成预验证。 **摘要（≤200 字）**：计划见 [plan.md](tmp/task36/plan.md)。新建 `behavior_tree/` 七文件，构成非泛型 async 引擎，含三值+失败码、4 个组合节点、6 个装饰节点、3 个叶节点；`core/mod.rs` 加一行，`robot_arch.md` 加 7 行目录树；16 个单测分布为 6/5/3/2。最终代码草案已在 [verify_bt](tmp/task36/verify_bt/behavior_tree) 中预验证：`rustc --test` 编译 0 warning、16/16 全绿、禁用词 grep 干净；实现阶段逐字复制即可。判据：`check --lib`/`--workspace` 绿，`test --lib behavior_tree` 16 passed，整包 `--lib` 190 passed/2 个既有失败。阈值按 §12.2.6 原文：Timeout 用 `>=`、Wait 用 `>`；注释禁写禁用词以免自触发 grep。首轮 `prev_issues` 为无。

## Step 3 — 协议升版 + 全仓删协同
- 起止：2026-10-27 06:05:57Z → 2026-10-27 07:08:59Z
- 范围：base 协议 + 三个设备 crate 的编译级修复
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task36/plan.md`（722 行，11 节，含逐字代码、12 个单测、文档新旧文本、验收命令与判据）。 三个关键发现（都已写进计划）： 1. **范围冲突**：`queue.rs:93` 与 `sim/emergency_stop.rs:219` 在 step.md 路径清单外，但 `Mission` 去 `members`、`GoalService::new` 去 `own_peer_id` 必然波及，各只需删 1 处字面量，无替代方案 —— 需评审批准为范围例外。 2. **基线实测**：`cargo check --workspace` 真绿；`cargo test --workspace` **基线即红**（base/tests 下 5 个既有集成测试编译失败），故判据改为 `--lib` + 分包：base 190 passed/2 既有失败、ugv 87 / uav 93 / sim 262 → 目标 69 / 75 / 244（各 −18 为 assignment 自带用例）。 3. **grep 判据坑**：`assignment` 会误命中 `TUI/mod.rs` 局部变量；`members` 判据与 control.rs 测试冲突 —— 已分别收窄判据、改键名为 `legacy`。 摘要（≤200 字，可抄进 workbook）： > 计划见 tmp/task36/plan.md。⚠范围例外：queue.rs:93 与 sim/emergency_stop.rs:219 在清单外但必被波及（各删1处字面量），请批准。协议 count(1)+9n 严格长度，count==0 才取消、全未知→None 不动队列；12 单测(5+7)。删 3×assignment.rs、own_peer_id、两处失败分支；control.rs 保留 circle 名只造 Goto。基线 check 绿、base --lib 190/2 既有；ugv/uav/sim 87/93/262→69/75/244；整包 test 基线即红，判据用

## Step 4 — ugv 迁移 + base 接线
- 起止：2026-10-27 07:08:59Z → 2026-10-27 08:53:38Z
- 范围：base（trait/main_loop）+ pleiades-ugv + uav/sim stub
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task36/plan.md)（1587 行，12 节：改动清单、逐字签名与代码、file:line 衔接点、9 步实施顺序、风险、验收命令与判据、上一轮 issues=无）。 **已核对的事实**：`cargo test -p pleiades-ugv` 基线 69 passed；`cargo test -p pleiades-base --lib` 基线 190 passed、2 failed（`vm::engine`，既有）；`--workspace test` 基线即红，故判据用分包。 **需评审批准 3 项**：① 删 `planning/` 必波及 `ugv/slam/task.rs:18`（1 行 import）；② `Navigator::step` 改 `async`（要拿 grid 读锁，同步只能 `try_read` 会假无路并打乱 20 拍去抖）；③ 新增 `NavPort`（§12.4.9 #6~#8 的「假端口」测树没有它无法实现）。另登记 D6：建图增量走 `mark_obstacle` 而非 sim 独有的 `Notify_Changed`，以保三份副本逐字相同。 ```text tmp/task36/plan.md。base 接 TickOutcome/build_task/auto_tick；ugv 建 nav/ 与 mission/（Goto 树），删 goal.rs、planning/；uav/sim 仅 stub。测试+14，ugv 69→80、base 190→193。待批：slam 一行 import、step 改 async、加 NavPort（树单测）。 ``` 未改动任何源码（`git status` 里的改动全部来自 Step 1~3）；未起进程、未 git add。

## Step 5 — uav 迁移（与 ugv 同构）
- 起止：2026-10-27 08:53:38Z → 2026-10-27 09:27:32Z
- 范围：pleiades-uav
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task36/plan.md`（818 行、10 节），本次未改动任何源码（`git status` 与 Step 4 结束时一致，也没有执行 git add/commit 或启动长驻进程）。 **摘要（≤200 字）** 计划见 [plan.md](tmp/task36/plan.md)。nav/ 下四个文件与 mission/ 下两个文件从 ugv 逐字节复制（mission 各仅改 1 行 `use`）；robot_handler.rs 整文件重写为四步驱动（不执行第②步急停）；删 goal.rs 与 planning/；机端差异全走参数（`ObstacleMode::Ignore`、`map_rx=None`）。uav/mod.rs 加一行模块级 `#[allow(dead_code, unused_imports)] pub mod nav;`，让四个 nav 文件逐字相同，warning 由 14 降到 8。已在 `tmp/task36/verify_uav` 预验证：check 通过、test 83+2（副本缺 `camera_photo.lua`），真仓库预期 85（基线 75）。基线：ugv 80、base 193/2 既有红。待批：该模块级 allow。

## Step 6 — sim 迁移（ugv_sim / uav_sim 双栈）
- 起止：2026-10-27 09:27:32Z → 2026-10-27 10:45:47Z
- 范围：pleiades-sim
- 轮次：2 / 3
- 结果：PASS
- 规划要点：计划已完成，并通过只读勘察与副本预验证。`git status` 前后一致（60 条、sim 10 条），未改任何源码。 ## 计划摘要（≤200 字，可抄进 workbook） 计划见 [plan.md](tmp/task36/plan.md)。本轮仅 1 处源码改动：`uav_sim/mod.rs` 测试里加 2 条 `instance_count` 断言（同 TaskId 两次 build 仍 1 实例），既消 dead_code 又补 §12.6.7 #4 机栈覆盖；不加 `#[allow]`。已在 `tmp/task36/verify_sim6` 副本预验证：告警 2→1（仅 origin）、315 测试全绿。判据 V1~V9（cmp 四 nav 文件 SAME、mission 只差 1 行 use、git 越界自查 10 行逐行一致），impl.md §1 告警台账按实测改写。 ## 关键事实（勘察实测） | 项 | 实测 | |---|---| | issue 成立 | `cargo check -p pleiades-sim --tests` → `origin:188` + `uav_sim/mod.rs:136 instance_count` = 2 warnings | | 修法可行 | 副本打补丁后 → 1 warning（仅 `origin`）、`315 passed; 0 failed` | | 基线真相 | Step 6 删 `pathfinder.rs` 时已消掉 `planning/mod.rs` 的 `DStarLite` 告警，故「新增 1 条」无对冲，必须真消 | | 不可动项已锁定 | 两栈 `nav/` 四文件 `cmp` 全 SAME；`mission/` 与真机只差 1 行 `use`（`goto.rs:28`／`mod.rs:17`）；ugv 80 / uav 85 不受影响 |

## Step 7 — Pictor 协议同步 + 逐车下发
- 起止：2026-10-27 10:45:47Z → 2026-10-27 11:57:15Z
- 范围：Pictor/（Godot GDScript，仓外 clone）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task36/plan.md`（883 行）；本轮未改动任何源码（`git -C Pictor status` 仍为空），只新增了只读核对脚本 `tmp/task36/verify_pictor/check_layout.py`（21 条字节级核对断言全部通过）。 > 计划见 tmp/task36/plan.md。载荷 1+9n、取消帧 1B；Encode 改为单参数，且 type≠0 时降级为 Goto；Decode 严格校验长度并去掉成员键；删 CIRCLE 常量与 builder，kernel_bridge 去掉自动填充名单；新增 target_alloc.gd（门控+占位算点），auto_handler 改为逐车一帧；测试与 docs §3.5 同步。三项关键注意事项已处理；待批 S1~S5 （摘要 199 字。待批例外：S1 删 `MISSION_TYPE_CIRCLE`/`build_auto_push_circle`、S2 Encode 类型降级、S3 修正两处 yaw 文案、S4 KernelBridge 多 target 告警、S5 新增 `_test_target_alloc`。）

## Step 8 — sim 端到端验证（队列语义 + 双栈）
- 起止：2026-10-27 11:57:15Z → 2026-10-27 15:42:44Z
- 范围：base 命令入口 + sim 控制口/status + 测试脚本
- 轮次：3 / 3
- 结果：FAIL（已中断）
- 规划要点：计划已写入 [plan.md](tmp/task36/plan.md)（599 行，10 节）。本轮未改任何源码（`git status` 与上一轮结束时逐行一致），未起进程、未 git add。 **计划摘要（≤200 字，可抄进 workbook）** 复核确认 §12.8.2 四条前置改动（`AutoCmd::Append/Preempt`、`DeviceHandler::suspend`、控制口 `append/preempt/queue`、`status` 四键）已全部落地（含 file:line），故**本轮不改 Rust**，只硬化 `Tool/sim_queue_test.py`：用例 4 的"≤1 tick 停车"改为换头后位移上界＝1 tick＋惯性＋位姿量化（位姿 10 Hz 量不出 50 ms，登记为 D3）；用例 5/6 的 ticks 偏差 60→10 并加上界；新增 `--pair` 双栈聚合出 10 行终表（用例 10 不再空采样）。之后 ugv→uav→`--pair` 重跑取证，产物落 `RobotSim_TestEnv/results/`。基线写死：base `--lib` 197/2 既有红、sim 328、ugv 80、uav 85。
- 3 轮未通过的问题：
评审子 agent 失败：Error: dispatchsubagent: subagent run ended with error

## 引擎结束
- 结束时间：2026-10-27 15:42:44Z
- 状态：ABORTED — Step 8（sim 端到端验证（队列语义 + 双栈））3 轮未通过
- git status（结束时）：
```
 M .gitignore
 M Architecture/Pleiades_Architecture.md
 M Architecture/robot_arch.md
 M Architecture/uav_arch.md
 M docs/design_doc/orion_protocol.md
 M docs/design_doc/robot_controller.md
 M pleiades-base/src/Robot/core/command.rs
 D pleiades-base/src/Robot/core/mission.rs
 M pleiades-base/src/Robot/core/mod.rs
 M pleiades-base/src/Robot/core/protocol/command_decode.rs
 M pleiades-base/src/Robot/core/protocol/messages.rs
 M pleiades-base/src/Robot/core/protocol/mod.rs
 M pleiades-base/src/Robot/core/robot.rs
 M pleiades-base/src/Robot/mod.rs
 M pleiades-sim/src/sim/control.rs
 M pleiades-sim/src/sim/emergency_stop.rs
 D pleiades-sim/src/sim/goal.rs
 M pleiades-sim/src/sim/mod.rs
 D pleiades-sim/src/sim/planning/assignment.rs
 M pleiades-sim/src/sim/planning/mod.rs
 D pleiades-sim/src/sim/planning/pathfinder.rs
 M pleiades-sim/src/sim/sim_handler.rs
 D pleiades-uav/src/uav/goal.rs
 M pleiades-uav/src/uav/mod.rs
 D pleiades-uav/src/uav/planning/assignment.rs
 D pleiades-uav/src/uav/planning/cluster_obstacles.rs
 D pleiades-uav/src/uav/planning/mod.rs
 D pleiades-uav/src/uav/planning/pathfinder.rs
 M pleiades-uav/src/uav/robot_handler.rs
 M pleiades-ugv/src/ugv/emergency_stop.rs
 D pleiades-ugv/src/ugv/goal.rs
 M pleiades-ugv/src/ugv/mod.rs
 D pleiades-ugv/src/ugv/planning/assignment.rs
 D pleiades-ugv/src/ugv/planning/cluster_obstacles.rs
 D pleiades-ugv/src/ugv/planning/mod.rs
 D pleiades-ugv/src/ugv/planning/pathfinder.rs
 M pleiades-ugv/src/ugv/robot_handler.rs
 M pleiades-ugv/src/ugv/slam/task.rs
?? Task/task_36_behavior_tree.md
?? Task/task_37_detector.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task36.config.json
?? Tool/execution_engine_task36.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Tool/sim_queue_test.py
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_36_behavior_tree.md
?? pleiades-base/src/Robot/core/behavior_tree/composite.rs
?? pleiades-base/src/Robot/core/behavior_tree/decorator.rs
?? pleiades-base/src/Robot/core/behavior_tree/frame.rs
?? pleiades-base/src/Robot/core/behavior_tree/leaf.rs
?? pleiades-base/src/Robot/core/behavior_tree/mod.rs
?? pleiades-base/src/Robot/core/behavior_tree/node.rs
?? pleiades-base/src/Robot/core/behavior_tree/status.rs
?? pleiades-base/src/Robot/core/mission_queue/element.rs
?? pleiades-base/src/Robot/core/mission_queue/mod.rs
?? pleiades-base/src/Robot/core/mission_queue/queue.rs
?? pleiades-sim/src/sim/uav_sim/mission/goto.rs
?? pleiades-sim/src/sim/uav_sim/mission/mod.rs
?? pleiades-sim/src/sim/uav_sim/mod.rs
?? pleiades-sim/src/sim/uav_sim/nav/cluster_obstacles.rs
?? pleiades-sim/src/sim/uav_sim/nav/mod.rs
?? pleiades-sim/src/sim/uav_sim/nav/navigator.rs
?? pleiades-sim/src/sim/uav_sim/nav/pathfinder.rs
?? pleiades-sim/src/sim/ugv_sim/mission/goto.rs
?? pleiades-sim/src/sim/ugv_sim/mission/mod.rs
?? pleiades-sim/src/sim/ugv_sim/mod.rs
?? pleiades-sim/src/sim/ugv_sim/nav/cluster_obstacles.rs
?? pleiades-sim/src/sim/ugv_sim/nav/mod.rs
?? pleiades-sim/src/sim/ugv_sim/nav/navigator.rs
?? pleiades-sim/src/sim/ugv_sim/nav/pathfinder.rs
?? pleiades-uav/src/uav/mission/goto.rs
?? pleiades-uav/src/uav/mission/mod.rs
?? pleiades-uav/src/uav/nav/cluster_obstacles.rs
?? pleiades-uav/src/uav/nav/mod.rs
?? pleiades-uav/src/uav/nav/navigator.rs
?? pleiades-uav/src/uav/nav/pathfinder.rs
?? pleiades-ugv/src/ugv/mission/goto.rs
?? pleiades-ugv/src/ugv/mission/mod.rs
?? pleiades-ugv/src/ugv/nav/cluster_obstacles.rs
?? pleiades-ugv/src/ugv/nav/mod.rs
?? pleiades-ugv/src/ugv/nav/navigator.rs
?? pleiades-ugv/src/ugv/nav/pathfinder.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```

## 执行引擎启动（重跑 Step 8（第 2 次））
- 开始时间：2026-10-28 00:40:25Z
- 引擎：Tool/execution_engine_task36.ts + Tool/execution_engine_task36.config.json
- 运行标记：重跑 Step 8（第 2 次）（本次从 Step 8 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次

## Step 8 — sim 端到端验证（队列语义 + 双栈）（重跑 Step 8（第 2 次））
- 起止：2026-10-28 00:40:25Z → 2026-10-28 02:26:35Z
- 范围：base 命令入口 + sim 控制口/status + 测试脚本
- 轮次：2 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task36/plan.md)，共 438 行：复核表、改动清单、逐字代码、file:line 衔接点、S0–S13 实施顺序、风险、验收判据、预验证。 计划摘要（≤200 字）： > 计划见 tmp/task36/plan.md。复核确认 §12.8.2 四条前置改动全落地（附 file:line），Rust 零改动，只硬化 sim_queue_test.py：加日志 µs 时序判据（用例2 设备零动作、用例4 preempt→suspend≤50ms、用例7/8→reset≤50ms）、用例9 失败 ticks≥1100 证 Timeout 真走完；cases.json 加构建指纹，--pair 六项校验出 10 行终表（旧产物实测 FAIL）。helper/pair 已预验证（Δ=0.13/0.12/0.10ms），再双栈重跑取证。 补充给评审的两点：① `prev_issues.md` 只有“规划子 agent 报错”一行，非实质意见，§7 已给对账表与处理方式；② 预验证脚本在 `tmp/task36/verify_r2/`（`probe.py`、`probe_pair.py`），证明新判据在旧产物上会 FAIL、新产物才会 PASS。

## 引擎结束（重跑 Step 8（第 2 次））
- 结束时间：2026-10-28 02:26:35Z
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M .gitignore
 M Architecture/Pleiades_Architecture.md
 M Architecture/robot_arch.md
 M Architecture/uav_arch.md
 M docs/design_doc/orion_protocol.md
 M docs/design_doc/robot_controller.md
 M pleiades-base/src/Robot/core/command.rs
 D pleiades-base/src/Robot/core/mission.rs
 M pleiades-base/src/Robot/core/mod.rs
 M pleiades-base/src/Robot/core/protocol/command_decode.rs
 M pleiades-base/src/Robot/core/protocol/messages.rs
 M pleiades-base/src/Robot/core/protocol/mod.rs
 M pleiades-base/src/Robot/core/robot.rs
 M pleiades-base/src/Robot/mod.rs
 M pleiades-sim/src/sim/control.rs
 M pleiades-sim/src/sim/emergency_stop.rs
 D pleiades-sim/src/sim/goal.rs
 M pleiades-sim/src/sim/mod.rs
 D pleiades-sim/src/sim/planning/assignment.rs
 M pleiades-sim/src/sim/planning/mod.rs
 D pleiades-sim/src/sim/planning/pathfinder.rs
 M pleiades-sim/src/sim/sim_handler.rs
 D pleiades-uav/src/uav/goal.rs
 M pleiades-uav/src/uav/mod.rs
 D pleiades-uav/src/uav/planning/assignment.rs
 D pleiades-uav/src/uav/planning/cluster_obstacles.rs
 D pleiades-uav/src/uav/planning/mod.rs
 D pleiades-uav/src/uav/planning/pathfinder.rs
 M pleiades-uav/src/uav/robot_handler.rs
 M pleiades-ugv/src/ugv/emergency_stop.rs
 D pleiades-ugv/src/ugv/goal.rs
 M pleiades-ugv/src/ugv/mod.rs
 D pleiades-ugv/src/ugv/planning/assignment.rs
 D pleiades-ugv/src/ugv/planning/cluster_obstacles.rs
 D pleiades-ugv/src/ugv/planning/mod.rs
 D pleiades-ugv/src/ugv/planning/pathfinder.rs
 M pleiades-ugv/src/ugv/robot_handler.rs
 M pleiades-ugv/src/ugv/slam/task.rs
?? Task/task_36_behavior_tree.md
?? Task/task_37_detector.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task36.config.json
?? Tool/execution_engine_task36.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Tool/sim_queue_test.py
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_36_behavior_tree.md
?? pleiades-base/src/Robot/core/behavior_tree/composite.rs
?? pleiades-base/src/Robot/core/behavior_tree/decorator.rs
?? pleiades-base/src/Robot/core/behavior_tree/frame.rs
?? pleiades-base/src/Robot/core/behavior_tree/leaf.rs
?? pleiades-base/src/Robot/core/behavior_tree/mod.rs
?? pleiades-base/src/Robot/core/behavior_tree/node.rs
?? pleiades-base/src/Robot/core/behavior_tree/status.rs
?? pleiades-base/src/Robot/core/mission_queue/element.rs
?? pleiades-base/src/Robot/core/mission_queue/mod.rs
?? pleiades-base/src/Robot/core/mission_queue/queue.rs
?? pleiades-sim/src/sim/uav_sim/mission/goto.rs
?? pleiades-sim/src/sim/uav_sim/mission/mod.rs
?? pleiades-sim/src/sim/uav_sim/mod.rs
?? pleiades-sim/src/sim/uav_sim/nav/cluster_obstacles.rs
?? pleiades-sim/src/sim/uav_sim/nav/mod.rs
?? pleiades-sim/src/sim/uav_sim/nav/navigator.rs
?? pleiades-sim/src/sim/uav_sim/nav/pathfinder.rs
?? pleiades-sim/src/sim/ugv_sim/mission/goto.rs
?? pleiades-sim/src/sim/ugv_sim/mission/mod.rs
?? pleiades-sim/src/sim/ugv_sim/mod.rs
?? pleiades-sim/src/sim/ugv_sim/nav/cluster_obstacles.rs
?? pleiades-sim/src/sim/ugv_sim/nav/mod.rs
?? pleiades-sim/src/sim/ugv_sim/nav/navigator.rs
?? pleiades-sim/src/sim/ugv_sim/nav/pathfinder.rs
?? pleiades-uav/src/uav/mission/goto.rs
?? pleiades-uav/src/uav/mission/mod.rs
?? pleiades-uav/src/uav/nav/cluster_obstacles.rs
?? pleiades-uav/src/uav/nav/mod.rs
?? pleiades-uav/src/uav/nav/navigator.rs
?? pleiades-uav/src/uav/nav/pathfinder.rs
?? pleiades-ugv/src/ugv/mission/goto.rs
?? pleiades-ugv/src/ugv/mission/mod.rs
?? pleiades-ugv/src/ugv/nav/cluster_obstacles.rs
?? pleiades-ugv/src/ugv/nav/mod.rs
?? pleiades-ugv/src/ugv/nav/navigator.rs
?? pleiades-ugv/src/ugv/nav/pathfinder.rs
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```
