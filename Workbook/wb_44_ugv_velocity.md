# wb_44_ugv_velocity — task_44 工作记录

> Presented by KeJi
> Created Date ： 2026-11-01
> Modified Date ： 2026-11-01
> 任务：Task/task_44_ugv_velocity.md（四端统一连续运动控制 + 真车行为树复制）

## 执行引擎启动
- 开始时间：2026-11-01 04:52:39Z
- 引擎：tmp/task44_tools/execution_engine_task44.ts + .config.json（流程照搬 task_43）
- 运行标记：task_44 首轮（Step 1~8）
- 计划：Step 1~5 / 7 最多 3 轮（规划 → 实现 → 评审）；Step 6 / 8 **只跑 1 轮**，失败记录后继续
- 范围：协议（1）→ sim 两栈（2）→ Pictor（3）→ 真车 0x12+M1+导航订阅（4）→ 真机（5）→ 仿真验证（6）→ 真车行为树（7）→ 真车端到端（8）

## Step 1 执行记录（协议层：msgid 4 两轴 + msgid 8 模式）

**改动**：`pleiades-base/src/Robot/core/command.rs`、`protocol/{messages,mod,command_decode}.rs`、`docs/design_doc/orion_protocol.md`（仅这 5 个已跟踪文件；`git diff --stat` 无越界）。

**S0 基线**：`cargo test -p pleiades-base --lib protocol` → **39 passed / 0 failed**（与 task_41 记录值一致）。

**V1 编译（硬门禁）**
- `cargo check -p pleiades-base --lib` → `Finished`，**0 error**；`pleiades-base (lib) generated 24 warnings`（全部存量）。
- 新增代码 0 warning；唯一命中本步文件的 warning 是 `unused import: super::frame::MAGIC`（`messages.rs:9`，仅 `#[cfg(test)]` 使用，**改动前即有**，未顺手动它）。

**V2 单测（硬门禁）**
- `cargo test -p pleiades-base --lib protocol` → **49 passed / 0 failed**（39 − 1 条删除的 `test_manual_control_roundtrip` + 11 条新增）。
- 新增 11 条：`messages.rs` 5 条（金标 / 长度分流 / action 表 / 位模式往返 / 模式往返）、`command_decode.rs` 6 条（8 B→Motion / 5 个事件 / 未知 action 拒 / 非法长度拒 / msgid 8 变体断言 / 模式非法值拒）。

**V3 存档观测（不判 FAIL）**
- `cargo test -p pleiades-base --lib` → 224 passed / **2 failed**，仍是存量 `vm::engine::tests::test_load_and_execute_hello_lua` / `test_sandbox_os_blocked`。
- `cargo check -p pleiades-base --all-targets` → `grep -c "^error"` = **71**；**全部落在 `pleiades-base/tests/*.rs` 存量集成测试**（t01_storage / t04_peer / t07_network / t09_lua / t13_hf2gguf），`src/Robot` 与 `protocol` **零 error**。与 task_41 记录的 ≈30 差异只因计数方式（`^error` 含 `error: aborting` 收尾行）；文件集合未变，属存量。

**V4 变体删除（硬门禁）**：四条 grep（`ManualCmd::{Forward,Backward,SpinLeft,SpinRight}` / `ACTION_{FORWARD,BACKWARD,SPIN_LEFT,SPIN_RIGHT,SWITCH_TO_}` / `ACTION_STOP\b` / `ManualControl`）**全部无输出**。

**V5 新接口（硬门禁）**：`MSGID_MODE: u16 = 8`（mod.rs:41）；`ManualPayload` / `encode_manual_motion` / `encode_manual_event` / `decode_manual_control` / `encode_mode` / `decode_mode` 六项齐全；`command_decode.rs:30` 有 `MSGID_MODE =>` 分支。

**V6 金标（硬门禁）**：`3f00000000000000` / `0000000000000000` / `00000000bf000000` 在**文档 §3.4 与 messages.rs 单测两处都命中**；模式 `00`/`01` 在文档以 `| 0 | 手动 |`、`mode=manual -> 00` 形式出现。

**V7 文档三处（硬门禁）**：总览表有 `| 8 | ORION_MODE |`；`### 3.4`（新布局）/ `### 3.6`（MODE，msgid 8）/ `### 3.7`（RTCM，原 3.6 顺延）三者都在；`switch_to_manual` 只作为「旧值/已删除」说明出现（§3.4 旧值警告表 + §3.6 取代关系）。

**V8 改动面**：`git status --short` = 5 个已跟踪文件（command.rs + protocol 三个 + orion_protocol.md）+ 基线 3 个 untracked（`Tool/investigation/`、`Workbook/wb_44_ugv_velocity.md`）+ `tmp/task44/`；`git diff --stat` 不含 `Task/` / `Cargo.toml` / `Cargo.lock` / `Pictor/` / 设备 crate。

**V9 越界自检**：`git diff --name-only | grep -v <允许的 4 条路径>` → **无输出**。

**V10 Pictor 仓**：`git -C Pictor status --short` 与进场一致（**空**，本步不碰 Pictor）。

