# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 位姿下行自检（task_31 §三 / Step 1）：协议形态 / World 接线 / 真 WS 端到端 10 Hz 位姿。
# 用法：timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_pose.gd
# 期望：failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR/SCRIPT ERROR。
#
# 覆盖 task_31 §三 6.3 的 P1~P9：
#   A1~A7 树外（无场景）：常量 / BuildPose 形状与默认值 / JSON 回环 / 周期常量 /
#         _process(delta) 接线 / _BroadcastPoses 白盒 lint（E23 的三条禁止变红灯）
#   B1~B2 建真 World.tscn + Listen(0) 临时端口
#   C1~C12 真 WS + 真场景 + 真物理：速率 10 Hz / 字段 / 与 chassis_ready.spawn 一致 /
#         yaw 范围与世界时间单调 / 传感器连接收不到 pose / forward 的 x 与 vx /
#         left 的 yaw 与 vy 符号 / 断开后不再发 / vehicle==null 与"每周期每连接一次"
#
# 时间基（关键，§7-⑤）：--fixed-fps 60 关闭实时同步，墙钟与仿真时间脱钩，
#   所以"窗口 / 速率"断言一律按**帧数**（_wait_frames / _wait_physics），
#   墙钟只用于网络事件的 deadline（沿用 check_ws_manager.gd 已验证写法）。
#
# 白盒断言（manager._links 注入替身）在本仓库是既有做法（check_ws_manager.gd 的 A8 同例）。

extends SceneTree

const WireProtocol := preload("res://src/net/WireProtocol.gd")
const ComponentLink := preload("res://src/net/ComponentLink.gd")

const WS_MANAGER_SCRIPT := "res://src/net/WebSocketManager.gd"
const WIRE_SCRIPT := "res://src/net/WireProtocol.gd"
const WORLD_SCENE := "res://src/scenes/World.tscn"

const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)
## P3：x/y/yaw 走 1e-6（64.0 -> (x,z,y) -> 64.0 往返精确），z 单独 1e-3（竖直方向落地沉降）。
const POSE_TOL := 1e-6
const POSE_TOL_Z := 1e-3
## 位姿消息的 8 个键（多一个少一个都算契约破裂）。
const POSE_KEYS := ["msg", "vx", "vy", "world_time", "x", "y", "yaw", "z"]
## 60 fps 下 1 s = 60 帧；10 Hz 容忍 1 帧漏发。
const FRAMES_PER_SECOND := 60
const MIN_POSES_PER_SECOND := 9

var _total: int = 0
var _failed: int = 0
var _harness_errors: int = 0
var _sims: Array = []


# ================================================================ 测试替身：sim 侧 WS 客户端

