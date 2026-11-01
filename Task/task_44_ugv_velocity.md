# task_44_ugv_velocity — 真车/真机/sim 统一连续运动控制

> Presented by KeJi
> Created Date ： 2026-11-01
> Modified Date ： 2026-11-01
> 状态：**Step 1 ~ Step 8 待人类确认**。执行按 `.github/instructions.md` 的执行引擎流程（每 step：规划 → 实现 → 评审）。

## 一、目标

### 1.1 三个大目标

1. **行动模式替换（四端）**：真车（`pleiades-ugv`）、真机（`pleiades-uav`）、sim 车（`pleiades-sim/ugv_sim`）、sim 机（`pleiades-sim/uav_sim`）的行动模型**整体换成连续速度**（v, ω）。旧的离散动作（`0x11 FUNC_CAR_RUN`、`MotionAction` 加三状态机执行器）**整体删除**，不是"再加一条通道并存"。
2. **仿真世界验证**：模型换完之后，在仿真世界里跑 sim 车 + sim 机，确认车能走、机能飞、手动两轴能合成弧线。
3. **行为树搬到真车**：真车的行动模型与 sim 车对齐后，把 sim 车栈的 `Follow` / `Containment` 行为树、detector 设备、`Target` 类型、协助请求发布者**逐份复制**到 `pleiades-ugv`，让真车也具备真跟随与真封控。

### 1.2 已定口径（讨论已确认，执行时不再回头问）

| # | 项 | 定值 |
|---|---|---|
| 1 | 旧通道 | `0x11 FUNC_CAR_RUN` **彻底不用**（含停车）；`MotionAction` / 三状态机执行器 / `Actuator::UgvDiscrete` 全部删除 |
| 2 | 统一量纲 | 协议与执行器统一用 **(v m/s, ω rad/s)**；`ω > 0` = 左转（逆时针） |
| 3 | `msgid 4` | **原地改布局**：运动 8 B（`v f32 BE | ω f32 BE`）+ 事件 3 B（`action u8 | param i16 BE`），按长度分流，其余整帧拒 |
| 4 | `msgid 8` | **新增**：模式切换，1 B（`0` = 手动 / `1` = 自动） |
| 5 | 兼容 | **不做版本兼容**，四端 + Pictor 强制同批升级 |
| 6 | 手动 | 两轴连续量（WASD 可同时前进 + 转向）；**全部松开键发 (0, 0)**；**不做**指令超时 / 心跳重发 |
| 7 | Pictor | 在范围内：WASD 改两轴、模式切换改走 `msgid 8` |
| 8 | 拷贝而非共享 | detector / `Target` / 协助请求 / 行为树**一律各 crate 自带一份**，不进 base（即使逐字节相同） |
| 9 | 真车车型 | `CarType` 增加 **`M1 = 0x0A`**，`[chassis] car_type` 缺省改 **M1**（本车是麦克纳姆 M1，不是 X3Plus） |
| 10 | 验证口径 | 沿用 task_43 #12 / #13：**以「行为出现」为准**，数值只记录不判 FAIL；验证步只跑 1 轮、FAIL 不阻断；代码级失败不适用此条 |
| 11 | 执行器量级 | **定值**：`v = 0.45 m/s`、`ω` 绝对值 `0.5 rad/s`；执行器内**不做任何比例换算**、不读百分比 |
| 12 | config 键 | `[chassis] forward_speed` / `turn_speed`（百分比）**删除**，换成 `cruise_speed`（m/s）/ `cruise_yaw_rate`（rad/s）；`car_type` 缺省 `M1` |

### 1.3 四端各自落到什么执行机构

| 端 | 执行器输出 | 到执行机构 |
|---|---|---|
| 真车 | `(v, ω)` | `0x12 FUNC_MOTION`：`vx = v`、`vz = 标定表(ω, v)` |
| 真机 | `(v, ω)` | mavlink 速度 + 偏航角速度设定（现有 `set_desired` 语义换成定量） |
| sim 车 | `(v, ω)` | `motion_cmd{action:"velocity", v, omega}` → 世界 `DiffDrive` 直接收物理量 |
| sim 机 | `(v, ω)` | 同现状（世界 `Quadrotor` 已收物理量） |

> 执行器算法**四端同形**：车两份逐字相同（真车 / sim 车），机两份逐字相同（真机 / sim 机）。差异只允许出现在 `use` 路径与文件头。

### 1.4 车栈执行器的输出（两端逐字相同）

