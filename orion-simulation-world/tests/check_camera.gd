# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# 观察者相机自检（Step 3，非组件）：场景 / 输入映射 / 三模式 / 输入链路 / 解耦 / 物理不受影响。
# 用法：godot --headless --path orion-simulation-world --fixed-fps 60 --script res://tests/check_camera.gd
#
# 决定本文件写法的三条实测事实：
#   P3  Input.parse_input_event 投递到 _unhandled_input 需要 1~2 帧（本文件等 3 个 process_frame）
#   P5  headless 下 DisplayServer 无鼠标设备，鼠标捕获必须由脚本内部守卫（否则报警）
#   P7  轨迹逐帧复现不可靠（相同落体二次重跑会错开 1 个物理帧，60 帧末差 0.158 m）
#       → 解耦证明绝不用"重放对比轨迹"，只用三种对齐无关的不变量：
#         A23 静止体不被打扰（位置/速度/sleeping 不变）
#         A24 自由落体 y 的二阶差分 == -g·dt²（与起始帧对齐无关）
#         A25 同帧释放的两个落体逐帧 y 相等（同帧对照）
#   另：SceneTree.physics_frame 在节点 _physics_process 之前发出，读效果要比"改动帧"多等 1 帧。
#
# 注意（GDScript 4.7 类型推断陷阱）：凡 instantiate() / get_nodes_in_group() / source_code /
# load() 的返回值一律显式标注类型，否则 Parse Error: Cannot infer the type of ...

extends SceneTree

const ObserverCameraScript := preload("res://scripts/camera/ObserverCamera.gd")

const CAMERA_SCENE := "res://ObserverCamera.tscn"
const WORLD_SCENE := "res://World.tscn"
const CAMERA_SCRIPT_PATH := "res://scripts/camera/ObserverCamera.gd"

const TOL := 0.001

## COMPONENTS.md §1.3：11 条相机动作与 physical_keycode。
const CAM_ACTIONS := {
	"cam_forward": KEY_W,
	"cam_back": KEY_S,
	"cam_left": KEY_A,
	"cam_right": KEY_D,
	"cam_up": KEY_E,
	"cam_down": KEY_Q,
	"cam_boost": KEY_SHIFT,
	"cam_mode_free": KEY_F1,
	"cam_mode_orbit": KEY_F2,
	"cam_mode_follow": KEY_F3,
	"cam_target_next": KEY_TAB,
}

## A12 白盒 lint：相机脚本里不允许出现的物理 / 非 _unhandled_input 输入符号。
const BANNED_TOKENS := [
	"intersect_ray", "direct_space_state", "PhysicsServer",
	"PhysicsRayQueryParameters3D", "move_and_slide", "apply_impulse",
	"apply_central_impulse", "linear_velocity", "get_rid", "RigidBody3D",
	"CharacterBody3D", "StaticBody3D", "Area3D", "RayCast3D",
	"_input(", "_gui_input",
]

## 落体场景（Godot 坐标，均在 World.gd::OBSTACLES 之外的空地上）。
const DROP_POS_A := Vector3(58.0, 3.0, 58.0)
const DROP_POS_B := Vector3(44.0, 12.0, 44.0)
const DROP_POS_C := Vector3(50.0, 10.0, 50.0)
const DROP_POS_D := Vector3(52.0, 10.0, 50.0)

var _total: int = 0
var _failed: int = 0
var _harness_errors: int = 0


func _initialize() -> void:
	_run()


