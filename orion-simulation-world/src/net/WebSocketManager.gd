# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# WebSocketManager 组件（task_30 §5.1 / COMPONENTS.md §4.5）：世界侧 WS 服务端的唯一实例。
#   监听一个端口 -> accept N 条连接 -> 每条连接记住「我是谁的、我是什么组件」
#   -> 收 chassis_hello / sensor_hello 装配 avatar -> 按 manifest 判就绪
#   -> 回 chassis_ready / sensor_ready / assembly_ready
#   -> 10 Hz 向**底盘连接**下发 pose（世界坐标位姿，task_31 §三）-> 按连接断开清理。
#
# 两张主表（task_30 §5.1）：
#   1) peer_id -> vehicle      = _peer_vehicles
#   2) 连接 -> (peer_id, 角色)  = 每个 ComponentLink 自己带的字段（_links 保证可枚举）
#   _manifests / _manifest_parts / _assembly_sent / _pending_sensors 只是**就绪簿记**，
#   不构成第三套身份。
#
# 就绪语义（关键澄清）：`manifest` 就绪 = **manifest 声明的槽位都在对应连接上就位**
#   （task_30 §五："这辆车应该有几个零件、都连上了没"）。"几何是否实例化成功"是世界侧能力
#   问题（Lidar2D.tscn 到 Step 8 才有），**不得**让它阻塞 assembly_ready，否则 Step 6
#   根本做不出验收；两者在日志与 _manifest_parts 里区分清楚。
#
# 不做什么：不算物理（Vehicle/底盘）、不做传感（Step 8）、不碰相机（相机走 group "vehicle"）。
# 不声明全局类名（§9.1 E5）；World.tscn 通过 instance + node_paths 绑定两个 @export（E16/E20）。

extends Node


## 一台 avatar 建好并挂上底盘（在 chassis_ready 之后、回放 pending 之前发出）。
signal vehicle_spawned(peer_id: String, vehicle: Node3D)
## manifest 全部就位、assembly_ready 已发出（每 peer 仅一次）。
signal assembly_completed(peer_id: String)
## 一台 avatar 被移除（底盘断开 / 重复 hello / Stop）。
signal vehicle_removed(peer_id: String)
## 一条连接被清理（含未握手的连接；未握手时 peer_id 可能为空）。
signal link_closed(peer_id: String, role: StringName, slot: String)


const ComponentLink := preload("res://src/net/ComponentLink.gd")
const WireProtocol := preload("res://src/net/WireProtocol.gd")
const WorldClock := preload("res://src/common/WorldClock.gd")
const WorldTransform := preload("res://src/common/WorldTransform.gd")
## 出生原点唯一来源（P8：preload 后可直接读其 const，禁止复制字面量）。
const World := preload("res://src/world/World.gd")

const VEHICLE_SCENE_PATH := "res://src/scenes/Vehicle.tscn"
const CHASSIS_CATALOG_PATH := "res://src/chassis/ChassisCatalog.gd"
## Step 8 才落地：文件不存在时 _ResolveCatalogScene() 直接返回 null（不会 load 报错）。
const SENSOR_CATALOG_PATH := "res://src/sensors/SensorCatalog.gd"

const SETTING_WS_PORT := "sim_world/network/ws_port"
const DEFAULT_WS_PORT := 9999

## WebSocket 关闭码（§8.3 / RFC 6455）：1000 正常、1001 走开、1002 协议错、1008 策略、1011 内部错。
const CLOSE_NORMAL := 1000
const CLOSE_GOING_AWAY := 1001
const CLOSE_PROTOCOL := 1002
const CLOSE_POLICY := 1008
const CLOSE_INTERNAL := 1011

const ROLE_CHASSIS := &"chassis"
const ROLE_SENSOR := &"sensor"

## UAV 的巡航高度（世界 z，米，§7.1）；UGV = 0。
const UAV_CHASSIS := "quadrotor_uav"
const UAV_ALTITUDE_M := 2.5
## 多车 spawn 错开：8 个方位、每环 +2 m（避免两台车叠在同一格）。
const SPAWN_RING_SLOTS := 8
const SPAWN_RING_STEP_M := 2.0
## u16 语义（§8.1-7）：> 65535 回绕到 1。
const MAX_WIRE_ID := 65535
## 位姿下行周期（秒）：对齐真机 state_notifier 的 100 ms（base robot.rs:213）。
const POSE_BROADCAST_INTERVAL_S := 0.1


# ================================================================ 装配入口（World.tscn 绑定）

## avatar 场景：World.tscn 里指向 res://src/scenes/Vehicle.tscn（E20：必须 node_paths + node_paths 相对自身）。
@export var vehicle_scene: PackedScene = null
## avatar 容器：World.tscn 里指向 ../../Vehicles（相对本节点自身！写成 ../Vehicles 会静默 null）。
@export var vehicles_root: Node3D = null
## 0 -> 读 sim_world/network/ws_port（9999）；> 0 -> 直接用它。
@export var listen_port: int = 0
## false = 不自动监听（自检用；默认 true）。
@export var autolisten: bool = true
## 连接建立后多久没收到合法 hello 就关（§8.3-2）。
@export var handshake_timeout_ms: int = 5000
## 传感器先到、底盘一直不来：pending 连接最多等这么久，防泄漏。
@export var pending_timeout_ms: int = 10000
## true 时 Heartbeat 每秒打一行统计（默认关，避免日志爆炸，R19）。
@export var stats_log: bool = false


