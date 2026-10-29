# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-29
#
# Quadrotor 四旋翼底盘自检（Step 7）：场景形态 / 目录装配 / 无重力悬停 / 升降 / 平移 / 偏航 / 边界与白盒。
# 用法：timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_quadrotor.gd
# 期望：failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR（COMPONENTS.md §11 Step 7）。
#
# 决定本文件写法的实测事实（plan.md §2）：
#   P1  MOTION_MODE_FLOATING + 零速度、没有任何重力代码 → 300 帧 dy=0.00000000
#       （CharacterBody3D 从不自动施加重力）→ "无重力悬停" 可以直接用**零漂移**断言。
#   P2  竖直斜坡（0→1.0 m/s，3 m/s²）60 帧位移 = 0.8417 m（20 帧到顶 + 40 帧匀速）。
#   P3  水平斜坡（0→2.0 m/s，3 m/s²）60 帧位移 = 1.3500 m，且 dy = 0。
#   P4  从 y=2.5 持续 -1.0 m/s 下降 240 帧 → 停在 y≈0.0001，再 60 帧 dz=0（不打穿地面）。
#       （2.5 m 全速下降约 160 帧 < 240，结论不变。）
#   P5  巡航 2.5 m **高于** 2 m 可见矮墙（task_32 Step 3 把巡航高度与墙高解耦，本文件期望一律 2.5）；
#       本文件所有路径仍在 (64,64) 净空区，不依赖「UAV 低于墙高」。
#   P17 高度软件限幅（task_32 Step 3）：Quadrotor 每物理帧把宿主世界 z 夹到 [0, 31.8]，
#       越界时 velocity.y 归零（D5 的三项断言；31.8 = World.AIR_WALL_HEIGHT_M(32.0) - 0.2）。
#   P6  CharacterBody3D.MOTION_MODE_GROUNDED == 0 是默认值 → 必须先置 GROUNDED 再断言被改成 FLOATING。
#   P8  未入树访问 global_* 会报 ERROR → Step() 内部走 is_inside_tree() 兜底，本文件直接断言它。
#   P15 含 await 的辅助函数必须被 await 调用；本文件所有辅助函数都不含 await。
#   P16 一次 null 解引用会让协程中断、quit() 不执行 → 入口先判空，命令带 timeout。
#
# 所有断言都用 --fixed-fps 60 下的累计量（15/30/60/120/240/300 帧），不逐帧比对瞬时值；
# 容差 = ±8% 或 1e-3 绝对（物理积分 + move_and_slide 的 safe_margin）。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")

const CHASSIS_SCENE := "res://src/scenes/chassis/Quadrotor.tscn"
const CHASSIS_SCRIPT := "res://src/chassis/Quadrotor.gd"
const BASE_SCRIPT := "res://src/chassis/ChassisModel.gd"
const CATALOG_SCRIPT := "res://src/chassis/ChassisCatalog.gd"
const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const WORLD_SCENE := "res://src/scenes/World.tscn"
const WORLD_SCRIPT := "res://src/world/World.gd"

const CHASSIS_NAME := "quadrotor_uav"
## 机身厚度（米）：碰撞形状 = BoxShape3D(size, 0.12, size)。
const BODY_HEIGHT := 0.12
## 旋翼盘面比机顶高 0.01 m（默认 size=0.40 -> y = 0.13）。
const ROTOR_LIFT := 0.01
const ROTOR_NAMES := ["RotorFL", "RotorFR", "RotorRL", "RotorRR"]

## task_32 Step 3 的高度限幅口径（测试侧镜像；D5-E3 断言它与 World / Quadrotor 的常量自洽，防漂移）。
const CLAMP_MARGIN_M := 0.2
## 限幅上限（米）= World.AIR_WALL_HEIGHT_M(32.0) - CLAMP_MARGIN_M(0.2) = 31.8。
const CLAMP_CEILING_M := 31.8

const TOL := 0.001
## dt = 1/60（--fixed-fps 60）。
const DT := 1.0 / 60.0
## 出生 / 巡航高度 2.5 m（§7.1 / §8.1-1；task_32 Step 3 由 2 m 提高到 2.5，越过 2 m 可见矮墙）。
const SPAWN_WORLD := Vector3(64.0, 64.0, 2.5)
## 高空悬停：证明"不掉高度"与 default_gravity 无关（不是"贴着地面所以不掉"）。
const HIGH_WORLD := Vector3(64.0, 64.0, 10.0)

## 解析值（plan.md §7）：60 帧水平 / 15 帧水平 / 120 帧水平 / 60 帧竖直 / 60 帧偏航（rad）。
const WANT_FWD_15 := 0.10
const WANT_FWD_60 := 1.35
const WANT_FWD_120 := 3.35
const WANT_CLIMB_60 := 0.8417
const WANT_YAW_60 := 1.1912
## 水平从 2.0 m/s 斜坡停住的漂移（forward 60 -> up 60 的水平漂移区间）。
const DRIFT_MIN := 0.50
const DRIFT_MAX := 0.85

const DEFAULT_PARAMS := {
	"max_speed": 2.0,
	"max_climb_rate": 1.0,
	"max_yaw_rate": 90.0,
	"size": 0.40,
}

## 白盒 lint：底盘脚本里**不允许**出现的符号（越界 / 违反 D8 / 私加物理）。
const BANNED_TOKENS := [
	"class_name", "move_and_slide", "global_position", "intersect_ray",
	"direct_space_state", "WebSocketPeer", "get_gravity", "apply_gravity",
	"MOTION_MODE_GROUNDED",
]

