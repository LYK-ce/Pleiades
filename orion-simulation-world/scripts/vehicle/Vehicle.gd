# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# Vehicle 组件（task_30 §4.1 D11 / COMPONENTS.md §4.3）：avatar 的"身体"。
#   本身是 CharacterBody3D（不是 Node3D 容器）；位置更新在世界侧（D8），
#   物理推进只做三件事：问底盘要控制量 -> 转 yaw -> 由物理引擎推进位置。
#   行为/行动模式由底盘（Chassis）决定；感知能力由传感器（Sensors）决定。
#
# 本文件**不做**的事（Step 5/7/8 的边界）：
#   不做运动学积分/速度环（在底盘 Step 里）、不做射线投射（在 Lidar2D 里）、
#   不碰 WebSocket（在 WebSocketManager 里）。
#
# 坐标：Godot Y-up，前进轴 = 局部 +X；换轴只经 WorldTransform（唯一入口）。
#   Chassis.Step() 的 yaw_rate 是 **Godot rotation.y** 的角速度（正 = 左转）；
#   GetPose().yaw 是 **世界 yaw** = -rotation.y（两者符号相反，勿混用）。
#
# 装配契约（鸭子类型，见 COMPONENTS.md §4.3）：
#   底盘   必需 Step(velocity, delta) -> {velocity, yaw_rate}；
#          可选 Configure(self, params) / SetCommand(cmd) / GetDefaultParams()
#   传感器 必需根节点是 Node3D；可选 Configure(params) / Sample() / ScanOnce()
#
# 手写 Vehicle.tscn 时，本文件的 4 个 @export 节点引用必须在场景里靠
# node_paths=PackedStringArray(...) 才会绑定（COMPONENTS.md §9.1 E16）。
#
# 不声明全局类名（COMPONENTS.md §9.1 E5）。

extends CharacterBody3D

const WorldTransform := preload("res://scripts/common/WorldTransform.gd")

## 底盘目录（Step 5 才落地）：存在才 load()，否则只支持显式传入场景（避免 preload 解析失败）。
const CHASSIS_CATALOG_PATH := "res://scripts/chassis/ChassisCatalog.gd"

const GROUP_VEHICLE := &"vehicle"
## 车身尺寸下限（米）：避免零尺寸 mesh / shape 让物理与渲染退化。
const MIN_BODY_SIZE := 0.01


## 底盘装配完成（manager 可选监听；Step 6 不依赖它）。
signal chassis_installed(chassis_name: String)
## 传感器挂载完成（slot = 归一化后的节点名）。
signal sensor_attached(slot: String, sensor: Node)


## 车身外观（MeshInstance3D）。底盘 Configure() / SetBodySize() 替换其 mesh。
@export var body: MeshInstance3D = null
## 碰撞形状（CollisionShape3D）。position.y 恒 = 车身高度/2（原点=底面中心）。
@export var collision_shape: CollisionShape3D = null
## 底盘挂载点（Node3D）。运行时**至多一个**子节点。
@export var chassis_root: Node3D = null
## 传感器挂载点（Node3D）。子节点名 = slot。
@export var sensors_root: Node3D = null

var _chassis: Node3D = null          # 底盘策略节点（chassis_root 的唯一子节点）
var _chassis_name: String = ""       # hello/motion 里用的零件名，如 "diff_drive_ugv"
var _chassis_params: Dictionary = {} # 生效参数（= 底盘默认值 <- 传入值 合并结果）
var _cmd: Dictionary = {}            # 最新一条 motion_cmd（只保留最新，不排队）
var _sensors: Dictionary = {}        # slot(String) -> Node3D
var _nose: MeshInstance3D = null     # Body/Nose（朝向标记，SetBodySize 时跟着摆）


# ================================================================ 生命周期

