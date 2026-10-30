# wb_40_ugv_motion — task_40 工作记录

> Presented by KeJi
> Created Date ： 2026-10-29
> Modified Date ： 2026-10-29
> 任务：`Task/task_40_ugv_motion.md`（车栈执行器连续化 / 验证 / Pictor 命令通道 / Follow 进 Pictor / 四旋翼外观）

## 状态（2026-10-29 21:42 +08:00）
- 文档**定稿**；§八 8 条参数/口径已由人类确认。
- **未开工**：执行引擎尚未启动，本文件仅建立上下文。

## 五步
1. 车栈执行器 → 与 sim uav 同构三档（沿用 `[chassis]` 百分比档位；新 `ugv_sim/executor.rs`；`sim/executor.rs` 一行不改）
2. 验证：小车能否正常运动（只验不改；6 用例 / 8 判据；对照 `results/queue_ugv_dev/sim_1.log`）
3. Pictor WS 命令通道（GDScript 节点 + `EventBus.cmd_send` 双向 + `Pictor/tool/pictor_ws_ctl.py`）
4. `Follow`(type=2) 进 Pictor 并用它下发（只验直走；FAIL 不阻塞，跟随由人类自验）
5. 四旋翼换免费模型（CC0 优先 / CC-BY 可接受；AABB 保持 `0.64×~0.13×0.64`，动力学零改动）

## 关键上下文（后续 agent 只需读这些）
- **两层协议**：层 1 = ORION/libp2p（Pictor ↔ 车，只有任务/模式/离散手动）；层 2 = sim 进程内 `motion_cmd` JSON（决策链 → Godot 世界）。executor 输出只活层 2。
- **Step 2 基线**（不单独跑）：`RobotSim_TestEnv/results/queue_ugv_dev/sim_1.log` → 21.5 s / 4.02 m / `Stop` 10 / `MoveForward` 5。
- **测试环境**：世界用 `RobotSim_TestEnv/world_fast/`（`msaa_3d=0`），`Xvfb`；`llvmpipe` 直跑只有 2 fps、位姿龄 245 ms，判据会失真。
- **代理**：mihomo `http://127.0.0.1:7897`（mixed-port，实测 200）；本机直连也通，代理仅备用。
- **Step 5 自检钉子**：`check_models.gd` 路径/AABB/`UAV_NoseRed`、`check_sensor_models.gd:31`、`Vehicle.tscn:16`。

## Step 记录
（开工后按 step 追加：step 号 / 轮次 / 结果 / 关键决策 / 起止时间）

## 执行引擎启动
- 开始时间：2026-10-29 13:44:13Z
- 引擎：Tool/execution_engine_task40.ts + Tool/execution_engine_task40.config.json（流程照搬 task_36/38/39）
- 运行标记：首轮（Step 1~5）（本次从 Step 1 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次
- 范围：车栈执行器（1）→ 验证（2）→ Pictor WS 命令通道（3）→ Follow 进 Pictor（4）→ 四旋翼外观（5）
- 验收红线：Step 2/3/4 **只验不改**；Step 4 判据 3（真跟、只验直走）允许 FAIL 且不阻塞

## Step 1 — Step 1 — 车栈执行器改成与 sim uav 同一套逻辑（首轮（Step 1~5））
- 起止：2026-10-29 13:44:13Z → 2026-10-29 14:02:23Z
- 范围：新增 sim 车栈连续执行器 + 线上 turn 字段 + 世界侧 DiffDrive velocity 分支（不动 sim/executor.rs、不动 nav/mission、不碰真机与 ORION）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task40/plan.md)，没有改动任何源码，`git status` 仍与进场基线一致。 计划摘要（≤200 字）：新增 `ugv_sim/executor.rs`（三档控制律 + 身后直行，输出档位；常量 5°/10°/0.5 m；`Δθ>0⇒turn<0`，共 13 条单测）；`MotionCmd` 加 `turn`、`Send_Velocity_Pct`、`Apply_Velocity_Pct`（因重载被占用，登记偏差 D1）、`sim_handler` 三态 `Actuator`；`DiffDrive.gd` 加 `velocity`（解除互斥）并补 3 条自检；`simq_ugv` 的 `turn_speed` 设为 30；架构文档补 §8.11、D-S9。验收：逐 crate `cargo check/test --bins`、`godot --headless` 自检（`total=44`），并对两个 executor 执行 `diff` 验证逐字相同。

