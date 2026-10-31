# wb_43_ugv_camera — task_43 工作记录

> Presented by KeJi
> Created Date ： 2026-10-31
> Modified Date ： 2026-10-31
> 任务：Task/task_43_ugv_camera.md（UGV 相机：真车 camera_ugv + sim ugv 相机 + follow/containment 树与验证）

## 执行引擎启动
- 开始时间：2026-10-31 06:14:44Z
- 引擎：tmp/task43_tools/execution_engine_task43.ts + .config.json（流程照搬 task_41）
- 运行标记：task_43 首轮（Step 1~6）
- 计划：Step 1/2/3/5 最多 3 轮（规划 → 实现 → 评审）；Step 4/6 **只跑 1 轮**，失败记录后继续
- 范围：真车 camera_ugv + Lua camera 移除（1）→ sim ugv 相机（2）→ 车栈 follow 树（3）→ follow 验证（4）→ 车栈 containment 树 + Pictor 协控（5）→ 4 车 containment 验证（6）

## Step 1 — 真车 ugv camera device（camera_ugv / SDK dlopen）+ Lua camera 全部移除（task_43 首轮（Step 1~6））
- 起止：2026-10-31 06:14:44Z → 2026-10-31 06:36:43Z
- 范围：pleiades-ugv 新增 camera_ugv（sdk/slot/mod）+ config/bootstrap/handler 接线 + 三 crate 删 Lua camera 注册 + 6 个脚本与 3 处文档
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task43/plan.md)（682 行，仅只读分析，未改任何源码）。 - `camera_ugv` 三文件：`sdk.rs`（11 个必需符号 `dlopen`、偏移常量化+`const _`/`Debug_Assert_Layout` 双锁、回调 `catch_unwind`/`try_lock`/零日志/8 MiB 闸门、清理跳过 `FreeCameraList`）；`slot.rs` 第 4 份（写入口改 `Try_Publish`，唯一差异入文件头）；`mod.rs` 零线程、`Open` 同步、首帧诊断只 `warn`。 - Lua：三 crate 空实现 `register_lua_caps`、删 `Info_Table` 与 5 条 Lua 测试、`device_cap.rs` 重写占位、删 6 脚本、改 3 文档。 - 验收 V0~V7 逐条判据，基线实测：ugv 96 测试、uav 155、sim 2 条既有失败（`world_link`）。

## Step 2 — sim ugv 相机接入（复用 camera_sim，配置 + 校验）（task_43 首轮（Step 1~6））
- 起止：2026-10-31 06:36:43Z → 2026-10-31 06:57:40Z
- 范围：pleiades-sim 的相机挑选 / detector 启动与 chassis 无关的查证 + 节点配置（不入库）+ 少量清理
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task43/plan.md)（395 行；本次规划只读取源码，未修改任何源码）。 **计划摘要（≤200 字）** 计划见 tmp/task43/plan.md。查证：相机分支 sensors/mod.rs:128-137、Select_Camera sim_handler.rs:377/:533、detector spawn :573-609 均无 chassis 依赖（分栈在 :615），世界侧装配通用 ⇒ 零功能改动。改动＝新建 simq_s2_ugv 配置（camera_front 挂点 0.15/0/0.25/yaw1.5708/pitch-5、detector yolov8s/cpu）＋8 处注释日志清理（:381/:536 保留以保护 Tool 的锚点）。实跑 2 轮：A 验首帧+detector spawn+推理行；B 让行人 spawn(68,64) 观测 dets≥1（不计判据）。验收 V1~V6，上轮 issues=(无)。 补充要点（供评审快速核对，不入摘要）：实测进场基线与工作树完全一致；既有 2 条 test 失败 = `world_link::parts::tests::test_build_sensor_hello` 和 `world_link::protocol::tests::test_serialize_sensor_hello_mount`（均因 `MountOut` 多出 `pitch` 键而未更新 4 键断言），已写入计划 R5，且 `world_link/` 不在允许路径、本步不改。