## 目录查询结果：区分「目录里没登记这个名字」与「目录脚本还没落地」（§8.3-4 只看前者）。
const CATALOG_UNKNOWN := 0     ## 目录存在，但未登记该名字 -> 拒绝 / 忽略该 slot
const CATALOG_KNOWN := 1       ## 目录里登记了 -> 继续解析场景
const CATALOG_MISSING := 2     ## 目录脚本本身还没落地（SensorCatalog 到 Step 8 才有）


# ---------------------------------------------------------------- 主表 1：peer_id -> vehicle
var _peer_vehicles: Dictionary = {}
# ---------------------------------------------------------------- 主表 2 的可枚举侧：连接对象自己带 (peer_id, 角色)
var _links: Array = []
# ---------------------------------------------------------------- 装配簿记
## peer_id -> {slot(String): bool 已就位}
var _manifests: Dictionary = {}
## peer_id -> {slot(String): sensor 名}（校验 hello 与 manifest 声明是否一致）
var _manifest_parts: Dictionary = {}
## peer_id -> bool（assembly_ready 每 peer 仅一次）
var _assembly_sent: Dictionary = {}
## peer_id -> Array[{link, hello, since_ms}]（传感器先于底盘到达时的暂存）
var _pending_sensors: Dictionary = {}

var _next_avatar_id: int = 1
var _next_sensor_id: int = 1
var _next_conn_id: int = 1
var _tcp_server: TCPServer = null
## 实际监听端口（0 = 未监听）；自检用临时端口（P5）。
var _listen_port: int = 0
## 传感器场景缺失只告警一次（Step 8 前每个 slot 都刷一行没有意义）。
var _warned_missing_sensor := false
## SensorCatalog 未落地只告警一次（此时无法校验传感器名，按"已知"放行）。
var _warned_missing_sensor_catalog := false
## vehicles_root 缺失只告警一次。
var _warned_no_root := false
## 位姿下行累加器（秒）；与物理帧解耦，见 _BroadcastPoses()。
var _pose_acc: float = 0.0


# ================================================================ 生命周期

func _ready() -> void:
	_EnsureRefs()
	var heartbeat: Timer = get_node_or_null("Heartbeat") as Timer
	if heartbeat != null and not heartbeat.timeout.is_connected(_OnHeartbeatTimeout):
		heartbeat.timeout.connect(_OnHeartbeatTimeout)
	if autolisten:
		var err: Error = Listen(_ResolvePort(listen_port))
		if err != OK:
			push_warning("[ws] 自动监听失败（port=%d, %s），世界继续运行但不可接入"
				% [_ResolvePort(listen_port), error_string(err)])


func _exit_tree() -> void:
	Stop()


## 每帧：accept 新连接 -> 逐条 poll 并分派 -> 清理已判死的连接 -> 位姿下行。
func _process(delta: float) -> void:
	_AcceptConnections()
	_PollLinks()
	_SweepClosed()
	_BroadcastPoses(delta)


# ================================================================ 监听 / 停止

## 监听端口（幂等：先 Stop()）。port <= 0 -> OS 临时端口（自检用，P5）。
## 失败只 push_warning（**绝不 push_error**：端口被占是环境问题，不该让日志出现 ERROR，R4）。
func Listen(port: int) -> Error:
	Stop()
	var server := TCPServer.new()
	var err: Error = server.listen(port)
	if err != OK:
		push_warning("[ws] listen(port=%d) 失败：%s" % [port, error_string(err)])
		return err
	_tcp_server = server
	_listen_port = server.get_local_port()
	print("[ws] listening on port %d (links=0 vehicles=0)" % _listen_port)
	return OK


## 全停：关全部连接 + 清 5 张表 + 移除全部 avatar + 关监听（幂等）。
func Stop() -> void:
	var links: Array = _links.duplicate()
	for link: Variant in links:
		_CloseLink(link, CLOSE_GOING_AWAY, "manager stopped")
	for peer: Variant in _peer_vehicles.keys():
		_RemoveVehicle(str(peer), null)
	for link: Variant in links:
		_CleanupLink(link)
	_links.clear()
	_manifests.clear()
	_manifest_parts.clear()
	_assembly_sent.clear()
	_pending_sensors.clear()
	if _tcp_server != null:
		_tcp_server.stop()
		_tcp_server = null
	if _listen_port > 0:
		print("[ws] stopped (was port %d)" % _listen_port)
	_listen_port = 0


func GetListenPort() -> int:
	return _listen_port


func IsListening() -> bool:
	return _tcp_server != null and _listen_port > 0


# ================================================================ 查询（只读快照）

