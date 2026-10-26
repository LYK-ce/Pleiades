# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-25
#
# WebSocketManager 自检：场景形态 / World.tscn 接线 / 底盘+传感器装配 / 运动下行 /
# 传感器先于底盘 / 空 manifest / 断开清理 / 错误路径 / camera 装配与出帧闸门 / 规范 lint。
# 用法（两档都要绿，COMPONENTS.md §11 Step 6）：
#   timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_ws_manager.gd
#   DISPLAY=:99 timeout 400 godot --path orion-simulation-world --fixed-fps 60 \
#     --rendering-driver opengl3 --audio-driver Dummy \
#     --script res://src/tests/check_ws_manager.gd
# 期望：failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR/SCRIPT ERROR（COMPONENTS.md §11 Step 6）。
#   两档 total 差 1（76 / 77）：headless = dummy 渲染器，J7 必然 [skip]。
#
# task_34 Step 3 追加：A9（`type=2` 帧格式）/ A10（连接缓冲 4 MiB）/
#   J1~J6（`sensor="camera"` 真装配 + 出帧闸门）/ [skip] J7（帧内容，无渲染器时不计入 total）。
#
# task_35 Step 2：A11（kind 规范化 + `motion_cmd{v,dir}`）/ K1~K13（行人装配、超时停车、spawn 校验）。
# task_35 Step 2 第 2 轮：H5/K4/K5/K6 的等待窗口改用引擎帧（Xvfb 下 ~10~14 fps 也必须绿）；
#   K14~K16 锁跨 kind 清场（robot->pedestrian / pedestrian->robot / 同一连接重建）。
#
# 关键手法：**同一个进程里既当世界、又当 sim**。真 World.tscn + 真 WebSocketPeer 客户端
#   （P1/P6 实测 --headless 下 TCPServer + accept_stream + connect_to_url 完整可用），
#   所以"一个 sim 连上并完成装配、收到 assembly_ready"是**真 WS、真场景、真物理**跑出来的，
#   不是直接调私有函数。
#
# 决定本文件写法的实测事实：
#   P3  WebSocketPeer.close() 返回 void（不取值）。
#   P4  CONNECTING 状态下 close() 有效（握手超时用例依赖它）。
#   P5  TCPServer.listen(0) 取 OS 临时端口 -> 自检不抢 9999（本机 9999 常被代理工具占用）。
#   P7  node_paths 的 NodePath **相对该节点自身**（B2 直接断言 vehicles_root == World/Vehicles）。
#   P16 一次 null 解引用会让协程中断、quit() 不执行 -> 每条断言先判空，所有等待都带超时。
#   E21 JSON.parse_string() 非法输入会向 stderr 打 ERROR -> 协议层改用 JSON.new().parse()。
#   频率类等待一律用**引擎帧数**（`_wait_frames`）而不是墙钟毫秒：`--fixed-fps 60` 只固定 delta，
#   不限制实际帧率，墙钟在引擎时间里不可控（task_34 风险 R4）。
#
# 白盒断言（manager._manifests 等）在本仓库是既有做法（check_diff_drive.gd 的 A35 lint 同例）。

extends SceneTree

const WireProtocol := preload("res://src/net/WireProtocol.gd")
const ComponentLink := preload("res://src/net/ComponentLink.gd")
const WorldTransform := preload("res://src/common/WorldTransform.gd")

const WS_MANAGER_SCENE := "res://src/scenes/WebSocketManager.tscn"
const WS_MANAGER_SCRIPT := "res://src/net/WebSocketManager.gd"
const WIRE_SCRIPT := "res://src/net/WireProtocol.gd"
const LINK_SCRIPT := "res://src/net/ComponentLink.gd"
const WORLD_SCENE := "res://src/scenes/World.tscn"
const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const LIDAR_SCENE := "res://src/scenes/sensors/Lidar2D.tscn"
const CAMERA_SCRIPT := "res://src/sensors/CameraSensor.gd"

const NET_SCRIPTS := [WIRE_SCRIPT, LINK_SCRIPT, WS_MANAGER_SCRIPT]
const HEADER_FILES := [WIRE_SCRIPT, LINK_SCRIPT, WS_MANAGER_SCRIPT, WS_MANAGER_SCENE]

const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)
const SPAWN_TOL := 0.001
## LiDAR 一圈 360 线：15 + 4*360 = 1455 字节（§8.2）。
const LIDAR_RAYS := 360
const LIDAR_FRAME_BYTES := 1455

## camera 装配（task_34 Step 3）：目录默认 640×480@1 Hz，用 `sensor_hello.params` 显式下发。
const CAMERA_SLOT := "camera_nose"
const CAMERA_WANT_SIZE := Vector2i(640, 480)
const CAMERA_MOUNT := {"x": 0.15, "y": 0.0, "z": 0.12, "yaw": 0.0, "pitch": 0.0}
const CAMERA_PARAMS := {"width": 640, "height": 480, "fov_deg": 70.0,
	"quality": 0.8, "rate_hz": 1.0}
## 90 个引擎帧 = 引擎时间 1.5 s（1 Hz 下至少到点 1 次）。
const CAMERA_GATE_FRAMES := 90

## task_35 Step 2：行人（kind="pedestrian"）+ v/dir + 300 ms 超时
const PEDESTRIAN_SCRIPT := "res://src/chassis/Pedestrian.gd"
const PED_ID := "human-01"
const PED_SPAWN := {"x": 70.0, "y": 70.0, "z": 0.0, "yaw": 0.0}
const PED_SPAWN_TOL := 0.05
const PED_CONVERGE_FRAMES := 60      # 1 s：180 °/s 转 90° 只需 0.5 s
const PED_IDLE_WAIT_MS := 600        # > 300 ms，留一倍余量（墙钟）
## 默认出生点（非法 / 缺失 spawn 的回退点）= World.SPAWN_WORLD + PEDESTRIAN_SPAWN_OFFSET_M。
const PED_DEFAULT_X := 64.0
const PED_DEFAULT_Y := 56.0

var _total: int = 0
var _failed: int = 0
var _skipped: int = 0
var _harness_errors: int = 0
var _sims: Array = []
## 移除事件流水（K14~K16 用 slice(n) 只看本用例新增的部分）。
var _removed_events: Array = []


# ================================================================ 测试替身：sim 侧 WS 客户端

class Sim:
	var peer: WebSocketPeer = null
	## 已解析的文本帧（FIFO）
	var inbox: Array = []
	## 收到的二进制帧计数（断言"下行没有乱发"）
	var binary_count: int = 0
	## 按到达顺序存下的二进制帧（只用于 camera 段：断言 type=2 帧真的到了 sim 侧）
	var binaries: Array = []
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
					binaries.append(bytes)
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

	func SendRaw(text: String) -> Error:
		if not IsOpen():
			return ERR_UNCONFIGURED
		return peer.send_text(text)

	## 客户端主动断开（模拟 sim 进程退出）。
	func CloseClient() -> void:
		if peer != null and not closed:
			peer.close(1000, "test bye")

	func Has(msg_name: String) -> bool:
		for msg: Variant in inbox:
			var dict: Dictionary = msg
			if str(dict.get("msg", "")) == msg_name:
				return true
		return false

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

	## 数 inbox 里某 msg 的条数（task_35 Step 2：K3 数 `pose` 条数用）。
	## 只读，不改 Connect/Poll/Send/Take/Has/Names 的任何行为。
	func Count(msg_name: String) -> int:
		var n: int = 0
		for msg: Variant in inbox:
			var dict: Dictionary = msg
			if str(dict.get("msg", "")) == msg_name:
				n += 1
		return n


# ================================================================ 入口

