# task_31_sim_world_link — 完善 sim：在 Godot 模拟世界里跑起来

> Presented by KeJi
> Created Date ： 2026-10-21
> Modified Date ： 2026-10-21
> 状态：**待人类给定范围与步骤**（本文档只记录目标与已核实现状）
> 关联文档：`Architecture/robot_arch.md`、`orion-simulation-world/COMPONENTS.md`、`Task/task_30_sim_world.md`

---

## 一、目标

（人类原话）完善 sim 这部分，确保 sim 可以在模拟世界里跑起来。

分工：**世界模拟那一块（`orion-simulation-world`，task_30 已交付）负责提供模拟数据；`pleiades-sim` 负责接入与使用。**

---

## 二、现状（2026-10-21，实测，只列事实）

### 2.1 sim 侧（`pleiades-sim`）

| 事实 | 证据 |
|---|---|
| 10 文件 / 1756 行；无任何 WebSocket / HTTP 客户端 | `pleiades-sim/Cargo.toml` 依赖仅 base / tokio / tracing / async-trait / serde / toml |
| `on_tick` 跑完整决策链（寻路 + 三状态机），但"执行动作"只有一行日志 | `pleiades-sim/src/sim/sim_handler.rs:98` |
| 位姿从不更新，永远停在 `Robot::new` 注入的 origin | `RobotState` 写者只有：base 注入 origin、stm32 RX、lg290p RX、mavlink RX；sim 无写者 |
| `executor.rs` / `planning/*` 与 `pleiades-ugv` **逐字节相同**（副本共 3 份：ugv / uav / sim） | `diff`：executor 286 行、assignment 425、pathfinder 401、cluster_obstacles 131 |
| `slam`（建图）、`lidar`（扫描类型与设备）、`emergency_stop` 在 sim 内**完全不存在** | `pleiades-sim/src/sim/` 无对应模块 |
| 走 `boot.run()`（带 TUI），无 headless 入口 | `pleiades-sim/src/main.rs:68`；base 已有 `run_headless()`（`pleiades-base/src/bootstrap.rs:57`） |
| 配置只有共享段 + `obstacle_inflation_radius` | `pleiades-sim/src/config.rs` |
| origin 只来自 CLI 参数，`(64,64,0)` 兜底 | `pleiades-sim/src/main.rs:26` |
| 已是完整 P2P 节点：mDNS 发现、gossipsub 收发、ORION 命令通道、`ClusterInfoTable` 全部可用 | `pleiades-sim/src/bootstrap.rs`、`pleiades-base/src/Robot/core/robot.rs` |

### 2.2 世界侧（`orion-simulation-world`，task_30 已交付部分）

| 事实 | 证据 |
|---|---|
| `--headless --quit` EXIT=0，日志 0 条 ERROR；12104 体素 / 6.8 ms 构建；WS 监听 9999 | 本机实测 |
| 8 个自检脚本全 `SELF-CHECK OK`：project 8 / world 26 / camera 32 / vehicle 36 / diff_drive 41 / quadrotor 45 / ws_manager 51 / lidar 48 项，`failed=0` | 工程复制到 /tmp 独立跑通，工程未改 |
| 装配握手端到端可用 | Python 探针：`chassis_ready{avatar=1, spawn=(64,64,0)}` → `sensor_ready{sensor_id=1}` → `assembly_ready`；随后 41 帧 1455 B @10 Hz lidar，世界日志 0 ERROR |
| 断开/重连按 `peer_id` 清理并移除 avatar；重连后 avatar 自增、spawn 变 `(66,64)` | 同上实测 |

### 2.3 世界侧已实现的 wire 契约（代码事实）

| 方向 | msg | 实际字段 |
|---|---|---|
| sim→world | `chassis_hello` | `peer_id`, `chassis`, `params`, `manifest[{slot,sensor}]` |
| world→sim | `chassis_ready` | `peer_id`, `avatar_id`, `spawn{x,y,z,yaw}`, `world_time` |
| sim→world | `sensor_hello` | `peer_id`, `slot`, `sensor`, `mount{x,y,z,yaw}`, `params` |
| world→sim | `sensor_ready` | `slot`, `sensor_id` |
| sim→world | `motion_cmd` | `action ∈ {forward,backward,left,right,up,down,stop}`、`speed` 0~100（世界只取最新一条，忽略 `seq`/`peer_id`，无频率限制） |
| world→sim | `assembly_ready` | `peer_id`, `world_time`（manifest 到齐才发；重复 hello 后会重发） |
| world→sim | 二进制帧 | `[type u8=1][slot_id u16][seq u32][world_time f64][f32×rays]`，小端；360 线 = 1455 B；10 Hz；距离 0.0 = 无回波；角度不上线（`angle = TAU·i/rays`，`ob_angle = yaw + angle` 直接可用，mount yaw 已是世界 yaw 语义） |
| — | `imu` / `gnss` | **仅常量，无生产者**：`scripts/net/WireProtocol.gd:32-33`；manager 无分支，落到 unknown-msg warning |

**世界侧当前没有"自身位姿/姿态"下行。**

### 2.4 接入时会遇到的世界侧行为（实测/代码事实）

1. **未知 sensor 会静默卡死**：`handshaken = true` 在传感器目录校验**之前**写入（`orion-simulation-world/scripts/net/WebSocketManager.gd:455-458`），随后未知 sensor 直接 return、不回 `sensor_ready`；该 slot 永不置位 → 永不发 `assembly_ready`；该连接再发 `sensor_hello` 会被"已握手重复 hello 忽略"（`:322-325`），只能断开重连。
2. **连接可永久悬空**：`_MountSensor` 的返回值在 `:477` 与 `:543` 均未使用，且回放 pending 前已 `erase`（`:536`）→ 若回放时车已被移除，连接停在 `open + handshaken + 无 ready`，`pending_timeout` 不再覆盖它（`:679-682`）。
3. **spawn 由世界决定且多车错开**：`_PickSpawn`（`:828-843`）以 `avatar_id` 索引，第 1 台 `(64,64,0)`，第 2~9 台在半径 2 m 的八方位环上（每环 +2 m）；`z = 2.0` 仅对 `quadrotor_uav`，UGV 为 0；`yaw` 恒 0。回给 sim 的 `spawn` 与落位一致。
4. **超时常量**：握手 5 s（关闭码 1002）、pending 10 s（1008）、WS 心跳 10 s（`set_heartbeat_interval`）、巡检 Timer 1 s；`stats_log` 默认 false。
5. **出生点附近 12 m 内无任何障碍**：出生点净空半径 3 m，最近障碍 `pillar_e` 在 19.8 m 外，lidar `range` 12 m → 出生点一圈 360 条射线全为 `0.0`（实测，非 bug）。
6. **UAV 巡航高度 2.0 m 恰好等于边界墙高 2.0 m**（`WebSocketManager.gd:837` vs `World.gd:32`，COMPONENTS.md 记为已知事实）。
7. **建图距离上限**：`pleiades-ugv/src/ugv/slam/lidar_mapper.rs:22` `LIDAR_MAX_RANGE = 5.0`，比世界 lidar 量程 12 m 小，5 m 外的点会被丢弃。

---

## 三、Step 1 — 世界侧下发自身位姿（模拟世界坐标）

### 1 目标

世界把 avatar 的**世界坐标位姿**通过底盘连接周期下发，让 sim 不必等任何传感器零件就能拿到自己的位置与朝向。

本 step 只做"世界发"，**sim 侧一行不改**（消费是后续 step）。

### 2 坐标转换：不需要转，但必须取对源

| 侧 | 坐标 | 说明 |
|---|---|---|
| Godot 内部 | Y-up，`global_position` | x/z 水平面，y 是高度；`rotation.y = -world_yaw` |
| wire / Orion | Z-up，x=东、y=南、z=高，yaw 绕 +z、归一化 [-PI, PI) | 与 `RobotState.x/y/z`、`attitude.yaw` 同构；与 `chassis_ready.spawn` 同坐标系 |

换轴只发生在 Godot 内部，唯一入口是 `scripts/common/WorldTransform.gd`（`PosToGodot/PosToWorld/YawToGodot/YawToWorld`），**不上线**。`Vehicle.GetPose()`（`scripts/vehicle/Vehicle.gd:241`）返回 `WorldTransform.PoseFromTransform(global_transform)`，已经是世界坐标，直接发即可。

**三条禁止**（写入 `COMPONENTS.md §9.1`，新增 E23）：

1. ❌ 不得发 `global_position` —— 那是 Godot Y-up 坐标（y=高度、z=南向）
2. ❌ 不得发 `rotation.y` —— 它等于 `-world_yaw`，符号相反
3. ❌ 不得自己用 `basis` 算 yaw —— 必须走 `GetPose()`
4. ⚠️ 前提：`PoseFromTransform` 用 `basis.get_euler().y`，**仅在 avatar 只绕 Y 旋转时成立**。当前世界的车只有 `rotation.y`，无 roll/pitch，成立；以后若引入倾斜/翻车，此路失效。

sim 侧不需要处理 yaw 范围：`angle_to_target` 内部有 `normalize_angle`（`pleiades-sim/src/sim/executor.rs:189-192`），任意范围都能用。

### 3 消息格式（对齐真机 `PoseData`）

```jsonc
// world -> sim（底盘连接；10 Hz）
{ "msg": "pose", "world_time": 12345.678,
  "x": 64.0, "y": 64.0, "z": 0.0, "yaw": 0.0,
  "vx": 0.0, "vy": 0.0 }
```

字段来源（对照 `pleiades-base/src/Robot/core/protocol/messages.rs:18-34` 的 `PoseData`）：

| 本消息字段 | 真机对应 | 世界侧取值 | 说明 |
|---|---|---|---|
| `world_time` | `time_boot_ms` | `WorldClock.Now()` | 秒；与既有 `chassis_ready`/`assembly_ready` 一致 |
| `x` / `y` / `z` | `x` / `y` / `z` | `GetPose()` | **`z` 必须带**：`RobotState.z` 存在，UGV 恒 0、UAV 写 2.0；真机 `decode_pose` 也严格要求含 z 的 38 字节 |
| `yaw` | `yaw` | `GetPose()`（已 `YawToWorld` 且归一化） | 弧度 |
| `vx` / `vy` | `vx` / `vy` | `WorldTransform.VelToWorld(vehicle.velocity)` | `PoseData` 有此二项；`VelToWorld` 已存在且当前无消费者，正好在此启用 |
| — | `valid` / `sub_gx` / `sub_gy` | **不发** | 这是决策链的**产物**（`ExecuteState.sub_target`），不是世界的输入；由 sim 自己产生 |
| — | `rtk_fixed` | **不发** | 世界没有 RTK 概念；sim 侧 `rtk_fixed` 保持 false（与真机未启用 RTK 时一致） |

JSON 文本帧（数据小，与 `chassis_ready` 同形态），不走二进制。

### 4 频率与连接（对齐真机）

- **频率 10 Hz（100 ms）**：真机位姿广播就是 100 ms —— `state_notifier` 的 `interval(Duration::from_millis(100))`（`pleiades-base/src/Robot/core/robot.rs:213`）。写死常量 `POSE_BROADCAST_INTERVAL_S := 0.1`，**不做成配置项**（`state_notifier` 也是硬编码）。
- **走底盘连接**：`motion_cmd` 已在这条连接上（`_ApplyMotionCommand`），位姿与运动指令对称；且不新增 sensor 零件、不进 `manifest`，绕开 §2.4-1 的"未知 sensor 静默卡死"。
- 只发给 `role == ROLE_CHASSIS && handshaken && !closed && vehicle 有效` 的连接；传感器连接一条都不发。

### 5 涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `orion-simulation-world/scripts/net/WireProtocol.gd` | 改 | 加 `MSG_POSE`；加 `BuildPose()`；删 `MSG_IMU`/`MSG_GNSS` |
| `orion-simulation-world/scripts/net/WebSocketManager.gd` | 改 | `_process` 用 `delta`；加 `_BroadcastPoses()` + 累加器 |
| `orion-simulation-world/tests/check_pose.gd` | **新建** | 位姿下行自检（沿用 `check_ws_manager.gd` 的同进程真 World + 真 WS 客户端模式、`Listen(0)`、`--fixed-fps 60`） |
| `orion-simulation-world/COMPONENTS.md` | 改 | §4.5 接口、§7.3 协议常量、§8.1 消息表、§9.1 新增 E23、§10 边界（`imu`/`gnss` 改为不做） |
| `orion-simulation-world/scripts/vehicle/Vehicle.gd` | **不改** | `GetPose()` 已存在 |
| `orion-simulation-world/project.godot` | **不改** | 频率是常量，不进配置 |
| `pleiades-sim/**` | **不改** | 本 step 只让世界"发"位姿 |

### 6 逐文件改动

**6.1 `scripts/net/WireProtocol.gd`**

- `MSG_ASSEMBLY_READY` 之后新增 `const MSG_POSE := "pose"`；删除现 `:32-33` 的 `MSG_IMU` / `MSG_GNSS`（不再预留：位姿走本条消息，imu/gnss 不做）。
- 新增构造函数（JSON 形状只有一处定义，与 `BuildLidar2DFrame` 对称）：