## 不存在 / 已释放 = null。
func GetVehicle(peer_id: String) -> Node3D:
	var vehicle: Variant = _peer_vehicles.get(peer_id, null)
	if vehicle == null or not is_instance_valid(vehicle):
		return null
	return vehicle as Node3D


func GetVehicleCount() -> int:
	return _peer_vehicles.size()


## 第二张表的只读快照：每条连接一行（含未握手的；open 字段见 IsOpen）。
func GetLinkInfos() -> Array:
	var out: Array = []
	for link: Variant in _links:
		out.append({
			"conn_id": link.conn_id,
			"peer_id": link.peer_id,
			"role": link.role,
			"slot": str(link.slot),
			"sensor_id": link.sensor_id,
			"open": link.IsOpen(),
			"handshaken": link.handshaken,
		})
	return out


## 诊断快照（自检 / stats_log）：连接数、车数、暂存数、已装配数。
func GetStats() -> Dictionary:
	var pending := 0
	for peer: Variant in _pending_sensors.keys():
		var queue: Array = _pending_sensors[peer]
		pending += queue.size()
	var assembled := 0
	for peer: Variant in _assembly_sent.keys():
		if bool(_assembly_sent[peer]):
			assembled += 1
	return {
		"listening": IsListening(),
		"port": _listen_port,
		"links": _links.size(),
		"vehicles": _peer_vehicles.size(),
		"pending": pending,
		"assembled": assembled,
	}


# ================================================================ 连接接入 / 轮询

func _AcceptConnections() -> void:
	if _tcp_server == null:
		return
	while _tcp_server.is_connection_available():
		var tcp: StreamPeerTCP = _tcp_server.take_connection()
		if tcp == null:
			break
		var link: Variant = ComponentLink.new()
		link.Init(tcp, _next_conn_id)
		_next_conn_id += 1
		_links.append(link)
		if link.closed:
			push_warning("[ws] accept 后握手初始化失败（%s）" % link.last_error)
		else:
			print("[ws] accept conn=%d from %s:%d links=%d" % [
				link.conn_id, tcp.get_connected_host(), tcp.get_connected_port(), _links.size()])


func _PollLinks() -> void:
	for link: Variant in _links.duplicate():
		if link.closed:
			continue
		link.Poll()
		var packets: Array = link.TakePackets()
		for packet: Variant in packets:
			if link.closed:
				break                      # 本连接已被前面的包关掉：剩余包丢弃
			var item: Dictionary = packet
			var bytes: PackedByteArray = item.get("bytes", PackedByteArray())
			var was_string: bool = bool(item.get("was_string", false))
			_HandlePacket(link, bytes, was_string)


# ================================================================ 报文分派

## 首包必须是 *_hello（§8.3-1）；已握手后按 msg 名分派，未知 msg 只 warning（§8.3-6）。
func _HandlePacket(link: Variant, bytes: PackedByteArray, was_string: bool) -> void:
	if not was_string:
		# §8.2：二进制只下行。sim 发上来直接忽略，不断连（别把调试工具踢出去）。
		push_warning("[ws] conn=%d 收到二进制帧，忽略（二进制只下行）" % link.conn_id)
		return
	var msg: Dictionary = WireProtocol.ParseJson(bytes)
	if msg.is_empty():
		push_warning("[ws] conn=%d 非法 JSON / 非对象，关闭 (1002)" % link.conn_id)
		_CloseLink(link, CLOSE_PROTOCOL, "bad json")
		return
	var msg_name := str(msg.get("msg", "")).strip_edges()
	if msg_name.is_empty():
		push_warning("[ws] conn=%d 缺少 msg 字段，关闭 (1002)" % link.conn_id)
		_CloseLink(link, CLOSE_PROTOCOL, "missing msg")
		return

	if not link.handshaken:
		match msg_name:
			WireProtocol.MSG_CHASSIS_HELLO:
				_RegisterChassis(link, msg)
			WireProtocol.MSG_SENSOR_HELLO:
				_RegisterSensor(link, msg)
			_:
				push_warning("[ws] conn=%d 首包是 '%s'，不是 *_hello，关闭 (1002)"
					% [link.conn_id, msg_name])
				_CloseLink(link, CLOSE_PROTOCOL, "first message must be *_hello")
		return

	match msg_name:
		WireProtocol.MSG_MOTION_CMD:
			_ApplyMotionCommand(link, msg)
		WireProtocol.MSG_CHASSIS_HELLO:
			if link.role == ROLE_CHASSIS:
				_RegisterChassis(link, msg)          # 重复 hello = 重建 avatar（§8.3-7）
			else:
				# 一条连接只有一个角色：传感器连接自称底盘属于协议错误，忽略比改角色安全
				push_warning("[ws] conn=%d 传感器连接发来 chassis_hello，忽略（不改变角色）"
					% link.conn_id)
		WireProtocol.MSG_SENSOR_HELLO:
			# 传感器连接一槽一条，不重挂（改槽请断开重连）
			push_warning("[ws] conn=%d 已握手连接重复 sensor_hello（slot=%s），忽略"
				% [link.conn_id, link.slot])
		_:
			push_warning("[ws] conn=%d 未知 msg '%s'，忽略（不断连）" % [link.conn_id, msg_name])