## Step 3 — sim ugv follow 行为树（照抄机栈，不经 D*）（task_43 首轮（Step 1~6））
- 起止：2026-10-31 06:57:40Z → 2026-10-31 07:11:16Z
- 范围：sim 车栈 Follow 真树 + TaskDeps 扩 detector/assist + Stack::new 对齐机栈 + 调用点接线
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已完成并写入 `tmp/task43/plan.md`（594 行，只读分析，源码零改动：`git diff` 哈希与进场一致）。 **计划摘要（≤200 字，可写进 workbook）** 计划见 tmp/task43/plan.md。①follow.rs 用 sed 照抄机栈那份（6 处 uav_sim→ugv_sim，793 行，diff 为空即判据）；②mission/mod.rs 加 mod follow、AssistPublisher（Step 5 移入 containment.rs），TaskDeps 加 detector/assist，Follow 换真树、None→原地 Hold；③Stack 加两字段两参数；④sim_handler 把 publisher 提到 match 外两栈共用＋第二 impl；⑤control.rs 文案「车/机通用」。两处 allow(dead_code) 经 rustc 实测必需。验收 V1~V10：0 error、0 新增 warning、follow 34、ugv_sim 92、全量 466+2 既有红、nav/goto 哈希不变。上轮 (无)。 补充要点（供评审快速核对）：基线实测 454 测试/452 过（2 条 `world_link` 旧红）、`--bins follow` 20 绿、`--bins ugv_sim` 78 绿、warning 4 行（camera:92/:168、sim_handler:291×2）；关键不变式 `diff <(sed 's/uav_sim/ugv_sim/g' uav…) ugv…` 应为空（`goto.rs` 两栈拷贝的既有惯例）；assist 的落点冲突与「Step 5 勿重复加 impl / 删两处 allow」已写入 R1、R2。