## Step 2 — Step 2 — 验证：最新的小车能不能正常运动（首轮（Step 1~5））
- 起止：2026-10-29 14:27:27Z → 2026-10-29 14:34:23Z
- 范围：**只验不改**（新建 `Tool/sim_task40_ugv_test.py` + `RobotSim_TestEnv/results/task40_20261029T142726Z/`）；Rust / 世界 / Pictor / `Task/` 零改动
- 轮次：1 / 3
- 结果：**7 条 PASS / 1 条 FAIL / 1 条记录**（判据 2「不抖」FAIL）
- 环境：`world_fast`(msaa_3d=0) + `xvfb-run -a`；位姿龄 中位/最大 50/177 ms；执行器活体：`[Sim] UGV` 1815 行、旧离散 `[Sim] 位置=(` 0 行；二进制 `f93250f16195`（=target）、`sim_queue_test.py` `28baace30c4f`
- 用例：M0 10 s 静止位移 0.000 m｜M1 20.25 m/46.9 s｜M2 18+18 m/87.4 s｜M3 22 m(44 格)/49.9 s｜M4 19.80 m(45°)/45.9 s｜M5 20+20+17.5+14 m(3.5 m 门洞)/171.5 s；合计 169.55 m / 400 s ≈ 0.424 m/s
- 判据 1 到达：9/9 段 `[Nav] 到达` + `[Robot] 任务完成`，末位逐轴最大 **0.25 m**（≤0.26）→ PASS
- 判据 2 不抖：turn 反号 **M1 11 次（≤2）／M4 8 次（≤3）** → **FAIL**（直线上 ±30 档单拍纠偏交替出现，非停走、不撞墙）。⚠ 三次观测 M1 = **2 / 5 / 11**、M4 = 4 / 8 ⇒ 该指标**贴阈值且轮间波动大**（`task40_sanity_20261029T143819Z/` 就是 M1=2 那次复跑）
- 判据 3 不打转：10 s 净位移 M1 最小 4.14 m／M4 4.33 m（≥2）→ PASS
- 判据 4 不撞墙：急停 0 次；几何净空 min 1.73 m（`wall_north` @ (34.28,125.77)）→ PASS（口径见 D4）
- 判据 5 该停会停：M0 44 行全 (0,0)、位移 0.000 m；任务后首个 (0,0) 延迟 max 0.25 s（≤1.0）→ PASS
- 判据 6 停走对比（只记录）：任务期 (0,0) 时间占比 2.2%／行占比 2.2%；改前基线 3.0%／50.0%（离散日志事件驱动、连续日志 5 Hz ⇒ 只有时间加权同口径）
- 判据 7/8：`[Sim] UGV` 1815 行全部解析成功（失败 0）；`拍无路` 0 / 急停 0 / 任务失败 0 → PASS
- 偏差：D1 只复用 `sim_queue_test.py`｜D2 M5 用世界最窄 3.5 m 门洞（无 1 格走廊）｜D3 时间轴用日志 µs（钟偏 0.1 ms）｜D4 判据 4 = 急停 0 + 几何净空｜D5 前置 #1 与进场基线比对
- 产物：`RobotSim_TestEnv/results/task40_20261029T142726Z/{verdict.md(18 行), trace.csv(2082 采样), sim.log(2.4 MB), world.log, cases.json, config.toml}`；脚本 `Tool/sim_task40_ugv_test.py`
- 结论：连续执行器**能正常走完**（到达 / 不打转 / 不撞墙 / 该停会停 / 无回归全过）；唯一不达标的是判据 2「不抖」——全跑那轮直线上交替 ±30 单拍纠偏 11 / 8 次，但三次观测跨度 2~11 说明它压在阈值上。按 §三「只验不改」，本轮不改阈值、不改代码，留待人类裁决