## motion_cmd 只在底盘连接上有意义（数据面绑在底盘连接，§4.5）。
func _ApplyMotionCommand(link: Variant, msg: Dictionary) -> void:
	if link.role != ROLE_CHASSIS:
		push_warning("[ws] conn=%d motion_cmd 出现在 %s 连接，忽略"
			% [link.conn_id, str(link.role)])
		return
	var vehicle: Node3D = link.vehicle
	if vehicle == null or not is_instance_valid(vehicle) \
			or not vehicle.has_method("SetMotionCommand"):
		push_warning("[ws] conn=%d motion_cmd 无可用的 vehicle，忽略" % link.conn_id)
		return
	vehicle.call("SetMotionCommand", msg)


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


# ================================================================ 装配：底盘

## chassis_hello：建 avatar -> 挂底盘 -> 回 chassis_ready -> 回放 pending -> 判就绪。
func _RegisterChassis(link: Variant, msg: Dictionary) -> void:
	var peer_id := str(msg.get("peer_id", "")).strip_edges()
	if peer_id.is_empty():
		push_warning("[ws] conn=%d chassis_hello 缺 peer_id，关闭 (1002)" % link.conn_id)
		_CloseLink(link, CLOSE_PROTOCOL, "chassis_hello missing peer_id")
		return
	var chassis_name := str(msg.get("chassis", "")).strip_edges()
	if chassis_name.is_empty():
		push_warning("[ws] conn=%d chassis_hello 缺 chassis，关闭 (1002)" % link.conn_id)
		_CloseLink(link, CLOSE_PROTOCOL, "chassis_hello missing chassis")
		return
	if not _CatalogHas(CHASSIS_CATALOG_PATH, chassis_name):
		push_warning("[ws] peer='%s' 未知底盘 '%s'（目录外），关闭 (1002）"
			% [peer_id, chassis_name])
		_CloseLink(link, CLOSE_PROTOCOL, "unknown chassis")
		return
	print("[ws] chassis_hello peer='%s' chassis=%s conn=%d" % [peer_id, chassis_name, link.conn_id])
	var chassis_scene: PackedScene = _ResolveCatalogScene(CHASSIS_CATALOG_PATH, chassis_name)
	if chassis_scene == null:
		# 目录里有名字、场景还没落地（如 Step 7 前的 quadrotor_uav）
		push_warning("[ws] peer='%s' 底盘 '%s' 的场景尚未落地，关闭 (1011）"
			% [peer_id, chassis_name])
		_CloseLink(link, CLOSE_INTERNAL, "chassis scene missing")
		return

	# 重复 hello：先拆旧 avatar（**不关这条新 link**，R13）
	if _peer_vehicles.has(peer_id):
		_RemoveVehicle(peer_id, link)

	var vehicle: Node3D = _InstantiateVehicle()
	if vehicle == null:
		push_warning("[ws] peer='%s' avatar 实例化失败，关闭 (1011)" % peer_id)
		_CloseLink(link, CLOSE_INTERNAL, "vehicle scene unavailable")
		return
	if vehicles_root == null or not is_instance_valid(vehicles_root):
		_EnsureRefs()
	if vehicles_root == null or not is_instance_valid(vehicles_root):
		push_warning("[ws] peer='%s' 缺 Vehicles 容器（World.tscn 的 node_paths 检查），关闭 (1011)"
			% peer_id)
		vehicle.queue_free()
		_CloseLink(link, CLOSE_INTERNAL, "vehicles root missing")
		return

	var avatar_id := _next_avatar_id
	# spawn 必须在 _next_avatar_id 自增**之前**取（_PickSpawn 用 idx = _next_avatar_id - 1）
	var spawn := _PickSpawn(chassis_name)
	_next_avatar_id = 1 if _next_avatar_id >= MAX_WIRE_ID else _next_avatar_id + 1

	vehicle.name = _UniqueVehicleName(_SanitizePeerId(peer_id))
	vehicles_root.add_child(vehicle)                     # 先入树，再设位姿（P8/§4.4 P8）
	vehicle.global_position = WorldTransform.PosToGodot(
		Vector3(float(spawn["x"]), float(spawn["y"]), float(spawn["z"])))
	vehicle.rotation.y = WorldTransform.YawToGodot(float(spawn["yaw"]))
	vehicle.set_meta("avatar_id", avatar_id)
	vehicle.set_meta("peer_id", peer_id)

	var params: Variant = msg.get("params", {})
	var chassis_params: Dictionary = params if params is Dictionary else {}
	if not bool(vehicle.call("ApplyChassis", chassis_name, chassis_params, chassis_scene)):
		push_warning("[ws] peer='%s' ApplyChassis('%s') 失败，关闭 (1011)"
			% [peer_id, chassis_name])
		_DiscardVehicle(vehicle)
		_CloseLink(link, CLOSE_INTERNAL, "ApplyChassis failed")
		return

	link.peer_id = peer_id
	link.role = ROLE_CHASSIS
	link.slot = &""
	link.vehicle = vehicle
	link.handshaken = true
	_peer_vehicles[peer_id] = vehicle

	var slots := _ReadManifest(msg)
	var manifest: Dictionary = {}
	for slot: Variant in slots.keys():
		manifest[slot] = false
	_manifests[peer_id] = manifest
	_manifest_parts[peer_id] = slots
	_assembly_sent[peer_id] = false

	link.SendJson({
		"msg": WireProtocol.MSG_CHASSIS_READY,
		"peer_id": peer_id,
		"avatar_id": avatar_id,
		"spawn": spawn,
		"world_time": WorldClock.Now(),
	})
	print("[ws] vehicle spawned peer='%s' chassis=%s avatar=%d slots=%d node=%s spawn=(%.2f, %.2f, %.2f)"
		% [peer_id, chassis_name, avatar_id, manifest.size(), str(vehicle.name),
			float(spawn["x"]), float(spawn["y"]), float(spawn["z"])])
	vehicle_spawned.emit(peer_id, vehicle)

	_ReplayPendingSensors(peer_id)      # 先落 manifest，再回放 pending（顺序不能反）
	_TryAssembly(peer_id)               # manifest 为空数组时这里立刻 assembly_ready


