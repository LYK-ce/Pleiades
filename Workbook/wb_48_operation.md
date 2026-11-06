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

## Step 3 — 车栈距离参数分家（Containment 收紧 / Follow 放宽 / Guard 不变）（task_48 首轮）
- 起止：2026-11-05 11:40:05Z → 2026-11-05 11:53:08Z
- 范围：两份 follow.rs 的常量 + 两份 containment.rs 与两份 mission/mod.rs 的传参
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 5615feb
- 规划要点：计划已写完（`tmp/task48/plan.md`，460 行，源码零改动，`git status` 仍只有 workbook 与既有未跟踪目录）。 **摘要（≤200 字）**：6 个文件：两份 `follow.rs` 把 `D_FAR/D_NEAR` 改 2.4/1.4、新增 `CONTAINMENT_D_*` 2.0/1.0，同步 6 处注释（含 `1.7→1.9`）；两份 `containment.rs` 的 `params` 显式传 `CONTAINMENT_*`；两份 `mod.rs` 的 `Mission::Follow` 显式传 `D_*`（不依赖 `default()`）。`guard.rs` 2.2/1.2、机端 3.6/2.6、base、`tests/`、Task/、根 Cargo.toml 一行不动。两份副本用同一脚本套改，改后逐字一致（diff hunk 数 follow 2 / containment 4）；各加 1 条鉴别力单测（同一距离 2.1/1.3 m 两种口径结论相反，迟滞带用 epsilon 防 f32 假红）。验收：`check --lib` + 两个设备 crate `--all-targets`；`--bins` 测试 13→14 / 17→18 / 31→32 / 33→34；`grep` 常量 8 行对拍；V6 卡死 `FollowParams::default()` 生产残留；不 fmt、不 git add/commit。

## Step 4 — Pictor：类型 / 长度表 / 编解码 / 下发 / X→Follow（task_48 首轮）
- 起止：2026-11-05 11:53:08Z → 2026-11-05 12:10:13Z
- 范围：Pictor 的协议层、两个下发点、ws 命令门控、X 键入口
- 轮次：1 / 3
- 结果：PASS
- 提交：ok ff532f7
- 规划要点：计划已写入 [tmp/task48/plan.md](tmp/task48/plan.md)（451 行，源码零改动，Pictor 仓仍干净）。 **计划摘要（≤200 字）** 10 个文件、全部锚到 `文件:行号`：`protocol_def.gd`(常量+归一化+修过期注释)、`orion_messages.gd`(长度表/encode 17B 分支/decode fx,fy)、`message_builder.gd`(两个新 builder)、`coop_controller.gd:218` 与 `auto_handler.gd:179-180` 改成对 `[Goto,Face]`、`event_bus.gd` 加 `follow_requested`、`ws_command_server.gd`(门控加 4/5 判据 + `_Handle_Guard`)、`input_handler.gd`(焦点门控 + X 随访状态机)、两份单测(4 条金标同 Rust 串 + fx/fy 透传 + 长度拒)。 两处校正：①`build_task_set` 必须透传 `fx/fy`（step.md 未点名，漏了 type=5 注视点恒 0）；②`protocol_def.gd` 里「Containment 只有 sim 机栈真实现」已过期，随本条注释一并纠正。 禁起 Godot → 验收改为 `tmp/task48/verify_step4.py` 独立复算 4 条金标 + TAB/偏移/口径 grep 对拍 + `git -C Pictor status` 范围自查，并把 `godot --headless …` 原命令留 Step 7 人类补跑。

## Step 5 — sim 控制口：face / guard 命令（含 help 11→13）（task_48 首轮）
- 起止：2026-11-05 12:10:13Z → 2026-11-05 12:22:41Z
- 范围：pleiades-sim 的控制口（control.rs）
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 18f90de
- 规划要点：计划已写入 [tmp/task48/plan.md](tmp/task48/plan.md)（共 466 行）。 plan.md 共 466 行，本步只改 `sim/control.rs` 的 14 处：`ControlCmd` 加 `Face`、`Guard`；`Parse_Control` 加两个分支（复用 `Param_F32`，先做校验再判 `dx==dy==0` 是否重合）；`Handle_Cmd` 加两个 arm；`help` 从 11 条增至 13 条，三处顺序一致。测试新增 4 条、追加断言 1 处：`--bins control` 46→50、全量 495→499。一处校正：单测注视点用 120 而非金标 128（越界）。

