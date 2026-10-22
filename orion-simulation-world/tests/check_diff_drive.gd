# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# DiffDrive 差速底盘自检（Step 5）：场景形态 / 目录与装配 / 运动语义 / 撞墙会停 / 边界与白盒。
# 用法：timeout 180 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://tests/check_diff_drive.gd
# 期望：failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR（COMPONENTS.md §11 Step 5）。
#
# 决定本文件写法的实测事实（plan.md §2.2）：
#   P3  真实 World.tscn 里 0.30 m 长的车身从 world x=126.5 恒速东进，停在 godot x=127.3499
#       （车头 127.4999，墙面 127.5）→ "撞墙会停" 用**位置**断言，不用速度。
#   P4  move_and_slide() 撞墙后自己把 velocity 归零 → velocity≈0 不能当撞墙判据。
#   P5  CylinderMesh 只有 top_radius / bottom_radius / height（没有 radius）。
#   P6  CharacterBody3D.MOTION_MODE_GROUNDED 是默认值 → 必须先置 FLOATING 再断言被改回。
#   P8  未入树访问 global_position 会报 ERROR → 一律 "先 add_child 再设 global_position"；
#       Step() 内部的树外兜底也要断言（不报错）。
#   P15 含 await 的辅助函数必须被 await 调用；本文件所有辅助函数都不含 await。
#   P16 一次 null 解引用会让协程中断、quit() 不执行 → 入口先判空，命令带 timeout。
#
# 所有断言都用 --fixed-fps 60 下的累计量（15/30/60/240 帧），不逐帧比对瞬时值；
# 容差 = ±10% 或 1e-3 绝对（物理积分 + move_and_slide 的 safe_margin）。

extends SceneTree

const WorldTransform := preload("res://scripts/common/WorldTransform.gd")

const CHASSIS_SCENE := "res://chassis/DiffDrive.tscn"
const CHASSIS_SCRIPT := "res://scripts/chassis/DiffDrive.gd"
const BASE_SCRIPT := "res://scripts/chassis/ChassisModel.gd"
const CATALOG_SCRIPT := "res://scripts/chassis/ChassisCatalog.gd"
const VEHICLE_SCENE := "res://Vehicle.tscn"
const WORLD_SCENE := "res://World.tscn"

const CHASSIS_NAME := "diff_drive_ugv"
const CHASSIS_OTHER := "quadrotor_uav"
const WHEEL_NAMES := ["WheelFL", "WheelFR", "WheelRL", "WheelRR"]

const TOL := 0.001
## dt = 1/60（--fixed-fps 60）；斜坡到顶时间 = 30 帧（0.5 s）。
const DT := 1.0 / 60.0
const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)
## 撞墙起点：world (126.5, 64) 朝 +x，车头 126.65 → 墙面 127.5，需前进 0.85 m。
const WALL_RUN_WORLD := Vector3(126.5, 64.0, 0.0)
const WALL_X := 127.5
## 解析值（plan.md §6）：60 帧前进位移 / 15 帧位移 / 60 帧左转角度。
const WANT_FWD_60 := 1.1375
const WANT_FWD_15 := 0.10
const WANT_TURN_60 := 1.1912
## 撞墙后的停位与倒车位移（P3 实测 + 解析）。
const WANT_WALL_X := 127.35
const WANT_BACK_30 := 0.3875

const DEFAULT_PARAMS := {
	"max_speed": 1.5,
	"max_turn_rate": 90.0,
	"wheel_base": 0.30,
	"track_width": 0.24,
	"height": 0.16,
}