# ================================================================ 装配：传感器

## sensor_hello：记身份 -> 目录校验 -> 无车则暂存 -> 有车则挂载。
func _RegisterSensor(link: Variant, msg: Dictionary) -> void:
	var peer_id := str(msg.get("peer_id", "")).strip_edges()
	var slot := _SanitizeSlot(str(msg.get("slot", "")))
	var sensor_name := str(msg.get("sensor", "")).strip_edges()
	if peer_id.is_empty() or slot.is_empty() or sensor_name.is_empty():
		push_warning("[ws] conn=%d sensor_hello 缺字段（peer_id/slot/sensor），关闭 (1002)"
			% link.conn_id)
		_CloseLink(link, CLOSE_PROTOCOL, "sensor_hello missing fields")
		return
	link.peer_id = peer_id
	link.role = ROLE_SENSOR
	link.slot = StringName(slot)
	link.handshaken = true
	var catalog_status := _CatalogStatus(SENSOR_CATALOG_PATH, sensor_name)
	if catalog_status == CATALOG_UNKNOWN:
		# §8.3-4：未知 sensor -> 忽略该 slot（不回 ready、不置位、**保持连接**）
		push_warning("[ws] peer='%s' slot='%s' 未知 sensor '%s'，忽略该 slot（连接保持）"
			% [peer_id, slot, sensor_name])
		return
	if catalog_status == CATALOG_MISSING and not _warned_missing_sensor_catalog:
		# Step 8 前 SensorCatalog 不存在：没有可校验的名字表，只能按"已知"登记连接
		_warned_missing_sensor_catalog = true
		push_warning("[ws] SensorCatalog 尚未落地（Step 8），传感器名不做目录校验，只登记连接")
	if not _peer_vehicles.has(peer_id):
		# §5.1-3：传感器可能先于底盘到达 -> 暂存（pending 期间**不发** sensor_ready，R9）
		var queue: Array = _pending_sensors.get(peer_id, [])
		queue.append({"link": link, "hello": msg.duplicate(true), "since_ms": Time.get_ticks_msec()})
		_pending_sensors[peer_id] = queue
		print("[ws] sensor pending peer='%s' slot=%s sensor=%s（等底盘，pending=%d）"
			% [peer_id, slot, sensor_name, queue.size()])
		return
	_MountSensor(link, msg)


## 把一条传感器连接挂到已有 avatar 上：先 sensor_ready，再 SetStream（§5.3 时序铁律）。
## 返回 true = 连接已处理（含"场景未落地但已登记"）；false = 没有可用 vehicle。
func _MountSensor(link: Variant, hello: Dictionary) -> bool:
	var peer_id: String = link.peer_id
	var slot := _SanitizeSlot(str(link.slot))
	var vehicle: Node3D = GetVehicle(peer_id)
	if vehicle == null or slot.is_empty():
		return false
	var sensor_name := str(hello.get("sensor", "")).strip_edges()
	var mount: Variant = hello.get("mount", {})
	var params: Variant = hello.get("params", {})
	var sensor_scene: PackedScene = _ResolveCatalogScene(SENSOR_CATALOG_PATH, sensor_name)
	var node: Node = null
	if sensor_scene != null:
		node = vehicle.call("AttachSensor", slot, sensor_scene,
			mount if mount is Dictionary else {}, params if params is Dictionary else {})
	else:
		# Step 8 前 Lidar2D.tscn 不存在：只登记连接（就绪语义 = 连接就位，见文件头）
		if not _warned_missing_sensor:
			push_warning("[ws] 传感器场景 '%s' 尚未落地（Step 8），slot 仅登记连接、不实例化几何"
				% sensor_name)
			_warned_missing_sensor = true
	link.sensor_id = _TakeNextSensorId()
	link.vehicle = vehicle
	link.SendJson({
		"msg": WireProtocol.MSG_SENSOR_READY,
		"slot": slot,
		"sensor_id": link.sensor_id,
	})
	if node != null and is_instance_valid(node) and node.has_method("SetStream"):
		node.call("SetStream", link, link.sensor_id)   # 顺序写死：先 ready 再绑流（R10）
	var counted := false
	if _manifests.has(peer_id):
		var table: Dictionary = _manifests[peer_id]
		if table.has(slot):
			table[slot] = true
			counted = true
	if not counted:
		push_warning("[ws] peer='%s' slot='%s' 不在 manifest，已挂但不算就绪" % [peer_id, slot])
	var parts: Variant = _manifest_parts.get(peer_id, null)
	if parts is Dictionary:
		var parts_dict: Dictionary = parts
		if parts_dict.has(slot) and str(parts_dict[slot]) != sensor_name:
			push_warning("[ws] peer='%s' slot='%s' 的 sensor 与 manifest 不一致（manifest=%s hello=%s），以 slot 为准"
				% [peer_id, slot, str(parts_dict[slot]), sensor_name])
	print("[ws] sensor_ready peer='%s' slot=%s sensor=%s sensor_id=%d geometry=%s"
		% [peer_id, slot, sensor_name, link.sensor_id, node != null])
	_TryAssembly(peer_id)
	return true