## Step 4 — sim ugv follow 验证（单车）（task_43 首轮（Step 1~6））
- 起止：2026-10-31 07:18:52Z → 2026-10-31 07:31:05Z
- 范围：**只验不改**（世界 + 1 台 sim ugv（`camera_front` + `[detector] enabled`）+ 1 个行人；控制口 9152 注入 `{"cmd":"follow"}`、`{"cmd":"status"}` 采样 10 Hz）
- 轮次：1 / 1（口径 #13：无论 PASS/FAIL 只跑一轮；补齐口径「只验不改」→ 本轮零 Rust / 零世界 / 零 Pictor 改动）
- 结果：**PASS**
- 规划要点（≤200 字）：计划见 `tmp/task43/plan.md`（543 行，只读分析）。①硬件/单测零改动，产物全在 `RobotSim_TestEnv/`（`simq_s4_ugv_cam` 实例 + `task43_step4` 两脚本）与 `results/task43_s4_<ts>/`；②场景几何把行人压进相机接受窗（`pitch=-2.0` ⇒ `d < 0.25/tan2° ≈ 7.16 m`），六段 82 s 剧本（acq/walk1/stand1/walk2/escape/lost）；③硬判据只判行为（H1 人车同动同停、H2 跟丢停住不追旧点、H3 无 panic、H4 证据落盘），数值全进软观测（口径 #12）；④`--regrade` 只读日志重算，不重跑。
- 实跑记录（1/1 轮，产物 `/vepfs-mlp2/c20250205/240804016/Workspace/Orion/RobotSim_TestEnv/results/task43_s4_20261031T072644Z/`）：
  - 环境：HEAD `8253a099…`；根仓 status 79 行 = 基线；Pictor 0 行；`cargo check -p pleiades-sim --all-targets` 0 error、31 warning 与 Step 3 `check_post.log` **逐行相同**；`cargo build -p pleiades-sim --release` 0 error；`cargo test -p pleiades-sim --bins follow` 34 passed / 0 failed。
  - 时间线（相对进程启动）：世界监听 9999 `+0.8 s` → 车辆 spawn `+1.0 s` → `[camera] slot=camera_front 首帧` `+0.8 s` → `[detector] 已启动` `+0.0 s` → assembled `+0.0 s` → 行人 ready `+0.4 s`；follow 注入回包 `{"cmd":"follow","ok":true}`（t_wall=1793431607.691）；行人六段 82.0 s，`[ped] done distance=35.6m`，spawn 未被回退（(69,64)）；收尾 SIGTERM 后无残留进程、9999/9152 空闲、`/tmp/.X*-lock` 0 个。
  - **H1 人动→车动 / 人停→车停：PASS** — walk1 车 dx=+8.69 m（人 +9.44 m、采纳帧 33）、walk2 车 dx=+5.14 m（人 +7.25 m、采纳帧 25），两窗均出现 `next=Some(...) → speed=40` 的 Advance 拍（64 / 40 拍）；stand1 内车先动 10 拍，人停后 `+2.11 s` 停住，其后 9.92 s（窗内 82%）Advance=0、dx=+0.21 m（< 一格 0.5 m）。任务未被弹掉（完成/失败行 0）。
  - **H2 跟丢（`age > 800 ms` / 人出画）→ 停住且不追旧点：PASS** — 最后采纳后 25.08 s 内 0 采纳、37 条 person「拒答（框底边不在画面下半部 v=232..239 需 > cy=240）」；最后采纳后 1.5 s 起 Advance=0；最后一个 Advance 拍在结束前 24.6 s，其后 105 拍全 Hold、车净位移 dx=+0.135 m（不追旧点）；`task_id` 恒定、任务完成/失败/建树失败 0 行。
  - **H3 无 panic：PASS** — sim/world/ped/driver 日志 panic 指纹 0、world 无 SCRIPT/Parse ERROR、收尾前两进程存活。
  - **H4 证据落盘：PASS** — `verdict.md`/`cases.json`/`sim.log`(97 KB)/`sim_stdout.log`/`world.log`/`ped.log`/`ped_trace.csv`/`status_trace.csv`/`detector_trace.csv`/`exec_trace.csv`/`ctl_follow.json`/`configs/node.toml` 全部存在（`sim_stdout.log` 合法 0 B：tracing 只写文件）。
  - 软观测（只记录、不判 FAIL）：距离带 2.5~4.0 m 内占比 acq 25.0% / walk1 48.5% / stand1 23.8% / walk2 28.0%（每段 min~max：acq 3.06~5.24、walk1 2.44~5.00、stand1 1.95~3.53、walk2 1.95~5.01、escape 4.00~7.00）；过冲/振荡 Advance↔Hold 切换 10 次、单拍最大位移 0.16 m、`dx_stop`=0.29 m、`dx_lost`=0.45 m；`age_ms`/检测频率：采纳行间隔中位 597 ms / 最大 629 ms、推理中位 496.5 ms、有效 1.67 Hz（142 帧 / 100 采纳 / 137 person 行）；绕小圈：各段 `净位移/路径`=0.997~1.000、`Σ|Δyaw|` 0~31.6°（无打转）；急停 `[Sim] 前方障碍` 0 次；耗时/时基：剧本 82.0 s（计划 82）、`clock_k=0.9998`、截图 0/4（`import` 打不开 xvfb 的 X server，属尽力项，已修 Shooter 的 DISPLAY/XAUTHORITY 推断）。
  - 收尾：无残留 godot/pleiades-sim/Xvfb；端口 9999/9152/9142 全空；`git status -uall` 与 `tmp/task43/git_baseline.txt` **逐行相同**（79 行）；Pictor 0 行；`git log` 仍 `8253a099…`；未 `git add`/`commit`、未 `cargo fmt`。
  - 偏差（详见 `tmp/task43/impl.md`）：①首轮驱动在等 `[ws] vehicle spawned peer='ws_ugv_s43'` 处超时（该行打印的是握手 peer_id，不是 `peer_name`），未注入 follow、未起行人 ⇒ 非行为轮，现场留档 `tmp/task43/step4/attempt1_driver_bug/`；②H1a/H2 的窗口边界按「最后一个 Advance 拍」「最后采纳 + 1.5 s（3 帧 CPU 推理）」判，计划原文的固定 2 s 起算会把已出现的停车/停住判成 FAIL（口径 #12 禁止以数值卡行为），原文数字仍逐字保留在 verdict 的 detail 里；③`sim_stdout.log` 只断言存在；④验证脚本放 `RobotSim_TestEnv/task43_step4/`（step.md 允许路径优先于任务文档 §六的 `tmp/task43_tools/`）。