| 条件 | v (m/s) | ω (rad/s) |
|---|---|---|
| `next_cell = None` | 0 | 0 |
| 目标格在身后且很近（`d < PASS_THROUGH_M`，`Δθ` 绝对值 > 90°） | 0.45 | 0 |
| `Δθ` 绝对值 ≤ 8°（`DEADBAND_DEG`） | 0.45 | 0 |
| `Δθ` 绝对值在 8°~12° 之间（`ARC_MAX_DEG`） | 0.45 | ±0.5 |
| `Δθ` 绝对值 > 12° | 0 | ±0.5 |

- 判断顺序即上表顺序（身后直行最先）；`ω < 0` = 右转。
- 手动就是同一张表：`W` → `(0.45, 0)`、`W+A` → `(0.45, +0.5)`、`A` → `(0, +0.5)`、全松 → `(0, 0)`。
- 量级全是定值，不随别的东西变化；三个阈值（`PASS_THROUGH_M` / `DEADBAND_DEG` / `ARC_MAX_DEG`）保持不变。
- 机栈执行器同样是定值三档（直行 / 边走边转 / 原地转），落点见 §1.3。

---

## 二、Step

### Step 1 — 协议层：`msgid 4` 改布局 + 新增 `msgid 8`

**目的**
把 ORION 的手动命令从"一维动作枚举"改成"两轴连续量"，把模式切换拆成独立消息。这是后面所有 step 的地基，先做、先测死。

**涉及文件**

修改：
- `pleiades-base/src/Robot/core/protocol/messages.rs` —— 删 `ACTION_FORWARD/BACKWARD/SPIN_LEFT/SPIN_RIGHT/STOP`；新增运动 8 B 编解码；事件 `action` 重排为 `1=beep / 2=start_lidar / 3=stop_lidar / 4=takeoff / 5=land`；`decode_manual_control` 改为按长度分流的枚举
- `pleiades-base/src/Robot/core/protocol/mod.rs` —— 新增 `MSGID_MODE: u16 = 8`；导出新函数与新常量
- `pleiades-base/src/Robot/core/protocol/command_decode.rs` —— `msgid 4` 分流（8 B → 运动 / 3 B → 事件）；新增 `msgid 8` 分支 → `Command::Mode`
- `pleiades-base/src/Robot/core/command.rs` —— `ManualCmd` 删 `Forward/Backward/SpinLeft/SpinRight`，新增 `Motion { v: f32, omega: f32 }`；`Stop` **保留**（内部语义，见下）
- `docs/design_doc/orion_protocol.md` —— §3 总览表加 `msgid 8`；§3.4 整节重写；新增 §3.6 模式切换

**实施步骤**

1. 定字节（大端，与现有协议一致）：
   ```
   运动（8 B）: v f32 BE | omega f32 BE
   事件（3 B）: action u8 | param i16 BE
   模式（1 B）: mode u8        0 = 手动 / 1 = 自动
   ```
2. `decode_manual_control(payload)` 改成返回枚举：
   - `len == 8` → `ManualPayload::Motion { v, omega }`
   - `len == 3` → `ManualPayload::Event { action, param }`
   - 其它长度 → `None`（整帧拒，不做"猜"）
3. `msgid 8` 解码：`len != 1` → 拒；`mode` 只能是 `0` / `1`，其它值 → 拒。
4. `parse_orion_frame`：
   - `msgid 8` → `Command::Mode(SwitchToManual | SwitchToAuto)`（**必须映射到 `Command::Mode`，不能映射成 `ManualCmd`** —— 它要绕过 `main_loop` 的手动模式门闩，否则"从手动切回自动"会被自己拦住）
   - `msgid 4` 运动 → `Command::Manual(ManualCmd::Motion { v, omega })`
   - `msgid 4` 事件 → 对应 `ManualCmd`（`Beep` / `StartLidarScan` / `StopLidarScan` / `Takeoff` / `Land`）
5. `ManualCmd::Stop` 的定位：协议里**不再有** stop 动作（全零的 8 B 运动帧就是停）；`Stop` 只作为内部命令保留（sim 控制口的 `stop`、切模式时的显式停车）。设备端把 `Stop` 解释为 `(0, 0)`。
6. 金标字节串（写进单测，三端共用同一串）：
   ```
   v=0.5,  omega=0      -> 3f000000 00000000
   v=0,    omega=0      -> 00000000 00000000
   v=0,    omega=-0.5   -> 00000000 bf000000
   mode=manual          -> 00
   mode=auto            -> 01
   ```