## A35 白盒 lint：三个底盘脚本里**不允许**出现的符号（越界 / 违反 D8 / §9.1）。
const BANNED_TOKENS := [
	"class_name", "move_and_slide", "global_position", "intersect_ray",
	"direct_space_state", "WebSocketPeer",
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
	_check("scene-loads", true, "%s -> %s" % [CHASSIS_SCENE, probe.get_class()])

	# A2 根节点：名字 / 类型 / 脚本
	var probe_script: Script = probe.get_script()
	_check("root-type", probe.name == "DiffDrive" and probe is Node3D
		and probe_script != null and probe_script.resource_path == CHASSIS_SCRIPT,
		"name=%s class=%s script=%s" % [probe.name, probe.get_class(),
			probe_script.resource_path if probe_script != null else "null"])

	# A3 节点树 + 轮子外观（CylinderMesh r=0.06 h=0.04，轴向靠 rotation_degrees.x=90）
	var wheels: Node3D = probe.get_node_or_null("Wheels") as Node3D
	var wheels_ok := wheels != null and wheels is Node3D
	var wheels_detail := "Wheels=%s" % wheels
	if wheels_ok:
		for i in range(WHEEL_NAMES.size()):
			var wheel: MeshInstance3D = wheels.get_node_or_null(WHEEL_NAMES[i]) as MeshInstance3D
			if wheel == null:
				wheels_ok = false
				wheels_detail += " 缺 %s" % WHEEL_NAMES[i]
				continue
			var cyl: CylinderMesh = wheel.mesh as CylinderMesh
			if cyl == null:
				wheels_ok = false
				wheels_detail += " %s.mesh=%s" % [WHEEL_NAMES[i], wheel.mesh]
				continue
			wheels_ok = wheels_ok \
				and absf(cyl.top_radius - 0.06) < TOL \
				and absf(cyl.bottom_radius - 0.06) < TOL \
				and absf(cyl.height - 0.04) < TOL \
				and absf(wheel.rotation_degrees.x - 90.0) < TOL
			wheels_detail += " %s(r=%.3f,h=%.3f,rotx=%.1f)" % [WHEEL_NAMES[i],
				cyl.top_radius, cyl.height, wheel.rotation_degrees.x]
	_check("wheels-shape", wheels_ok, wheels_detail)

	# A4 默认摆位（wheel_base=0.30 / track_width=0.24，前 +X、右 +Z）
	var layout_ok := wheels != null
	var layout_detail := ""
	var want_pos := [Vector3(0.15, 0.06, -0.12), Vector3(0.15, 0.06, 0.12),
		Vector3(-0.15, 0.06, -0.12), Vector3(-0.15, 0.06, 0.12)]
	if wheels != null:
		for i in range(WHEEL_NAMES.size()):
			var wheel: Node3D = wheels.get_node_or_null(WHEEL_NAMES[i]) as Node3D
			if wheel == null:
				layout_ok = false
				continue
			layout_ok = layout_ok and wheel.position.distance_to(want_pos[i]) < TOL
			layout_detail += " %s=%s" % [WHEEL_NAMES[i], wheel.position]
	_check("wheels-layout", layout_ok, layout_detail)

	# A5 四个契约方法 + 基类路径
	var methods_ok: bool = probe.has_method("Configure") and probe.has_method("GetDefaultParams") \
		and probe.has_method("SetCommand") and probe.has_method("Step")
	var base: Script = probe_script.get_base_script() if probe_script != null else null
	_check("contract-methods", methods_ok and base != null and base.resource_path == BASE_SCRIPT,
		"methods=%s base=%s" % [methods_ok, base.resource_path if base != null else "null"])

	# A6 默认参数（与目录表逐键一致）
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

	# A8 / A9 目录
	var has_dd: bool = bool(catalog.call("Has", CHASSIS_NAME)) if catalog != null else false
	var has_quad: bool = bool(catalog.call("Has", CHASSIS_OTHER)) if catalog != null else false
	var has_bogus: bool = bool(catalog.call("Has", "no_such_chassis")) if catalog != null else true
	var bogus_defaults: Variant = null
	if catalog != null:
		bogus_defaults = catalog.call("GetDefaults", "no_such_chassis")
	_check("catalog-has", catalog != null and has_dd and has_quad and not has_bogus
		and bogus_defaults is Dictionary and (bogus_defaults as Dictionary).is_empty(),
		"dd=%s quad=%s bogus=%s defaults=%s" % [has_dd, has_quad, has_bogus,
			bogus_defaults])

	var scene_a: Variant = catalog.call("GetScene", CHASSIS_NAME)
	var scene_b: Variant = catalog.call("GetScene", CHASSIS_NAME)
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

	# A10 装配（不传场景 -> 走目录解析）；先把它设成 FLOATING（P6：GROUNDED 是默认值）
	vehicle.motion_mode = CharacterBody3D.MOTION_MODE_FLOATING
	var mode_before: int = vehicle.motion_mode
	var applied: bool = vehicle.ApplyChassis(CHASSIS_NAME, {"max_speed": 1.2}, null)
	var child: Node = _first_child(vehicle.chassis_root)
	_check("apply-chassis", applied and vehicle.HasChassis()
		and vehicle.GetChassisName() == CHASSIS_NAME
		and vehicle.chassis_root.get_child_count() == 1
		and child != null and child.name == "DiffDrive",
		"applied=%s name=%s children=%d child=%s" % [applied, vehicle.GetChassisName(),
			vehicle.chassis_root.get_child_count(), child])

	if child == null:
		_fatal("chassis-installed", "目录装配失败，后续断言无法进行")
		_finish()
		return

	# A11 GROUNDED 是 Configure 真改的（不是默认值蒙对）
	_check("motion-mode-grounded", mode_before == CharacterBody3D.MOTION_MODE_FLOATING
		and vehicle.motion_mode == CharacterBody3D.MOTION_MODE_GROUNDED,
		"before=%d after=%d want=%d" % [mode_before, vehicle.motion_mode,
			CharacterBody3D.MOTION_MODE_GROUNDED])

	# A12 车身 / 碰撞形状：BoxShape3D(wheel_base, height, track_width)，原点 = 底面中心
	var body: MeshInstance3D = vehicle.body
	var shape: CollisionShape3D = vehicle.collision_shape
	var size_ok := false
	var size_detail := "body/shape 缺失"
	if body != null and shape != null and body.mesh is BoxMesh and shape.shape is BoxShape3D:
		var box: BoxMesh = body.mesh as BoxMesh
		var shape_box: BoxShape3D = shape.shape as BoxShape3D
		size_ok = box.size.is_equal_approx(Vector3(0.30, 0.16, 0.24)) \
			and shape_box.size.is_equal_approx(Vector3(0.30, 0.16, 0.24)) \
			and absf(body.position.y - 0.08) < 1e-4 \
			and absf(shape.position.y - 0.08) < 1e-4
		size_detail = "mesh=%s shape=%s body.y=%.4f shape.y=%.4f" % [box.size,
			shape_box.size, body.position.y, shape.position.y]
	_check("body-shape", size_ok, size_detail)

	# A13 参数合并 + 轮子按自定义尺寸重新摆位
	var merged: Dictionary = vehicle.GetChassisParams()
	var merge_ok := absf(float(merged.get("max_speed", 0.0)) - 1.2) < TOL \
		and absf(float(merged.get("track_width", 0.0)) - 0.24) < TOL \
		and absf(float(merged.get("max_turn_rate", 0.0)) - 90.0) < TOL
	var applied_custom: bool = vehicle.ApplyChassis(CHASSIS_NAME,
		{"wheel_base": 0.40, "track_width": 0.20}, null)
	var child2: Node = _first_child(vehicle.chassis_root)
	var layout2_ok := false
	var layout2_detail := "child=%s" % child2
	if child2 != null:
		var wheels2: Node3D = child2.get_node_or_null("Wheels") as Node3D
		if wheels2 != null:
			var fl: Node3D = wheels2.get_node_or_null("WheelFL") as Node3D
			var fr: Node3D = wheels2.get_node_or_null("WheelFR") as Node3D
			if fl != null and fr != null:
				layout2_ok = absf(fl.position.x - 0.20) < TOL \
					and absf(fl.position.z + 0.10) < TOL \
					and absf(fr.position.z - 0.10) < TOL
				layout2_detail = "FL=%s FR=%s" % [fl.position, fr.position]
	_check("params-merge-and-relayout", merge_ok and applied_custom and layout2_ok,
		"merged=%s applied=%s %s" % [merged, applied_custom, layout2_detail])

	# A14 换回默认参数（后续运动断言基于 1.5 m/s / 90 °/s）
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

	# ============================================================ C. 运动语义（黑盒：只看位置 / 朝向）
	# --- A15/A16/A17 前进：15 帧证明斜坡、60 帧累计 1.1375 m
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	var x0: float = vehicle.global_position.x
	var y0: float = vehicle.global_position.y
	await _wait_physics(15)
	var dx15: float = vehicle.global_position.x - x0
	_check("forward-ramp-15f", dx15 > 0.07 and dx15 < 0.13,
		"dx=%.4f (want≈%.3f，恒速会是 0.375 -> 证明斜坡存在)" % [dx15, WANT_FWD_15])
	await _wait_physics(45)
	var dx60: float = vehicle.global_position.x - x0
	_check("forward-60f", dx60 > WANT_FWD_60 * 0.92 and dx60 < WANT_FWD_60 * 1.08,
		"dx=%.4f (want≈%.4f)" % [dx60, WANT_FWD_60])
	var pose_fwd: Dictionary = vehicle.GetPose()
	_check("forward-pose", absf(vehicle.rotation.y) < TOL
		and absf(float(pose_fwd.get("yaw", 9.0))) < TOL
		and absf(float(pose_fwd.get("yaw", 9.0)) + vehicle.rotation.y) < TOL
		and absf(vehicle.global_position.y - y0) < TOL,
		"rot_y=%.5f yaw=%.5f dy=%.6f" % [vehicle.rotation.y,
			float(pose_fwd.get("yaw", 0.0)), vehicle.global_position.y - y0])

	# --- A18 后退 60 帧
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "backward", "speed": 100})
	x0 = vehicle.global_position.x
	await _wait_physics(60)
	var dx_back: float = vehicle.global_position.x - x0
	_check("backward-60f", dx_back < -WANT_FWD_60 * 0.92 and dx_back > -WANT_FWD_60 * 1.08
		and absf(vehicle.rotation.y) < TOL,
		"dx=%.4f (want≈-%.4f) rot_y=%.5f" % [dx_back, WANT_FWD_60, vehicle.rotation.y])

	# --- A19/A20 左转 60 帧：原地转 + 世界 yaw 反号
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "left", "speed": 100})
	var pos_before: Vector3 = vehicle.global_position
	await _wait_physics(60)
	var drot: float = vehicle.rotation.y
	var planar: float = Vector2(vehicle.global_position.x - pos_before.x,
		vehicle.global_position.z - pos_before.z).length()
	_check("left-turn-60f", drot > WANT_TURN_60 * 0.92 and drot < WANT_TURN_60 * 1.08
		and planar < 1e-3,
		"d_rot_y=%.4f (want≈%.4f) planar=%.6f" % [drot, WANT_TURN_60, planar])
	var pose_left: Dictionary = vehicle.GetPose()
	_check("left-turn-pose-yaw", absf(float(pose_left.get("yaw", 0.0)) + WANT_TURN_60) < 0.02
		and absf(float(pose_left.get("yaw", 0.0)) + vehicle.rotation.y) < TOL,
		"yaw=%.4f (want≈-%.4f) rot_y=%.4f" % [float(pose_left.get("yaw", 0.0)),
			WANT_TURN_60, vehicle.rotation.y])

	# --- A21 右转 60 帧
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "right", "speed": 100})
	await _wait_physics(60)
	var drot_r: float = vehicle.rotation.y
	_check("right-turn-60f", drot_r < -WANT_TURN_60 * 0.92 and drot_r > -WANT_TURN_60 * 1.08,
		"d_rot_y=%.4f (want≈-%.4f)" % [drot_r, WANT_TURN_60])

	# --- A22 stop：减速到 0 后 30 帧完全不动
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	await _wait_physics(60)
	vehicle.SetMotionCommand({"action": "stop"})
	await _wait_physics(60)
	var x_stop: float = vehicle.global_position.x
	await _wait_physics(30)
	var dx_stop: float = absf(vehicle.global_position.x - x_stop)
	_check("stop-then-still", dx_stop < TOL and vehicle.velocity.length() < TOL,
		"dx=%.6f |v|=%.6f" % [dx_stop, vehicle.velocity.length()])

	# --- A23/A24 up / down：UGV 无竖直自由度，完全不动
	for action: String in ["up", "down"]:
		_prepare(vehicle, SPAWN_WORLD)
		vehicle.SetMotionCommand({"action": action, "speed": 100})
		var p0: Vector3 = vehicle.global_position
		await _wait_physics(30)
		var moved: float = (vehicle.global_position - p0).length()
		_check("no-vertical-" + action, moved < TOL and absf(vehicle.rotation.y) < TOL,
			"moved=%.6f rot_y=%.6f" % [moved, vehicle.rotation.y])

	# --- A25 未知 action = stop
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "teleport", "speed": 100})
	var px: float = vehicle.global_position.x
	await _wait_physics(30)
	_check("unknown-action-stops", absf(vehicle.global_position.x - px) < TOL
		and absf(vehicle.rotation.y) < TOL and is_instance_valid(vehicle),
		"dx=%.6f rot_y=%.6f" % [vehicle.global_position.x - px, vehicle.rotation.y])

	# --- A26 speed=250 -> clamp 到 100%（不是 2.5 倍速）
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 250})
	x0 = vehicle.global_position.x
	await _wait_physics(60)
	var dx_over: float = vehicle.global_position.x - x0
	_check("speed-clamped", dx_over > WANT_FWD_60 * 0.92 and dx_over < WANT_FWD_60 * 1.08,
		"dx=%.4f (want≈%.4f，2.5 倍速会是 2.84)" % [dx_over, WANT_FWD_60])

	# --- A27 speed=0 / 缺 speed：不动
	for cmd: Dictionary in [{"action": "forward", "speed": 0},
			{"action": "forward"}, {"action": "backward", "speed": 0}]:
		_prepare(vehicle, SPAWN_WORLD)
		vehicle.SetMotionCommand(cmd)
		var q0: Vector3 = vehicle.global_position
		await _wait_physics(30)
		_check("zero-speed-still", (vehicle.global_position - q0).length() < TOL,
			"cmd=%s moved=%.6f" % [cmd, (vehicle.global_position - q0).length()])

	# --- A28 非数字 speed：不崩、不动
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": "abc"})
	var r0: Vector3 = vehicle.global_position
	await _wait_physics(30)
	_check("bad-speed-tolerated", (vehicle.global_position - r0).length() < TOL
		and is_instance_valid(vehicle),
		"moved=%.6f" % [(vehicle.global_position - r0).length()])

	# --- A29 频繁改指令：forward -> stop -> backward 各 5 帧，末帧速度方向为负
	_prepare(vehicle, SPAWN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	await _wait_physics(5)
	vehicle.SetMotionCommand({"action": "stop"})
	await _wait_physics(5)
	vehicle.SetMotionCommand({"action": "backward", "speed": 100})
	await _wait_physics(5)
	_check("rapid-command-switch", is_instance_valid(vehicle) and vehicle.velocity.x < -0.1,
		"v=%s" % [vehicle.velocity])

	# --- A30 从未下发指令：默认 stop
	_prepare(vehicle, SPAWN_WORLD)
	var s0: Vector3 = vehicle.global_position
	await _wait_physics(30)
	_check("no-command-default-stop", (vehicle.global_position - s0).length() < TOL
		and absf(vehicle.rotation.y) < TOL,
		"moved=%.6f" % [(vehicle.global_position - s0).length()])

	# ============================================================ D. 撞墙会停（本 step 核心验收）
	_prepare(vehicle, WALL_RUN_WORLD)
	vehicle.SetMotionCommand({"action": "forward", "speed": 100})
	await _wait_physics(240)
	var pose_wall: Dictionary = vehicle.GetPose()
	var wall_x: float = float(pose_wall.get("x", 0.0))
	var nose_x: float = wall_x + 0.15
	var wall_ok := wall_x > WANT_WALL_X - 0.05 and wall_x < WANT_WALL_X + 0.05
	_check("wall-stops", wall_ok and nose_x <= WALL_X + 5e-3,
		"x=%.4f (want≈%.4f) 车头=%.4f 墙面=%.2f" % [wall_x, WANT_WALL_X, nose_x, WALL_X])

	# A32 最后 30 帧逐帧位移 < 5e-4：真的停住（不是慢慢蹭）
	var max_step: float = 0.0
	var prev_x: float = vehicle.global_position.x
	var prev_z: float = vehicle.global_position.z
	for _i in range(30):
		await physics_frame
		var now_x: float = vehicle.global_position.x
		var now_z: float = vehicle.global_position.z
		max_step = maxf(max_step, Vector2(now_x - prev_x, now_z - prev_z).length())
		prev_x = now_x
		prev_z = now_z
	_check("wall-settled", max_step < 5e-4
		and absf(vehicle.global_position.y) < TOL
		and absf(float(vehicle.GetPose().get("y", 0.0)) - 64.0) < TOL,
		"max_step=%.6f y=%.6f pose_y=%.4f" % [max_step, vehicle.global_position.y,
			float(vehicle.GetPose().get("y", 0.0))])

	# A33 撞墙不是卡死：先停稳（速度环归零），再倒车 30 帧应退出 ≈0.3875 m
	vehicle.SetMotionCommand({"action": "stop"})
	await _wait_physics(60)
	var x_wall_back: float = vehicle.global_position.x
	vehicle.SetMotionCommand({"action": "backward", "speed": 100})
	await _wait_physics(30)
	var dx_escape: float = vehicle.global_position.x - x_wall_back
	_check("wall-escape-backward", dx_escape < -WANT_BACK_30 * 0.8
		and dx_escape > -WANT_BACK_30 * 1.2,
		"dx=%.4f (want≈-%.4f)" % [dx_escape, WANT_BACK_30])

	# A34 撞墙全程：车辆仍有效、无自检装置层错误
	_check("wall-survives", is_instance_valid(vehicle) and vehicle.HasChassis()
		and _harness_errors == 0,
		"valid=%s has=%s harness_errors=%d" % [is_instance_valid(vehicle),
			vehicle.HasChassis(), _harness_errors])

	# ============================================================ E. 边界 / 白盒 / 契约
	# A35 三个脚本的白盒边界（去注释行后 grep）
	var banned_hits: Array[String] = []
	for path: String in [BASE_SCRIPT, CHASSIS_SCRIPT, CATALOG_SCRIPT]:
		var script: Script = load(path) as Script
		var code: String = _strip_comment_lines(script.source_code if script != null else "")
		for token: String in BANNED_TOKENS:
			if code.find(token) >= 0:
				banned_hits.append("%s:%s" % [path.get_file(), token])
	var dd_code: String = _code_of(CHASSIS_SCRIPT)
	var cat_code: String = _code_of(CATALOG_SCRIPT)
	_check("white-box-boundaries", banned_hits.is_empty()
		and dd_code.find("MOTION_MODE_GROUNDED") >= 0
		and dd_code.find("Approach(") >= 0
		and cat_code.find("load(") >= 0 and cat_code.find("preload(") < 0,
		"banned=%s order=%s" % [banned_hits, banned_hits.is_empty()])

	# A36 树外 Step：不报 !is_inside_tree，返回零控制量
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

	# A37 Configure(null, {}) 不崩；空指令视为 stop
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

	# A38 与 Step 4 契约互操作：无传感器时底盘照常 Step
	var slots: Dictionary = vehicle.GetStreamSlots()
	_check("interop-vehicle-contract", slots.is_empty() and vehicle.HasChassis()
		and vehicle.GetChassisName() == CHASSIS_NAME,
		"slots=%d has=%s name=%s" % [slots.size(), vehicle.HasChassis(),
			vehicle.GetChassisName()])

	# A39 全程无自检装置层面的错误
	_check("no-harness-errors", _harness_errors == 0,
		"harness_errors=%d" % _harness_errors)

	# ============================================================ 清理（P12：退出时无 ERROR）
	vehicle.SetMotionCommand({"action": "stop"})
	vehicle.queue_free()
	await process_frame
	await process_frame
	world.queue_free()
	await process_frame
	await process_frame
	_finish()


# ================================================================ 测试装置

## 每段独立运动前复位：重装底盘（Configure 会清零速度环状态）+ 摆正位姿（world 坐标）。
## 先对**旧**底盘发 stop，新底盘因此从未收到过指令（A30 依赖这一点）。
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


## 字典逐键相同（键集 + 值相等）；用于默认参数比对。
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