class Sim:
	var peer: WebSocketPeer = null
	## 已解析的文本帧（FIFO）
	var inbox: Array = []
	## 收到的二进制帧计数（位姿必须是文本帧，二进制计数只用于诊断）
	var binary_count: int = 0
	## 解析失败的文本帧计数
	var bad_json_count: int = 0
	var closed: bool = false
	var close_code: int = -1000

	func Connect(port: int) -> Error:
		peer = WebSocketPeer.new()
		peer.set_heartbeat_interval(10.0)
		return peer.connect_to_url("ws://127.0.0.1:%d" % port)

	func Poll() -> void:
		if peer == null:
			return
		peer.poll()
		var state: int = peer.get_ready_state()
		if state == WebSocketPeer.STATE_OPEN or state == WebSocketPeer.STATE_CLOSING:
			while peer.get_available_packet_count() > 0:
				var bytes: PackedByteArray = peer.get_packet()
				if not peer.was_string_packet():
					binary_count += 1
					continue
				var json := JSON.new()
				if json.parse(bytes.get_string_from_utf8()) != OK or not (json.data is Dictionary):
					bad_json_count += 1
					continue
				var msg: Dictionary = json.data
				inbox.append(msg)
		if state == WebSocketPeer.STATE_CLOSED and not closed:
			closed = true
			close_code = peer.get_close_code()

	func IsOpen() -> bool:
		return peer != null and not closed and peer.get_ready_state() == WebSocketPeer.STATE_OPEN

	func Send(msg: Dictionary) -> Error:
		if not IsOpen():
			return ERR_UNCONFIGURED
		return peer.send_text(JSON.stringify(msg))

	## 客户端主动断开（模拟 sim 进程退出）。
	func CloseClient() -> void:
		if peer != null and not closed:
			peer.close(1000, "test bye")

	func Has(msg_name: String) -> bool:
		return Count(msg_name) > 0

	## 摘走第一条同 msg 的记录；没有 -> {}。
	func Take(msg_name: String) -> Dictionary:
		for i in range(inbox.size()):
			var dict: Dictionary = inbox[i]
			if str(dict.get("msg", "")) == msg_name:
				inbox.remove_at(i)
				return dict
		return {}

	func Names() -> Array:
		var out: Array = []
		for msg: Variant in inbox:
			var dict: Dictionary = msg
			out.append(str(dict.get("msg", "")))
		return out

	## 同 msg 的条数（姿态下行的"速率"证据）。
	func Count(msg_name: String) -> int:
		var n := 0
		for msg: Variant in inbox:
			var dict: Dictionary = msg
			if str(dict.get("msg", "")) == msg_name:
				n += 1
		return n

	## 摘走**全部**同 msg 的记录（保持到达顺序 = 发送顺序）。
	func Drain(msg_name: String) -> Array:
		var out: Array = []
		var kept: Array = []
		for msg: Variant in inbox:
			var dict: Dictionary = msg
			if str(dict.get("msg", "")) == msg_name:
				out.append(dict)
			else:
				kept.append(dict)
		inbox = kept
		return out


# ================================================================ 测试替身：位姿计数的 avatar（P9）

## 必须是 CharacterBody3D：_BroadcastPoses 读 vehicle.velocity（纯 Node3D 替身会 Invalid access）。
class PoseSpy extends CharacterBody3D:
	var calls: int = 0

	func GetPose() -> Dictionary:
		calls += 1
		return {"x": 70.0, "y": 71.0, "z": 0.5, "yaw": 0.25}


# ================================================================ 入口