7. 单测覆盖：8/3 长度分流、非法长度整帧拒、未知 action 拒、`msgid 8` 非法值拒、五条金标逐字节、`msgid 8` 映射到 `Command::Mode`（断言变体，不是 `Command::Manual`）。
8. 文档同步（协议表、布局表、`action` 枚举表、新增模式消息一节）。

**验收**
1. `cargo test -p pleiades-base --lib protocol` 全绿（含新增用例）；
2. `cargo check -p pleiades-base --lib` 0 error、新增代码 0 warning；
3. `ManualCmd` 里已无 `Forward/Backward/SpinLeft/SpinRight`（`grep` 为空）；
4. 五条金标与 `docs/design_doc/orion_protocol.md` 里写的字节完全一致。

---

### Step 2 — sim 车栈 + sim 机栈换模型

**目的**
把仿真两栈的执行器与手动入口改到 `(v, ω)`，删掉离散执行器与遗留文件；世界侧 `DiffDrive` 改收物理量。做完这一步，仿真里就没有"档位"这个概念了。

**涉及文件**

修改：
- `pleiades-sim/src/sim/ugv_sim/executor.rs` —— `VelocityCmd` 从 `{speed: u8, turn: i16}` 改为 `{v: f32, omega: f32}`；输出量级改成定值（构造参数传入 0.45 / 0.5），五档判断顺序与三个阈值（`DEADBAND_DEG=8` / `ARC_MAX_DEG=12` / `PASS_THROUGH_M=0.5`）保持不变
- `pleiades-sim/src/sim/uav_sim/executor.rs` —— 输出结构统一为同一个 `VelocityCmd`（行为不变）
- `pleiades-sim/src/sim/sim_handler.rs` —— 删 `Actuator::UgvDiscrete` 与 `Discrete_Actuator()`；`on_tick` ④ 只剩车/机两条连续臂；`handle_manual_cmd` 支持 `ManualCmd::Motion`（直接交底盘，不再走离散动作）
- `pleiades-sim/src/sim/chassis/{mod.rs, ugv.rs, uav.rs}` —— 删 `Apply_Velocity_Pct`，车与机统一 `Apply_Velocity(v, omega)`
- `pleiades-sim/src/sim/chassis/action_map.rs` —— 离散动作映射随 `0x11` 一起删除；`SpeedBinding` 改为 `cruise_speed: f32` / `cruise_yaw_rate: f32`；若文件只剩无用内容则整文件删
- `pleiades-sim/src/config.rs` —— `[chassis] forward_speed` / `turn_speed`（百分比）删除，换成 `cruise_speed`（m/s，缺省 0.45）/ `cruise_yaw_rate`（rad/s，缺省 0.5），补全逻辑同步
- `pleiades-sim/src/sim/control.rs` —— `manual.action` 字符串（`forward|backward|spin_left|spin_right|stop`）改成两轴：`{"cmd":"manual","v":…,"omega":…}`
- `pleiades-sim/src/world_link/{mod.rs, protocol.rs}` —— `motion_cmd` 的 `speed`/`turn` 键换成 `v`/`omega`；删 `Send_Velocity_Pct`
- `orion-simulation-world/src/chassis/DiffDrive.gd` —— `"velocity"` 分支改为直接收 `v`(m/s) / `omega`(rad/s)，删 `SpeedPct` / `_TurnPct` 百分比换算；5 个离散 action 分支一并删除
- `orion-simulation-world/src/chassis/Quadrotor.gd` —— 确认已是物理量，预计不改

删除：
- `pleiades-sim/src/sim/executor.rs`（遗留离散三状态机，与真车 `executor.rs` 逐字相同的那份）

**实施步骤**

1. 先把 `VelocityCmd` 统一（车/机同一个结构），改两个执行器文件；控制律一行不改，只换输出量的单位。
2. `sim_handler`：三态 `Actuator` 收成两态；删 `UgvDiscrete` 分支与 `Discrete_Actuator()`；`on_tick` 里 `ExecuteState` 仍只写 `sub_target`（`state` 留 `Idle`）。
3. 底盘：`Apply_Velocity_Pct` 删除 → 车：`Apply_Velocity(v, omega)` 直接发 `motion_cmd{action:"velocity", v, omega}`；机沿用。
4. 世界侧 `DiffDrive`：`"velocity"` 分支改成 `_speed_target = v`、`_yaw_target = omega`（仍走原有斜坡限幅与 clamp）；删掉百分比换算与离散 action 分支。`max_speed` / `max_turn_rate` 只留作 clamp 上限。
5. 控制口 `manual` 改两轴解析（`v` 与 `omega` 都要做范围校验）。
6. 单测：执行器五档（含 `PASS_THROUGH`、`±π` 归一化、符号）、`Apply_Velocity` 未装配时的丢弃行为、控制口 `manual` 参数校验；GDScript 侧 `check_diff_drive.gd` 的 velocity 用例改收物理量。
7. `cargo check -p pleiades-sim --all-targets` + 相关单测；确认 `sim/executor.rs` 已删除且无引用。

