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

## Step 2 — 真机 Follow：把平凡树换成真树（task_45 首轮（Step 1~3 + 邮件））
- 起止：2026-11-03 03:07:53Z → 2026-11-03 03:14:26Z
- 范围：pleiades-uav 的任务层与装配（follow 树新建 + 分派 + detector 接线）
- 轮次：1 / 3
- 结果：PASS
- 提交：ok c5c6214
- 规划要点：计划已写入 `tmp/task45/plan.md`（337 行；源码零改动，`git status` 与进场基线逐行一致）。 **摘要（<200 字）** 计划见 tmp/task45/plan.md：只动 3 个文件。① 复制真车 follow.rs→uav/mission/follow.rs，仅改 4 行（日期/文件头/use 指 crate::uav::robot_handler::detector）。② mission/mod.rs：TaskDeps 加 detector；Follow 有 detector→follow::build，无 detector→Action 恒 Running 且两字段 None；Containment 不改；Follow 用例按 §一#8 改为 Hold。③ robot_handler.rs 仅加 detector: d.detector.clone() 与注释，on_tick 不动。detector/、config.rs、executor 零改动。验收全静态：check、test --bins --no-run、diff 仅 3 hunk、Success 仍 1 处、#[path] 仍 1 处。

## Step 3 — sim 验证：ugv + uav 的 Follow / Containment（task_45 首轮（Step 1~3 + 邮件））
- 起止：2026-11-03 03:14:26Z → 2026-11-03 04:08:22Z
- 范围：只验不改：sim ugv / sim uav 各跑 Follow 与 Containment
- 轮次：1 / 1
- 结果：FAIL（已验证，按口径 #10 继续）
- 提交：(未提交)
- 规划要点：计划已经写好：[tmp/task45/plan.md](tmp/task45/plan.md)（464 行；源码零改动，`git status` 与进场基线逐行一致，仅新增该 md）。 **计划摘要（<200 字，可写入 workbook）** 计划见 [plan.md](tmp/task45/plan.md)：只新增夹具脚本 `RobotSim_TestEnv/task45_step3/step3_driver.py`（母本 task44 驱动）—— 两份 config 都加相机和 detector，`Run_Round` 换成 C1 到 C4 四组 case，判据换成现象级。先运行 `cargo build -p pleiades-sim --release`（现有二进制早于 Step1/2）。环境为：一台 ugv(64,64)、一台 uav(66,64,2.5) 加行人(69,64)（分别相距 5 m、3 m）。ugv 相机 pitch 改为 0°（-2° 时 5 m 处脚仅低中线 6 px，会被 NotLowerHalf 拒）；机臂 -15° 不动。只判现象：进队、靠近、next=None 走点、停住、四角顺时针、切跟随、回巡逻；数值全部作为软观测。无上轮 issues。 两个关键判断已在计划里给出算式和依据：① 不改 pitch，地面车在 5 m 处会被投影的 `NotLowerHalf` 整条拒绝（Follow 起不来）；② uav 的「停在 1.2 m」在 2.5 m 巡航高度与机头 -15° 下几何上到不了，按口径 #10 记为数值观测而非 FAIL。
- 未通过的问题：
C2 ugv-Containment 硬判据失败（日志报错 + 巡逻/切跟随/回巡逻现象未出现）：RobotSim_TestEnv/results/task45_s3_20261103T033606Z/sim_ugv.log:6924 记录 [Sim] 任务 TaskId(1) 失败，原因码 1（03:39:37.990，距 03:38:38.041 开始任务 59.95 s）。期望：四角顺时针巡逻 (56,56)→(72,56)→(72,72)→(56,72)，人进框后切 Follow、出框后回巡逻；实际：nav_events.csv:2 只有 (56,56) 一个角（:3 的 (48,76) 是收尾 goto，不在巡逻序里），patrol n_corners=1，switch_follow/return_patrol 均未出现；cases.json 记 C2 errors.task_fail=4，其中 sim_ugv.log:20470-20471 两行属收尾 goto(48,76)（C2 窗口尾 1793677834.5 把 PARK_UGV 也包进来了，判据应为 2 行）。复现：cd RobotSim_TestEnv/results/task45_s3_20261103T033606Z && grep -n '任务 TaskId(1) 失败' sim_ugv.log && cat nav_events.csv
C3 uav-Follow 硬判据失败（机完全不动）：status_uav.csv:19972-21378（C3 窗口 t=1793678008.886~1793678081.042，1407 个样本）内 x,y,yaw 的唯一值只有 66.0000,64.0000,0.0000（72 s 零位移）；exec_trace.csv:10430-10747 共 318 行全为 next=None / v=0（v≠0 的行数 0）；detector 采纳 0 条（行人在 ped_trace.csv:10796 停在 (78.321,57.756)，距 uav 13.81 m > max_range_m=12）。期望：机朝人靠近并出现 point_drive；实际：Follow 任务已进队但整窗完全静止。复现：cd RobotSim_TestEnv/results/task45_s3_20261103T033606Z && awk -F, '$1>=1793678008.886 && $1<=1793678081.042 {print $5,$6,$8}' status_uav.csv | sort -u && grep '^uav,' exec_trace.csv | awk -F, '$2>=1793678008.886 && $2<=1793678081.042 && $7!=0' | wc -l
C4 uav-Containment：巡逻现象看到了（nav_events.csv:4-11 四角全到、顺序 (56,56)→(72,56)→(72,72)→(56,72)，03:58:43.68 重新武装 (56,56)），但 switch_follow / return_patrol 未出现：assist_events.csv 只有表头（assist 0 条），exec_trace.csv 无 next 由 Some→None 且 v≠0 连续 ≥3 行的形态，行人在 ped_trace.csv:10796 停在 (78.321,57.756)（|x−64|=14.32 > BOX_HALF=8.0），从未进框，出框/回巡逻更无从发生。期望：人进方框后切 Follow、出框后新增 [Nav] 新目标 (56.00,56.00)；实际：两现象均未出现。复现：cd RobotSim_TestEnv/results/task45_s3_20261103T033606Z && cat nav_events.csv && cat assist_events.csv && grep -c '78.321,57.756' ped_trace.csv
夹具/环境导致的关键现象未真正测到（供人类亲验；按 Step 3 口径仍判 FAIL）：行人在 C2 开始前基本不再响应 walk_to —— ped_trace.csv:915（t=1793677074.552）到 :10796（t=1793678065.218）共 9882 行位置恒为 (76.535,57.478)，同期 world_time 96.0→1086.7 s（pose 帧仍在流，不是接收线程死了）；03:54:25–03:55:25 只挪到 (78.321,57.756) 后又停住到结束；driver.log:68 / :77 / :88 三次 walk_to 全部 到位=False（d=6.556 / 8.886 / 2.334）。直接后果：C2 的『人进框→切跟随』、C3 的换到机头前 4.5 m、C4 的『人进框→切跟随/出框→回巡逻』都没真正发生。复现：cd RobotSim_TestEnv/results/task45_s3_20261103T033606Z && sed -n '915p;10796p' ped_trace.csv && grep -n '到位=False' driver.log
驱动缺陷（C4 记录靠外部脚本重建）：driver.log:93 [run] 异常：KeyError: 'goal'（_new_goals() 返回的 nav 事件没有 goal 键），C4 的 case 字典未落进 ctx.json，由 tmp/task45/reconstruct_c4.py 事后重建（cases.json:711 reconstructed=true；verdict.md:155 注明）。期望：四组 case 均由驱动器正常落盘；实际：C4 的窗口边界/注入记录由脚本写回。判据本身仍走同一套日志重算、patrol 4/4 我已按 sim_uav.log 与 nav_events.csv 复核，故不改本轮结论，但『单轮只验』的记录完整性有缺口。复现：cd RobotSim_TestEnv/results/task45_s3_20261103T033606Z && grep -n KeyError driver.log && grep -n reconstructed cases.json | head -2
驱动判据与 plan §3.5-6 不一致 + 标记语义错：RobotSim_TestEnv/task45_step3/step3_driver.py:2718 的 switch_follow 只 AND 了 (a) next 形态与 (b) 距离缩减，plan 要求 (a)∧(b)∧(c)（assist_request 计数增加）；:2725 的 assist_observable=bool(assist_all) 实为『有没有 assist 事件』而非『该日志行能不能被观测』——D2 已给 sim 实例开 pleiades_base::network::command_handler=debug，sim_ugv.log 里 pose 13701 条 / map 182 条 gossipsub 发布行说明 assist 行本来可见，而 cases.json 的 C2/C4 记 assist_observable=false 会误导。本轮 (a) 为假，故不影响 PASS/FAIL。复现：sed -n '2712,2725p' RobotSim_TestEnv/task45_step3/step3_driver.py；grep -c 'gossipsub 发布成功' RobotSim_TestEnv/results/task45_s3_20261103T033606Z/sim_ugv.log
规范（死代码）：RobotSim_TestEnv/task45_step3/step3_driver.py 里 7 个常量定义后零引用 —— STEP3_DIR:81、T_APPROACH:125、GOLD_CONT_LEN:143、GOLD_FOLLOW:144、GOLD_FOLLOW_LEN:145、DET_PERSON_RE:426、CAMERA_SLOT_PAT:460。期望：无用常量删除或真正使用；实际：各仅 1 次出现（定义处）。复现：for n in STEP3_DIR T_APPROACH GOLD_CONT_LEN GOLD_FOLLOW GOLD_FOLLOW_LEN DET_PERSON_RE CAMERA_SLOT_PAT; do grep -c $n RobotSim_TestEnv/task45_step3/step3_driver.py; done（每个都返回 1）。另：cargo check 的 field origin never read 警告（pleiades-sim/src/sim/sim_handler.rs:282）在 ddde6bb 已存在，非本步引入。
单测红（只记录、不单独判 FAIL；均非 task_45 引入，涉及的 6 个文件都不在 ddde6bb..HEAD 改动清单里）：① 用例假设旧结构——pleiades-sim/src/world_link/parts.rs:132、protocol.rs:365 断言 mount 恰好 4 键 [x,y,yaw,z]，而生产协议自 task_37（12f9786 透传 mount.pitch）起是 5 键（多 pitch）；复现 cargo test -p pleiades-sim --bins world_link::。② 用例假设旧默认值——pleiades-ugv/src/config.rs:772、device/detector/projection.rs:628 断言默认 70.0/0.20/0.045/-15.0，实际 ugv 真车标定默认值 47.8/0.1/0.135/1.7（84134e8 改默认值时测试没同步）；复现 cargo test -p pleiades-ugv --bins config::tests::test_ensure_ugv_section_fills_detector_camera。③ 环境/工作目录假设——pleiades-base/src/VM/engine.rs:64 读 programs/user/hello.lua，cargo test 的 cwd 是 crate 目录故 NotFound（该文件在仓库根确实存在）；复现 cargo test -p pleiades-base --lib vm::engine::tests::test_load_and_execute_hello_lua。④ 真 bug（非本步引入）——pleiades-base/src/VM/engine.rs:55 test_sandbox_os_blocked 里 os 仍可访问（LuaContext::new 有意保留 os 供 clock/date），与测试期望冲突；engine.rs 自 task_23（c45c6d4）未变。⑤ 抖动——pleiades-sim/src/sim/detector/mod.rs:919 test_no_target_when_rejected 首次全量跑失败（460 passed/3 failed），单独跑与随后 2 次全量跑均通过（461 passed/2 failed），属异步等待抖动，非确定性 bug。
现场记录与日志不一致（非阻塞，已由 --regrade 修正）：driver.log:59 / :65 / :84 / :90 四处写『建树=False』、:66 写
- 验证产物：/vepfs-mlp2/c20250205/240804016/Workspace/Orion/RobotSim_TestEnv/results/task45_s3_20261103T033606Z

