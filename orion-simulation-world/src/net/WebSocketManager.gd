# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-25
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
# task_35 Step 2（行人）：chassis_hello.kind = "pedestrian" 走**另一条支路** ——
#   `Pedestrian.tscn` 根节点**就是** avatar（不套 Vehicle.tscn），进 `Pedestrians` 容器
#   （与 `Vehicles` 平级、同父），**不进** `_peer_vehicles`、**不占** `_PickSpawn` 的 avatar 排位、
#   不消耗 `_next_avatar_id`。行人有自己的一套表与编号（`_pedestrians` / `_next_pedestrian_id`）。
#   指令面 `motion_cmd{v, dir}`；**300 ms 没新指令就自动 v=0**（只停、不断连、不刷屏）。
#   车**没有**超时（既有 D1/G 的语义一字不变）。
#
# 第 2 轮（prev_issues #2）：换 kind（同一 peer_id 先后 robot / pedestrian）按"重建 avatar"处理：
#   注册前**不分类型**先 `_RemoveAvatar(peer_id, except_link=本连接)`（两张表 + 两个容器一起清），
#   旧连接被关且交出 `link.vehicle`；被关连接的 `_CleanupLink()` 不得再动新 avatar；
#   `_ApplyMotionCommand()` 只按 `link.vehicle` 的身份分流（不看"表里有没有这个 peer"）。
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
## 一个行人 avatar 建好并挂进 `Pedestrians`（与 vehicle_spawned 平级、互不覆盖；task_35 Step 2）。
signal pedestrian_spawned(peer_id: String, pedestrian: Node3D)
## 一个行人 avatar 被移除（连接断开 / 重复 hello / Stop）。
signal pedestrian_removed(peer_id: String)
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

# ---------------------------------------------------------------- 行人（task_35 Step 2）
## 行人（task_35 Step 2）：chassis_hello.kind == "pedestrian" 的 avatar 走这条支路。
const PEDESTRIAN_CHASSIS := "pedestrian"
## 行人容器名：与 Vehicles 平级、同父（World 根）。**World.tscn 不绑**，首个行人到达时惰性建。
const PEDESTRIANS_NODE := "Pedestrians"
## 默认出生点偏移（世界坐标米）：spawn 缺失/非法时的回退点 = World.SPAWN_WORLD + 该偏移。
## 为什么是正南 8 m：车 avatar 的环是「8 方位 × 半径 2·ring」，270° 半径 8 处的点是 (64,56)
##   —— 那要第 31 台车才会出现；且它已在 SPAWN_CLEAR_RADIUS_M(3 m) 之外、与 15 个
##   World.OBSTACLES 全无交、离 PERSONS(60,76)/(68,70) 与 WALKERS 环路(y≥96) 都很远。
const PEDESTRIAN_SPAWN_OFFSET_M := Vector3(0.0, -8.0, 0.0)
## spawn 校验：与已占 avatar 的最小水平中心距（米）。
const PEDESTRIAN_SPAWN_MIN_GAP_M := 1.0
## spawn 校验：静态障碍外扩余量（米）≈ 胶囊半径 0.25 + 0.05。
const PEDESTRIAN_SPAWN_MARGIN_M := 0.3
## spawn 校验：高度上限（米）—— 行人默认站地面，允许少量抬高，不做"空中出生"。
const PEDESTRIAN_SPAWN_MAX_Z_M := 2.0
## 指令超时（毫秒）：超过这么久没收到新 motion_cmd -> 下发 v=0（只停、不断连、不刷屏）。
const PEDESTRIAN_CMD_TIMEOUT_MS := 300


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
## 行人容器（task_35 Step 2）：**可留空**（World.tscn 不绑）。留空时按"与 vehicles_root
## 同父的 Pedestrians"惰性解析/建（与 Bounds / Persons / Walkers 的"代码建、幂等"策略一致）。
@export var pedestrians_root: Node3D = null


## 目录查询结果：区分「目录里没登记这个名字」与「目录脚本还没落地」（§8.3-4 只看前者）。
const CATALOG_UNKNOWN := 0     ## 目录存在，但未登记该名字 -> 拒绝 / 忽略该 slot
const CATALOG_KNOWN := 1       ## 目录里登记了 -> 继续解析场景
const CATALOG_MISSING := 2     ## 目录脚本本身还没落地（SensorCatalog 到 Step 8 才有）