## 底盘到达后回放暂存的传感器连接（挂载会各自推进 manifest）。
func _ReplayPendingSensors(peer_id: String) -> void:
	if not _pending_sensors.has(peer_id):
		return
	var queue: Array = _pending_sensors[peer_id]
	_pending_sensors.erase(peer_id)
	for item: Variant in queue:
		var entry: Dictionary = item
		var link: Variant = entry.get("link", null)
		if link == null or bool(link.closed):
			continue
		var hello: Variant = entry.get("hello", {})
		_MountSensor(link, hello if hello is Dictionary else {})


# ================================================================ 就绪判定

## manifest 全部就位 -> 给底盘连接发 assembly_ready（每 peer 仅一次，§8.1-4）。
func _TryAssembly(peer_id: String) -> void:
	if GetVehicle(peer_id) == null:
		return
	if not _manifests.has(peer_id):
		return
	if bool(_assembly_sent.get(peer_id, false)):
		return
	var table: Dictionary = _manifests[peer_id]
	for slot: Variant in table.keys():
		if not bool(table[slot]):
			return
	var chassis_link: Variant = _FindLink(peer_id, ROLE_CHASSIS)
	if chassis_link == null or not chassis_link.IsOpen():
		return
	var err: Error = chassis_link.SendJson({
		"msg": WireProtocol.MSG_ASSEMBLY_READY,
		"peer_id": peer_id,
		"world_time": WorldClock.Now(),
	})
	_assembly_sent[peer_id] = true          # 发过就不再发（即使发送失败也不重试，§8.1-4）
	assembly_completed.emit(peer_id)
	if err != OK:
		push_warning("[ws] peer='%s' assembly_ready 发送失败：%s" % [peer_id, error_string(err)])
	print("[ws] assembly_ready peer='%s' slots=%d" % [peer_id, table.size()])


# ================================================================ 清理

## 移除一台 avatar：先从表里摘（R8），再 free；同 peer 的其余连接一并关闭。
## except_link = 刚接手的**新**连接（重复 hello 时绝不能把它关掉，R13）。
func _RemoveVehicle(peer_id: String, except_link: Variant = null) -> void:
	var vehicle: Variant = _peer_vehicles.get(peer_id, null)
	_peer_vehicles.erase(peer_id)           # 先摘表，再 free（queue_free 后同帧仍 is_instance_valid）
	if vehicle != null and is_instance_valid(vehicle):
		var parent: Node = vehicle.get_parent()
		if parent != null:
			parent.remove_child(vehicle)
		vehicle.queue_free()
	for link: Variant in _links.duplicate():
		if link == except_link:
			continue
		if link.peer_id == peer_id:
			_CloseLink(link, CLOSE_GOING_AWAY, "peer removed")
	_manifests.erase(peer_id)
	_manifest_parts.erase(peer_id)
	_assembly_sent.erase(peer_id)
	_pending_sensors.erase(peer_id)
	vehicle_removed.emit(peer_id)
	print("[ws] vehicle removed peer='%s'" % peer_id)


## 清理一条连接的业务痕迹（由 _SweepClosed / Stop 调用；**不**从 _links 删除）。
func _CleanupLink(link: Variant) -> void:
	if link == null:
		return
	var peer_id: String = link.peer_id
	var role: StringName = link.role
	var slot := str(link.slot)
	if link.handshaken:
		if role == ROLE_CHASSIS:
			_RemoveVehicle(peer_id, null)   # 底盘断开 = 整台车走了（顺手关该 peer 其余连接）
		elif role == ROLE_SENSOR:
			var vehicle: Node3D = GetVehicle(peer_id)
			if vehicle != null and vehicle.has_method("DetachSensor"):
				vehicle.call("DetachSensor", slot)
			if _manifests.has(peer_id):
				var table: Dictionary = _manifests[peer_id]
				if table.has(slot):
					table[slot] = false
			_DropPending(peer_id, link)
	link.closed = true
	link_closed.emit(peer_id, role, slot)
	print("[ws] link closed conn=%d peer='%s' role=%s slot=%s (%s)"
		% [link.conn_id, peer_id, str(role), slot, link.last_error])


