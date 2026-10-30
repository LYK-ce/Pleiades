# task_40_ugv_motion — sim 车栈执行器：离散动作 → 连续动作

> Presented by KeJi
> Created Date ： 2026-10-29
> Modified Date ： 2026-10-29
> 状态：**全部完成**（Step 1~5：2026-10-29 均 PASS；Step 6：2026-10-30 死区 5°→8° 上调 + 复验，按人类裁决 PASS）
> 参考：`Task/archived task/task_39_executor_and_path.md`（机栈同类改造，本文档逐节对齐它的做法）

---

## 一、目标

把 `pleiades-sim` **车栈**的执行器从「到格心 + 原地转向」的离散动作，改成与 **sim 机栈 `uav_sim/executor.rs` 同一套逻辑**的连续动作（直行 / 边走边转 / 原地转，每拍下发一次）。

**差异只有一处：量纲。** 机栈用物理量（`[flight_ctrl]` 的 `vel_fwd` m/s、`yaw_rate_deg` °/s）；车栈**沿用现有参数** `[chassis] forward_speed / turn_speed`（0~100 整数百分比，缺键默认 30 / 10），百分比→物理量的换算留在世界侧，sim 侧不抄世界的 `max_speed` / `max_turn_rate`。

两条原则（沿用 task_39）：

- **先补"说法"，再补"行为"**：停走不是"车反应慢"，是 `DiffDrive.gd:97-109` 里 `speed` 与 `yaw` **互斥**（`forward` 清 yaw、`left/right` 清速度）。说法不补，改参数只会变成另一种抖。
- **并存式加法**：新分支/新字段/新文件全部是"额外加"，旧的离散路径一行不改，之后再整体替换。

**五步一览**（Step 1~4 是主线，Step 5 是顺手的外观小事，互不阻塞）：

| Step | 做什么 | 触到哪一层 |
|---|---|---|
| 1 | 车栈执行器改成与 sim uav 同一套三档逻辑（沿用百分比档位） | 层 2（sim ↔ 世界） |
| 2 | 验证：最新的小车能不能正常运动（只验不改） | — |
| 3 | Pictor 加 WS 命令通道，让 Python 进程能发命令、读事件 | 层 1（ORION / libp2p） |
| 4 | `Follow` 进 Pictor，并用 Pictor 下发跟随（只验直走，失败不阻塞） | 层 1 + 层 2 |
| 5 | 四旋翼外观换成像样的免费模型 | 纯资产 |

---

## 二、Step 1 — 车栈执行器改成与 sim uav 同一套逻辑

### 1 要做什么（五条）

| # | 项 | 内容 |
|---|---|---|
| 1 | 新增车栈连续执行器 | `pleiades-sim/src/sim/ugv_sim/executor.rs`，三档控制律，输出**档位**（前进 pct / 转向 pct）；`sim/executor.rs`（286 行离散三状态机，与 `pleiades-ugv` 逐字相同）**一行不改** |
| 2 | 世界侧加分支 | `DiffDrive.gd` 的 `SetCommand` 加 `"velocity"`：**前进档位与转向档位同时生效**；5 个离散 action（forward/backward/left/right/stop）保留 |
| 3 | 线上加字段 | `MotionCmd` 加 `turn: Option<i16>`（−100..+100，带符号转向档位），`skip_serializing_if` ⇒ 旧帧序列化逐字不变；前进档位复用现有 `speed: u8` |
| 4 | 接线 | `DiffDriveChassis::Apply_Velocity`（新增）+ `MotionSender::Send_Velocity_Pct`（新增）+ `sim_handler` 按执行器选型分派；`Apply_Velocity` 返回 `false` 时**停车 + 限流告警**（与机栈同款，不自动回落） |
| 5 | 转向速度调大 | `[chassis] turn_speed`：`10`（= 9 °/s）→ 初值 `30`（= 27 °/s），**数值可调**，见 §2.4.5 |

### 2 涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `pleiades-sim/src/sim/ugv_sim/executor.rs` | **新增** | 连续档控制内核 + 单测 |
| `pleiades-sim/src/sim/chassis/ugv.rs` | 改 | `DiffDriveChassis::Apply_Velocity`（~10 行） |
| `pleiades-sim/src/world_link/mod.rs` | 改 | `MotionSender::Send_Velocity_Pct(speed: u8, turn: i16)`（照 `Send_Velocity` 同构） |
| `pleiades-sim/src/world_link/protocol.rs` | 改 | `MotionCmd` 加 `turn` 字段 + 序列化单测（锁旧帧逐字不变 / 新帧带 `turn`） |
| `pleiades-sim/src/sim/sim_handler.rs` | 改 | 执行器选型与分派（车 → 连续、机 → 现有）；车栈日志加节流（可选，Step 2 取数用） |
| `pleiades-sim/src/sim/ugv_sim/mod.rs` | 改（按需） | 执行器参数装配（若挂在栈上） |
| `orion-simulation-world/src/chassis/DiffDrive.gd` | 改 | `"velocity"` 分支 + 文件头 action 表补一行 + 自检 |
| `RobotSim_TestEnv/*/.config/config.toml` | 改 | 车实例 `turn_speed` 调到新值 |
| `Architecture/robot_arch.md` | 改 | 见 §2.6 |

**一行不动**：`pleiades-base`（`ExecuteState` / `MotionAction` / `DeviceHandler`）、`pleiades-ugv`、`pleiades-uav`、`pleiades-terminal`、`sim/executor.rs`、`sim/*/nav/`、`sim/*/mission/`、`Mission` / 队列 / ORION 协议、手动遥控链。

### 3 控制律（与 `uav_sim/executor.rs` 同构）

```text
每拍（50 ms）：
  target = next_cell 的格心；None → (speed=0, turn=0)
  Δθ     = normalize(atan2(ty−y, tx−x) − yaw)        // 顺时针为正（世界 yaw 约定）

  if |Δθ| ≤ DEADBAND_DEG:        speed = forward_speed, turn = 0            // 直行
  elif |Δθ| ≤ ARC_MAX_DEG:       speed = forward_speed, turn = ∓turn_speed  // 边走边转
  else:                          speed = 0,             turn = ∓turn_speed  // 原地转
```