# ---------------------------------------------------------------- 主表 1：peer_id -> vehicle
var _peer_vehicles: Dictionary = {}
# ---------------------------------------------------------------- 主表 1b：peer_id -> Pedestrian（task_35 Step 2）
## 行人**不进** `_peer_vehicles`（`GetVehicleCount()` / `GetVehicle()` / `_PickSpawn` /
## `Vehicles` 容器四条既有语义看不到人）。
var _pedestrians: Dictionary = {}
## peer_id -> 最近一次 motion_cmd 的墙钟毫秒（超时停车用；墙钟，不用帧数）。
var _ped_last_motion_ms: Dictionary = {}
## peer_id -> bool（已经下过 v=0；避免逐帧重复下发 / 重复打日志）。
var _ped_idle: Dictionary = {}
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
## u16 语义的行人编号（与 _next_avatar_id **完全独立**：人**不占**车的排位）。
var _next_pedestrian_id: int = 1
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


## 每帧：accept 新连接 -> 逐条 poll 并分派 -> 清理已判死的连接 -> 行人超时停车 -> 位姿下行。
func _process(delta: float) -> void:
	_AcceptConnections()
	_PollLinks()
	_SweepClosed()
	_Tick_Pedestrian_Timeouts()          # task_35 Step 2：只对行人生效，车没有超时
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


## 全停：关全部连接 + 清全部表 + 移除全部 avatar + 关监听（幂等）。
func Stop() -> void:
	var links: Array = _links.duplicate()
	for link: Variant in links:
		_CloseLink(link, CLOSE_GOING_AWAY, "manager stopped")
	var avatars: Array = _peer_vehicles.keys() + _pedestrians.keys()
	for peer: Variant in avatars:
		_RemoveAvatar(str(peer), null)
	for link: Variant in links:
		_CleanupLink(link)
	_links.clear()
	_manifests.clear()
	_manifest_parts.clear()
	_assembly_sent.clear()
	_pending_sensors.clear()
	_pedestrians.clear()
	_ped_last_motion_ms.clear()
	_ped_idle.clear()
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


## 行人 avatar（task_35 Step 2）。不存在 / 已释放 = null。
func GetPedestrian(peer_id: String) -> Node3D:
	var pedestrian: Variant = _pedestrians.get(peer_id, null)
	if pedestrian == null or not is_instance_valid(pedestrian):
		return null
	return pedestrian as Node3D


func GetPedestrianCount() -> int:
	return _pedestrians.size()


## 这个 peer 是行人吗（车返回 false；不存在的 peer 也 false）。
func IsPedestrian(peer_id: String) -> bool:
	return GetPedestrian(peer_id) != null


## 行人容器（惰性解析 / 建；没有 Vehicles 容器时返回 null）。只读查询，不改变已有节点的父子关系。
func GetPedestriansRoot() -> Node3D:
	return _Pedestrians_Root()


## 统一取 avatar：先查车、再查人（车/人两张表**互不相交**，顺序不影响结果）。
func GetAvatar(peer_id: String) -> Node3D:
	var vehicle := GetVehicle(peer_id)
	if vehicle != null:
		return vehicle
	return GetPedestrian(peer_id)


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
		"pedestrians": _pedestrians.size(),
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
## 行人（task_35 Step 2）走**另一条支路**：v/dir 语义 + 记录 last_motion_at；车的分支一字未改。
func _ApplyMotionCommand(link: Variant, msg: Dictionary) -> void:
	if link.role != ROLE_CHASSIS:
		push_warning("[ws] conn=%d motion_cmd 出现在 %s 连接，忽略"
			% [link.conn_id, str(link.role)])
		return
	var peer_key := str(link.peer_id)
	# 第 2 轮：**不用** `_pedestrians.has(peer_key)` 判 —— 表里有人不代表这条连接就是人
	# （换 kind 的过渡帧里，旧连接可能还活着）。唯一可信的判据是"这条连接的 avatar 是不是
	# 人表里那个节点"，与 pose 下行、清理共用同一个字段（`link.vehicle`）。
	var pedestrian: Node3D = GetPedestrian(peer_key)
	if pedestrian != null and link.vehicle == pedestrian:
		_Apply_Pedestrian_Command(link, pedestrian, msg)
		return
	var vehicle: Node3D = link.vehicle
	if vehicle == null or not is_instance_valid(vehicle) \
			or not vehicle.has_method("SetMotionCommand"):
		push_warning("[ws] conn=%d motion_cmd 无可用的 vehicle，忽略" % link.conn_id)
		return
	vehicle.call("SetMotionCommand", msg)


