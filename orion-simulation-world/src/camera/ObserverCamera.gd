# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 观察者相机（task_30 §4.2 / COMPONENTS.md §4.2）：Camera3D + 三种视角。
#   FREE   自由飞行：WASD 平移、Q/E 升降、Shift 加速、右键拖拽转向、滚轮调速
#   ORBIT  环绕   ：中键拖拽绕目标转、滚轮缩放半径
#   FOLLOW 跟随   ：第三人称跟车（车辆前向 = 局部 +X，见 COMPONENTS.md §6.5）
#
# 与仿真完全解耦（本文件必须守住的硬约束）：
#   1) 零依赖：不 preload 任何脚本，不与任何仿真组件共享状态；
#   2) 不接触任何物理 API：不做射线查询、不建刚体、不做运动学积分；
#   3) 位置与朝向只由本文件写，且没有任何其它组件读它 → 转镜头不影响任何物理量；
#   4) 相机不进 "vehicle" 组、不发不收信号；
#   5) 输入事件只走 _unhandled_input（不抢 UI 输入，task_30 §4.2）；
#      连续按键（WASD/QE/Shift）用 Input.get_axis 轮询，全项目不出现 _input / _gui_input。
#
# 坐标系：Godot Y-up，相机前向 -Z，车辆前向局部 +X；内侧角度一律弧度。
# 状态连续性：切模式时由 _sync_state_from_transform() 从当前 transform 反推新模式状态，
# 保证"切视角不瞬移"。打印纪律：只在 _ready 与模式/目标切换时各打 1 行，不逐帧打。
#
# 不声明 class_name（COMPONENTS.md §9.1 E5）。

extends Camera3D

enum Mode { FREE = 0, ORBIT = 1, FOLLOW = 2 }

const MODE_NAMES := ["FREE", "ORBIT", "FOLLOW"]

# ---- project.godot [input] 的 11 条相机动作（只读，Step 2 已就绪）
const ACTION_FORWARD := &"cam_forward"
const ACTION_BACK := &"cam_back"
const ACTION_LEFT := &"cam_left"
const ACTION_RIGHT := &"cam_right"
const ACTION_UP := &"cam_up"
const ACTION_DOWN := &"cam_down"
const ACTION_BOOST := &"cam_boost"
const ACTION_MODE_FREE := &"cam_mode_free"
const ACTION_MODE_ORBIT := &"cam_mode_orbit"
const ACTION_MODE_FOLLOW := &"cam_mode_follow"
const ACTION_TARGET_NEXT := &"cam_target_next"

# ---- 模式内常量（不做成 @export：v1 不需要外部调参）
const FREE_BOOST_MULTIPLIER := 3.0
const ORBIT_PITCH_LIMIT_DEG := 80.0
const FOLLOW_DIST_MIN := 2.0
const FOLLOW_DIST_MAX := 40.0
const MIN_ZOOM_STEP := 1.01
const MIN_LOOK_DIR := 0.001

@export var control_enabled: bool = true
## 观察目标所在组（Step 4 的 Vehicle 根节点组名，见 COMPONENTS.md §4.3）。
@export var vehicles_group: StringName = &"vehicle"
@export var free_speed: float = 12.0
@export var free_speed_min: float = 1.0
@export var free_speed_max: float = 60.0
@export var free_speed_step: float = 1.15
## 鼠标灵敏度：弧度 / 像素。
@export var look_sensitivity: float = 0.0035
@export var pitch_limit_deg: float = 88.0
@export var orbit_distance: float = 18.0
@export var orbit_distance_min: float = 2.0
@export var orbit_distance_max: float = 80.0
## 指数平滑系数（1/s）：越大越"硬"。
@export var orbit_smoothing: float = 12.0
@export var follow_distance: float = 8.0
@export var follow_height: float = 3.0
@export var follow_smoothing: float = 8.0
## 无目标时环绕的中心（Godot 坐标 = 世界中心地面）。
@export var orbit_fallback_focus: Vector3 = Vector3(64.0, 1.0, 64.0)

## 当前模式（Mode）。
var mode: int = Mode.FREE
## 环绕 / 跟随的目标；null = 无目标。
var target: Node3D = null

# ---- 视角状态：切模式时由 _sync_state_from_transform() 从 transform 反推
var _yaw: float = 0.0
var _pitch: float = 0.0
var _free_speed_current: float = 12.0
var _orbit_yaw: float = 0.0
var _orbit_pitch: float = 0.0
var _orbit_dist: float = 18.0
var _follow_yaw: float = 0.0
var _follow_dist: float = 8.0
var _looking: bool = false
var _orbiting: bool = false