## Step 2 — Step 2 — 验证：最新的小车能不能正常运动（首轮（Step 1~5））
- 起止：2026-10-29 14:02:23Z → 2026-10-29 14:52:55Z
- 范围：只验不改：新建验证脚本 + 结果目录（不动 Rust / 世界 / Pictor）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task40/plan.md)（470 行）。全程只读代码：`git status` 与本步进场基线逐行一致（仅有基线文件自身缺尾换行的差异），没有改动任何源码。 **关键结论（读码得出、会直接决定成败）**： 1. **单任务 60 s 业务超时**（`ugv_sim/mission/goto.rs:31`）+ `forward_speed=30`（0.45 m/s）⇒ 每段最多约 25 m。任务文档里 M2 的 32 m 段、M3 的 24 m 段按原样写会**必然 Timeout**，且 `turn_speed` 属 §八 定值不可改 ⇒ 只能把用例切段（全部 ≤20 m，共 9 段 / 170 m ≈ 6.3 min）。 2. **`RobotSim_TestEnv/world_fast/` 是旧副本**，其 `DiffDrive.gd`（7 710 B, 10-23）**没有 `velocity` 分支** ⇒ 直接跑车根本不动；须照 task39 的 `world_stamp` 逻辑重拷（源已 9 185 B, 10-29 13:55）。 3. **`target/debug/pleiades-sim` 早于 Step 1 改动**（10-27 13:21）⇒ 必须先 `cargo build`；并用 `[Sim] UGV` vs `[Sim] 位置=(` 做"跑的是新执行器"的活体闸门。 4. **世界没有 1 格（0.5 m）宽走廊**：`OBSTACLES` 全表最小的可通行缝隙是 `wall_d` 北端↔北边界墙 = 3.5 m ⇒ M5 按此实现并登记偏差；判据 4 的"全向 lidar"无观测源（sim 不打印 range、控制口无传感器字段），改为「急停 0 次（仪器化）+ 轨迹到 OBSTACLES 的几何净空」双口径。 5. 陷阱：`sim_queue_test.write_config()` 会把 `turn_speed` 覆写成 10 ⇒ 脚本自带 config 模板并回读断言 30。 计划摘要（

## Step 3 — Step 3 — Pictor 无头驱动：WS 命令通道 + Python 控制脚本（首轮（Step 1~5））
- 起止：2026-10-29 14:52:55Z → 2026-10-29 15:31:15Z
- 范围：Pictor 内新增 WS 服务节点 + Python 控制脚本（不动 ORION 协议、不动 executor、不管 Manual）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task40/plan.md)（354 行）；仅读码，根仓 `git status -uall` 与进场基线逐行一致、Pictor 仓干净。要点：`Pictor/bin` 缺 .so 且 debug 版陈旧→先 release 构建再拷（bin/ 已被忽略）；kernel 按 CWD 读 `.config`→Pictor 须以 `RobotSim_TestEnv/pictor_ws/` 为 CWD 起；`OpMode` 默认 Auto；`valid=sub_target` 支撑双信号判到达；`cmd_send` 需 hex vid（复用 VehicleRegistry）。改动 15 项（新增 4 文件 + 2 个 .uid；改 main.tscn:16/29、main.gd:8、requirements:3、docs:69/503/520），零越界。K1~K7 逐条给命令与判据；基线比对须 `git status -uall`（默认会折叠未跟踪目录致假红）。