## 行人指令：v / dir 由 `Pedestrian.SetCommand` 解析（也接受 action/speed 折算）。
## 每收到一条就刷新时间戳并把 idle 置回 false（**恢复时不打日志**，避免 20~50 Hz 抖动刷屏）。
func _Apply_Pedestrian_Command(link: Variant, ped: Node3D, msg: Dictionary) -> void:
	if ped == null or not is_instance_valid(ped) or not ped.has_method("SetCommand"):
		push_warning("[ws] conn=%d motion_cmd 无可用的 pedestrian，忽略" % link.conn_id)
		return
	var peer_key := str(link.peer_id)
	_ped_last_motion_ms[peer_key] = Time.get_ticks_msec()
	_ped_idle[peer_key] = false
	ped.call("SetCommand", msg)


## 行人超时停车（task_35 Step 2）：超过 PEDESTRIAN_CMD_TIMEOUT_MS 没有新指令 -> 下发 v=0。
## **只停**：不断连、不改朝向（不带 dir）、不刷日志（_ped_idle 边沿触发，每段"动->停"只一行）。
## 只遍历 `_pedestrians`：**车没有超时**（既有 D1/D2、check_pose C 组行为一字不变）。
func _Tick_Pedestrian_Timeouts() -> void:
	if _pedestrians.is_empty():
		return
	var now := Time.get_ticks_msec()
	for peer: Variant in _pedestrians.keys():
		var key := str(peer)
		var ped: Variant = _pedestrians.get(peer, null)
		if ped == null or not is_instance_valid(ped):
			continue
		if bool(_ped_idle.get(key, true)):
			continue                                   # 已经停着：什么都不做
		if now - int(_ped_last_motion_ms.get(key, now)) < PEDESTRIAN_CMD_TIMEOUT_MS:
			continue
		_ped_idle[key] = true
		ped.call("SetCommand", WireProtocol.BuildMotionCommand(0.0))
		print("[ws] pedestrian idle peer='%s'（%d ms 无新指令）-> v=0"
			% [key, PEDESTRIAN_CMD_TIMEOUT_MS])


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
	# task_35 Step 2：kind 缺省 = robot（老 sim 一字不改也走老路）；非法值 warn 后按 robot。
	var kind := _Kind_Of(msg, peer_id)
	if kind == WireProtocol.KIND_PEDESTRIAN:
		if chassis_name != PEDESTRIAN_CHASSIS:
			push_warning("[ws] peer='%s' kind=%s 与 chassis='%s' 不匹配（行人应为 '%s'），按 robot 处理"
				% [peer_id, kind, chassis_name, PEDESTRIAN_CHASSIS])
			kind = WireProtocol.KIND_ROBOT
		else:
			_Register_Pedestrian(link, msg, peer_id)
			return
	var chassis_scene: PackedScene = _ResolveCatalogScene(CHASSIS_CATALOG_PATH, chassis_name)
	if chassis_scene == null:
		# 目录里有名字、场景还没落地（如 Step 7 前的 quadrotor_uav）
		push_warning("[ws] peer='%s' 底盘 '%s' 的场景尚未落地，关闭 (1011）"
			% [peer_id, chassis_name])
		_CloseLink(link, CLOSE_INTERNAL, "chassis scene missing")
		return

	# 重复 hello / 换 kind：先拆旧 avatar（**不关这条新 link**，R13）。
	# 第 2 轮修正（prev_issues #2）：**不分类型**地先摘 —— 车换人、人换车都要清场，
	# 否则旧 avatar 留在另一个容器（Vehicles / Pedestrians）里，pose 与 motion_cmd 会被误路由。
	# `_RemoveAvatar()` 自己判「两张表里有没有」：都没有就什么都不做（**不 print、不 emit、
	# 不牵连其它连接**），所以这里无条件调用是安全的 —— 也正因为如此，
	# 「传感器先于底盘」（E 组 pending）不会被误关。
	_RemoveAvatar(peer_id, link)

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

	vehicle.name = _UniqueAvatarName(_SanitizePeerId(peer_id), vehicles_root, avatar_id)
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
		"kind": WireProtocol.KIND_ROBOT,          # task_35 Step 2：只增不改（老消费方忽略新键）
		"spawn": spawn,
		"world_time": WorldClock.Now(),
	})
	print("[ws] vehicle spawned peer='%s' chassis=%s avatar=%d slots=%d node=%s spawn=(%.2f, %.2f, %.2f)"
		% [peer_id, chassis_name, avatar_id, manifest.size(), str(vehicle.name),
			float(spawn["x"]), float(spawn["y"]), float(spawn["z"])])
	vehicle_spawned.emit(peer_id, vehicle)

	_ReplayPendingSensors(peer_id)      # 先落 manifest，再回放 pending（顺序不能反）
	_TryAssembly(peer_id)               # manifest 为空数组时这里立刻 assembly_ready