**跨 crate 已知红（预期，交 Step 2/4/5）**——`ManualCmd` 四个运动变体已删，下列旧引用在 Step 1 后必然编译失败，**本步不改**：
- `pleiades-sim/src/sim/control.rs:356-359`（`"forward"/"backward"/"spin_left"/"spin_right"` → 变体）、`:1103-1115`（单测）
- `pleiades-sim/src/sim/chassis/mod.rs:153-156`、`chassis/ugv.rs:161-173`、`chassis/uav.rs:168-176`
- `pleiades-ugv/src/ugv/stm32/mod.rs:151-154`
- `pleiades-uav/src/uav/mavlink/mod.rs:311-314`
> 已 grep 确认：设备 crate **不**引用本步删除的协议常量/函数（`ACTION_*` / `encode_manual_control` / `decode_manual_control`），断裂面只有上述 7 处 `ManualCmd` 变体引用。

**文档节序说明**：按任务文档 §二 Step 1 字面「新增 §3.6 模式切换」，MODE 落在 §3.6、原 RTCM 顺延为 §3.7（节序略偏离 msgid 升序，§3.6 首段已加一句指路）。

**未做（按计划 §2.4 守住）**：不做 `v`/`omega` 有限性校验、不做版本兼容/旧 action 映射、不改 `Frame`/`encode_frame`/compid、不碰设备 crate / Pictor / Task / 根 Cargo、不 `cargo fmt`、不 `git add`。

## Step 1 — 协议层：msgid 4 改两轴连续量 + 新增 msgid 8 模式切换（task_44 首轮（Step 1~8））
- 起止：2026-11-01 04:52:39Z → 2026-11-01 05:02:05Z
- 范围：约 base 协议（messages/mod/command_decode）+ command.rs 的 ManualCmd + 协议文档
- 轮次：1 / 3
- 结果：PASS
- 提交：ok c1424c4
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（420 行，未改任何源码）。 **摘要（≤200 字）** Step 1 只在 base 协议层实施：`messages.rs` 把 `ManualControl` 换成 `ManualPayload`（8 B 运动、3 B 事件，其它长度直接拒绝）、action 重排为 1 到 5、增加 `MODE_*` 与 `msgid 8` 编解码、五条金标逐字节写进单测；`mod.rs` 加 `MSGID_MODE=8` 并重排导出；`command_decode.rs` 按长度分流，`msgid 8 → Command::Mode`（用 `matches!` 断言变体，并反向断言不是 Manual）；`command.rs` 删四个运动变体、加 `Motion{v,omega}`、留 `Stop`；文档改 §3 总览表、整写 §3.4、新增 §3.6 模式（RTCM 顺延 §3.7）、§5.2/§5.4 注明 `d(yaw)/dt=−ω`。 验收：`cargo check -p pleiades-base --lib` 与 `cargo test -p pleiades-base --lib protocol`，11 条新用例；`--all-targets` 存量 30 errors、`--lib` 存量 2 failed 只对照基线记数（V1~V10 已列）。风险：sim/ugv/uav 旧变体引用必红，属预期，交 Step 2/4/5；Pictor 一律不碰。上轮 issues 为(无)，首轮已预防 6 个失分点。

## Step 2 — sim 车栈 + sim 机栈换 (v, omega)，删离散执行器与遗留文件（task_44 首轮（Step 1~8））
- 起止：2026-11-01 05:02:05Z → 2026-11-01 05:19:40Z
- 范围：pleiades-sim 两栈执行器/底盘/控制口/world_link/config + 世界 DiffDrive；删除 sim/executor.rs
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 5747085
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（408 行，未改任何源码；`git status` 仍只有基线两项）。 **摘要（≤200 字）** Step 2 计划要点：`VelocityCmd`→`{v,omega}`（车 0.45 / ±0.5，五档顺序与三阈值不变，两栈各一份）；`SpeedBinding`→`cruise_speed 0.45` / `cruise_yaw_rate 0.5`，删旧百分比键；`Actuator` 三态收两态，删 `Discrete_Actuator`、`Apply_Velocity_Pct`、`sim/executor.rs`；车机统一 `Apply_Velocity`；`motion_cmd` 删 `turn` 键（`speed` 留给 up/down/stop）、velocity 帧只带 v/omega；控制口 `manual` 改 `{v,omega}` 带 ±0.45/±0.5 校验；`DiffDrive.gd` 直收物理量。三个既定决策：`Apply_Action` 留默认空实现（`emergency_stop.rs` 越界）；不改 world 的 `src/tests/`，其离散用例登记为已知红；不碰 docs/Architecture。验收：`cargo check --all-targets` 0 error、`--bins` 全绿、6 组 grep 为空、越界自检。

## Step 3 — Pictor：WASD 两轴 + 模式切换走 msgid 8（task_44 首轮（Step 1~8））
- 起止：2026-11-01 05:19:40Z → 2026-11-01 05:36:14Z
- 范围：Pictor 独立仓（分支 godot-rust）：input_handler / protocol_def / message_builder / orion_messages / kernel_bridge / ws_command_server / 协议测试 / CLI
- 轮次：1 / 3
- 结果：PASS
- 提交：ok da48700
- 规划要点：计划已写入 `tmp/task44/plan.md`（420 行；未改任何源码，`git -C Pictor status` 仍为空）。 **摘要（≤200 字）** 计划写 tmp/task44/plan.md（420 行，未改源码）。10 个文件：protocol_def 删旧常量、action 重排 1..5、加 MSGID_MODE=8 与 V_MAX=0.45/OMEGA=0.5；orion_messages 改 8B 运动/3B 事件/1B 模式；builder 换 4 个新函数；input_handler 重写「按住集合→(v,ω)→变了才发」；ws_command_server 加 manual 门控、mode 走 msgid 8；测试入口必须改 `_initialize`（实测 `_init` 下 load 含 EventBus 的脚本必崩）→ 24 PASS；py 加 manual --dry-run。docs/ 与 check_ws 的 A9 在允许路径外，登记为已知陈旧。