## Step 4 — Step 4 — Follow 进 Pictor，并用 Pictor 下发跟随（首轮（Step 1~5））
- 起止：2026-10-29 15:31:15Z → 2026-10-29 15:49:00Z
- 范围：Pictor 协议层放开 `type=2`（`protocol_def.gd` / `orion_messages.gd` / `message_builder.gd`）+ 单测 + `pictor_ws_ctl.py follow` + docs + 端到端验收脚本（`pleiades-base` / `pleiades-sim` / 真机一行未动）
- 轮次：1 / 3
- 结果：PASS（K1~K7 全 PASS；**判据 3 一次就过，未用上"允许 FAIL"的豁免**）
- 规划要点：计划已写入 [plan.md](tmp/task40/plan.md)（488 行）。要点：① 代码改动集中在 3 个 GDScript（`message_builder.gd` **只改注释**，它委托 `ProtocolDef.Mission_Type_From`，再建白名单会变成两处规则）；② `type=3` 的"仍被丢"只能用"降级成 0 + 告警"证明（Pictor 降级、车端 `command_decode` 才真正丢）；③ 检测链在 debug 构建下不可用 → 统一 `cargo build -p pleiades-sim --release` 一份二进制；④ 判据 1/2/4/5/6 全部不依赖"跟得上"，判据 3 按 step.md §6 允许 FAIL。
- 实施结果：单测新增 `_test_follow_task_set`（金标 `01020000000000000000`，`ALL PASS: true`）；`follow` 子命令带本地帧布局断言（`1+9n` / type 字节 / 金标）+ `--watch` 采样；端到端 `RobotSim_TestEnv/task40_step4/step4_driver.py`（1 471 行）跑 K1~K7 全绿、exit 0，收尾 0 残留 / 端口全空。
- 关键实测：机栈 `Follow { x: 0.0, y: 0.0 }` 入队（`[Robot] 开始任务 TaskId(0)`），车端无 `未知 mission type: 2`、无退化 Hold；`--type 3` 在 pictor.log 留 `mission type 3 unsupported (only {0,2})`、车端收到 `Goto`；直走跟随 10 s 位移 **7.74 m**、`k=1.0`（`llvmpipe` 下基本实时）、`d` 7.97→9.94 m（slope +0.183 m/物理秒）、`Σ|Δyaw|=180.65°`。
- 产物：[verdict.md](RobotSim_TestEnv/results/task40_s4_20261029T154557Z/verdict.md) + `sim.log` / `sim_ugv.log` / `ped.log` / `pictor.log` / `world.log` / `ped_trace.csv` / `pose_trace.csv` / `cases.json`（同目录）
- 偏差：D1~D14 见 verdict §4。三条来自实测打脸（已写进 `tmp/task40/impl.md`）：**D12** K4 的 target 必须用实测位姿（世界给第二台车分配的是 **(66,64)**，且 uav 停在 (64,64) 会被车端 lidar 标成障碍 → `D* rhs[start]=INF 不可达`、Goto 永不完成）；**D13** 自检判据放宽成"不得出现新 FAIL"（A10 实测 run 到 run 会抖，本步两次分别 failed=0 / failed=1）；**D14** 行人客户端要 `python3 -u`（stdout 重定向到文件是块缓冲，`[client] ready` 会卡住）。
- 备注：§八 #6「Follow 只走 WS 命令」已遵守——不加 UI 按钮、不加 LLM 词表；`Follow{x,y}` 语义未动（`x/y` 预留填 0）。