## Step 6 — Python 工具 + 文档（task_48 首轮）
- 起止：2026-11-05 12:22:41Z → 2026-11-05 12:38:27Z
- 范围：第三处长度表（Pictor/tool/pictor_ws_ctl.py）+ 架构文档 + 协议文档
- 轮次：1 / 3
- 结果：PASS
- 提交：ok f1bf780
- 规划要点：计划已写入 [tmp/task48/plan.md](tmp/task48/plan.md)（832 行，`git status` 仍只有既有 `M Workbook/…` 与 `?? Tool/investigation/`，源码零改动）。 **摘要（≤200 字）**：4 个文件、全部锚 `文件:行号`：`pictor_ws_ctl.py`（常量+长度表 4:9/5:17+归一化+金标+**payload 复刻加 17 B 分支**+`cmd_guard` 走 `{cmd:"guard",targets}`+argparse+pose_hz）、副本 `cp` 保持逐字、`robot_arch.md`（头版本/§1.2 五变体/§8.12 type4-5 与金标/§8.15 车端 2.4-1.4 + 三组距离注/新增 §8.16 全文/常量速查）、`orion_protocol.md §3.5`（补 3/4/5、变长、Σlen、整帧拒）。四处校正：§8.15 写真实 Follow `2.4/1.4`（`2.0/1.0` 是 Containment，口径 #19 优先）；§3.5 现缺 type 3 须一并修；Guard 参数是 `targets` 数组；step.md 那条 grep 实测只命中 Rust，判据改为 A1/A6/A7 数值复算 + 三条按文件 grep。不加 `face` 子命令（`follow --type 4` 覆盖）。验收 12 条 V 判据 + `tmp/task48/verify_step6.py`（A1~A10）。

## Step 7 — 仿真验证：Face / Guard / Containment×Face（三 agent 自动跑）（task_48 首轮）
- 起止：2026-11-05 12:38:27Z → 2026-11-05 12:52:31Z
- 范围：只写 tmp/task48/s7/ 的日志与结论；**不改任何源码**
- 轮次：1 / 3
- 结果：PASS
- 提交：ok d65f4ae
- 规划要点：计划已写完并落在 `tmp/task48/plan.md`（307 行，源码零改动，`git status` 仍只有既有 workbook 与 `Tool/investigation/`）。 **摘要（≤200 字，可直接写进 workbook）** > Step 7 计划已写 `tmp/task48/plan.md`（307 行，源码零改动）。只加 `tmp/task48/s7/`：根 `cargo build --release` + CUDA 三层证据；夹具全落 `s7/env/`（世界与 Pictor 镜像副本、`TMPDIR` 迁移，`/` 仅剩 110 MB）；驱动照 task46 `step4_driver.py` 精简移植；10 案坐标全在无障碍矩形内。三处校正落为偏差：控制口 `missions` 只支持 Goto（F1/F4 两段式）、源码无「进区/出区」日志（用行为证据、不伪造）、世界须 xvfb。配 D1~D5 / R1~R12 / V1~V20。⚠️需裁决：`step.md:52` 允许起进程 vs 约束「禁止起 sim」，默认硬跑，保留禁令则按 ABORT 全记 FAIL。 计划关键点（供 review 一眼看）： | 项 | 结论 | |---|---| | 环境实测 | godot 4.7.2 / xvfb / nvcc 均在；`target/release/pleiades-sim` 已链接 `libcudart`；5 端口全空 | | 冲突 | 约束里那句「禁止起 sim」出自 `step.md:39`（自带「Step 1~6」限定），与 `step.md:52`「本 step 允许起长驻进程」矛盾；task_46 Step 4 就是引擎在本机真跑的 | | 不允许的改动 | 无源码、无 Task/、无根 Cargo.toml、无 fmt、无 git 操作；夹具也不写 `RobotSim_TestEnv/` | | 三处必须记录的偏差 | D1 控制口只支持 Goto、D2 无「进区/出区」日志、D3 headless 无相机帧 |