## Step 4 — 邮件通知（引擎执行）（task_45 首轮（Step 1~3 + 邮件））
- 起止：2026-11-03 04:08:22Z → 2026-11-03 04:08:26Z
- 结果：邮件已发送
- 摘要：状态 ALL_STEPS_DONE；Step 3 未通过

## 引擎结束
- 结束时间：2026-11-03 04:08:26Z
- 状态：ALL_STEPS_DONE（验证步未过：3，待人类亲验）
- 需人类亲验：Step 3
- 邮件：已发送
- git status（结束时）：
```
M Workbook/wb_45_follow_logic.md
?? Task/task_45_follow_logic.md
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
```

## 收尾后：远端同步 + Step 3 复盘（2026-11-03 晚）

### 拉进来的两个远端提交（`Pleiades-Orion`/LYK-ce）
- `5f183ae` 机栈 Follow 距离独立：**机栈 `D_FAR 3.6` / `D_NEAR 2.6`**（真机 + sim 机），车栈仍 `2.2/1.2`。依据：UAV 2.5 m 高 + 下压 15° + 垂直 70° ⇒ 画面下边缘只到地面 `2.5/tan(50°) ≈ 2.10 m`，车栈的 1.2 m 对机栈物理不可达（实测会飞到水平 0.49 m 才被迫停）。
- `42b0539` 夹具记录（任务文档 §三）：`[sim_world.sensor.mount].pitch` 写进了 `params` 段 → 世界 `Vehicle.AttachSensor` 只读 `mount.pitch`，静默忽略 → 物理光轴 0°、投影按 −15° 算，真值 5.00 m 被算成 **2.90 m（−42%）**；移到 `mount` 后 4.88 m（−2.4%）。