## 移除已判死的连接（统一出口；遍历副本 + 只在这里 erase，R7/R6）。
func _SweepClosed() -> void:
	for link: Variant in _links.duplicate():
		if not bool(link.closed):
			continue
		_CleanupLink(link)
		_links.erase(link)


## 从 pending 队列里摘掉一条连接（连接断开时）；空了就删键。
func _DropPending(peer_id: String, link: Variant) -> void:
	if not _pending_sensors.has(peer_id):
		return
	var queue: Array = _pending_sensors[peer_id]
	var kept: Array = []
	for item: Variant in queue:
		var entry: Dictionary = item
		if entry.get("link", null) != link:
			kept.append(entry)
	if kept.is_empty():
		_pending_sensors.erase(peer_id)
	else:
		_pending_sensors[peer_id] = kept


func _CloseLink(link: Variant, code: int, reason: String) -> void:
	if link == null:
		return
	if link.IsOpen() or link.IsConnecting():
		link.Close(code, reason)            # E19：Close() 返回 void
	link.closed = true
	if link.last_error.is_empty():
		link.last_error = "close %d '%s'" % [code, reason]


# ================================================================ Heartbeat 巡检

## 每秒一次：握手超时 / pending 超时 / 可选统计日志（墙钟，不用帧数，R20）。
func _OnHeartbeatTimeout() -> void:
	var now := Time.get_ticks_msec()
	for link: Variant in _links.duplicate():
		if link.closed or link.handshaken:
			continue
		if now - int(link.created_ms) > handshake_timeout_ms:
			push_warning("[ws] conn=%d 握手超时（%d ms），关闭 (1002）"
				% [link.conn_id, handshake_timeout_ms])
			_CloseLink(link, CLOSE_PROTOCOL, "hello timeout")
	for peer: Variant in _pending_sensors.keys():
		var queue: Array = _pending_sensors[peer]
		for item: Variant in queue:
			var entry: Dictionary = item
			var link: Variant = entry.get("link", null)
			if link == null or bool(link.closed):
				continue
			if now - int(entry.get("since_ms", now)) > pending_timeout_ms:
				push_warning("[ws] peer='%s' 传感器等待底盘超时（%d ms），关闭 (1008）"
					% [str(peer), pending_timeout_ms])
				_CloseLink(link, CLOSE_POLICY, "pending timeout")
	if stats_log:
		print("[ws] stats ", GetStats())


# ================================================================ 内部工具：引用 / 端口

## E16/E20 兜底：node_paths 漏写 / NodePath 写错时 @export 会静默 null，这里按拓扑回退一次。
func _EnsureRefs() -> void:
	if vehicles_root == null:
		var node: Node = self
		while node != null:
			var found: Node3D = node.get_node_or_null("Vehicles") as Node3D
			if found != null:
				vehicles_root = found
				break
			node = node.get_parent()
	if vehicles_root == null and not _warned_no_root:
		_warned_no_root = true
		push_warning("[ws] 找不到 Vehicles 容器（检查 World.tscn 的 node_paths / NodePath）")
	if vehicle_scene == null:
		vehicle_scene = load(VEHICLE_SCENE_PATH) as PackedScene


## 0 / 负 -> 读 [sim_world] network/ws_port；缺省 9999。
func _ResolvePort(port: int) -> int:
	if port > 0:
		return port
	var raw: Variant = ProjectSettings.get_setting(SETTING_WS_PORT, DEFAULT_WS_PORT)
	var resolved := int(raw) if raw is int or raw is float else 0
	if resolved <= 0:
		push_warning("[ws] %s 非法（%s），回退 %d" % [SETTING_WS_PORT, str(raw), DEFAULT_WS_PORT])
		return DEFAULT_WS_PORT
	return resolved


# ================================================================ 内部工具：目录 / 场景

## 目录脚本（不存在 -> null，不 load、不报错）。
func _CatalogScript(path: String) -> Script:
	if not ResourceLoader.exists(path):
		return null
	return load(path) as Script


func _CatalogHas(catalog_path: String, name: String) -> bool:
	return _CatalogStatus(catalog_path, name) == CATALOG_KNOWN


## 三态查询：目录没落地 **不等于** 名字非法（Step 6 早于 Step 8，SensorCatalog 尚不存在）。
func _CatalogStatus(catalog_path: String, name: String) -> int:
	var catalog: Script = _CatalogScript(catalog_path)
	if catalog == null:
		return CATALOG_MISSING
	if bool(catalog.call("Has", name)):
		return CATALOG_KNOWN
	return CATALOG_UNKNOWN