func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ A. 树外：组件 / 协议形态
	var manager_packed: PackedScene = load(WS_MANAGER_SCENE) as PackedScene
	_check("A1 scene-loads", manager_packed != null, WS_MANAGER_SCENE)
	if manager_packed == null:
		_finish()
		return
	var probe: Node = manager_packed.instantiate()
	var probe_script: Script = probe.get_script()
	_check("A2 root-node", probe.name == "WebSocketManager" and probe is Node
		and not (probe is Node3D) and probe_script != null
		and probe_script.resource_path == WS_MANAGER_SCRIPT,
		"name=%s class=%s script=%s" % [probe.name, probe.get_class(),
			probe_script.resource_path if probe_script != null else "null"])

	var heartbeat: Timer = probe.get_node_or_null("Heartbeat") as Timer
	_check("A3 heartbeat-timer", heartbeat != null and is_equal_approx(heartbeat.wait_time, 1.0)
		and heartbeat.autostart,
		"Heartbeat=%s wait=%.2f autostart=%s" % [heartbeat,
			heartbeat.wait_time if heartbeat != null else -1.0,
			heartbeat.autostart if heartbeat != null else false])

	var required := ["Listen", "Stop", "GetListenPort", "IsListening", "GetVehicle",
		"GetVehicleCount", "GetLinkInfos", "GetStats"]
	var missing: Array = []
	for method_name: String in required:
		if not probe.has_method(method_name):
			missing.append(method_name)
	_check("A4 contract-methods", missing.is_empty(),
		"missing=%s" % [missing])

	_check("A5 wire-constants",
		WireProtocol.MSG_CHASSIS_HELLO == "chassis_hello"
		and WireProtocol.MSG_CHASSIS_READY == "chassis_ready"
		and WireProtocol.MSG_SENSOR_HELLO == "sensor_hello"
		and WireProtocol.MSG_SENSOR_READY == "sensor_ready"
		and WireProtocol.MSG_MOTION_CMD == "motion_cmd"
		and WireProtocol.MSG_ASSEMBLY_READY == "assembly_ready"
		and WireProtocol.FRAME_TYPE_LIDAR == 1
		and WireProtocol.FRAME_TYPE_CAMERA == 2
		and WireProtocol.FRAME_HEADER_SIZE == 15,
		"hello=%s ready=%s assembly=%s frame=%d/%d/%d" % [
			WireProtocol.MSG_CHASSIS_HELLO, WireProtocol.MSG_SENSOR_READY,
			WireProtocol.MSG_ASSEMBLY_READY, WireProtocol.FRAME_TYPE_LIDAR,
			WireProtocol.FRAME_TYPE_CAMERA, WireProtocol.FRAME_HEADER_SIZE])

	var sample := {"msg": "motion_cmd", "seq": 3, "peer_id": "simA", "action": "forward"}
	var round_trip: Dictionary = WireProtocol.ParseJson(WireProtocol.MakeJson(sample))
	var bad_json: Dictionary = WireProtocol.ParseJson("{oops".to_utf8_buffer())
	var bad_array: Dictionary = WireProtocol.ParseJson("[1,2]".to_utf8_buffer())
	_check("A6 json-roundtrip",
		str(round_trip.get("msg", "")) == "motion_cmd"
		and str(round_trip.get("peer_id", "")) == "simA"
		and int(round_trip.get("seq", -1)) == 3
		and bad_json.is_empty() and bad_array.is_empty(),
		"round_trip=%s bad_json=%s bad_array=%s" % [round_trip, bad_json, bad_array])

	var ranges := PackedFloat32Array()
	ranges.resize(LIDAR_RAYS)
	for i in range(LIDAR_RAYS):
		ranges[i] = float(i) * 0.5
	var frame: PackedByteArray = WireProtocol.BuildLidar2DFrame(3, 9, 1.5, ranges)
	var header: Dictionary = WireProtocol.ParseFrameHeader(frame)
	var short_header: Dictionary = WireProtocol.ParseFrameHeader(frame.slice(0, 14))
	_check("A7 lidar-frame",
		frame.size() == LIDAR_FRAME_BYTES
		and int(header.get("type", -1)) == WireProtocol.FRAME_TYPE_LIDAR
		and int(header.get("slot_id", -1)) == 3
		and int(header.get("seq", -1)) == 9
		and absf(float(header.get("world_time", 0.0)) - 1.5) < 1e-9
		and absf(frame.decode_float(WireProtocol.FRAME_HEADER_SIZE) - 0.0) < 1e-6
		and absf(frame.decode_float(WireProtocol.FRAME_HEADER_SIZE + 4) - 0.5) < 1e-6
		and absf(frame.decode_float(WireProtocol.FRAME_HEADER_SIZE + 8) - 1.0) < 1e-6
		and short_header.is_empty(),
		"size=%d header=%s short=%s" % [frame.size(), header, short_header])

	var bare_link: Variant = ComponentLink.new()
	var send_err: Error = bare_link.SendJson({"msg": "x"})
	var binary_err: Error = bare_link.SendBinary(PackedByteArray())
	_check("A8 link-defaults",
		bare_link.conn_id == 0 and bare_link.peer_id == "" and bare_link.role == &""
		and bare_link.slot == &"" and bare_link.sensor_id == 0 and not bare_link.handshaken
		and not bare_link.closed and not bare_link.IsOpen() and not bare_link.IsConnecting()
		and bare_link.NextSeq() == 1 and bare_link.NextSeq() == 2 and bare_link.NextSeq() == 3
		and send_err == ERR_UNCONFIGURED and binary_err == ERR_UNCONFIGURED,
		"send=%d binary=%d seq=%d" % [send_err, binary_err, bare_link.out_seq])
	bare_link = null

	# ---- A9（task_34 Step 3）：camera 帧格式 = 15 B 头 + **JPEG 原样 payload**
	var camera_payload := PackedByteArray([0xFF, 0xD8, 0xFF, 0xE0, 0x42])
	var camera_frame: PackedByteArray = WireProtocol.BuildCameraFrame(4, 11, 3.25, camera_payload)
	var camera_header: Dictionary = WireProtocol.ParseFrameHeader(camera_frame)
	var camera_tail: PackedByteArray = camera_frame.slice(WireProtocol.FRAME_HEADER_SIZE)
	var empty_frame: PackedByteArray = WireProtocol.BuildCameraFrame(4, 11, 3.25, PackedByteArray())
	_check("A9 camera-frame",
		camera_frame.size() == WireProtocol.FRAME_HEADER_SIZE + camera_payload.size()
		and int(camera_header.get("type", -1)) == WireProtocol.FRAME_TYPE_CAMERA
		and int(camera_header.get("slot_id", -1)) == 4
		and int(camera_header.get("seq", -1)) == 11
		and absf(float(camera_header.get("world_time", 0.0)) - 3.25) < 1e-9
		and camera_tail == camera_payload
		and empty_frame.size() == WireProtocol.FRAME_HEADER_SIZE,
		"size=%d type=%d slot=%d seq=%d empty=%d" % [
			camera_frame.size(), int(camera_header.get("type", -1)),
			int(camera_header.get("slot_id", -1)), int(camera_header.get("seq", -1)),
			empty_frame.size()])

	# ---- A10（task_34 Step 3）：连接缓冲提到 4 MiB（默认 64 KiB 撑不住一张 720p JPEG）
	var probe_link: Variant = ComponentLink.new()
	probe_link.Init(null, 99)          # null tcp 分支在 accept_stream 之前 return，缓冲已设好
	var buffer_expect: int = 4 * 1024 * 1024
	_check("A10 link-buffer-size",
		ComponentLink.MAX_FRAME_BUFFER_BYTES == buffer_expect
		and probe_link.peer != null
		and probe_link.peer.inbound_buffer_size == buffer_expect
		and probe_link.peer.outbound_buffer_size == buffer_expect
		and probe_link.closed,
		"in=%d out=%d closed=%s" % [
			probe_link.peer.inbound_buffer_size if probe_link.peer != null else -1,
			probe_link.peer.outbound_buffer_size if probe_link.peer != null else -1,
			str(probe_link.closed)])
	probe_link.peer = null
	probe_link = null

	# ---- A11（task_35 Step 2）：kind 规范化 + motion_cmd{v, dir} 编解码形态
	var motion_round: Dictionary = WireProtocol.ParseJson(
		WireProtocol.MakeJson(WireProtocol.BuildMotionCommand(1.0, PI / 2.0, 7)))
	var stop_round: Dictionary = WireProtocol.ParseJson(
		WireProtocol.MakeJson(WireProtocol.BuildMotionCommand(0.0)))
	_check("A11 wire-pedestrian",
		WireProtocol.KIND_ROBOT == "robot"
		and WireProtocol.KIND_PEDESTRIAN == "pedestrian"
		and WireProtocol.NormalizeKind(" PEDESTRIAN ") == "pedestrian"
		and WireProtocol.NormalizeKind("robot") == "robot"
		and WireProtocol.NormalizeKind("quadcopter").is_empty()
		and WireProtocol.NormalizeKind(3).is_empty()
		and WireProtocol.NormalizeKind(null).is_empty()
		and str(motion_round.get("msg", "")) == WireProtocol.MSG_MOTION_CMD
		and int(motion_round.get("seq", -1)) == 7
		and absf(float(motion_round.get("v", -1.0)) - 1.0) < 1e-9
		and absf(float(motion_round.get("dir", -9.0)) - PI / 2.0) < 1e-9
		and not motion_round.has("action") and not motion_round.has("speed")
		and absf(float(stop_round.get("v", -1.0))) < 1e-9
		and not stop_round.has("dir"),
		"motion=%s stop=%s" % [motion_round, stop_round])
	probe.free()

	# ============================================================ B. World.tscn 接线
	var world_packed: PackedScene = load(WORLD_SCENE) as PackedScene
	if world_packed == null:
		_fatal("B0 world-loads", WORLD_SCENE)
		_finish()
		return
	var world: Node3D = world_packed.instantiate() as Node3D
	var manager: Node = world.get_node_or_null("Systems/WebSocketManager")
	if manager == null:
		_fatal("B0 manager-instanced", "World/Systems/WebSocketManager")
		world.free()
		_finish()
		return
	# 取脚本默认值（此时还没 add_child，_ready 未跑）
	var default_autolisten: bool = manager.autolisten
	var default_listen_port: int = manager.listen_port
	manager.autolisten = false          # 自检自带 Listen(0)，不抢 9999
	root.add_child(world)
	await process_frame
	# 第 2 轮：移除事件用**信号**记（日志是给人看的，断言要可数）。
	# `_RemoveAvatar()` 现在只在真摘到东西时才 emit —— 换 kind 的"幽灵移除"会多出一条，K14~K16 直接红。
	manager.vehicle_removed.connect(_On_Vehicle_Removed)
	manager.pedestrian_removed.connect(_On_Pedestrian_Removed)

	var manager_script: Script = manager.get_script()
	var vehicles_root: Node3D = world.get_node_or_null("Vehicles") as Node3D
	_check("B1 manager-node", manager_script != null
		and manager_script.resource_path == WS_MANAGER_SCRIPT and world.has_node("Systems"),
		"script=%s systems=%s" % [
			manager_script.resource_path if manager_script != null else "null",
			world.has_node("Systems")])
	# E20 的直接红灯：NodePath 写错会静默 null，静态兜底不算通过
	_check("B2 vehicles-root-bound", manager.vehicles_root != null
		and manager.vehicles_root == vehicles_root,
		"vehicles_root=%s world/Vehicles=%s" % [manager.vehicles_root, vehicles_root])
	_check("B3 export-defaults", manager.vehicle_scene != null
		and manager.vehicle_scene.resource_path == VEHICLE_SCENE
		and default_autolisten and default_listen_port == 0,
		"scene=%s autolisten=%s listen_port=%d" % [
			manager.vehicle_scene.resource_path if manager.vehicle_scene != null else "null",
			default_autolisten, default_listen_port])

	var listen_err: Error = manager.Listen(0)
	var port: int = manager.GetListenPort()
	_check("B4 ephemeral-listen", listen_err == OK and port > 0
		and manager.GetStats().get("listening", false) == true,
		"err=%d port=%d stats=%s" % [listen_err, port, manager.GetStats()])

	# spawn 规则（白盒）：第 1 台 UGV = World 出生原点；UAV 抬到 2.5 m（§7.1 / task_32 Step 3）
	var spawn_ugv: Dictionary = manager.call("_PickSpawn", "diff_drive_ugv")
	var spawn_uav: Dictionary = manager.call("_PickSpawn", "quadrotor_uav")
	_check("B5 spawn-rule",
		absf(float(spawn_ugv.get("x", -1.0)) - SPAWN_WORLD.x) < SPAWN_TOL
		and absf(float(spawn_ugv.get("y", -1.0)) - SPAWN_WORLD.y) < SPAWN_TOL
		and absf(float(spawn_ugv.get("z", -1.0))) < SPAWN_TOL
		and absf(float(spawn_uav.get("z", -1.0)) - 2.5) < SPAWN_TOL,
		"ugv=%s uav=%s" % [spawn_ugv, spawn_uav])

	# ============================================================ C. 底盘 + 传感器按序装配（peer A）
	var sim_a: Variant = _new_sim(port)
	var a_open: bool = await _wait_open(sim_a, 1500)
	_check("C1 chassis-ready", a_open, "sim A open=%s" % a_open)
	sim_a.Send({
		"msg": "chassis_hello",
		"peer_id": "simA",
		"chassis": "diff_drive_ugv",
		"params": {"max_speed": 1.5},
		"manifest": [{"slot": "lidar_front", "sensor": "lidar_2d"}],
	})
	var ready: Dictionary = await _wait_msg(sim_a, "chassis_ready", 1500)
	_check("C1b chassis-ready-received", not ready.is_empty(), "inbox=%s" % [sim_a.Names()])
	_check("C2 chassis-ready-fields",
		str(ready.get("peer_id", "")) == "simA" and int(ready.get("avatar_id", -1)) == 1,
		"ready=%s" % [ready])

	var spawn: Dictionary = ready.get("spawn", {})
	_check("C3 spawn-payload",
		absf(float(spawn.get("x", -1.0)) - SPAWN_WORLD.x) < SPAWN_TOL
		and absf(float(spawn.get("y", -1.0)) - SPAWN_WORLD.y) < SPAWN_TOL
		and absf(float(spawn.get("z", -1.0))) < SPAWN_TOL
		and absf(float(spawn.get("yaw", -1.0))) < SPAWN_TOL,
		"spawn=%s" % [spawn])
	_check("C4 chassis-ready-time",
		float(ready.get("world_time", -1.0)) >= 0.0, "world_time=%s" % ready.get("world_time"))

	var vehicle_a: Node3D = manager.GetVehicle("simA")
	var node_a: Node = vehicles_root.get_node_or_null(NodePath("simA")) if vehicles_root != null else null
	_check("C5 vehicle-in-world", vehicle_a != null and node_a == vehicle_a
		and manager.GetVehicleCount() == 1,
		"GetVehicle=%s node=%s count=%d" % [vehicle_a, node_a, manager.GetVehicleCount()])
	_check("C6 chassis-attached",
		vehicle_a != null and vehicle_a.HasChassis()
		and vehicle_a.GetChassisName() == "diff_drive_ugv"
		and vehicle_a.GetChassisParams().has("max_speed"),
		"has=%s name=%s params=%s" % [
			vehicle_a.HasChassis() if vehicle_a != null else false,
			vehicle_a.GetChassisName() if vehicle_a != null else "null",
			vehicle_a.GetChassisParams() if vehicle_a != null else {}])

	var pose_a: Dictionary = vehicle_a.GetPose() if vehicle_a != null else {}
	_check("C7 vehicle-pose",
		absf(float(pose_a.get("x", -1.0)) - SPAWN_WORLD.x) < SPAWN_TOL
		and absf(float(pose_a.get("y", -1.0)) - SPAWN_WORLD.y) < SPAWN_TOL
		and absf(float(pose_a.get("z", 99.0))) < 0.05
		and absf(float(pose_a.get("yaw", 9.0))) < SPAWN_TOL,
		"pose=%s" % [pose_a])

	_check("C8 not-assembled-yet", await _wait_absent(sim_a, "assembly_ready", 500),
		"manifest 未收齐时不应有 assembly_ready")

	var sim_a2: Variant = _new_sim(port)
	await _wait_open(sim_a2, 1500)
	sim_a2.Send({
		"msg": "sensor_hello",
		"peer_id": "simA",
		"slot": "lidar_front",
		"sensor": "lidar_2d",
		"mount": {"x": 0.1, "y": 0.0, "z": 0.2, "yaw": 0.0},
		"params": {"range": 12.0, "rays": 360, "rate_hz": 10},
	})
	var sensor_ready: Dictionary = await _wait_msg(sim_a2, "sensor_ready", 1500)
	_check("C9 sensor-ready", not sensor_ready.is_empty(), "inbox=%s" % [sim_a2.Names()])
	var sensor_id := int(sensor_ready.get("sensor_id", 0))
	_check("C10 sensor-ready-fields",
		str(sensor_ready.get("slot", "")) == "lidar_front" and sensor_id >= 1,
		"sensor_ready=%s" % [sensor_ready])

	var assembly: Dictionary = await _wait_msg(sim_a, "assembly_ready", 1500)
	_check("C11 assembly-ready",
		not assembly.is_empty() and str(assembly.get("peer_id", "")) == "simA"
		and float(assembly.get("world_time", -1.0)) >= 0.0,
		"assembly=%s" % [assembly])
	var assembled_once: bool = await _wait_absent(sim_a, "assembly_ready", 300)
	_check("C12 assembly-once",
		assembled_once and int(manager.GetStats().get("assembled", -1)) == 1,
		"no_duplicate=%s stats=%s" % [assembled_once, manager.GetStats()])

	var infos: Array = manager.GetLinkInfos()
	var chassis_info: Dictionary = {}
	var sensor_info: Dictionary = {}
	for info: Variant in infos:
		var row: Dictionary = info
		if str(row.get("peer_id", "")) == "simA" and StringName(row.get("role", &"")) == &"chassis":
			chassis_info = row
		elif str(row.get("peer_id", "")) == "simA" and StringName(row.get("role", &"")) == &"sensor":
			sensor_info = row
	_check("C13 link-table",
		infos.size() == 2
		and not chassis_info.is_empty() and bool(chassis_info.get("open", false))
		and bool(chassis_info.get("handshaken", false))
		and int(chassis_info.get("sensor_id", -1)) == 0
		and not sensor_info.is_empty() and bool(sensor_info.get("open", false))
		and bool(sensor_info.get("handshaken", false))
		and str(sensor_info.get("slot", "")) == "lidar_front"
		and int(sensor_info.get("sensor_id", -1)) == sensor_id,
		"infos=%s" % [infos])

	# C14 跨 Step 不回归：Lidar2D.tscn 存在 <=> 几何已附着（Step 8 前应双双为假）
	var lidar_exists := ResourceLoader.exists(LIDAR_SCENE)
	var lidar_attached: bool = vehicle_a != null and vehicle_a.GetSensor("lidar_front") != null
	_check("C14 sensor-geometry-consistency", lidar_exists == lidar_attached,
		"Lidar2D.tscn=%s attached=%s" % [lidar_exists, lidar_attached])

	# ============================================================ D. 运动指令下行
	sim_a.Send({"msg": "motion_cmd", "seq": 1, "action": "forward", "speed": 100})
	await _wait_physics(30)
	var pose_moved: Dictionary = vehicle_a.GetPose() if vehicle_a != null else {}
	_check("D1 motion-forward", float(pose_moved.get("x", 0.0)) > SPAWN_WORLD.x + 0.05,
		"pose=%s" % [pose_moved])
	sim_a.Send({"msg": "motion_cmd", "seq": 2, "action": "fly", "speed": 100})
	await _wait_physics(10)
	_check("D2 unknown-action-safe",
		sim_a.IsOpen() and vehicle_a != null and is_instance_valid(vehicle_a)
		and vehicle_a.GetPose().has("x"),
		"open=%s vehicle=%s" % [sim_a.IsOpen(), vehicle_a])
	sim_a.Send({"msg": "motion_cmd", "seq": 3, "action": "stop", "speed": 0})

	# ============================================================ E. 传感器先于底盘（peer B）
	var sim_b2: Variant = _new_sim(port)
	await _wait_open(sim_b2, 1500)
	sim_b2.Send({
		"msg": "sensor_hello",
		"peer_id": "simB",
		"slot": "lidar_front",
		"sensor": "lidar_2d",
		"mount": {"x": 0.0, "y": 0.0, "z": 0.2},
		"params": {"range": 12.0, "rays": 360, "rate_hz": 10},
	})
	_check("E1 sensor-parked", await _wait_absent(sim_b2, "sensor_ready", 400),
		"底盘未到前不应回 sensor_ready")
	_check("E2 pending-count", int(manager.GetStats().get("pending", -1)) == 1,
		"stats=%s" % [manager.GetStats()])

	var sim_b: Variant = _new_sim(port)
	await _wait_open(sim_b, 1500)
	sim_b.Send({
		"msg": "chassis_hello",
		"peer_id": "simB",
		"chassis": "diff_drive_ugv",
		"params": {},
		"manifest": [{"slot": "lidar_front", "sensor": "lidar_2d"}],
	})
	var ready_b: Dictionary = await _wait_msg(sim_b, "chassis_ready", 1500)
	_check("E3 chassis-ready-B", not ready_b.is_empty(), "inbox=%s" % [sim_b.Names()])
	var replay_ready: Dictionary = await _wait_msg(sim_b2, "sensor_ready", 1500)
	var assembly_b: Dictionary = await _wait_msg(sim_b, "assembly_ready", 1500)
	_check("E4 replay-then-assembly",
		not replay_ready.is_empty() and str(replay_ready.get("slot", "")) == "lidar_front"
		and not assembly_b.is_empty(),
		"replay=%s assembly=%s" % [replay_ready, assembly_b])

	var vehicle_b: Node3D = manager.GetVehicle("simB")
	var pose_b: Dictionary = vehicle_b.GetPose() if vehicle_b != null else {}
	_check("E5 two-vehicles-distinct",
		manager.GetVehicleCount() == 2 and vehicle_b != null
		and vehicle_b.name != vehicle_a.name
		and not is_equal_approx(float(pose_b.get("x", 0.0)), float(pose_moved.get("x", 0.0))),
		"count=%d name_b=%s pose_b=%s" % [manager.GetVehicleCount(),
			vehicle_b.name if vehicle_b != null else "null", pose_b])

	# ============================================================ F. 空 manifest
	var sim_c: Variant = _new_sim(port)
	await _wait_open(sim_c, 1500)
	sim_c.Send({
		"msg": "chassis_hello",
		"peer_id": "simC",
		"chassis": "diff_drive_ugv",
		"params": {},
		"manifest": [],
	})
	var ready_c: Dictionary = await _wait_msg(sim_c, "chassis_ready", 1500)
	var assembly_c: Dictionary = await _wait_msg(sim_c, "assembly_ready", 1500)
	_check("F1 empty-manifest-assembled",
		not ready_c.is_empty() and not assembly_c.is_empty()
		and int(manager.GetStats().get("assembled", -1)) == 3,
		"ready=%s assembly=%s stats=%s" % [ready_c, assembly_c, manager.GetStats()])

	# ============================================================ G. 断开清理
	sim_b2.CloseClient()
	await _wait_ms(500)
	var manifest_b: Dictionary = manager._manifests.get("simB", {})
	_check("G1 sensor-detach",
		vehicle_b != null and is_instance_valid(vehicle_b)
		and vehicle_b.GetSensor("lidar_front") == null
		and manifest_b.has("lidar_front") and bool(manifest_b["lidar_front"]) == false,
		"vehicle=%s manifest=%s" % [vehicle_b, manifest_b])

	# 重新挂一条传感器连接（为 G2/G3 的"服务端关兄弟连接"准备真实连接）
	var sim_b2b: Variant = _new_sim(port)
	await _wait_open(sim_b2b, 1500)
	sim_b2b.Send({
		"msg": "sensor_hello",
		"peer_id": "simB",
		"slot": "lidar_front",
		"sensor": "lidar_2d",
		"mount": {},
		"params": {},
	})
	await _wait_msg(sim_b2b, "sensor_ready", 1500)

	sim_b.CloseClient()
	await _wait_ms(500)
	var node_b: Node = vehicles_root.get_node_or_null(NodePath("simB")) if vehicles_root != null else null
	_check("G2 vehicle-removed",
		manager.GetVehicle("simB") == null and node_b == null
		and manager.GetVehicleCount() == 2,
		"GetVehicle=%s node=%s count=%d" % [manager.GetVehicle("simB"), node_b,
			manager.GetVehicleCount()])
	_check("G3 sibling-closed-by-server", sim_b2b.closed,
		"closed=%s code=%d" % [sim_b2b.closed, sim_b2b.close_code])
	_check("G4 others-unaffected",
		sim_a.IsOpen() and sim_a2.IsOpen()
		and manager.GetVehicle("simA") == vehicle_a and manager.GetVehicle("simC") != null,
		"a=%s a2=%s vehicle_a=%s vehicle_c=%s" % [sim_a.IsOpen(), sim_a2.IsOpen(),
			manager.GetVehicle("simA"), manager.GetVehicle("simC")])
	_check("G5 link-count", int(manager.GetStats().get("links", -1)) == 3,
		"stats=%s infos=%s" % [manager.GetStats(), manager.GetLinkInfos()])

	# ============================================================ H. 错误路径
	var sim_h1: Variant = _new_sim(port)
	await _wait_open(sim_h1, 1500)
	sim_h1.Send({"msg": "motion_cmd", "seq": 1, "action": "forward", "speed": 10})
	var h1_closed: bool = await _wait_closed(sim_h1, 1200)
	_check("H1 first-packet-not-hello", h1_closed
		and (sim_h1.close_code == 1002 or sim_h1.close_code == -1 or sim_h1.close_code == 0),
		"closed=%s code=%d" % [h1_closed, sim_h1.close_code])

	var sim_h2: Variant = _new_sim(port)
	await _wait_open(sim_h2, 1500)
	sim_h2.SendRaw("{oops")
	var h2_closed: bool = await _wait_closed(sim_h2, 1200)
	_check("H2 bad-json", h2_closed, "closed=%s code=%d" % [h2_closed, sim_h2.close_code])

	var count_before: int = manager.GetVehicleCount()
	var sim_h3: Variant = _new_sim(port)
	await _wait_open(sim_h3, 1500)
	sim_h3.Send({
		"msg": "chassis_hello",
		"peer_id": "simH3",
		"chassis": "no_such_chassis",
		"params": {},
		"manifest": [],
	})
	var h3_closed: bool = await _wait_closed(sim_h3, 1200)
	_check("H3 unknown-chassis",
		h3_closed and manager.GetVehicleCount() == count_before,
		"closed=%s count=%d (was %d)" % [h3_closed, manager.GetVehicleCount(), count_before])

	var sim_h4: Variant = _new_sim(port)
	await _wait_open(sim_h4, 1500)
	sim_h4.Send({
		"msg": "sensor_hello",
		"peer_id": "simA",
		"slot": "lidar_ghost",
		"sensor": "no_such_sensor",
		"mount": {},
		"params": {},
	})
	# H4 跨 Step 容忍：SensorCatalog 落地后「未知 sensor」必须被忽略（无 sensor_ready）；
	# Step 6 阶段目录还不存在，没有可校验的名字表 -> 只登记连接，但**绝不能**因此算就绪。
	var sensor_catalog_exists := ResourceLoader.exists("res://src/sensors/SensorCatalog.gd")
	var ghost_ready_sent: bool = await _wait_absent(sim_h4, "sensor_ready", 500)
	var extra_assembly: bool = await _wait_absent(sim_a, "assembly_ready", 10)
	var manifest_a: Dictionary = manager._manifests.get("simA", {})
	_check("H4 unknown-sensor-policy",
		sim_h4.IsOpen() and extra_assembly and not manifest_a.has("lidar_ghost")
		and (ghost_ready_sent if sensor_catalog_exists else not ghost_ready_sent),
		"catalog=%s no_ready=%s open=%s no_extra_assembly=%s manifest=%s" % [
			sensor_catalog_exists, ghost_ready_sent, sim_h4.IsOpen(), extra_assembly, manifest_a])

	manager.handshake_timeout_ms = 300
	var sim_h5: Variant = _new_sim(port)
	await _wait_open(sim_h5, 1500)
	# 握手超时由 `Heartbeat`（Timer，1.0 s = **60 引擎帧**）巡检，判据是墙钟 300 ms。
	# 旧写法 `_wait_closed(sim_h5, 2500)` 是纯墙钟：Xvfb（~10~14 fps）2.5 s 只跑 25~35 帧，
	# 一次心跳都没到 -> 必红（HEAD 上就红，不是本任务的回归）。
	# 修法：**两个时钟都满足**才允许判"没关" —— ≥70 帧（至少一次心跳）+ ≥400 ms 墙钟（时间戳真的过期）。
	await _wait_frames(70)
	await _wait_ms(400)
	var h5_closed: bool = await _wait_closed_frames(sim_h5, 90)
	manager.handshake_timeout_ms = 5000
	_check("H5 handshake-timeout", h5_closed, "closed=%s code=%d" % [h5_closed, sim_h5.close_code])

	sim_a.Send({"msg": "bogus", "seq": 9})
	await _wait_ms(300)
	_check("H6 unknown-msg-ignored",
		sim_a.IsOpen() and manager.GetVehicle("simA") != null,
		"open=%s vehicle=%s" % [sim_a.IsOpen(), manager.GetVehicle("simA")])

	var count_before_h7: int = manager.GetVehicleCount()
	sim_a2.Send({
		"msg": "chassis_hello",
		"peer_id": "simX",
		"chassis": "diff_drive_ugv",
		"params": {},
		"manifest": [],
	})
	await _wait_ms(400)
	_check("H7 sensor-declares-chassis",
		sim_a2.IsOpen() and manager.GetVehicle("simX") == null
		and manager.GetVehicleCount() == count_before_h7,
		"open=%s simX=%s count=%d (was %d)" % [sim_a2.IsOpen(), manager.GetVehicle("simX"),
			manager.GetVehicleCount(), count_before_h7])

	# ============================================================ J. camera 装配（task_34 Step 3）
	# 刻意放在 G/H 的**所有计数断言之后**：J 新建第 4 台车 + 第 2 条传感器连接，
	#   因此 C5 GetVehicleCount()==1 / F1 assembled==3 / G2 count==2 / G5 links==3 全部不受影响。
	var sim_cam: Variant = _new_sim(port)
	await _wait_open(sim_cam, 1500)
	sim_cam.Send({
		"msg": "chassis_hello",
		"peer_id": "simCam",
		"chassis": "diff_drive_ugv",
		"params": {},
		"manifest": [{"slot": CAMERA_SLOT, "sensor": "camera"}],
	})
	var ready_cam: Dictionary = await _wait_msg(sim_cam, "chassis_ready", 1500)
	var sim_cam2: Variant = _new_sim(port)
	await _wait_open(sim_cam2, 1500)
	sim_cam2.Send({
		"msg": "sensor_hello",
		"peer_id": "simCam",
		"slot": CAMERA_SLOT,
		"sensor": "camera",
		"mount": CAMERA_MOUNT,
		"params": CAMERA_PARAMS,
	})
	var camera_ready: Dictionary = await _wait_msg(sim_cam2, "sensor_ready", 1500)
	var camera_id: int = int(camera_ready.get("sensor_id", 0))
	_check("J1 camera-sensor-ready",
		not ready_cam.is_empty() and not camera_ready.is_empty()
		and str(camera_ready.get("slot", "")) == CAMERA_SLOT and camera_id >= 1,
		"sensor_id=%d ready=%s" % [camera_id, camera_ready])

	var vehicle_cam: Node3D = manager.GetVehicle("simCam")
	var camera_node: Node = vehicle_cam.GetSensor(CAMERA_SLOT) if vehicle_cam != null else null
	var camera_script: Script = camera_node.get_script() as Script if camera_node != null else null
	var camera_script_path: String = camera_script.resource_path if camera_script != null else ""
	var camera_size: Vector2i = camera_node.call("Get_Image_Size") if camera_node != null \
		else Vector2i(-1, -1)
	var camera_params: Dictionary = camera_node.call("Get_Params") if camera_node != null else {}
	var camera_can_capture: bool = bool(camera_node.call("Can_Capture")) \
		if camera_node != null else false
	var headless: bool = DisplayServer.get_name() == "headless"
	_check("J2 camera-geometry",
		camera_node != null and camera_script_path == CAMERA_SCRIPT
		and camera_size == CAMERA_WANT_SIZE
		and is_equal_approx(float(camera_params.get("fov_deg", -1.0)), 70.0)
		and is_equal_approx(float(camera_params.get("rate_hz", -1.0)), 1.0)
		and camera_can_capture == (not headless),
		"script=%s size=%s fov=%.1f rate=%.2f can_capture=%s" % [
			camera_script_path, str(camera_size),
			float(camera_params.get("fov_deg", -1.0)),
			float(camera_params.get("rate_hz", -1.0)), str(camera_can_capture)])

	# 白盒：证明 manager 在 sensor_ready 之后**真的调了 SetStream**，且用的是 ready 里的那个 id。
	_check("J3 camera-stream-bound",
		camera_node != null and bool(camera_node.call("HasStream"))
		and int(camera_node.get("_sensor_id")) == camera_id,
		"has_stream=%s sensor_id=%d（ready 里是 %d）" % [
			str(camera_node.call("HasStream")) if camera_node != null else "n/a",
			int(camera_node.get("_sensor_id")) if camera_node != null else -1, camera_id])

	# E27 在装配线上的守卫：相机零件纯视觉，子树里零碰撞 / 零物理体。
	var camera_physics: int = _Count_Physics(camera_node) if camera_node != null else -1
	_check("J4 camera-no-collision", camera_physics == 0,
		"physics_nodes=%d" % camera_physics)

	var assembly_cam: Dictionary = await _wait_msg(sim_cam, "assembly_ready", 1500)
	_check("J5 camera-assembly",
		not assembly_cam.is_empty() and str(assembly_cam.get("peer_id", "")) == "simCam",
		"assembly=%s" % [assembly_cam])

	# 出帧闸门：到点必须发生（引擎帧计时，不看墙钟）；无渲染器时**绝不空转出帧**。
	var tick_before: int = int(camera_node.call("Get_Tick_Count")) if camera_node != null else 0
	await _wait_frames(CAMERA_GATE_FRAMES)
	var tick_after: int = int(camera_node.call("Get_Tick_Count")) if camera_node != null else 0
	var camera_binaries: Array = sim_cam2.binaries
	var types_ok: bool = true
	for item: Variant in camera_binaries:
		var row_bytes: PackedByteArray = item
		var row_header: Dictionary = WireProtocol.ParseFrameHeader(row_bytes)
		if int(row_header.get("type", -1)) != WireProtocol.FRAME_TYPE_CAMERA \
				or int(row_header.get("slot_id", -1)) != camera_id:
			types_ok = false
	var frames_ok: bool = types_ok and (camera_binaries.size() >= 1
		if camera_can_capture else camera_binaries.is_empty())
	_check("J6 camera-stream-gate", tick_after - tick_before >= 1 and frames_ok,
		"ticks=+%d frames=%d types=%s can_capture=%s" % [
			tick_after - tick_before, camera_binaries.size(),
			"ok" if types_ok else "bad", str(camera_can_capture)])

	# 帧内容：只有真渲染器下才有帧可看（headless = dummy 渲染器，物理上出不了图）。
	if camera_can_capture and not camera_binaries.is_empty():
		var last_frame: PackedByteArray = camera_binaries[camera_binaries.size() - 1]
		var last_payload: PackedByteArray = last_frame.slice(WireProtocol.FRAME_HEADER_SIZE)
		_check("J7 camera-payload-jpeg",
			last_payload.size() > 1000 and last_payload.slice(0, 3).hex_encode() == "ffd8ff",
			"payload=%d head=%s" % [last_payload.size(),
				last_payload.slice(0, 3).hex_encode()])
	else:
		print("[skip] J7 camera-payload-jpeg（display=%s：dummy 渲染器，相机不出帧）"
			% DisplayServer.get_name())
		_skipped += 1

	# ============================================================ K. 行人（task_35 Step 2）
	# 刻意放在 J 之后：C5/F1/G2/G5 这些计数断言全在前面，K 段新增的车/人**不影响**它们。
	# 帧数用 _wait_physics / _wait_frames，墙钟用 _wait_ms（--fixed-fps 只定 delta，不定实际帧率）。
	var vehicles_before_ped: int = manager.GetVehicleCount()
	var sim_ped: Variant = _new_sim(port)
	await _wait_open(sim_ped, 1500)
	sim_ped.Send({
		"msg": "chassis_hello",
		"peer_id": PED_ID,
		"chassis": "pedestrian",
		"kind": "pedestrian",
		"params": {},
		"spawn": PED_SPAWN,
		"manifest": [],
	})
	var ped_ready: Dictionary = await _wait_msg(sim_ped, "chassis_ready", 1500)
	var ped_spawn: Dictionary = ped_ready.get("spawn", {}) if not ped_ready.is_empty() else {}
	_check("K1 pedestrian-assembled",
		not ped_ready.is_empty()
		and str(ped_ready.get("kind", "")) == WireProtocol.KIND_PEDESTRIAN
		and int(ped_ready.get("pedestrian_id", -1)) >= 1
		and absf(float(ped_spawn.get("x", -999.0)) - float(PED_SPAWN["x"])) < PED_SPAWN_TOL
		and absf(float(ped_spawn.get("y", -999.0)) - float(PED_SPAWN["y"])) < PED_SPAWN_TOL
		and absf(float(ped_spawn.get("z", -999.0)) - float(PED_SPAWN["z"])) < PED_SPAWN_TOL
		and absf(float(ped_spawn.get("yaw", -999.0)) - float(PED_SPAWN["yaw"])) < PED_SPAWN_TOL,
		"ready=%s" % [ped_ready])

	var ped_node: Node3D = manager.GetPedestrian(PED_ID)
	var ped_parent: Node = ped_node.get_parent() if ped_node != null else null
	var ped_script: Script = ped_node.get_script() as Script if ped_node != null else null
	var ped_script_path: String = ped_script.resource_path if ped_script != null else ""
	var ped_in_vehicles: Node = vehicles_root.get_node_or_null(NodePath(PED_ID)) \
		if vehicles_root != null else null
	_check("K2 pedestrian-routing",
		ped_node != null and manager.GetVehicle(PED_ID) == null
		and manager.GetVehicleCount() == vehicles_before_ped
		and ped_in_vehicles == null
		and ped_parent != null and str(ped_parent.name) == "Pedestrians"
		and ped_script_path == PEDESTRIAN_SCRIPT,
		"node=%s parent=%s script=%s vehicles=%d (was %d) in_vehicles=%s" % [
			ped_node, ped_parent.name if ped_parent != null else "null", ped_script_path,
			manager.GetVehicleCount(), vehicles_before_ped, ped_in_vehicles])

	# K3：pose 照常 10 Hz 只发底盘连接（与 check_pose.C2 同口径：引擎帧数窗口）
	var pose_before: int = sim_ped.Count("pose")
	await _wait_frames(60)
	var pose_delta: int = sim_ped.Count("pose") - pose_before
	_check("K3 pedestrian-pose-10hz", pose_delta >= 9, "pose=+%d in 60 frames" % pose_delta)

	# K4：v + dir -> 先转后走（弧线，不是瞬移）。等待窗口内**每帧重发**（Python 客户端的真实节奏），
	# 否则 300 ms 超时会在窗口中途把 v 归零（Xvfb 下实测只走 3 帧）。
	await _drive_physics(sim_ped, {"msg": "motion_cmd", "seq": 1, "v": 1.0, "dir": PI / 2.0},
		PED_CONVERGE_FRAMES)
	var pose_turned: Dictionary = ped_node.GetPose() if ped_node != null else {}
	var yaw_err: float = absf(wrapf(float(pose_turned.get("yaw", 0.0)) - PI / 2.0, -PI, PI))
	_check("K4 pedestrian-motion",
		ped_node != null and yaw_err < 0.05
		and float(pose_turned.get("y", -999.0)) > float(PED_SPAWN["y"]) + 0.2
		and absf(float(pose_turned.get("x", -999.0)) - float(PED_SPAWN["x"])) < 0.6,
		"pose=%s yaw_err=%.4f（dir=%.4f）" % [pose_turned, yaw_err, PI / 2.0])

	# K5：老协议 action/speed 对行人也生效（forward 50% -> 1.26 × 0.5 m/s）。同样每帧重发。
	await _drive_physics(sim_ped, {"msg": "motion_cmd", "seq": 2, "action": "forward", "speed": 50}, 4)
	var speed_legacy: float = float(ped_node.call("Get_Speed")) if ped_node != null else -1.0
	_check("K5 pedestrian-legacy-action", absf(speed_legacy - 0.63) < 0.02,
		"speed=%.4f（want 0.63）" % speed_legacy)

	# K6：白盒 —— 超时阈值 + "刚发过指令" 的簿记。用同一 helper，保证最后一帧刚下发。
	var ped_manager_script: Script = manager.get_script()
	var ped_manager_consts: Dictionary = ped_manager_script.get_script_constant_map() \
		if ped_manager_script != null else {}
	var timeout_ms: int = int(ped_manager_consts.get("PEDESTRIAN_CMD_TIMEOUT_MS", -1))
	await _drive_physics(sim_ped, {"msg": "motion_cmd", "seq": 3, "v": 1.0, "dir": PI / 2.0}, 2)
	var motion_seen: bool = manager._ped_last_motion_ms.has(PED_ID)
	var idle_now: bool = bool(manager._ped_idle.get(PED_ID, true))
	_check("K6 pedestrian-timeout-meta",
		timeout_ms == 300 and motion_seen and not idle_now,
		"timeout=%d last_motion=%s idle=%s" % [timeout_ms, motion_seen, idle_now])

	# K7：停发指令 -> 300 ms 后自动 v=0（只停、不断连），随后位姿完全不动
	await _wait_ms(PED_IDLE_WAIT_MS)
	var speed_idle: float = float(ped_node.call("Get_Speed")) if ped_node != null else -1.0
	var idle_after: bool = bool(manager._ped_idle.get(PED_ID, false))
	var still_a: Dictionary = ped_node.GetPose() if ped_node != null else {}
	await _wait_physics(10)
	var still_b: Dictionary = ped_node.GetPose() if ped_node != null else {}
	var dx_still: float = absf(float(still_b.get("x", 0.0)) - float(still_a.get("x", 0.0)))
	var dy_still: float = absf(float(still_b.get("y", 0.0)) - float(still_a.get("y", 0.0)))
	_check("K7 pedestrian-timeout-stop",
		absf(speed_idle) < 0.01 and idle_after and sim_ped.IsOpen()
		and dx_still < 1e-4 and dy_still < 1e-4,
		"speed=%.4f idle=%s open=%s dx=%.6f dy=%.6f" % [
			speed_idle, idle_after, sim_ped.IsOpen(), dx_still, dy_still])

	# K8：非法 spawn（越界）-> warn + 回退默认点；回执 spawn == pose 静止值
	var sim_ped2: Variant = _new_sim(port)
	await _wait_open(sim_ped2, 1500)
	sim_ped2.Send({
		"msg": "chassis_hello",
		"peer_id": "human-02",
		"chassis": "pedestrian",
		"kind": "pedestrian",
		"params": {},
		"spawn": {"x": 200.0, "y": 5.0, "z": 0.0},
		"manifest": [],
	})
	var ready_ped2: Dictionary = await _wait_msg(sim_ped2, "chassis_ready", 1500)
	var spawn_ped2: Dictionary = ready_ped2.get("spawn", {}) if not ready_ped2.is_empty() else {}
	var pose_ped2: Dictionary = await _wait_msg(sim_ped2, "pose", 1000)
	_check("K8 pedestrian-spawn-invalid",
		not ready_ped2.is_empty()
		and float(spawn_ped2.get("x", -999.0)) >= 0.0
		and float(spawn_ped2.get("x", -999.0)) < 128.0
		and absf(float(spawn_ped2.get("x", -999.0)) - 200.0) > 1.0
		and absf(float(spawn_ped2.get("x", -999.0)) - PED_DEFAULT_X) < PED_SPAWN_TOL
		and absf(float(spawn_ped2.get("y", -999.0)) - PED_DEFAULT_Y) < PED_SPAWN_TOL
		and absf(float(pose_ped2.get("x", -999.0)) - float(spawn_ped2.get("x", 0.0))) < PED_SPAWN_TOL
		and absf(float(pose_ped2.get("y", -999.0)) - float(spawn_ped2.get("y", 0.0))) < PED_SPAWN_TOL,
		"spawn=%s pose=%s" % [spawn_ped2, pose_ped2])

	# K9：不带 spawn -> 默认点（合法用法，不 warn）
	var sim_ped3: Variant = _new_sim(port)
	await _wait_open(sim_ped3, 1500)
	sim_ped3.Send({
		"msg": "chassis_hello",
		"peer_id": "human-03",
		"chassis": "pedestrian",
		"kind": "pedestrian",
		"params": {},
		"manifest": [],
	})
	var ready_ped3: Dictionary = await _wait_msg(sim_ped3, "chassis_ready", 1500)
	var spawn_ped3: Dictionary = ready_ped3.get("spawn", {}) if not ready_ped3.is_empty() else {}
	_check("K9 pedestrian-spawn-default",
		not ready_ped3.is_empty()
		and absf(float(spawn_ped3.get("x", -999.0)) - PED_DEFAULT_X) < PED_SPAWN_TOL
		and absf(float(spawn_ped3.get("y", -999.0)) - PED_DEFAULT_Y) < PED_SPAWN_TOL
		and absf(float(spawn_ped3.get("z", -999.0))) < PED_SPAWN_TOL
		and absf(float(spawn_ped3.get("yaw", -999.0))) < PED_SPAWN_TOL,
		"spawn=%s（want %.1f, %.1f, 0, 0）" % [spawn_ped3, PED_DEFAULT_X, PED_DEFAULT_Y])

	# K10：非法 kind（"quadcopter"）-> 不断连、按 robot 处理、进 Vehicles
	var sim_kind: Variant = _new_sim(port)
	await _wait_open(sim_kind, 1500)
	sim_kind.Send({
		"msg": "chassis_hello",
		"peer_id": "simKind",
		"chassis": "diff_drive_ugv",
		"kind": "quadcopter",
		"params": {},
		"manifest": [],
	})
	var ready_kind: Dictionary = await _wait_msg(sim_kind, "chassis_ready", 1500)
	_check("K10 kind-invalid-robot",
		not ready_kind.is_empty() and sim_kind.IsOpen()
		and str(ready_kind.get("kind", "")) == WireProtocol.KIND_ROBOT
		and manager.GetVehicle("simKind") != null
		and manager.GetPedestrian("simKind") == null,
		"ready=%s open=%s vehicle=%s pedestrian=%s" % [ready_kind, sim_kind.IsOpen(),
			manager.GetVehicle("simKind"), manager.GetPedestrian("simKind")])

	# K11：不带 kind -> 缺省 robot（老 sim 一字不改也走老路）
	var peds_before_plain: int = manager.GetPedestrianCount()
	var sim_plain: Variant = _new_sim(port)
	await _wait_open(sim_plain, 1500)
	sim_plain.Send({
		"msg": "chassis_hello",
		"peer_id": "simPlain",
		"chassis": "diff_drive_ugv",
		"params": {},
		"manifest": [],
	})
	var ready_plain: Dictionary = await _wait_msg(sim_plain, "chassis_ready", 1500)
	_check("K11 kind-default-robot",
		not ready_plain.is_empty()
		and str(ready_plain.get("kind", "")) == WireProtocol.KIND_ROBOT
		and manager.GetVehicle("simPlain") != null
		and manager.GetPedestrianCount() == peds_before_plain,
		"ready=%s peds=%d (was %d)" % [ready_plain,
			manager.GetPedestrianCount(), peds_before_plain])

	# K12：行人没有传感器挂载点 -> 不许 Invalid call（SCRIPT ERROR），只登记连接
	var sim_ped4: Variant = _new_sim(port)
	await _wait_open(sim_ped4, 1500)
	sim_ped4.Send({
		"msg": "chassis_hello",
		"peer_id": "human-04",
		"chassis": "pedestrian",
		"kind": "pedestrian",
		"params": {},
		"spawn": {"x": 74.0, "y": 70.0, "z": 0.0, "yaw": 0.0},
		"manifest": [{"slot": "lidar_front", "sensor": "lidar_2d"}],
	})
	var ready_ped4: Dictionary = await _wait_msg(sim_ped4, "chassis_ready", 1500)
	var sim_ped4_sensor: Variant = _new_sim(port)
	await _wait_open(sim_ped4_sensor, 1500)
	sim_ped4_sensor.Send({
		"msg": "sensor_hello",
		"peer_id": "human-04",
		"slot": "lidar_front",
		"sensor": "lidar_2d",
		"mount": {"x": 0.0, "y": 0.0, "z": 0.2},
		"params": {"range": 12.0, "rays": 360, "rate_hz": 10},
	})
	var sensor_ready_ped: Dictionary = await _wait_msg(sim_ped4_sensor, "sensor_ready", 1500)
	_check("K12 pedestrian-sensor-no-crash",
		not ready_ped4.is_empty() and not sensor_ready_ped.is_empty()
		and str(sensor_ready_ped.get("slot", "")) == "lidar_front"
		# 行人没有 Chassis/Sensors 挂载点 -> 走"几何未落地"分支：回执里**没有** geometry 键。
		# 这才是"没发生 Invalid call"的直接证据（此前只靠外部 grep 0 ERROR 间接兜住）。
		and not sensor_ready_ped.has("geometry")
		and sim_ped4.IsOpen() and sim_ped4_sensor.IsOpen(),
		"ready=%s sensor_ready=%s geometry=%s open=%s/%s" % [ready_ped4, sensor_ready_ped,
			str(sensor_ready_ped.get("geometry", "<absent>")),
			sim_ped4.IsOpen(), sim_ped4_sensor.IsOpen()])

	# K13：底盘连接断开 -> 行人被移除（计数先取快照，不硬编码）
	var peds_before_close: int = manager.GetPedestrianCount()
	sim_ped.CloseClient()
	await _wait_ms(500)
	var peds_root: Node3D = manager.GetPedestriansRoot()
	var ped_node_after: Node = peds_root.get_node_or_null(NodePath(PED_ID)) \
		if peds_root != null else null
	_check("K13 pedestrian-removed-on-close",
		manager.GetPedestrian(PED_ID) == null
		and not manager._pedestrians.has(PED_ID)
		and ped_node_after == null
		and manager.GetPedestrianCount() == peds_before_close - 1,
		"GetPedestrian=%s in_table=%s node=%s count=%d (was %d)" % [
			manager.GetPedestrian(PED_ID), manager._pedestrians.has(PED_ID), ped_node_after,
			manager.GetPedestrianCount(), peds_before_close])

	# K14（第 2 轮，prev_issues #2-①）：robot -> pedestrian 换 kind（**新连接**同 peer_id）。
	#   旧车必须从 Vehicles/_peer_vehicles 消失、旧连接必须被关；pose 必须来自新行人；
	#   旧连接下一帧的 _CleanupLink() 不得把**刚注册**的行人再摘一次。
	var vehicles_before_k14: int = manager.GetVehicleCount()
	var peds_before_k14: int = manager.GetPedestrianCount()
	var ev_k14: int = _removed_events.size()
	var sim_k14a: Variant = _new_sim(port)
	await _wait_open(sim_k14a, 1500)
	sim_k14a.Send({"msg": "chassis_hello", "peer_id": "human-05", "chassis": "diff_drive_ugv",
		"params": {}, "manifest": []})
	var ready_k14a: Dictionary = await _wait_msg(sim_k14a, "chassis_ready", 1500)
	var sim_k14b: Variant = _new_sim(port)
	await _wait_open(sim_k14b, 1500)
	sim_k14b.Send({"msg": "chassis_hello", "peer_id": "human-05", "chassis": "pedestrian",
		"kind": "pedestrian", "spawn": {"x": 72.0, "y": 74.0, "z": 0.0, "yaw": 0.0},
		"params": {}, "manifest": []})
	var ready_k14b: Dictionary = await _wait_msg(sim_k14b, "chassis_ready", 1500)
	var k14_old_closed: bool = await _wait_closed_frames(sim_k14a, 120)
	await _wait_frames(2)                       # 让旧连接的 _CleanupLink 跑完
	var pose_k14: Dictionary = await _wait_msg(sim_k14b, "pose", 1500)
	var ped_k14: Node3D = manager.GetPedestrian("human-05")
	_check("K14 kind-switch-robot-to-pedestrian",
		not ready_k14a.is_empty() and not ready_k14b.is_empty()
		and str(ready_k14b.get("kind", "")) == WireProtocol.KIND_PEDESTRIAN
		and k14_old_closed and ped_k14 != null
		and manager.GetVehicle("human-05") == null
		and manager.GetVehicleCount() == vehicles_before_k14      # 快照在注册前：+1（车）-1（摘）= 回到原值
		and manager.GetPedestrianCount() == peds_before_k14 + 1
		and vehicles_root.get_node_or_null(NodePath("human-05")) == null
		and absf(float(pose_k14.get("x", -999.0)) - 72.0) < 0.1
		and absf(float(pose_k14.get("y", -999.0)) - 74.0) < 0.1
		and _removed_events.slice(ev_k14) == ["v:human-05"],
		"vehicles=%d(was %d) peds=%d(was %d) old_closed=%s pose=%s events=%s" % [
			manager.GetVehicleCount(), vehicles_before_k14, manager.GetPedestrianCount(),
			peds_before_k14, k14_old_closed, pose_k14, _removed_events.slice(ev_k14)])

	# K15（prev_issues #2-②）：pedestrian -> robot 换 kind（新连接同 peer_id）。
	#   残留行人必须消失；车指令必须作用在**车**上（旧实现里会打到残留行人、车纹丝不动）。
	var vehicles_before_k15: int = manager.GetVehicleCount()
	var peds_before_k15: int = manager.GetPedestrianCount()
	var ev_k15: int = _removed_events.size()
	var sim_k15a: Variant = _new_sim(port)
	await _wait_open(sim_k15a, 1500)
	sim_k15a.Send({"msg": "chassis_hello", "peer_id": "human-06", "chassis": "pedestrian",
		"kind": "pedestrian", "spawn": {"x": 76.0, "y": 74.0, "z": 0.0, "yaw": 0.0},
		"params": {}, "manifest": []})
	var ready_k15a: Dictionary = await _wait_msg(sim_k15a, "chassis_ready", 1500)
	var sim_k15b: Variant = _new_sim(port)
	await _wait_open(sim_k15b, 1500)
	sim_k15b.Send({"msg": "chassis_hello", "peer_id": "human-06", "chassis": "diff_drive_ugv",
		"params": {}, "manifest": []})
	var ready_k15b: Dictionary = await _wait_msg(sim_k15b, "chassis_ready", 1500)
	var k15_old_closed: bool = await _wait_closed_frames(sim_k15a, 120)
	await _wait_frames(2)
	var vehicle_k15: Node3D = manager.GetVehicle("human-06")
	var k15_pose_before: Dictionary = vehicle_k15.GetPose() if vehicle_k15 != null else {}
	await _drive_physics(sim_k15b,
		{"msg": "motion_cmd", "seq": 1, "action": "forward", "speed": 100}, 20)
	var k15_pose_after: Dictionary = vehicle_k15.GetPose() if vehicle_k15 != null else {}
	var peds_root_k15: Node3D = manager.GetPedestriansRoot()
	_check("K15 kind-switch-pedestrian-to-robot",
		not ready_k15a.is_empty() and not ready_k15b.is_empty()
		and str(ready_k15b.get("kind", "")) == WireProtocol.KIND_ROBOT
		and k15_old_closed and vehicle_k15 != null
		and manager.GetPedestrian("human-06") == null
		and not manager._pedestrians.has("human-06")
		and manager.GetPedestrianCount() == peds_before_k15          # 快照在注册前：+1（人）-1（摘）= 回到原值
		and manager.GetVehicleCount() == vehicles_before_k15 + 1
		and peds_root_k15 != null and peds_root_k15.get_node_or_null(NodePath("human-06")) == null
		and float(k15_pose_after.get("x", 0.0)) > float(k15_pose_before.get("x", 0.0)) + 0.05
		and _removed_events.slice(ev_k15) == ["p:human-06"],
		"vehicles=%d(was %d) peds=%d(was %d) old_closed=%s pose=%s->%s events=%s" % [
			manager.GetVehicleCount(), vehicles_before_k15, manager.GetPedestrianCount(),
			peds_before_k15, k15_old_closed, k15_pose_before, k15_pose_after,
			_removed_events.slice(ev_k15)])

	# K16（prev_issues #2-③ / R13）：**同一条连接**上 robot -> pedestrian 重建。
	#   本连接不许被自己关掉；旧车只许摘一次（多一条 v: 就是幽灵移除）。
	var ev_k16: int = _removed_events.size()
	var sim_k16: Variant = _new_sim(port)
	await _wait_open(sim_k16, 1500)
	sim_k16.Send({"msg": "chassis_hello", "peer_id": "human-07", "chassis": "diff_drive_ugv",
		"params": {}, "manifest": []})
	var ready_k16a: Dictionary = await _wait_msg(sim_k16, "chassis_ready", 1500)
	sim_k16.Send({"msg": "chassis_hello", "peer_id": "human-07", "chassis": "pedestrian",
		"kind": "pedestrian", "spawn": {"x": 72.0, "y": 78.0, "z": 0.0, "yaw": 0.0},
		"params": {}, "manifest": []})
	var ready_k16b: Dictionary = await _wait_msg(sim_k16, "chassis_ready", 1500)
	await _wait_frames(2)
	var peds_root_k16: Node3D = manager.GetPedestriansRoot()
	_check("K16 kind-switch-same-connection",
		not ready_k16a.is_empty() and not ready_k16b.is_empty()
		and str(ready_k16b.get("kind", "")) == WireProtocol.KIND_PEDESTRIAN
		and sim_k16.IsOpen()
		and manager.GetVehicle("human-07") == null
		and manager.GetPedestrian("human-07") != null
		and vehicles_root.get_node_or_null(NodePath("human-07")) == null
		and peds_root_k16 != null and peds_root_k16.get_node_or_null(NodePath("human-07")) != null
		and _removed_events.slice(ev_k16) == ["v:human-07"],
		"open=%s vehicle=%s ped=%s events=%s" % [sim_k16.IsOpen(),
			manager.GetVehicle("human-07"), manager.GetPedestrian("human-07"),
			_removed_events.slice(ev_k16)])

	# ============================================================ I. 规范 lint
	var lint_bad: Array = []
	for path: String in NET_SCRIPTS:
		var code: String = _code_of(path)
		if code.is_empty() or code.contains("class_name") or code.contains("send_binary"):
			lint_bad.append(path)
	var manager_code: String = _code_of(WS_MANAGER_SCRIPT)
	_check("I1 net-lint", lint_bad.is_empty()
		and manager_code.contains("WorldTransform.PosToGodot("),
		"bad=%s pos_to_godot=%s" % [lint_bad, manager_code.contains("WorldTransform.PosToGodot(")])

	var missing_headers: Array = []
	for path: String in HEADER_FILES:
		var source: String = _raw_source_of(path)
		if not (source.contains("Presented by KeJi") and source.contains("Created Date")
				and source.contains("Modified Date")):
			missing_headers.append(path)
	_check("I2 file-headers", missing_headers.is_empty(), "missing=%s" % [missing_headers])
	_check("I3 no-push-error", not manager_code.contains("push_error"),
		"push_error_count=%d" % manager_code.count("push_error"))

	# ============================================================ 收尾（P12：退出时无 ERROR）
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