## Step 4 — sim ugv follow 验证（单车）（task_43 首轮（Step 1~6））
- 起止：2026-10-31 07:11:16Z → 2026-10-31 07:36:39Z
- 范围：只验不改：1 台 sim ugv（带相机+detector）+ 世界 + 行人；产物落 RobotSim_TestEnv/results/task43_s4_<ts>/
- 轮次：1 / 1
- 结果：PASS
- 规划要点：计划已写入 `tmp/task43/plan.md`（543 行，源码零改动：git status 与进场基线逐行一致，Pictor 0 行）。 **要点披露**：Step 2 的注释说「pitch −2.0 已实测覆盖 2~7 m」，但 `run_b/sim.log` 里实际生效的是 **−5.0**，实测在 4 m 处 person 被「框底边不在画面下半部」拒答。用针孔几何反算（f≈343，h=0.25）`pitch=−5,d=4 → v≈231.5`，与实测 232/233 吻合；`−2.0` 的接受窗是 `d < 7.16 m`。故沿用 `−2.0`，把场景几何全压在 2.5~7.1 m，不改任何阈值。 --- **计划摘要（≤200 字，写入 workbook）** 计划见 tmp/task43/plan.md（543 行，只读分析，源码零改动）。只验不改、只跑 1 轮：新建 `RobotSim_TestEnv/task43_step4/{step4_driver.py,ped_phases.py}` + 实例 `simq_s4_ugv_cam`（pitch −2.0、forward 40=0.6 m/s、控制口 9152），起世界+1 车+行人六段剧本（站8→走20→站12→走15→逃15@1.26→站12，共 82 s），控制口注入 `{"cmd":"follow"}`、10 Hz 采 `status`。硬判据只判「行为出现」：日志有/无 Advance 拍（`next=Some`+`speed>0`）与 `next=None`，位移/带宽/帧率全归软观测（口径 #12）。先 `--dry-run` 再实跑，`--regrade` 只读日志重算；产物 results/task43_s4_<ts>/verdict.md。上轮 issues=(无)。
- 验证产物：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/RobotSim_TestEnv/results/task43_s4_20261031T072644Z

## Step 5 — sim ugv containment 行为树 + Pictor 协控（排除请求者 / 3 点环）（task_43 首轮（Step 1~6））
- 起止：2026-10-31 07:36:39Z → 2026-10-31 07:49:01Z
- 范围：sim 车栈 Containment 真树 + 自带 AssistPublisher trait + publisher 双 impl；Pictor 独立仓改 coop_controller
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task43/plan.md)（385 行）。只读核对：根仓 `git status` 79 行 = 进场基线、Pictor 0 行，**源码零改动**。 **计划摘要（198 字，可入 workbook）** 计划见 [plan.md](tmp/task43/plan.md)。Rust：照抄 containment.rs（trait 搬入、删 allow），Containment 换真树、None→Hold；sim_handler 只改 import（勿重复 impl）。Pictor：排除请求者→N=3=θ2πi/3+3 点单测。验收 containment 33、482+2 旧红、Pictor PASS、base 空。 要点披露（供评审快速核对）：① `sim_handler.rs:434` 的第二份 `impl UgvAssistPublisher` **Step 3 已交付**，本步只改 `:105` 一行 import 路径，重复加会 `E0119`（V8 用 `grep -c == 1` 钉住）；② `Node` trait 无 `as_any` 且 base 零改动 ⇒「树形（外层必须 `ReactiveFallback`）」只能行为等价判定，T16 用「界内→出界→回界内」三拍把 `ReactiveFallback` 与记忆式 `Fallback` 区分开；③ Pictor 的 `requester_id` 与 `VehicleRegistry` 键**同为 sysid hex**（`task_41` 实跑日志 `pictor.log:31-35` 实证），`cars.erase(str(requester_id))` 直接命中、无需 hex↔base58 转换；④ 已用探测脚本 `tmp/task43/scratch/probe_coop.gd` 实测 `Coop.new()` + 注入 `_registry`/`_bus`（不 add_child）在 `-s` 模式下可用，改前输出 4 条指派含请求者（正是待修