var _total: int = 0
var _failed: int = 0
var _harness_errors: int = 0


func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ A. 树外：场景与脚本形态
	var packed: PackedScene = load(CHASSIS_SCENE) as PackedScene
	if packed == null:
		_fatal("scene-loads", CHASSIS_SCENE)
		_finish()
		return
	var probe: Node = packed.instantiate()
	if probe == null:
		_fatal("scene-instantiates", CHASSIS_SCENE)
		_finish()
		return
	_check("scene-loads", probe is Node3D, "%s -> %s" % [CHASSIS_SCENE, probe.get_class()])

	# A2 根节点：名字 / 类型 / 脚本
	var probe_script: Script = probe.get_script()
	_check("root-type", probe.name == "Quadrotor" and probe is Node3D
		and probe_script != null and probe_script.resource_path == CHASSIS_SCRIPT,
		"name=%s class=%s script=%s" % [probe.name, probe.get_class(),
			probe_script.resource_path if probe_script != null else "null"])

	# A3 节点树 + 旋翼外观（CylinderMesh r=0.12 h=0.02 radial_segments=16，平盘不转轴）
	var rotors: Node3D = probe.get_node_or_null("Rotors") as Node3D
	var rotors_ok := rotors != null
	var rotors_detail := "Rotors=%s" % rotors
	if rotors != null:
		for rotor_name: String in ROTOR_NAMES:
			var rotor: MeshInstance3D = rotors.get_node_or_null(rotor_name) as MeshInstance3D
			if rotor == null:
				rotors_ok = false
				rotors_detail += " 缺 %s" % rotor_name
				continue
			var cyl: CylinderMesh = rotor.mesh as CylinderMesh
			if cyl == null:
				rotors_ok = false
				rotors_detail += " %s.mesh=%s" % [rotor_name, rotor.mesh]
				continue
			rotors_ok = rotors_ok \
				and absf(cyl.top_radius - 0.12) < TOL \
				and absf(cyl.bottom_radius - 0.12) < TOL \
				and absf(cyl.height - 0.02) < TOL \
				and cyl.radial_segments == 16
			rotors_detail += " %s(r=%.3f,h=%.3f,seg=%d)" % [rotor_name,
				cyl.top_radius, cyl.height, cyl.radial_segments]
	_check("rotors-shape", rotors_ok, rotors_detail)

	# A4 默认摆位（size=0.40 -> x/z = ±0.2，y = 0.12 + 0.01 = 0.13）
	var layout_ok := rotors != null
	var layout_detail := ""
	var want_pos := [Vector3(0.2, 0.13, -0.2), Vector3(0.2, 0.13, 0.2),
		Vector3(-0.2, 0.13, -0.2), Vector3(-0.2, 0.13, 0.2)]
	if rotors != null:
		for i in range(ROTOR_NAMES.size()):
			var rotor: Node3D = rotors.get_node_or_null(ROTOR_NAMES[i]) as Node3D
			if rotor == null:
				layout_ok = false
				continue
			layout_ok = layout_ok and rotor.position.distance_to(want_pos[i]) < TOL
			layout_detail += " %s=%s" % [ROTOR_NAMES[i], rotor.position]
	_check("rotors-layout", layout_ok, layout_detail)

	# A5 四个契约方法 + 基类路径
	var methods_ok: bool = probe.has_method("Configure") and probe.has_method("GetDefaultParams") \
		and probe.has_method("SetCommand") and probe.has_method("Step")
	var base: Script = probe_script.get_base_script() if probe_script != null else null
	_check("contract-methods", methods_ok and base != null and base.resource_path == BASE_SCRIPT,
		"methods=%s base=%s" % [methods_ok, base.resource_path if base != null else "null"])

	# A6 默认参数（与目录表逐键逐类型一致）
	var defs: Variant = probe.call("GetDefaultParams")
	var defs_dict: Dictionary = defs if defs is Dictionary else {}
	var catalog: Script = load(CATALOG_SCRIPT) as Script
	var catalog_dict: Dictionary = {}
	if catalog != null:
		var catalog_defaults: Variant = catalog.call("GetDefaults", CHASSIS_NAME)
		if catalog_defaults is Dictionary:
			catalog_dict = catalog_defaults
	_check("default-params", _dict_equal(defs_dict, DEFAULT_PARAMS)
		and _dict_equal(defs_dict, catalog_dict),
		"obj=%s catalog=%s" % [defs_dict, catalog_dict])

	# A7 树外 free（无 !is_inside_tree ERROR）
	probe.free()
	_check("free-outside-tree", not is_instance_valid(probe), "树外实例已释放")

	# A8 目录：quadrotor_uav 的场景**从 Step 6 的 null 变为可用**（Has 一直为真）
	var has_quad: bool = bool(catalog.call("Has", CHASSIS_NAME)) if catalog != null else false
	var has_bogus: bool = bool(catalog.call("Has", "no_such_chassis")) if catalog != null else true
	var quad_scene: Variant = catalog.call("GetScene", CHASSIS_NAME) if catalog != null else null
	var bogus_defaults: Variant = null
	if catalog != null:
		bogus_defaults = catalog.call("GetDefaults", "no_such_chassis")
	_check("catalog-scene-now-resolves", catalog != null and has_quad and not has_bogus
		and quad_scene is PackedScene
		and bogus_defaults is Dictionary and (bogus_defaults as Dictionary).is_empty(),
		"quad=%s bogus=%s scene=%s defaults=%s" % [has_quad, has_bogus, quad_scene,
			bogus_defaults])

	# A9 场景缓存：两次 GetScene 同一对象（避免重复读盘）
	var scene_a: Variant = catalog.call("GetScene", CHASSIS_NAME) if catalog != null else null
	var scene_b: Variant = catalog.call("GetScene", CHASSIS_NAME) if catalog != null else null
	_check("catalog-scene-cache", scene_a is PackedScene and scene_b is PackedScene
		and scene_a == scene_b, "a=%s b=%s same=%s" % [scene_a, scene_b, scene_a == scene_b])

	# ============================================================ B. 真实 World + Vehicle 装配
	var world_packed: PackedScene = load(WORLD_SCENE) as PackedScene
	if world_packed == null:
		_fatal("world-loads", WORLD_SCENE)
		_finish()
		return
	var vehicle_packed: PackedScene = load(VEHICLE_SCENE) as PackedScene
	if vehicle_packed == null:
		_fatal("vehicle-loads", VEHICLE_SCENE)
		_finish()
		return
	var world: Node3D = world_packed.instantiate() as Node3D
	root.add_child(world)
	await _wait_physics(2)
	var vehicles_root: Node3D = world.get_node_or_null("Vehicles") as Node3D
	if vehicles_root == null:
		_fatal("vehicles-root", "World/Vehicles 缺失")
		_finish()
		return
	var vehicle: CharacterBody3D = vehicle_packed.instantiate() as CharacterBody3D
	vehicles_root.add_child(vehicle)
	vehicle.global_position = WorldTransform.PosToGodot(SPAWN_WORLD)
	vehicle.rotation.y = 0.0
	await _wait_physics(2)
	if vehicle.chassis_root == null:
		_fatal("chassis-root", "Vehicle/Chassis 缺失")
		_finish()
		return

	# B1 出生高度 = 巡航高度 2.5 m（§7.1 / §8.1-1；UAV 不落地）
	var pose_spawn: Dictionary = vehicle.GetPose()
	_check("spawn-height", absf(float(pose_spawn.get("z", -1.0)) - SPAWN_WORLD.z) < TOL,
		"pose.z=%.4f want=%.4f pose=%s" % [float(pose_spawn.get("z", 0.0)),
			SPAWN_WORLD.z, pose_spawn])

	# B2 装配（不传场景 -> 走目录解析）+ 参数合并快照（B3 会用 {} 重装，必须在这里取）
	var applied: bool = vehicle.ApplyChassis(CHASSIS_NAME, {"max_speed": 2.5}, null)
	var merged: Dictionary = vehicle.GetChassisParams()
	var child: Node = _first_child(vehicle.chassis_root)
	_check("apply-chassis", applied and vehicle.HasChassis()
		and vehicle.GetChassisName() == CHASSIS_NAME
		and vehicle.chassis_root.get_child_count() == 1
		and child != null and child.name == "Quadrotor",
		"applied=%s name=%s children=%d child=%s" % [applied, vehicle.GetChassisName(),
			vehicle.chassis_root.get_child_count(), child])

	if child == null:
		_fatal("chassis-installed", "目录装配失败，后续断言无法进行")
		_finish()
		return

	# B3 FLOATING 是 Configure 真改的（P6：GROUNDED==0 是默认值，不能直接断言）
	vehicle.motion_mode = CharacterBody3D.MOTION_MODE_GROUNDED
	var mode_before: int = vehicle.motion_mode
	vehicle.ApplyChassis(CHASSIS_NAME, {}, null)
	_check("motion-mode-floating", mode_before == CharacterBody3D.MOTION_MODE_GROUNDED
		and vehicle.motion_mode == CharacterBody3D.MOTION_MODE_FLOATING,
		"before=%d after=%d want=%d" % [mode_before, vehicle.motion_mode,
			CharacterBody3D.MOTION_MODE_FLOATING])

	# B4 车身 / 碰撞形状：Box(size, 0.12, size)，原点 = 底面中心（不双重抬升）
	var body: MeshInstance3D = vehicle.body
	var shape: CollisionShape3D = vehicle.collision_shape
	var size_ok := false
	var size_detail := "body/shape 缺失"
	if body != null and shape != null and body.mesh is BoxMesh and shape.shape is BoxShape3D:
		var box: BoxMesh = body.mesh as BoxMesh
		var shape_box: BoxShape3D = shape.shape as BoxShape3D
		size_ok = box.size.is_equal_approx(Vector3(0.40, BODY_HEIGHT, 0.40)) \
			and shape_box.size.is_equal_approx(Vector3(0.40, BODY_HEIGHT, 0.40)) \
			and absf(body.position.y - BODY_HEIGHT * 0.5) < 1e-4 \
			and absf(shape.position.y - BODY_HEIGHT * 0.5) < 1e-4
		size_detail = "mesh=%s shape=%s body.y=%.4f shape.y=%.4f" % [box.size,
			shape_box.size, body.position.y, shape.position.y]
	_check("body-shape", size_ok, size_detail)

	# B5 按自定义 size 重新摆旋翼 + 参数表（默认值 <- {"max_speed": 2.5}）逐键正确
	var merge_ok := absf(float(merged.get("max_speed", 0.0)) - 2.5) < TOL \
		and absf(float(merged.get("max_climb_rate", 0.0)) - 1.0) < TOL \
		and absf(float(merged.get("max_yaw_rate", 0.0)) - 90.0) < TOL \
		and absf(float(merged.get("size", 0.0)) - 0.40) < TOL
	var applied_custom: bool = vehicle.ApplyChassis(CHASSIS_NAME, {"size": 0.60}, null)
	var child2: Node = _first_child(vehicle.chassis_root)
	var layout2_ok := false
	var layout2_detail := "child=%s" % child2
	if child2 != null:
		var rotors2: Node3D = child2.get_node_or_null("Rotors") as Node3D
		var shape2: CollisionShape3D = vehicle.collision_shape
		var shape2_ok := shape2 != null and shape2.shape is BoxShape3D \
			and (shape2.shape as BoxShape3D).size.is_equal_approx(
				Vector3(0.60, BODY_HEIGHT, 0.60))
		if rotors2 != null:
			var fl: Node3D = rotors2.get_node_or_null("RotorFL") as Node3D
			var rr: Node3D = rotors2.get_node_or_null("RotorRR") as Node3D
			if fl != null and rr != null:
				layout2_ok = shape2_ok \
					and absf(fl.position.x - 0.30) < TOL \
					and absf(fl.position.z + 0.30) < TOL \
					and absf(absf(rr.position.x) - 0.30) < TOL \
					and absf(fl.position.y - (BODY_HEIGHT + ROTOR_LIFT)) < TOL
				layout2_detail = "FL=%s RR=%s shape=%s" % [fl.position, rr.position,
					(shape2.shape as BoxShape3D).size if shape2_ok else "?"]
	_check("params-merge-and-relayout", merge_ok and applied_custom and layout2_ok,
		"merged=%s applied=%s %s" % [merged, applied_custom, layout2_detail])

	# B6 换回默认参数（后续运动断言基于 2.0 m/s / 1.0 m/s / 90 °/s）
	var old_child: Node = child2
	var reapplied: bool = vehicle.ApplyChassis(CHASSIS_NAME, {}, null)
	await _wait_physics(1)
	var child3: Node = _first_child(vehicle.chassis_root)
	var defaults_now: Dictionary = vehicle.GetChassisParams()
	_check("reapply-defaults", reapplied and vehicle.chassis_root.get_child_count() == 1
		and child3 != null and child3 != old_child and not is_instance_valid(old_child)
		and _dict_equal(defaults_now, DEFAULT_PARAMS),
		"children=%d params=%s old_valid=%s" % [vehicle.chassis_root.get_child_count(),
			defaults_now, is_instance_valid(old_child)])

	# B7 与 Step 4 契约互操作：无传感器时底盘照常 Step
	var slots: Dictionary = vehicle.GetStreamSlots()
	_check("interop-vehicle-contract", slots.is_empty() and vehicle.HasChassis()
		and vehicle.GetChassisName() == CHASSIS_NAME,
		"slots=%d has=%s name=%s" % [slots.size(), vehicle.HasChassis(),
			vehicle.GetChassisName()])

	# ============================================================ C. 运动语义（黑盒：只看位置 / 朝向）
	# --- hover-idle-300f：核心验收"无重力悬停"（P1）
	_prepare(vehicle, SPAWN_WORLD)
	var y_hover: float = vehicle.global_position.y
	await _wait_physics(300)
	var dy_hover: float = vehicle.global_position.y - y_hover
	_check("hover-idle-300f", absf(dy_hover) < 1e-6
		and vehicle.velocity.length() < 1e-6 and absf(vehicle.rotation.y) < 1e-6,
		"dy=%.8f |v|=%.8f rot_y=%.8f mode=%d" % [dy_hover, vehicle.velocity.length(),
			vehicle.rotation.y, vehicle.motion_mode])

	# --- hover-high-altitude：10 m 处同样零漂移（排除"贴地所以不掉"的解释）
	_prepare(vehicle, HIGH_WORLD)
	var y_high: float = vehicle.global_position.y
	await _wait_physics(300)
	var dy_high: float = vehicle.global_position.y - y_high
	_check("hover-high-altitude", absf(dy_high) < 1e-6
		and absf(vehicle.global_position.y - HIGH_WORLD.z) < TOL,
		"dy=%.8f y=%.4f want=%.2f" % [dy_high, vehicle.global_position.y, HIGH_WORLD.z])

	# --- forward：15 帧证明斜坡、60 帧累计 1.35 m、高度不变（平移 = 悬停中平移）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	var x0: float = vehicle.global_position.x
	var y0: float = vehicle.global_position.y
	await _wait_physics(15)
	var dx15: float = vehicle.global_position.x - x0
	_check("forward-ramp-15f", dx15 > 0.07 and dx15 < 0.13,
		"dx=%.4f (want≈%.3f，恒速会是 0.50 -> 证明斜坡存在)" % [dx15, WANT_FWD_15])
	await _wait_physics(45)
	var dx60: float = vehicle.global_position.x - x0
	_check("forward-60f", dx60 > WANT_FWD_60 * 0.92 and dx60 < WANT_FWD_60 * 1.08,
		"dx=%.4f (want≈%.4f；UGV 同帧数是 1.1375 -> 证明 max_speed=2.0)" % [dx60,
			WANT_FWD_60])
	var dy_fwd: float = vehicle.global_position.y - y0
	_check("forward-holds-altitude", absf(dy_fwd) < TOL,
		"dy=%.6f (平移不影响高度)" % dy_fwd)
	var pose_fwd: Dictionary = vehicle.GetPose()
	_check("forward-pose", absf(vehicle.rotation.y) < TOL
		and absf(float(pose_fwd.get("yaw", 9.0))) < TOL
		and absf(float(pose_fwd.get("yaw", 9.0)) + vehicle.rotation.y) < TOL
		and absf(float(pose_fwd.get("z", 0.0)) - SPAWN_WORLD.z) < TOL,
		"rot_y=%.5f yaw=%.5f pose.z=%.4f" % [vehicle.rotation.y,
			float(pose_fwd.get("yaw", 0.0)), float(pose_fwd.get("z", 0.0))])

	# --- backward 60 帧
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "backward", "speed": 100})
	x0 = vehicle.global_position.x
	y0 = vehicle.global_position.y
	await _wait_physics(60)
	var dx_back: float = vehicle.global_position.x - x0
	_check("backward-60f", dx_back < -WANT_FWD_60 * 0.92 and dx_back > -WANT_FWD_60 * 1.08
		and absf(vehicle.global_position.y - y0) < TOL,
		"dx=%.4f (want≈-%.4f) dy=%.6f" % [dx_back, WANT_FWD_60,
			vehicle.global_position.y - y0])

	# --- yaw-left 60 帧：原地偏航 68.25°，水平与竖直都不动
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "left", "speed": 100})
	var pos_before: Vector3 = vehicle.global_position
	await _wait_physics(60)
	var drot: float = vehicle.rotation.y
	var planar: float = Vector2(vehicle.global_position.x - pos_before.x,
		vehicle.global_position.z - pos_before.z).length()
	var dy_yaw: float = vehicle.global_position.y - pos_before.y
	_check("yaw-left-60f", drot > WANT_YAW_60 * 0.92 and drot < WANT_YAW_60 * 1.08
		and planar < TOL and absf(dy_yaw) < TOL,
		"d_rot_y=%.4f (want≈%.4f) planar=%.6f dy=%.6f" % [drot, WANT_YAW_60, planar,
			dy_yaw])
	var pose_left: Dictionary = vehicle.GetPose()
	_check("yaw-left-pose", absf(float(pose_left.get("yaw", 0.0)) + WANT_YAW_60) < 0.02
		and absf(float(pose_left.get("yaw", 0.0)) + vehicle.rotation.y) < TOL,
		"yaw=%.4f (want≈-%.4f) rot_y=%.4f" % [float(pose_left.get("yaw", 0.0)),
			WANT_YAW_60, vehicle.rotation.y])

	# --- yaw-right 60 帧
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "right", "speed": 100})
	await _wait_physics(60)
	var drot_r: float = vehicle.rotation.y
	_check("yaw-right-60f", drot_r < -WANT_YAW_60 * 0.92 and drot_r > -WANT_YAW_60 * 1.08,
		"d_rot_y=%.4f (want≈-%.4f)" % [drot_r, WANT_YAW_60])

	# ============================================================ C-velocity. 连续速度指令（task_39 §四.5）
	# 与上面 6 个离散 action **并存**：{action:"velocity", v, omega} 只设水平 + 偏航，爬升 0。

	# --- velocity-arc-60f：v 与 omega 同时生效（边走边转的弧线），且不掉高度
	_prepare(vehicle, SPAWN_WORLD)
	var ch_v: Node = _first_child(vehicle.chassis_root)
	vehicle.SetMotionCommand({"action": "velocity", "v": 1.0, "omega": 0.5})
	var pos_arc0: Vector3 = vehicle.global_position
	await _wait_physics(60)
	var planar_arc: float = Vector2(vehicle.global_position.x - pos_arc0.x,
		vehicle.global_position.z - pos_arc0.z).length()
	var dy_arc: float = vehicle.global_position.y - pos_arc0.y
	_check("velocity-arc-60f", ch_v != null
		and absf(float(ch_v.call("GetSpeed")) - 1.0) < 0.05
		and absf(float(ch_v.call("GetYawRate")) - 0.5) < 0.05
		and vehicle.rotation.y > 0.3 and planar_arc > 0.5 and absf(dy_arc) < TOL,
		"v=%s omega=%s rot_y=%.4f planar=%.4f dy=%.6f" % [
			ch_v.call("GetSpeed") if ch_v != null else "null",
			ch_v.call("GetYawRate") if ch_v != null else "null",
			vehicle.rotation.y, planar_arc, dy_arc])

	# --- velocity-sign-right：负 omega = 右转（rotation.y 减小），与 action="right" 同向
	_prepare(vehicle, SPAWN_WORLD)
	ch_v = _first_child(vehicle.chassis_root)
	vehicle.SetMotionCommand({"action": "velocity", "v": 0.0, "omega": -0.5})
	await _wait_physics(30)
	_check("velocity-sign-right", ch_v != null and vehicle.rotation.y < -0.1
		and absf(float(ch_v.call("GetYawRate")) + 0.5) < 0.05,
		"rot_y=%.4f yaw_rate=%s (want≈-0.5)" % [vehicle.rotation.y,
			ch_v.call("GetYawRate") if ch_v != null else "null"])

	# --- velocity-clamps：两轴各自 clampf 到 ±max_speed / ±max_yaw_rate，不放大
	_prepare(vehicle, SPAWN_WORLD)
	ch_v = _first_child(vehicle.chassis_root)
	vehicle.SetMotionCommand({"action": "velocity", "v": 99.0, "omega": -999.0})
	await _wait_physics(60)
	_check("velocity-clamps", ch_v != null
		and absf(float(ch_v.call("GetSpeed")) - 2.0) < 1e-3
		and absf(float(ch_v.call("GetYawRate")) + PI * 0.5) < 1e-3,
		"v=%s (want 2.0) yaw_rate=%s (want %.4f)" % [
			ch_v.call("GetSpeed") if ch_v != null else "null",
			ch_v.call("GetYawRate") if ch_v != null else "null", -PI * 0.5])

	# --- velocity-zero-hovers：先 forward 100 跑起来，再 v=omega=0 → 停住悬停（执行器"无目标"那条）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	await _wait_physics(60)
	vehicle.SetMotionCommand({"action": "velocity", "v": 0.0, "omega": 0.0})
	await _wait_physics(60)
	var pos_vz0: Vector3 = vehicle.global_position
	await _wait_physics(30)
	var planar_vz: float = Vector2(vehicle.global_position.x - pos_vz0.x,
		vehicle.global_position.z - pos_vz0.z).length()
	var dy_vz: float = vehicle.global_position.y - pos_vz0.y
	_check("velocity-zero-hovers", planar_vz < TOL and absf(vehicle.rotation.y) < TOL
		and absf(dy_vz) < TOL,
		"planar=%.6f rot_y=%.6f dy=%.6f（斜坡 0.67 s 内必须已经停住）" % [planar_vz,
			vehicle.rotation.y, dy_vz])

	# --- velocity-missing-fields：只有 action、没有 v / omega → 容错取 0（不崩、不动）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "velocity"})
	var pos_mf0: Vector3 = vehicle.global_position
	await _wait_physics(30)
	var planar_mf: float = Vector2(vehicle.global_position.x - pos_mf0.x,
		vehicle.global_position.z - pos_mf0.z).length()
	_check("velocity-missing-fields", is_instance_valid(vehicle) and planar_mf < TOL
		and absf(vehicle.rotation.y) < TOL,
		"valid=%s planar=%.6f rot_y=%.6f" % [is_instance_valid(vehicle), planar_mf,
			vehicle.rotation.y])

	# --- climb 60 帧：竖直 0.8417 m，水平不动
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "up", "speed": 100})
	var pos_climb0: Vector3 = vehicle.global_position
	await _wait_physics(60)
	var dz_climb: float = vehicle.global_position.y - pos_climb0.y
	var dxy_climb: float = Vector2(vehicle.global_position.x - pos_climb0.x,
		vehicle.global_position.z - pos_climb0.z).length()
	_check("climb-60f", dz_climb > WANT_CLIMB_60 * 0.92 and dz_climb < WANT_CLIMB_60 * 1.08
		and dxy_climb < TOL,
		"dz=%.4f (want≈%.4f) dxy=%.6f" % [dz_climb, WANT_CLIMB_60, dxy_climb])
	var pose_climb: Dictionary = vehicle.GetPose()
	_check("climb-pose", absf(float(pose_climb.get("z", 0.0)) - vehicle.global_position.y) < TOL
		and float(pose_climb.get("z", 0.0)) > SPAWN_WORLD.z,
		"pose.z=%.4f godot.y=%.4f（高度 = 世界 z）" % [float(pose_climb.get("z", 0.0)),
			vehicle.global_position.y])

	# --- climb-stop-settles：爬升中 stop -> 竖直速度归零 -> 高度保持（不是掉回地面）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "up", "speed": 100})
	await _wait_physics(60)
	vehicle.SetMotionCommand({"action": "stop"})
	await _wait_physics(60)
	var vz_after_stop: float = vehicle.velocity.y
	var y_settled: float = vehicle.global_position.y
	await _wait_physics(30)
	var dz_settled: float = vehicle.global_position.y - y_settled
	_check("climb-stop-settles", absf(vz_after_stop) < 1e-3 and absf(dz_settled) < 1e-4
		and y_settled > SPAWN_WORLD.z + 0.5,
		"vz=%.6f dz=%.8f y=%.4f" % [vz_after_stop, dz_settled, y_settled])

	# --- descend 60 帧：-0.8417 m
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "down", "speed": 100})
	var y_desc0: float = vehicle.global_position.y
	await _wait_physics(60)
	var dz_desc: float = vehicle.global_position.y - y_desc0
	_check("descend-60f", dz_desc < -WANT_CLIMB_60 * 0.92 and dz_desc > -WANT_CLIMB_60 * 1.08,
		"dz=%.4f (want≈-%.4f)" % [dz_desc, WANT_CLIMB_60])

	# --- land-on-ground / land-still：持续下降落到地面（P4），不打穿、不抖
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "down", "speed": 100})
	await _wait_physics(240)
	var z_land: float = float(vehicle.GetPose().get("z", -1.0))
	_check("land-on-ground", z_land >= 0.0 and z_land <= 0.02,
		"pose.z=%.4f (want ∈ [0, 0.02]) godot.y=%.4f" % [z_land,
			vehicle.global_position.y])
	var y_land: float = vehicle.global_position.y
	await _wait_physics(60)
	var dz_land: float = vehicle.global_position.y - y_land
	_check("land-still", absf(dz_land) < 1e-4 and vehicle.global_position.y > -0.005,
		"dz=%.8f y=%.5f" % [dz_land, vehicle.global_position.y])

	# --- stop-then-still：停住后完全不动（水平与竖直都停）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	await _wait_physics(60)
	vehicle.SetMotionCommand({"action": "stop"})
	await _wait_physics(60)
	var pos_stop: Vector3 = vehicle.global_position
	await _wait_physics(30)
	var d_stop: float = (vehicle.global_position - pos_stop).length()
	_check("stop-then-still", d_stop < TOL and vehicle.velocity.length() < 1e-3
		and absf(vehicle.global_position.y - pos_stop.y) < TOL,
		"moved=%.6f |v|=%.6f" % [d_stop, vehicle.velocity.length()])

	# --- speed-clamped：speed=250 -> clamp 到 100%（不是 2.5 倍速）；120 帧把差别拉开
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 250})
	x0 = vehicle.global_position.x
	await _wait_physics(120)
	var dx_over: float = vehicle.global_position.x - x0
	_check("speed-clamped", dx_over > WANT_FWD_120 * 0.92 and dx_over < WANT_FWD_120 * 1.08,
		"dx=%.4f (want≈%.4f，不 clamp 会是 5.875)" % [dx_over, WANT_FWD_120])

	# --- zero-speed-still：speed=0 / 缺 speed：三轴都不动
	for cmd: Dictionary in [{"action": "forward", "speed": 0},
			{"action": "forward"}, {"action": "backward", "speed": 0}]:
		_prepare(vehicle, SPAWN_WORLD)
		vehicle.SetMotionCommand(cmd)
		var q0: Vector3 = vehicle.global_position
		await _wait_physics(30)
		_check("zero-speed-still", (vehicle.global_position - q0).length() < TOL,
			"cmd=%s moved=%.6f" % [cmd, (vehicle.global_position - q0).length()])

	# --- bad-speed-tolerated：非数字 speed 不崩、不动
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": "abc"})
	var r0: Vector3 = vehicle.global_position
	await _wait_physics(30)
	_check("bad-speed-tolerated", (vehicle.global_position - r0).length() < TOL
		and is_instance_valid(vehicle),
		"moved=%.6f" % [(vehicle.global_position - r0).length()])

	# --- rapid-command-switch：forward -> stop -> backward 各 5 帧，只保留最新指令
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	await _wait_physics(5)
	vehicle.SetMotionCommand({"action": "stop"})
	await _wait_physics(5)
	vehicle.SetMotionCommand({"action": "backward", "speed": 100})
	await _wait_physics(5)
	_check("rapid-command-switch", is_instance_valid(vehicle) and vehicle.velocity.x < -0.1,
		"v=%s" % [vehicle.velocity])

	# --- unknown-action-stops：未知 action = 三轴目标 0（悬停）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "teleport", "speed": 100})
	var u0: Vector3 = vehicle.global_position
	await _wait_physics(30)
	_check("unknown-action-stops", (vehicle.global_position - u0).length() < TOL
		and absf(vehicle.rotation.y) < TOL and is_instance_valid(vehicle),
		"moved=%.6f rot_y=%.6f" % [(vehicle.global_position - u0).length(),
			vehicle.rotation.y])

	# --- up-clears-horizontal：单 action 语义（up 会把水平目标清零，斜坡下行 ~0.667 m）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	await _wait_physics(60)
	var pos_mix0: Vector3 = vehicle.global_position
	vehicle.SetMotionCommand({"action": "up", "speed": 100})
	await _wait_physics(60)
	var dz_mix: float = vehicle.global_position.y - pos_mix0.y
	var drift_mix: float = Vector2(vehicle.global_position.x - pos_mix0.x,
		vehicle.global_position.z - pos_mix0.z).length()
	_check("up-clears-horizontal", dz_mix > WANT_CLIMB_60 * 0.92
		and dz_mix < WANT_CLIMB_60 * 1.08
		and drift_mix > DRIFT_MIN and drift_mix < DRIFT_MAX
		and absf(vehicle.velocity.x) < 1e-3 and absf(vehicle.velocity.z) < 1e-3,
		"dz=%.4f (want≈%.4f) drift=%.4f (want ∈ [%.2f, %.2f]) v.x=%.6f" % [dz_mix,
			WANT_CLIMB_60, drift_mix, DRIFT_MIN, DRIFT_MAX, vehicle.velocity.x])

	# ============================================================ D. 边界 / 白盒 / 契约
	# D1 白盒 lint：底盘脚本不得越过 D8（写位置 / 推进物理 / 施加重力），且必须用 FLOATING
	var quad_code: String = _code_of(CHASSIS_SCRIPT)
	var cat_code: String = _code_of(CATALOG_SCRIPT)
	var banned_hits: Array[String] = []
	for token: String in BANNED_TOKENS:
		if quad_code.find(token) >= 0:
			banned_hits.append(token)
	_check("white-box-boundaries", banned_hits.is_empty()
		and quad_code.find("MOTION_MODE_FLOATING") >= 0
		and quad_code.find("Approach(") >= 0
		and cat_code.find("load(") >= 0 and cat_code.find("preload(") < 0,
		"banned=%s floating=%s approach=%s catalog_load=%s" % [banned_hits,
			quad_code.find("MOTION_MODE_FLOATING") >= 0,
			quad_code.find("Approach(") >= 0, cat_code.find("load(") >= 0])

	# D2 树外 Step：不报 !is_inside_tree，返回零控制量（P8）
	var lite: Node = packed.instantiate()
	var lite_ctl: Variant = lite.call("Step", Vector3.ZERO, DT)
	var lite_ok: bool = lite_ctl is Dictionary
	if lite_ok:
		var lite_dict: Dictionary = lite_ctl
		var lite_vel: Variant = lite_dict.get("velocity", null)
		lite_ok = (lite_vel is Vector3 and (lite_vel as Vector3).is_zero_approx()
			and absf(float(lite_dict.get("yaw_rate", 9.0))) < 1e-6)
	lite.free()
	_check("step-outside-tree", lite_ok, "ctl=%s" % [lite_ctl])

	# D3 Configure(null, {}) 不崩；空指令视为 stop（三轴归零）
	var bare: Node = packed.instantiate()
	bare.call("Configure", null, {})
	var bare_alive: bool = is_instance_valid(bare)
	bare.call("SetCommand", {"action": "forward", "speed": 100})
	var go: Dictionary = bare.call("Step", Vector3.ZERO, DT)
	bare.call("SetCommand", {})
	var halt: Dictionary = bare.call("Step", Vector3.ZERO, 1.0)
	var go_vel: Vector3 = go.get("velocity", Vector3.ZERO)
	var halt_vel: Vector3 = halt.get("velocity", Vector3.ZERO)
	_check("null-vehicle-and-empty-cmd", bare_alive
		and go_vel.length() > 0.0 and halt_vel.length() < 1e-6
		and absf(float(halt.get("yaw_rate", 9.0))) < 1e-6,
		"go=%s halt=%s" % [go_vel, halt_vel])
	bare.free()

	# ============================================================ D5. 高度软件限幅（task_32 Step 3）
	# 位置说明：D5 排在 D4 之前，因为 D4 是「全程无自检装置层面错误」的汇总闸，必须最后跑。
	# E1 越界即夹：停在 31.9（> 上限 31.8）不发任何指令，3 帧内被拉回 31.8 且竖直速度为 0。
	_prepare(vehicle, Vector3(64.0, 64.0, 31.9))
	await _wait_physics(3)
	var y_pin: float = vehicle.global_position.y
	_check("clamp-ceiling-pins",
		y_pin >= CLAMP_CEILING_M - 0.001 and y_pin <= CLAMP_CEILING_M + 0.001
		and absf(vehicle.velocity.y) < 1e-6,
		"y=%.4f want=%.4f vz=%.8f" % [y_pin, CLAMP_CEILING_M, vehicle.velocity.y])

	# E2 全速爬升也压不穿顶盖：从 31.5 满杆 up 60 帧，逐帧取最大值（容忍 ≤1 帧物理推进的溢出）。
	_prepare(vehicle, Vector3(64.0, 64.0, 31.5))
	vehicle.SetMotionCommand({"action": "up", "speed": 100})
	var y_max: float = vehicle.global_position.y
	for _i in range(60):
		await physics_frame
		y_max = maxf(y_max, vehicle.global_position.y)
	_check("clamp-ceiling-holds-under-climb",
		y_max <= CLAMP_CEILING_M + 0.02 and y_max > 31.6,
		"y_max=%.4f ceiling=%.4f（不 clamp 会到 ≈32.34）" % [y_max, CLAMP_CEILING_M])

	# E3 上限不是魔数：Quadrotor.CLAMP_MARGIN_M 与 World.AIR_WALL_HEIGHT_M 相加 == 本文件的 31.8。
	var chassis_script: Script = load(CHASSIS_SCRIPT) as Script
	var world_script: Script = load(WORLD_SCRIPT) as Script
	var margin_now: float = -1.0
	var air_height_now: float = -1.0
	if chassis_script != null:
		margin_now = float(chassis_script.get_script_constant_map().get("CLAMP_MARGIN_M", -1.0))
	if world_script != null:
		air_height_now = float(
			world_script.get_script_constant_map().get("AIR_WALL_HEIGHT_M", -1.0))
	_check("clamp-ceiling-derivation",
		absf(margin_now - CLAMP_MARGIN_M) < 1e-9 and absf(air_height_now - 32.0) < 1e-9
		and absf(air_height_now - margin_now - CLAMP_CEILING_M) < 1e-9,
		"margin=%.3f air=%.3f ceiling=%.3f" % [margin_now, air_height_now,
			CLAMP_CEILING_M])

	# D4 全程无自检装置层面的错误
	_check("no-harness-errors", _harness_errors == 0 and is_instance_valid(vehicle)
		and vehicle.HasChassis(),
		"harness_errors=%d valid=%s has=%s" % [_harness_errors,
			is_instance_valid(vehicle), vehicle.HasChassis()])

	# ============================================================ 清理（退出时无 ERROR）
	vehicle.SetMotionCommand({"action": "stop"})
	vehicle.queue_free()
	await process_frame
	await process_frame
	world.queue_free()
	await process_frame
	await process_frame
	_finish()