- **符号（本文件最容易写错的一处，证据链与机栈相同）**：
  `Δθ > 0`（目标在右）⇒ 世界 yaw 增大（顺时针为正）⇒ `rotation.y` 减小（`common/WorldTransform.gd`：`YawToGodot = −world_yaw`）⇒ 世界 `"velocity"` 分支**直接取 turn 的符号、不取反** ⇒ **`Δθ > 0` 时 `turn = −turn_speed`**。
  与 `sim/uav_sim/executor.rs` 文件头的推导同源（`action_map.rs:21-24`：`TurnRight → "right"`），但车/机是世界侧两份脚本，**符号单测要各写一条**。
- `speed` / `turn` 均为 0 时即停车（不必另发 `stop`）；`next_cell = None` 时就发 `(0, 0)`。
- 无状态、零副作用：只返回档位，不写 `ExecuteState`、不发命令。`ExecuteState` 的写点仍由 `sim_handler` 单点负责。

### 4 数值（全部可调，附算术依据）

#### 4.1 三条约束（不是口味，是算出来的）

1. **死区 ≥ 每"可见"拍转角**：位姿 10 Hz ⇒ 每拍转 `yaw_rate × 0.1 s`。task_39 第二轮就是踩这条（机上 90 °/s × 75 ms = 6.8° > 死区 5° ⇒ 极限环，`omega` 翻转 35 次）。
2. **弧线档宽度 ≥ 2 个"可见"拍** 且 **弧线半径可接受**：`R = v / ω = (forward% × 1.5) / (turn% × 90°/s)`。
   task_39 的 `L9b`：机上 `R = 1.53 m` 远大于 0.5 m 格 ⇒ 拐角 `omega` 翻转 29 次。
3. **世界侧 clamp**：`max_speed = 1.5 m/s`、`max_turn_rate = 90 °/s`（`DiffDrive.gd:44-45`），超了**静默截断**；百分比 ≤ 100 天然满足。

#### 4.2 初值

| 常量 | 初值 | 依据 |
|---|---|---|
| `DEADBAND_DEG` | **8.0** | 过冲 ≈ ω × 0.15 s = 4.0°（尖峰 4.8°）⇒ 死区须 ≥ 过冲；**Step 6 由 5.0 上调**（§七.3） |
| `ARC_MAX_DEG` | **12.0** | 死区 8° 之后弧线档留 4° 宽；**Step 6 由 10.0 上调** |
| `forward_speed` | 30（不改） | 0.45 m/s |

#### 4.3 车版独有的一档判断

`next_cell` 的格心可能落在**车身后方**（导航给的下一格常常就是车身所在格或其邻格，车已越过格心）。离散执行器此时会掉头 180° 再走回来（实测目标方位 −3° → −45° → +37°，`TurnLeft` 紧跟 `TurnRight`）。连续档加一条：

```text
if d < PASS_THROUGH_M && |Δθ| > 90°:  speed = forward_speed, turn = 0   // 直行穿过，等 nav 换格
```

`PASS_THROUGH_M` 初值 **0.5 m**（= 1 格）。这条只在"目标在身后且很近"时生效，不影响正常转向。

#### 4.4 `omega` 每拍转角（供 §4.5 取舍参考）

| `turn_speed` | ω | 每拍（位姿 10 Hz）转角 | `R = 0.45/ω` | 90° 拐角耗时 |
|---|---|---|---|---|
| 10（现状） | 9 °/s | 0.9° | 2.86 m | 10 s |
| **30（初值）** | 27 °/s | 2.7° | 0.95 m | 3.3 s |
| 50 | 45 °/s | 4.5° | 0.57 m | 2 s |
| 60 | 54 °/s | 5.4° | 0.48 m | 1.7 s |
| 100 | 90 °/s | 9.0° | 0.29 m | 1 s |

> 说明：只要 `R` 仍 ≫ 0.5 m 格，弧线档就**只能用来修小偏角**，拐角仍靠原地转。要让弧线真正可用，`turn_speed` 需 ≥ 60（`R ≤ 半格`），但那时死区必须同步抬到 7~8°（§4.1 第 1 条）。这两个数是绑在一起的，改一个必须重算另一个。

#### 4.5 `turn_speed` 初值

`30`（27 °/s，90° 拐角 10 s → 3.3 s，`R = 0.95 m`）。**可调**：若要弧线档可用取 60，若要极快取 100；同时按 §4.1 调整 `DEADBAND_DEG`。

### 5 世界侧（`DiffDrive.gd`）

`SetCommand`（`:93-113`）加一个分支，让两个档位**同时生效**：

```gdscript
"velocity":
    # 车栈连续档：前进档位与转向档位同时生效（唯一的新行为 = 解除互斥）
    _speed_target = _max_speed * clampf(float(cmd.get("speed", 0)) / 100.0, 0.0, 1.0)
    _yaw_target   = _max_turn_rate * clampf(float(cmd.get("turn", 0)) / 100.0, -1.0, 1.0)
```

- 百分比→物理量的换算**仍留世界侧**：sim 侧不知道也不用知道 `1.5` / `90`。
- `"velocity"` 的字段语义**按底盘解释**：机栈读 `v` / `omega`（物理量），车栈读 `speed` / `turn`（档位）；两个底盘脚本不共享解析代码，所以同一个 action 字面量不构成冲突。
- 5 个离散分支与 `_` 兜底**一个字不改**。
- 文件头 action 表（`:23-29`）补 `velocity` 一行，写明"`yaw` 是 `rotation.y` 角速度（正 = 左转），世界 yaw = −rotation.y"。
- 自检：加一条 —— 发 `velocity{speed:30, turn:-50}` 后 `Step()` 的 `velocity` 与 `yaw_rate` 都不为 0 且符号符合预期（**这条是"解除互斥"的唯一证据**）。

### 6 线上字段（`protocol.rs`）