**验收**
1. `cargo check -p pleiades-sim --all-targets` 0 error、新增代码 0 warning；
2. `cargo test -p pleiades-sim --bins executor` / `--bins control` 全绿；
3. 全仓 `grep -rn "Apply_Velocity_Pct\|UgvDiscrete\|Discrete_Actuator" pleiades-sim orion-simulation-world` 为空；
4. `pleiades-sim/src/sim/executor.rs` 已删除、无引用；
5. 世界侧 `DiffDrive.gd` 的 `"velocity"` 分支只做 `v`/`omega` 的赋值，无百分比。

---

### Step 3 — Pictor：WASD 两轴 + 模式走 `msgid 8`

**目的**
把地面站的手动控制从"按键发一帧、松手停车"改成"按键状态 → (v, ω) → 变了才发"，并把模式切换改到新消息。

**涉及文件**

修改：
- `Pictor/src/control/input_handler.gd` —— 维护"当前按住的键"集合；每次按下/松开重算 `(v, omega)`，与上一次不同才发一帧；集合为空发 `(0, 0)`；**不再**"任何一个键松开就发停车"
- `Pictor/src/websocket/protocol/protocol_def.gd` —— 删 `ACTION_FORWARD/BACKWARD/SPIN_LEFT/SPIN_RIGHT/STOP`；事件 action 重排为 `1..5`；新增 `MSGID_MODE = 8` 与 `MODE_MANUAL/MODE_AUTO`
- `Pictor/src/websocket/protocol/message_builder.gd` —— 新增 `build_manual_motion(v, omega)`、`build_mode_switch_manual/auto()`（走 `msgid 8`）；删 `build_manual_forward/backward/spin_left/spin_right/stop`
- `Pictor/src/websocket/protocol/orion_messages.gd` —— `Encode/Decode_Manual_Control` 改 8 B（运动）与 3 B（事件）；新增 `Encode/Decode_Mode`
- `Pictor/src/kernel/kernel_bridge.gd` —— `Build_Cmd` 支持 `msgid 8`
- `Pictor/src/ui/WebSocket/vehicle_panel_manager.gd` / `ws_command_server.gd` —— 模式切换调用点改到新 builder（`ws_command_server.gd` 的 `mode` 指令仍可用）
- `Pictor/src/test/test_orion_protocol.gd` —— 金标换成 Step 1 的五串；补 `msgid 8` 用例
- `Pictor/tool/pictor_ws_ctl.py` —— 补 `manual --v --omega`（测试用；无此入口手动链无脚本化验证手段）

**实施步骤**

1. 把"按键集合 → (v, ω)"提成**纯函数**（`_Input_To_Velocity(held: Dictionary) -> Vector2` 之类），键位与量级常量（0.45 / 0.5）集中在一处；这样可以在无头测试里直接断言。
2. `_input` 里只维护集合 + 调纯函数 + 比较上一帧值决定是否发帧；`echo` 事件仍忽略。
3. 模式切换改 `build_mode_switch_manual/auto()` → `msgid 8`；面板勾选逻辑不变。
4. 协议常量与编码改造，金标对齐 Step 1。
5. 单测：① `W` → `(v_max, 0)`；② `W+A` → `(v_max, +ω_max)`（左转为正）；③ 松开 A 保留 W → `(v_max, 0)`（**不是** `(0,0)`）；④ 全松开 → `(0,0)`；⑤ 模式帧 1 B。
6. `godot --headless --path Pictor -s src/test/test_orion_protocol.gd` 全绿。

**验收**
1. 上述五条单测全绿，`test_orion_protocol.gd` 输出 `ALL PASS: true`；
2. `grep -rn "build_manual_forward\|ACTION_SPIN" Pictor/src` 为空；
3. 模式切换只出现在 `msgid 8`（协议 4 里已无 `switch_to_manual/auto`）；
4. `pictor_ws_ctl.py` 能发一条 `manual`（`--dry-run` 级别即可，不必连车）。