## Step 6 — sim ugv containment 验证（4 车：1 带相机 + 3 待命 + Pictor 协控）（task_43 首轮（Step 1~6））
- 起止：2026-10-31 08:08:54Z → 2026-10-31 08:14:30Z
- 范围：只验不改：4 台 sim ugv（c1 带 `camera_front`+detector 跑 Containment / c2~c4 待命无相机）+ 世界 + Pictor 镜像（订阅含 assist）+ 行人；产物落 `RobotSim_TestEnv/results/task43_s6_<UTC>/`
- 轮次：1 / 1
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task43/plan.md)（521 行，只读分析、源码零改动）。**要点披露**：① 判据分两层（口径 #12）：硬判据只判「行为是否出现」，软观测 6 项只记录；**只跑 1 轮**（口径 #13）。② 场景几何按源码算：正方形 `(74,64)±6`（避开 c2~c4 的 spawn 环）、人 spawn `(74,59)`（界内距南边线 1 m，落在相机竖直接受窗 7.16 m 与水平半 FOV 43° 的交集里）、出界走**正北** 14 m（避开环上 3 点）、legs `74,59@70:0.50;74,73@25:1.26`。③ 环 R=3.0/N=3（`coop_controller.gd` 排除请求者后 N=3）。④ 注入走 Pictor WS 的 `containment` 子命令（本地控制口只认 `{x,y}`，不能发 type=3）。⑤ 先起 Pictor 再起 sim、注入前留 10 s（task_41 的「assist 静默失败」教训）。
- 实测（该轮）：19 条预检全过；世界/Pictor/4 sim 起来后 c1 `[camera] 首帧` + `[detector] 已启动` + `[Sim] 车栈执行器 = 连续档位` 全中；注入 `0103429400004280000040c00000`（14 B、ack_ok）。
  - **H1 PASS**：`[Coop] 协助请求` **9 条**、`requester` == c1 的 vid、`unknown msgid` **0**、c1 侧广播失败 0；D2 对照 c1 `gossipsub 发布成功 assist` 9 / `pose` 1755 / 失败 0。
  - **H2 PASS**：首个请求块 3 条 `[Coop] 指派`（不含 c1），c2/c3/c4 各 `开始任务 …Goto`、40 s 窗内净位移 **9.74 / 13.25 / 10.31 m**、Advance 84/118/94 拍。
  - **H3 PASS**：c1 `task_id` 恒 0、`queue_len` 恒 1、`task_ticks` 0→3172、`开始任务 Containment`=1 / `Goto`=0 / 完成失败建树失败=0；人出界后 25 s 内净位移 2.01 m、73 拍 Advance（回巡逻）。
  - **H4 PASS**：13 份日志 panic 指纹 0、收尾前 world/pictor/sim×4 全存活、4 车 `任务失败` 0、c1 `task_ticks` 末 20 s 仍增长。
  - 软观测（只记录）：detector 284 帧 / 1.63 Hz / 推理中位 509 ms；person 行 145、采纳 101、拒答 44（**全部**是「框底边不在画面下半部 v=232~240」= 接受窗边界，与几何预测一致）；`位姿龄` 中位 73 ms；世界 fps 中位 **6**（5~8，5 进程同跑的已知限制）；急停/无路/blocked 全 0；截图 **4/4 成功**（另留一张 3× 裁切 `shots/zoom_40s_ring.png`，能看到人站在路上、4 车围成环）。