```rust
pub struct MotionCmd {
    pub msg: &'static str,
    pub seq: u64,
    pub action: &'static str,
    pub speed: u8,                       // 前进档位（车）/ 旧离散档位
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v: Option<f32>,                  // 机栈：m/s（task_39）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omega: Option<f32>,              // 机栈：rad/s（task_39）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<i16>,               // 车栈：转向档位 −100..+100（本任务）
}
```

单测（照 task_39 的两条）：① 三个可选字段全 `None` 时序列化**逐字**等于旧帧；② `turn` 为负/正/0 各一条；③ `speed` 仍夹 `0..100`。

### 7 完成判据

1. 新增执行器单测 ≥ 8 条：直行 / 右弧 / 左弧 / 原地右 / 原地左 / 无目标 → `(0,0)` / 身后格心直行 / `Δθ = ±π` 归一化。
2. 符号单测：`Δθ > 0` ⇒ `turn < 0`（与 `WorldTransform.gd` 的 `YawToGodot = −world_yaw` 一致）；Godot 自检里再锁一条。
3. `cargo check --all-targets` 零 error、新增代码零 warning；`cargo test -p pleiades-sim --bins` 全绿（现有 410 passed / 2 failed 的 `MountOut.pitch` 遗留不属本任务）。
4. 协议单测：旧帧序列化逐字不变（§2.6 三条）。
5. `sim/executor.rs` 与 `pleiades-ugv/src/ugv/executor.rs` 仍**逐字相同**（`diff` 无输出）。
6. `git diff --stat` 只含 §2.2 表内文件；`orion-simulation-world` 只差 `DiffDrive.gd`。
7. Godot 自检：`velocity{speed:30, turn:-50}` 下 `velocity.z` 与 `yaw_rate` 同时非零，符号符合预期。

### 8 不做（Step 1）

- 不改 `sim/executor.rs`、不改真机 `pleiades-ugv`、不改 ORION 协议；
- 不动 `nav/`、`mission/`、行为树形状；
- 不加配置开关（回退方式 = `Apply_Velocity` 返回 `false` 自动回落离散；要 A/B 就回退代码）；
- 不做路径平滑 / 前瞻格 / 倒车；
- 不清死代码。

---

## 三、Step 2 — 验证：最新的小车能不能正常运动

### 1 目标

回答一个问题：**换成连续执行器之后，车还能不能正常走完**（到达、不抖、不撞墙、该停的时候停）。
**只验不改**：不调阈值、不改代码，不通过就如实写结论。

### 2 用例

| 用例 | 场景 | 看什么 |
|---|---|---|
| M0 静止 | 不发任务 | 全程 `speed = turn = 0`，车不动 |
| M1 直线 | 正东 20 m | 到达；无停段；末位误差 |
| M2 L 形 | 需一次 90° 拐 | 到达；拐角不反复停走 |
| M3 长路径 | 40~50 格 | 到达；无 `Blocked`、无急停 |
| M4 斜线 | 45° 方向 20 m（8 连通后的主导形态） | 到达；`omega` 不来回翻号 |
| M5 窄通道 | 1 格宽走廊 + 转角 | 到达；**不贴墙**（最小 `lidar` 距离） |

### 3 判据

| # | 判据 | 口径 |
|---|---|---|
| 1 | 到达 | nav 判 `Arrived`，`TickOutcome::Terminated(None)`；末位逐轴 ≤ 0.26 m（sim 到达口径） |
| 2 | 不抖 | **同一格内**相邻非零 `turn` 反号次数（M1 ≤ 2、M4 ≤ 3）；**换格那一拍**的反号属结构性的方向切换，只记录不设限（2026-10-30 人类裁决，依据见 §七.10） |
| 3 | 不打转 | 10 s 内净位移 ≥ 2 m（M1/M4） |
| 4 | 不撞墙 | 全程最小 `lidar` 距离 ≥ 0.3 m |
| 5 | 该停会停 | M0 全程 `(0,0)`；任务结束后 1 s 内 `(0,0)` |
| 6 | 停走对比 | `(0,0)` 拍数 / 总拍数 **记录并与改前同一路径对比**（不设百分比红线，只记录） |
| 7 | 日志可解析 | `[Sim] UGV` 行逐行解析成功；解析数 ≠ 行数 ⇒ 该判据记 `FAIL(env)` |
| 8 | 无回归 | 无 `Blocked`、无急停触发、无 `[Sim] 任务失败` |

### 4 取数与产物

- 环境：Godot 世界 + `Xvfb`，用 **`RobotSim_TestEnv/world_fast/`**（`msaa_3d=0`，task_39 实测 2 fps → 11 fps）；`llvmpipe` 下 2 fps 时位姿龄 245 ms，会让判据失真。
- 取数：`[Sim] UGV` 日志（`speed`/`turn` 两列）+ 位姿序列；物理时间用位姿时间戳，不用墙钟。
- 产物：`RobotSim_TestEnv/results/task40_<UTC>/{trace.csv, world.log, sim.log, verdict.md}`；结论写 `Workbook/wb_40_ugv_motion.md`。
- 脚本：新建 `Tool/sim_task40_ugv_test.py`，控制口/起停/日志解析复用 `Tool/sim_queue_test.py`、`Tool/sim_follow_test.py` 的现有设施。

### 5 不做（Step 2）

- 不调阈值、不改代码（**只验不改**；不通过就写结论）；
- 不做"改前 vs 改后"的并排 A/B 脚本（§3 第 6 条只记录同一路径的两组数）；
- 不测多车、不测跟随、不测真机。

---

## 四、Step 3 — Pictor 无头驱动：WS 命令通道 + Python 控制脚本

### 1 目标

**本步只做一件事：让外部 Python 进程能通过 Pictor 下发命令、读回遥测。** 用**现有** ORION 消息（`TASK_SET` type=0 Goto / 模式），不新增协议、不碰 `Follow`（那是 Step 4）。

**不动 executor**：executor 的输出（`motion_cmd`）只活在 sim 进程内，Pictor 既不产生也不需要它。

### 2 两层定位（本步的前提，写下来避免再混）