---

### Step 4 — 真车：M1 车型 + `0x12` 通道 + 执行器替换 + 导航订阅建图增量

**目的**
把真车的行动模型换成 `(v, ω)`：`0x11` 全删、`0x12` 顶上；顺带把车型改对（M1），并把导航器的建图增量订阅接上。

**涉及文件**

修改：
- `pleiades-ugv/src/ugv/types.rs` —— `CarType::M1 = 0x0A`；`from_u8` / `from_str("m1")` / `motion_limits()`（限幅先按 X3Plus 同档 `(0.7, 0.7, 3.2)`，标定后修正）
- `pleiades-ugv/src/config.rs` —— `[chassis] car_type` 缺省改 **`"M1"`**；`forward_speed` / `turn_speed`（百分比）换成 `cruise_speed = 0.45` / `cruise_yaw_rate = 0.5`；`ensure_ugv_section` 的补全值同步
- `pleiades-ugv/src/ugv/stm32/mod.rs` —— 删 `forward/backward/left/right/spin_left/spin_right/apply_action/handle_manual_cmd` 的离散实现；新增 `apply_velocity(v: f32, omega: f32)`（组 `0x12` 帧）与 `stop()`（`0x12` 全零）；`set_motion` 与 `apply_velocity` 合并成一条
- `pleiades-ugv/src/ugv/stm32/protocol.rs` —— 删 `pack_car_run`；`pack_motion` 保留；`constants.rs` 里 `FUNC_CAR_RUN` / `MotionState` 删除
- `pleiades-ugv/src/ugv/executor.rs` —— 三状态机删掉，换成与 `pleiades-sim/src/sim/ugv_sim/executor.rs` **逐字相同**的连续执行器（只差 `use` 路径）
- `pleiades-ugv/src/ugv/robot_handler.rs` —— `on_tick` ④ 改为 `decide(...)` → `stm32.apply_velocity(v, omega)`；`handle_manual_cmd` 改为吃 `ManualCmd::Motion`；`stop()` / `reset` / `suspend` 的停车走 `apply_velocity(0, 0)`；**建完 `Navigator` 后补 `nav.subscribe_map(&self.robot.map_tx)`**
- `pleiades-ugv/src/ugv/emergency_stop.rs` —— 急停停车走 `apply_velocity(0, 0)`

**实施步骤**

1. 车型：加 `M1`，`from_str` 认 `"m1"`，缺省改 M1；`cargo test -p pleiades-ugv --bins types` 通过。
2. 驱动：`apply_velocity(v, omega)` → `pack_motion(car_type, v, vz, 0.0, limits)`，其中
   - `vx = v`（`0x12` 的 `vx` 本来就是 m/s，直接填）
   - `vz = 标定(omega, v)`：`vz` 与 `omega` **同号**（`vz > 0` = 左转 = 逆时针 = `omega > 0`）；
   - `vz` 不是角速度，所以要反解（2026-11-01 实测）：原地转死区 `|vz| ≤ 0.2`、`ω ≈ 40 °/s × (vz − 0.2)` ⇒ `vz = sign(ω)·(0.2 + |ω|/0.70 rad/s)`；**边走边转的 ω 约为原地转的 2.8 倍** ⇒ 按 `|v|` 选两套系数（不分段会让弧线转过头）；上限 `|vz| ≤ 3.2`（≈120 °/s 原地）。
   - 系数写死成常量并标注「待上车标定」，**不要**写成"以后再算"的空函数；`omega = 0` 必须发 `vz = 0`（不能发 0.2，否则车会一直转）。
3. 执行器：整份照抄 sim 车栈那份，只换 `use`；`decide` 输出 `VelocityCmd{v, omega}`；两个量级直接取自 `[chassis] cruise_speed` / `cruise_yaw_rate`（0.45 / 0.5），**不做百分比换算**。
4. `robot_handler`：五步结构不变（门闩 → 急停 → 树 → 决策 → 终值），只把第 ④ 步的执行调用换掉；`ExecuteState` 仍只写 `sub_target`。
5. 导航订阅：`Navigator` 建好后 `nav.subscribe_map(&self.robot.map_tx)`（方法已存在，当前是死代码）。
6. 清理：全仓 `grep` 确认 `pack_car_run` / `MotionState` / `apply_action` 在 `pleiades-ugv` 里已无引用。
7. 单测：执行器五档（沿用 sim 那几条，符号/`±π`/pass-through）；`CarType` 四位 + M1；急停停车路径。