func _run() -> void:
	var cam_packed: PackedScene = load(CAMERA_SCENE)
	var world_packed: PackedScene = load(WORLD_SCENE)

	# ---- 1. 场景可加载 / 根节点 / 引擎原生属性
	if not _check("scene-loads", cam_packed != null, CAMERA_SCENE):
		_finish()
		return
	var probe_cam: Node = cam_packed.instantiate()
	var probe_is_camera: bool = probe_cam is Camera3D
	_check("scene-loads-instantiate", probe_is_camera, str(probe_cam.get_class()))
	if probe_is_camera:
		var probe_script: Script = probe_cam.get_script()
		_check("scene-root", probe_cam.name == "ObserverCamera" and probe_script != null
			and probe_script.resource_path == CAMERA_SCRIPT_PATH,
			"%s / %s" % [probe_cam.name, probe_script.resource_path if probe_script != null else "null"])
		var pc: Camera3D = probe_cam as Camera3D
		_check("scene-camera-props", pc.current and absf(pc.fov - 70.0) < 0.01
			and absf(pc.near - 0.05) < TOL and absf(pc.far - 500.0) < 0.01,
			"current=%s fov=%s near=%s far=%s" % [pc.current, pc.fov, pc.near, pc.far])
	probe_cam.free()

	# ---- 2. World.tscn 里的相机：唯一、current、替代 DebugCamera
	if not _check("world-loads", world_packed != null, WORLD_SCENE):
		_finish()
		return
	var world: Node3D = world_packed.instantiate()
	root.add_child(world)
	await physics_frame
	await physics_frame

	var cam: Camera3D = world.get_node_or_null("ObserverCamera")
	var cam_ok: bool = cam != null and cam is Camera3D
	_check("world-has-camera", cam_ok, str(cam))
	if not cam_ok:
		_harness_errors += 1
		_finish()
		return
	_check("debug-camera-removed", world.get_node_or_null("DebugCamera") == null)

	var cameras: Array[Camera3D] = _collect_cameras(world)
	var current_count := 0
	for item: Camera3D in cameras:
		if item.current:
			current_count += 1
	_check("single-current-camera", cameras.size() == 1 and current_count == 1
		and root.get_camera_3d() == cam,
		"cameras=%d current=%d viewport_cam=%s" % [cameras.size(), current_count,
			str(root.get_camera_3d())])
	_check("default-pose-frustum", cam.is_position_in_frustum(Vector3(64.0, 0.0, 64.0)),
		"pos=%s fov=%s" % [cam.global_position, cam.fov])

	# ---- 3. 输入映射（project.godot 已就绪，相机只读用）
	var missing: Array[String] = []
	var wrong_keys: Array[String] = []
	for action_name: String in CAM_ACTIONS:
		if not InputMap.has_action(action_name):
			missing.append(action_name)
			continue
		var events: Array[InputEvent] = InputMap.action_get_events(action_name)
		if events.size() != 1:
			wrong_keys.append("%s(events=%d)" % [action_name, events.size()])
			continue
		var key_event := events[0] as InputEventKey
		if key_event == null or int(key_event.physical_keycode) != int(CAM_ACTIONS[action_name]):
			wrong_keys.append("%s(key=%s)" % [action_name,
				str(key_event.physical_keycode) if key_event != null else "not-key"])
	_check("input-actions", missing.is_empty() and wrong_keys.is_empty()
		and CAM_ACTIONS.size() == 11,
		"missing=%s wrong=%s" % [missing, wrong_keys])

	# ---- 4. 模式 API（含非法值）
	var api_ok: bool = cam.GetMode() == ObserverCameraScript.Mode.FREE \
		and cam.GetModeName() == "FREE"
	api_ok = api_ok and cam.SetMode(ObserverCameraScript.Mode.ORBIT) \
		and cam.GetModeName() == "ORBIT"
	api_ok = api_ok and cam.SetMode(ObserverCameraScript.Mode.FOLLOW) \
		and cam.GetModeName() == "FOLLOW"
	api_ok = api_ok and cam.SetMode(ObserverCameraScript.Mode.FREE) \
		and cam.GetModeName() == "FREE"
	var bad_ok: bool = not cam.SetMode(99) and cam.GetMode() == ObserverCameraScript.Mode.FREE
	_check("mode-api", api_ok and bad_ok, "mode=%s name=%s" % [cam.GetMode(), cam.GetModeName()])

	# ---- 5. 按键端到端：F2/F3/F1 真的经 _unhandled_input 生效（P3，异步 1~2 帧）
	var keys_ok := await _press_key_and_expect(cam, KEY_F2, ObserverCameraScript.Mode.ORBIT)
	keys_ok = keys_ok and await _press_key_and_expect(cam, KEY_F3, ObserverCameraScript.Mode.FOLLOW)
	keys_ok = keys_ok and await _press_key_and_expect(cam, KEY_F1, ObserverCameraScript.Mode.FREE)
	_check("mode-key-end2end", keys_ok, "mode=%s" % cam.GetModeName())

	# ---- 6. 自由飞行：按键真的位移，且方向 = 相机视线；松键即停
	var p0: Vector3 = cam.global_position
	Input.action_press(&"cam_forward")
	await _wait_physics(24)
	var moved: Vector3 = cam.global_position - p0
	var forward: Vector3 = -cam.global_transform.basis.z
	var angle_deg: float = rad_to_deg(moved.normalized().angle_to(forward)) if moved.length() > TOL else 180.0
	_check("free-move-forward", moved.length() > 1.0 and angle_deg < 2.0,
		"len=%.3f angle=%.2fdeg (want >1.0m, <2deg)" % [moved.length(), angle_deg])
	Input.action_release(&"cam_forward")
	await _wait_physics(1)
	var p1: Vector3 = cam.global_position
	await _wait_physics(12)
	var drift: float = cam.global_position.distance_to(p1)
	_check("free-move-stops", drift < 0.02, "drift=%.4f" % drift)

	# ---- 7. 解耦（白盒）：源码无物理 API、输入只走 _unhandled_input
	var cam_script: Script = cam.get_script()
	var source: String = cam_script.source_code if cam_script != null else ""
	var code := _strip_comment_lines(source)
	var hits: Array[String] = []
	var lint_target := code.replace("_unhandled_input", "")
	for token: String in BANNED_TOKENS:
		if lint_target.find(token) >= 0:
			hits.append(token)
	_check("decoupling-static", cam_script != null and hits.is_empty()
		and code.find("_unhandled_input") >= 0,
		"banned=%s (len=%d)" % [hits, source.length()])

	# ---- 8. 解耦（节点）：相机子树里没有任何物理 / 几何节点
	var physical_children := _count_physical_nodes(cam)
	_check("decoupling-nodes", physical_children == 0 and cam.get_child_count() == 0,
		"physical=%d children=%d" % [physical_children, cam.get_child_count()])

	# ---- 9. 解耦（组）：相机不在 vehicle 组，不被任何仿真逻辑选中
	_check("decoupling-group", not cam.is_in_group(&"vehicle")
		and not cam.is_in_group(cam.vehicles_group),
		"group=%s" % cam.vehicles_group)

	# ---- 10. 自由视角转向：右键拖拽改 yaw、俯仰夹紧、松键后不再转
	var yaw0: float = cam.rotation.y
	cam._unhandled_input(_mouse_button(MOUSE_BUTTON_RIGHT, true))
	cam._unhandled_input(_mouse_motion(Vector2(50.0, 0.0)))
	await _wait_physics(2)
	var dyaw: float = cam.rotation.y - yaw0
	cam._unhandled_input(_mouse_motion(Vector2(0.0, -2000.0)))
	await _wait_physics(2)
	var pitch_deg: float = rad_to_deg(cam.rotation.x)
	cam._unhandled_input(_mouse_button(MOUSE_BUTTON_RIGHT, false))
	await _wait_physics(1)
	var rot_before: Vector3 = cam.rotation
	cam._unhandled_input(_mouse_motion(Vector2(100.0, 0.0)))
	await _wait_physics(2)
	_check("free-look", absf(dyaw + 50.0 * cam.look_sensitivity) < TOL
		and absf(pitch_deg) <= 88.0 + 0.1 and cam.rotation.is_equal_approx(rot_before),
		"dyaw=%.4f (want %.4f) pitch=%.2fdeg" % [dyaw, -50.0 * cam.look_sensitivity, pitch_deg])

	# ---- 11. 目标与注视点
	var dummy: Node3D = _make_target(world, "veh_a", Vector3(30.0, 0.0, 40.0), 0.0)
	await _wait_physics(2)
	cam.SetTarget(dummy)
	var focus: Vector3 = cam.GetFocusPoint()
	var want_focus: Vector3 = dummy.global_position + Vector3.UP * cam.follow_height
	_check("orbit-follow-target", cam.GetTargetName() == "veh_a"
		and focus.distance_to(want_focus) < TOL,
		"target=%s focus=%s want=%s" % [cam.GetTargetName(), focus, want_focus])

	# ---- 12. 环绕：切模式不瞬移，就位后距离 = orbit_distance 且视线对准 focus
	cam.global_position = focus + Basis.from_euler(Vector3(deg_to_rad(-25.0), deg_to_rad(20.0), 0.0)) \
		* Vector3(0.0, 0.0, cam.orbit_distance)
	var pos_before: Vector3 = cam.global_position
	var orbit_set: bool = cam.SetMode(ObserverCameraScript.Mode.ORBIT)
	await _wait_physics(1)
	var jump: float = cam.global_position.distance_to(pos_before)
	await _wait_physics(180)
	var focus_now: Vector3 = cam.GetFocusPoint()
	var orbit_dist: float = cam.global_position.distance_to(focus_now)
	var look_dot: float = (focus_now - cam.global_position).normalized().dot(-cam.global_transform.basis.z)
	_check("orbit-pose", orbit_set and jump < 0.05
		and absf(orbit_dist - cam.orbit_distance) < 0.5 and look_dot > 0.99,
		"jump=%.4f dist=%.3f (want %.3f) dot=%.4f" % [jump, orbit_dist, cam.orbit_distance, look_dot])

	# ---- 13. 环绕缩放：滚轮上滚拉远，且不越上限
	var dist_before: float = cam.GetViewState()["distance"]
	cam._unhandled_input(_mouse_button(MOUSE_BUTTON_WHEEL_UP, true))
	var dist_after: float = cam.GetViewState()["distance"]
	for _i in range(60):
		cam._unhandled_input(_mouse_button(MOUSE_BUTTON_WHEEL_UP, true))
	var dist_max: float = cam.GetViewState()["distance"]
	_check("orbit-zoom", dist_after > dist_before * 1.1
		and dist_max <= cam.orbit_distance_max + TOL,
		"before=%.2f after=%.2f capped=%.2f max=%.2f" % [dist_before, dist_after, dist_max,
			cam.orbit_distance_max])

	# ---- 14. 跟随：相机落在车「后」方（车辆前向 = 局部 +X），高度与距离符合参数
	dummy.rotation.y = deg_to_rad(35.0)
	for _i in range(60):
		dummy.global_position += dummy.global_transform.basis.x.normalized() * 0.1
		await physics_frame
	var tpos: Vector3 = dummy.global_position
	var back: Vector3 = -dummy.global_transform.basis.x
	back.y = 0.0
	back = back.normalized()
	cam.global_position = tpos + Vector3.UP * cam.follow_height + back * cam.follow_distance
	var follow_set: bool = cam.SetMode(ObserverCameraScript.Mode.FOLLOW)
	await _wait_physics(180)
	var gap: Vector3 = cam.global_position - dummy.global_position
	var want_gap: float = sqrt(cam.follow_distance * cam.follow_distance
		+ cam.follow_height * cam.follow_height)
	var behind: float = gap.dot(dummy.global_transform.basis.x)
	_check("follow-behind", follow_set and absf(gap.length() - want_gap) < 0.6 and behind < 0.0,
		"gap=%.3f want=%.3f dot(+X)=%.3f" % [gap.length(), want_gap, behind])

	# ---- 15. Tab 循环切目标 / 空组不崩
	dummy.queue_free()
	await process_frame
	await process_frame
	var t_b: Node3D = _make_target(world, "veh_b", Vector3(20.0, 0.0, 20.0), 0.0)
	var t_c: Node3D = _make_target(world, "veh_c", Vector3(24.0, 0.0, 24.0), 0.0)
	await _wait_physics(2)
	var n1: Node3D = cam.SelectNextTarget()
	var n2: Node3D = cam.SelectNextTarget()
	var n3: Node3D = cam.SelectNextTarget()
	_check("target-cycle", _name_of(n1) == "veh_b" and _name_of(n2) == "veh_c"
		and _name_of(n3) == "veh_b" and cam.GetTarget() == n3,
		"%s -> %s -> %s" % [_name_of(n1), _name_of(n2), _name_of(n3)])
	t_b.queue_free()
	t_c.queue_free()
	await process_frame
	await process_frame
	var n_empty: Node3D = cam.SelectNextTarget()
	_check("target-empty-group", n_empty == null and cam.GetTarget() == null,
		str(n_empty))

	# ---- 16. 目标被释放后自动失联，不崩、模式不变
	var t_d: Node3D = _make_target(world, "veh_d", Vector3(40.0, 0.0, 40.0), 0.0)
	await _wait_physics(2)
	cam.SetTarget(t_d)
	var mode_before: String = cam.GetModeName()
	t_d.queue_free()
	await process_frame
	await process_frame
	_check("target-prune", cam.GetTarget() == null and cam.GetModeName() == mode_before
		and is_instance_valid(cam),
		"target=%s mode=%s" % [cam.GetTargetName(), cam.GetModeName()])

	# ---- 17. control_enabled=false：一切输入都不生效（自检可控、也是"冻结"语义）
	cam.control_enabled = false
	var rot_snapshot: Vector3 = cam.rotation
	var pos_snapshot: Vector3 = cam.global_position
	var mode_snapshot: int = cam.GetMode()
	cam._unhandled_input(_key_event(KEY_F2))
	cam._unhandled_input(_mouse_button(MOUSE_BUTTON_RIGHT, true))
	cam._unhandled_input(_mouse_motion(Vector2(200.0, 60.0)))
	cam._unhandled_input(_mouse_button(MOUSE_BUTTON_WHEEL_UP, true))
	await _wait_physics(2)
	_check("no-control", cam.rotation.is_equal_approx(rot_snapshot)
		and cam.global_position.is_equal_approx(pos_snapshot) and cam.GetMode() == mode_snapshot,
		"mode=%s pos=%s" % [cam.GetModeName(), cam.global_position])

	# ---- 18. 物理不变量 A：静止体不被相机打扰（P7：不做轨迹重放对比）
	var rest_body: RigidBody3D = _make_drop_body(world, DROP_POS_A)
	await _wait_physics(240)
	var rest_pos: Vector3 = rest_body.global_position
	var rest_speed: float = rest_body.linear_velocity.length()
	var rest_sleeping: bool = rest_body.sleeping
	_check("physics-rest", rest_pos.y > 0.13 and rest_pos.y < 0.18 and rest_speed < 0.05,
		"y=%.4f |v|=%.5f sleeping=%s" % [rest_pos.y, rest_speed, rest_sleeping])
	for i in range(120):
		_shake_camera(cam, i)
		await physics_frame
	var rest_delta: float = rest_body.global_position.distance_to(rest_pos)
	var rest_speed_after: float = rest_body.linear_velocity.length()
	_check("physics-rest-invariant", rest_delta < 1e-5 and rest_speed_after < 1e-3
		and rest_body.sleeping == rest_sleeping,
		"delta=%.9f |v|=%.9f sleeping=%s" % [rest_delta, rest_speed_after, rest_body.sleeping])

	# ---- 19. 物理不变量 B：自由落体二阶差分 == -g·dt²（与对齐无关，容差 2e-3）
	var fall_body: RigidBody3D = _make_drop_body(world, DROP_POS_B)
	var samples: Array[float] = []
	for i in range(48):
		_shake_camera(cam, i + 200)
		await physics_frame
		samples.append(fall_body.global_position.y)
	var gravity: float = float(ProjectSettings.get_setting("physics/3d/default_gravity", 9.8))
	var dt := 1.0 / 60.0
	var want_d2: float = -gravity * dt * dt
	var worst_d2: float = 0.0
	for i in range(1, samples.size() - 1):
		var d2: float = samples[i + 1] - 2.0 * samples[i] + samples[i - 1]
		worst_d2 = maxf(worst_d2, absf(d2 - want_d2))
	var drop: float = samples[0] - samples[samples.size() - 1]
	_check("physics-freefall-invariant", worst_d2 < 2e-3 and drop > 1.0,
		"worst|d2-(-g·dt²)|=%.6f (want %.6f) drop=%.3f m" % [worst_d2, want_d2, drop])

	# ---- 20. 物理不变量 C：同帧释放的两具落体逐帧同步（同帧对照，天然对齐）
	var body_a: RigidBody3D = _make_drop_body(world, DROP_POS_C)
	var body_b: RigidBody3D = _make_drop_body(world, DROP_POS_D)
	var worst_gap := 0.0
	for i in range(30):
		_shake_camera(cam, i + 400)
		await physics_frame
		worst_gap = maxf(worst_gap, absf(body_a.global_position.y - body_b.global_position.y))
	_check("physics-two-body-control", worst_gap < 1e-3, "worst|ya-yb|=%.9f" % worst_gap)

	# ---- 21. 物理引擎设置未被相机改动
	var grav_now: float = float(ProjectSettings.get_setting("physics/3d/default_gravity", 9.8))
	_check("physics-engine-untouched", Engine.physics_ticks_per_second == 60
		and is_equal_approx(Engine.time_scale, 1.0) and is_equal_approx(grav_now, gravity),
		"ticks=%d time_scale=%s gravity=%s" % [Engine.physics_ticks_per_second,
			Engine.time_scale, grav_now])

	cam.control_enabled = true
	rest_body.queue_free()
	fall_body.queue_free()
	body_a.queue_free()
	body_b.queue_free()
	await process_frame

	# ---- 22. 全程无 push_error（脚本层面）；SCRIPT ERROR 由命令行 grep 兜底（见 COMPONENTS.md §1.4）
	_check("no-runtime-errors", code.find("push_error(") < 0 and _harness_errors == 0,
		"push_error_calls=%d harness_errors=%d" % [1 if code.find("push_error(") >= 0 else 0,
			_harness_errors])

	_finish()