## Step 4 — Step 4 — Follow 进 Pictor，并用 Pictor 下发跟随（首轮（Step 1~5））
- 起止：2026-10-29 15:31:15Z → 2026-10-29 15:56:53Z
- 范围：Pictor 协议层放开 type=2 + Python follow 子命令 + 只验直走场景（不动 base/sim/真机）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task40/plan.md`；本轮只读取代码，根仓 `git status` 与进场基线逐行一致，Pictor 仍是 9 条改动。 **摘要**：`protocol_def` 增加 `MISSION_TYPE_FOLLOW=2`，并让 `Mission_Type_From` 接受 `"follow"`；`orion_messages` 的降级条件改为不在 `{0,2}` 时降级，并更新告警文案；`message_builder` 只改注释（归一化本就委托 ProtocolDef）；单测新增 `follow_task_set`，标准字节为 `01020000…`；`pictor_ws_ctl` 新增 `follow` 子命令（含 `--type`、`--watch`）；文档改 4 处；新增 `task40_step4/step4_driver.py`。K1、K2、K4、K5、K6、K7 必须通过；K3 只验直走，允许 FAIL 且不阻塞。检测链强制使用 release 构建，物理时间由 `world_time` 折算。

## Step 5 — Step 5 — 四旋翼外观：换一个像样的免费模型（首轮（Step 1~5））
- 起止：2026-10-29 16:11:20Z → 2026-10-29 16:16:32Z
- 范围：纯资产 + 两处路径常量（`Vehicle.tscn:16`、`check_models.gd`、`check_sensor_models.gd:31`）+ `assets/README.md` 台账；**不动**动力学/碰撞尺寸/`Quadrotor.tscn`/`Vehicle.tscn` 节点结构
- 轮次：1 / 3
- 结果：PASS
- 选型：Poly Pizza `Drone` / **NateGazzard**（https://poly.pizza/m/DNbUoMtG3H，上传 2023-12-31，站内 `Tris: 4564`）→ **CC BY 3.0**（§八 #7「CC0 优先；CC-BY 可接受」）。CC0 路径已逐条排除：Poly Pizza `quadcopter` 17 条里唯一 CC0（`EQJ2MECUbx`）渲染确认**是直升机**；`drone` 21 条 **0 条 CC0**；Kenney 直连与 `-x http://127.0.0.1:7897` 均 **403**；Sketchfab 下载需登录 token。⇒ 走 CC-BY 并做齐四件：`assets/ATTRIBUTION.md`（署名 + 改动说明）/ `assets/chassis/LICENSE_uav_quadrotor.txt`（CC BY 3.0 全文 19 467 B）/ README §5 台账 / 改动清单。
- 留证：源 `259 676 B` / `sha256 bc9f0d765e0fdb838a9baf26b472f955a11bb2509a3337e918054241b134638d`（2026-10-29 直连抓取，未走代理）；落地 `299 376 B` / `sha256 7aa77d6b21213819cef03c4531b7a810bcf8c962be83e7d7828f3238443daba9`。台账与许可全文已核字节数并与页面元数据（标题/作者/许可链接/`Tris: 4564`）逐条对齐。
- 归一（`tmp/task40/tools/uav_norm.py`，Blender 4.5.14，脚本自带硬断言）：清非 MESH → 断父保变换 → `transform_apply` → 绕 Z `+90°`（机头 −Y → **+X**）→ `Body` 追加 `UAV_NoseRed` 并把「正面朝前且靠前」的 **194 / 3 856 面**重涂（保住第 2 个 surface 结构，M4 by-mat 通道靠它）→ 非等比缩放 `k = (0.8191, 1.1158, 0.6744)` → 足迹居中、底面 `z = 0` → GLB/Y-up/无 Draco 导出。实测输出：`FINAL size = [0.6400, 0.6400, 0.1400]`、`lo = [-0.32, -0.32, 0]`、`TRIS = 4564`、材质 = `{ColourPalette, UAV_NoseRed}`、红鼻面片中心 `x = 0.2105`。
- 资产层探针（`tmp/task40/tools/uav_probe.gd`，真工程内跑）：`ROOT xf = identity`、6 个 `MeshInstance3D`、`WHOLE pos=(-0.3200, 0.0000, -0.3200) size=(0.6400, 0.1400, 0.6400)`、`Body surf[1] = UAV_NoseRed`、`NOSE-HIT center=(0.2105, 0.0788, 0.0000) plus_x=true`。
- 导入：`godot --headless --path orion-simulation-world --import` → **0 ERROR / 0 WARNING**，`assets/chassis/` 落地 4 件（`uav_quadrotor.glb` 299 376 + `.glb.import` 1 169 + `uav_quadrotor_ColorPalette.png` 2 882 + `.png.import` 1 062），旧 `assets/uav_quadrotor_lowpoly.glb`(+`.import`) 已 `git rm`。
- 自检：`check_models` **total=24 failed=0**（`M1-model-nodes` 显示新路径 + identity、`M5-uav-aabb` P(-0.32,0,-0.32) S(0.64,0.14,0.64)、`M4-uav-nose-plus-x` by-mat `center=(0.210,0.079,0.000)`、`M6-uav-hull-box` **size=(0.4,0.12,0.4) 未变**）；`check_sensor_models` **29/0**（`uav-aabb` 过）；`check_vehicle` **40/0**；`check_quadrotor` **53/0** —— 四个自检全绿，`total` 一个不差。
- 视觉（`tmp/task40/shots/uav_00_q34.png` / `uav_01_human.png` / `uav_02_top.png`，Xvfb + llvmpipe，1600×900）：能数出机架 / 4 条机臂 / 4 个电机舱 / 4 支桨（黄尖）/ 起落架（白滑橇）/ 机腹云台舱；俯视图 4 臂对称、红鼻朝 +X（画面右侧）；与人形（1.73 m）同框时无人机高度约 **1/12**，材质写实、无「方块感」。
- 性能（`world_fast`，同命令 3 次取中位）：改前 **11 / 10 / 10 → 中位 10**；改后 **10 / 11 / 10 → 中位 10**；另做配对 A/B（把改前世界副本重建到 `tmp/task40/fps_ab/before/`，交替各跑 3 次）改前 **10 / 10 / 10**、改后 **11 / 10 / 11（中位 11）** ⇒ **无回退**（6 个改后值中位 10.5 ≥ 基线 10）。`llvmpipe` 噪声 ±1 fps。
- 偏差（详见 `tmp/task40/impl.md`）：D1 `COMPONENTS.md:107/898/940` 仍写旧名（不在允许路径）；D2 `Quadrotor.tscn` 未改（`_layout_rotors()` 运行时按 `_size` 覆盖桨盘 `position`，改字面值无作用且会破坏「场景与脚本一致」注释）；D3 AABB y 取 **0.14**（文档写 0.13；`check_sensor_models` 的 `0.14 ±2%` 与 `check_models` 的 `0.11~0.15` 把 y 夹死）且横向非等比；D4 红鼻用「正面面片重涂」而非新增零件；D5 改前基线实测 10（非 §6.6 写的历史值 11）；D6 归一脚本与 `.blend` 只留 `tmp/`。
- 产物：`orion-simulation-world/assets/chassis/uav_quadrotor.glb`(+`.import`)、`uav_quadrotor_ColorPalette.png`(+`.import`)、`assets/ATTRIBUTION.md`、`assets/chassis/LICENSE_uav_quadrotor.txt`；改动 `Vehicle.tscn` / `check_models.gd` / `check_sensor_models.gd` / `assets/README.md`；删 `assets/uav_quadrotor_lowpoly.glb`(+`.import`)。
- 收尾：无 godot / Xvfb 残留进程，9999/9098/9100 端口空闲（7897 是常驻 mihomo，非本步启动）。