func _ready() -> void:
	_ensure_refs()
	if not is_in_group(GROUP_VEHICLE):
		push_warning("[vehicle] 不在 \"%s\" 组内，相机无法选中（检查 Vehicle.tscn 的 groups）"
			% GROUP_VEHICLE)
	var shown := _chassis_name if not _chassis_name.is_empty() else "<none>"
	print("[vehicle] ready: chassis=%s sensors=%d pos=(%.2f, %.2f, %.2f)"
		% [shown, _sensors.size(), global_position.x, global_position.y, global_position.z])


## 物理推进：空车原地不动；有底盘才问控制量、转 yaw 并推进位置（唯一位置更新点，D8）。
func _physics_process(delta: float) -> void:
	if not _is_chassis_alive():
		# 空车 / 底盘被外部释放：本帧不推进，速度归零（世界权威：没人给指令就不动）。
		if not velocity.is_zero_approx():
			velocity = Vector3.ZERO
		return
	var ctl: Variant = _chassis.call("Step", velocity, delta)
	if typeof(ctl) != TYPE_DICTIONARY:
		return
	var ctl_dict: Dictionary = ctl
	rotation.y += float(ctl_dict.get("yaw_rate", 0.0)) * delta
	velocity = ctl_dict.get("velocity", Vector3.ZERO)
	move_and_slide()


# ================================================================ 装配接口

## 挂底盘（含换底盘）。成功 true；失败 false 且**状态不变**。
## 名字 -> 场景的解析顺序：显式 chassis_scene -> ChassisCatalog（Step 5 落地后）-> 失败。
func ApplyChassis(chassis_name: String, params: Dictionary = {},
		chassis_scene: PackedScene = null) -> bool:
	var key := chassis_name.strip_edges()
	if key.is_empty():
		push_warning("[vehicle] ApplyChassis: 空底盘名")
		return false
	if chassis_root == null:
		_ensure_refs()
	if chassis_root == null:
		push_warning("[vehicle] ApplyChassis: chassis_root 缺失，无法挂载")
		return false
	var scene: PackedScene = chassis_scene if chassis_scene != null else _resolve_chassis_scene(key)
	if scene == null:
		push_warning("[vehicle] ApplyChassis: 找不到底盘场景 '%s'（Step 5 前请显式传 chassis_scene）"
			% key)
		return false
	var node: Node = scene.instantiate()
	if node == null:
		push_warning("[vehicle] ApplyChassis: '%s' 场景实例化失败" % key)
		return false
	var chassis: Node3D = node as Node3D
	if chassis == null or not chassis.has_method("Step"):
		push_warning("[vehicle] ApplyChassis: '%s' 根节点不是 Node3D 或缺少 Step()" % key)
		node.queue_free()
		return false
	if not chassis.has_method("Configure"):
		push_warning("[vehicle] ApplyChassis: '%s' 未实现 Configure()，motion_mode/车身尺寸需自行设置"
			% key)

	_clear_chassis()                       # 换底盘：先摘旧（remove_child）再 queue_free
	_chassis = chassis
	_chassis_name = key
	_chassis_params = _merge_params(chassis, params)
	chassis_root.add_child(chassis)
	if chassis.has_method("Configure"):
		chassis.call("Configure", self, _chassis_params)
	chassis_installed.emit(key)
	print("[vehicle] chassis installed: %s params=%s" % [key, _chassis_params])
	return true


