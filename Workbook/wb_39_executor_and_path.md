# wb_39_executor_and_path — task_39 工作记录

> Presented by KeJi
> Created Date ： 2026-10-29
> Modified Date ： 2026-10-29
> 任务：Task/task_39_executor_and_path.md（路径 8 连通 + 规划三态预算 + sim 机栈连续执行 + 验证）

## 执行引擎启动
- 开始时间：2026-10-29 06:42:22Z
- 引擎：Tool/execution_engine_task39.ts + Tool/execution_engine_task39.config.json（流程照搬 task_36/38）
- 运行标记：首轮（Step 1a/1b/2/3）（本次从 Step 0 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次
- 范围：规划三态+预算（1a）→ 回退膨胀+8 连通（1b）→ sim 机栈连续执行器（2）→ 验证（3）
- 验收红线：Step 3 **只验不改**；不通过就如实汇报，不调阈值/不改代码

## Step 1 — Step 1a — 规划预算与三态返回（首轮（Step 1a/1b/2/3））
- 起止：2026-10-29 06:42:22Z → 2026-10-29 07:02:11Z
- 范围：四份 nav/pathfinder + goto 叶子 + 三份 config（不动执行器/协议）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task39/plan.md`（465 行，含逐文件:行号清单、接口常量、实施顺序、风险、验收命令与判据表）。 **摘要（可直接写入 workbook）** 计划见 tmp/task39/plan.md。三态（StepResult/NavStep）加时间预算；NoPath 只在收敛后给出，返回 NotReady 前不读 g/rhs；预算每 64 次迭代查表且仅在 iter>0 时查，budget==0 时短路，因此每拍必有进展、续算不会死循环。关键取舍：不动 Navigator::new 与 NavPort::step 的签名，改为 set_plan_budget 加带默认实现的 step_result——因为 5 处测试装配点及 uav_sim/follow.rs 的 NavPort 实现都在允许路径外，改即越界。共 23 文件（nav 12、goto 4、config 3、handler 3、文档 1）；单测每变体 5 条，另加 nav 三态锁 1 条，config 8 条；验收逐 crate check --all-targets、逐 crate --lib、四对 diff、git diff 恰好 23 文件。

## Step 2 — Step 1b — 回退 sim 膨胀 + 4 连通改 8 连通（首轮（Step 1a/1b/2/3））
- 起止：2026-10-29 07:02:11Z → 2026-10-29 07:18:26Z
- 范围：四份 pathfinder.rs（不动 nav 接口、执行器、协议）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [`tmp/task39/plan.md`](tmp/task39/plan.md)（577 行）。 **摘要（写入 workbook）**： 计划见 tmp/task39/plan.md。顺序：A 先回退 sim 膨胀（删 Blocked、Passable 只看本格、cost 同真机、Notify 2 环→1 环、删 2 条膨胀测试、补贴邻反向锁）；B 四份改 8 连通（N→NW 固定序、cardinal 1.0/对角 √2、禁切角、octile、修补 1 环）。实测更正：三 crate 仅 `[[bin]]`，step.md 的 `--lib` 跑不通，改 `--bin`+`pathfinder`（基线 ugv 9/uav 9/sim 44 全绿、pathfinder 零 warning）。改后单测 15/15/27×2；判据 7 改为比对 git_baseline.txt；robot_arch.md:676/763、follow.rs:17 陈旧表述超授权，登记待裁。

## Step 3 — Step 2 — sim 机栈执行器改为连续速度 + 世界侧速度指令（首轮（Step 1a/1b/2/3））
- 起止：2026-10-29 07:18:26Z → 2026-10-29 07:36:23Z
- 范围：新建 sim 机栈执行器 + sim 下发通道 + 世界侧 Quadrotor 一条速度指令（不动车栈、不碰 ORION 协议）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task39/plan.md`（440 行）。我只读了代码并写了计划，没有改动任何源码，`git status` 与进场基线逐行一致。 计划摘要（可直接写进 workbook）： 计划见 tmp/task39/plan.md（440 行）。共改 10 个文件：Rust 7 个（含新建 `uav_sim/executor.rs`）、Godot 2 个、文档 1 个。关键决定：`omega=-sign(Δθ)·turn`（证据链 `action_map.rs:21-24 → Quadrotor.gd:135`），世界侧 `clampf` 不再取反；`MotionCmd` 加两个 `Option` 字段（`ugv.rs` 在允许路径外读 `.action/.speed`，不能换 enum/换类型）。`net/` 已核实无需改（整条 msg 透传）。单测 +14 条（398→412）；`--lib` 实测跑不通，改用 `--bin pleiades-sim`；Godot 基线实测 48→53；验收含逐 crate `check`、5 个未授权文件的 `git diff` 必须为空。 要点提醒实现者： - **符号**：Δθ>0 = 右转 ⇒ `omega` 取负（`rotation.y` 减小）；执行器单测左右各一条 + Godot `velocity-sign-right` 双锁。 - **单位**：按 step.md 的字面片段 `clampf(v, ±_max_speed)`，实际即满速 2.0 m/s / 满舵 90°/s（旧 forward 仅 0.6 m/s）；已用 `velocity-clamps` 断言把该饱和登记为已知行为，备选方案（百分比解释）留给 step 3 调参。 - **不越界**：`sim/executor.rs`、`chassis/ugv.rs`、`config.rs`、`pleiades-base`、`net/` 一个字不许动；不 `cargo fmt`、不 commit。