```gdscript
## 位姿下行帧（世界坐标 Z-up、米、yaw 弧度绕 +z、归一化 [-PI,PI)）。
## 位置/朝向必须来自 Vehicle.GetPose()（§9.1 E23），不得传 global_position / rotation.y。
static func BuildPose(world_time: float, pose: Dictionary, velocity: Vector3) -> Dictionary:
	return {
		"msg": MSG_POSE, "world_time": world_time,
		"x": float(pose.get("x", 0.0)), "y": float(pose.get("y", 0.0)),
		"z": float(pose.get("z", 0.0)), "yaw": float(pose.get("yaw", 0.0)),
		"vx": velocity.x, "vy": velocity.y,
	}
```

> `velocity` 由调用方传 `WorldTransform.VelToWorld(vehicle.velocity)` 的结果，`WireProtocol` 不 preload `WorldTransform`（保持"只做协议事实"的分层）。

**6.2 `scripts/net/WebSocketManager.gd`**

- `:143` `func _process(_delta: float)` → `func _process(delta: float)`，体内加 `_BroadcastPoses(delta)`。
- 常量区（`MAX_WIRE_ID` 附近）加：

```gdscript
## 位姿下行周期（秒）：对齐真机 state_notifier 的 100 ms（base robot.rs:213）。
const POSE_BROADCAST_INTERVAL_S := 0.1
```

- 成员区加 `var _pose_acc: float = 0.0`。
- 数据面新增（放在 `_ApplyMotionCommand` 附近）：

```gdscript
## 位姿下行：只发给已握手、仍活着、挂着 avatar 的**底盘**连接。
## 周期与物理帧解耦；落后超过 1 个周期只补发一帧，其余丢弃（不积压）。
func _BroadcastPoses(delta: float) -> void:
	_pose_acc += delta
	if _pose_acc + 1e-6 < POSE_BROADCAST_INTERVAL_S:   # 与 Lidar2D 同款容差，防 60 fps 漏帧
		return
	_pose_acc = 0.0 if _pose_acc > POSE_BROADCAST_INTERVAL_S * 2.0 else _pose_acc - POSE_BROADCAST_INTERVAL_S
	var t: float = WorldClock.Now()
	for link: Variant in _links.duplicate():
		if link.role != ROLE_CHASSIS or not link.handshaken or bool(link.closed):
			continue
		var vehicle: Node3D = link.vehicle
		if vehicle == null or not is_instance_valid(vehicle):
			continue                                        # 已被 queue_free 的 avatar，跳过
		var pose: Dictionary = vehicle.call("GetPose")
		var vel: Vector3 = WorldTransform.VelToWorld(vehicle.velocity)
		link.SendJson(WireProtocol.BuildPose(t, pose, vel))  # 非 OPEN 返回 ERR_UNCONFIGURED，属正常，不刷日志
```

> `ComponentLink`/`WireProtocol`/`WorldClock`/`WorldTransform` 在 `:37-40` 已 preload，无需新增。

**6.3 `tests/check_pose.gd`（新建）**

沿用 `check_ws_manager.gd` 已验证的形态：同进程真 `World.tscn` + 真 `WebSocketPeer` 客户端、`manager.Listen(0)` 取临时端口、命令行 `--fixed-fps 60`、外部 `timeout` 兜底。断言：

| # | 断言 |
|---|---|
| P1 | 握手完成后 1 s 内收到 ≥ 9 条 `pose`（10 Hz 容差） |
| P2 | 字段齐全、类型正确（`world_time`/`x`/`y`/`z`/`yaw`/`vx`/`vy` 全为数字），无多余必需字段缺失 |
| P3 | **静止时 `pose.x/y/z/yaw` 与 `chassis_ready.spawn` 完全相等** —— 坐标系与 yaw 约定的硬证据 |
| P4 | `yaw ∈ [-PI, PI)`；`world_time` 单调不降 |
| P5 | 发 `motion_cmd{action:"forward",speed:100}` 后 `x` 单调增大（`yaw=0` → +x），且 `vx > 0` |
| P6 | 发 `motion_cmd{action:"left",speed:100}` 后 `yaw` 单调**减小**（世界侧 `rotation.y += yaw_rate·dt`、`world_yaw = -rotation.y`），且 `vy` 符号与转向一致 |
| P7 | 传感器连接**收不到**任何 `pose` |
| P8 | 断开底盘连接后不再尝试发送（无 warning、无 ERROR） |
| P9 | 白盒：`_BroadcastPoses()` 对 `vehicle == null` 的连接不调用 `GetPose()`（用替身/已移除 avatar 验证不产生 `Invalid call`） |

**6.4 `COMPONENTS.md`**

- §4.5：补 `POSE_BROADCAST_INTERVAL_S`、`_BroadcastPoses()` 说明
- §7.3：`WireProtocol` 常量表加 `MSG_POSE`、`BuildPose`；移除 `imu`/`gnss`
- §8.1：消息表加 `pose` 行（含坐标系、yaw 语义、字段来源）
- §9.1：新增 **E23**：位姿只能取 `GetPose()`（附三条禁止 + "仅绕 Y 旋转"前提）
- §10 v1 边界：`imu`/`gnss` 从"预留"改为"不做"，并注明位姿由 `pose` 提供

### 7 数据流

```
Vehicle._physics_process(delta)                     # 唯一位置更新点（Vehicle.gd:76-89）
  → Chassis.Step → rotation.y += yaw_rate·delta → move_and_slide()
        ↓
WebSocketManager._process(delta) → _BroadcastPoses(delta)
        ↓  link.vehicle.GetPose()            → WorldTransform.PoseFromTransform（世界坐标）
        ↓  WorldTransform.VelToWorld(vehicle.velocity)
        ↓  WireProtocol.BuildPose → ComponentLink.SendJson → peer.send_text
        ↓
（本 step 到此为止；sim 侧消费在后续 step）
```

### 8 完成判据

1. `godot --headless --path orion-simulation-world --quit` → EXIT=0，且 `grep -E "SCRIPT ERROR|Parse Error|ERROR"` 为空
2. `timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://tests/check_pose.gd` → `SELF-CHECK OK` / `failed=0`
3. **既有 8 个自检仍全 PASS**（尤其 `check_ws_manager.gd` 51 项 —— 动了 `_process`，必须回归）
4. 外部探针（Python，与 task_30 验收同法）：收到 `pose`；静止位姿 == `chassis_ready.spawn`；`forward` 后 `x` 增；`left` 后 `yaw` 减
5. `stats_log` 仍为 false，日志无新增 warning

### 9 风险与坑

| # | 风险 | 处理 |
|---|---|---|
| 1 | 改了每帧调用的 `_process` | 必须回归既有 51 项自检 |
| 2 | `link.vehicle` 可能是已 `queue_free` 的节点 | `is_instance_valid` 守卫；否则 `GetPose()` 报 `Invalid call`（Godot 里属 `SCRIPT ERROR` 且**进程不退出**，只能靠 `timeout`） |
| 3 | 60 fps 下 `delta` 恒 `1/60`，累加器无容差会少发 | 用 `+1e-6` 容差（`Lidar2D` 踩过：无容差 1 s 只发 9 次） |
| 4 | `SendJson` 在非 OPEN 返回 `ERR_UNCONFIGURED` | 属正常，不刷日志、不断连 |
| 5 | 本 step 下发的是**理想真值**（无噪声/无延迟） | 接受；定位误差模拟是后续话题 |
| 6 | `PoseFromTransform` 依赖"仅绕 Y 旋转" | 写入 E23 前提；以后加倾斜需重做 |
| 7 | UAV 的 `z` 会是 2.0 | 消费方不得假设 `z == 0` |

### 10 不在本 step

- sim 侧任何改动（消费位姿、写 `RobotState`）
- `imu` / `gnss` 传感器零件
- 位姿噪声 / 延迟 / 丢包模拟
- 位姿广播给传感器连接或多个 peer 的优化

---

---

## 四、Step 2 — sim 侧零件声明与实现：按 config 选底盘/传感器、启动实现、WS 连上世界并完成装配

### 1 目标

`pleiades-sim` 从 `.config/config.toml` 读到"这辆车用什么底盘、挂哪些传感器"，据此

1. 校验零件名（必须是世界 `ChassisCatalog` / `SensorCatalog` 里存在的）
2. 建立 **1 条底盘连接 + N 条传感器连接**
3. 发送 `chassis_hello` / `sensor_hello`，收 `chassis_ready` / `sensor_ready` / `assembly_ready`
4. 断线按退避重连，世界重启后自动重新装配

本 step **只做到"装配完成"**：位姿/扫描的数据落地与动作上行是后续 step，但本 step 要把它们的接口留好、并把收到的消息计数/打日志（证明链路通）。

### 2 设想流程与代码现实的 7 处差异（照字面实现会卡在这些地方）

| # | 现实 | 依据 | 因此要这样做 |
|---|---|---|---|
| 1 | `DeviceHandler::start()` 是在主循环**之前** `await` 的，阻塞它就等于不起 tick | `pleiades-base/src/Robot/core/robot.rs:271`（`device.start().await` 在 loop 前） | `start()` **只 spawn、不等待连接**，立即返回 `Ok(())`。世界没起也要能正常启动 |
| 2 | `start()` 一返回，`auto_tick` 每 50 ms 就跑 | `robot.rs:280,335-343` | 必须有**装配门闩**：未 `Assembled` 时 `on_tick` 不决策、不写 `ExecuteState`（写 Idle），也不消费 mission |
| 3 | 零件名必须是**世界的目录名**，不是车体型号 | 世界 `ChassisCatalog`：`diff_drive_ugv` / `quadrotor_uav`；`SensorCatalog`：`lidar_2d`；而 ugv 的 `[chassis] car_type` 是 `X3Plus`（`pleiades-ugv/src/ugv/types.rs`） | 配置里写**世界名**；名字错时底盘被世界 `1002` 断开、传感器会**静默卡死** |
| 4 | 一个组件一条 WS 连接，`manifest` 必须与真实传感器连接一一对应 | 世界 `_RegisterChassis` 用 `manifest` 判就绪 | `manifest` **由传感器清单推导**，不写第二份；连接数 = `1 + sensors.len()` |
| 5 | `manifest` 为空时，世界在 `chassis_hello` 处理末尾**立刻**发 `assembly_ready` | 世界 `_RegisterChassis` 末尾调 `_TryAssembly`；空 manifest 立即满足 | 配置漏写传感器会"假装装配成功"。sim 侧日志必须打 `slots=N`，`N=0` 时给 warning |
| 6 | 重连 = 一台**新车**：世界按 `peer_id` 移除旧 avatar，重连后 `avatar_id` 自增、`spawn` 变 | 世界 `_RemoveVehicle` / `_PickSpawn`；实测 `(64,64,0)` → `(66,64,0)` | 每次 `chassis_ready` 都重新采纳 `spawn`；装配状态回退到 `Connecting` |
| 7 | 位姿不是传感器零件，走底盘连接（Step 1） | task_31 §三 | 配置里**不声明 `pose`**；传感器清单里只有 `lidar_2d` 这类真零件 |

**最关键的一条防线**：世界对未知 sensor 会静默卡死（§2.4-1），因此 **sim 必须在发 hello 之前用本地目录自校验**——名字不在本地目录里就直接报错并拒绝启动（或跳过该 slot + warning），绝不能让世界把它挂成半死连接。

### 3 配置设计（`.config/config.toml` 新增段）

新增独立段 `[sim_world]`，**不与 ugv 的 `[chassis]` / `[lidar]` 复用**（语义不同：这里是世界的零件名）：

```toml
[world]
enabled = true                    # false = 不连世界（等价现状：纯决策链空跑）
url = "ws://127.0.0.1:9999"
chassis = "diff_drive_ugv"        # 必须是世界 ChassisCatalog 里的名字（速度绑定见 [chassis] 段，Step 4）
reconnect_ms_min = 200            # 退避起点
reconnect_ms_max = 3000           # 退避上限
handshake_timeout_ms = 8000       # 必须 > 世界侧 5000（世界会在 5 s 后 1002）

[sim_world.chassis_params]            # 可选，透传进 chassis_hello.params；缺省用世界默认
max_speed = 1.5
max_turn_rate = 90

[[sim_world.sensor]]                  # 一个传感器一条连接；manifest 由本清单推导
slot = "lidar_front"
sensor = "lidar_2d"

[sim_world.sensor.mount]              # 外参：世界 yaw 语义（Vehicle.gd:165）
x = 0.1
y = 0.0
z = 0.2
yaw = 0.0

[sim_world.sensor.params]
range = 12.0
rays = 360
rate_hz = 10
```

约定：

- `params` / `chassis_params` 用 `Option<serde_json::Value>` 承载（TOML 表可直接反序列化成 `serde_json::Value`），不做字段白名单——世界侧对未知参数容忍、缺省走默认
- `sensor` 为空或缺省 → 只建底盘连接（合法：空 manifest）
- `slot` 必须非空且唯一（世界侧 `slot` 是 `Vehicle/Sensors` 下的节点名，重复会互相覆盖）