# ================================================================ 装配：行人（task_35 Step 2）

## chassis_hello{kind:"pedestrian", spawn?}：`Pedestrian.tscn` **根节点就是 avatar**。
## 顺序是硬约束（顺序错了会 SCRIPT ERROR 或飘位）：校验 scene -> 取容器 -> 摘旧 -> 实例化 ->
## 入树 -> 摆位 -> Configure（Configure 读 rotation.y 当初始朝向，必须在摆位**之后**）-> 记账 -> 回执。
## **不碰** `_next_avatar_id` / `_peer_vehicles` / `Vehicles`：人**不占**车的排位（task_35 §一）。
func _Register_Pedestrian(link: Variant, msg: Dictionary, peer_id: String) -> void:
	var chassis_scene: PackedScene = _ResolveCatalogScene(CHASSIS_CATALOG_PATH, PEDESTRIAN_CHASSIS)
	if chassis_scene == null:
		push_warning("[ws] peer='%s' 行人场景尚未落地（chassis='%s'），关闭 (1011)"
			% [peer_id, PEDESTRIAN_CHASSIS])
		_CloseLink(link, CLOSE_INTERNAL, "pedestrian scene missing")
		return
	var root: Node3D = _Pedestrians_Root()
	if root == null:
		push_warning("[ws] peer='%s' 缺 Pedestrians 容器（连 Vehicles 容器也没有），关闭 (1011)"
			% peer_id)
		_CloseLink(link, CLOSE_INTERNAL, "pedestrians root missing")
		return
	# 同上：**不分类型**先摘（robot -> pedestrian 时旧车必须从 Vehicles 消失、旧连接必须被关）
	_RemoveAvatar(peer_id, link)

	var node: Node = chassis_scene.instantiate()
	var pedestrian: Node3D = node as Node3D
	if pedestrian == null or not pedestrian.has_method("SetCommand") \
			or not pedestrian.has_method("GetPose"):
		push_warning("[ws] peer='%s' 行人场景缺 SetCommand/GetPose 契约，关闭 (1011)" % peer_id)
		if node != null and is_instance_valid(node):
			node.queue_free()
		_CloseLink(link, CLOSE_INTERNAL, "pedestrian contract missing")
		return

	var pedestrian_id := _next_pedestrian_id
	_next_pedestrian_id = 1 if _next_pedestrian_id >= MAX_WIRE_ID else _next_pedestrian_id + 1
	var spawn := _Pick_Pedestrian_Spawn(msg, peer_id)

	pedestrian.name = _UniqueAvatarName(_SanitizePeerId(peer_id), root, pedestrian_id)
	root.add_child(pedestrian)                          # 先入树，再设 global_*（P8）
	pedestrian.global_position = WorldTransform.PosToGodot(
		Vector3(float(spawn["x"]), float(spawn["y"]), float(spawn["z"])))
	pedestrian.rotation.y = WorldTransform.YawToGodot(float(spawn["yaw"]))

	var params: Variant = msg.get("params", {})
	var pedestrian_params: Dictionary = params if params is Dictionary else {}
	# 宿主 = 自己（行人**自推进**，不经 Vehicle）；必须在摆位之后。
	pedestrian.call("Configure", pedestrian, _Merge_Avatar_Params(pedestrian, pedestrian_params))
	pedestrian.set_meta("pedestrian_id", pedestrian_id)
	pedestrian.set_meta("peer_id", peer_id)

	link.peer_id = peer_id
	link.role = ROLE_CHASSIS
	link.slot = &""
	link.vehicle = pedestrian                          # pose 下行 / 清理共用这一个字段
	link.handshaken = true
	_pedestrians[peer_id] = pedestrian
	_ped_last_motion_ms[peer_id] = Time.get_ticks_msec()
	_ped_idle[peer_id] = true                          # 还没发过指令：不要立刻"停"一次

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
		"kind": WireProtocol.KIND_PEDESTRIAN,
		"pedestrian_id": pedestrian_id,
		"spawn": spawn,
		"world_time": WorldClock.Now(),
	})
	print("[ws] pedestrian spawned peer='%s' id=%d node=%s spawn=(%.2f, %.2f) yaw=%.2f"
		% [peer_id, pedestrian_id, str(pedestrian.name),
			float(spawn["x"]), float(spawn["y"]), float(spawn["yaw"])])
	pedestrian_spawned.emit(peer_id, pedestrian)

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
	if not _HasAvatar(peer_id):
		# §5.1-3：传感器可能先于底盘到达 -> 暂存（pending 期间**不发** sensor_ready，R9）
		# task_35 Step 2：行人也是 avatar（pedestrian 的 manifest 同样要能就位）
		var queue: Array = _pending_sensors.get(peer_id, [])
		queue.append({"link": link, "hello": msg.duplicate(true), "since_ms": Time.get_ticks_msec()})
		_pending_sensors[peer_id] = queue
		print("[ws] sensor pending peer='%s' slot=%s sensor=%s（等底盘，pending=%d）"
			% [peer_id, slot, sensor_name, queue.size()])
		return
	_MountSensor(link, msg)