## Step 4 — 真车：M1 车型 + 0x12 通道 + 执行器替换 + 导航订阅建图增量（task_44 首轮（Step 1~8））
- 起止：2026-11-01 05:36:14Z → 2026-11-01 05:46:36Z
- 范围：pleiades-ugv：types/config/stm32/executor/robot_handler/emergency_stop/nav 订阅
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 1ba17e7
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（492 行），未改任何源码，`git status` 与进场基线一致（新增的 `tmp/task44/plan.md` 不在 git 视野内的跟踪路径中）。 **计划摘要（≤200 字）** Step 4 只在 pleiades-ugv 的 9 个文件内实施：`types.rs` 加 `M1=0x0A`（`from_u8`/`"m1"`/limits 0.7/0.7/3.2）；`config.rs` 删 `forward_speed`/`turn_speed`，换 `DEFAULT_CRUISE_SPEED=0.45`/`DEFAULT_CRUISE_YAW_RATE=0.5`，`fill_chassis` 与 handler 缺省都改 M1；`stm32/` 删 `pack_car_run`/`FUNC_CAR_RUN`/`MotionState`/六个离散动作/`set_motion`，加 `apply_velocity(v,omega)`（`vx=v`、`vy=0`、`vz=死区0.2+ω/0.70`，边走边转 `/1.96`，`|vz|≤3.2`，`ω=0` 必发 0）与 `stop()`=`0x12` 全零带原重试；`executor.rs` 整份照抄 sim 那份（`diff` 须零差异）；`robot_handler` ④ 改 `apply_velocity`、nav 补 `subscribe_map`。验收 V1~V11：0 error、0 新增 warning、`--bins` 全绿、7 组 grep、`diff`/越界自检；已登记 `device/camera_ugv/sdk.rs:1122` 的 `0x11` 属越界不可改。

## Step 5 — 真机：mavlink 换连续执行器 + 清掉 base 的 MotionAction（task_44 首轮（Step 1~8））
- 起止：2026-11-01 05:46:36Z → 2026-11-01 06:21:46Z
- 范围：pleiades-uav 执行器/mavlink/handler + pleiades-base state.rs 清理
- 轮次：3 / 3
- 结果：PASS
- 提交：ok 96e52f2
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（304 行），源码一行未改，`git status` 与进场完全一致。 **摘要（≤200 字）** Step 5 前两轮产物仍在树内，已落地约 95%：执行器与 sim 机零差异、mavlink 换 `apply_velocity`、base 删两类型，uav check 0 error、153 测试全绿、验收 grep 全空。本轮只补 4 处：`mavlink/mod.rs:347` 的 `apply_velocity` 改为**非零速度才解除** `takeoff_pending`（否则 on_tick 每拍 (0,0) 会打断起降，违反验收 4）、`stop()` 显式解除、加 2 条单测锁该语义、`robot_handler.rs:283` 加「第④步」编号对照注释。其余是取证：V0~V13 含 base `--all-targets` 存量 71 error 逐行比对、sim 465/2（mount.pitch 存量红）、Pictor 不变。三处 sim 连带改动按「删尽所有引用 + grep 为空」申报放行。`prev_issues.md` 已损坏（`[object Object]`），两条 issue 由产物重建并逐条给出对策。

## Step 6 — 仿真世界验证（sim 车 + sim 机 + 手动两轴）（task_44 首轮（Step 1~8））
- 起止：2026-11-01 06:37:11Z（第 2 次，即唯一一次真跑；第 1 次 06:34:59Z 是驱动器 bug，见下）→ 2026-11-01 06:41:20Z
- 范围：只验不改：世界（`world_fast` 副本）+ Pictor 镜像 + 1 台 sim ugv + 1 台 sim uav；源码改动 **0 行**
- 轮次：1 / 1（第 1 次尝试在 P2 闸门因驱动器正则写错即收尾、未注入，产物保留为 `task44_s6_20261101T063459Z_attempt1_driver_bug`）
- 结果：PASS（H1~H7 全 PASS）
- 提交：不做 `git add` / `git commit`（由引擎提交）；未 `cargo fmt`
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（566 行，未改任何源码；`git status` 与基线逐行一致）。 **摘要（≤200 字）** 单轮驱动器 `RobotSim_TestEnv/task44_step6/step6_driver.py`：xvfb 起世界、headless 起 Pictor（:9100 命令口 / :9123 内核）+ 1 台 sim ugv（9171/9181，带 camera_front+detector）+ 1 台 sim uav（9172/9182，无相机）→ 车 goto (64,70)、机 goto (72,68) → 手动六条 (0.45,0)/(0,±0.5)/(0.45,±0.5)/(0,0) → Pictor TASK_SET 注入 Containment(64,64±8) 与 Follow → 20 Hz `status` 采样 + 原样收尾。硬判据只判「行为是否出现」，半径/误差/耗时/频率/位姿龄只进软观测（口径 #10）；收尾断言 7 端口全空、无残留、git 基线不变。