### 4 涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `pleiades-sim/Cargo.toml` | 改 | 加 `tokio-tungstenite = "0.24"`（**保持默认特性**：`connect` + `handshake`，客户端与服务端都有；默认**不含 TLS**）、`futures-util = "0.3"`、`serde_json = "1.0"`（三者均已在 `Cargo.lock`） |
| `pleiades-sim/src/config.rs` | 改 | `SimConfig` 加 `pub world: Option<WorldLinkConfig>` |
| `pleiades-sim/src/world_link/mod.rs` | **新建** | 模块导出 + `WorldLink`（状态机 + 事件循环）+ `WorldLinkHandle` |
| `pleiades-sim/src/world_link/config.rs` | **新建** | `WorldLinkConfig` / `SensorDecl` / `Mount` + 校验与默认值 |
| `pleiades-sim/src/world_link/catalog.rs` | **新建** | sim 侧零件目录（世界目录的镜像）+ `Validate_Chassis/Sensor` |
| `pleiades-sim/src/world_link/protocol.rs` | **新建** | hello / ready / pose / motion_cmd 的 serde 结构与编解码 |
| `pleiades-sim/src/world_link/link.rs` | **新建** | 单连接封装：connect / send_json / recv / 关闭码分类 |
| `pleiades-sim/src/world_link/parts.rs` | **新建** | `PartDecl` → hello 负载构造（底盘/传感器共用） |
| `pleiades-sim/src/sim/sim_handler.rs` | 改 | `start()` 建零件 + spawn `WorldLink`（不阻塞）；`on_tick` 门闩；`shutdown()` 停链路 |
| `pleiades-sim/src/sim/mod.rs` | 改 | 加 `pub mod world_link;` 的引用（或 `main.rs` 加 `mod world_link;`） |
| `pleiades-sim/src/main.rs` | 不改 | headless 属后续 step |

### 5 逐文件改动

**5.1 `Cargo.toml`**

```toml
tokio-tungstenite = "0.24"   # 默认特性 = connect + handshake（客户端/服务端都有），默认不含 TLS
futures-util = "0.3"
serde_json = "1.0"
```