```text
【层 1｜ORION / libp2p】Pictor ──► 车/机端（sim 进程）
    只承载：任务 TASK_SET / 模式 / 离散手动 MANUAL_CONTROL
    ← 上行：POSE、MAP_DELTA、peer_*
         ▼  车端（sim 进程内）
【层 2｜motion_cmd JSON / WS】决策链 ──► Godot 世界
    MissionQueue → 行为树 → nav(D*) → executor（Step 1 的新执行器）
    executor 输出 speed/turn ← 只在这层，Pictor 看不到
    ← 位姿 10 Hz
```

- Auto 模式下 Pictor 只发 `TASK_SET`(type=0 Goto)，其余全在 sim 进程内发生 → **Step 1 改执行器不影响层 1**。
- **本次不管手动模式**（Auto only）：手动遥控仍走层 1 的 `MANUAL_CONTROL` → 层 2 的离散 `action`，绕过 executor，现状零改动。

### 3 要做什么（五条）

| # | 项 | 内容 |
|---|---|---|
| 1 | Pictor 内 WS 服务（GDScript） | `WebSocketPeer`/`TCPServer`，在 `_process` 里 poll（不阻塞主循环）；收到命令 → `EventBus.cmd_send.emit(targets, cmd_dict)`，`cmd_dict` 就是 `Build_Cmd` 吃的 `{msgid, ...}` 形状，**Python 不需要懂 ORION 字节** |
| 2 | 上行事件转发 | 订阅 EventBus 的 `pose_received` / `map_delta_received` / `map_full_received` / `peer_*` → JSON 推给客户端；带**订阅开关 + 降频**（pose 是 10 Hz × N 车，直连会变火管） |
| 3 | 车列表查询 | 暴露"已注册车：车名 + peer_id + node_type"，供 Python 填 `targets`（`targets` 是**车名**，`auto_handler.gd:64` 做车名→vehicle_id） |
| 4 | 无头启动 | `godot --headless --path Pictor` 起来后 WS 可连、kernel bootstrap 与 mDNS 发现正常（主场景无头冒烟在 `Pictor/Task/task_24_circle_command.md:249` 有先例） |
| 5 | Python 控制脚本 | **产出到 `Pictor/tool/`**（与 `stt_server.py`、`tile_cut.py` 同级），依赖写进 `Pictor/tool/requirements.txt` |

### 4 涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `Pictor/src/kernel/ws_command_server.gd` | **新增** | WS 服务节点：连接管理、命令下行、事件上行、车列表查询 |
| `Pictor/src/main/main.tscn` / `main.gd` | 改 | 挂节点 + 启动参数（端口/开关） |
| `Pictor/src/vehicle_registry/`（现有） | 只读复用 | 车列表数据源（不新增状态） |
| `Pictor/tool/pictor_ws_ctl.py` | **新增** | Python 控制脚本：等注册 → 发任务 → 收事件 → 判定 |
| `Pictor/tool/requirements.txt` | 改 | 加 `websockets`（若已有则不重复） |
| `Pictor/docs/project_code_overview.md` | 改 | 补一节「WS 命令通道」 |

**一行不动**：`pleiades-base`、`pleiades-ugv`、`pleiades-uav`、`pleiades-terminal`、`pleiades-sim`、`orion-simulation-world`、`kernel_bridge.gd`、`orion_messages.gd` 的 `Build_Cmd`（沿用）。

### 5 WS 消息格式（对外契约）

下行（Python → Pictor）：

```json
{"id": 1, "cmd": "vehicles"}
{"id": 2, "cmd": "task_set", "targets": ["ugv_1"], "missions": [{"type": 0, "x": 70.0, "y": 64.0}]}
{"id": 3, "cmd": "mode",     "targets": ["ugv_1"], "to": "auto"}
{"id": 4, "cmd": "subscribe", "events": ["pose", "peer"], "pose_hz": 2, "targets": ["ugv_1"]}
```

上行（Pictor → Python）：

```json
{"ack": 1, "ok": true}                       // 仅表示「已交给 kernel」，**不代表车端收到**
{"event": "vehicles", "list": [{"name": "ugv_1", "vid": "...", "node_type": "car"}]}
{"event": "pose", "vid": "ugv_1", "x": 64.2, "y": 63.9, "yaw": 0.12,
 "valid": 1, "gx": 128, "gy": 128, "rtk_fixed": 0}
{"event": "peer_connected", "vid": "...", "name": "ugv_1", "node_type": "car"}
```

⚠️ **`ack` 不是送达确认**：车端 ACK 恒回 `b"OK"` 且发送方不读（`pleiades-terminal` 侧不发结果）。判定"命令生效"只能靠上行事件（`pose` 的 `valid/sub_gx/sub_gy` 变化）或 sim 日志。

### 6 完成判据

1. **无头可跑**：`godot --headless --path Pictor` 启动后 Python 能在 10 s 内连上并查到车列表，Godot 侧无脚本错误。
2. **单机闭环**：发 `task_set` → sim 日志出现任务入队与决策链推进 → `pose.sub_target` 变化 → 到达。
3. **多车**：两台 sim（两 ugv 或 ugv+uav）各收到**自己**的目标点（逐车发帧语义，`missions[0]` 即该车目标）。
4. **上行可用**：`pose` 可按 `pose_hz` 降频、按 `targets` 过滤；`peer_*` 可读；订阅关闭时不推送。
5. **健壮性**：客户端断开 Pictor 不崩；重连后可继续；未注册车时 `task_set` 返回明确失败（不静默）。
6. **零协议改动**：只用现有 msgid（`TASK_SET` / `MANUAL_CONTROL`），`git diff` 不含 base/ugv/uav/terminal/sim/世界。
7. **脚本自洽**：`pictor_ws_ctl.py` 一次运行完成「等注册 → 发任务 → 收 pose → 判到达 → 退出码 0/1」。

### 7 不做（Step 3）