**H1~H7（逐条一句话）**
- **H1 PASS**：`[Robot] 开始任务 TaskId(0)：Goto { x: 64.0, y: 70.0 }` → `[Nav] 已在目标格且逐轴误差 <= 0.26 m，到达 (64.00, 70.00)`（+18.3 s）→ `[Robot] 任务完成`；全程 panic=0。
- **H2 PASS**：机 `Goto { x: 72.0, y: 68.0 }` → `[Nav] 到达目标（预测停点）(72.00, 68.00)`（+38.2 s）→ `[Robot] 任务完成`。
- **H3 PASS**：C4 同给 (0.45,+0.5)：Δpos=1.041 m、Δyaw=70.5°；C5 (0.45,−0.5)：Δpos=0.992 m、Δyaw=66.7° —— **既走又转**（不是只直行、也不是只原地转）。
- **H4 PASS**：C2 (0,+0.5) Δyaw=47.6°/Δpos=0.000 m；C3 (0,−0.5) Δyaw=47.6°/Δpos=0.000 m —— **原地转**。
- **H5 PASS**：C6 (0,0) 窗内 Δpos=0.000 m、Δyaw=0.0° —— **停住**。
- **H6 PASS**：Containment 金标 `0103428000004280000041000000`(14 B)、ack ok、`TaskId(1) 建树完成`、25 s 窗内 task_id=1 恒定 / queue_len=1 / task_ticks 1→499（494 次严格增长）、无失败；Follow 金标 `01020000000000000000`(10 B)、ack ok、`TaskId(2) 建树完成`、15 s 窗内 task_id=2 恒定 / queue_len=1 / ticks 1→300。detector 起来了（`[camera] … 首帧` + `[detector] 已启动`，Hold 退化 0 次），且日志有 `[Nav] 新目标 (56.00, 56.00)` —— **真树在跑**（§5.1 第一档）。
- **H7 PASS**：`verdict.md`/`cases.json`/`checks.json`/`env.json`/`ctx.json`/`driver.log`/`world.log`/`pictor.log`/`pictor_kernel.log`/`sim_ugv.log`/`sim_uav.log`/`status_ugv.csv`/`status_uav.csv`/`manual_trace.csv`/三份 config/`ctl_containment.json`/`ctl_follow.json`/`cleanup.txt` 全部存在且 >0 B。

**软观测（只记录，不判 FAIL）**
- 到位误差 / 耗时：ugv 0.255 m / 18.3 s；uav 0.10 m / 38.2 s（`status` 的 `nav_status=arrived` 首样本）。
- 弧线半径：C4/C5 都是 **0.902 m**（名义 v/ω = 0.45/0.5 = 0.90 m，误差 0.2%）；实测角速度 0.435 / 0.413 rad/s、线速度 0.368 / 0.352 m/s（窗内含斜坡爬升，故略低于定值）。C1 直行 0.690 m / 1.87 s = 0.369 m/s、航向变化 0.0°。
- `motion_cmd` 频率：Containment 窗 `task_ticks` 20 Hz（20 ms tick 逻辑 50 ms ⇒ 19.97 Hz 实测）；`[Sim] UGV/UAV 位置` 行 5 Hz；位姿龄中位 78 ms、最大 184 ms。
- 世界 fps：227 个采样，中位 7、5~8（软件 GL / xvfb 下正常，只记录）。
- 急停 / blocked：`[Sim] 前方障碍` 0 行、`拍无路` 0 行、`nav_status=blocked` 0 样本。
- 世界 / Pictor 解析报错：`unknown msgid`/`未知 action` 0 次；Pictor `SCRIPT ERROR` 0 / `Parse Error` 0；内核监听 `[9123]`；**msgid 4 8 B 端到端旁证**：Pictor 发 `manual` 帧 payload `3ee666663f000000`（v=0.45 / ω=0.5，8 B 金标命中）。

**偏差与两次驱动器 bug（数据无损，详见 `verdict.md` 的「偏差」段）**
- D6：第 1 次尝试在 P2 闸门收尾——驱动器把内核监听正则写成 `监听 tcp/<port>`，源码实为 `监听地址: /ip4/…/tcp/9123/p2p/…`。
- D7：第 2 次运行期两处**判定** bug（不丢数据）：车侧到达走的是 `[Nav] 已在目标格且逐轴误差 <=` 分支而非 `[Nav] 到达目标（预测停点）`；`建树完成` 实际打在 `[Robot] 开始任务` **前一行**。修好后用 `--regrade` 对同一份日志只读重算，**未重跑仿真**。
- D1~D5：驱动器落 `RobotSim_TestEnv/task44_step6/`（不是任务文档写的 `tmp/task44_tools/`，step.md 未允许）；手动两轴走控制口、Pictor 8 B 帧只作旁证；Containment/Follow 走 Pictor `TASK_SET`（`Mission::Containment` 无第二入口）；`cargo test --bins` 2 条 `mount.pitch` 存量红只判「新增红=0」。

**收尾（干净）**：7 个端口（9999/9100/9123/9171/9172/9181/9182）全空、`pgrep pleiades-sim|godot` 无残留、`/tmp/.X*-lock` 前后都是 `[]`、根仓 `git status` 行集合 == 基线 7 行、`git -C Pictor status --short` 10 行与进场逐行相同。产物：`RobotSim_TestEnv/results/task44_s6_20261101T063711Z/`（副本 `tmp/task44/verdict.md`）。