func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ A. 树外：协议形态
	var wire: Script = load(WIRE_SCRIPT) as Script
	var consts: Dictionary = wire.get_script_constant_map() if wire != null else {}
	var pose_ok: bool = consts.has("MSG_POSE") and str(consts.get("MSG_POSE", "")) == "pose"
	var old_ok: bool = str(consts.get("MSG_CHASSIS_HELLO", "")) == "chassis_hello" \
		and str(consts.get("MSG_CHASSIS_READY", "")) == "chassis_ready" \
		and str(consts.get("MSG_SENSOR_HELLO", "")) == "sensor_hello" \
		and str(consts.get("MSG_SENSOR_READY", "")) == "sensor_ready" \
		and str(consts.get("MSG_MOTION_CMD", "")) == "motion_cmd" \
		and str(consts.get("MSG_ASSEMBLY_READY", "")) == "assembly_ready"
	var legacy_gone: bool = not consts.has("MSG_IMU") and not consts.has("MSG_GNSS")
	_check("A1 wire-const-pose", wire != null and pose_ok and old_ok and legacy_gone,
		"pose=%s old=%s imu/gnss-gone=%s" % [pose_ok, old_ok, legacy_gone])

	# A2：8 键、vz 被丢弃、float32 分量用 1e-6 比较（§7-⑰）
	var sample: Dictionary = WireProtocol.BuildPose(
		1.5, {"x": 64.0, "y": 65.0, "z": 2.0, "yaw": 0.25}, Vector3(0.3, -0.4, 1.1))
	var keys: Array = sample.keys()
	keys.sort()
	var keys_ok: bool = str(keys) == str(POSE_KEYS)
	var fields_ok: bool = str(sample.get("msg", "")) == "pose" \
		and absf(float(sample.get("world_time", -1.0)) - 1.5) < 1e-9 \
		and absf(float(sample.get("x", -1.0)) - 64.0) < 1e-9 \
		and absf(float(sample.get("y", -1.0)) - 65.0) < 1e-9 \
		and absf(float(sample.get("z", -1.0)) - 2.0) < 1e-9 \
		and absf(float(sample.get("yaw", -1.0)) - 0.25) < 1e-9 \
		and absf(float(sample.get("vx", -99.0)) - 0.3) < POSE_TOL \
		and absf(float(sample.get("vy", -99.0)) + 0.4) < POSE_TOL
	_check("A2 build-pose-shape", keys_ok and fields_ok and not sample.has("vz"),
		"keys=%s fields=%s sample=%s" % [keys, fields_ok, sample])

	# A3：缺 key 不崩，四个位姿字段回 0.0
	var blank: Dictionary = WireProtocol.BuildPose(0.0, {}, Vector3.ZERO)
	_check("A3 build-pose-defaults",
		blank.size() == 8
		and absf(float(blank.get("x", -1.0))) < 1e-9
		and absf(float(blank.get("y", -1.0))) < 1e-9
		and absf(float(blank.get("z", -1.0))) < 1e-9
		and absf(float(blank.get("yaw", -1.0))) < 1e-9
		and float(blank.get("vx", -1.0)) == 0.0 and float(blank.get("vy", -1.0)) == 0.0,
		"blank=%s" % [blank])

	# A4：可上线（JSON 回环无 StringName 泄漏）
	var back: Dictionary = WireProtocol.ParseJson(WireProtocol.MakeJson(sample))
	_check("A4 build-pose-json",
		str(back.get("msg", "")) == "pose"
		and typeof(back.get("msg", null)) == TYPE_STRING
		and absf(float(back.get("world_time", -1.0)) - 1.5) < 1e-9
		and absf(float(back.get("x", -1.0)) - 64.0) < 1e-9
		and absf(float(back.get("z", -1.0)) - 2.0) < 1e-9
		and absf(float(back.get("vx", -99.0)) - 0.3) < POSE_TOL,
		"round_trip=%s" % [back])

	# A5：周期常量 = 0.1 s（硬编码，不进配置）
	var manager_script: Script = load(WS_MANAGER_SCRIPT) as Script
	var manager_consts: Dictionary = manager_script.get_script_constant_map() \
		if manager_script != null else {}
	var interval := float(manager_consts.get("POSE_BROADCAST_INTERVAL_S", -1.0))
	_check("A5 manager-interval", absf(interval - 0.1) < 1e-9,
		"POSE_BROADCAST_INTERVAL_S=%s" % [interval])

	# A6 / A7：源码 lint（去整行注释后按函数体截取）
	var manager_code: String = _code_of(WS_MANAGER_SCRIPT)
	var process_body: String = _func_body(manager_code, "_process")
	_check("A6 manager-process-delta",
		process_body.contains("func _process(delta: float)")
		and process_body.contains("_BroadcastPoses(delta)"),
		"body=%s" % [process_body.replace("\n", " | ")])

	var broadcast_body: String = _func_body(manager_code, "_BroadcastPoses")
	var forbidden: Array = []
	for token: String in ["global_position", "rotation.y", "global_rotation"]:
		if broadcast_body.contains(token):
			forbidden.append(token)
	_check("A7 broadcast-lint",
		broadcast_body.contains("GetPose") and broadcast_body.contains("VelToWorld(")
		and broadcast_body.contains("WireProtocol.BuildPose") and forbidden.is_empty(),
		"forbidden=%s body=%s" % [forbidden, broadcast_body.replace("\n", " | ")])

	# ============================================================ B. World.tscn 接线
	var world_packed: PackedScene = load(WORLD_SCENE) as PackedScene
	if world_packed == null:
		_fatal("B1 world-manager-listen", "%s 不存在" % WORLD_SCENE)
		_finish()
		return
	var world: Node3D = world_packed.instantiate() as Node3D
	var manager: Node = world.get_node_or_null("Systems/WebSocketManager") if world != null else null
	if manager == null:
		_fatal("B1 world-manager-listen", "World/Systems/WebSocketManager 缺失")
		if world != null:
			world.free()
		_finish()
		return
	var default_autolisten: bool = manager.autolisten
	var default_listen_port: int = manager.listen_port
	manager.autolisten = false          # 自检自带 Listen(0)，不抢 9999
	root.add_child(world)
	await process_frame

	var manager_script_live: Script = manager.get_script()
	var listen_err: Error = manager.Listen(0)
	var port: int = manager.GetListenPort()
	_check("B1 world-manager-listen",
		manager_script_live != null and manager_script_live.resource_path == WS_MANAGER_SCRIPT
		and default_autolisten and default_listen_port == 0
		and listen_err == OK and port > 0
		and manager.GetStats().get("listening", false) == true,
		"script=%s autolisten=%s listen_port=%d err=%d port=%d stats=%s" % [
			manager_script_live.resource_path if manager_script_live != null else "null",
			default_autolisten, default_listen_port, listen_err, port, manager.GetStats()])
	_check("B2 initially-empty",
		manager.GetVehicleCount() == 0 and int(manager.GetStats().get("links", -1)) == 0,
		"stats=%s" % [manager.GetStats()])

	# ============================================================ C. 端到端（真 WS + 真场景 + 真物理）
	var chassis: Variant = _new_sim(port)
	await _wait_open(chassis, 1500)
	chassis.Send({
		"msg": "chassis_hello",
		"peer_id": "simPose",
		"chassis": "diff_drive_ugv",
		"params": {"max_speed": 1.5},
		"manifest": [{"slot": "lidar_front", "sensor": "lidar_2d"}],
	})
	var ready: Dictionary = await _wait_msg(chassis, "chassis_ready", 1500)
	var spawn: Dictionary = ready.get("spawn", {}) if not ready.is_empty() else {}
	_check("C1 chassis-ready",
		not ready.is_empty() and str(ready.get("peer_id", "")) == "simPose"
		and int(ready.get("avatar_id", -1)) == 1
		and absf(float(spawn.get("x", -9.0)) - SPAWN_WORLD.x) < POSE_TOL
		and absf(float(spawn.get("y", -9.0)) - SPAWN_WORLD.y) < POSE_TOL
		and absf(float(spawn.get("z", -9.0)) - SPAWN_WORLD.z) < POSE_TOL
		and absf(float(spawn.get("yaw", -9.0))) < POSE_TOL,
		"ready=%s spawn=%s" % [ready, spawn])

	# C2 / P1：1.0 s（60 帧）内 >= 9 条 pose（10 Hz）
	chassis.Drain("pose")                       # 丢掉与 chassis_ready 同帧的零星帧，窗口从下一帧起
	await _wait_frames(FRAMES_PER_SECOND)
	var window: Array = chassis.Drain("pose")
	_check("C2 pose-rate-10hz", window.size() >= MIN_POSES_PER_SECOND,
		"%d 帧收到 %d 条（期望 >= %d）" % [FRAMES_PER_SECOND, window.size(), MIN_POSES_PER_SECOND])

	# C3 / P2：字段齐全、类型正确、键集合恰为 8 键
	var bad_type: Array = []
	var bad_keys: Array = []
	for item: Variant in window:
		var pose: Dictionary = item
		var got: Array = pose.keys()
		got.sort()
		if str(got) != str(POSE_KEYS):
			bad_keys.append(got)
		for key: String in ["world_time", "x", "y", "z", "yaw", "vx", "vy"]:
			var value: Variant = pose.get(key, null)
			var t := typeof(value)
			if t != TYPE_FLOAT and t != TYPE_INT:
				bad_type.append("%s=%s" % [key, value])
		if str(pose.get("msg", "")) != "pose":
			bad_type.append("msg=%s" % pose.get("msg"))
	_check("C3 pose-fields", window.size() > 0 and bad_keys.is_empty() and bad_type.is_empty(),
		"count=%d bad_keys=%s bad_type=%s first=%s" % [window.size(), bad_keys, bad_type,
			window[0] if window.size() > 0 else {}])

	# C4 / P3：静止位姿 == chassis_ready.spawn（坐标系与 yaw 约定的硬证据）
	var first: Dictionary = window[0] if window.size() > 0 else {}
	var c4_ok: bool = first.size() == 8 \
		and absf(float(first.get("x", -9.0)) - float(spawn.get("x", 0.0))) <= POSE_TOL \
		and absf(float(first.get("y", -9.0)) - float(spawn.get("y", 0.0))) <= POSE_TOL \
		and absf(float(first.get("z", -9.0)) - float(spawn.get("z", 0.0))) <= POSE_TOL_Z \
		and absf(float(first.get("yaw", -9.0)) - float(spawn.get("yaw", 0.0))) <= POSE_TOL \
		and absf(float(first.get("vx", -9.0))) <= POSE_TOL \
		and absf(float(first.get("vy", -9.0))) <= POSE_TOL
	_check("C4 pose-matches-spawn", c4_ok, "first=%s spawn=%s" % [first, spawn])

	# C5 / P4：yaw ∈ [-PI, PI)；world_time 逐条单调不降（容差 0）
	var yaw_bad: Array = []
	var time_bad: Array = []
	var prev_time := -1e18
	for item: Variant in window:
		var pose: Dictionary = item
		var yaw := float(pose.get("yaw", 9.0))
		if yaw < -PI or yaw >= PI:
			yaw_bad.append(yaw)
		var wt := float(pose.get("world_time", -1.0))
		if wt < prev_time:
			time_bad.append(wt)
		prev_time = wt
	_check("C5 pose-yaw-range", window.size() > 0 and yaw_bad.is_empty(),
		"bad=%s" % [yaw_bad])
	_check("C6 pose-time-monotonic", window.size() > 0 and time_bad.is_empty(),
		"bad=%s first=%s last=%s" % [time_bad,
			window[0].get("world_time") if window.size() > 0 else null,
			window[window.size() - 1].get("world_time") if window.size() > 0 else null])

	# C7：传感器连接就位 + assembly_ready
	var sensor: Variant = _new_sim(port)
	await _wait_open(sensor, 1500)
	sensor.Send({
		"msg": "sensor_hello",
		"peer_id": "simPose",
		"slot": "lidar_front",
		"sensor": "lidar_2d",
		"mount": {"x": 0.1, "y": 0.0, "z": 0.2, "yaw": 0.0},
		"params": {"range": 12.0, "rays": 360, "rate_hz": 10},
	})
	var sensor_ready: Dictionary = await _wait_msg(sensor, "sensor_ready", 1500)
	var assembly: Dictionary = await _wait_msg(chassis, "assembly_ready", 1500)
	_check("C7 sensor-assembly",
		not sensor_ready.is_empty() and str(sensor_ready.get("slot", "")) == "lidar_front"
		and int(sensor_ready.get("sensor_id", 0)) >= 1
		and not assembly.is_empty() and str(assembly.get("peer_id", "")) == "simPose",
		"sensor_ready=%s assembly=%s" % [sensor_ready, assembly])

	# C8 / P7：位姿只发底盘连接 -> 传感器连接一条都没有
	chassis.Drain("pose")
	sensor.Drain("pose")
	await _wait_frames(FRAMES_PER_SECOND)
	var chassis_window: Array = chassis.Drain("pose")
	var sensor_poses: int = sensor.Count("pose")
	_check("C8 pose-not-to-sensor",
		chassis_window.size() >= MIN_POSES_PER_SECOND and sensor_poses == 0,
		"chassis=%d sensor=%d sensor_inbox=%s" % [chassis_window.size(), sensor_poses,
			sensor.Names()])

	# C9 / P5：forward 100 -> x 单调增、vx > 0、vy ≈ 0
	chassis.Send({"msg": "motion_cmd", "seq": 1, "action": "forward", "speed": 100})
	await _wait_physics(2)                      # 让指令落到物理帧（避免窗口首帧是静止旧值）
	chassis.Drain("pose")
	await _wait_physics(90)
	var fwd: Array = chassis.Drain("pose")
	var x_drop: Array = []
	var x_first := 0.0
	var x_last := 0.0
	var vx_max := -1e18
	var vx_last := -1e18
	var vy_max := 0.0
	for i in range(fwd.size()):
		var pose: Dictionary = fwd[i]
		var x := float(pose.get("x", 0.0))
		var vx := float(pose.get("vx", 0.0))
		var vy := float(pose.get("vy", 0.0))
		if i == 0:
			x_first = x
		if i > 0 and x < float(fwd[i - 1].get("x", 0.0)) - POSE_TOL:
			x_drop.append(x)                 # x 逐条不降（容差 1e-6）
		x_last = x
		vx_max = maxf(vx_max, vx)
		vx_last = vx
		vy_max = maxf(vy_max, absf(vy))
	_check("C9 pose-forward",
		fwd.size() > 0 and x_drop.is_empty()
		and x_last - x_first > 0.1 and vx_max > 0.1 and vx_last > 0.5 and vy_max <= 1e-3,
		"samples=%d dx=%.4f vx_max=%.4f vx_last=%.4f |vy|max=%.6f drop=%s" % [
			fwd.size(), x_last - x_first, vx_max, vx_last, vy_max, x_drop])

	# C10 / P6：left 100 -> yaw 单调减、|Δyaw| < PI、移动样本 vy < 0（左转 => 世界 yaw 为负）
	chassis.Send({"msg": "motion_cmd", "seq": 2, "action": "left", "speed": 100})
	await _wait_physics(2)
	chassis.Drain("pose")
	await _wait_physics(90)
	var turn: Array = chassis.Drain("pose")
	var yaw_rise: Array = []
	var vy_bad: Array = []
	var yaw_first := 0.0
	var yaw_last := 0.0
	var moving := 0
	for i in range(turn.size()):
		var pose: Dictionary = turn[i]
		var yaw := float(pose.get("yaw", 0.0))
		var vx := float(pose.get("vx", 0.0))
		var vy := float(pose.get("vy", 0.0))
		if i == 0:
			yaw_first = yaw
		elif yaw > float(turn[i - 1].get("yaw", 0.0)) + POSE_TOL:
			yaw_rise.append(yaw)
		yaw_last = yaw
		if absf(vx) > 0.2:
			moving += 1
			if vy >= 0.0:
				vy_bad.append("vx=%.3f vy=%.3f yaw=%.3f" % [vx, vy, yaw])
	var dyaw := yaw_last - yaw_first
	_check("C10 pose-left",
		turn.size() > 0 and yaw_rise.is_empty() and dyaw < -0.3 and absf(dyaw) < PI
		and moving > 0 and vy_bad.is_empty(),
		"samples=%d dyaw=%.4f moving=%d yaw_rise=%s vy_bad=%s" % [
			turn.size(), dyaw, moving, yaw_rise, vy_bad])

	# C11 / P8：断开底盘 -> 车没了、连接表空了、再调一次不报错、pose 不再增长
	chassis.CloseClient()
	var drained_at: int = await _wait_links_drained(manager, 180)
	_check("C11 disconnect-cleanup",
		drained_at >= 0 and manager.GetVehicleCount() == 0,
		"frames=%d vehicles=%d links=%s" % [drained_at, manager.GetVehicleCount(),
			manager.GetLinkInfos()])
	await _wait_ms(300)                        # 把在途帧排空（否则"不再增长"会被迟到帧污染）
	chassis.Drain("pose")
	manager.call("_BroadcastPoses", 1.0)       # 无连接时空转（不得报错 / 不得崩）
	await _wait_frames(30)
	_check("C12 no-pose-after-disconnect",
		chassis.Count("pose") == 0 and manager._links.is_empty(),
		"poses=%d links=%s" % [chassis.Count("pose"), manager.GetLinkInfos()])

	# C13 / P9a：vehicle == null 的连接不调用 GetPose（替身放在它后面：能跑完 = 没被打断）
	var spy: CharacterBody3D = PoseSpy.new()
	var null_link: Variant = ComponentLink.new()
	null_link.role = &"chassis"
	null_link.handshaken = true
	null_link.closed = false
	null_link.vehicle = null
	var spy_link: Variant = ComponentLink.new()
	spy_link.role = &"chassis"
	spy_link.handshaken = true
	spy_link.closed = false
	spy_link.vehicle = spy
	var vehicles_before: int = manager.GetVehicleCount()
	manager._links.append(null_link)
	manager._links.append(spy_link)
	manager.call("_BroadcastPoses", 1.0)
	var calls_first: int = spy.calls
	_check("C13 null-vehicle-guard", calls_first == 1 and manager.GetVehicleCount() == vehicles_before,
		"spy_calls=%d vehicles=%d（0 表示循环被 null avatar 打断）" % [
			calls_first, manager.GetVehicleCount()])

	# C14 / P9b：每周期每连接恰好一次（非 OPEN 的 SendJson 返回 ERR_UNCONFIGURED 不打断循环）
	manager.call("_BroadcastPoses", 1.0)
	_check("C14 one-pose-per-period", spy.calls == 2, "spy_calls=%d" % [spy.calls])
	manager._links.erase(null_link)
	manager._links.erase(spy_link)
	spy.free()

	# ============================================================ 收尾（退出时无 ERROR）
	for sim: Variant in _sims:
		sim.CloseClient()
	await _wait_ms(150)
	manager.Stop()
	await process_frame
	world.queue_free()
	await process_frame
	await process_frame
	_finish()


