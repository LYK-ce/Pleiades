# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# WebSocketManager 自检（Step 6）：场景形态 / World.tscn 接线 / 底盘+传感器装配 / 运动下行 /
# 传感器先于底盘 / 空 manifest / 断开清理 / 错误路径 / 规范 lint。
# 用法：timeout 180 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://tests/check_ws_manager.gd
# 期望：failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR/SCRIPT ERROR（COMPONENTS.md §11 Step 6）。
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
#
# 白盒断言（manager._manifests 等）在本仓库是既有做法（check_diff_drive.gd 的 A35 lint 同例）。

extends SceneTree

const WireProtocol := preload("res://scripts/net/WireProtocol.gd")
const ComponentLink := preload("res://scripts/net/ComponentLink.gd")
const WorldTransform := preload("res://scripts/common/WorldTransform.gd")

const WS_MANAGER_SCENE := "res://WebSocketManager.tscn"
const WS_MANAGER_SCRIPT := "res://scripts/net/WebSocketManager.gd"
const WIRE_SCRIPT := "res://scripts/net/WireProtocol.gd"
const LINK_SCRIPT := "res://scripts/net/ComponentLink.gd"
const WORLD_SCENE := "res://World.tscn"
const VEHICLE_SCENE := "res://Vehicle.tscn"
const LIDAR_SCENE := "res://sensors/Lidar2D.tscn"

const NET_SCRIPTS := [WIRE_SCRIPT, LINK_SCRIPT, WS_MANAGER_SCRIPT]
const HEADER_FILES := [WIRE_SCRIPT, LINK_SCRIPT, WS_MANAGER_SCRIPT, WS_MANAGER_SCENE]

const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)
const SPAWN_TOL := 0.001
## LiDAR 一圈 360 线：15 + 4*360 = 1455 字节（§8.2）。
const LIDAR_RAYS := 360
const LIDAR_FRAME_BYTES := 1455

var _total: int = 0
var _failed: int = 0
var _harness_errors: int = 0
var _sims: Array = []


# ================================================================ 测试替身：sim 侧 WS 客户端

class Sim:
	var peer: WebSocketPeer = null
	## 已解析的文本帧（FIFO）
	var inbox: Array = []
	## 收到的二进制帧计数（断言"下行没有乱发"）
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

	# spawn 规则（白盒）：第 1 台 UGV = World 出生原点；UAV 抬到 2.0 m（§7.1）
	var spawn_ugv: Dictionary = manager.call("_PickSpawn", "diff_drive_ugv")
	var spawn_uav: Dictionary = manager.call("_PickSpawn", "quadrotor_uav")
	_check("B5 spawn-rule",
		absf(float(spawn_ugv.get("x", -1.0)) - SPAWN_WORLD.x) < SPAWN_TOL
		and absf(float(spawn_ugv.get("y", -1.0)) - SPAWN_WORLD.y) < SPAWN_TOL
		and absf(float(spawn_ugv.get("z", -1.0))) < SPAWN_TOL
		and absf(float(spawn_uav.get("z", -1.0)) - 2.0) < SPAWN_TOL,
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
	var sensor_catalog_exists := ResourceLoader.exists("res://scripts/sensors/SensorCatalog.gd")
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
	var h5_closed: bool = await _wait_closed(sim_h5, 2500)
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


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		_pump()
		await physics_frame


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
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