## Step 4 — Step 3 — 验证（常规移动 + 跟随）（首轮（Step 1a/1b/2/3））
- 起止：2026-10-29 08:17:53Z → 2026-10-29 08:22:17Z
- 范围：仅新增 `Tool/sim_task39_step3_test.py` + `RobotSim_TestEnv/results/task39_20261029T081753Z/`（不改 Rust / 不改世界 / 不改 `Tool/sim_follow_test.py`）
- 轮次：1 / 3
- 结果：**FAIL(env)** —— 常规移动 M1/M2/M3 全部**未到达**（`[Sim] 任务 TaskId(…) 失败，原因码 1`＝`Timeout`，goto 树 `Timeout(60_000 ms)` 到期）；跟随 F1~F4 **未执行**（F1 前置归位 `goto(56.25,64.25)` 同样 60 s Timeout，实测停在 (69.76, 64.89)，按计划记 FAIL(env) 并停）
- 规划要点：计划见 `tmp/task39/plan.md`（496 行）。只验不改：新建脚本复用 `Tool/sim_follow_test.py`（sha256 `2e4a54d3…`，一字节未动）的起停 / 控制口 / 行人 / detector 解析 / 截图设施，自写 16 条前置 + S0 11 锚点 + 9 条 status 断言 + 3 个常规移动用例 + 27 条判据 + verdict。判据口径：`[Sim] UAV` 行 `v==0 且 omega==0` 即「停」（旧 `动作=Stop` 的语义等价，Step 2 后该字段已不存在），并列字面条数；主口径一律物理时间（物理时钟 = detector `frame_ts_ms` 插值）。
- 实测（逐条见 `RobotSim_TestEnv/results/task39_20261029T081753Z/verdict.md`）：
  - 环境：相机 **2.11 Hz**（`rate_hz=10` 拿不到；llvmpipe 渲染 **2.0 fps**，帧间隔中位 **0.475 s**）；pose 下行被渲染帧率压到 2 Hz，`位姿龄` 中位 **229~245 ms**、最大 **491~540 ms**；实测最大速度 **0.38~0.72 m/s**（而档位被当 2.0 m/s）⇒ 世界动力学仅 **≈0.26× 墙钟**；`k = Δ物理/Δ墙钟 = 1.0269`。
  - M1 直线 20 m：60 s 内净位移 **4.27 m**、路径 6.27 m、yaw −86.8°~+59.8°（跨度 146.6°）；`L1 FAIL`，`L2/L3/L4/L5 PASS`（停段 0 段）。
  - M2 L 形：60 s 内净位移 3.63 m、停段 1 段；`L6 FAIL`、`L9b FAIL`（omega 符号翻转 **10 > 3**），其余 PASS。
  - M3 长路径（44 格）：60 s 内净位移 3.23 m、停段 1 段；`L11 FAIL`，其余 PASS。
  - F1/F2/F3/F4：**FAIL(env) 未执行**（归位不可达）。