**5.2 `src/world_link/config.rs`**

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Mount { pub x: Option<f32>, pub y: Option<f32>, pub z: Option<f32>, pub yaw: Option<f32> }

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SensorDecl {
    pub slot: String,
    pub sensor: String,
    pub mount: Option<Mount>,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WorldLinkConfig {
    pub enabled: Option<bool>,
    pub url: Option<String>,
    pub chassis: Option<String>,          // 世界零件名；速度绑定见 [chassis] 段（Step 4）
    pub reconnect_ms_min: Option<u64>,
    pub reconnect_ms_max: Option<u64>,
    pub handshake_timeout_ms: Option<u64>,
    pub chassis_params: Option<serde_json::Value>,
    pub sensor: Option<Vec<SensorDecl>>,
}
```

再给一个 `impl WorldLinkConfig { pub fn Resolved(&self) -> Result<ResolvedWorld, String> }`：填默认值（`url` 默认 `ws://127.0.0.1:9999`、退避 200/3000、握手 8000）、校验 `slot` 非空且唯一、`handshake_timeout_ms >= 5000`（否则模拟世界会先关，必失败）。

**5.3 `src/world_link/catalog.rs`**

sim 侧"世界的镜像目录"，只做名字校验与默认参数（**不复制世界的数值默认**，缺省一律交给世界）：

```rust
pub const CHASSIS: [&str; 2] = ["diff_drive_ugv", "quadrotor_uav"];
pub const SENSORS: [&str; 1] = ["lidar_2d"];

pub fn Validate_Chassis(name: &str) -> Result<(), String>;
pub fn Validate_Sensor(name: &str) -> Result<(), String>;
```

> 校验失败的处理：`chassis` 名校验失败 → `start()` 返回 `Err`（世界侧会 1002，且此刻根本没有可用的车）；`sensor` 名校验失败 → 跳过该 slot 并 `warn!`，其余传感器照常连（避免一颗坏零件拖死整车），但 `manifest` 里**不包含**被跳过的 slot。

**5.4 `src/world_link/protocol.rs`**

```rust
// 出站
#[derive(serde::Serialize)]
pub struct ManifestEntry<'a> { pub slot: &'a str, pub sensor: &'a str }

#[derive(serde::Serialize)]
pub struct ChassisHello<'a> {
    pub msg: &'static str,          // "chassis_hello"
    pub peer_id: &'a str,
    pub chassis: &'a str,
    pub params: serde_json::Value,
    pub manifest: Vec<ManifestEntry<'a>>,
}

#[derive(serde::Serialize)]
pub struct SensorHello<'a> {
    pub msg: &'static str,          // "sensor_hello"
    pub peer_id: &'a str,
    pub slot: &'a str,
    pub sensor: &'a str,
    pub mount: MountOut,            // {x,y,z,yaw}
    pub params: serde_json::Value,
}

#[derive(serde::Serialize)]
pub struct MotionCmd { pub msg: &'static str, pub seq: u64, pub action: &'static str, pub speed: u8 }

// 入站（先用 serde_json::Value 读 "msg" 再分派）
pub enum Inbound {
    ChassisReady { avatar_id: u16, spawn: Spawn, world_time: f64 },
    SensorReady { slot: String, sensor_id: u16 },
    AssemblyReady { world_time: f64 },
    Pose { world_time: f64, x: f32, y: f32, z: f32, yaw: f32, vx: f32, vy: f32 },
    Unknown { msg: String },
}
pub fn Parse_Text(s: &str) -> Option<Inbound>;
```

要点：

- `Inbound::Pose` 是 **Step 1 定义的字段**（`msg`/`world_time`/`x`/`y`/`z`/`yaw`/`vx`/`vy`）
- 未知 `msg` → `Inbound::Unknown`，只 debug 日志（世界侧对未知 msg 是忽略，sim 侧对等处理）
- 反序列化用 `serde_json`，**缺失字段不得 panic**：`Pose` 缺 `vx/vy` 也要能解析（`Option` 或 `#[serde(default)]`）
- 二进制帧不在本 step 解析（lidar 属后续），`Inbound` 只留一个 `Binary { bytes: Vec<u8> }` 透传

**5.5 `src/world_link/link.rs`**

```rust
pub enum LinkEvent { Text(String), Binary(Vec<u8>), Closed { code: u16, reason: String } }

pub struct Link { ws: tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>, url: String }

impl Link {
    pub async fn Connect(url: &str, timeout_ms: u64) -> Result<Self, String>;
    pub async fn Send_Json<T: serde::Serialize>(&mut self, v: &T) -> Result<(), String>;
    pub async fn Next(&mut self) -> Option<LinkEvent>;   // None = 流结束
    pub async fn Close(&mut self);
}
```

要点：`Send_Json` 用 `Message::Text`（与世界的 `send_text` 对应）；**不要**用 `Message::Binary`（世界侧会 warning "二进制只下行"）。

**5.6 `src/world_link/mod.rs`（状态机与事件循环）**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldState { Disabled, Connecting, ChassisReady, Assembling, Assembled, Reconnecting }

pub struct WorldLinkHandle {
    state: Arc<RwLock<WorldState>>,
    cmd_tx: mpsc::Sender<MotionCmd>,      // 本 step 只建通道，不接 on_tick
    cancel: CancellationToken,
}

impl WorldLinkHandle {
    pub async fn State(&self) -> WorldState;
    pub async fn Is_Assembled(&self) -> bool;
    pub async fn Send_Motion(&self, action: &'static str, speed: u8) -> Result<(), String>;
    pub fn Shutdown(&self);
}

pub struct WorldLink { /* config + node_handle + state + cmd_tx */ }

impl WorldLink {
    pub fn Spawn(config: ResolvedWorld, peer_id: String) -> WorldLinkHandle;
}
```

循环骨架（每个连接一条 `tokio::spawn`，事件汇到 `mpsc`）：

```
loop {
    if cancelled { break }
    // 1) 底盘连接
    match Link::Connect(...) { Err -> backoff 后 continue }
    send chassis_hello（peer_id + chassis + params + manifest）
    等 chassis_ready（<= handshake_timeout）→ 记录 avatar_id / spawn，state = ChassisReady
    // 2) 传感器连接 ×N（顺序建立；世界侧 pending 10 s 兜底，顺序颠倒也能活）
    for decl in sensors { connect + sensor_hello → 等 sensor_ready 或超时 }
    state = Assembling
    // 3) 等 assembly_ready（幂等：重复收到只更新 world_time + debug 日志）
    state = Assembled；info!("[world] assembled avatar={} slots={} spawn=({},{})")
    // 4) 之后进入收包循环：pose / lidar(binary) → 计数 + debug（本 step 不落库）
    //    任一连接 Closed/Err → state = Reconnecting，关掉其余连接，backoff 后回到 1)
}
```

关闭码处理（与 Step 1 的 8 s 握手超时配套）：

| 关闭码/原因 | 处理 |
|---|---|
| `1002`（协议错 / `hello timeout` / `bad json`） | 说明**我们的 hello 不合法**：打 `error!`（含原因），退避 3 s 重连；连续 3 次同因 → 停止重连并 `error!` 指路（避免无限刷屏） |
| `1008`（`pending timeout`） | 传感器等底盘超时：正常情况不该出现（我们顺序建立连接）；打 warning，重建全部连接 |
| `1011`（`chassis scene missing` / 装配失败） | 世界侧装配失败：`error!` + 不再重连（重连也不会成功），`state = Disabled` |
| `1001`（`peer removed` / `manager stopped`） | 世界重启或移除本车：静默重连 |
| 连接失败 / 超时 | 退避重连（200 ms 起 ×2，上限 3 s，带 ±20% 抖动） |

**5.7 `src/sim/sim_handler.rs`**

```rust
pub struct SimDeviceHandler {
    robot: Arc<Robot>,
    config: SimConfig,
    node_handle: Arc<NodeHandle>,
    origin: (f32, f32, f32),
    inner: OnceLock<SimInner>,
    world: OnceLock<WorldLinkHandle>,     // 新增
}

async fn start(&self) -> Result<(), String> {
    // 1) 建 GoalService + DecisionExecutor（现有逻辑不变）
    // 2) 若 [world].enabled：Resolved() 校验 + WorldLink::Spawn(peer_id = node_handle.Get_Local_Peer_Id().to_base58())
    //    —— 只 spawn，不等待；失败（配置非法）返回 Err
    // 3) 立即 Ok(()) —— 绝不 await 网络
}

async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
    // 门闩：未 Assembled → 保持 Idle，不寻路、不决策、不发命令
    if let Some(w) = self.world.get() {
        if !w.Is_Assembled().await {
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
            return;
        }
    }
    // …以下保持现有决策链（actions 仍只打印，本 step 不发 motion_cmd）
}

fn stop(&self)  { /* 现有 + w.Send_Motion("stop", 0) 尽力而为 */ }
async fn shutdown(&self) { /* 现有 + w.Shutdown() */ }
```

### 6 装配状态机

```
Disabled ──(enabled=false)──────────────────────────────┐
   │                                                    │
Connecting ──connect ok──▶ 发 chassis_hello             │
   │                          │                         │
   │                     chassis_ready                   │
   │                          ▼                         │
   │                    ChassisReady ──连 N 条传感器──▶ Assembling
   │                          │                         │
   │                          │                  assembly_ready（幂等）
   │                          │                         ▼
   └──任一连路断/超时──▶ Reconnecting ◀────────── Assembled
                              │（退避 200ms→3s）
                              └──▶ Connecting
```

- 只有 `Assembled` 允许决策（`on_tick` 门闩）
- `Reconnecting` 期间不做任何决策，`ExecuteState` 保持 Idle
- `1002 ×3 同因` / `1011` → 进入 `Disabled`（不再重连），日志给出可操作提示

### 7 数据流（本 step）

```
config.toml [world]
   → SimConfig.world → WorldLinkConfig::Resolved()（默认值 + 校验）
   → catalog 校验 chassis / 每个 sensor
   → WorldLink::Spawn（后台 task）
        ├─ Link::Connect(url)        tokio-tungstenite, ws://
        ├─ ChassisHello  → 世界 → chassis_ready（avatar_id / spawn / world_time）
        ├─ SensorHello×N → 世界 → sensor_ready（slot / sensor_id）
        └─ 世界 → assembly_ready       → state = Assembled
   → SimDeviceHandler::on_tick 门闩读 state（未装配 → Idle）
   → 收到的 pose / lidar 二进制：只计数 + debug 日志（落库在后续 step）
```

### 8 完成判据

**单元/编译**

1. `cargo check -p pleiades-sim`、`cargo test -p pleiades-sim` 全绿
2. 新增单测：`WorldLinkConfig::Resolved()` 的默认值/非法值、`slot` 重复检测、`handshake_timeout_ms < 5000` 拒绝、`Parse_Text` 对 `chassis_ready`/`sensor_ready`/`assembly_ready`/`pose`/未知 msg 的分派、`pose` 缺 `vx/vy` 仍能解析

**联调（必须真跑，不能只看代码）**

3. **世界未启动**：`cargo run -p pleiades-sim` 后 10 s 内进程存活、日志显示重试退避（无 panic、无 `ERROR` 刷屏）
4. **世界启动**：sim 自动连上；世界日志出现
   `[ws] chassis_hello peer='...' chassis=diff_drive_ugv conn=1` → `[ws] vehicle spawned ... slots=1` → `[ws] sensor_ready ...` → `[ws] assembly_ready`
   sim 日志出现 `[world] assembled avatar=N slots=1 spawn=(64.00, 64.00)`（或 66/64 视 avatar 序号）
5. **断线重连**：kill 世界 → sim 进入 `Reconnecting`（日志可见）；重起世界 → sim 自动重新装配，`avatar` 自增、`spawn` 更新
6. **配置错误**：
   - `chassis = "tank"` → sim `start()` 失败并给出明确错误，**不发起连接**（本地目录拦截，不让世界踩 1002）
   - `sensor = "imu"` → 该 slot 跳过 + `warn!`，其余照常；`manifest` 里不含该 slot
7. **空传感器清单** → 只建底盘连接；世界立刻回 `assembly_ready`；sim 日志 `slots=0` + 一条 warning（暴露"这车没有传感器"）
8. **未装配时决策门闩**：世界未起时 `on_tick` 不产生任何动作日志（现有 `[Sim] … 动作=` 一行都不出现），`ExecuteState` 保持 Idle

### 9 风险与坑

| # | 风险 | 处理 |
|---|---|---|
| 1 | `start()` 里 await 网络 → 世界没起时 tick 永不开始，连命令都收不到 | 只 spawn，立即返回（差异 1） |
| 2 | 世界对未知 sensor 静默卡死（§2.4-1） | 本地目录先校验；不合规 slot 直接跳过，不进 `manifest` |
| 3 | 空 `manifest` 世界立刻 ready（差异 5） | `slots=0` 打 warning；装配日志必须带 `slots=N` |
| 4 | 重连后 `spawn` 变了（实测 `(64,64)`→`(66,64)`） | 每次 `chassis_ready` 重新采纳 `spawn`（写入点放在后续 step，但本 step 要把 `spawn` 存进状态） |
| 5 | 误以为要关默认特性 | **不能**写 `default-features = false`：那会同时关掉 `connect` 与 `handshake`（`connect_async` / `accept_async` 都被 `#[cfg(feature)]` 门禁）。默认特性 `["connect","handshake"]` 本身**不含 TLS**，正确写法就是 `tokio-tungstenite = "0.24"` |
| 6 | 发送用 `Message::Binary` 会被世界 warning | 文本一律 `Message::Text` |
| 7 | `handshake_timeout_ms` 配得比世界 5 s 短 | `Resolved()` 里强制 ≥ 5000 并报错 |
| 8 | 传感器连接顺序若与 `manifest` 不一致 | `manifest` 从同一份清单推导，天然一致 |
| 9 | 世界侧 `assembly_ready` 可能重发 | 幂等：只更新 `world_time`，不重复打 info |
| 10 | 进程退出时连接残留 | `shutdown()` 里 `cancel` + 关闭全部链路；不依赖 `Drop` |

### 10 不在本 step

- `pose` → `RobotState` 的写入（后续 step）
- lidar 二进制帧 → `LaserScan` → 建图（后续 step）
- `on_tick` → `motion_cmd` 的真实上行（本 step 只留 `WorldLinkHandle::Send_Motion` 接口）
- `--headless`、多实例独立 `.config`、任务注入
- 急停、装配重对齐（清 D*）等策略

---

## 五、Step 3 — sim 自带激光雷达传感器与激光建图（写入公共 grid，禁止引用 ugv）

### 1 认知前提（本 step 的立足点）

- **`robot.grid` 是公共世界状态**：`Arc<RwLock<OccupancyGrid>>`，owner 是 base 的 `Robot`（`pleiades-base/src/Robot/core/robot.rs:51`），ugv / uav / sim 共用同一套语义。
- **写 grid 的不是决策链，是传感器**：有激光雷达才有激光建图；以后有摄像头，摄像头也往同一张 grid 写。每个传感器自带自己的感知 task，各自往里写。
- **决策链只读 grid**：`GoalService` 寻路时读，不写。
- **真机就是这个结构**：`LidarDevice::spawn(port, baud, Some(SlamContext))` 内部 `spawn_slam(...)`（`pleiades-ugv/src/ugv/lidar/mod.rs:95-112` → `slam/task.rs:154`），建图 task 归传感器所有，**不在** `on_tick` 里。

所以 sim 的建图必须**放在 lidar 传感器内部**，与真机同构；`SimDeviceHandler::on_tick` 一行不碰 grid。

### 2 硬约束

1. **禁止引用 `pleiades-ugv`**：`pleiades-sim` 不得 `use pleiades_ugv::...`、不得加它作依赖（ugv 本身也没有 `[lib]` target，只有 `[[bin]]`）。扫描类型、帧解析、建图算法**全部在 sim 内自带实现**；算法语义允许照抄，代码必须是 sim 自己的。
2. **允许且应当**使用 `pleiades-base`：`OccupancyGrid` / `Delta` / `world_to_grid` / `CELL_RESOLUTION` / `CHUNK_SIZE` / `DYNAMIC_OBSTACLE_DECAY`（`pleiades-base/src/Robot/core/grid.rs`）、`encode_map_delta` / `encode_frame` / `MSGID_MAP_DELTA` / `TOPIC_ROBOT_MAP` —— 这些是底座，不是 ugv。
3. **动态障碍膨胀**用 sim 自己已有的 `sim/planning/cluster_obstacles.rs`（本 crate 内副本），不引 ugv。
4. 交付后必须可验证：`grep -r "pleiades_ugv" pleiades-sim/` 输出为空。

### 3 涉及文件（全部在 `pleiades-sim/`）

| 文件 | 动作 | 内容 |
|---|---|---|
| `src/sim/sensors/mod.rs` | **新建** | `Sensor` trait + 按配置构造的工厂 + 停止接口 |
| `src/sim/sensors/lidar/mod.rs` | **新建** | `LidarSensor`：帧入口、`LaserScan` 快照、生命周期、拉起建图 task |
| `src/sim/sensors/lidar/types.rs` | **新建** | `LaserPoint` / `LaserScan` / `LidarState`（自带定义，字段与真机一致，便于对账） |
| `src/sim/sensors/lidar/frame.rs` | **新建** | 二进制帧解析（15 B 头 + f32 数组，小端） |
| `src/sim/sensors/lidar/mapper.rs` | **新建** | 点云 → `OccupancyGrid`（照抄语义、代码独立：hit +3 / miss −1 / 射线截断 / 去重 / 动态障碍抵消） |
| `src/sim/sensors/lidar/mapping_task.rs` | **新建** | 200 ms 建图 + 每 5 帧（1 s）增量广播 |
| `src/sim/pose_ingest.rs` | **新建** | `pose` → `RobotState` + 位姿新鲜度（建图需要位姿） |
| `src/sim/mod.rs` | 改 | `pub mod sensors; pub mod pose_ingest;` |
| `src/sim/sim_handler.rs` | 改 | `start()` 按配置建传感器并启动建图；`shutdown()` 停 |
| `src/world_link/mod.rs` | 改 | 传感器连接任务把 `Binary` 直接交给该 slot 的传感器；`Pose` 交给 `PoseIngest` |

### 4 逐文件改动

**4.1 `sensors/lidar/types.rs`（自带 schema，不 import）**

```rust
/// 单个激光点（弧度 / 米 / 强度 0~1）
pub struct LaserPoint { pub angle: f32, pub range: f32, pub intensity: f32 }

/// 一圈扫描（字段与真机 LaserScan 一致，便于以后对账）
pub struct LaserScan {
    pub points: Vec<LaserPoint>,
    pub stamp: u64,        // 纳秒（由帧头 world_time * 1e9 得来）
    pub scan_freq: f32,    // Hz（配置 rate_hz）
    pub scan_time: f32,    // 秒（1/rate_hz）
}

#[derive(Debug, Default)]
pub struct LidarState { pub scan: Option<LaserScan> }
```

**4.2 `sensors/lidar/frame.rs`**

```rust
pub const FRAME_HEADER_SIZE: usize = 15;
pub const FRAME_TYPE_LIDAR: u8 = 1;
pub const SAMPLE_BYTES: usize = 4;

pub struct LidarFrame { pub sensor_id: u16, pub seq: u32, pub world_time: f64, pub ranges: Vec<f32> }

/// 小端解析（世界侧 COMPONENTS.md §8.2）。短包 / 类型不符 / 空 payload → Err。
pub fn Parse_Lidar_Frame(bytes: &[u8]) -> Result<LidarFrame, String>;
```

- 头：`[0] u8 type`、`[1..3] u16 slot_id`、`[3..7] u32 seq`、`[7..15] f64 world_time`
- payload：`f32` 小端数组，点数 `n = (len - 15) / 4`；**尾部不足 4 字节的部分丢弃并 warning（限流）**
- `type != 1` → `Err`（相机帧类型 2 尚未实现，直接丢弃）

**4.3 `sensors/lidar/mod.rs`**

```rust
pub struct LidarParams { pub rays: usize, pub range: f32, pub rate_hz: f32 }   // 来自 [sim_world.sensor.params]

pub struct LidarSensor {
    slot: String,
    params: LidarParams,
    state: Arc<RwLock<LidarState>>,   // 建图 / 后续急停读
    cancel: CancellationToken,
}

impl LidarSensor {
    pub fn New(slot: &str, params: LidarParams) -> Self;
    pub fn Slot(&self) -> &str;
    /// 一帧二进制进来：解析 → 组 LaserScan → 写 state（try_write，抢不到就丢这一帧）
    pub fn On_Binary(&self, bytes: &[u8]);
    pub async fn Get_Scan(&self) -> Option<LaserScan>;
    /// 拉起本传感器的建图 task（200 ms）
    pub fn Spawn_Mapping(&self, ctx: MappingCtx);
    pub fn Shutdown(&self);
}
```

角度与无回波语义（与真机对齐）：

- **角度不上线**：`angle = TAU * i / n`，`n` 取**帧实际点数**（与配置 `rays` 不符时 warning 限流并以实际为准，否则角度语义会错）
- `range == 0.0`（无回波）与 `< 0.01` **保留原值**，不剔除——过滤交给 mapper，这样 `LaserScan` 与真机同义
- `intensity`：世界不上线（`FRAME_TYPE_LIDAR` 只有距离），一律 `0.0`
- `stamp = (world_time * 1e9) as u64`；`scan_freq = rate_hz`；`scan_time = 1.0 / rate_hz`

**4.4 `sensors/lidar/mapper.rs`（算法照抄语义，代码独立）**

```rust
const LIDAR_MAX_RANGE: f32 = 5.0;   // 与真机一致：>5 m 不入图
const LIDAR_MIN_RANGE: f32 = 0.1;

pub struct RobotPose { pub x: f32, pub y: f32, pub yaw: f32 }   // sim 自己的类型

pub fn Update(
    grid: &mut OccupancyGrid,
    pose: &RobotPose,
    scan: &[(f32, f32)],                  // (angle_rad, range_m)
    masked: &HashSet<(i32, i32)>,         // 他车格（动态障碍掩蔽）
) -> Vec<Delta>;
```

语义必须与真机一致（`pleiades-ugv/src/ugv/slam/lidar_mapper.rs`）：

1. 过滤 `range < 0.1` 或 `> 5.0`；`laser_to_world`：`a = yaw + angle`，`(x + r·cos a, y + r·sin a)`
2. 端点去重成 `HashSet`，限制在 `[0, CHUNK_SIZE)` 内
3. 先 hit：端点格 `grid.update(gx, gy, true)`（+3），跳过机器人自身格
4. 再 miss：Bresenham 从机器人格到端点，遇本帧端点格即截断，每帧每格最多一次 `grid.update(..., false)`（−1）
5. 最后动态障碍：对 `masked` 每格 `grid.decay(gx, gy, DYNAMIC_OBSTACLE_DECAY)`
6. 收集 `delta != 0` 的 `Delta` 返回

**4.5 `sensors/lidar/mapping_task.rs`**

```rust
pub struct MappingCtx {
    pub grid: Arc<RwLock<OccupancyGrid>>,
    pub robot_state: Arc<RwLock<RobotState>>,
    pub cluster_table: Arc<ClusterInfoTable>,
    pub map_tx: broadcast::Sender<Vec<MapDelta>>,
    pub node_handle: Option<Arc<NodeHandle>>,
    pub local_peer_id: Option<Vec<u8>>,
    pub obstacle_inflation_radius: f32,
    pub pose_age: Arc<AtomicU64>,        // 由 PoseIngest 维护（毫秒）
}
pub fn Spawn_Mapping(ctx: MappingCtx, lidar: Arc<LidarSensor>, cancel: CancellationToken);
```

节奏与真机一致（`ugv/slam/task.rs:55,107`）：**200 ms 更新，每 5 帧（1 s）广播一次**，跨帧聚合净变化后再发。

每个 tick 的顺序（**锁绝不跨 await**，真机踩过：`robot.rs:216` 的注释）：

1. 读位姿快照 → 立刻 drop 锁（`x, y, yaw`）
2. 位姿陈旧（`pose_age > 1000 ms`）→ **本周期跳过建图**（不拿旧位姿写脏数据）+ warning 限流；未装配时天然走到这里，等于"装配门闩"
3. `lidar.Get_Scan()` 拿最新一圈（拿不到就跳过）
4. 锁外算 `masked`：`cluster_table.snapshot()` + sim 自己的 `cluster_to_obstacle_cells`
5. `grid.write()` → `mapper::Update` → 累加进 `pending`
6. 每 5 帧：`drain_pending` → `map_tx.send` + `encode_map_delta`/`encode_frame(MSGID_MAP_DELTA)` → `Gossipsub_Publish(TOPIC_ROBOT_MAP)`

**4.6 `src/sim/pose_ingest.rs`**

```rust
pub struct PoseIngest { state: Arc<RwLock<RobotState>>, last_ms: Arc<AtomicU64> }

impl PoseIngest {
    pub fn New(state: Arc<RwLock<RobotState>>) -> Self;
    /// 收到 pose：写 x / y / z / attitude.yaw / vx / vy，刷新新鲜度
    pub async fn On_Pose(&self, p: &PoseFields);
    /// 收到 chassis_ready：用 spawn 覆盖 origin（多车时世界会把车摆在错开的方位）
    pub async fn On_Spawn(&self, s: &SpawnFields);
    /// 距上次 pose 的毫秒数（建图判新鲜度用）
    pub fn Age_Ms(&self) -> u64;
}
```

- 计龄用**本地单调时钟**（`Instant`），不拿世界 `world_time` 减本地墙钟（两者不同源，task_30 已记录该坑）
- 未收到过 pose → `Age_Ms()` 返回 `u64::MAX`（等价"永不新鲜"）
- 只写 `RobotState` 里世界能给的那几个字段；`rtk_fixed` 保持 false（世界没有 RTK，与真机未启用 RTK 一致）

**4.7 `src/world_link/mod.rs`**

- 传感器连接任务持有 `(slot, sensor_id)`：收到的 `Message::Binary` **直接交给该 slot 的传感器**，不需要按 `slot_id` 反查
- 帧头 `sensor_id` 与连接登记值不符 → warning（限流）+ 丢弃该帧
- `Inbound::Pose` → `PoseIngest::On_Pose`；`Inbound::ChassisReady` → `PoseIngest::On_Spawn` + 记录 `avatar_id`

**4.8 `src/sim/sim_handler.rs`**

```rust
struct SimInner {
    goal_service: Arc<Mutex<GoalService>>,
    executor: DecisionExecutor,
    sensors: Vec<Arc<dyn Sensor>>,      // 新增：按配置构造
    pose: PoseIngest,                   // 新增
}

async fn start(&self) -> Result<(), String> {
    // 1) GoalService + DecisionExecutor（现有）
    // 2) PoseIngest::New(robot.robot_state.clone())
    // 3) 遍历 [sim_world.sensor] → SensorFactory::Build(decl)
    //      lidar_2d → LidarSensor::New(slot, params).Spawn_Mapping(MappingCtx{...})
    //    未知 sensor 已在 Step 2 被 catalog 拦掉，这里不会出现
    // 4) 立即 Ok(())（不等待网络，Step 2 的约束不变）
}

async fn shutdown(&self) { /* 现有 + 每个 sensor.Shutdown() */ }
```

### 5 数据流

```
世界 Lidar2D ──binary──▶ 传感器连接（slot=lidar_front, sensor_id=N）
    → LidarSensor::On_Binary
         ├─ frame::Parse_Lidar_Frame（15B 头 + f32 数组，小端）
         └─ LaserScan{points[(TAU·i/n, range, 0.0)], stamp, scan_freq, scan_time} → LidarState

世界底盘连接 ──pose──▶ PoseIngest::On_Pose → RobotState{x,y,z,attitude.yaw,vx,vy} + 新鲜度

MappingTask（200 ms，归 LidarSensor 所有）
    ├─ 读 RobotState 快照（drop 锁）
    ├─ pose_age > 1000 ms → 跳过本周期（含未装配）
    ├─ lidar.Get_Scan() → 最新一圈
    ├─ cluster_to_obstacle_cells（他车格）→ masked
    ├─ grid.write() → mapper::Update → pending（Δ）
    └─ 每 5 帧 → map_tx + gossip TOPIC_ROBOT_MAP

决策链（on_tick，50 ms）── 只读 grid（GoalService → D*）
```

### 6 完成判据

**编译与单测**

1. `cargo check -p pleiades-sim`、`cargo test -p pleiades-sim` 全绿
2. 帧解析：构造 360 点 / 1455 B 帧 → `sensor_id`/`seq`/`world_time`/`ranges.len()` 全对；`type != 1`、短包、尾字节不整除 → 各自的错误分支
3. `LaserScan`：`angle[i] == TAU·i/n`；`range == 0.0` 的点**保留**；`stamp == (world_time·1e9) as u64`
4. mapper：单点点命中 → `delta == +3`；射线途经格 → `−1`；越界坐标丢弃；`masked` 格 → `decay`；同格同帧不重复
5. `PoseIngest`：写入后 `Age_Ms()` 递增；1.2 s 后 > 1000；`On_Spawn` 覆盖 origin

**禁止引用 ugv（硬性）**

6. `grep -r "pleiades_ugv" pleiades-sim/` 无输出
7. `git status` 中除 Step 1 的 `orion-simulation-world/` 外，改动**只在 `pleiades-sim/`**

**联调**

8. 世界 + sim：sim 日志显示收到 lidar 帧（计数/频率 ≈ 10 Hz，`rays` 与实际一致）
9. 车开到 `wall_c`（36,92 附近）→ sim 侧 `grid` 的占据格数增长，且墙呈连续格；`TOPIC_ROBOT_MAP` 有增量广播（`map_tx` 或日志可证）
10. 世界未启动 / 未装配时：**不建图**（位姿陈旧，每一 tick 都跳过），不产生 `MAP_DELTA`

### 7 风险与坑

| # | 风险 | 处理 |
|---|---|---|
| 1 | 位姿（10 Hz）与扫描（10 Hz）不同源，建图用的是"最新位姿 + 最新扫描"，两者最多差 100 ms | 高速时墙会被"甩"出偏移；已知误差，记录在 workbook；后续可做时间对齐 |
| 2 | `rays` 与帧实际点数不一致 | 以帧实际点数为准（否则角度全错），限流 warning |
| 3 | 持锁跨 `await` 会让位姿抖动 | 先取快照并 drop，再 await 取扫描，最后 `grid.write()`（真机踩过） |
| 4 | `LIDAR_MAX_RANGE = 5.0` 远小于世界量程 12 m | 与真机一致，不改；远墙不入图属预期 |
| 5 | 建图 200 ms 而扫描 10 Hz → 每次只用最新一帧，会丢帧 | 与真机同（真机也是"读最新"），不补帧 |
| 6 | `grid` 是公共资源，多写者会打架 | Step 3 之前 sim 无任何 grid 写者；本 step 之后写者只有 lidar 传感器 |
| 7 | 帧率无关性 | 建图用 `tokio::time::interval`，不按帧计数；角度按点数算，不按帧率 |
| 8 | `intensity` 世界不上线 | 恒 0.0；若以后要强度图，需扩帧格式（另立 step） |

### 8 不在本 step

- 急停（需要 scan，但属决策策略，另立 step）
- 相机传感器与相机感知（往同一张 grid 写）
- `MAP_FULL` 全量帧、地面站渲染
- 位姿/扫描的时间对齐与插值
- 多雷达（多 slot）融合

---

## 六、Step 4 — sim 自带底盘实现：动作翻译、速度绑定、位置更新

### 1 目标

按 `[sim_world].chassis` 选出的底盘，在 sim 内实现与真机**同形**的设备接口：

- `Apply_Action(MotionAction)` —— 与 `STM32Device::apply_action` / `MavlinkDevice::apply_action` 同签名（同步、无返回值）
- `Handle_Manual_Cmd(&ManualCmd)` —— 与真机 `handle_manual_cmd` 同签名，**按底盘类型选择性响应**
- 位置更新 —— `pose` 下行 → `RobotState`（真机上这一步是底盘 RX 回调做的）
- 把动作**翻译成世界的 `motion_cmd`**（唯一翻译入口，集中在一处）

### 2 翻译检查结果（这是本 step 的重点核对）

#### 2.1 `apply_action` 对照表

| `MotionAction` | 真机（`pleiades-ugv/src/ugv/stm32/mod.rs:153-164`） | 真机 wire（`MotionState`） | 真机速度 | 世界 `action` | 世界侧语义（`DiffDrive.gd:92-104`） | 世界 `speed` |
|---|---|---|---|---|---|---|
| `MoveForward` | `forward(forward_speed)` | `Forward = 1` | `forward_speed` | `forward` | `_speed_target = +max_speed·pct`，`_yaw_target = 0` | `forward_speed` |
| `MoveBackward` | `backward(forward_speed)` | `Backward = 2` | `forward_speed` | `backward` | `−max_speed·pct`，`yaw=0` | `forward_speed` |
| `TurnLeft` | **`spin_left(turn_speed)`** | **`SpinLeft = 5`** | `turn_speed` | `left` | `speed=0`，`yaw=+max_turn_rate·pct` | `turn_speed` |
| `TurnRight` | **`spin_right(turn_speed)`** | **`SpinRight = 6`** | `turn_speed` | `right` | `speed=0`，`yaw=−max_turn_rate·pct` | `turn_speed` |
| `Stop` | `stop()`（有界重试 10×10 ms） | `Stop = 0` | `0` | `stop` | `speed=0`，`yaw=0` | `0` |

**第一个检查结论**：真机的转向是 **`SpinLeft`/`SpinRight`（原地偏航）**，`apply_action` **从不使用** `MotionState::Left = 3` / `Right = 4`（那是弧线转向，协议里有但决策链不用）。而世界的 `left`/`right` 在 `DiffDrive` 里正是 `_speed_target = 0.0` + 纯 `yaw_target` —— **也是原地偏航**。所以 `TurnLeft → "left"`、`TurnRight → "right"` 语义严格对齐，没有把"原地转"错翻成"弧线转"。

#### 2.2 符号三方一致性（推导链）

1. **世界侧**：`right` → `_yaw_target = −max_turn_rate·pct` → `Rotation += yaw_rate·dt`（`Vehicle.gd:86`）→ `rotation.y` 减小
2. **换轴**：`world_yaw = -rotation.y`（`WorldTransform.gd:41`）→ 世界 yaw **增大**
3. **几何**：世界 x=东、y=南、yaw 从 +x 起向 +y 增大；从东转向南 = 俯视顺时针 = **右转** ✓
4. **决策侧**：`delta = normalize(atan2(dy,dx) − yaw)`；`delta > TURN_ALIGN_RAD → TurnRight`（`executor.rs:69-80`）→ 目标角更大 = 需要 yaw 增大 = 右转 ✓

→ `TurnRight ↔ "right"`、`TurnLeft ↔ "left"` 三方一致，**符号不需要取反**。

#### 2.3 单位一致性

- 真机 `speed`：`clamp(0, 100)` 的 **i16 百分比**（`robot_handler.rs:99-100`），STM32 内部速度环解释
- 世界 `speed`：**0~100 百分比**，世界侧 `clamp(0,100)/100 × max_speed`（`ChassisModel.SpeedPct` / `DiffDrive.SetCommand`）
- → 同一量纲，**直接透传，不做 m/s 换算** ✓

#### 2.4 手动命令对照（`ManualCmd` → 世界指令）

| `ManualCmd` | 真机行为（`stm32/mod.rs:134-147`） | sim 行为 |
|---|---|---|
| `Forward(_)` | `forward(forward_speed)` —— **忽略报文里的 i16** | `forward` + `forward_speed`（同样忽略载荷） |
| `Backward(_)` | `backward(forward_speed)` | `backward` + `forward_speed` |
| `SpinLeft(_)` | `spin_left(turn_speed)` | `left` + `turn_speed` |
| `SpinRight(_)` | `spin_right(turn_speed)` | `right` + `turn_speed` |
| `Stop` | `stop()` | `stop` |
| `Beep(ms)` | `beep(ms)` —— 车支持 | **忽略**（世界无蜂鸣；记 `warn!` 一次，与"设备选择性响应"一致，不崩） |
| `Takeoff` / `Land` | 车不支持（`_ => return`） | 差速：忽略；四旋翼：`up` / `down` |
| `StartLidarScan` / `StopLidarScan` | 由 `CarDeviceHandler` 拦截（`robot_handler.rs:189-205`） | 由 `SimDeviceHandler` 拦截：sim 的雷达由世界驱动，**不响应**（记 `info!`） |

#### 2.5 位置更新：谁写哪些字段

| 字段 | 真机写者 | sim 写者 | 说明 |
|---|---|---|---|
| `x` / `y` | STM32 里程计积分 + LG290P RTK 覆盖 | **`pose` 下行**（世界权威） | sim 不积分（D8） |
| `z` | 车恒 0 / 机写飞控 EKF | `pose` 下行 | UGV 0、UAV 2.0 |
| `attitude.yaw` | STM32 `attitude` 帧 | `pose` 下行 | 弧度，已归一化 |
| `vx` / `vy` | STM32 | `pose` 下行 | 世界给的是 `VelToWorld(vehicle.velocity)` |
| `vz` | STM32（车 0）/ 飞控 | **不写**（保持 0） | 世界 `pose` 不含 vz；记录为已知差异 |
| `battery` / `gyro` / `accel` / `mag` | STM32 | **不写**（保持默认 0） | 世界不提供；**不造数**，记录为已知差异 |
| `rtk_fixed` | LG290P 独写 | **不写**（恒 false） | 世界无 RTK；与真机未启用 RTK 一致 |

**第二个检查结论**：真机 `RobotState` 的写者是「底盘 RX 回调」这一个；sim 必须保持同样的**单写者**结构——所以 Step 3 的 `PoseIngest` **归底盘持有并调用**，不另设写者。写 `RobotState` 的只有底盘。

#### 2.6 检查出的 4 个差异 / 缺口（必须记录）

| # | 差异 | 处理 |
|---|---|---|
| G1 | 真机 `stop()` 是安全关键：`try_send` 有界重试 10×10 ms，**不静默丢弃**（`stm32/mod.rs:117-131`）；而 `DeviceHandler::stop()` 是**同步 fn**，不能 await WS 写 | sim 用有界通道 `try_send` 发 `stop`；通道满则登记 `pending_stop` 标志，由发送 task 下一拍必发 + `warn!`。**绝不静默丢 stop** |
| G2 | 真机另有 `set_motion(vx,vy,vz)`（`FUNC_MOTION` 连续速度）；世界**没有**对应指令 | sim 不实现；UAV 的高度只能靠 `up`/`down` 离散指令。记录能力差异 |
| G3 | 真机 UAV 速度是 m/s 常量（`vel_fwd`/`yaw_rate_deg`，`mavlink/mod.rs`），世界是 0~100 百分比 | sim 的 UAV 也传百分比：`vel_fwd → forward_speed`、`yaw_rate_deg → turn_speed` 各占一个 0~100 配置项；**语义近似、非等价**，记录 |
| G4 | 真机 `apply_action` 每次都发一帧；`DecisionResult.action == None` 表示"保持" | sim 同构：`None` 不发，世界保持上一条 `motion_cmd`（世界只取最新且不超时） |

### 3 硬约束

1. **禁止引用 `pleiades-ugv` / `pleiades-uav`**：`use pleiades_ugv::...`、`use pleiades_uav::...` 一律禁止，也不得加依赖。动作翻译、速度绑定、位置写回全部在 sim 内自带；算法/映射表可以照抄，代码必须是 sim 的。
2. 允许 `pleiades-base`：`MotionAction`、`ManualCmd`、`RobotState`、`OccupancyGrid` 等都在底座。
3. 交付后验证：`grep -rE "pleiades_ugv|pleiades_uav" pleiades-sim/` 为空。

### 4 涉及文件（全部在 `pleiades-sim/`）

| 文件 | 动作 | 内容 |
|---|---|---|
| `src/config.rs` | 改 | 新增 `[chassis]` 段（`enabled` / `forward_speed` / `turn_speed`，与 ugv 同名同字段、同默认 30/10）；`[sim_world]` 不再持有速度 |
| `src/sim/chassis/mod.rs` | **新建** | `Chassis` trait + 按 `[sim_world].chassis` 构造的工厂 |
| `src/sim/chassis/action_map.rs` | **新建** | **唯一翻译入口**：`Action_To_Wire(MotionAction, &SpeedBinding) -> MotionWire` + 单元测试锁表 |
| `src/sim/chassis/ugv.rs` | **新建** | 差速底盘：`apply_action` / `handle_manual_cmd` / 停车 |
| `src/sim/chassis/uav.rs` | **新建** | 四旋翼：同上 + `Takeoff→up` / `Land→down` |
| `src/sim/chassis/pose_ingest.rs` | **新建（由 Step 3 移入）** | `pose` → `RobotState` + 新鲜度；**由底盘持有**（单写者） |
| `src/sim/sim_handler.rs` | 改 | `start()` 建底盘；`on_tick` 把 `action` 交底盘；`handle_manual_cmd` / `reset` / `stop` / `shutdown` 交底盘 |
| `src/world_link/mod.rs` | 改 | 把 `Send_Motion` 通道交给底盘（`MotionSender` 句柄），本 step 起真正有发送者 |

### 5 逐文件改动

**5.1 `src/config.rs`**

```rust
/// 底盘速度绑定：与 ugv 的 [chassis] 同名同字段、同默认值（robot_handler.rs:99-100）
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChassisConfig {
    pub enabled: Option<bool>,
    pub forward_speed: Option<i32>,   // 0~100，默认 30
    pub turn_speed: Option<i32>,      // 0~100，默认 10
}

pub struct SimConfig {
    #[serde(flatten)] pub base: BaseConfig,
    pub obstacle_inflation_radius: Option<f32>,
    pub chassis: Option<ChassisConfig>,       // 新增
    pub sim_world: Option<WorldLinkConfig>,   // Step 2 已有
}
```

**5.2 `src/sim/chassis/action_map.rs`（唯一翻译入口）**

```rust
pub struct SpeedBinding { pub forward: u8, pub turn: u8 }   // clamp(0,100) 后的配置值

pub struct MotionWire { pub action: &'static str, pub speed: u8 }

/// 真机→世界 的**唯一**翻译点。表中每一行都对应 §2.1 的检查结论。
pub fn Action_To_Wire(action: MotionAction, speed: &SpeedBinding) -> MotionWire {
    match action {
        MotionAction::MoveForward  => MotionWire { action: "forward",  speed: speed.forward },
        MotionAction::MoveBackward => MotionWire { action: "backward", speed: speed.forward },
        MotionAction::TurnLeft     => MotionWire { action: "left",     speed: speed.turn },
        MotionAction::TurnRight    => MotionWire { action: "right",    speed: speed.turn },
        MotionAction::Stop         => MotionWire { action: "stop",     speed: 0 },
    }
}
```

**5.3 `src/sim/chassis/mod.rs`**

```rust
#[async_trait]
pub trait Chassis: Send + Sync {
    fn Name(&self) -> &str;                              // 世界零件名
    /// 与 STM32Device::apply_action / MavlinkDevice::apply_action 同形
    fn Apply_Action(&self, action: MotionAction);
    /// 与真机 handle_manual_cmd 同形（设备选择性响应）
    fn Handle_Manual_Cmd(&self, cmd: &ManualCmd);
    /// 位置更新（真机是底盘 RX 回调写 RobotState）——单写者
    async fn On_Pose(&self, p: &PoseFields);
    async fn On_Spawn(&self, s: &SpawnFields);
    fn Pose_Age_Ms(&self) -> u64;
    fn Stop(&self);
}

pub fn Build(world_name: &str, cfg: &ChassisConfig, motion: MotionSender, robot_state: Arc<RwLock<RobotState>>)
    -> Result<Arc<dyn Chassis>, String>;
```

**5.4 `src/sim/chassis/ugv.rs`**

```rust
pub struct DiffDriveChassis { name: String, speed: SpeedBinding, motion: MotionSender, pose: PoseIngest }

impl Chassis for DiffDriveChassis {
    fn Apply_Action(&self, action: MotionAction) {
        let w = Action_To_Wire(action, &self.speed);
        self.motion.Send(w.action, w.speed);         // 通道满/未装配 → warn（限流），不 panic
    }
    fn Handle_Manual_Cmd(&self, cmd: &ManualCmd) {
        match cmd {
            ManualCmd::Forward(_)   => self.motion.Send("forward",  self.speed.forward),  // 忽略载荷
            ManualCmd::Backward(_)  => self.motion.Send("backward", self.speed.forward),
            ManualCmd::SpinLeft(_)  => self.motion.Send("left",     self.speed.turn),
            ManualCmd::SpinRight(_) => self.motion.Send("right",    self.speed.turn),
            ManualCmd::Stop         => self.motion.Send_Stop_Priority(),
            ManualCmd::Beep(_)      => warn!("[sim] Beep 世界无对应指令，忽略"),           // 车支持蜂鸣、世界不支持
            _ /* Takeoff/Land/Lidar* */ => {}                                          // 车不响应（与真机一致）
        }
    }
}
```

**5.5 `src/sim/chassis/uav.rs`**

- `Apply_Action` 同表（`turn` 用 `turn_speed`，即 `yaw_rate_deg` 的百分比版本）
- `Handle_Manual_Cmd`：`Takeoff → "up" + speed.turn`、`Land → "down" + speed.turn`、`Forward/Backward/Spin*` 同差速、`Beep` 忽略
- `climb` 速度复用 `turn_speed`（世界侧 `Quadrotor` 的 `up/down` 用 `max_climb_rate·pct`，与 `left/right` 共用 `SpeedPct`）——**G3 的近似**在这里体现

**5.6 `src/sim/chassis/pose_ingest.rs`（从 Step 3 移入，内容不变）**

- `On_Pose` 写 `x/y/z/attitude.yaw/vx/vy` + 刷新本地 `Instant`
- `On_Spawn` 覆盖 origin
- `Age_Ms()`：未收到过 → `u64::MAX`

**5.7 `src/sim/sim_handler.rs`**

```rust
struct SimInner {
    goal_service: Arc<Mutex<GoalService>>,
    executor: DecisionExecutor,
    chassis: Option<Arc<dyn Chassis>>,     // 新增：无 [chassis] 或 enabled=false 时为 None
    sensors: Vec<Arc<dyn Sensor>>,
}

async fn on_tick(&self, rs: &RobotState, es: &Arc<RwLock<ExecuteState>>) {
    // 装配门闩（Step 2）
    // 无底盘 → 保持 Idle（与真机 enabled=false 语义一致）
    // 寻路 → 决策 → 写 ExecuteState → 若有 action：chassis.Apply_Action(action)
}

async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
    // 与真机 robot_handler 同构：先拦截雷达命令，其余交底盘
    match cmd { ManualCmd::StartLidarScan | StopLidarScan => info!("[sim] 雷达由世界驱动，忽略"), _ => chassis.Handle_Manual_Cmd(cmd) }
}

async fn reset(&self, es) { chassis.Stop(); goal_service.reset(); *es = Idle }
fn stop(&self)           { chassis.Stop(); }
async fn shutdown(&self) { sensors 停 + 底盘停止 + WorldLink 停 }
```

### 6 数据流

```
决策链（50 ms）on_tick
   → DecisionExecutor::decide(...) → DecisionResult.action: Option<MotionAction>
   → chassis.Apply_Action(action)
        → action_map::Action_To_Wire(action, SpeedBinding{forward,turn})
        → MotionSender（有界通道，mpsc 32）
        → WorldLink 发送 task → junction 底盘连接 → motion_cmd{action, speed}

世界 chassis 连接 ──pose──▶ chassis.On_Pose ──▶ RobotState{x,y,z,attitude.yaw,vx,vy} + 新鲜度
世界 chassis_ready ──spawn──▶ chassis.On_Spawn ──▶ 初始位姿（覆盖 CLI origin）

ManualCmd（地面站/TUI）──▶ SimDeviceHandler::handle_manual_cmd ──▶ chassis.Handle_Manual_Cmd
reset / stop / shutdown ──▶ chassis.Stop() ──▶ Send_Stop_Priority（不静默丢）
```

### 7 完成判据

**编译与单测**

1. `cargo check -p pleiades-sim`、`cargo test -p pleiades-sim` 全绿
2. `action_map` 单测：五个动作逐项断言 `(action, speed)`；`SpeedBinding` 的 `clamp(0,100)`；`Stop` 的 speed 恒 0 —— **把 §2.1 的检查表变成可执行断言**
3. 手动命令单测：差速忽略 `Takeoff/Land`；四旋翼 `Takeoff → up`、`Land → down`；`Beep` 只 warn 不下发
4. `PoseIngest`：`On_Pose` 后字段正确、`Age_Ms()` 递增、`On_Spawn` 覆盖 origin

**禁止引用**

5. `grep -rE "pleiades_ugv|pleiades_uav" pleiades-sim/` 无输出
6. `git status`：改动只在 `pleiades-sim/`（外加 Step 1 的 `orion-simulation-world/`）

**联调（真跑）**

7. 发一条 Goto → 世界里的车朝目标转、再直行；`pose.yaw` 先变、`pose.x/y` 后变，方向正确
8. `TurnLeft` 时 `pose.yaw` **递减**、`TurnRight` 时 **递增**（与 Step 1 的 P6 互为正反证）
9. sim 日志里每条下发的 `(action, speed)` 与配置一致（`forward_speed=30` → `speed=30`，不是 0.3、也不是 1.5）
10. 位置更新：`RobotState.x/y/yaw` 跟随 `pose`；`vx/vy` 非 0（车在动时）
11. 停车链路：切 Manual / `reset` / `shutdown` 都发出 `stop`；世界里的车停下（`pose` 不再变化）
12. `[chassis] enabled = false` → 不建底盘、`on_tick` 保持 Idle（与真机一致）

### 8 风险与坑

| # | 风险 | 处理 |
|---|---|---|
| 1 | `stop()` 是同步 fn，不能 await 发送 | 有界通道 + `pending_stop` 补发（G1），绝不静默丢弃 |
| 2 | 世界只取最新一条 `motion_cmd` | 不排队、不补发历史；`None` 表示"保持"与真机一致 |
| 3 | UAV 的速度语义是近似（G3） | `vel_fwd`/`yaw_rate_deg` 各映射一个 0~100 配置项；记录差异 |
| 4 | `battery/gyro/accel/mag/vz` 世界不提供 | 保持默认 0，**不造数**；记录为已知差异（Pictor 面板会显示 0） |
| 5 | 未装配时 `Apply_Action` 也可能被调用（例如门闩逻辑漏了） | `MotionSender` 未装配时直接丢弃 + `debug!`，不下发 |
| 6 | `[chassis]` 与 ugv 同名段：同一份 config 在两个进程里各有解释 | sim 只读 `enabled/forward_speed/turn_speed`；`port/baudrate/car_type` 等与 sim 无关，忽略即可 |
| 7 | 方向搞反（left/right 符号） | 已有 §2.2 推导 + §7-8 的双向联调断言；单测锁定动作表 |

### 9 不在本 step

- 急停（读 scan 后强制 `stop`，属决策策略）
- `set_motion` 连续速度接口（世界不支持，G2）
- 里程计积分（世界权威，D8；`odometry` 在 sim 里不需要）
- 电池/IMU 等世界不提供字段的模拟
- 多底盘切换（运行期换 `[sim_world].chassis`）

---

## 七、Step 5 — sim 本地控制口（WS 9098）：端到端验收的注入通路

### 1 目标与定位

给 `pleiades-sim` 加一个**本地控制口**，便于在没有人介入的情况下做端到端验收（下发 Goto / 切模式 / 停车 / 查状态）。

**定位是测试夹具，不是产品接口**：

- 只绑 `127.0.0.1`，不对局域网暴露，无鉴权
- **不进决策链**、**不参与 P2P**、**不走 ORION 帧**
- 正式的任务来源仍然是地面站（`pleiades-terminal` → request-response 单播 `DataType::Robot` → `robot_cmd_frame_rx`）

### 2 注入路径（与地面站同一条下游）

```
地面站：TASK_SET 帧 ──▶ command_consumer ──▶ parse_orion_frame ──▶ Command ──┐
                                                                              ├──▶ cmd_rx ──▶ main_loop
控制口：                               直接构造 Command ──▶ robot_cmd_tx.send ─┘
                                                                              │
                                            op_mode 门闩 → mission_queue.replace → device.reset()
```

- 通道在 base：`Robot.robot_cmd_tx: pub mpsc::Sender<Command>`（`pleiades-base/src/Robot/core/robot.rs:48`）
- 发送端在 sim：`sim_bootstrap` 已返回 `Arc<Robot>`
- **与地面站唯一的差异**：跳过 `parse_orion_frame` 的帧解码；`Command` 之后完全同路（替换语义、模式门闩、`device.reset()` 一致）

### 3 配置（默认开启，可关闭）

```toml
[control]
enabled = true            # 默认值就是 true；写 false 才关
bind = "127.0.0.1"        # 只绑本机，严禁 0.0.0.0
port = 9098
```

- 这是**测试用**的口，所以**默认开启**：`config.toml` 里**完全没有 `[control]` 段**时，按 `enabled = true` / `127.0.0.1` / `9098` 起监听
- 关闭方式：`[control] enabled = false`
- 关闭时必须打一行 `info!("[control] 已关闭（[control].enabled = false）")`，避免"连不上却不知道是没开"
- 其余字段（`bind` / `port`）可单独覆盖，缺省值同上

### 4 协议（WS 文本帧，一问一答）

| 请求 | 参数 | 注入的 `Command` | 说明 |
|---|---|---|---|
| `{"cmd":"goto"}` | `x`, `y`（米，`[0,128)`） | `Command::Auto(AutoCmd::Set(vec![Mission::Goto{x, y, members: vec![]}]))` | 单车 Goto |
| `{"cmd":"circle"}` | `x`, `y`（圆心） | `Command::Auto(AutoCmd::Set(vec![Mission::Circle{x, y, members: vec![]}]))` | 单车围圈 |
| `{"cmd":"stop"}` | — | Auto 模式：`Auto(Set(vec![]))`；Manual 模式：`Manual(ManualCmd::Stop)` | **按当前模式选**，否则会被门闩吞掉 |
| `{"cmd":"mode"}` | `value`: `"auto"` / `"manual"` | `Command::Mode(ModeCmd::SwitchToAuto / SwitchToManual)` | |
| `{"cmd":"manual"}` | `action`: `forward` `backward` `spin_left` `spin_right` `stop` | `Command::Manual(ManualCmd::Forward(0) / Backward(0) / SpinLeft(0) / SpinRight(0) / Stop)` | **只在 Manual 模式生效**；载荷 `i16` 传 0（真机也忽略它，用配置速度） |
| `{"cmd":"status"}` | — | 无（只读） | 回状态 JSON |
| `{"cmd":"help"}` | — | 无 | 回命令列表 |

答复格式：

```jsonc
{"ok": true,  "cmd": "goto"}                       // 已投递进 cmd_tx
{"ok": false, "cmd": "goto", "err": "x 超范围"}     // 参数/模式校验失败
{"ok": true,  "cmd": "status",
 "data": {"peer_id":"12D3Koo…","assembled":true,"avatar_id":1,
          "mode":"auto","x":64.02,"y":63.98,"z":0.0,"yaw":0.12,"rtk_fixed":false}}
```

**模式校验（避免"静默无效"）**：`manual` 命令在 Auto 模式下会被 main_loop 忽略，所以控制口在发送前先读 `*robot.op_mode.read().await`，不匹配直接回 `{"ok":false,"err":"当前为 Auto 模式，manual 命令不会生效"}`，**不发**。

**`members` 本 step 不支持**：`Mission::{Goto,Circle}.members: Vec<Vec<u8>>` 是 peer_id 字节，转换需要 `libp2p::PeerId` 解析，而 `pleiades-sim` 不直接依赖 libp2p。群发仍走地面站。

### 5 涉及文件（全部在 `pleiades-sim/`）

| 文件 | 动作 | 内容 |
|---|---|---|
| `src/config.rs` | 改 | 新增 `[control]` 段（`enabled` / `bind` / `port`） |
| `src/sim/control.rs` | **新建** | `TcpListener` + `accept_async` + 逐连接读循环 + 命令分派 + 状态回包 |
| `src/sim/control.rs` 内单测 | — | `Parse_Control(&str) -> Result<ControlCmd, String>` 纯函数，覆盖非法 JSON / 缺字段 / 越界 / 未知 cmd |
| `src/sim/mod.rs` | 改 | `pub mod control;` |
| `src/sim/sim_handler.rs` | 改 | `start()` 里 `control::Spawn(...)`（持有 `Arc<Robot>` + `Option<WorldLinkHandle>`） |

### 6 关键实现

```rust
pub enum ControlCmd {
    Goto { x: f32, y: f32 },
    Circle { x: f32, y: f32 },
    Stop,
    Mode { auto: bool },
    Manual { action: String },
    Status,
    Help,
}

pub fn Parse_Control(text: &str) -> Result<ControlCmd, String>;

/// 起控制口：绑 127.0.0.1:port，失败只 warn（不阻断仿真）
pub fn Spawn(cfg: ControlConfig, robot: Arc<Robot>, world: Option<WorldLinkHandle>, cancel: CancellationToken);
```

要点：

1. `tokio::net::TcpListener::bind((bind.as_str(), port))`；失败 → `warn!("[control] 监听失败 …")` **并继续跑**（控制口挂了不该拖死仿真），但日志必须显眼
2. 每连接一个 task：`accept_async(stream)` → `while let Some(msg) = ws.next().await` → 只处理 `Message::Text`，`Binary` 忽略
3. 分派后 `robot.robot_cmd_tx.send(cmd).await`；`send` 失败（通道关闭）→ `{"ok":false,"err":"cmd 通道关闭"}`
4. `status` 读 `robot.robot_state`（x/y/z/yaw/rtk_fixed）+ `robot.op_mode` + `WorldLinkHandle::State()/Avatar_Id()`
5. 参数校验：`x`/`y` 必须有限且落在 `[0,128)`（与 `pleiades-sim/src/main.rs` 的 `WORLD_MIN/WORLD_MAX` 一致）；否则 `{"ok":false,"err":"x 超范围"}`
6. 每次注入打一行 `info!("[control] goto (36.0, 92.0) → cmd_tx")`，便于日志对账
7. 不新增依赖：`tokio-tungstenite`（Step 2 已加）+ `serde_json`（Step 2 已加）

> `WorldLinkHandle` 需要补一个只读访问器 `Avatar_Id() -> Option<u16>`（Step 2 的 handle 只定义了 `State/Is_Assembled/Send_Motion/Shutdown`），供 `status` 使用。

### 7 完成判据

**编译与单测**

1. `cargo check -p pleiades-sim`、`cargo test -p pleiades-sim` 全绿
2. `Parse_Control` 单测：四种合法命令解析正确；非法 JSON / 缺 `cmd` / 未知 `cmd` / `x` 缺失 / `x=200`（越界）/ `mode.value` 非法 → 各自的错误文本
3. 端口占用时：`bind` 失败只 warn，进程继续（用两次启动验证）

**联调（本 step 的核心价值：终于能端到端验收）**

4. 起世界 → 起 sim → 装配完成（`[world] assembled avatar=1 slots=1`）
5. `ws://127.0.0.1:9098` 发 `{"cmd":"status"}` → 回 `assembled:true` + 当前位姿（应等于 `chassis_ready.spawn`）
6. 发 `{"cmd":"goto","x":36,"y":92}` → 回 `ok:true` → 世界里的车开始朝该方向转、再直行（`pose.yaw` 先变、`pose.x/y` 后变）
7. 发 `{"cmd":"stop"}` → 车停下（`pose` 不再变化），日志出现 `[control] stop`
8. 发 `{"cmd":"mode","value":"manual"}` → 再发 `{"cmd":"manual","action":"forward"}` → 车直行；回到 auto 后再发 `manual` → 回 `ok:false` 且**不发**命令
9. `{"cmd":"goto","x":200,"y":0}` → `ok:false` 且 cmd 未投递
10. **默认开启**：配置里**删掉整个 `[control]` 段**后启动，9098 依然能连、`status` 正常返回
11. **可关闭**：`[control] enabled = false` 后启动，9098 连不上，日志有 `[control] 已关闭`，世界链路与仿真照常

### 8 风险与坑

| # | 风险 | 处理 |
|---|---|---|
| 1 | 控制口无鉴权，且**默认开启** | **只绑 `127.0.0.1`**（本机任意进程可连，但要跑在别人能访问的机器上时设 `enabled = false`）；文档写明不得改 0.0.0.0；不参与 P2P |
| 2 | `manual` 命令在 Auto 模式被 main_loop 静默忽略 | 控制口发送前读 `op_mode` 校验，不匹配直接回 `ok:false`（不静默） |
| 3 | `stop` 在两种模式下走不同命令 | 按当前模式选 `Auto(Set(vec![]))` 或 `Manual(Stop)` |
| 4 | 控制口与地面站同时下发 | 两者都是 `Auto(Set)` **替换**语义，后到的覆盖先到的（与真机一致）；日志可对账 |
| 5 | 端口 9098 被占 | 只 warn 不阻断；`status` 也拿不到 → 验收时先确认端口空闲 |
| 6 | `members` 群发不支持 | 明确回 `ok:false,"err":"members 不支持，请用地面站"`，不假装成功 |
| 7 | 长连接不设超时 | 逐连接循环直到对端关闭；`cancel` 时全部断开 |

### 9 不在本 step

- 群发 `members`（需要 libp2p `PeerId` 解析）
- 鉴权 / TLS / 多客户端并发写的一致性
- 视频流、地图/点云可视化
- 把控制口推广到 ugv / uav（如需长期调试口，再决定是否上提 base）

---

## 八、Step 6 — 行动逻辑复刻：把真车 `on_tick` 的全链路搬到 sim

> 编号说明：原计划的「测试」步骤顺延为 Step 7。

### 1 目标

`pleiades-sim` 的 `on_tick` 与真车 `CarDeviceHandler::on_tick`（`pleiades-ugv/src/ugv/robot_handler.rs:217-244`）**逐项对齐**，包括现在缺的那一步——**急停**。

对齐之后，sim 的每个 50 ms tick 与真车完全同构：急停 → 寻路 → 决策 → 执行。

### 2 对齐表（真车 vs sim 现状）

| 顺序 | 真车 `CarDeviceHandler::on_tick` | sim 现状 | 本 step 要做 |
|---|---|---|---|
| 0 | 无底盘（`stm32` 为 `None`）→ `ExecuteState = Idle`，直接返回 | 已有（Step 4） | 保持 |
| 1 | **急停**：`check_emergency_stop(lidar, rs, stm32, &goal_service)`；触发则 `stm32.stop()` + `ExecuteState = Idle`（**保留 goal** 供绕行） | **没有** | **新增** |
| 2 | 寻路：`goal_service.lock().await.get_path().await` | 已有 | 保持 |
| 3 | 决策：`executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current)` | 已有 | 保持 |
| 4 | 写状态：`ExecuteState { state, sub_target }` | 已有 | 保持 |
| 5 | 执行：`if let Some(action) = result.action { stm32.apply_action(action) }` | 已有（Step 4 → `motion_cmd`） | 保持 |
| 6 | 无雷达（`lidar` 为 `None`）→ 不急停，直接进决策 | — | 同语义（sensor 可选） |

### 3 急停逻辑（逐行复刻 `pleiades-ugv/src/ugv/emergency_stop.rs`，代码独立）

```rust
const OBSTACLE_THRESHOLD_M: f32 = 0.3;

pub async fn Check_Emergency_Stop(
    scan: Option<LaserScan>,          // LidarSensor::Get_Scan()（Step 3）
    rs: &RobotState,
    chassis: &dyn Chassis,            // Stop()：发 stop
    goal_service: &Arc<tokio::sync::Mutex<GoalService>>,
    mount: &MountOut,                 // 传感器外参（LidarSensor::Mount()），只用于算障碍格
) -> bool
```

判定（与真机一字不差）：

1. 无 scan → `false`（真机 `let Some(scan) = ... else { return false }`）
2. 取最近点：`a = angle < 0 ? angle + 2π : angle`；**筛选 `range >= 0.1`**（世界侧无回波是 `0.0`，正好被这条过滤掉）**且 `a < π/4 || a >= 7π/4`**（前方 ±45° 扇区）
3. 最近点 `range < 0.3` → 触发：
   - `chassis.Stop()`（真机是 `stm32.stop()`；sim 走 Step 4 的 `Send_Stop_Priority`，不静默丢）
   - 障碍格：**必须补外参 + 外推**（与建图同式，`Obstacle_Cell`）：
     - `Sensor_Pose(x, y, yaw, mount)` 把车体位姿换算到**传感器挂载点**（与 `mapping_task::Sensor_Pose` 同一函数，同一取位姿口径）
     - `a = sensor.yaw + p.angle`（世界侧保证 `angle` 是"从车头 +X 起、yaw 增大方向"，所以只需加挂载 yaw，不做符号取反）
     - 命中点沿射线**外推 `HIT_EPS_M = 0.02 m`** 再量化：`wx = sx + (r + 0.02)·cos a`、`wy = sy + (r + 0.02)·sin a`，格 = `floor(/0.5)`
     - **为什么必须这样**（2026-10-21 实测，Step 7 第 3 轮 D8）：不补外参会把障碍格标到墙**前面一格**（实测 `(99,183)`），因为挂载点前移 0.1 m ≈ 格子宽度的 20%；不补 2 cm 外推则墙面正好落在格边界时，命中格会在相邻两格间反复翻面（约 1 Hz）。修正后实测落在 `wall_c` 本体 `(99,184)` / `(101,184)`，与 mapper 写进 grid 的格子一致。
   - `goal_service.lock().await.mark_obstacle((gx, gy)).await`
   - 返回 `true`

**关键理解（决定实现对不对）**：`mark_obstacle` 只是**促使 D* 重算受影响邻居**，它本身不改地图。真正让 D* 改路的是 **mapper 把那格写进 `grid` 并累积到 `Occupied`**（log-odds：命中 `+3`，`> +6` 才算 Occupied → 约 3 帧 = 0.3 s）。所以：

- 急停必须**每 tick 都复核**（车停着、前方仍有 0.2 m 点 → 持续触发），不能做成"一次触发就永久锁定"
- 真机就是这么工作的；sim 照抄即可，**不要额外给 sim 造"感知直连规划"的旁路**

### 4 涉及文件（全部在 `pleiades-sim/`）

| 文件 | 动作 | 内容 |
|---|---|---|
| `src/sim/emergency_stop.rs` | **新建** | `Check_Emergency_Stop`（40 行）+ 单测（见 §6） |
| `src/sim/mod.rs` | 改 | `pub mod emergency_stop;` |
| `src/sim/sim_handler.rs` | 改 | `on_tick` 插入急停分支（顺序：门闩 → 无底盘 → 急停 → 寻路 → 决策 → 动作） |

### 5 逐文件改动

**5.1 `src/sim/emergency_stop.rs`**

```rust
//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

//! 前方障碍急停（与 pleiades-ugv/src/ugv/emergency_stop.rs 同语义，代码独立，不引用 ugv）。
//! 触发：前方 ±45° 扇区最近点 < 0.3 m → 停车 + 标记障碍格到 D*。

pub async fn Check_Emergency_Stop(...) -> bool { /* §3 的四步 */ }
```

**5.2 `src/sim/sim_handler.rs`（`on_tick` 的最终形状）**

```rust
async fn on_tick(&self, rs: &RobotState, es: &Arc<RwLock<ExecuteState>>) {
    let d = self.inner.get().expect("device not started");

    // 0) 装配门闩（Step 2）：未装配不决策
    if let Some(w) = self.world.get() {
        if !w.Is_Assembled().await {
            *es.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
            return;
        }
    }
    // 1) 无底盘 → 保持 Idle（与真机 stm32 为 None 一致）
    let Some(chassis) = &d.chassis else {
        *es.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        return;
    };

    // 2) 急停（真机第一步）
    let emergency = match d.lidar.as_ref() {
        Some(l) => Check_Emergency_Stop(l.Get_Scan().await, rs, chassis.as_ref(), &d.goal_service).await,
        None => false,                       // 无雷达 → 不急停（真机同语义）
    };
    if emergency {
        chassis.Stop();                      // 真机 stm32.stop()；此处置 Idle、**保留 goal**
        *es.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        return;
    }

    // 3) 寻路 → 4) 决策 → 5) 写状态 → 6) 执行（Step 4 已有，不改）
    let next_cell = d.goal_service.lock().await.get_path().await;
    let current = es.read().await.clone();
    let result = d.executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
    *es.write().await = ExecuteState { state: result.state, sub_target: result.sub_target };
    if let Some(action) = result.action { chassis.Apply_Action(action); }
}
```

> `d.lidar` 从哪来：Step 3 的传感器集合里，`sensor == "lidar_2d"` 的那一个（sim 现在只支持一种传感器，取第一个即可；多个时取 slot 名为 `lidar_*` 的第一个，并在文档里写明）。

### 6 完成判据

**单测（`emergency_stop.rs` 内）**

1. 前方 0.2 m 有点 → `true`
2. 前方 0.5 m 有点 → `false`（阈值 0.3）
3. 侧方 0.2 m 有点（`a = π/2`，在 ±45° 外）→ `false`
4. 全部无回波（`range = 0.0`）→ `false`（`>= 0.1` 过滤）
5. 无 scan（`None`）→ `false`
6. 边界角：`a = π/4 - ε` 触发、`a = π/4 + ε` 不触发；`a = 7π/4` 触发
7. 障碍格坐标：`x=64, y=64, yaw=0, angle=0, range=0.2` → 格 `(128, 128)`（`64.2/0.5 = 128.4` → floor 128）

**联调**

8. 目标设在墙后、且不给地图（或地图尚空）→ 车直冲 → 在墙前约 0.3 m **停下**，日志出现 `急停` 与 `mark_obstacle`
9. 停下后**不出现"贴墙顶死"**：位姿在 3 s 内出现变化（D* 重规划后转向绕行），而不是连续 5 s 位置完全不变
10. 急停是**持续复核**：停车期间每 tick 都能看到急停日志（限流可接受，但不能只出现一次就再也不复核）
11. 直线无遮挡时**不误触发**：空地上正常行驶，`急停` 日志一次都不出现

### 7 风险与坑

| # | 风险 | 处理 |
|---|---|---|
| 1 | 急停只在"前方"判定，车被侧向障碍卡住不会触发 | 与真机一致；侧向靠 mapper + D* |
| 2 | `mark_obstacle` 不写地图，只促使重算 → 单独调它看不出效果 | 依赖 mapper 把该格推成 `Occupied`（3 帧）；**联调必须等 0.3 s 以上再判"没绕开"** |
| 3 | 急停每 tick 打 warn → 日志爆炸 | 限流（同一障碍格 1 s 内只打一次），但**判定与动作不能限流** |
| 4 | 车停住后 `sub_target` 被清 → `ExecuteState` 回 Idle → 下 tick 重新决策 | 与真机一致；`goal` 保留 |
| 5 | 无雷达配置时行为退化 | 与真机 `lidar: None` 同语义；日志提示一次 |
| 6 | 急停阈值 0.3 m 与车体 footprint（半径 0.15 m） | 真机值，照抄；世界里车身半宽约 0.12 m，0.3 m 有安全余量 |

### 8 不在本 step

- 感知直连规划的旁路（真机没有，sim 也不造）
- 侧向/后向避障
- 三维避障（UAV）
- 多雷达融合后的急停

---

## 九、Step 7 — 端到端冒烟测试

### 1 目标

用真实编译产物 + 真实 Godot 世界，证明「sim 能在模拟世界里跑起来」这个任务目标成立。**只做冒烟，不做用例矩阵**。

### 2 环境（一实例一目录）

```
RobotSim_TestEnv/           ← 已被 .gitignore 忽略
├── sim1/                   ← 本 step 只用这一个实例
├── tools/                  ← 探针脚本（不入库）
└── results/                ← 轨迹、日志摘要、结论
```

准备：

```bash
cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion
cargo build -p pleiades-sim
cp target/debug/pleiades-sim RobotSim_TestEnv/sim1/
```

`RobotSim_TestEnv/sim1/.config/config.toml` 里需要：

```toml
[sim_world]
enabled = true
url = "ws://127.0.0.1:9999"
chassis = "diff_drive_ugv"

[chassis]
enabled = true
forward_speed = 30
turn_speed = 10

[control]
enabled = true
bind = "127.0.0.1"
port = 9098

[[sim_world.sensor]]
slot = "lidar_front"
sensor = "lidar_2d"

[sim_world.sensor.mount]
x = 0.1
y = 0.0
z = 0.2
yaw = 0.0

[sim_world.sensor.params]
range = 12.0
rays = 360
rate_hz = 10
```

### 3 用例（4 条，全 PASS 才算通过）

| 用例 | 步骤 | 通过判据 |
|---|---|---|
| **S1 装配** | 起世界（`godot --headless --path orion-simulation-world`），再起 `RobotSim_TestEnv/sim1/pleiades-sim` | 世界日志有 `vehicle spawned` + `sensor_ready` + `assembly_ready`；sim 日志有 `[world] assembled`；控制口 `{"cmd":"status"}` 回 `assembled:true` 且位姿 == `chassis_ready.spawn` |
| **S2 闭环行驶** | 控制口发 `{"cmd":"goto","x":40,"y":80}`（空地，避开 `wall_c`：x∈[36,66] y∈[92,93]） | **160 s** 内到达，`\|x−40\|` 与 `\|y−80\|` 均 ≤ 0.3 m；期间位姿持续变化（不是跳变）；日志无 ERROR |
| **S3 建图与绕行** | 发 `{"cmd":"goto","x":36,"y":110}`（`wall_c` 北侧，直线必穿墙） | **320 s** 内到达且 ≤ 0.3 m；**轨迹在 x<36 或 x>66 处出现拐点**（绕端，不是穿墙） |
| **S4 断线重连** | S2 完成后 kill 世界进程 → 5 s 后重启 | sim 日志出现重连并重新 `assembled`；`status` 位姿 = 新 `spawn`（逐轴 ≤ 1e-3）；**无需重启 sim** |

**2026-10-22 人类裁决，判据据此修订：**

1. **时间预算放宽**（保留世界默认速度 `max_speed=1.5`、`max_turn_rate=90`，不改 `[sim_world].chassis_params`）：`forward_speed=30 × 1.5 = 0.45 m/s`、`turn_speed=10 × 90 = 9°/s`，实测外推 S2 ≈ 132~137 s、S3 ≈ 229~277 s，故取 **S2 160 s / S3 320 s**（+15% 余量）。
2. **急停不再作为端到端判据**：`mapper` 在 5 m 处就把墙写成 `Occupied`、`D*` 在 ~1.2 m 已改道，正常路径**急停永不触发**（实测 0 条），与「必须绕行」几何互斥。急停的验证由 **Step 6 的单测**（7 条）承担；端到端用例不测急停（原 S3b「贴着墙走」用例一并取消）。
3. **`avatar_id` 递增不再作为判据**：`_next_avatar_id` 是 `WebSocketManager.gd` 的**进程内变量**，世界重启即归零，跨重启递增在设计上不可达。改为「重新装配 + 位姿回到新 `spawn`」。

### 4 观测手段

- 控制口 `status` 每 200 ms 采一次 → `RobotSim_TestEnv/results/s1_trace.csv`（`t, case, assembled, avatar_id, x, y, z, yaw`；`case` 列用于逐用例区分轨迹，2026-10-22 按实测补入）
- sim 日志：`[world] assembled` / `[control] ...` / 急停 / `[D*]` 关键行
- 世界日志：`[ws] vehicle spawned ... spawn=(...)` / `assembly_ready` / `vehicle removed`

### 5 完成判据

1. S1~S4 全 PASS，且 `results/` 里有轨迹文件与结论记录
2. 全程 `grep -E "ERROR|SCRIPT ERROR|Parse Error"` 在两侧日志都为空；无 panic、无僵尸进程
3. 结论写进 `Workbook/wb_31_sim_world_link.md`
4. `grep -rE "pleiades_ugv|pleiades_uav" pleiades-sim/` 仍为空（Step 3/4 的硬约束在联调后再确认一次）
5. `git status`：代码改动只在 `pleiades-sim/`（外加 Step 1 的 `orion-simulation-world/`）；测试产物都在 `RobotSim_TestEnv/`（已忽略）

### 6 不在本 step

- 多车并跑（多实例、P2P 发现、他车膨胀）
- UAV 场景
- 与地面站（Pictor）联调
- 性能/长稳测试