# ================================================================ 测试装置

func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		await physics_frame


func _press_key_and_expect(cam: Camera3D, keycode: int, want_mode: int) -> bool:
	Input.parse_input_event(_key_event(keycode))
	await process_frame
	await process_frame
	await process_frame
	return cam.GetMode() == want_mode


func _key_event(keycode: int) -> InputEventKey:
	var event := InputEventKey.new()
	event.physical_keycode = keycode
	event.pressed = true
	return event


func _mouse_button(button: int, pressed: bool) -> InputEventMouseButton:
	var event := InputEventMouseButton.new()
	event.button_index = button
	event.pressed = pressed
	return event


func _mouse_motion(relative: Vector2) -> InputEventMouseMotion:
	var event := InputEventMouseMotion.new()
	event.relative = relative
	return event


## 造一个"车"靶子：Node3D + 进 vehicle 组 + 挂到世界容器下（group 查询需在树内）。
func _make_target(world: Node3D, node_name: String, pos: Vector3, yaw: float) -> Node3D:
	var node := Node3D.new()
	node.name = node_name
	node.add_to_group(&"vehicle")
	var container: Node3D = world.get_node("Vehicles")
	if container == null:
		_harness_errors += 1
		container = world
	container.add_child(node)
	node.global_position = pos
	node.rotation.y = yaw
	return node