- 不新增 ORION 消息（`ORION_VELOCITY_CMD` 见 §八 后续）；
- **不放开 `Follow`**：Pictor 仍把 type≠0 强制降级成 Goto —— Step 4 再做；
- 不管 Manual 模式；
- 不动 executor / nav / 世界 / 协议编解码；
- 不做鉴权（只绑 `127.0.0.1`）、不做 TLS。

---

## 五、Step 4 — `Follow` 进 Pictor，并用 Pictor 下发跟随

### 1 目标

把 `Mission::Follow`（type=2）从"只有协议和 sim 控制口认识"补到**地面站能发**：Pictor 侧放开 type=2，然后用 Step 3 的 WS 通道下发 `Follow`，让 **sim 机栈的真跟随**（`uav_sim/mission/follow.rs`）跑起来。

### 2 现状（为什么现在发不出去）

| 位置 | 现状 |
|---|---|
| `Pictor/src/websocket/protocol/protocol_def.gd:48` | 只有 `MISSION_TYPE_GOTO := 0`，注释明确写"Pictor 侧只生成 0" |
| `protocol_def.gd` 的 `Mission_Type_From()` | `t is String` 一律返回 `GOTO`（`"follow"` 也变 0） |
| `orion_messages.gd:181-186`（`Encode_Task_Set`） | 非 0 类型**强制降级成 0** 并告警 |
| `message_builder.gd:88`（`_Normalize_Mission_Type`） | 同上，另一处归一化 |
| 协议/base/sim | ✅ 早已支持（`MISSION_FOLLOW = 2`、`decode_task_set` 解 2、`Mission::Follow{x,y}`） |

### 3 要做什么

| # | 文件 | 改动 |
|---|---|---|
| 1 | `protocol_def.gd` | 加 `MISSION_TYPE_FOLLOW := 2`；`Mission_Type_From()` 认 `"follow"` / `2`；更新"MISSION_TYPE_GOTO"上方那段注释（结论已变） |
| 2 | `orion_messages.gd`（`Encode_Task_Set`） | 降级条件从"非 0 一律"改为"**不在 {0,2}** 才降级并告警" |
| 3 | `message_builder.gd`（`_Normalize_Mission_Type` / `build_task_set`） | 同步放开 type=2 |
| 4 | `Pictor/tool/pictor_ws_ctl.py` | 加 `follow` 子命令：`{"cmd":"task_set","missions":[{"type":2,"x":0,"y":0}]}` |
| 5 | 文档 | `Pictor/docs/project_code_overview.md` 或 `Architecture/robot_arch.md` 记一句"Pictor 支持 type=2" |

**不动**：`pleiades-base`（`MISSION_FOLLOW` 已在）、`pleiades-sim`（`follow.rs` 已在）、`pleiades-uav`/`pleiades-ugv`（`Follow` → 平凡树是 task_37 的既有设计，本步不改）。

### 4 验证前置（sim 机栈真跟随的依赖，缺一跑不动）

- sim 实例为 **机栈**（`[sim_world].chassis = "quadrotor_uav"`），且 `[detector] enabled = true` + 相机段（`RobotSim_TestEnv/sim1` 已具备，见 task_37）；
- 世界侧相机 `[[sim_world.sensor]] sensor = "camera"`，`mount` 与 `[detector.camera]` 两份手抄数值必须一致（task_37 §8.8 已登记的坑）；
- **行人源**：`orion-simulation-world/tool/pedestrian/*.py`（世界不内建行人，靠外部客户端 spawn）；
- 世界用 `RobotSim_TestEnv/world_fast/`（`msaa_3d=0`，否则 2 fps + 位姿龄 245 ms 会让跟随判据全失真）。

### 5 完成判据

1. **编码正确**：Pictor 发出的 `TASK_SET` 里 `missions[i].type == 2`（`orion_messages.gd` 单测 + Python 侧断言帧字节）。
2. **车端解得出**：sim 日志出现 `Mission::Follow` 入队（不是被 `command_decode` 丢弃的 warn）。
3. **机栈真跟（只验"直走"一种场景）**：行人沿直线走，机跟随 —— 判据只用 task_37 的 F1 口径：10 s 内机位移 ≥ 5 m、距离不持续拉大、不原地打转。
4. **未知类型仍被丢**：`type=3` 仍降级/丢弃并告警（不能因为放开 2 就把校验放松）。
5. **零回归**：`type=0` Goto 与 Step 3 的行为逐条一致。
6. **车栈按设计走平凡树**：同一帧发给 ugv 时，车端一拍 `Success` + warn（这是既有设计，不是 bug）。

**失败处理（人类 2026-10-29 决定）**：

- **只验直走**（不做切线通过 F2、不做丢失 F3）。当前环境不适合验跟随场景：`llvmpipe` 下相机帧率与 detector 推理（~300 ms/帧 → 检测链上限 ~3.3 Hz）叠加行人速度，切向场景本就不可靠（task_39 §7.3 已登记）。
- 判据 1/2/4/5/6 **必须 PASS**；**判据 3 允许 FAIL**：FAIL 时只记录现象与日志，**不阻塞、不修、继续后续 step**。
- **跟随的最终验证由人类自己做**（本项目例只证明"Follow 能从 Pictor 发出去并被车端解开"）。

### 6 不做（Step 4）

- 不做**真机**跟随（`pleiades-uav` 的 `Follow` 仍是平凡树，需要 detector 消费者 + 安全距离约束，单开任务）；
- 不做车栈跟随（车没有相机/detector；`Follow` 在车上会永远占队首，task_37 已定为平凡树）；
- 不改 `Follow{x,y}` 的语义（`x/y` 仍预留填 0）；
- 不做多目标、不做前瞻目标点；
- 不加 UI 按钮/LLM 词表（见 §九 #6）。

---

## 六、Step 5 — 四旋翼外观：换一个像样的免费模型（顺手小事，不阻塞前四步）

### 1 目标

现在世界里的四旋翼**太丑**：换成一个"像样"的免费模型，落到 `assets/`，并真正在 `Vehicle` 上生效。

### 2 现状（"丑"是两件事叠加）