- 偏差（详见 `tmp/task43/impl.md` §4）：①首次启动是**驱动自身 bug**（`{p.name: …} for p in CARS`，CARS 是 str）——未起 sim/未注入/未起行人，按口径 #13 **不算行为轮**，现场留档 `results/task43_s6_…_attempt1_driver_bug/`；②`P0.6` 对内核 `.so` 的 `strings|grep pleiades/robot/assist_request` 实测 0（字面量被编译器并进后缀池），改为「`assist_request` 子串 + 源码转发分支 + `.so` 晚于 base 源码」三条件；③`[Coop]` 行无时间戳 ⇒ 100 ms 泵打戳，H2 窗口取「泵戳 / 车侧 Goto 首帧」更早者（两者差 0.005 s）；④H3 的 `task_id/queue_len/task_ticks` 三条只在**任务运行窗**内检查（注入前 10 s 稳定窗不在其内），并把「注入前 100 样本 + 投递窗 1 样本（0.14 s）」逐条写进 detail；⑤金标 hex 大小写（`pictor_ws_ctl.py` 输出小写）导致 `mission_golden` 闸门行按旧代码报 FAIL，字节一致，比较已改 `.lower()`；⑥`--regrade` 用了 3 次（只读重算、未重跑仿真）；⑦`driver.log` 用同一次运行的控制台原件补齐。
- 验证产物：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/RobotSim_TestEnv/results/task43_s6_20261031T080937Z

## Step 6 — sim ugv containment 验证（4 车：1 带相机 + 3 待命 + Pictor 协控）（task_43 首轮（Step 1~6））
- 起止：2026-10-31 07:49:01Z → 2026-10-31 08:18:48Z
- 范围：只验不改：4 台 sim ugv + 世界 + Pictor；产物落 RobotSim_TestEnv/results/task43_s6_<ts>/
- 轮次：1 / 1
- 结果：PASS
- 规划要点：计划已写入 [tmp/task43/plan.md](tmp/task43/plan.md)（521 行；本次分析期间源码零改动：`git status` 仍 80 行、Pictor 仍 2 行）。 **计划摘要（≤200 字，可写入 workbook）** 计划见 tmp/task43/plan.md（521 行，只读分析，零源码改动）。新增 RobotSim_TestEnv/task43_step6/{step6_driver.py, ped_route.py}＋4 实例（9171-9174/9181-9184）＋重同步 pictor_ws（9123、订阅 assist）。几何已按源码算定：方框 (74,64)×6、人 (74,59) 离巡逻线 1 m、出界正北 1.26 m/s；containment 只能经 Pictor ctl 注入（本地控制口只发 Goto）。硬判据 H1~H4 只判行为，数值全归 S1~S6 软观测；release 二进制重建 + 镜像重同步是两处必做前置。上轮 issues=(无)。 几点须知（不入摘要）： - **两处进场即过期**：`target/release/pleiades-sim` 早于 Step 5 的源码改动；`RobotSim_TestEnv/pictor_ws` 是 task_41 旧镜像（无「排除请求者」）——计划把两者都定成阻断性前置（P0.5 / P0.11）。 - **本地控制口不能发 type=3**：`control.rs:318-332` 的 `Param_Missions` 只造 `Goto`，所以 §八 步骤 3 的 `{"cmd":"queue","missions":[{"type":3,…}]}` 走不通，必须用 `pictor_ws_ctl.py containment`（金标 `0103429400004280000040C00000`）。 - **相机几何是关键风险点**：mount `yaw=90°` ⇒ 光轴 = 车头方向（`Vehicle.gd:183-184`+`WorldTran
- 验证产物：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/RobotSim_TestEnv/results/task43_s6_20261031T080937Z

## 引擎结束
- 结束时间：2026-10-31 08:18:48Z
- 状态：ALL_STEPS_DONE
- 需人类亲验：无
- git status（结束时）：
```
 M .github/instructions.md
 M Architecture/Pleiades_Architecture.md
 M Tool/blender/models/build_models.py
 M Tool/blender/models/export_all.sh
 D Tool/execution_engine_human_model.config.json
 D Tool/execution_engine_human_model.ts
 D Tool/execution_engine_pedestrian.config.json
 D Tool/execution_engine_pedestrian.ts
 D Tool/execution_engine_task30.ts
 D Tool/execution_engine_task31.config.json
 D Tool/execution_engine_task31.ts
 D Tool/execution_engine_task32.config.json
 D Tool/execution_engine_task32.ts
 D Tool/execution_engine_task33.config.json
 D Tool/execution_engine_task33.ts
 D Tool/execution_engine_task34.config.json
 D Tool/execution_engine_task34.ts
 D Tool/execution_engine_task36.config.json
 D Tool/execution_engine_task36.ts
 D Tool/execution_engine_task37.config.json
 D Tool/execution_engine_task37.ts
 D Tool/execution_engine_task38.config.json
 D Tool/execution_engine_task38.ts
 D Tool/execution_engine_task39.config.json
 D Tool/execution_engine_task39.ts
 D Tool/execution_engine_task41.config.json
 D Tool/execution_engine_task41.ts
 D Tool/execution_engine_walking_person.config.json
 D Tool/execution_engine_walking_person.ts
 D Tool/sdk_plot_test.py
 D Tool/sdk_test.py
 D Tool/sdk_tmini_test.py
 D Tool/sim_task39_step3_test.py
 D Tool/sim_task41_containment_test.py
 D Tool/sim_task41_coop_test.py
 D Tool/sim_task41_pedestrian.py
 D Tool/test_lidar_raw.py
 D Tool/ydlidar.py
 M Workbook/wb_41_containment.md
 M docs/capabilities.md
 M orion-simulation-world/COMPONENTS.md
 M orion-simulation-world/assets/README.md