- 根因初判（代码级）：一拍转角 = `turn_speed`→世界 `clampf(omega,±π/2)`=90°/s × `位姿龄` 中位 0.245 s ≈ **22.1°**，**大于 `SPIN_DEG=20°`、远大于 `DEADBAND_DEG=5°`**（`pleiades-sim/src/sim/uav_sim/executor.rs`）⇒ 每拍纠正都越过死区，在「左转 / 右转 / 偶尔弧线」之间形成**极限环**，进不了直行档；再叠加 0.26× 墙钟，20 m 需 ≈38 s，多数用例在 60 s `Timeout` 内必然到期。**这不是「跟随被改坏」的证据，而是「本机不是连续执行器的合格验证环境」**；`cargo test -p pleiades-sim --bins uav_sim::executor`（6 条）、`uav_sim::mission::follow`（13 条）、`sim::uav_sim::nav`（50 条）全绿，控制律在理想 pose 频率下自洽。
- 复跑记录：共跑 4 次；`075624Z`/`080358Z`/`081001Z` 是 harness 三个自身 bug（k 初值为 None、用例日志窗只锁左边界导致跨用例串行、yaw 跨度写死）修复前的产物，已删除；`081753Z` 为最终证据。四次结论一致（M1~M3 均 Timeout、F1 归位不可达）。

## Step 4 — Step 3 — 验证（常规移动 + 跟随）（第 2 轮）
- 起止：2026-10-29 08:42:05Z → 2026-10-29 09:21:29Z
- 范围：仅 `Tool/sim_task39_step3_test.py`（改写）+ `RobotSim_TestEnv/world_fast/`（世界副本，gitignored）+ `RobotSim_TestEnv/results/task39_20261029T091203Z/`（产物）；不改 Rust / 不改世界 / 不改 `Tool/sim_follow_test.py`
- 轮次：2 / 3
- 结果：**FAIL**（28 行判据：PASS 17 / FAIL 10 / FAIL(env) 1；退出码 1）。常规移动 M1~M3 仍全部 `Timeout`；跟随 F1/F2 有真数据且**跟不住**；F3 关客户端前已自行停住（前置不过 → FAIL(env)）；F4 通过
- 规划要点：计划见 `tmp/task39/plan.md`（299 行）。上一轮的**验证方法**有问题（不是被测代码结论）：F1 归位抛 `FixtureError` 直接中止整轮 ⇒ 27 行只出 17 行、F1~F4 零数据；宣告 FAIL(env) 前一个 harness 旋钮都没试。本轮逐条对症：世界副本关 4× MSAA、用例逐个独立跑、判据兜底恒 28 行、detector 双口径（真人 vs adopted）、overall 按「有真实 FAIL 就判 FAIL」、退出码 0/1/2 分离
- 环境修复（本轮最大改动，全在允许路径内）：`RobotSim_TestEnv/world_fast/` = `orion-simulation-world/` 全量副本 + 唯一 `override.cfg`（`msaa_3d=0`），源世界零改动（preflight 19 `diff -r` 输出 0 行）。同机实测 **2.0 fps → 11.0 fps（5.5×）**，S0j `位姿龄` 中位 **70 ms**（上一轮 245 ms，门 ≤120 ms 过），`k=1.0323`；detector 推理中位 **302 ms ⇒ 检测链上限 3.31 Hz**（属被测代码，不可调）
- 实测（逐条见 `RobotSim_TestEnv/results/task39_20261029T091203Z/verdict.md`）：
  - §1 前置自检 **21/21 OK**（新增 5 条：副本存在 / override 单键 / 副本逐字节相同 / 新鲜度标记 / harness 死代码自检）
  - §3 判据 **28 行全有实测值**（计划写 27，`L9b` 是 M2 的 omega 子项单列 ⇒ 实为 28，已在 verdict 注明）
  - M1 直线 20 m：60 s `Timeout`（码 1），末位 (79.118, 64.376) Δ=(5.132, 0.126) → `L1 FAIL`
  - M2 L 形：60 s `Timeout`，末位 (84.103, 80.827) Δ=(0.147, 3.423) → `L6 FAIL`；`L9b FAIL`（omega 符号翻转 **35 > 3**）
  - M3 长路径 44 格：60 s `Timeout`，末位 (95.911, 81.629) Δ=(10.339, 2.621) → `L11 FAIL`；`L15 FAIL`（trace 单步最大 **0.693 m > 0.50**）
  - F1 直线跟随：10 s 位移 **1.058 m**（<5）、末距离 **30.523 m**（不掉队要求 ≤5）⇒ F1-1/F1-2 FAIL；F1-3 真人 min **5.660 m** PASS
  - F2 切线通过（重点）：真人最长出画 **15.531 s**（≤1 才过）、10 s 位移 **0.800 m** 且方向 cos **−0.916**（即往人的反方向）、停段 2 段（≤3 PASS）、真人 min **8.970 m**（≥2 PASS）
  - F3 丢失：关客户端前末 3 条 v=[0,0,0] ⇒ F3-1 FAIL(env)；关后漂移 0.000 m、|Δyaw| 1.400°、末 1 s 位移 0.000 m ⇒ F3-2/F3-3 PASS
  - F4 不撞人：三窗真人 min=[5.66, 8.97, 5.54]，合并 **5.540 m** ⇒ PASS
  - 逐 crate `cargo check --all-targets`：ugv/uav/terminal/sim + base（不带 --all-targets）全部 `Finished`、0 error；`cargo test -p pleiades-sim --bins`：**410 passed / 2 failed**，红的 2 条是 HEAD 即红的 `MountOut.pitch` 遗留项