# ================================================================ 测试装置

## 每段独立运动前复位：先对**旧**底盘发 stop -> 重装底盘（Configure 清零三轴状态）
## -> 摆正位姿（world 坐标，z = 高度）与朝向。新底盘因此"从未收到过指令"。
func _prepare(vehicle: CharacterBody3D, world_pos: Vector3) -> void:
	if vehicle == null or not is_instance_valid(vehicle):
		return
	vehicle.SetMotionCommand({"action": "stop"})
	vehicle.ApplyChassis(CHASSIS_NAME, {}, null)
	vehicle.global_position = WorldTransform.PosToGodot(world_pos)
	vehicle.rotation.y = 0.0
	vehicle.velocity = Vector3.ZERO


func _first_child(parent: Node) -> Node:
	if parent == null or parent.get_child_count() == 0:
		return null
	return parent.get_child(0)


## 字典逐键逐类型相同（键集 + 类型 + 值）；用于默认参数比对。
func _dict_equal(a: Dictionary, b: Dictionary) -> bool:
	if a.size() != b.size():
		return false
	for key: Variant in a.keys():
		if not b.has(key):
			return false
		if typeof(a[key]) != typeof(b[key]):
			return false
		if a[key] != b[key]:
			return false
	return true


## 去掉以 # 开头的注释行，只留可执行代码（白盒 lint 用）。
func _strip_comment_lines(source: String) -> String:
	var code := ""
	for raw_line: String in source.split("\n"):
		if raw_line.strip_edges().begins_with("#"):
			continue
		code += raw_line + "\n"
	return code


## 读脚本源码并去注释（脚本不存在 -> 空串）。
func _code_of(path: String) -> String:
	var script: Script = load(path) as Script
	if script == null:
		return ""
	return _strip_comment_lines(script.source_code)


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		await physics_frame


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