**验收**
1. `cargo check -p pleiades-ugv --all-targets` 0 error、新增代码 0 warning；
2. `cargo test -p pleiades-ugv --bins` 全绿（types / executor / mission / nav）；
3. `grep -rn "0x11\|FUNC_CAR_RUN\|MotionState\|apply_action" pleiades-ugv/src` 为空；
4. `[chassis] car_type` 缺省值在 `config.rs` 与 `ensure_ugv_section` 两处都是 `"M1"`；`forward_speed` / `turn_speed` 已删除，`cruise_speed` / `cruise_yaw_rate` 缺省为 0.45 / 0.5；
5. `subscribe_map` 在 `robot_handler.rs` 有且仅有 1 处调用；
6. **不上车**：本步只做代码与单测。

---

### Step 5 — 真机：mavlink 换连续执行器

**目的**
把真机从"三状态机 + `MotionAction`"换成与 sim 机同形的 `(v, ω)`，并清掉 base 里最后一个 `MotionAction` 消费者。

**涉及文件**

修改：
- `pleiades-uav/src/uav/executor.rs` —— 三状态机删掉，换成与 `pleiades-sim/src/sim/uav_sim/executor.rs` 逐字相同的连续执行器
- `pleiades-uav/src/uav/mavlink/mod.rs` —— 删 `apply_action` / `set_desired(MotionAction)` / `handle_manual_cmd` 的离散分支；新增 `apply_velocity(v, omega)`：线速度取 `v`、偏航角速度取 `omega`（沿用现有 `vel_fwd` / `yaw_rate_deg` 的量纲换算）
- `pleiades-uav/src/uav/robot_handler.rs` —— `on_tick` ④ 改 `mavlink.apply_velocity(v, omega)`；`handle_manual_cmd` 吃 `Motion`
- `pleiades-base/src/Robot/core/state.rs` —— 删 `MotionAction` / `DecisionResult`（此时四端都已不用）；`ExecuteState` / `DecisionState` 保留（连续栈仍写 `state: Idle`）

**实施步骤**

1. 执行器照抄 sim 机那份（三档：直行 / 边走边转 / 原地转）。
2. mavlink：确认现有速度设定消息能同时表达"前进速度 + 偏航角速度"；若现有实现是"速度与偏航互斥"，改成同时下发。
3. 手动：`ManualCmd::Motion` → `apply_velocity`（与自动同一条出口）；`Takeoff` / `Land` 保持走原消息。
4. base 清理：删 `MotionAction` / `DecisionResult`，修掉所有引用（`use`、测试、文档注释）。
5. 单测：执行器三档 + 符号；`handle_manual_cmd` 的 `Motion` 路径。

**验收**
1. `cargo check -p pleiades-uav -p pleiades-base --all-targets` 0 error；
2. `grep -rn "MotionAction\|DecisionResult" pleiades-uav pleiades-ugv pleiades-sim pleiades-base/src/Robot` 为空；
3. `cargo test -p pleiades-uav --bins` 全绿；
4. 真机 `Takeoff` / `Land` 行为不变（代码路径未动）。

---

### Step 6 — 仿真世界验证（sim 车 + sim 机 + 手动两轴）

**目的**
模型换完之后，在仿真世界里确认四件事：车能按 `Goto` 走、机能飞、手动两轴能合成弧线、`Follow` / `Containment` 仍能跑。这是"行动模式转正"之后的回归验证。

**涉及文件**

- 运行环境（不入库）：`RobotSim_TestEnv/<node>/`（1 台 sim ugv + 1 台 sim uav + 世界）、节点 `.config/config.toml`
- 验证脚本：`tmp/task44_tools/`（本任务专用脚本，不进 `Tool/`）
- 产物：`RobotSim_TestEnv/results/task44_s6_<时间戳>/`（日志 + verdict）；记录进 `Workbook/wb_44_ugv_velocity.md`

**实施步骤**

1. 起世界 + 1 台 sim ugv + 1 台 sim uav。
2. 车：控制口注入 `goto`，确认能到点（`status` 里 `valid` 归零、日志有 `[Nav] 到达`）。
3. 机：同样注入 `goto`，确认能飞到点。
4. 手动两轴（三条，逐一确认）：
   - 只给 `v` → 直行；
   - 只给 `omega` → 原地转；
   - 同时给 `v` 与 `omega` → **走弧线**（这是本次改动的核心行为）；
   - 给 `(0, 0)` → 停。