## 把一条传感器连接挂到已有 avatar 上：先 sensor_ready，再 SetStream（§5.3 时序铁律）。
## 返回 true = 连接已处理（含"场景未落地但已登记"）；false = 没有可用 avatar。
func _MountSensor(link: Variant, hello: Dictionary) -> bool:
	var peer_id: String = link.peer_id
	var slot := _SanitizeSlot(str(link.slot))
	var vehicle: Node3D = GetAvatar(peer_id)      # 车 / 行人共用（task_35 Step 2）
	if vehicle == null or slot.is_empty():
		return false
	var sensor_name := str(hello.get("sensor", "")).strip_edges()
	var mount: Variant = hello.get("mount", {})
	var params: Variant = hello.get("params", {})
	var sensor_scene: PackedScene = _ResolveCatalogScene(SENSOR_CATALOG_PATH, sensor_name)
	var node: Node = null
	# 行人没有 Chassis/Sensors 挂载点（没有 AttachSensor）：直接调会 Invalid call = SCRIPT ERROR，
	# 所以先 has_method 守卫，走"几何未落地"的同一分支语义（只登记连接，不影响 manifest 就绪）。
	var can_mount: bool = vehicle.has_method("AttachSensor")
	if sensor_scene != null and can_mount:
		node = vehicle.call("AttachSensor", slot, sensor_scene,
			mount if mount is Dictionary else {}, params if params is Dictionary else {})
	elif sensor_scene != null and not can_mount:
		if not _warned_missing_sensor:
			push_warning("[ws] peer='%s' slot='%s' 的 avatar 没有传感器挂载点（pedestrian），仅登记连接"
				% [peer_id, slot])
			_warned_missing_sensor = true
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
## task_35 Step 2：守卫从 GetVehicle 放宽到 GetAvatar（车的结果不变，空 manifest 的行人也能就绪）。
func _TryAssembly(peer_id: String) -> void:
	if GetAvatar(peer_id) == null:
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