- 根因初判（代码级，verdict §6 有全文）：
  1. `turn_speed=30` 被世界当 90°/s + 位姿龄 75 ms ⇒ 每拍转 ≈6.8°，**大于 `DEADBAND_DEG=5°`**（`uav_sim/executor.rs`），±30 档全有全无形成极限环（M2 omega 翻转 35 次）；
  2. `follow.rs` 一拍只给 **0.5 m 格**，机必须为这 0.5 m 转向；实测一次转向把 yaw 从 −14.4° 甩到 **−109.6°**（目标方位 −27°），人立刻滑出 43° 半 FOV ⇒ 下一拍 `lost` ⇒ `Hold` 冻住。这就是 F1 只走 1.06 m、F2 反向 0.8 m 的直接机制；
  3. 检测链 3.31 Hz 上限（推理 302 ms）⇒ 800 ms `stale` 只用得到 2.6 帧，人出画两帧就判丢失；
  4. detector 幻影：全轮 20 帧 `person` 框不对真人（3 帧 bbox 底边到 y=479、20 帧曾被 adopted），`距离=` 2.49~11.07 m；`follow.rs` 取 `range_m` 最小者 ⇒ 2.5 m 幻影会抢在真人之前被吃下
- 夹具修正（记偏差，非调判据）：`goto` 到达后机头朝向不可用（按位置判到达，yaw 在 −83.8°…+97.5° 之间甩），人落在 43° 半 FOV 外 ⇒ 首轮「零数据」会以「零检测」的形式重演；遂加 `prep_face()` 用 `manual spin_left/right` 闭环把 |Δθ| 压到 ≤20°（另修两处 harness 坑：`status.yaw` 是**弧度**；切回 auto 时旧队列会复活）。摆正后 F1/F2/F3 的 `vantage_ok` 全 True，且真的测到了人
- 残留：`pgrep -af 'godot|pleiades-sim|Xvfb'` 空、9999/9098 空闲、`/tmp/.X99-lock` 与 `/tmp/.X11-unix/X99` 已删（verdict §8/§10 各记一次）
- 复跑记录：上一轮 `081753Z` 保留；本轮 `task39_probe_20261029T084711Z`（单独旋钮门禁）、`task39_20261029T085418Z`（夹具朝向修正前的整轮，**M2 曾到达**）、`task39_20261029T091203Z`（**最终证据**）；8 个中途目录已删。已知不稳定：M2 一次到达（`085418Z`）、一次差 3.42 m（`091203Z`），M1 末位分别走 10.0 m / 5.1 m —— 复核以单个目录的 verdict.md 为准

## Step 3 之后：主 agent 口径对齐（2026-10-29）
- 人类决定：机栈速度改成**物理量**，与真机 UAV 同名同单位（`[flight_ctrl] vel_fwd` m/s / `yaw_rate_deg` °/s），不再借用车的 `[chassis]` 百分比档位。
- 已改（主 agent 直接落地，编译 + 单测通过）：`sim/uav_sim/executor.rs`（`VelocityCmd{f32,f32}`、`UavExecutor::new(vel_fwd, yaw_rate_rps)`）、`sim/chassis/{mod,uav}.rs`、`world_link/{mod,protocol}.rs`、`sim_handler.rs`（读 `[flight_ctrl]`）、`config.rs`（新增 `FlightCtrlConfig` + 默认 0.3/15 + clamp）。
- sim 实例：`RobotSim_TestEnv/sim1/.config/config.toml` 增 `[flight_ctrl] vel_fwd=0.8, yaw_rate_deg=30.0`；`target/release/pleiades-sim` 已重建并拷到 sim1（sha 需一致，前置自检会查）。
- 单测：`cargo test -p pleiades-sim --bins uav_sim::executor` 7 passed；`chassis::` 37 passed；`config::` 26 passed。
- 下一步：用执行引擎从 Step 4（验证）重跑，不再由主 agent 手工起测试脚本。