| 事实 | 位置 |
|---|---|
| UAV 模型是一份 **20 460 B** 的占位低模，2026-10-22 落库后没再动过 | `orion-simulation-world/assets/uav_quadrotor_lowpoly.glb` |
| 它被挂成 `Vehicle` 的 `ModelUAV` | `src/scenes/Vehicle.tscn:16` |
| 底盘组件 `Quadrotor.tscn` **只有 4 个 `CylinderMesh` 桨盘**（r=0.12 / h=0.02），没有机身 | `src/scenes/chassis/Quadrotor.tscn:16-49` |
| 对照：UGV 早在 task_33 就换成了 `assets/chassis/ugv_chassis.glb`（21 888 B，Blender 自制） | `assets/chassis/` |

**自检已经把 UAV 外观钉死了，换模型必须同步改**（否则红灯）：

| 常量 / 断言 | 位置 | 值 |
|---|---|---|
| `UAV_MODEL_SCENE` | `src/tests/check_models.gd:36` | `res://assets/uav_quadrotor_lowpoly.glb` |
| `UAV_SIZE_XZ` / `UAV_SIZE_Y_MIN..MAX` | `:46-48` | `0.64` / `0.11 ~ 0.15` |
| `UAV_HULL`（碰撞盒） | `:54` | `Vector3(0.40, 0.12, 0.40)` |
| `M4` 机头断言 | `:68` 附近 | 第 2 个 surface 材质名 `UAV_NoseRed` |
| `UAV_GLB` | `src/tests/check_sensor_models.gd:31` | 同路径 |

### 3 选型（已定）

**标准**（按优先级）：

1. **许可**：CC0 最优（仓库约定「除 `humans/` 外全部 CC0」）；**CC-BY 可接受**（人类 2026-10-29 确认：本项目非商用，但**仍须**落 `LICENSE` + `ATTRIBUTION.md` + 台账 + 说明改动）；**排除** `CC BY-SA`（同许可传染）、`CC BY-NC`（NC 定义模糊，公开/演示/论文配套都可能踩线）、`personal use only`（Free3D/TurboSquid 常见）、以及任何**品牌机型的翻制模型**（DJI 等，外观设计 + 商标风险）。
2. **形态**：真实四旋翼——有机架、机臂、电机、起落架，不是方块拼接；与 Renderpeople 写实人形同框不违和。
3. **规模**：≤ 50k tris、glb 单文件、贴图 ≤ 2K（世界跑在 `llvmpipe`，`world_fast` 基线只有 11 fps）。
4. **尺寸**：可缩放，但**目标 AABB `0.64 × 0.12~0.15 × 0.64`**（与现有碰撞体一致 ⇒ 动力学零改动）。
5. **口径**：朝 **+X**、底面 `y=0`（与 UGV / human 资产同一套口径）。
6. **红色机头**：保留 `UAV_NoseRed` 第二 surface，或同步改自检。

**来源**（按顺序尝试）：