## 移除一台 avatar（车 / 行人共用）：先从**两张表**里摘（R8），再 free；同 peer 的其余连接一并关闭。
## except_link = 刚接手的**新**连接（重复 hello 时绝不能把它关掉，R13）。
## 返回值 = 是否真的摘到（两张表都没有 -> false：**不打日志、不发信号、不牵连其它连接**）。
## 第 2 轮要点（prev_issues #2）：
##   ① 表是两张、容器是两个，摘除必须同时清，否则换 kind 会留幽灵；
##   ② 被判死刑的连接一律交出 `link.vehicle`（置 null）—— 它们下一帧的 `_CleanupLink()`
##      绝不能再把**刚注册**的新 avatar 摘掉；
##   ③ emit / print 只在真的摘到东西时发生（旧实现会对"已经没车的 peer"打 `vehicle removed`）。
func _RemoveAvatar(peer_id: String, except_link: Variant = null) -> bool:
	var was_vehicle := _peer_vehicles.has(peer_id)
	var was_pedestrian := _pedestrians.has(peer_id)
	if not was_vehicle and not was_pedestrian:
		return false                                  # 什么都没摘：静默（②/③ 的前提）
	var avatar: Variant = _pedestrians.get(peer_id, _peer_vehicles.get(peer_id, null))
	# 先摘表，再 free（queue_free 后同帧仍 is_instance_valid，R9）
	_peer_vehicles.erase(peer_id)
	_pedestrians.erase(peer_id)
	_ped_last_motion_ms.erase(peer_id)
	_ped_idle.erase(peer_id)
	for link: Variant in _links.duplicate():
		if link.vehicle == avatar:
			link.vehicle = null                       # 交出所有权（含 except_link）
		if link == except_link:
			continue
		if link.peer_id == peer_id:
			_CloseLink(link, CLOSE_GOING_AWAY, "peer removed")
	if avatar != null and is_instance_valid(avatar):
		var parent: Node = avatar.get_parent()
		if parent != null:
			parent.remove_child(avatar)
		avatar.queue_free()
	_manifests.erase(peer_id)
	_manifest_parts.erase(peer_id)
	_assembly_sent.erase(peer_id)
	_pending_sensors.erase(peer_id)
	if was_vehicle:
		vehicle_removed.emit(peer_id)
		print("[ws] vehicle removed peer='%s'" % peer_id)
	if was_pedestrian:
		pedestrian_removed.emit(peer_id)
		print("[ws] pedestrian removed peer='%s'" % peer_id)
	return true


## 兼容壳（既有语义 / 既有调用点：只移除车）。内部走 _RemoveAvatar()，返回值丢弃。
func _RemoveVehicle(peer_id: String, except_link: Variant = null) -> void:
	_RemoveAvatar(peer_id, except_link)


## 清理一条连接的业务痕迹（由 _SweepClosed / Stop 调用；**不**从 _links 删除）。
func _CleanupLink(link: Variant) -> void:
	if link == null:
		return
	var peer_id: String = link.peer_id
	var role: StringName = link.role
	var slot := str(link.slot)
	if link.handshaken:
		if role == ROLE_CHASSIS:
			# 底盘断开 = 整台 avatar 走了（顺手关该 peer 其余连接）。
			# 第 2 轮：只有这条连接仍持有 avatar 才摘。被 `_RemoveAvatar()` 判死刑的连接
			# 已经把 `link.vehicle` 置 null，**不能再摘**（否则会误删同 peer 刚注册的新 avatar）。
			if link.vehicle != null:
				_RemoveAvatar(peer_id, null)
		elif role == ROLE_SENSOR:
			var vehicle: Node3D = GetAvatar(peer_id)
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


## 重名 -> base_<suffix_id>（避免 add_child 把节点自动改名成 "simA2" 导致名字不可预期，R14）。
## 车/人共用：root = 该 avatar 的容器，suffix_id = 该容器的编号（车的 avatar_id / 人的 pedestrian_id）。
func _UniqueAvatarName(base: String, root: Node3D, suffix_id: int) -> String:
	var name_taken: bool = false
	if root != null and is_instance_valid(root):
		name_taken = root.get_node_or_null(NodePath(base)) != null
	if not name_taken:
		return base
	return "%s_%d" % [base, suffix_id]


## 兼容壳（既有语义 / 既有调用点：只按车容器判重名）。
func _UniqueVehicleName(base: String) -> String:
	return _UniqueAvatarName(base, vehicles_root, _next_avatar_id)


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


# ================================================================ 内部工具：行人容器 / spawn 校验（task_35 Step 2）