## 执行引擎启动（重跑验证）
- 开始时间：2026-10-29 09:51:03Z
- 引擎：Tool/execution_engine_task39.ts + .config.json（本轮 FROM_STEP=4）
- 运行标记：口径对齐后重跑验证（Step 4）
- 前置：机栈速度已改为物理量（`[flight_ctrl] vel_fwd/yaw_rate_deg`），sim 实例 0.8 m/s / 30°/s；二进制已重建
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；评审 FAIL 再跑一轮

## Step 4 — Step 3 — 验证（常规移动 + 跟随）（口径对齐后重跑 · 第 1 轮 / 3）
- 起止：2026-10-29 09:56:41Z → 2026-10-29 10:11:30Z
- 范围：仅 `Tool/sim_task39_step3_test.py`（唯一被改的文件）+ `RobotSim_TestEnv/results/task39_*/`（新产物）；**不改 Rust、不改世界、不改 `Tool/sim_follow_test.py`（sha256 `2e4a54d3…` 未变）**
- 轮次：1 / 3（口径对齐后从 Step 4 重跑的第 1 轮；上一轮「第 2 轮」的 `90°/s` 极限环结论文本已被 §四 修订取代）
- 结果：**FAIL**（28 行：PASS 23 / FAIL 5 / N-A 0，退出码 1）。常规移动 **M1/M2/M3 全部到达**（27.2 s / 29.2 s / 31.2 s，Δ 逐轴 ≤0.043 m）；跟随 F1/F2 **跟不住**（F1-2、F2-1、F2-2、F2-3 FAIL）、F3 通过、F4 通过；`L9b`（M2 拐角 omega 翻转 29 > 3）FAIL
- 规划要点：计划见 `tmp/task39/plan.md`（394 行）。本轮**只验不改**，先修掉 3 类会让结论失真的 harness 缺陷再跑：① `[Sim] UAV` 行解析只认整数（`v=0.8 omega=-0.5235988` 读不到 ⇒ 440 行只解析 133 行＝30%）→ 改 float + 新增覆盖率门（解析数≠日志行数 ⇒ 该判据 `FAIL(env)`）+ 第 23 条前置门 `uav_log_formats`（4 条逐字取自 `sim_handler.rs` 格式串的样本）；② 判据文案/根因仍是 `90°/s`、`2.0 m/s`、`1.27 m` 旧口径 → 全部改由 `executor_consts()` + `flight_ctrl_cfg()` + `world_caps()` 现算，新增第 22 条前置门 `config_flight_ctrl`（0.8 / 30.0）；③ 复算契约钉着上一轮数字 → 改「当轮 0 diff + 历史目录只登记差异」（`REGRADE_LEGACY` / `REGRADE_PIN`）。**判据阈值与用例常量一个都没动**
- 实测（逐条见 `RobotSim_TestEnv/results/task39_20261029T100655Z/verdict.md`）：
  - 环境：相机 **2.35 Hz**（帧间隔中位 0.426 s）、detector 推理中位 **300 ms**（检测链上限 3.34 Hz）、位姿龄中位 **70 ms**（门 ≤120 ms 过）、`k=1.0268`；§1 前置自检 **23/23 OK**
  - §3 判据 **28 行全有实测值**（无「未取到实测值」）；`[Sim] UAV` 全轮 **992/992 行解析**（覆盖率 100%）
  - M1 直线 20 m：**到达**，末位 (84.245, 64.236) Δ=(0.005, 0.014)，停段 2 段、微循环 0、路径长/直线 1.002
  - M2 L 形：**到达**，末位 (84.223, 84.293) Δ=(0.027, 0.043)；yaw 跨度 105.5°（真有一次 ≈90° 转向）；`L9b` FAIL（omega 符号翻转 **29 > 3**）
  - M3 长路径 44 格：**到达**，末位 (106.293, 84.222) Δ=(0.043, 0.028)，单步最大 0.107 m
  - F1 直线跟随：10 物理 s 位移 **6.694 m**（≥5 过）、末 5 s 距离中位 **20.648 m**、末距离 **23.673 m**（≤5 不过）、真人 min **5.660 m**
  - F2 切线通过（重点）：真人最长出画 **3.584 s**（≤1 不过；adopted 1.174 s）、10 s 位移 **1.586 m**（<5 不过）、方向 cos **−0.214**（<0.707 不过）、停段 **2 段**（≤3 过）、真人 min **5.240 m**（≥2 过）
  - F3 丢失：关前末 3 条 v=[0.8, 0.8, 0.8]、omega=[0.0, −0.5235988, −0.5235988]、关前位移 1.588 m ⇒ 前置成立；关后 3 s 窗漂移 **0.313 m**、|Δyaw| **6.25°**、末 1 s 位移 **0.000 m** ⇒ F3-2/F3-3 PASS（**不再是上一轮的 N-A 真空**）
  - F4 不撞人：三窗真人 min=[5.66, 5.24, 5.59]，合并 **5.240 m** ⇒ PASS