## Step 5 — Step 5 — 四旋翼外观：换一个像样的免费模型（首轮（Step 1~5））
- 起止：2026-10-29 15:56:53Z → 2026-10-29 16:23:18Z
- 范围：纯资产：找免费 UAV 模型 + 归一 + 落 assets + 接线 + 自检同步（不动动力学与碰撞尺寸）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task40/plan.md)（392 行）。本轮只做读取，没有改动源码：`diff` 与 `tmp/task40/git_baseline.txt` 逐行一致；此外，我在 `/tmp/ppq/`（未触及仓库）做完了端到端原型验证：下载 → Blender 归一 → 导出 glb → 在临时 Godot 工程 `--import`，实测出 AABB、surface 与材质名。因此计划中的数字都是实测量。 **计划摘要（约 100 汉字，可写入 workbook）** > 计划见 tmp/task40/plan.md（392 行），只读、与基线逐行一致。选型 Poly Pizza「Drone」/NateGazzard（CC BY 3.0，4564 tris，128² 贴图）；CC0 四旋翼两种搜法均 0（唯一 CC0 是直升机，已渲染确认），Kenney 403。归一处理：绕 Z 轴旋转 +90°、把正面面片重涂为 UAV_NoseRed（Body 第 2 surface）、非等比缩放到 0.64×0.14×0.64、底面 y=0；在临时 Godot 工程已实测 AABB 精确、nose 中心 x=0.21、材质名 UAV_NoseRed。改 13 条路径（5 改 / 2 个 git rm / 6 新增，含抽取的 _ColorPalette.png）；Quadrotor.tscn 不改（position 被 _layout_rotors 覆盖）；自检只改 2 个路径常量，AABB/HULL/M4/M5/M6 零改（y=0.14：±2% 夹到 0.1372~0.1428）。验收：导入 0 ERROR、24/0、29/0、三机位截图、fps 中位≥改前。 关键取舍（评审会问的三条）：① 不走 CC0 有证据链 —— Poly Pizza `quadcopter` 17 条仅 1 条 CC0 且是直升机、`drone` 21 条 0 条 CC0、Kenney 直连与代理皆 403；② y 取 0.14 而非文档的 0.13，因为 `che

## 引擎结束
- 结束时间：2026-10-29 16:23:18Z
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M Architecture/robot_arch.md
 M orion-simulation-world/assets/README.md
D  orion-simulation-world/assets/uav_quadrotor_lowpoly.glb
D  orion-simulation-world/assets/uav_quadrotor_lowpoly.glb.import
 M orion-simulation-world/src/chassis/DiffDrive.gd
 M orion-simulation-world/src/scenes/Vehicle.tscn
 M orion-simulation-world/src/tests/check_diff_drive.gd
 M orion-simulation-world/src/tests/check_models.gd
 M orion-simulation-world/src/tests/check_sensor_models.gd
 M pleiades-sim/src/sim/chassis/mod.rs
 M pleiades-sim/src/sim/chassis/ugv.rs
 M pleiades-sim/src/sim/sim_handler.rs
 M pleiades-sim/src/sim/ugv_sim/mod.rs
 M pleiades-sim/src/world_link/mod.rs
 M pleiades-sim/src/world_link/protocol.rs
?? Task/task_40_ugv_motion.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task40.config.json
?? Tool/execution_engine_task40.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Tool/sim_task40_ugv_test.py
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_40_ugv_motion.md
?? orion-simulation-world/assets/ATTRIBUTION.md
?? orion-simulation-world/assets/chassis/LICENSE_uav_quadrotor.txt
?? orion-simulation-world/assets/chassis/uav_quadrotor.glb
?? orion-simulation-world/assets/chassis/uav_quadrotor.glb.import
?? orion-simulation-world/assets/chassis/uav_quadrotor_ColorPalette.png
?? orion-simulation-world/assets/chassis/uav_quadrotor_ColorPalette.png.import
?? pleiades-sim/src/sim/ugv_sim/executor.rs
```