| 顺序 | 站点 | 许可 | 备注 |
|---|---|---|---|
| 1 | [Poly Pizza](https://poly.pizza) | CC0 / CC-BY，可按许可过滤 | 直下 `.glb`，面数低 |
| 2 | [Sketchfab](https://sketchfab.com)（Downloadable + CC0） | 逐条看 | 质量更高，但要逐条核许可，避开品牌机型 |
| 3 | [Kenney](https://kenney.nl/assets) | CC0 | 风格化，与写实人形略违和，仅兜底 |
| 4 | **Blender 自制** | 自有 | UGV 先例（`Tool/blender/models/ugv_chassis.blend`）——前三家都没有满足的件时用它 |

**下载代理（本机实测可用）**：mihomo 常驻进程 `./mihomo -d /vepfs-mlp2/c20250205/240804016/tool/clash`，配置 `config.yaml`：`mixed-port: 7897`（HTTP + SOCKS5 同端口）、`external-controller: 127.0.0.1:9090`、`bind-address: 127.0.0.1`（只监听本机）。

```bash
# 单次下载
curl -x http://127.0.0.1:7897 -L --fail -o <out> <URL>
# 或整段会话
export http_proxy=http://127.0.0.1:7897 https_proxy=http://127.0.0.1:7897
# SOCKS5 亦可（同一端口）
curl -x socks5h://127.0.0.1:7897 -L --fail -o <out> <URL>
```

实测：`curl -x http://127.0.0.1:7897 https://api.github.com/rate_limit` → `200`（0.66 s）。
⚠️ 本机**直连也通**，代理只用于直连失败的站点；**不要**默认把代理写进脚本或 CI。

### 4 涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `orion-simulation-world/assets/chassis/uav_quadrotor.glb` | **新增** | 归一后的模型（新名去掉 `lowpoly`；旧件删除，git 可回溯） |
| `orion-simulation-world/assets/chassis/uav_quadrotor.glb.import` | **新增** | 由 `--import` 生成，**必须一起提交**（E28） |
| `orion-simulation-world/assets/README.md` | 改 | 台账加一行：来源 URL / 许可 / 处理 / 字节 / sha256 |
| `orion-simulation-world/assets/ATTRIBUTION.md`（仅 CC-BY） | 新增 | 署名 |
| `orion-simulation-world/src/scenes/Vehicle.tscn:16` | 改 | `ext_resource` 路径 |
| `orion-simulation-world/src/tests/check_models.gd` | 改 | 路径常量 + AABB（若变了）+ Nose 断言 |
| `orion-simulation-world/src/tests/check_sensor_models.gd:31` | 改 | 路径常量 |
| `orion-simulation-world/src/scenes/chassis/Quadrotor.tscn` | 视情况改 | 4 个桨盘**保留**（叠在模型桨位），必要时微调 `position` |

### 5 实施步骤

1. **选型 + 留证**：下载（直连不通时加 `-x http://127.0.0.1:7897`）→ 记 `URL` / 许可原文 / 字节 / `sha256` / 抓取日期（照 `assets/humans/README.md` 的格式）。
2. **Blender 归一**：轴/朝向改 +X、底面落 `y=0`、整体缩放到 AABB ≈ `0.64 × 0.13 × 0.64`、贴图从 glb 抽出、导出 glb。
   （脚本可参照 `tmp/human_model/tools/s3_convert.py` 的做法，但**不复用其资产**。）
3. **落库**：放 `assets/chassis/`，删旧 `uav_quadrotor_lowpoly.glb`（`git rm`）。
4. **接线**：`Vehicle.tscn` 改路径；`Quadrotor.tscn` 保留 4 个 `CylinderMesh` 桨盘（转速/转向的视觉表达不能丢）。
5. **自检更新**：三个常量 + `Nose` 断言；若 AABB 与 `0.64/0.13/0.64` 差 > 1 cm，同步改 `check_models.gd` 常量**并**在 `assets/README.md` 记「碰撞盒随外观变更」。
6. **导入**：`godot --headless --path orion-simulation-world --import`（生成 `.import`）。
7. **台账**：`assets/README.md` 加行。

### 6 完成判据

1. **导入零错**：`--import` 0 ERROR；`check_models.gd` 全绿（路径 / AABB / M4 机头 / M5·M6 碰撞盒）。
2. **看得见**：`Xvfb` 截图里能分辨机架、机臂、电机、桨、起落架；与写实人形同框不违和。
3. **性能不回退**：`RobotSim_TestEnv/world_fast/` 下 fps ≥ 改前基线（11 fps）。
4. **许可干净**：`assets/README.md` 有台账行；CC-BY 时 `ATTRIBUTION.md` 到位；无品牌机型。
5. **动力学零改动**：`max_speed` / `max_turn_rate` / `_size` / `BODY_HEIGHT` 一个不改；`check_models` 的 M5/M6 与动力学判据逐条不变。
6. **`git diff` 只含** §6.4 表内文件 + `assets/README.md`。

### 7 不做（Step 5）

- 不改动力学参数与碰撞尺寸（外观换了但"物理还是那台机"）；
- 不加碰撞子体、不改 `Vehicle.tscn` 节点结构；
- 不做 LOD / 烘焙动画 / 骨骼；
- 不改桨盘转速与转向的视觉逻辑；
- 不顺手换 UGV 或其他资产。

---

## 七、Step 6 — 死区调大 + 复验（Step 2 判据 2 未达标的收口）

### 1 背景

Step 2（`RobotSim_TestEnv/results/task40_20261029T142726Z/verdict.md`）判据 2「不抖」FAIL：`turn` 反号 **M1 11 次**（阈值 ≤2）、**M4 8 次**（阈值 ≤3）；同一用例三次观测是 **2 / 5 / 11**，说明指标贴在临界线上。

### 2 根因（算术，不是猜测）

- 三档律是 **bang-bang + 无迟滞**：`decide()` 每拍从零重算，符号只看 `Δθ` 正负。
- **过冲量 ≈ ω × 滞后**，滞后 = 位姿龄（实测中位 50 ms、最大 177 ms）+ 一个位姿周期 100 ms ≈ **0.15 s**；
  `ω = 27 °/s` ⇒ 过冲 **≈ 4.0°**（尖峰 4.8°），而死区只有 **5°** ⇒ 余量几乎为零，误差一越零线就翻号。
- 日志实证（`sim.log` 14:34:18）：`yaw = +5.3 → +0.7 → −1.1`，一次纠偏穿过零线到另一侧，下一拍必然反号。

### 3 改动（只动两个常量）

| 常量 | 旧 | 新 | 依据 |
|---|---|---|---|
| `DEADBAND_DEG` | 5.0 | **8.0** | 过冲 4.0°（尖峰 4.8°）< 8°，余量约 2 倍 |
| `ARC_MAX_DEG` | 10.0 | **12.0** | 死区抬到 8° 后，弧线档只剩 2° 宽 ⇒ 顺带保住 4° 宽 |

**不动**：`turn_speed = 30`（拐角仍 3.3 s，**不拿拐角时间换稳定**）、`PASS_THROUGH_M = 0.5`、`forward_speed = 30`、世界侧、协议、nav、树。

### 4 为什么"调小转向速度"不采用

`turn 30 → 20`（ω=18 °/s）过冲降到 2.7°，勉强压住 5° 死区，但 **90° 拐角从 3.3 s 退回 5 s**；`turn → 10` 才真正稳，但拐角回到 10 s —— 等于把 Step 1 的收益全吐回去。所以只调死区。

### 5 涉及文件

| 文件 | 动作 |
|---|---|
| `pleiades-sim/src/sim/ugv_sim/executor.rs` | 两个常量 + 文件头三档表/注释 + 单测断言与边界用例 |
| `Architecture/robot_arch.md` | D-S9 与常量速查同步（把「§八 已定不得再调」改成「Step 6 已调」） |
| `RobotSim_TestEnv/` | 复验产物 |
| `Workbook/wb_40_ugv_motion.md` | 记录 |

### 6 单测

1. `test_constants_are_spec`：`DEADBAND_DEG == 8.0`、`ARC_MAX_DEG == 12.0`。
2. 边界：`7°` → 直行（`turn=0`）；`8.5°`/`10°`/`12°` → 弧线档（`speed=30 且 turn≠0`）；`12.5°` → 原地转（`speed=0`）。
3. 原有用例全部保留（右弧/左弧/原地/无目标/身后格心/±π），只改边界数值。

### 7 复验（只验不改，判据不变）

1. **主判据**：M1 反号 **≤2**、M4 反号 **≤3**（与 Step 2 同口径、同一脚本 `Tool/sim_task40_ugv_test.py`）。
2. **回归**：M1/M2/M4 到达（末位逐轴 ≤0.26 m）、急停 0 次、`该停会停`、无 `Blocked`、日志全解析。
3. **拐角不劣化**：M2 耗时 ≤ **100 s**（Step 2 实测 87.4 s，留 15% 余量）。
4. 环境同 Step 2：`world_fast`(msaa=0) + `xvfb-run -a`；前置自检（位姿龄、活体闸门、二进制 sha）照旧。
5. 产物：`RobotSim_TestEnv/results/task40_s6_<UTC>/{verdict.md, sim.log, trace.csv}`。

### 8 判据未过时怎么办

- **不许**为该判据变绿而再调阈值或改判据、改脚本口径；
- 若 M1/M4 仍 FAIL，**如实记录**（哪条、实测值、日志片段）并停下来等人类裁决——**不要**自动试第二组参数。

### 9 不做

- 不改 `turn_speed`、`PASS_THROUGH_M`、`forward_speed`；
- 不加迟滞/状态（那是备选方案 A，本步不做）；
- 不改世界侧、协议、nav、行为树、Pictor；
- 不重跑 Step 3/4/5 的验证。

### 10 收口结论（2026-10-30，人类裁决 PASS）

**实测**（`RobotSim_TestEnv/results/task40_s6_20261030T010511Z/`）：死区 8° / 弧线档 12° 生效（`sim.log:29` 常量回执）；
M1 反号 **2**（≤2 ✅）、M4 反号 **4**（≤3）、到达 9/9、末位 0.24 m、急停 0、M2 87.8 s（+0.4 s）。

**口径问题（本步最重要的发现）**：把两次实测的每一次反号按「翻转时目标格是否变化」分类 ——

| 数据 | 总反号 | 同一格内（真抖动） | 换格那一拍（结构性） |
|---|---|---|---|
| Step 2（死区 5°） | 105 | **0** | 105 |
| Step 6（死区 8°） | 46 | **0** | 46 |

两次**全部**落在换格那一拍，同格内 0 次。原因：M4 是 45° 斜线，8 连通走阶梯路径，每换一格目标方位跳 ~45°，左右交替是几何必然。
⇒ 判据 2 原口径数的不是"抖动"，而是"换格时的方向切换"，故改口径为「**同一格内**反号」。
（诚实备注：车栈日志 5 Hz 节流，200 ms 以下的高频抖动不可见；"同格内 0 次" 指 ≥200 ms 粒度下为 0。）

**裁决**：人类 2026-10-30 决定按新口径**判 PASS**，不再调参数、不上迟滞。
**保留原始数据**：verdict.md 中的 2 / 4 不做修改，追加裁决段说明口径。

---

## 八、后续（本次不做，等你定步骤）

1. **整体替换**：把 `sim/executor.rs` 的离散路径下线。⚠️ 手动遥控现在也走离散 action（`Handle_Manual_Cmd → Send_Manual_Motion → forward/left/...`），替换时必须把**手动链一起迁到 `velocity`**，否则 Manual 模式与控制口的手动命令会失效。
2. **真机 UGV 连续运动**（单开 task）：`FUNC_MOTION` 的 `set_motion(vx, 0, ω)` 台架验证 + `omega` 符号标定 + 回报帧。`pack_motion` 的第三轴 `vz` 是角速度 rad/s（`docs/ros_stm32_protocol.md:137-150`），语义完备，但无调用方、无回报帧、符号无证据。
3. **ORION 速度消息**：`ORION_VELOCITY_CMD` 取代 `MANUAL_CONTROL` 的运动分支。口径建议**物理量**（`v` m/s、`omega` rad/s），与真机 `FUNC_MOTION` 和 uav 的 `[flight_ctrl]` 同名同单位；Step 1 的 `speed`/`turn` 百分比只作 **sim↔世界内部表示**。⚠️ ORION 无版本协商，须车/机/终端/Pictor 同批升级；且速度是持续指令，**车端必须自带超时停车**（建议 300 ms 未收到新速度即 `stop()`，因为协议刻意不做心跳）。
4. **地面站速度控制**：手动遥控从离散 action 迁到 `velocity` 后，Pictor 侧才能有"摇杆/连续速度"（依赖第 3 条）。

---

## 九、已定参数与口径（人类确认，执行时照此办理，不再回头问）

| # | 项 | **定值** | 说明 |
|---|---|---|---|
| 1 | `turn_speed` | **30**（27 °/s，`R = 0.95 m`，90° 拐角 3.3 s） | 弧线档在 30 下只能修小偏角；要弧线真正可用需 ≥ 60（`R ≤ 0.5 m`），那时 `DEADBAND_DEG` 同步抬到 7~8° |
| 2 | `ARC_MAX_DEG` | **12.0°**（Step 6 由 10° 上调） | 弧线档宽 4°（`DEADBAND=8°`）；若 #1 改成 60 以上，此值需重算 |
| 3 | `PASS_THROUGH_M` | **0.5 m**（1 格） | 目标格心在身后时直行穿过的阈值 |
| 4 | WS 端口形式 | **命令行参数 `--ws-port=9100`，默认关闭** | 不加 `Pictor` 配置段；无头启动时命令行本来就在 |
| 5 | 上行事件默认 | **`pose` + `peer`，`pose_hz = 2`，按 `targets` 过滤** | 不降频会被 10 Hz × N 车刷屏 |
| 6 | `Follow` 入口 | **只走 WS 命令**（脚本发 `type=2`），不加 UI 按钮、不加 LLM 词表 | 加 UI 会把改动从 3 个 GDScript 扩到 `auto_handler.gd` + 面板 |
| 7 | 模型许可 | **CC0 优先；CC-BY 可接受**（须 `LICENSE` + `ATTRIBUTION.md` + 台账 + 说明改动）；排除 CC BY-SA / CC BY-NC / personal-only / 品牌机型 | 见 §6.3 标准 |
| 8 | `DEADBAND_DEG`（车） | **8.0°**（Step 6 由 5° 上调） | 过冲 ≈ ω×0.15 s ≈ 4.0°，死区 8° 留约 2 倍余量；见 §七.2 |
| 9 | Step 2 改前基线 | **不单独跑**：用现有 `RobotSim_TestEnv/results/queue_ugv_dev/sim_1.log`（21.5 s / 4.02 m / `Stop` 10 次 / `MoveForward` 5 次）作对照 | 省一整轮测试；改后只跑 Step 2 的 6 个用例 |

> 执行中若实测明显不达判据，先按 §2.4 的算术重算 #1~#3，再决定是否调整——**不许**为了让判据变绿而改判据。