## 挂传感器：slot 归一化后作为节点名；同 slot 重挂 = 先卸后挂（同帧即可）。
## mount = {x, y, z, yaw}，**车体局部坐标 + 世界轴向**（x 前 / y 右 / z 高 / yaw 绕 z）。
func AttachSensor(slot: String, sensor_scene: PackedScene, mount: Dictionary = {},
		params: Dictionary = {}) -> Node:
	var key := _normalize_slot(slot)
	if key.is_empty():
		push_warning("[vehicle] AttachSensor: 非法 slot '%s'" % slot)
		return null
	if sensor_scene == null:
		push_warning("[vehicle] AttachSensor: slot '%s' 的传感器场景为空" % key)
		return null
	if sensors_root == null:
		_ensure_refs()
	if sensors_root == null:
		push_warning("[vehicle] AttachSensor: sensors_root 缺失，无法挂载")
		return null
	var node: Node = sensor_scene.instantiate()
	var holder: Node3D = node as Node3D
	if holder == null:
		push_warning("[vehicle] AttachSensor: slot '%s' 的根节点不是 Node3D" % key)
		if node != null:
			node.queue_free()
		return null
	DetachSensor(key)
	holder.name = key
	sensors_root.add_child(holder)
	# mount 外参：车体局部 + 世界轴向，换算只经 WorldTransform（唯一入口）。
	holder.position = WorldTransform.PosToGodot(Vector3(
		float(mount.get("x", 0.0)), float(mount.get("y", 0.0)), float(mount.get("z", 0.0))))
	holder.rotation.y = WorldTransform.YawToGodot(float(mount.get("yaw", 0.0)))
	if holder.has_method("Configure"):
		holder.call("Configure", params)
	_sensors[key] = holder
	sensor_attached.emit(key, holder)
	print("[vehicle] sensor attached: %s -> %s" % [key, holder.get_class()])
	return holder


## 卸传感器；不存在则空操作（不崩、不打印）。
func DetachSensor(slot: String) -> void:
	var key := _normalize_slot(slot)
	if key.is_empty() or not _sensors.has(key):
		return
	var holder: Node3D = _sensors[key]
	_sensors.erase(key)
	if holder != null and is_instance_valid(holder):
		var parent: Node = holder.get_parent()
		if parent != null:
			parent.remove_child(holder)    # 先摘再 free：名字立刻可复用
		holder.queue_free()
		print("[vehicle] sensor detached: %s" % key)


## 车身尺寸唯一入口：尺寸 -> body.mesh + collision_shape + "原点 = 底面中心"的抬升。
## 不含任何运动学；底盘仍可绕过它直接写 vehicle.body（不强制）。
func SetBodySize(size: Vector3) -> void:
	var s := Vector3(maxf(size.x, MIN_BODY_SIZE), maxf(size.y, MIN_BODY_SIZE),
		maxf(size.z, MIN_BODY_SIZE))
	if body != null and is_instance_valid(body):
		var box_mesh := BoxMesh.new()
		box_mesh.size = s
		body.mesh = box_mesh              # surface_material_override/0 挂在 MeshInstance3D 上，材质不丢
		body.position = Vector3(0.0, s.y * 0.5, 0.0)
	if collision_shape != null and is_instance_valid(collision_shape):
		var box_shape := BoxShape3D.new()
		box_shape.size = s
		collision_shape.shape = box_shape
		collision_shape.position = Vector3(0.0, s.y * 0.5, 0.0)
	if _nose != null and is_instance_valid(_nose):
		_nose.position = Vector3(s.x * 0.5, 0.0, 0.0)   # 朝向标记顶在车头


# ================================================================ 查询

## 未挂 = ""。
func GetChassisName() -> String:
	return _chassis_name


func HasChassis() -> bool:
	return _is_chassis_alive()


## 生效参数（底盘默认值 <- 传入值）；未挂 = {}。
func GetChassisParams() -> Dictionary:
	return _chassis_params.duplicate(true)


## 不存在 = null。键名与 Sensors/<slot> 的节点名一致。
func GetSensor(slot: String) -> Node3D:
	var key := _normalize_slot(slot)
	if key.is_empty():
		return null
	var holder: Node3D = _sensors.get(key, null)
	if holder == null or not is_instance_valid(holder):
		return null
	return holder


## slot -> 传感器节点（副本，供 manager 建表 / 清理）。
func GetStreamSlots() -> Dictionary:
	return _sensors.duplicate()


## 位姿 {x, y, z, yaw}：世界坐标（Z-up，米），yaw 归一化到 [-PI, PI)。
func GetPose() -> Dictionary:
	return WorldTransform.PoseFromTransform(global_transform)


# ================================================================ 统一指令入口

## 只存最新一条 motion_cmd 并转发给底盘（不排队、不依赖下发频率）。
func SetMotionCommand(cmd: Dictionary) -> void:
	_cmd = cmd.duplicate(true)
	if _is_chassis_alive() and _chassis.has_method("SetCommand"):
		_chassis.call("SetCommand", _cmd)