## 行人容器：与 `Vehicles` 平级、同父（World 根）。
## 每次调用都 `is_instance_valid` 复检：Step 3 的 `World.gd::BuildPedestrians()` 可能重建这个容器，
## 那时缓存的指针会悬空 —— 复检 + 按名字重新解析，绝不留一个死指针。
func _Pedestrians_Root() -> Node3D:
	if pedestrians_root != null and is_instance_valid(pedestrians_root):
		return pedestrians_root
	if vehicles_root == null or not is_instance_valid(vehicles_root):
		_EnsureRefs()
	var parent: Node = null
	if vehicles_root != null and is_instance_valid(vehicles_root):
		parent = vehicles_root.get_parent()
	if parent == null:
		return null
	var found: Node3D = parent.get_node_or_null(NodePath(PEDESTRIANS_NODE)) as Node3D
	if found == null:
		found = Node3D.new()
		found.name = PEDESTRIANS_NODE
		parent.add_child(found)          # 首个行人到达时才建（无人连入 = 节点树零变化，R11）
		print("[ws] pedestrians root created: %s" % str(found.get_path()))
	pedestrians_root = found
	return found


## 这个 peer 已经有 avatar 了吗（车或人）。
func _HasAvatar(peer_id: String) -> bool:
	return _peer_vehicles.has(peer_id) or _pedestrians.has(peer_id)


## 全部**活着**的 avatar（车 + 人），供 spawn 重叠校验用。
func _All_Avatars() -> Array:
	var out: Array = []
	for table: Dictionary in [_peer_vehicles, _pedestrians]:
		for peer: Variant in table.keys():
			var avatar: Variant = table[peer]
			if avatar != null and is_instance_valid(avatar):
				out.append(avatar)
	return out


## 默认出生点：World.SPAWN_WORLD + PEDESTRIAN_SPAWN_OFFSET_M = (64, 56, 0)，yaw 0。
func _Pedestrian_Default_Spawn() -> Dictionary:
	var origin: Vector3 = World.SPAWN_WORLD
	return {
		"x": origin.x + PEDESTRIAN_SPAWN_OFFSET_M.x,
		"y": origin.y + PEDESTRIAN_SPAWN_OFFSET_M.y,
		"z": origin.z + PEDESTRIAN_SPAWN_OFFSET_M.z,
		"yaw": 0.0,
	}


## spawn 校验（task_35 Step 2 / COMPONENTS.md §8.1-9）：
## 缺失 = 合法用法（默认点、不 warn）；有值但非法 = warn + 默认点。
func _Pick_Pedestrian_Spawn(msg: Dictionary, peer_id: String) -> Dictionary:
	if not msg.has("spawn"):
		return _Pedestrian_Default_Spawn()
	var raw: Variant = msg["spawn"]
	if not (raw is Dictionary):
		push_warning("[ws] peer='%s' spawn 不是对象（%s），回退默认点" % [peer_id, str(raw)])
		return _Pedestrian_Default_Spawn()
	var spawn: Dictionary = raw
	var x := _To_Float(spawn.get("x", NAN), NAN)
	var y := _To_Float(spawn.get("y", NAN), NAN)
	var z := _To_Float(spawn.get("z", 0.0), 0.0)
	var yaw := _To_Float(spawn.get("yaw", 0.0), 0.0)
	var span := _World_Span_M()
	if not is_finite(x) or not is_finite(y):
		push_warning("[ws] peer='%s' spawn 的 x/y 不是有限数（%s），回退默认点" % [peer_id, str(spawn)])
		return _Pedestrian_Default_Spawn()
	if x < 0.0 or x >= span or y < 0.0 or y >= span:
		push_warning("[ws] peer='%s' spawn (%.2f, %.2f) 越界（[0, %.1f)），回退默认点"
			% [peer_id, x, y, span])
		return _Pedestrian_Default_Spawn()
	if not is_finite(z) or z < 0.0 or z > PEDESTRIAN_SPAWN_MAX_Z_M:
		push_warning("[ws] peer='%s' spawn.z=%s 越界（[0, %.1f]），回退默认点"
			% [peer_id, str(spawn.get("z", 0.0)), PEDESTRIAN_SPAWN_MAX_Z_M])
		return _Pedestrian_Default_Spawn()
	var point := Vector3(x, y, z)
	if _Spawn_In_Obstacle(point):
		push_warning("[ws] peer='%s' spawn (%.2f, %.2f) 落在静态障碍内，回退默认点" % [peer_id, x, y])
		return _Pedestrian_Default_Spawn()
	if _Spawn_Occupied(point):
		push_warning("[ws] peer='%s' spawn (%.2f, %.2f) 与已占 avatar 重叠（< %.1f m），回退默认点"
			% [peer_id, x, y, PEDESTRIAN_SPAWN_MIN_GAP_M])
		return _Pedestrian_Default_Spawn()
	return {"x": x, "y": y, "z": z, "yaw": wrapf(yaw, -PI, PI) if is_finite(yaw) else 0.0}