# ================================================================ 生命周期

func _ready() -> void:
	_free_speed_current = clampf(free_speed, free_speed_min, free_speed_max)
	_orbit_dist = clampf(orbit_distance, orbit_distance_min, orbit_distance_max)
	_follow_dist = clampf(follow_distance, FOLLOW_DIST_MIN, FOLLOW_DIST_MAX)
	_sync_state_from_transform()
	print("[camera] ObserverCamera ready: mode=%s target=%s pos=(%.2f, %.2f, %.2f) fov=%.1f speed=%.1f"
		% [GetModeName(), GetTargetName(), global_position.x, global_position.y,
			global_position.z, fov, _free_speed_current])


func _physics_process(delta: float) -> void:
	if not control_enabled:
		return
	_prune_target()
	match mode:
		Mode.FREE:
			_apply_free(delta)
		Mode.ORBIT:
			_apply_orbit(delta)
		Mode.FOLLOW:
			_apply_follow(delta)


# ================================================================ 公开接口

## 切换视角。非法值 push_warning 并返回 false（模式不变）；成功返回 true。
## 切模式不瞬移：先把当前 transform 反推成新模式的全部状态。
func SetMode(p_mode: int) -> bool:
	if p_mode < Mode.FREE or p_mode > Mode.FOLLOW:
		push_warning("[camera] 非法模式 %d（合法 %d..%d），模式保持 %s"
			% [p_mode, Mode.FREE, Mode.FOLLOW, GetModeName()])
		return false
	mode = p_mode
	_sync_state_from_transform()
	print("[camera] mode -> %s (target=%s, dist=%.1f)"
		% [GetModeName(), GetTargetName(), _current_distance()])
	return true


func GetMode() -> int:
	return mode


func GetModeName() -> String:
	if mode < 0 or mode >= MODE_NAMES.size():
		return "UNKNOWN"
	return MODE_NAMES[mode]


## 设置环绕 / 跟随目标；允许 null。不在 vehicles_group 的节点只告警，仍然接受
## （自检要拿普通 Node3D 当靶子；Step 6 也可能按 peer_id 指定）。
func SetTarget(p_target: Node3D) -> void:
	target = p_target
	if target != null and not target.is_in_group(vehicles_group):
		push_warning("[camera] 目标 %s 不在组 \"%s\" 中（仍然接受）"
			% [target.name, vehicles_group])
	_sync_state_from_transform()
	print("[camera] target -> %s" % GetTargetName())


func GetTarget() -> Node3D:
	if target != null and not is_instance_valid(target):
		target = null
	return target


func GetTargetName() -> String:
	var t := GetTarget()
	if t == null:
		return "<none>"
	return str(t.name)


## Tab：按名字稳定排序遍历 vehicles_group，循环切换目标；空组返回 null。
func SelectNextTarget() -> Node3D:
	var raw: Array = get_tree().get_nodes_in_group(vehicles_group)
	var candidates: Array[Node3D] = []
	for item in raw:
		var node := item as Node3D
		if node != null and is_instance_valid(node) and node.is_inside_tree():
			candidates.append(node)
	candidates.sort_custom(_compare_node_name)
	if candidates.is_empty():
		print("[camera] no vehicle in group \"%s\"" % vehicles_group)
		SetTarget(null)
		return null
	var current := GetTarget()
	var index := -1
	if current != null:
		index = candidates.find(current)
	var next_index := 0 if index < 0 else (index + 1) % candidates.size()
	var next_target: Node3D = candidates[next_index]
	SetTarget(next_target)
	return next_target


## 注视点（Godot 坐标）：有目标 → 目标位置 + UP*follow_height；否则用 fallback。
## 注：这是相机自己的取景中心，不参与任何位姿/传感计算。
func GetFocusPoint() -> Vector3:
	var t := GetTarget()
	if _is_target_valid():
		return t.global_position + Vector3.UP * follow_height
	return orbit_fallback_focus


## 只读快照，供自检 / 未来的 HUD 使用（不写入任何仿真状态）。
func GetViewState() -> Dictionary:
	return {
		"mode": mode,
		"mode_name": GetModeName(),
		"target": GetTargetName(),
		"distance": _current_distance(),
		"focus": GetFocusPoint(),
	}


# ================================================================ 输入（唯一入口）