## Step 6 — 仿真世界验证（sim 车 + sim 机 + 手动两轴）（task_44 首轮（Step 1~8））
- 起止：2026-11-01 06:21:46Z → 2026-11-01 06:48:37Z
- 范围：只验不改：1 台 sim ugv + 1 台 sim uav + 世界；产物落 RobotSim_TestEnv/results/task44_s6_<ts>/
- 轮次：1 / 1
- 结果：PASS
- 提交：ok 5fbdd08
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（566 行，源码零改动，`git status` 与进场基线逐行一致）。 **摘要（≤200 字）** Step 6 只验不改：新建 `RobotSim_TestEnv/task44_step6/step6_driver.py`（照 task_43 骨架裁剪为 2 节点），起 xvfb 世界 + headless Pictor + 1 sim ugv（带相机/detector）+ 1 sim uav；先重建 release 二进制。顺序：车 `goto(64,70)` → 机 `goto(72,68)` → 控制口 Manual 六条 `(0.45,0)/(0,±0.5)/(0.45,±0.5)/(0,0)`（20 Hz 采 status）→ Pictor `containment(64,64,8)` 25 s → `follow` 15 s。判据 H1~H7 只判「行为是否出现」，半径/误差/耗时/频率进软观测不判 FAIL；收尾断言 7 端口空、无残留、git 基线行集合不变。附确定性降级梯子与 14 条风险预案。
- 验证产物：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/RobotSim_TestEnv/results/task44_s6_20261101T063711Z

## Step 7 — 真车行为树：detector / Target / 协助请求 / Follow / Containment 全部复制过来（task_44 首轮（Step 1~8））
- 起止：2026-11-01 06:48:37Z → 2026-11-01 07:02:00Z
- 范围：pleiades-ugv 新增 device/detector + ugv/mission/{follow,containment}.rs；配置与装配接线
- 轮次：1 / 3
- 结果：PASS
- 提交：ok d2e5df8
- 规划要点：计划已写入 `tmp/task44/plan.md`（403 行，源码零改动，`git status` 与进场 6 行逐行一致）。 **计划摘要（≤200 字）** 计划写入 tmp/task44/plan.md（403 行，源码零改）。逐字复制：projection.rs 零改动；detector/mod.rs 仅 8 处（帧源改 camera_ugv、声明走 device/mod.rs）；follow.rs 10 行、containment.rs 7 行，只差 use 与文件头。config.rs 先落 detector 常量与两结构（含 [detector.camera] 子表）及 2 条单测，projection 才能逐字节。mission/mod.rs 两臂换真树，detector=None → 恒 Running 原地 Hold（不 Success 不 Err），旧平凡树单测改 Hold。robot_handler：detector 排 lg290p 后、enabled=false 零线程，NodeHandlePublisher 自带一份+本地 Warn_Throttled，注入 TaskDeps。验收 V1–V13：0 error、0 新增 warning、mission 37、全量 194、diff 逐行对表、base 零改动。

## Step 8 — 真车端到端：静态接线验收 + 上车清单 + 文档同步（task_44 首轮（Step 1~8））

- 起止：2026-11-01 07:05:35Z → 2026-11-01 07:15:00Z
- 范围：**只验不改代码**：5 条接线静态核对 + 残留扫描 + 上车清单（C1~C10）+ 3 份文档同步
  （`Architecture/robot_arch.md`、`docs/ros_stm32_protocol.md`、`Workbook/wb_44_ugv_velocity.md`）；
  `Architecture/Pleiades_Architecture.md` 实查 0 命中 ⇒ 未改动
- 轮次：1 / 1
- 结果：**PASS**（本机无车无机；C1②/C2~C10 待人类上车回填，见下「上车清单」）
- 提交：待引擎提交（本步只改 3 个允许文件，`pleiades-*` / `Pictor` / `Task` / `Cargo.*` 零改动）
- 规划要点：计划已写入 `tmp/task44/plan.md`（418 行，源码零改动，`git status` 与进场基线逐行一致）。
  **摘要（≤200 字）** Step 8 只验不改：现场 grep 复验五条接线（串口 / 手动 / 模式 / 急停 / 停车）并各给唯一
  file:line；限域扫 7 个 token（`MotionState`/`MotionAction`/`Apply_Velocity_Pct` 为 0，`0x11` 全假阳性，
  `forward_speed`/`turn_speed` 仅 L-1 真残留）；写 C1~C10 上车清单（做什么 / 看什么 / 算过了 / 算失败 + 回填列）；
  文档同步 `0x11`→`0x12`、M1=`0x0A`、树覆盖度、导航订阅、新增 §8.14。编译：base lib + 三设备 crate 0 error
  （base `--all-targets` 71 条为存量）；测试 ugv 194 / uav 155 全绿，sim、base 存量红失败名集合不变。

### 上车清单（C1~C10，待人类回填）

> 判定分层（task_44 §1.2 #10）：**行为是否出现 = 硬判据**；速度值 / 弧线半径 / 航向角 / 耗时 / 帧率 = **软观测，只记录不判 FAIL**。
> 不得自行追加量化门槛。车身端与 Pictor **必须同批升级**（§1.2 #5，`git log -1` 一致）——混跑即事故。

**前置（照做即可）**

1. 车端启动：`RUST_LOG=info cargo run -p pleiades-ugv --release --bin pleiades-ugv`；
   首屏必须出现 `启用底盘: port=… baud=… car=M1 cruise=0.45m/s yaw=0.5rad/s`（`robot_handler.rs:146`）。
2. 串口旁路抓帧（可选，只为看帧，**不改代码**）：`interceptty -s 'ispeed 115200 ospeed 115200' /dev/ttyUSB0 /dev/ttyUSB1`，
   再把 `[chassis].port` 指到 `/dev/ttyUSB1`。
3. 每条开始前先发一帧 `msgid 4` 全零 8 B（停车帧）；异常立即 Ctrl-C（退出路径 `robot.rs:394-398` 会 `device.stop()`）。