## 在 ms 窗口内**没有**收到 msg_name 才返回 true。
func _wait_absent(sim: Variant, msg_name: String, ms: int) -> bool:
	var deadline := Time.get_ticks_msec() + ms
	var seen := false
	while Time.get_ticks_msec() < deadline:
		_pump()
		if sim.Has(msg_name):
			seen = true
			sim.Take(msg_name)
		await process_frame
	_pump()
	if sim.Has(msg_name):
		seen = true
		sim.Take(msg_name)
	return not seen


func _wait_closed(sim: Variant, timeout_ms: int) -> bool:
	var deadline := Time.get_ticks_msec() + timeout_ms
	while Time.get_ticks_msec() < deadline:
		_pump()
		if sim.closed:
			return true
		await process_frame
	_pump()
	return sim.closed


func _wait_ms(ms: int) -> void:
	var deadline := Time.get_ticks_msec() + ms
	while Time.get_ticks_msec() < deadline:
		_pump()
		await process_frame
	_pump()


## 按**引擎帧**等连接关闭（第 2 轮 H5 的教训）：`--fixed-fps` 只固定 delta、不限制实际帧率，
## Xvfb 下一帧可到 ~100 ms。凡是"世界必须再跑 N 帧才会发生"的事都用帧数，不用墙钟。
func _wait_closed_frames(sim: Variant, frames: int) -> bool:
	for _i in range(frames):
		_pump()
		if sim.closed:
			return true
		await process_frame
	_pump()
	return sim.closed