## 执行引擎启动（Step 6）
- 开始时间：2026-10-30 00:55:54Z
- 引擎：Tool/execution_engine_task40.ts + .config.json（FROM_STEP=6）
- 运行标记：Step 6 死区复验（DEADBAND 5→8 / ARC 10→12）
- 计划：Step 6 最多 3 轮（规划 → 实现 → 评审）
- 目的：判据 2「不抖」收口 —— 只调死区（5°→8° / ARC 10°→12°），不拿拐角速度换稳定

## Step 6 — Step 6 — 死区调大（5°→8° / ARC 10°→12°）+ 复验（第 1 轮）
- 起止：2026-10-30 00:58:43Z → 2026-10-30 01:13:11Z
- 范围：只改 `pleiades-sim/src/sim/ugv_sim/executor.rs`（`DEADBAND_DEG` 5.0→8.0、`ARC_MAX_DEG` 10.0→12.0 + 文件头三档表/两处 doc + 单测同步）；`Architecture/robot_arch.md` D-S9 与常量速查同步；复验产物 `RobotSim_TestEnv/results/task40_s6_20261030T010511Z/`；`Tool/sim_task40_ugv_test.py` **一字未改**
- 轮次：1 / 3
- 结果：**7 条 PASS / 1 条 FAIL / 1 条记录**（判据 2「不抖」仍 FAIL）
- 环境：`world_fast`(msaa_3d=0) + `xvfb-run -a`；位姿龄 中位/最大 50/171 ms（中位 ≤150 ✓）；执行器活体 `[Sim] UGV` 1802 行、旧离散 `[Sim] 位置=(` 0 行；二进制 sha256[:12]=`89bc4e2eb0e8`（=target）；config 回读 `forward_speed=30 turn_speed=30`
- 用例：M0 10 s 静止｜M1 正东 20.25 m 46 s｜M2 18+18 m 88 s｜M3 22 m(44 格) 50 s｜M4 19.80 m(45°) 46 s｜M5 20+20+17.5+14 m(3.5 m 门洞) 172 s
- 判据 1 到达：9/9 段 `[Nav] 到达` + `[Robot] 任务完成`，末位逐轴最大 **0.24 m**（≤0.26）→ PASS（Step 2 为 0.25 m，余量反而变松）
- 判据 2 不抖：turn 反号 **M1 2 次（≤2）／M4 4 次（≤3）** → **FAIL**（M1 压线命中、M4 超 1）。对照 Step 2：M1 11（三次观测 2/5/11）、M4 8（4/8）⇒ 本步 M1 由 11→2、M4 由 8→4，**两条都在改善，但 M4 仍差 1 次**
- 判据 3 不打转：10 s 净位移 M1 最小 4.28 m／M4 4.29 m（≥2）→ PASS
- 判据 4 不撞墙：急停 0 次；几何净空 min 1.70 m（`wall_north` @ M5）→ PASS
- 判据 5 该停会停：M0 43 行全 (0,0)、位移 0.000 m；任务后首个 (0,0) 延迟 max 0.25 s（≤1.0）→ PASS
- 判据 6 停走对比（只记录）：任务期 (0,0) 时间占比 2.1%／行占比 2.2%（Step 2 为 2.2%/2.2%，基本不变）
- 判据 7/8：`[Sim] UGV` 1802 行全部解析成功（失败 0）；`拍无路` 0 / 急停 0 / 任务失败 0 → PASS
- 追加判据（人工）：**M2 耗时 87.8 s ≤100 s → PASS**（Step 2 87.4 s，+0.4 s ⇒ 拐角**未劣化**，符合「不拿拐角速度换稳定」）
- 新值生效证据：`sim.log:29` = `[Sim] 车栈执行器 = 连续档位（forward_speed=30 turn_speed=30，DEADBAND=8°，ARC_MAX=12°，PASS_THROUGH=0.50 m）`，grep 命中 1 行
- 单测：`cargo test -p pleiades-sim --bins executor` ⇒ **29 passed / 0 failed**（车栈 14 条：13 原有用例全保留 + 新增 `test_deadband_boundary_7_vs_8_5`；改名 `test_spin_right/left_over_12deg`、`test_arc_boundary_12_vs_12_5`）
- 偏差：D-S6-1 脚本内 3 处旧死值（`DEADBAND_DEG = 5.0` / `ARC_MAX_DEG = 10.0` / docstring），零引用死常量，按 §七.7「同一脚本同口径」不改｜D-S6-2 `pleiades-base` 的 `--all-targets` 有 **30 条既有编译错误**（`test "t01_storage_integration"`，与本步无关，改动前后一致）｜D-S6-3 `robot_arch.md:817-818`「`turn_speed ≥ 60` 须抬到 7~8°」按计划保留原文（前瞻条件，与现值 8° 不冲突，但字面读起来像可再降）｜D-S6-4 在 §8.11 速查里补了一句 §七.2 的过冲算术（A3 之外的一句，用于自解释 8° 的来历）
- 产物：`RobotSim_TestEnv/results/task40_s6_20261030T010511Z/{verdict.md(24 行，含 Step 6 人工追加段), cases.json, config.toml, sim.log(2.4 MB), trace.csv(2084 行), world.log, sim_stdout.log}`
- 收尾：sim/world 已停；9999/9098 空闲；残留进程 0；`/tmp/.X*-lock` 0
- 结论：**死区 8°/12° 已生效且基本方向正确**（M1 11→2 达标、M4 8→4、到达余量 0.25→0.24 m、M2 87.4→87.8 s 未劣化、其余 7 条判据全 PASS），但**判据 2 仍未达标（M4 4 > 3）**。按 §七.8 不调阈值/不改判据/不改脚本、不自动试第二组参数 ⇒ **记 FAIL 并停下等人类裁决**（候选方向：接受 M4 贴阈值并复跑 2 次取中位／确认 `TURN_FLIP_MAX` 是否该按 M4 单独放宽／改 45° 斜线用例的判据口径 —— 均需人类定，本步不动）