# ================================================================ 测试装置

func _new_sim(port: int) -> Variant:
	var sim: Variant = Sim.new()
	_sims.append(sim)
	var err: Error = sim.Connect(port)
	if err != OK:
		push_warning("[check] sim 连接失败：%s" % error_string(err))
	return sim


func _pump() -> void:
	for sim: Variant in _sims:
		sim.Poll()


## 按**帧数**等（--fixed-fps 60 下 n 帧 = n/60 秒仿真时间，§7-⑤）。
func _wait_frames(frames: int) -> void:
	for _i in range(frames):
		_pump()
		await process_frame
	_pump()


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		_pump()
		await physics_frame
	_pump()


func _wait_open(sim: Variant, timeout_ms: int) -> bool:
	var deadline := Time.get_ticks_msec() + timeout_ms
	while Time.get_ticks_msec() < deadline:
		_pump()
		if sim.IsOpen():
			return true
		await process_frame
	_pump()
	return sim.IsOpen()


func _wait_msg(sim: Variant, msg_name: String, timeout_ms: int) -> Dictionary:
	var deadline := Time.get_ticks_msec() + timeout_ms
	while Time.get_ticks_msec() < deadline:
		_pump()
		if sim.Has(msg_name):
			return sim.Take(msg_name)
		await process_frame
	_pump()
	if sim.Has(msg_name):
		return sim.Take(msg_name)
	return {}