## 目录里登记了、但场景文件还没落地（如 Step 7 前的 quadrotor_uav / Step 8 前的 lidar_2d）。
## **禁止 preload**（§7.6/§11.1）：先看目录的 SCENES 表再 ResourceLoader.exists，
## 避免让目录自己的 GetScene() push_error（日志必须干净）。
func _CatalogSceneExists(catalog: Script, name: String) -> bool:
	var consts: Dictionary = catalog.get_script_constant_map()
	var scenes: Variant = consts.get("SCENES", null)
	if not (scenes is Dictionary):
		return true                        # 目录没有 SCENES 表 -> 交给它自己判断
	var table: Dictionary = scenes
	if not table.has(name):
		return true
	return ResourceLoader.exists(str(table[name]))


## 名字 -> 场景（目录外 / 场景缺失 / 加载失败 -> null；不 push_error）。
func _ResolveCatalogScene(catalog_path: String, name: String) -> PackedScene:
	var catalog: Script = _CatalogScript(catalog_path)
	if catalog == null:
		return null
	if not bool(catalog.call("Has", name)):
		return null
	if not _CatalogSceneExists(catalog, name):
		return null
	var scene: Variant = catalog.call("GetScene", name)
	if scene is PackedScene:
		return scene as PackedScene
	return null


func _InstantiateVehicle() -> Node3D:
	var scene: PackedScene = vehicle_scene
	if scene == null:
		scene = load(VEHICLE_SCENE_PATH) as PackedScene
	if scene == null:
		return null
	var node: Node = scene.instantiate()
	var vehicle: Node3D = node as Node3D
	if vehicle == null or not vehicle.has_method("ApplyChassis") \
			or not vehicle.has_method("AttachSensor"):
		if node != null:
			node.queue_free()
		return null
	return vehicle


## 丢弃一台刚建好但装配失败的空车（可能已入树，也可能还没有）。
func _DiscardVehicle(vehicle: Node3D) -> void:
	if vehicle == null or not is_instance_valid(vehicle):
		return
	var parent: Node = vehicle.get_parent()
	if parent != null:
		parent.remove_child(vehicle)
	vehicle.queue_free()


# ================================================================ 内部工具：名字 / 编号 / spawn

## peer_id -> 合法节点名（非法字符替换为 _，COMPONENTS §5.2 用 validate_node_name()）。
func _SanitizePeerId(peer_id: String) -> String:
	var key := peer_id.strip_edges().validate_node_name()
	if key.is_empty():
		key = "peer"
	return key


func _SanitizeSlot(slot: String) -> String:
	return slot.strip_edges().validate_node_name()


## 重名 -> base_<avatar_id>（避免 add_child 把节点自动改名成 "simA2" 导致名字不可预期，R14）。
func _UniqueVehicleName(base: String) -> String:
	var name_taken: bool = false
	if vehicles_root != null and is_instance_valid(vehicles_root):
		name_taken = vehicles_root.get_node_or_null(NodePath(base)) != null
	if not name_taken:
		return base
	return "%s_%d" % [base, _next_avatar_id]


func _TakeNextSensorId() -> int:
	var id := _next_sensor_id
	_next_sensor_id = 1 if _next_sensor_id >= MAX_WIRE_ID else _next_sensor_id + 1
	return id


## 确定性出生点（无 RNG）：第 1 台 = World.SPAWN_WORLD；之后按 ring 错开（§7.2 ring）。
## UAV 抬高到 2.5 m（§7.1 巡航高度，task_32 Step 3 由 2 m 提高以越过 2 m 可见矮墙），UGV = 0。
func _PickSpawn(chassis_name: String) -> Dictionary:
	var idx := _next_avatar_id - 1
	var offset := Vector3.ZERO
	if idx > 0:
		var ring := floori(float(idx - 1) / float(SPAWN_RING_SLOTS)) + 1
		var ring_slot := posmod(idx - 1, SPAWN_RING_SLOTS)
		var angle := TAU * float(ring_slot) / float(SPAWN_RING_SLOTS)
		offset = Vector3(cos(angle), sin(angle), 0.0) * SPAWN_RING_STEP_M * float(ring)
	var origin: Vector3 = World.SPAWN_WORLD
	var z := UAV_ALTITUDE_M if chassis_name == UAV_CHASSIS else 0.0
	return {
		"x": origin.x + offset.x,
		"y": origin.y + offset.y,
		"z": z,
		"yaw": 0.0,
	}


## manifest 数组 -> {slot: sensor}（去重、丢弃非法项；保持声明顺序）。
func _ReadManifest(msg: Dictionary) -> Dictionary:
	var out: Dictionary = {}
	var raw: Variant = msg.get("manifest", [])
	if not (raw is Array):
		return out
	for item: Variant in raw:
		if not (item is Dictionary):
			continue
		var entry: Dictionary = item
		var slot := _SanitizeSlot(str(entry.get("slot", "")))
		if slot.is_empty():
			continue
		out[slot] = str(entry.get("sensor", "")).strip_edges()
	return out


## 该 peer 的某角色连接中第一条还活着的（用于发 assembly_ready）。
func _FindLink(peer_id: String, role: StringName) -> Variant:
	for link: Variant in _links:
		if link.peer_id == peer_id and link.role == role and not bool(link.closed):
			return link
	return null