## 引擎结束
- 结束：2026-11-05 12:52:34Z
- 状态：ALL_STEPS_DONE
- git status：
```
M Workbook/wb_48_operation.md
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
```

## Step 7 重跑说明（主 agent，2026-11-05）
- 第一轮 Step 7 **ABORT**：实现提示词里的「禁止起 sim」与 step.md「本 step 允许起长驻进程」冲突，子 agent 选了更严的一边；10 个 case 全部 `FAIL — 未跑`，**源码零改动**。
- 已修：提示词改为「是否允许起长驻进程以 step.md 为准」；engine `from_step = 7` 只重跑本步。
- 上一次审计到的环境缺口（本轮必须处理）：release 二进制早于新代码、uav 配置端口 9099/cuda:1、Pictor 根配置缺 subscribe_topics、世界需 xvfb。

## 执行引擎启动（task_48 首轮·step7 重跑）
- 开始：2026-11-05 12:53:19Z
- 计划：S7

## Step 7 — 仿真验证：Face / Guard / Containment×Face（三 agent 自动跑）（task_48 首轮·step7 重跑）
- 起止：2026-11-05 12:53:19Z → 2026-11-05 14:08:50Z
- 范围：只写 tmp/task48/s7/ 的日志与结论；**不改任何源码**
- 轮次：1 / 3
- 结果：PASS
- 提交：ok c397e4f
- 规划要点：计划已经写入 [plan.md](tmp/task48/plan.md)（共 523 行；本次仅写入这一个文件，`git status` 仍只显示进场基线。） **摘要（200 字内，可直接写进 workbook）** > 计划写入 tmp/task48/plan.md，源码零改动。先做全工作区 cargo build --release（无 -p），再启动世界、两个 sim 实例、行人和 Pictor，依次运行 F1~F4、G1~G4、C1、C2，按 V1~V22 判断。唯一不在 tmp 下的改动：在 Pictor/.config 中增加 subscribe_topics。已登记三处偏差：F1/F4 改为两段注入，因为控制口只能装 Goto，单帧成对下发由 C1/C2 覆盖；进区/出区日志不存在，改用行为证据；机端 assist 无成功日志。/ 仅剩 110MB，TMPDIR 移到 s7/env。 要点（供 review 快速查看）： | 项 | 结论 | |---|---| | 进场实测 | HEAD d65f4ae；release 二进制陈旧（11-04 02:02，源码 11-05 11:36）；现有二进制已是 CUDA（cudart）；`cargo tree … \| grep -c cuda` = 1；5 端口全空 | | 落盘边界 | 源码零改动；只新建 `tmp/task48/s7/`；唯一非 tmp 改动是 `Pictor/.config/config.toml:19` 一行（两处 `.gitignore` 已覆盖，git 不可见） | | 关键源码锚点 | `control.rs:273/278/681/689`、`robot.rs:463/486`、`sim_handler.rs:831/858`、`detector/mod.rs:606`、`guard.rs:53-63/111/175/225/247`、`face.rs:37-39/63-90`、`coop_controller.gd:209/216/218-221`、`message_buil

## 引擎结束（task_48 首轮·step7 重跑）
- 结束：2026-11-05 14:08:52Z
- 状态：ALL_STEPS_DONE
- git status：
```
M Workbook/wb_48_operation.md
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
```

## 执行引擎启动（task_48 首轮·step8-9）
- 开始：2026-11-06 00:38:34Z
- 计划：S8 → S9

## Step 8 — 去掉 Face 的 60 拍时限（四份 face.rs）（task_48 首轮·step8-9）
- 起止：2026-11-06 00:38:34Z → 2026-11-06 00:49:50Z
- 范围：四份 face.rs（ugv / uav / sim_ugv / sim_uav）
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 8920892
- 规划要点：计划 tmp/task48/plan.md + 快照 step8_face_rs_final.rs，源码零改。四份 face.rs 删 :38-39 常量、:51-52 ticks、:69-80 计数分支，halt 改空，加退化保护；用例删 2 加 3。验收 cp→md5 同→face 6/6/12→--all-targets。自重合用例须 yaw=90°。6 处「60 拍」白名单外未动。