D  orion-simulation-world/assets/chassis/ugv_chassis.glb
D  orion-simulation-world/assets/chassis/ugv_chassis.glb.import
 M orion-simulation-world/src/scenes/Vehicle.tscn
 M orion-simulation-world/src/tests/check_models.gd
 M orion-simulation-world/src/tests/check_sensor_models.gd
 M pleiades-sim/src/sim/control.rs
 M pleiades-sim/src/sim/device_cap.rs
 M pleiades-sim/src/sim/sensors/mod.rs
 M pleiades-sim/src/sim/sim_handler.rs
 M pleiades-sim/src/sim/ugv_sim/mission/mod.rs
 M pleiades-sim/src/sim/ugv_sim/mod.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/device/camera_pc/mod.rs
 M pleiades-uav/src/device/camera_uav/mod.rs
 M pleiades-ugv/src/bootstrap.rs
 M pleiades-ugv/src/config.rs
 M pleiades-ugv/src/device/mod.rs
 M pleiades-ugv/src/ugv/robot_handler.rs
 D programs/user/camera_photo.lua
 D programs/user/test_camera.lua
 D programs/user/yolo_test_uav.lua
 D programs/user/yolo_test_ugv.lua
 D programs/user/yolo_uav.lua
 D programs/user/yolo_ugv.lua
?? Task/task_42_ugv_model.md
?? Task/task_43_ugv_camera.md
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_42_ugv_model.md
?? Workbook/wb_43_ugv_camera.md
?? orion-simulation-world/assets/chassis/ugv_m1.glb
?? orion-simulation-world/assets/chassis/ugv_m1.glb.import
?? pleiades-sim/src/sim/ugv_sim/mission/containment.rs
?? pleiades-sim/src/sim/ugv_sim/mission/follow.rs
?? pleiades-ugv/src/device/camera_ugv/mod.rs
?? pleiades-ugv/src/device/camera_ugv/sdk.rs
?? pleiades-ugv/src/device/camera_ugv/slot.rs
```