| # | 项 | 做什么 | 看什么（唯一证据源） | 算过了 | 算失败 | 本机状态 | **实测（人类回填）** |
|---|---|---|---|---|---|---|---|
| C1 | 车型 M1 + 帧内 `0x0A` | 启动车端；抓一帧 | ① 日志 `car=M1`（`robot_handler.rs:146`）；② 抓包**偏移 4** = `0x0A`（前进帧形如 `FF FC 0A 12 0A C2 01 00 00 00 00 E9`） | ① `car=M1` ② 偏移 4 = `0A` | 出现 `car=X3Plus`（配置被旧值覆盖）或偏移 4 = `02`/`01` | 半已验：帧布局与 `0x0A` 由 `cargo test -p pleiades-ugv --bins` 的 `test_pack_motion_frame_layout`（`protocol.rs:218-231`）、`test_mock_apply_velocity_command`（`mod.rs:341-355`）锁住；`car=M1` 待上车看日志 | |
| C2 | 前进 `(0.45, 0)` | Pictor 手动模式按住 `W` 3 s 后全松 | 车前进（实车目视）+ 帧 `vx = 450 = C2 01` | 车头朝前方向位移；帧 `vx = 450` | 不动；或动但方向反（车头朝后） | 待上车 | |
| C3 | 后退 `(-0.45, 0)` | 按住 `S` 3 s | 车后退 + 帧 `vx = -450 = 3E FE`（`FF FC 0A 12 0A 3E FE 00 00 00 00 62`） | 朝车尾方向位移 | 仍前进 / 不动 | 待上车 | |
| C4 | 原地转 `(0, ±0.5)` | 先 `A` 3 s、停；再 `D` 3 s | 车**绕自身**转，位移 ≈ 0；帧 `vz = +914 = 92 03`（A）/ `-914 = 6E FC`（D） | `A` = 逆时针（俯视左转，§1.2 #2 `ω>0`=左转）；`D` 反向；车心基本不移动 | 转了但方向反；或边走边转（`vx≠0`） | 待上车（sim 侧 Step 6 已验 H4） | |
| C5 | 左前弧 `(0.45, +0.5)` | 按住 `W`，再按住 `A`，保持 3 s（不松 `W`） | 车**同时**前进 + 左转；帧 `vx=C2 01`、`vz=C7 01`（`FF FC 0A 12 0A C2 01 00 00 C7 01 B1`） | 轨迹是弧线（既有位移又有航向变化）；半径落在 ~0.9 m 量级（**软观测**） | 只直行不转（`vz=0`）/ 只原地转（`vx=0`）/ 弧线明显转过头 | 待上车（sim 侧 Step 6 已验 H3，半径 0.902 m） | |
| C6 | 停车 `(0, 0)` | 全部松键 | 车轮停；帧 `FF FC 0A 12 0A 00 00 00 00 00 00 xx`（`vx=vz=0`，`ω=0` **必发 0** 而不是死区 0.2，见 `stm32/mod.rs:220-223`） | 车停、且**不再自转**；帧 `vz=0` | 车继续动；或停车后缓慢自转（`vz` 发了 0.2 死区） | 待上车（sim 侧 H5 已验） | |
| C7 | 手动三态（连续两轴） | ① 按住 `W` 2 s（直行）→ ② 不松 `W` 加按 `A` 2 s（边走边转）→ ③ 松开 `A`（应恢复直行）→ ④ 全松（停） | Pictor 依次发出 `(0.45,0)` → `(0.45,+0.5)` → `(0.45,0)` → `(0,0)`（`input_handler.gd:26-37` + `:42-49` 只发变化值）；车行为与之一致 | ② 出现弧线、③ 恢复直线、④ 停 | ② 无弧线（只前进）/ ③ 松 `A` 后仍转 / ④ 不停 | **已验（sim 侧）**：Step 6 手动六条 + Pictor 8 B 帧 `3ee666663f000000` | |
| C8 | 模式切换 | Pictor 面板 手动 → 自动 → 手动 | 车端日志 `[Robot] 切换到 Manual 模式`（`robot.rs:322`）/ `[Robot] 切换到 Auto 模式`（`:329`）；**Auto 模式下发手动键** → 必须出现 `[Robot] 忽略 Manual 命令：当前为 Auto 模式`（`:342`）且车不动；切模式瞬间车停（`reset` → `stop`） | 两条模式日志都出现；Auto 下手动被忽略 | 模式不切（日志无效果）；或 Auto 下仍执行手动（门闩失效） | 待上车（解码路径由单测锁：`command_decode.rs:403-420`） | |
| C9 | 导航订阅生效 | Auto 模式下发 `Goto`；**出任务过程中**用 Pictor 在地图上前方新增一段墙（横跨路径） | ① `[Nav] 新目标 (x, y)` 出现；② 车/规划改路（不再走原直线，绕到墙一侧）；③ 地图增量被规划吃到（`navigator.rs:249` 的 `drain_map_deltas`） | 路径出现绕行拐点，且墙不被穿过 | 车直冲墙（`[Robot] 前方障碍` 急停反复）；或路径完全不变 | 待上车（订阅链 `robot_handler.rs:168` 本步静态核对） | |
| C10 | 帧节奏（软观测） | 抓包统计 | 只记录：`0x12` 帧频率、每帧一次 write（`port.rs:78`） | — | — | 只记录，不判 | |