- 根因初判（代码级，verdict §6 有全文）：
  1. `[flight_ctrl]` 30°/s × 位姿龄 68.5 ms ≈ **2.1°/拍 ≤ `DEADBAND_DEG=5°`** ⇒ **直行档**，上一轮的「极限环」不成立；M1~M3 全部到达即为直接证据
  2. `L9b` 仍红的原因换成了**几何**：`r = v/ω = 0.8 / (30°/s) ≈ 1.53 m` ≫ 0.5 m 格 ⇒ M2 的一次 90° 转向要画 ≈1.5 m 弧，`omega` 在 ±0.5235988 之间来回 29 次（世界侧 `clampf(ω,±1.57)`、`clampf(v,±2.0)` ⇒ 不截断）
  3. F1-2 必 FAIL 是**算术**：机速上限 0.8 m/s < 人速 1.26 m/s（净差 0.46 m/s），起点间距 5.74 m ⇒ 20 s 后 ≈14.94 m；实测 23.67 m（机实际只跑出 6.69 m/10 s）
  4. F2-1/F2-2/F2-3 是**检测链 + 跟随机制**：300 ms 单帧推理 ⇒ 3.34 Hz；`follow.rs` 取 `range_m` 最小者、`STALE_MS=800`；人沿 +y 横穿、机从侧向切入 ⇒ 净位移方向由「指向人」的方位决定（cos 只能 ≈0），判据与切向几何不自洽
- 夹具/口径偏差（记偏差，非调判据）：`F2_START` 仍未接线（保持单跳归位，避免为过而移动起点）；`F3_DRIFT_MAX=0.70` 按旧满速推导、新口径下偏宽（理论 0.107 m），保留不动只记录；M2 实跑成「先北向直线再 90° 转向」（不是计划预判的 45° 对角），已按实测改写 §5 观察项（`yaw 跨度 ≥60° ⇒ 拐角语义成立`）
- 复跑记录：共 2 次完整取证跑 + 2 次复算。`task39_20261029T100147Z`（第一次，§5 的 M2 逐秒 |Δyaw| 因 harness 双重弧度换算写出 1604.3°，已修；判据部分不受影响）、**`task39_20261029T100655Z`（最终证据）**；`task39_regrade_probe_20261029T101113Z`（对口径修订后的 2 用例 probe `094918Z` 复算：解析 440/440、差异只有 L4/F2-4、其余 26 行逐字节相同）、`task39_selfcheck_20261029T101111Z`（对最终证据复算：**28/28 verdict + 28/28 measured 0 diff**，`REGRADE_PIN` 已按实测填死）。**已知不稳定**：两次跑 F2-2 一次 5.205 m（PASS）一次 1.586 m（FAIL）、F2-1 一次 1.179 s 一次 3.584 s —— 取证以单目录 `verdict.md` 为准，不取最好成绩
- 残留：`pgrep -af 'godot|pleiades-sim|Xvfb'` 空、9999/9098 空闲、`/tmp/.X99-lock` 与 `/tmp/.X11-unix/X99` 已消失（verdict §8/§10 各记一次）；`git status --porcelain -uall` == 进场基线、`git diff --numstat` sha 未变、世界副本 `diff -r` 0 行
- 需要人类决定的两条（**本 step 无权改**）：① `F1-2`（机速上限 < 人速 ⇒ 几何上不可能过）——提高 `vel_fwd` 或降人速；② `F2-3`（切向通过时净位移方向必然不随人）——改判据口径或改场景