5. `Follow` / `Containment` 各跑一次（sim 车已带 detector 的节点配置），确认没被这次改动打坏。
6. 记录：是否出现 `motion_cmd` 解析报错、世界侧是否有 `speed`/`turn` 残留字段读取。

**验收（以「行为出现」为准，不卡数值）**

> 轮次与失败处理（口径 #10）：本步**只跑 1 轮**；无论 PASS / FAIL 都记录 + 继续 Step 7。
> 例外：编译不过 / 单测红 / 启动即 panic 属代码级失败，回 Step 2 修。

**硬判据（必须出现，缺一才 FAIL）**

1. sim 车 `Goto` 到点，全程无 `panic`；
2. sim 机 `Goto` 到点；
3. 手动同时给 `v` + `omega` 时，车走出弧线（不是"只直行"或"只原地转"）；给 `(0, ±0.5)` 时原地转；
4. `(0, 0)` 后车停住；
5. `Follow` / `Containment` 仍能进入并保持（不要求跟得好）；
6. 日志与证据写入 `Workbook/wb_44_ugv_velocity.md`。

**软观测（只记录，不判 FAIL）**

- 弧线半径、过冲、到位误差、到位耗时；
- `motion_cmd` 的发送频率与位姿龄；
- 车/机的加減速是否与改动前有明显差别。

---

### Step 7 — 真车行为树：detector / `Target` / 协助请求 / Follow / Containment 复制过来

**目的**
真车的行动模型已与 sim 车对齐，把 sim 车栈的 `Follow` / `Containment` 整套（含 detector、`Target`、协助请求发布者）复制到 `pleiades-ugv`，让真车也真的能跟随与封控。

**涉及文件**

新建（**全部按 crate 自带一份，不进 base**）：
- `pleiades-ugv/src/device/detector/{mod.rs, projection.rs}` —— 复制 `pleiades-uav/src/device/detector/` 那两份（`projection.rs` 与 uav / sim 逐字节相同；`mod.rs` 只改帧来源与 `Frame` 别名）
- `pleiades-ugv/src/ugv/mission/follow.rs` —— 复制 `pleiades-sim/src/sim/ugv_sim/mission/follow.rs`（`Target` 与 `TargetFeed` **也随本文件复制**，不引 sim）
- `pleiades-ugv/src/ugv/mission/containment.rs` —— 复制 `pleiades-sim/src/sim/ugv_sim/mission/containment.rs`（含 `AssistPublisher` trait）

修改：
- `pleiades-ugv/src/ugv/mission/mod.rs` —— `TaskDeps` 加 `detector` / `assist`（均可为 `None`）；`Follow` / `Containment` 分派到真树；`None` 时退化为**原地 Hold**（恒 `Running` + `next_cell = None`），**不得**一拍 `Success`
- `pleiades-ugv/src/config.rs` —— 新增 `[detector]`（`enabled` 缺省 false / `model` / `device` / `conf_thres` / `nms_thres`）与 `[detector.camera]`（内外参与安装参数，键名与 uav 那份一致）；`ensure_ugv_section` 补全（缺才补、幂等）
- `pleiades-ugv/src/ugv/robot_handler.rs` —— `start()` 里起 detector（`[detector].enabled=false` → 零线程）；`TaskDeps` 组装时传入 `detector` 与 `assist`
- `pleiades-ugv/src/ugv/robot_handler.rs`（或同文件内）—— 复制 `NodeHandlePublisher`（`impl AssistPublisher`：`encode_assist_request` → `Gossipsub_Publish_Try(TOPIC_ROBOT_ASSIST_REQUEST, …)`，失败 1 s 限流 warning）
- `pleiades-ugv/src/bootstrap.rs` —— 相机 `Arc` 交给检测器（帧源 = `camera_ugv.latest_frame()`）

**实施步骤**

1. 先复制 `projection.rs`（纯函数，零改动）与 detector `mod.rs`（只改帧源适配与 `Frame` 别名、`frame_ts` 语义）。
2. 复制 `follow.rs`（含 `Target` / `TargetFeed` / `TargetSource` / `step_cell` / `guard_cell` / 四个常量），只改 `use`。
3. 复制 `containment.rs`（含 `AssistPublisher` / `corners` / `inside_square` / `intruder` / `ASSIST_MOVE_M=1.0`），只改 `use`。
4. `mission::build` 接真树：`detector = None` → Hold（与 sim 车同款退化），并补那条 `warn`。
5. 协助请求发布者：方法体与 sim 那份逐字相同，只换类型路径。
6. 配置与装配：`[detector]` 段 + `start()` 里起设备 + 注入 `TaskDeps`；失败只 `warn`，不影响其它设备。
7. 单测：无 detector → 两棵树都 Hold（`Running` 且 `next_cell=None`，连 3 拍）；`step_cell` 方向；`guard_cell` 人格否决；`containment::corners` 顺序；`InsideSquare` 的 1 m 重发阈值；`halt` 清 `last_sent`。
8. 评审：与 sim 那两份逐行 `diff`，确认差异只有 `use` 路径与文件头。