## 点是否落在 World.OBSTACLES 的某个 box 内（外扩 PEDESTRIAN_SPAWN_MARGIN_M）。
## 障碍表是**世界坐标**（origin = 最小角，size = 尺寸），与 World.gd 逐字同源（P8：不复制字面量）。
func _Spawn_In_Obstacle(p: Vector3) -> bool:
	var margin := PEDESTRIAN_SPAWN_MARGIN_M
	for item: Variant in World.OBSTACLES:
		var box: Dictionary = item
		var origin: Vector3 = box.get("origin", Vector3.ZERO)
		var size: Vector3 = box.get("size", Vector3.ZERO)
		if p.x >= origin.x - margin and p.x <= origin.x + size.x + margin \
				and p.y >= origin.y - margin and p.y <= origin.y + size.y + margin \
				and p.z >= origin.z - margin and p.z <= origin.z + size.z + margin:
			return true
	return false


## 点的水平中心距是否与任何已占 avatar < PEDESTRIAN_SPAWN_MIN_GAP_M（用 GetPose 的世界坐标）。
func _Spawn_Occupied(p: Vector3) -> bool:
	for avatar: Variant in _All_Avatars():
		if avatar == null or not is_instance_valid(avatar) or not avatar.has_method("GetPose"):
			continue
		var pose: Dictionary = avatar.call("GetPose")
		var dx := float(pose.get("x", 0.0)) - p.x
		var dy := float(pose.get("y", 0.0)) - p.y
		if sqrt(dx * dx + dy * dy) < PEDESTRIAN_SPAWN_MIN_GAP_M:
			return true
	return false


## 世界边长（米）：唯一来源是 project.godot 的 [sim_world]（World.gd 同款），不写 128 字面量。
func _World_Span_M() -> float:
	var voxel := float(ProjectSettings.get_setting(World.SETTING_VOXEL_SIZE, 0.0))
	var cells := int(ProjectSettings.get_setting(World.SETTING_CELLS_X, 0))
	if voxel <= 0.0 or cells <= 0:
		return World.SPAWN_WORLD.x * 2.0
	return float(cells) * voxel


## 底盘默认参数 <- hello.params 覆盖（行人：{max_speed, max_turn_rate}；与 ChassisModel 同口径）。
func _Merge_Avatar_Params(avatar: Node, params: Dictionary) -> Dictionary:
	var merged: Dictionary = {}
	if avatar != null and is_instance_valid(avatar) and avatar.has_method("GetDefaultParams"):
		var defaults: Variant = avatar.call("GetDefaultParams")
		if defaults is Dictionary:
			merged = (defaults as Dictionary).duplicate(true)
	for key: Variant in params.keys():
		merged[key] = params[key]
	return merged


## 宽容的数值解析（与 ChassisModel._to_float 同款）：非法类型 / 非数字字符串 -> fallback。
func _To_Float(value: Variant, fallback: float) -> float:
	match typeof(value):
		TYPE_INT, TYPE_FLOAT:
			var number := float(value)
			return number if is_finite(number) else fallback
		TYPE_STRING, TYPE_STRING_NAME:
			var text := str(value).strip_edges()
			return text.to_float() if text.is_valid_float() else fallback
		_:
			return fallback


## chassis_hello.kind -> LEGAL 值；缺省 = robot；非法值 warn（一次一条）后按 robot 处理。
func _Kind_Of(msg: Dictionary, peer_id: String = "") -> String:
	if not msg.has("kind"):
		return WireProtocol.KIND_ROBOT
	var raw: Variant = msg["kind"]
	var kind := WireProtocol.NormalizeKind(raw)
	if kind.is_empty():
		push_warning("[ws] peer='%s' chassis_hello.kind 非法（%s），按 robot 处理"
			% [peer_id, str(raw)])
		return WireProtocol.KIND_ROBOT
	return kind


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