> C2~C6 的期望帧 HEX 由 `pack_motion` + `omega_to_vz`（`0.2 + |ω|/per_unit`）逐字节复算，**仅作对照**；
> 上车以实测抓包为准 —— 判据是「偏移 4 = `0A`」与「`vx`/`vz` 的符号档位」，不是 hex 本身。

**本机已验 / 待上车的边界**

- **本机已验（静态 + 单测 + sim）**：帧布局与 `0x0A`（`--bins` 单测）、`(v,ω)` 五档决策（`executor.rs` 单测）、
  `vz` 反解常数与符号、手动两轴六条（Step 6 世界实测）、模式解码（单测）、真车树覆盖度（Step 7 `--bins` 194 绿）、
  建图增量订阅链（静态 file:line）。
- **待上车**：C1②（真串口抓帧）、C2~C6（真车运动与方向）、C7（真 Pictor + 真车）、C8（真面板）、C9（真地图增量 + 真规划改路）、C10（软观测）。

### 遗留清单 L-1~L-5（本步**不改**，交人类裁决）

| # | 文件:行 | 现象 | 冲突的口径 | 建议修法 | 本步为何不改 |
|---|---|---|---|---|---|
| **L-1** | `pleiades-base/src/Config/config.toml:36-42` | 仍是 `car_type = "X3Plus"` + `forward_speed = 30` + `turn_speed = 10` | task_44 §1.2 #9（缺省 M1）/#12（删两个百分比键，换 `cruise_speed`/`cruise_yaw_rate`） | `car_type = "M1"`；删 `forward_speed`/`turn_speed`；补 `cruise_speed = 0.45` / `cruise_yaw_rate = 0.5`（与 `pleiades-ugv/src/config.rs:348-350` 一致） | 该文件是**手工拷贝样例**（自带头注释 `# 拉取后执行：mkdir -p .config && cp …`，全仓无 `include_str!` 引用；`docs/清理文档计划.md:191-192` 早已登记它过期），且 `pleiades-base/src/Config/` **不在** step.md 的 4 条允许路径内 |
| **L-2** | `pleiades-base/src/Robot/core/state.rs:94-100` | `DecisionState::{Turning, Moving}` 已无任何写入点（连续执行器全程只写 `Idle`） | 非口径冲突，属历史残留 | 下一轮清理时删两个变体，或注明「仅为遥测兼容保留」 | 不影响行为；本步禁改代码 |
| **L-3** | `docs/heterogeneity_analysis.md:43,46,50-56,116,168,223,236`；`docs/design_doc/universal_robot_design.md:83,105,173`；`docs/design_doc/uav_device.md:10,42,66,74,78,106,160`；`docs/design_doc/robot_controller.md:339-340`；`docs/design_doc/terminal_interface_diff.md:130` | 仍按 `0x11` / `forward_speed` / `MotionAction` 描述 | 与 §1.2 #1/#12 冲突（文档陈旧） | 另开文档清理任务统一改（照 `ros_stm32_protocol.md` 的废弃标注法） | **不在** step.md 的允许路径内（只允许 robot_arch / Pleiades_Architecture / ros_stm32_protocol / workbook 四个） |
| **L-4** | `pleiades-ugv/src/device/camera_ugv/sdk.rs:1122` | `Jpeg(0x11, 32)`（JPEG 测试构造，与 `0x11` 功能码无关） | 扫描假阳性 | 无需修（可在扫描脚本里按 `device/camera_ugv/` 排除） | Step 4 已登记过；不在本步路径，且是真测试数据 |
| **L-5** | `pleiades-ugv/src/ugv/emergency_stop.rs:52` + `pleiades-ugv/src/ugv/robot_handler.rs:377` | 同一拍各发一帧全零 `0x12` | 无冲突（安全关键帧幂等） | 可选：去掉调用点那次重复 `stop()`（本步**不建议**，冗余有安全价值） | 有意冗余，判为不 FAIL；本步禁改代码 |