func _unhandled_input(event: InputEvent) -> void:
	if not control_enabled:
		return
	if event is InputEventKey:
		var key := event as InputEventKey
		if key == null or not key.pressed or key.echo:
			return
		_handle_key(key)
	elif event is InputEventMouseButton:
		var button := event as InputEventMouseButton
		if button != null:
			_handle_mouse_button(button)
	elif event is InputEventMouseMotion:
		var motion := event as InputEventMouseMotion
		if motion != null:
			_handle_mouse_motion(motion)


# ================================================================ 三种模式

func _apply_free(delta: float) -> void:
	var cam_basis := Basis.from_euler(Vector3(_pitch, _yaw, 0.0))
	var ix := Input.get_axis(ACTION_LEFT, ACTION_RIGHT)     # 右 - 左
	var iz := Input.get_axis(ACTION_FORWARD, ACTION_BACK)   # 后 - 前（前移 = -Z）
	var iy := Input.get_axis(ACTION_DOWN, ACTION_UP)        # 上 - 下
	var wish := cam_basis * Vector3(ix, 0.0, iz) + Vector3.UP * iy
	if wish.length() > 1.0:
		wish = wish.normalized()
	if wish != Vector3.ZERO:
		var speed := _free_speed_current
		if Input.is_action_pressed(ACTION_BOOST):
			speed *= FREE_BOOST_MULTIPLIER
		global_position += wish * speed * delta
	rotation = Vector3(_pitch, _yaw, 0.0)


func _apply_orbit(delta: float) -> void:
	var focus := GetFocusPoint()
	var cam_basis := Basis.from_euler(Vector3(_orbit_pitch, _orbit_yaw, 0.0))
	var desired := focus + cam_basis * Vector3(0.0, 0.0, _orbit_dist)
	global_position = _smooth_vec(global_position, desired, orbit_smoothing, delta)
	_look_at_safe(focus)


func _apply_follow(delta: float) -> void:
	# 无目标：保持当前位姿（F3 在空世界里不改变视野，也不报错）。
	if not _is_target_valid():
		return
	var tpos: Vector3 = target.global_position
	var back: Vector3 = -target.global_transform.basis.x
	back.y = 0.0
	if back.length() < MIN_LOOK_DIR:
		back = Vector3.BACK
	else:
		back = back.normalized()
	back = Basis(Vector3.UP, _follow_yaw) * back
	var desired := tpos + Vector3.UP * follow_height + back * _follow_dist
	global_position = _smooth_vec(global_position, desired, follow_smoothing, delta)
	_look_at_safe(tpos + Vector3.UP * (follow_height * 0.5))


# ================================================================ 输入处理

func _handle_key(key: InputEventKey) -> void:
	if key.is_action(ACTION_MODE_FREE):
		SetMode(Mode.FREE)
	elif key.is_action(ACTION_MODE_ORBIT):
		SetMode(Mode.ORBIT)
	elif key.is_action(ACTION_MODE_FOLLOW):
		SetMode(Mode.FOLLOW)
	elif key.is_action(ACTION_TARGET_NEXT):
		SelectNextTarget()
	else:
		return
	_mark_input_handled()


func _handle_mouse_button(event: InputEventMouseButton) -> void:
	match event.button_index:
		MOUSE_BUTTON_RIGHT:
			_looking = event.pressed
			_set_mouse_captured(_looking or _orbiting)
		MOUSE_BUTTON_MIDDLE:
			_orbiting = event.pressed
			_set_mouse_captured(_looking or _orbiting)
		MOUSE_BUTTON_WHEEL_UP:
			if not event.pressed:
				return
			_apply_zoom(_zoom_step())
		MOUSE_BUTTON_WHEEL_DOWN:
			if not event.pressed:
				return
			_apply_zoom(1.0 / _zoom_step())
		_:
			return
	_mark_input_handled()


func _handle_mouse_motion(event: InputEventMouseMotion) -> void:
	var rel := event.relative
	if mode == Mode.FREE and _looking:
		_yaw -= rel.x * look_sensitivity
		var free_lim := deg_to_rad(clampf(pitch_limit_deg, 1.0, 89.0))
		_pitch = clampf(_pitch - rel.y * look_sensitivity, -free_lim, free_lim)
	elif mode == Mode.ORBIT and _orbiting:
		_orbit_yaw -= rel.x * look_sensitivity
		var orbit_lim := deg_to_rad(ORBIT_PITCH_LIMIT_DEG)
		_orbit_pitch = clampf(_orbit_pitch - rel.y * look_sensitivity, -orbit_lim, orbit_lim)
	else:
		return
	_mark_input_handled()


# ================================================================ 私有