func _wait_ms(ms: int) -> void:
	var deadline := Time.get_ticks_msec() + ms
	while Time.get_ticks_msec() < deadline:
		_pump()
		await process_frame
	_pump()


## 等"车没了 + 连接表空了"（墙钟无关，最多 max_frames 帧）；返回用掉的帧数，超时 -1。
func _wait_links_drained(mgr: Node, max_frames: int) -> int:
	var used := 0
	while used < max_frames:
		_pump()
		if mgr.GetVehicleCount() == 0 and mgr._links.is_empty():
			return used
		await process_frame
		used += 1
	_pump()
	return -1


## 去掉以 # 开头的整行注释，只留可执行代码（白盒 lint 用，同 check_diff_drive.gd）。
func _strip_comment_lines(source: String) -> String:
	var code := ""
	for raw_line: String in source.split("\n"):
		if raw_line.strip_edges().begins_with("#"):
			continue
		code += raw_line + "\n"
	return code


func _raw_source_of(path: String) -> String:
	var script: Script = load(path) as Script
	if script == null:
		return ""
	return script.source_code


func _code_of(path: String) -> String:
	return _strip_comment_lines(_raw_source_of(path))


## 截取 "func <name>(" 到下一个顶格 "func " 之间的函数体。
func _func_body(code: String, name: String) -> String:
	var marker := "func %s(" % name
	var start := code.find(marker)
	if start < 0:
		return ""
	var next := code.find("\nfunc ", start + marker.length())
	if next < 0:
		return code.substr(start)
	return code.substr(start, next - start)


# ================================================================ 断言与收尾

func _check(label: String, ok: bool, detail: String = "") -> bool:
	_total += 1
	if ok:
		print("[check] ", label, " ok", (" " + detail) if not detail.is_empty() else "")
		return true
	_failed += 1
	print("[check] ", label, " FAIL", (" " + detail) if not detail.is_empty() else "")
	return false


## 结构性前置条件失败：计数 + 中止（成功路径不占断言编号，保证 total 稳定）。
func _fatal(label: String, detail: String) -> void:
	_total += 1
	_failed += 1
	_harness_errors += 1
	print("[check] ", label, " FAIL ", detail)


func _finish() -> void:
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