## Step 6 — 死区调大（DEADBAND 5°→8° / ARC 10°→12°）+ 复验（人类裁决 PASS）
- 起止：2026-10-30 00:55:54Z → 2026-10-30 01:25Z（引擎第 1 轮实现完成后被主 agent 主动 cancel，未跑自动第 2 轮）
- 代码：`pleiades-sim/src/sim/ugv_sim/executor.rs` 两个常量 5.0→8.0 / 10.0→12.0 + 文件头三档表 + 6 条边界单测（`7°` 直行、`8.5°/10°/12°` 弧线、`12.5°` 原地）；`Architecture/robot_arch.md` D-S9 / §8.11 / 常量速查同步
- 主 agent 复核：`cargo check -p pleiades-sim --all-targets` 过（仅 2 条既存 warning）；`cargo test -p pleiades-sim --bins executor` **29 passed / 0 failed**
- 复验（`RobotSim_TestEnv/results/task40_s6_20261030T010511Z/`，同 Step 2 口径 6 用例 8 判据）：到达 9/9、末位 0.24 m、急停 0、M2 87.8 s(+0.4)、日志 1802 行全解析
- 判据 2 原始值：M1 **2**（≤2）、M4 **4**（≤3）——原口径下 M4 超 1
- **关键发现**：两次实测（Step 2 / Step 6）逐条分类 ⇒ 总反号 105 / 46，**同格内 0 / 0**，全部落在换格那一拍（M4 = 45° 斜线阶梯，几何必然）⇒ 判据 2 数的不是抖动
- **人类裁决（2026-10-30）**：口径改为「同一格内反号（M1≤2 / M4≤3）」，换格反号仅记录 ⇒ **判据 2 PASS**；不调参数、不上迟滞；原始实测值保留
- 备注：日志 5 Hz 节流 ⇒ "同格内 0" 指 ≥200 ms 粒度；若要更高分辨需把车栈日志改 50 ms（未做）