**验收**
1. `cargo check -p pleiades-ugv --all-targets` 0 error、新增代码 0 warning；
2. `cargo test -p pleiades-ugv --bins mission` / `--bins containment` 全绿；
3. 与 sim 对应文件 `diff` 只差 `use` 与文件头（评审逐行确认）；
4. `pleiades-base` 零改动（`git diff pleiades-base/` 为空）—— 本步不碰 base；
5. `[detector].enabled=false` 时**零线程**、`Follow`/`Containment` 建 Hold 树（不 `Success`、不 `Err`）。

---

### Step 8 — 真车端到端（静态验收 + 上车清单）

**目的**
真车这条链在本机没法跑（没有车），所以本步做两件事：静态验收所有改动的接线正确；给出一份可直接照着做的上车验证清单，由人类执行后回填。

**涉及文件**

- 代码：无新增（只做验收；若静态验收发现问题，回对应 step 修）
- 产物：`Workbook/wb_44_ugv_velocity.md` 的「上车清单」一节
- 文档：`Architecture/robot_arch.md`（`0x11` → `0x12`、M1 车型、树覆盖度、导航订阅）、`docs/design_doc/orion_protocol.md`（已在 Step 1 改完，本步复核）、`docs/ros_stm32_protocol.md`（`0x11` 标注废弃）

**实施步骤**

1. 静态接线检查（逐条给证据）：
   - 串口一条：`apply_velocity` → `pack_motion` → `spawn_port` 的 TX；确认每帧一次 `write`（固件要求）；
   - 手动一条：`msgid 4` 8 B → `ManualCmd::Motion` → `apply_velocity`；
   - 模式一条：`msgid 8` 1 B → `Command::Mode` → `main_loop`；
   - 急停一条：LiDAR 命中 → `apply_velocity(0, 0)`；
   - 停车一条：终值 / `reset` / `suspend` → `apply_velocity(0, 0)`。
2. 全仓残留扫描：`0x11` / `FUNC_CAR_RUN` / `MotionState` / `MotionAction` / `Apply_Velocity_Pct` / `speed`+`turn` 的旧用法，逐条确认已无（或写明保留原因）。
3. 写上车清单（每条都给"看什么、算过了算失败"）：
   - 车型：日志里 `car=M1`，且发出的 `0x12` 帧型号字节是 `0x0A`；
   - 前进：`(0.45, 0)` → 车前进（`vx` 就是 m/s；实测参考 0.3 → 0.275 m/s，约 8% 偏差）；
   - 后退：`(-0.45, 0)` → 后退；
   - 原地转：`(0, ±0.5)` → 左 / 右原地转（对照 `spin_left/spin_right` 的物理方向）；
   - 弧线：`(0.45, +0.5)` → 左前弧（实测同一 `vz` 下边走边转约为原地转的 2.8 倍，由驱动反解吸收）；
   - 停车：`(0, 0)` → 车轮停；
   - 手动：Pictor 按住 W 再按 A → 边走边转；松开 A → 恢复直行；全松 → 停；
   - 模式：Pictor 切手动/自动 → 车端日志显示 `Command::Mode`，Auto 模式下手动指令被忽略；
   - 导航：出任务时地图新增的墙能让规划改路（订阅生效）。
4. 把清单与"本机已验/待上车"的边界写进 workbook。
5. 文档同步（`robot_arch.md` 的通道、车型、树覆盖度；`ros_stm32_protocol.md` 标注 `0x11` 废弃）。

**验收**
1. 上述 5 条接线每条都能指出**唯一**的实现位置（文件 + 行）；
2. 残留扫描 0 命中（或每条保留都有书面理由）；
3. 上车清单每条都有可观察判据，不含"应该没问题"这类描述；
4. `cargo check -p pleiades-{base,ugv,uav,sim} --all-targets` 全部 0 error；
5. 文档三处（`robot_arch.md` / `orion_protocol.md` / `ros_stm32_protocol.md`）与代码一致。