### 本地随之同步（`cfcaa82`）
- `Task/task_45_follow_logic.md` §一 第 3 条：改为「除**距离常数**外只差 `use` 与文件头」，消除与口径 #4 的自相矛盾。
- `Architecture/robot_arch.md`：新增 **§8.15**（Follow 直接跟人的病根/改法/两套距离/已知限制）；§8.9 参数、§8.10 标题、D-S7 行打「已被 task_45 取代」标记；常量速查补车栈/机栈两行。
- 引擎 `tmp/task45_tools/*.config.json`（不进 git）：review 判据改为「执行器只差文件头；Follow 树除距离常数外只差 use/文件头，车 2.2/1.2、机 3.6/2.6 不得判 FAIL」；Step 3 的 uav-Follow 判据补成「到 **2.6 m** 停，不按车的 1.2 m 判」。
- 副本一致性复核：`ugv↔ugv_sim` 14 行、`uav↔uav_sim` 14 行（仅注释+use）；`ugv↔uav` 28 行（除注释只有 use + 两个常数）。四份执行器仍逐字节相同。

### Step 3 失败复盘（用轨迹数据核对，纠正「人车互相卡住」的猜测）
| 事实 | 数值 |
|---|---|
| 人–车最近距离 | **1.82 m**（C1 跟随时），**从未接触** |
| 人最长静止 | 990 s @ (76.54, 57.48) |
| 车最长静止 | 566 s @ (60.94, 59.47)（两者相距 15.7 m） |
| 机最长静止 | 1168 s @ (66, 64)，C1~C3 全程未动 |