## 按物理帧推进 n 帧，**每帧重发**同一条 motion_cmd（模拟 20~50 Hz 的 Python 客户端）。
## 为什么必须重发：行人 300 ms 超时按**墙钟**判，而 Xvfb 下一帧可达 70~100 ms ——
## 只发一次会让世界在等待窗口中途把 v 归零（第 2 轮 K4/K5 的教训）。车没有超时，重发也无害。
func _drive_physics(sim: Variant, msg: Dictionary, frames: int) -> void:
	for _i in range(frames):
		sim.Send(msg)
		_pump()
		await physics_frame


## 移除事件 -> "v:<peer>"（K14~K16 用它锁"只 emit 一次"）。
func _On_Vehicle_Removed(peer_id: String) -> void:
	_removed_events.append("v:" + peer_id)


## 移除事件 -> "p:<peer>"。
func _On_Pedestrian_Removed(peer_id: String) -> void:
	_removed_events.append("p:" + peer_id)


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		_pump()
		await physics_frame


## 等 n 个**引擎帧**（--fixed-fps 60 下 60 帧 = 引擎时间 1 s）。
## 频率类断言一律用它，**不用墙钟毫秒**：`--fixed-fps` 固定 delta 但不限制实际帧率（task_34 R4）。
func _wait_frames(frames: int) -> void:
	for _i in range(frames):
		_pump()
		await process_frame


## 子树物理节点计数（E27：传感器必须 0；与 check_camera_sensor.gd 的 `_Count_Physics` 同实现）。
func _Count_Physics(node: Node) -> int:
	var n: int = 0
	if node is CollisionShape3D or node is CollisionPolygon3D \
		or node is PhysicsBody3D or node is Area3D:
		n += 1
	for child in node.get_children():
		n += _Count_Physics(child)
	return n


## 去掉以 # 开头的注释行，只留可执行代码（白盒 lint 用，同 check_diff_drive.gd）。
func _strip_comment_lines(source: String) -> String:
	var code := ""
	for raw_line: String in source.split("\n"):
		if raw_line.strip_edges().begins_with("#"):
			continue
		code += raw_line + "\n"
	return code


func _raw_source_of(path: String) -> String:
	if path.ends_with(".tscn"):
		var file := FileAccess.open(path, FileAccess.READ)
		if file == null:
			return ""
		return file.get_as_text()
	var script: Script = load(path) as Script
	if script == null:
		return ""
	return script.source_code


func _code_of(path: String) -> String:
	return _strip_comment_lines(_raw_source_of(path))


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
	if _skipped > 0:
		print("[check] skipped=", _skipped, "（相机帧内容：无渲染器，不计入 total）")
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