## 从当前 transform 反推当前模式的全部状态（切模式不跳变的关键）。
func _sync_state_from_transform() -> void:
	var free_lim := deg_to_rad(clampf(pitch_limit_deg, 1.0, 89.0))
	var orbit_lim := deg_to_rad(ORBIT_PITCH_LIMIT_DEG)
	match mode:
		Mode.FREE:
			_yaw = rotation.y
			_pitch = clampf(rotation.x, -free_lim, free_lim)
		Mode.ORBIT:
			var focus := GetFocusPoint()
			var offset := global_position - focus
			var dist := offset.length()
			_orbit_dist = clampf(orbit_distance if dist < MIN_LOOK_DIR else dist,
				orbit_distance_min, orbit_distance_max)
			if dist < MIN_LOOK_DIR:
				_orbit_yaw = rotation.y
				_orbit_pitch = clampf(rotation.x, -orbit_lim, orbit_lim)
			else:
				var dir := offset / dist
				_orbit_yaw = atan2(dir.x, dir.z)
				_orbit_pitch = clampf(asin(clampf(-dir.y, -1.0, 1.0)), -orbit_lim, orbit_lim)
		Mode.FOLLOW:
			_follow_yaw = 0.0
			_follow_dist = clampf(follow_distance, FOLLOW_DIST_MIN, FOLLOW_DIST_MAX)
			if _is_target_valid():
				# 只取水平距离：follow_dist 是"车后方的水平偏移"（高度由 follow_height 单列）
				var flat := target.global_position - global_position
				flat.y = 0.0
				if flat.length() > MIN_LOOK_DIR:
					_follow_dist = clampf(flat.length(), FOLLOW_DIST_MIN, FOLLOW_DIST_MAX)
			else:
				# 无目标：跟随模式空转，仅同步一个可读的距离值
				_follow_dist = clampf(global_position.distance_to(orbit_fallback_focus),
					FOLLOW_DIST_MIN, FOLLOW_DIST_MAX)


func _mark_input_handled() -> void:
	var vp := get_viewport()
	if vp != null:
		vp.set_input_as_handled()


func _set_mouse_captured(p_captured: bool) -> void:
	# headless 没有鼠标设备（实测 DisplayServer.has_feature(FEATURE_MOUSE)==false）
	if not DisplayServer.has_feature(DisplayServer.FEATURE_MOUSE):
		return
	Input.mouse_mode = Input.MOUSE_MODE_CAPTURED if p_captured else Input.MOUSE_MODE_VISIBLE


func _apply_zoom(factor: float) -> void:
	match mode:
		Mode.FREE:
			_free_speed_current = clampf(_free_speed_current * factor,
				free_speed_min, free_speed_max)
		Mode.ORBIT:
			_orbit_dist = clampf(_orbit_dist * factor, orbit_distance_min, orbit_distance_max)
		Mode.FOLLOW:
			_follow_dist = clampf(_follow_dist * factor, FOLLOW_DIST_MIN, FOLLOW_DIST_MAX)


func _zoom_step() -> float:
	return maxf(free_speed_step, MIN_ZOOM_STEP)


## 帧率无关的指数平滑：w = 1 - exp(-k*dt)。
func _smooth_vec(current: Vector3, desired: Vector3, k: float, delta: float) -> Vector3:
	if k <= 0.0:
		return desired
	return current.lerp(desired, 1.0 - exp(-k * delta))


## look_at 退化守卫：与注视点重合、或视线与 UP 共线时不调用（否则会抖动 / 报警）。
func _look_at_safe(p_focus: Vector3) -> void:
	var to_focus := p_focus - global_position
	var gap := to_focus.length()
	if gap < MIN_LOOK_DIR:
		return
	var up := Vector3.UP
	if absf((to_focus / gap).dot(Vector3.UP)) > 0.999:
		up = Vector3.BACK
	look_at(p_focus, up)


func _is_target_valid() -> bool:
	return target != null and is_instance_valid(target) and target.is_inside_tree()


## 目标被 queue_free 后置 null（防悬空引用），只在真正失联时打印一次。
func _prune_target() -> void:
	if target == null:
		return
	if not is_instance_valid(target) or not target.is_inside_tree():
		print("[camera] target -> <none> (目标已离场)")
		target = null


func _current_distance() -> float:
	match mode:
		Mode.ORBIT:
			return _orbit_dist
		Mode.FOLLOW:
			return _follow_dist
		_:
			return global_position.distance_to(GetFocusPoint())


func _compare_node_name(a: Node3D, b: Node3D) -> bool:
	return str(a.name) < str(b.name)