## slot -> 传感器自报样本：Sample() 优先、ScanOnce() 次之、都没有给 null（鸭子契约）。
func SampleSensors() -> Dictionary:
	var out: Dictionary = {}
	for key: String in _sensors.keys():
		var holder: Node3D = _sensors[key]
		if holder == null or not is_instance_valid(holder):
			continue
		if holder.has_method("Sample"):
			out[key] = holder.call("Sample")
		elif holder.has_method("ScanOnce"):
			out[key] = holder.call("ScanOnce")
		else:
			out[key] = null
	return out


# ================================================================ 统一接口别名（step.md 契约名）

## == SetMotionCommand（step.md §八 Step 4 的对外统一接口名）。
func apply_motion(cmd: Dictionary) -> void:
	SetMotionCommand(cmd)


## == SampleSensors。
func sample_sensors() -> Dictionary:
	return SampleSensors()


## == GetPose。
func get_pose() -> Dictionary:
	return GetPose()


# ================================================================ 私有

## E16 兜底：node_paths 漏写时 @export 引用会静默为 null，这里按子节点名回退一次。
func _ensure_refs() -> void:
	if body == null:
		body = get_node_or_null("Body") as MeshInstance3D
	if collision_shape == null:
		collision_shape = get_node_or_null("CollisionShape3D") as CollisionShape3D
	if chassis_root == null:
		chassis_root = get_node_or_null("Chassis") as Node3D
	if sensors_root == null:
		sensors_root = get_node_or_null("Sensors") as Node3D
	_nose = null
	if body != null:
		_nose = body.get_node_or_null("Nose") as MeshInstance3D
	if body == null or collision_shape == null or chassis_root == null or sensors_root == null:
		push_warning("[vehicle] 挂载点缺失：body=%s shape=%s chassis=%s sensors=%s"
			% [body, collision_shape, chassis_root, sensors_root])


## 名字 -> 底盘场景。Step 5 未落地时静默返回 null（ResourceLoader.exists 不产生 ERROR）。
func _resolve_chassis_scene(chassis_name: String) -> PackedScene:
	if not ResourceLoader.exists(CHASSIS_CATALOG_PATH):
		return null
	var catalog: Script = load(CHASSIS_CATALOG_PATH) as Script
	if catalog == null:
		return null
	if not bool(catalog.call("Has", chassis_name)):
		return null
	var scene: Variant = catalog.call("GetScene", chassis_name)
	if scene is PackedScene:
		return scene as PackedScene
	return null


## 卸旧底盘：先 remove_child 再 queue_free（同帧重挂不会被改名成 "Xxx2"）。
func _clear_chassis() -> void:
	if _chassis != null and is_instance_valid(_chassis):
		var parent: Node = _chassis.get_parent()
		if parent != null:
			parent.remove_child(_chassis)
		_chassis.queue_free()
	_chassis = null
	_chassis_name = ""
	_chassis_params = {}


## 生效参数 = 底盘 GetDefaultParams()（左）<- 传入 params（右，优先）。
func _merge_params(chassis: Node, params: Dictionary) -> Dictionary:
	var merged: Dictionary = {}
	if chassis != null and chassis.has_method("GetDefaultParams"):
		var defaults: Variant = chassis.call("GetDefaultParams")
		if defaults is Dictionary:
			var defaults_dict: Dictionary = defaults
			merged = defaults_dict.duplicate(true)
	for key in params:
		merged[key] = params[key]
	return merged


## slot 归一化：去空白 + validate_node_name()（键名 = 节点名，与 get_node("Sensors/<slot>") 一致）。
func _normalize_slot(slot: String) -> String:
	return slot.strip_edges().validate_node_name()


## 每帧检查底盘是否还有效；失效则就地置空（防悬空引用），但保留 _chassis_name（只断引用）。
func _is_chassis_alive() -> bool:
	if _chassis == null:
		return false
	if not is_instance_valid(_chassis):
		_chassis = null
		return false
	return true