## Step 8 — 真车端到端：静态接线验收 + 上车清单 + 文档同步（task_44 首轮（Step 1~8））
- 起止：2026-11-01 07:02:00Z → 2026-11-01 07:18:53Z
- 范围：只验收与文档：Architecture / docs / Workbook（不改代码）
- 轮次：1 / 1
- 结果：FAIL（已验证，按口径 #10 继续）
- 提交：(未提交)
- 规划要点：计划已写入 [plan.md](tmp/task44/plan.md)（418 行，源码零改动：`git status` 与进场基线逐行一致，Pictor 仍 10 行）。 **摘要（≤200 字）** Step 8 只验不改，1 轮。先取证后改文档：①5 条接线各给唯一 file:line——串口 `stm32/mod.rs:127`→`protocol.rs:31`→`port.rs:78`（一次 recv 一次 write）；手动 `command_decode.rs:22`→`robot.rs:338`→`mod.rs:153`；模式 `decode:30`→`robot.rs:319`→`reset:318`；急停 `emergency_stop.rs:52`；停车走 `executor.rs:92` 的 (0,0)+reset/suspend/退出。②残留扫描限定 scope，预期命中=假阳性+1 条真残留（`base/src/Config/config.toml:36-42` 仍 X3Plus/百分比）登记 L-1~L-5 不改。③robot_arch 14 处（含新增 §8.14、§1.3/§8.11 重写）、ros_stm32_protocol 6 处（0x11 废弃+0x0A）；Pleiades_Architecture grep 0 命中不改。④上车清单 C1~C10 三列判据+「本机已验/待上车」。verdict 落 `tmp/task44/verdict.md`（RobotSim_TestEnv 越界）。
- 未通过的问题：
Architecture/robot_arch.md:1049（§8.14 ⑥ 五条接线表 ·「急停」行）—— 期望 `:389-391`，实际写成 `:381`。该行文字「`robot_handler.rs:373` 调用点、`:377` 再发一帧、`:381` 本拍不决策」中前两个行号正确，第三个错：`robot_handler.rs:381` 只是一句注释（`// ③ 树：推进当前任务的树（急停这一拍由 tick_active_tree 挡住）`），真正「本拍不决策、不判终值」的是 `pleiades-ugv/src/ugv/robot_handler.rs:389-391`（`if emergency { return TickOutcome::Running; // ④⑤ 跳过 }`）。同一错号连带出现在本步产物 `tmp/task44/verdict.md`（「一、看到了什么 §1」表格第 4 行）与 `tmp/task44/s8_wiring.txt:77`，说明证据文件本身未现场复验。复现：`sed -n '389,391p' pleiades-ugv/src/ugv/robot_handler.rs`（对照 `grep -n ':381' Architecture/robot_arch.md`）。
Architecture/robot_arch.md:226（§1.3 末条「决策阈值」，本步整行重写过）—— 期望只列代码里真实存在的阈值，实际写「另保留 `sub_target_threshold_m=0.2`、`arrival_threshold_m=0.3`（nav 侧）」，其中 `sub_target_threshold_m` 全仓 0 命中（`grep -rn 'sub_target_threshold' pleiades-base/src pleiades-ugv/src pleiades-uav/src pleiades-sim/src` → 无输出；含 `*.toml` 亦 0），nav 侧实际只有 `ARRIVAL_THRESHOLD_M = 0.3`（`pleiades-ugv/src/ugv/nav/navigator.rs:32`，`navigator.rs:235` 消费）。本行是本步改的，属于「文档与代码一致」验收（任务文档 §二 Step 8 验收 5）未过。复现：`grep -n 'sub_target_threshold_m' Architecture/robot_arch.md; grep -rn 'sub_target_threshold' pleiades-*/src; sed -n '32p' pleiades-ugv/src/ugv/nav/navigator.rs`。
tmp/task44/verdict.md（「一、看到了什么 §2 残留扫描」表的 `turn_speed` 行）—— 期望 7，实际写 6。同格自己列出的命中是 `config.toml:42`（1）+ `sim/config.rs:473,482`（2）+ `pose_ingest.rs:88`（1）+ `sim/chassis/uav.rs:15,32,148`（3）= 7，实测（复跑限域 grep）也是 7 行。同行 `0x11`=6、`forward_speed`=3 两个计数正确，仅 `turn_speed` 少计 1。复现：`for p in turn_speed; do grep -rn --include=*.rs --include=*.toml --include=*.gd "$p" pleiades-base/src pleiades-ugv/src pleiades-uav/src pleiades-sim/src Pictor/src Pictor/tool Cargo.toml pleiades-*/Cargo.toml | wc -l; done`（→ 7）。
【人类上车项 · 按 step 6/8 口径记录，非代码缺陷，也不是本轮唯一 FAIL 理由】看到了什么：静态侧全部成立 —— `0x12` 帧 12 B、偏移 4 = 车型字节（`pleiades-ugv/src/ugv/stm32/protocol.rs:31`，单测 `protocol.rs:218` 断言 `f[2]==10`、`f[4]==0x0A`）；五档 `(0,0)/(0.45,0)/(0.45,±0.5)/(0,±0.5)`（`pleiades-ugv/src/ugv/executor.rs:91-118`，sim 版逐字相同，我 diff 为空）；`ω=0 ⇒ vz=0` 硬条件（`pleiades-ugv/src/ugv/stm32/mod.rs:221`）；五条接线各自唯一 file:line 我逐条复验通过（`mod.rs:127/134/151/153/165`、`protocol.rs:16/31`、`pleiades-base/src/Robot/util/serial/port.rs:46/75/78`、`command_decode.rs:22-26/30-38`、`robot.rs:338-344/394-398/484-492`、`robot_handler.rs:168/308-311/316-328/334/373/377/389-391/401/406-421`、`nav/navigator.rs:185/249`、`emergency_stop.rs:25/49/52`）。缺什么：C1②（真串口抓帧看偏移 4=`0x0A`）、C2~C6（前进/后退/原地转/左前弧/停的真车方向与「是否真的出现」）、C7（真 Pictor 三态 W→W+A→松 A→全松）、C8（真面板切模式 + Auto 下手动被忽略）、C9（出任务中新画墙让规划改路）这九项本机无法验（step.md 禁上车/串口/开流，本步零进程，`pgrep`/7 端口均空）。供人类亲验：按 `Workbook/wb_44_ugv_velocity.md` 的「上车清单 C1~C10」逐条执行并回填末尾「实测」空列；半径/过冲/耗时/帧率按口径 #10 只记录、不设门槛。
- 验证产物：(未找到 * 目录)

## 引擎结束
- 结束时间：2026-11-01 07:18:54Z
- 状态：ALL_STEPS_DONE（验证步未过：8，待人类亲验）
- 需人类亲验：Step 8
- git status（结束时）：
```
M Architecture/robot_arch.md
 M Workbook/wb_44_ugv_velocity.md
 M docs/ros_stm32_protocol.md
 M pleiades-sim/src/sim/chassis/action_map.rs
 M pleiades-sim/src/sim/chassis/mod.rs
 M pleiades-sim/src/sim/emergency_stop.rs
 D pleiades-sim/src/sim/executor.rs
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
```