func _make_drop_body(world: Node3D, pos: Vector3) -> RigidBody3D:
	var body := RigidBody3D.new()
	var shape := BoxShape3D.new()
	shape.size = Vector3(0.3, 0.3, 0.3)
	var collision := CollisionShape3D.new()
	collision.shape = shape
	body.add_child(collision)
	body.mass = 1.0
	body.collision_layer = 4
	body.collision_mask = 1
	world.add_child(body)
	body.global_position = pos
	return body


## 每帧把相机"乱移"：物理体如果受相机影响，位置/速度/sleeping 会立刻变化。
func _shake_camera(cam: Camera3D, i: int) -> void:
	var t := float(i) * 0.7
	cam.global_position = Vector3(64.0 + 30.0 * sin(t), 8.0 + 4.0 * cos(t * 1.3),
		64.0 + 30.0 * cos(t))
	cam.rotation = Vector3(0.9 * sin(t * 0.5), t, 0.4 * cos(t * 0.31))


func _collect_cameras(node: Node) -> Array[Camera3D]:
	var found: Array[Camera3D] = []
	var camera := node as Camera3D
	if camera != null:
		found.append(camera)
	for child in node.get_children():
		found.append_array(_collect_cameras(child))
	return found


func _count_physical_nodes(node: Node) -> int:
	var count := 0
	for child in node.get_children():
		if child is CollisionObject3D or child is CollisionShape3D or child is PhysicsBody3D \
			or child is Area3D or child is RayCast3D or child is GridMap:
			count += 1
		count += _count_physical_nodes(child)
	return count


## 去掉以 # 开头的注释行，只留可执行代码（A12 白盒 lint 用）。
func _strip_comment_lines(source: String) -> String:
	var code := ""
	for raw_line: String in source.split("\n"):
		if raw_line.strip_edges().begins_with("#"):
			continue
		code += raw_line + "\n"
	return code


func _name_of(node: Node) -> String:
	return str(node.name) if node != null else "<null>"


# ================================================================ 断言与收尾

func _check(label: String, ok: bool, detail: String = "") -> bool:
	_total += 1
	if ok:
		print("[check] ", label, " ok", (" " + detail) if not detail.is_empty() else "")
		return true
	_failed += 1
	print("[check] ", label, " FAIL", (" " + detail) if not detail.is_empty() else "")
	return false


func _finish() -> void:
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