- **人不动 ≠ 被卡**：三次 `walk_to` 全部 `到位=False`（d=0.748 / 6.556 / 8.886），驱动器判未到位后**不再重发**，人就地站住。
- **车 C2 不是被卡**：全程 `v=0.45` 正常走，但 60 s 只走 8 m（直行 ↔ 原地转交替，与 Follow 同源的格心量化抖动，Goto 路径未在本次范围），触发 `goto` 自带 60 s 超时 → `TaskId(1) 失败 code=1` → 无任务停车。
- **机 C3 行为正确**：人在 (78.32, 57.76)，离机 **13.8 m > `max_range_m` 12**，detector 采纳 0 条 → Hold。
- **机 C4 巡逻正确**：(56,56)→(72,56)→(72,72)→(56,72) 四角全到、顺序对、重新武装正常；但人 `|x−64|=14.3 > half_side 8`，从未进框，故「切跟随/回巡逻」无从发生。

### 下一步（未做）
1. 修夹具三条：`walk_to` 真正走到位（差多少就再发）、人放到机头前 ~4.5 m / 方框内、C2 方框缩小或起点靠近第一个角（16 m 见方必超时）。
2. 只重跑 Step 3（引擎 `FROM_STEP = 3`），1/2 不必重跑。
3. 待人类：真车 Follow 摆动是否消失（上车清单仍待回填）。
