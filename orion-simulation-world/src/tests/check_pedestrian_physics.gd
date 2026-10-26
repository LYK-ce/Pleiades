# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# task_35 Step 5 第 4 + 5 项 —— **人撞车（车静止，Godot 层物理）** 与 **雷达可见** 的 headless 自检
# （22 项：S 组结构 7 + P 组推挤 8 + L 组雷达 7）。
#
# 回答两个问题（都只用既有接口，不新增任何生产代码路径）：
#   ④ 一台**静止**的 diff_drive UGV 停在空地上，一个 `Pedestrian.tscn` 以 1.2 m/s 正面走过来、
#      推 300 个物理帧：**不穿透**（胶囊轴到车身盒的水平距离始终 >= -1 mm）、
#      人被挡停（或沿车身滑开）、**车一动不动**（位移 < 1 mm —— 人推不动车）。
#   ⑤ 同一个人挡在车头时，`Lidar2D.ScanOnce()` 的 ±45° 扇区最近命中 ≈ 车头到胶囊表面（±5 cm）
#      且 < 3 m；把人挪到 10 m 外之后扇区最近就 > 3 m（证明命中的**就是这个人**）。
#
# 跑法（本文件是 headless 档，不需要 X / 不需要渲染）：
#   timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_pedestrian_physics.gd
#   期望：total=22 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR。
#   **必须带 `--fixed-fps 60`**：本文件靠 `await physics_frame` 自己数帧，需要 delta 恒 = 1/60。
#   （反过来说：Python 墙钟驱动的端到端验收**绝对不能**带 `--fixed-fps`，plan.md §2.1 / §6 R1。）
#
# 关键取舍（每条都有实测或代码依据，别"优化"掉）：
#   ① 世界入树**之前**把 `Systems/WebSocketManager.autolisten` 关掉：否则本自检会抢 9999，
#      既可能和正在跑的 e2e 世界打架（`ERROR: ... bind`），也会让"0 ERROR"验收假红（§6 R3）。
#   ② 雷达装**车头**（mount.x = CAR_HALF_X、mount.z = 0.6）：`Lidar2D` 量的是"雷达到命中点"，
#      而 step 的判据是"**车头到**胶囊表面"；装原点会差 0.15 m > ±5 cm 必红。
#      高度 0.6 m 落在胶囊圆柱段（Godot y ∈ [0.25, 1.50]）里，横截面**恰好** r = 0.25；
#      再低就落到下半球，横截面半径 < 0.25，会白吃 5 mm 容差（§6 R8）。
#   ③ 穿透判据 = **胶囊轴到盒子的水平距离 − r**，并且**逐帧**取 min（事后取 min 会掩盖瞬时穿透）：
#      `move_and_slide` 的 safe_margin 是 1 mm，静止间隙落在 [0, 1 mm] 里是正常的，
#      所以判据是 `gap >= -1 mm`，不是"恰好 0"（§6 R7）。
#   ④ 挪人（L 组）之后**必须**再 `Configure(ped, {})`：`Pedestrian.Configure()` 才把
#      `_yaw_target` 对齐当前 `rotation.y`；不调的话下一帧 `Step()` 会把它拧回旧目标（§6 R10）。
#   ⑤ 人**不能**当 Vehicle 的底盘塞：`Pedestrian._physics_process()` 在
#      `vehicle != null and vehicle != self` 时直接 return（§6 R11）→ 本文件走 `Configure(ped, ped, {})`
#      这条自宿主路径（世界 `BuildPedestrians()` 也是这么写的）。
#   ⑥ 本文件**不 push_error**：失败一律 `_check(label, false, detail)` + `quit(1)`（§6 R14）；
#      退出前逐级 `queue_free()` + 逐级 await 两帧，否则 `resources still in use at exit`（§6 R13）。
#   ⑦ 选点 (60,84) 面朝东：世界预置的两个人 (44,96)/(52,102) 都在 19.8 m 外（> 雷达 12 m 量程），
#      最近的静态障碍 wall_c 在 8 m 北侧、其**东端角**的方位角 53.9° 落在 ±45° 扇区之外
#      （它的东侧边缘在 ±45° 扇区里命中不到；后向扇区能扫到 11 m 之外的墙面 —— L5 允许 > 3 m）。
#
# 注意（GDScript 4.7）：`load()` / `instantiate()` / `call()` 的返回值一律显式标注类型。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")

# ---------------------------------------------------------------- 场景（唯一来源）
const WORLD_SCENE := "res://src/scenes/World.tscn"
const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const LIDAR_SCENE := "res://src/scenes/sensors/Lidar2D.tscn"
const PEDESTRIAN_SCENE := "res://src/scenes/props/Pedestrian.tscn"
const CHASSIS_NAME := "diff_drive_ugv"
const LIDAR_SLOT := "lidar_nose"

# ---------------------------------------------------------------- 几何（世界坐标，Z-up，米）
## 停车点：check_camera_shots 用过的同一块空地（8 m 内无静态障碍、离预置人物 20 m）。
const CAR_SPOT_W := Vector3(60.0, 84.0, 0.0)
## 车头朝 +X 东（世界 yaw 0）。车静止 = 本文件**从不下发**任何 SetMotionCommand。
const CAR_YAW := 0.0
## 车身碰撞盒（`DiffDrive` 的 wheel_base/height/track_width = 模型 AABB = 0.30 × 0.25 × 0.25）。
const CAR_BOX := Vector3(0.30, 0.25, 0.25)
const CAR_HALF_X := 0.15
const CAR_HALF_Y := 0.125
## 人起点：车心正东 2.5 m → 车头前表面再往前 2.35 m（0.15 车头 + 0.25 胶囊 + 2.10 走行段）。
const PED_START_W := Vector3(62.5, 84.0, 0.0)
## 人朝 −X 西（正碰，最稳；侧碰会变成"沿车身滑开"，两种行为都要覆盖但正碰最可复现）。
const PED_YAW := PI
## 胶囊半径（Pedestrian.tscn 的 CapsuleShape3D radius）。
const CAPSULE_R := 0.25
## 胶囊总高 / 中心高（底面正好 y = 0 = 脚底）。
const CAPSULE_H := 1.75
const CAPSULE_CENTER_Y := 0.875
## step.md 给的推挤速度。
const PUSH_V := 1.2
## step.md 硬性：推 300 个物理帧。
const PUSH_FRAMES := 300
## "贴实"速度 / 帧数：1.2 m/s 在 60 fps 下的单帧步长是 0.02 m，`move_and_slide` 会在一整步都会穿透的
## 那一帧停住 → 静止间隙可能停在 [0, 0.02] 的任一点（实测 0.0199）。P7 要量"车头到胶囊表面"的
## **真实**值，所以在 300 帧之后再用 0.05 m/s 逼近 60 帧（0.83 mm/帧，共 0.05 m > 0.02）。
const SETTLE_V := 0.05
const SETTLE_FRAMES := 60
## 走行段保险帧数（实测 ~105 帧接触，留 6× 余量）。
const APPROACH_GUARD := 600
## step.md 硬性：车位移 < 1 mm。
const CAR_MOVE_MAX_M := 1.0e-3
## 不穿透：`move_and_slide` 的 safe_margin = 1 mm → 允许 [-1 mm, 0] 的静止间隙。
const GAP_MIN_M := -1.0e-3
## "真的碰到了"的判据（否则 P 组是无意义空转）。
## **必须**按离散步长取：1.2 m/s @ 60 fps = 0.02 m/帧，`move_and_slide` 在"一整步都会穿透"的那一帧
## 就停住，所以静止间隙落在 [0, 0.02] 里（本轮实测 0.019952 = 恰好一整步），不可能 <= 5 mm。
## 阈值 = 一帧步长 + 5 mm 余量；P4 的"不穿透"另有 GAP_MIN_M 兜底，两把尺子各管一头。
const CONTACT_EPS_M := PUSH_V / 60.0 + 5.0e-3
## 雷达外参：**x = 车头**（step 判据是"车头到胶囊表面"）、z = 0.6（胶囊圆柱段中部）。
const LIDAR_MOUNT := {"x": 0.15, "y": 0.0, "z": 0.6, "yaw": 0.0}
## L 组：把人挪到车头前 2.5 m（→ 到胶囊表面 2.25 m）。
const LIDAR_STANDOFF_M := 2.5
## L6：再挪到车头前 10 m（→ 9.75 m）。
const LIDAR_FAR_M := 10.0
## step 硬性：±5 cm。
const LIDAR_TOL_M := 0.05
## step 硬性：< 3 m。
const LIDAR_MAX_M := 3.0
## ±45° 扇区。
const SECTOR_DEG := 45.0
## 半径 0.25 m 的胶囊在 2.5 m 处的张角 = 2·asin(0.25/2.5) ≈ 11.5° → 约 11~13 条射线。
const PERSON_RAYS_MIN := 6
const PERSON_RAYS_MAX := 26

# ---------------------------------------------------------------- 结构判据
## 体素格数下限（实测 12104；只做"世界真的建了"的弱断言）。
const MIN_CELLS := 12000
## 空气墙 / 顶盖的 box 数（World.gd AIR_WALL_BOXES，实测 5）。
const WANT_BOUNDS := 5
## 位姿容差：1 mm / 1 mrad。
const POSE_TOL_M := 1.0e-3
## 静止性观测帧数（无指令 -> 位移应为 0）。
const STILL_FRAMES := 12
const STILL_EPS := 1.0e-6
## 雷达期望参数（Lidar2D.gd 的 DEFAULT_*）。
const WANT_RAYS := 360
const WANT_RANGE := 12.0
const WANT_SCAN_MASK := 1 | 2 | 4

# ---------------------------------------------------------------- 状态
var _total: int = 0
var _failed: int = 0

var _world: Node3D = null
var _veh: Node3D = null
var _ped: Node3D = null
var _lidar: Node3D = null
var _ws: Node = null
var _chassis_ok: bool = false


func _initialize() -> void:
	_run()


func _run() -> void:
	var ok: bool = await _Build_World()
	if ok:
		ok = _Check_Ws_Inert()
	if ok:
		ok = await _Build_Vehicle()
	if ok:
		ok = await _Check_Car_Structure()       # 里面有 await physics_frame -> 必须 await
	if ok:
		ok = await _Build_Pedestrian()
	if ok:
		ok = _Check_Ped_Structure()
	if ok:
		ok = _Check_Spawn_Pose()
	if ok:
		ok = _Check_Lidar_Structure()
	if ok:
		ok = await _Run_Push()
	if ok:
		ok = await _Run_Lidar()
	await _Teardown()
	_Finish()


# ================================================================ S 组：结构与基线（7 项）

## S1：载真 `World.tscn`（入树前关 WS autolisten），断言体素世界 + 空气墙真的建起来了。
func _Build_World() -> bool:
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	var world: Node3D = packed.instantiate() as Node3D if packed != null else null
	if world == null:
		_Check_Fail_All(["S1-world-load"])
		return false
	_ws = world.get_node_or_null("Systems/WebSocketManager")
	if _ws != null:
		_ws.set("autolisten", false)          # §6 R3：不进 tree 就抢 9999 是"0 ERROR"的头号杀手
	root.add_child(world)
	_world = world
	await process_frame
	await process_frame

	var voxel: GridMap = world.get_node_or_null("VoxelWorld") as GridMap
	var cells: int = voxel.get_used_cells().size() if voxel != null else -1
	var bounds: Node = world.get_node_or_null("Bounds")
	var bounds_children: int = bounds.get_child_count() if bounds != null else -1
	_check("S1-world-load", cells >= MIN_CELLS and bounds_children == WANT_BOUNDS,
		"cells=%d(>=%d) bounds_children=%d(==%d)" % [cells, MIN_CELLS, bounds_children, WANT_BOUNDS])
	return true


## S2：WS 惰性（节点在、autolisten 关了、没 listen 9999）。
func _Check_Ws_Inert() -> bool:
	var autolisten: bool = bool(_ws.get("autolisten")) if _ws != null else true
	var listening: bool = bool(_ws.call("IsListening")) if _ws != null else true
	var ok: bool = _ws != null and not autolisten and not listening
	_check("S2-ws-inert", ok, "node=%s autolisten=%s IsListening=%s（不抢 9999）"
		% [str(_ws != null), str(autolisten), str(listening)])
	return ok


## 摆一台**静止**的 UGV（父节点 = World/Vehicles，位置/朝向一律经 WorldTransform），挂一把车头雷达。
func _Build_Vehicle() -> bool:
	var packed: PackedScene = load(VEHICLE_SCENE) as PackedScene
	var node: Node = packed.instantiate() if packed != null else null
	_veh = node as Node3D
	if _veh == null:
		_Check_Fail_All(["S3-car-box", "S4-car-idle", "S5-ped-capsule", "S6-ped-spawn-pose",
			"S7-lidar-mount"])
		return false
	_world.call("GetVehiclesRoot").add_child(_veh)
	_veh.global_position = WorldTransform.PosToGodot(CAR_SPOT_W)
	_veh.rotation.y = WorldTransform.YawToGodot(CAR_YAW)
	await process_frame
	_chassis_ok = bool(_veh.call("ApplyChassis", CHASSIS_NAME, {}, null))
	await physics_frame
	var lidar_packed: PackedScene = load(LIDAR_SCENE) as PackedScene
	_lidar = _veh.call("AttachSensor", LIDAR_SLOT, lidar_packed, LIDAR_MOUNT) as Node3D
	await process_frame
	await physics_frame
	return true


## S3：底盘装配成功，且车身碰撞盒 = 0.30 × 0.25 × 0.25（模型 = 碰撞盒，task_33 Step 3 的口径）。
## S4：车静止基线（还没下发任何指令）—— 12 帧内位移必须 < 1e-6。
func _Check_Car_Structure() -> bool:
	var shape_node: CollisionShape3D = _veh.get_node_or_null("CollisionShape3D") as CollisionShape3D
	var shape: Shape3D = shape_node.shape if shape_node != null else null
	var box: BoxShape3D = shape as BoxShape3D
	var size: Vector3 = box.size if box != null else Vector3.ZERO
	var shape_ok: bool = _chassis_ok and box != null and size.is_equal_approx(CAR_BOX)
	_check("S3-car-box", shape_ok,
		"ApplyChassis(%s)=%s shape=%s size=%s(want %s) layer=%d mask=%d"
		% [CHASSIS_NAME, str(_chassis_ok), shape.get_class() if shape != null else "<none>",
			str(size), str(CAR_BOX), _veh.collision_layer, _veh.collision_mask])

	var pos0: Vector3 = _veh.global_position
	var vel0: Vector3 = _veh.get("velocity")
	for _i in range(STILL_FRAMES):
		await physics_frame
	var drift: float = _veh.global_position.distance_to(pos0)
	var idle_ok: bool = drift < STILL_EPS and vel0.is_zero_approx()
	_check("S4-car-idle", idle_ok, "frames=%d drift=%.9f(<0.000001) velocity0=%s（车没有任何 motion 指令）"
		% [STILL_FRAMES, drift, str(vel0)])
	return shape_ok and idle_ok


## 摆一个自己的行人（**不**塞进 Vehicle 的 chassis_root：那是 §6 R11 的坑）。
## 顺序不能反：add_child -> 写 global_transform -> 再 Configure（对齐 _yaw_target）。
func _Build_Pedestrian() -> bool:
	var packed: PackedScene = load(PEDESTRIAN_SCENE) as PackedScene
	var node: Node = packed.instantiate() if packed != null else null
	_ped = node as Node3D
	if _ped == null:
		_Check_Fail_All(["S5-ped-capsule", "S6-ped-spawn-pose", "S7-lidar-mount"])
		return false
	_world.call("GetPedestriansRoot").add_child(_ped)
	_ped.global_position = WorldTransform.PosToGodot(PED_START_W)
	_ped.rotation.y = WorldTransform.YawToGodot(PED_YAW)
	_ped.call("Configure", _ped, {})
	await physics_frame
	await physics_frame
	return true


## S5：胶囊形状 + 碰撞层掩码（layer=2 让雷达看得见；mask=1|4 = 地面 + 车辆，**不含** 2）。
func _Check_Ped_Structure() -> bool:
	var shape_node: CollisionShape3D = _ped.get_node_or_null("CollisionShape3D") as CollisionShape3D
	var capsule: CapsuleShape3D = (shape_node.shape as CapsuleShape3D) if shape_node != null else null
	var radius: float = capsule.radius if capsule != null else -1.0
	var height: float = capsule.height if capsule != null else -1.0
	var center_y: float = shape_node.position.y if shape_node != null else -1.0
	var ok: bool = capsule != null and is_equal_approx(radius, CAPSULE_R) \
		and absf(height - CAPSULE_H) <= 1.0e-6 and absf(center_y - CAPSULE_CENTER_Y) <= 1.0e-6 \
		and _ped.collision_layer == 2 and _ped.collision_mask == 5
	_check("S5-ped-capsule", ok,
		"r=%.3f(want %.2f) h=%.3f(want %.2f) center_y=%.3f(want %.3f) "
		% [radius, CAPSULE_R, height, CAPSULE_H, center_y, CAPSULE_CENTER_Y]
		+ "layer=%d(want 2) mask=%d(want 5=1|4)" % [_ped.collision_layer, _ped.collision_mask])
	return ok


## S6：人入树后的位姿 == PED_START_W / PED_YAW（≤1 mm / 1 mrad）。
func _Check_Spawn_Pose() -> bool:
	var pose: Dictionary = _ped.call("GetPose")
	var got := Vector3(float(pose.get("x", -999.0)), float(pose.get("y", -999.0)),
		float(pose.get("z", -999.0)))
	var d_pos: float = (got - PED_START_W).length()
	var d_yaw: float = absf(wrapf(float(pose.get("yaw", -999.0)) - PED_YAW, -PI, PI))
	var ok: bool = d_pos <= POSE_TOL_M and d_yaw <= POSE_TOL_M
	_check("S6-ped-spawn-pose", ok,
		"pos=(%.4f, %.4f) Δ=%.6f m(<=%.4f) yaw=%.4f°(want %.1f°) Δ=%.6f rad(<=%.4f)"
		% [got.x, got.y, d_pos, POSE_TOL_M, rad_to_deg(float(pose.get("yaw", 0.0))),
			rad_to_deg(PED_YAW), d_yaw, POSE_TOL_M])
	return ok


## S7：雷达挂在 Vehicle/Sensors/lidar_nose、外参 = 车头、参数 = 目录默认值。
func _Check_Lidar_Structure() -> bool:
	var sensors_root: Node = _veh.get_node_or_null("Sensors")
	var parent_ok: bool = _lidar != null and _lidar.get_parent() == sensors_root
	var rays: int = int(_lidar.get("ray_count")) if _lidar != null else -1
	var rng: float = float(_lidar.get("range_limit")) if _lidar != null else -1.0
	var mask: int = int(_lidar.get("scan_mask")) if _lidar != null else -1
	var want_mount: Vector3 = WorldTransform.PosToGodot(Vector3(
		float(LIDAR_MOUNT["x"]), float(LIDAR_MOUNT["y"]), float(LIDAR_MOUNT["z"])))
	var mount_ok: bool = _lidar != null and _lidar.position.is_equal_approx(want_mount)
	var ok: bool = parent_ok and rays == WANT_RAYS and is_equal_approx(rng, WANT_RANGE) \
		and mask == WANT_SCAN_MASK and mount_ok
	_check("S7-lidar-mount", ok,
		"slot=%s parent=Vehicle/Sensors:%s local_pos=%s(want %s) rays=%d(==%d) "
		% [str(_lidar.name) if _lidar != null else "null", str(parent_ok),
			str(_lidar.position) if _lidar != null else "null", str(want_mount),
			rays, WANT_RAYS]
		+ "range=%.1f(==%.0f) scan_mask=%d(==%d=1|2|4)"
		% [rng, WANT_RANGE, mask, WANT_SCAN_MASK])
	return ok


# ================================================================ P 组：人推车（8 项）

## 胶囊轴到车身盒的**水平**最近距离（矩形外为欧氏距离，矩形内为 0），再回退掉胶囊半径。
## 车 yaw = 0 → 车身盒的轴与世界轴对齐，所以 CAR_HALF_X / CAR_HALF_Y 可直接用。
static func _Gap_To_Box(ped_w: Vector3, car_w: Vector3) -> float:
	var ox: float = maxf(absf(ped_w.x - car_w.x) - CAR_HALF_X, 0.0)
	var oy: float = maxf(absf(ped_w.y - car_w.y) - CAR_HALF_Y, 0.0)
	return sqrt(ox * ox + oy * oy) - CAPSULE_R


## 车心到胶囊表面的水平距离（正碰时它就该等于 CAR_HALF_X = 0.15）。
static func _Surface_Dist(ped_w: Vector3, car_w: Vector3) -> float:
	var ox: float = maxf(absf(ped_w.x - car_w.x) - CAR_HALF_X, 0.0)
	var oy: float = maxf(absf(ped_w.y - car_w.y) - CAR_HALF_Y, 0.0)
	return sqrt(ox * ox + oy * oy) - CAPSULE_R + CAR_HALF_X


## 推 300 个物理帧。**唯一推进方式是 `await physics_frame`**（别用 process_frame 充数：
## 位置只在 `_physics_process()` 里由 `move_and_slide()` 更新）。
func _Run_Push() -> bool:
	var car0: Dictionary = _veh.call("GetPose")
	var car_x0: float = float(car0["x"])
	var car_y0: float = float(car0["y"])
	var car_yaw0: float = float(car0["yaw"])
	var ped0: Dictionary = _ped.call("GetPose")

	# P1 的零点：12 帧内车位移必须 < 1e-6（此刻世界上还没有任何推力）。
	var car_pos0: Vector3 = _veh.global_position
	for _i in range(STILL_FRAMES):
		await physics_frame
	var baseline: float = _veh.global_position.distance_to(car_pos0)

	# 下发**唯一**一条指令：v = 1.2，不带 dir（保持 Configure 对齐好的朝向 = 正西）。
	_ped.call("SetCommand", {"v": PUSH_V})

	var gap_min: float = 1.0e9
	var contact_at: int = -1
	var ped_x_hist: Array[float] = []
	var ped_y_hist: Array[float] = []
	var min_rel_x: float = 1.0e9                 # 全程 min(ped.x - car.x)，P8 用
	var ped_at_contact := Vector3.ZERO
	var i: int = 0
	while i < APPROACH_GUARD + PUSH_FRAMES:
		await physics_frame
		var pp: Dictionary = _ped.call("GetPose")
		var cp: Dictionary = _veh.call("GetPose")
		var ped_w := Vector3(float(pp["x"]), float(pp["y"]), float(pp["z"]))
		var car_w := Vector3(float(cp["x"]), float(cp["y"]), float(cp["z"]))
		var gap: float = _Gap_To_Box(ped_w, car_w)
		gap_min = minf(gap_min, gap)
		ped_x_hist.append(ped_w.x)
		ped_y_hist.append(ped_w.y)
		min_rel_x = minf(min_rel_x, ped_w.x - car_w.x)
		if gap <= CONTACT_EPS_M and contact_at < 0:
			contact_at = i
			ped_at_contact = ped_w
		if contact_at >= 0 and i - contact_at >= PUSH_FRAMES:
			break
		i += 1

	var car_last: Dictionary = _veh.call("GetPose")
	var ped_last: Dictionary = _ped.call("GetPose")
	var moved: float = Vector2(float(car_last["x"]) - car_x0, float(car_last["y"]) - car_y0).length()
	var yaw_delta: float = absf(wrapf(float(car_last["yaw"]) - car_yaw0, -PI, PI))
	var frames: int = ped_x_hist.size()
	var push_frames: int = maxi(frames - maxi(contact_at, 0), 0)

	# ---- P1 车静止基线
	var p1: bool = baseline < STILL_EPS
	_check("P1-car-baseline", p1, "frames=%d moved=%.9f(<0.000001)（后面的\"没被推动\"以它为零点）"
		% [STILL_FRAMES, baseline])

	# ---- P2 人真的走了（接触前累计位移 >= 1.0 m，实测约 2.1 m）
	var ped_start := Vector3(float(ped0["x"]), float(ped0["y"]), 0.0)
	var walked: float = (ped_at_contact - ped_start).length() if contact_at >= 0 else -1.0
	var p2: bool = walked >= 1.0
	_check("P2-ped-walked", p2, "contact_at_frame=%d walked=%.3f m(>=1.0) v=%.2f m/s"
		% [contact_at, walked, PUSH_V])

	# ---- P3 接触成立（否则本组是无意义空转）
	var p3: bool = contact_at >= 0 and gap_min <= CONTACT_EPS_M
	_check("P3-contact", p3, "gap_min=%.6f m(<=%.4f) contact_at_frame=%d/%d 推挤帧=%d"
		% [gap_min, CONTACT_EPS_M, contact_at, frames, push_frames])

	# ---- P4 全程不穿透（逐帧 min，不是事后取 min）
	var p4: bool = contact_at >= 0 and gap_min >= GAP_MIN_M
	_check("P4-no-penetration", p4, "gap_min=%.6f m(>=%.4f) 逐帧取 min（%d 帧）"
		% [gap_min, GAP_MIN_M, frames])

	# ---- P5 推 300 帧后车位移 < 1 mm（且 |Δyaw| < 0.01 rad）
	var p5: bool = moved < CAR_MOVE_MAX_M and yaw_delta < 0.01
	_check("P5-car-not-pushed", p5, "moved=%.9f m(<%.4f) yaw_delta=%.9f rad(<0.01) 推挤=%d 帧"
		% [moved, CAR_MOVE_MAX_M, yaw_delta, push_frames])

	# ---- P6 人被挡停 **或** 沿车身滑开（两个数都写进日志）
	var tail: int = mini(60, maxi(frames - 1, 1))
	var fwd: float = 0.0
	var lat: float = 0.0
	if frames > tail:
		fwd = ped_x_hist[frames - 1 - tail] - ped_x_hist[frames - 1]   # 向西（接近方向）为正
		for k in range(frames - 1 - tail, frames):
			lat = maxf(lat, absf(ped_y_hist[k] - ped_y_hist[frames - 1 - tail]))
	var blocked: bool = fwd < 5.0e-3
	var slid: bool = lat > 0.1
	var p6: bool = contact_at >= 0 and (blocked or slid)
	_check("P6-ped-blocked", p6,
		"最后 %d 帧：沿接近方向前进 %.6f m(<0.005 判挡停) 横向位移 %.6f m(>0.1 判滑开) → %s"
		% [tail, fwd, lat, "挡停" if blocked else ("滑开" if slid else "都没有")])

	# ---- "贴实"：v=1.2 + 60 fps ⇒ 单帧步长 0.02 m，`move_and_slide` 在**整步都会穿透**的那一帧就停下，
	#      所以 300 帧后静止间隙落在 [0, 0.020] 里任一点（本轮实测 0.0199 —— 恰好一整步）。
	#      这不是穿透也不是悬空，只是"离散步长"的量化。为了量出**真实**的车头→胶囊表面距离，
	#      再以 SETTLE_V 逼近 SETTLE_FRAMES 帧（0.83 mm/帧，共 0.05 m > 0.02），让间隙收敛到 safe_margin 量级。
	#      注意：P6 的"被挡停"用的是**贴实之前**那 300 帧的尾巴（那段才叫"推 300 帧"）。
	_ped.call("SetCommand", {"v": SETTLE_V})
	for _i in range(SETTLE_FRAMES):
		await physics_frame
	var ped_settled: Dictionary = _ped.call("GetPose")

	# ---- P7 车心 -> 人胶囊表面 >= 0 且 ≈ CAR_HALF_X（贴实之后的真实接触距离）
	var ped_end := Vector3(float(ped_settled["x"]), float(ped_settled["y"]),
		float(ped_settled["z"]))
	var car_end := Vector3(float(car_last["x"]), float(car_last["y"]), float(car_last["z"]))
	var surface: float = _Surface_Dist(ped_end, car_end)
	var surface_raw: float = _Surface_Dist(
		Vector3(float(ped_last["x"]), float(ped_last["y"]), 0.0), car_end)
	var p7: bool = surface >= 0.0 and absf(surface - CAR_HALF_X) <= 0.01
	_check("P7-surface-distance", p7,
		"%.4f m(>=0, want ~%.2f ±0.01) ped=(%.3f, %.3f) car=(%.3f, %.3f)（1.2 m/s 推完时 %.4f m，"
		% [surface, CAR_HALF_X, ped_end.x, ped_end.y, car_end.x, car_end.y, surface_raw]
		+ "以 %.2f m/s 贴实 %d 帧后 %.4f m）" % [SETTLE_V, SETTLE_FRAMES, surface])

	# ---- P8 人没被挤穿到车后（全程 ped.x > car.x）
	var p8: bool = min_rel_x > 0.0
	_check("P8-no-cross", p8, "min(ped.x - car.x)=%.4f m(>0) 全程 %d 帧" % [min_rel_x, frames])
	return p1 and p2 and p3 and p4 and p5 and p6 and p7 and p8


# ================================================================ L 组：雷达可见（7 项）

## 与 index 0（= 局部 +X = 车头）夹角的绝对值落在 [abs_min_deg, abs_max_deg] 里的最近**有效**命中
## （0.0 = 无回波）。用 `absf(wrapf(...))` 归一化，别直接拿 `TAU*i/n` 去比（负角会整段漏掉）。
static func _Sector_Min(ranges: PackedFloat32Array, abs_min_deg: float, abs_max_deg: float) -> float:
	var n: int = ranges.size()
	var best: float = 0.0
	for i in range(n):
		var deg: float = 360.0 * float(i) / float(n)
		var rel: float = absf(wrapf(deg, -180.0, 180.0))
		if rel < abs_min_deg or rel > abs_max_deg:
			continue
		var d: float = ranges[i]
		if d > 0.0 and (best <= 0.0 or d < best):
			best = d
	return best


## 同一扇区里的命中条数。
static func _Sector_Hits(ranges: PackedFloat32Array, abs_min_deg: float, abs_max_deg: float) -> int:
	var n: int = ranges.size()
	var count: int = 0
	for i in range(n):
		var rel: float = absf(wrapf(360.0 * float(i) / float(n), -180.0, 180.0))
		if rel < abs_min_deg or rel > abs_max_deg:
			continue
		if ranges[i] > 0.0:
			count += 1
	return count


## 把人瞬移到 "车心 + (CAR_HALF_X + standoff) 沿 +X"，重对齐 yaw 目标，停住，再扫一圈。
func _Scan_With_Ped_At(standoff_m: float) -> PackedFloat32Array:
	var car: Dictionary = _veh.call("GetPose")
	var target := Vector3(float(car["x"]) + CAR_HALF_X + standoff_m, float(car["y"]), 0.0)
	_ped.global_position = WorldTransform.PosToGodot(target)
	# 瞬移之后**必须**重新 Configure：它把 `_yaw_target` 对齐到当前 rotation.y（§6 R10）。
	_ped.call("Configure", _ped, {})
	_ped.call("SetCommand", {"v": 0.0})
	await physics_frame
	await physics_frame
	return _lidar.call("ScanOnce") as PackedFloat32Array


## L1~L7：车头前 2.5 m 有人 -> 扇区最近 ≈ 2.25 m；挪到 10 m -> > 3 m（命中的是这个人）。
func _Run_Lidar() -> bool:
	var near: PackedFloat32Array = await _Scan_With_Ped_At(LIDAR_STANDOFF_M)
	var want_near: float = LIDAR_STANDOFF_M - CAPSULE_R          # 2.50 - 0.25 = 2.25
	var s_min: float = _Sector_Min(near, 0.0, SECTOR_DEG)
	var s_hits: int = _Sector_Hits(near, 0.0, SECTOR_DEG)
	var nose: float = near[0] if near.size() > 0 else -1.0
	var rear: float = _Sector_Min(near, 180.0 - SECTOR_DEG, 180.0)
	var hits_total: int = 0
	for v: float in near:
		if v > 0.0:
			hits_total += 1

	# ---- L1 返回长度 = 360、至少 1 条命中 > 0
	var l1: bool = near.size() == WANT_RAYS and hits_total > 0
	_check("L1-scan-shape", l1, "len=%d(==%d) hits_total=%d(>0) 命中率=%.1f%%"
		% [near.size(), WANT_RAYS, hits_total, 100.0 * float(hits_total) / float(maxi(near.size(), 1))])

	# ---- L2 ±45° 扇区最近 ≈ 车头 -> 胶囊表面（±5 cm）
	var l2: bool = _L2_Ok(s_min, want_near)
	_check("L2-sector-nearest", l2, "d=%.4f m(want %.4f ±%.2f) 扇区内 %d 条命中"
		% [s_min, want_near, LIDAR_TOL_M, s_hits])

	# ---- L3 该距离 < 3 m
	var l3: bool = s_min > 0.0 and s_min < LIDAR_MAX_M
	_check("L3-under-3m", l3, "d=%.4f m(<%.1f)" % [s_min, LIDAR_MAX_M])

	# ---- L4 正前方那条（index 0）与扇区最小值一致（±1 cm）——证明最近点来自人的"正脸"
	var l4: bool = s_min > 0.0 and absf(nose - s_min) <= 0.01
	_check("L4-nose-ray", l4, "ranges[0]=%.4f m vs 扇区 min=%.4f m（Δ=%.4f <= 0.01）"
		% [nose, s_min, absf(nose - s_min)])

	# ---- L5 后向 ±45° 扇区无命中（0）或 > 3 m（wall_c 的东端角在后向扇区里是 11 m 开外）
	var l5: bool = rear <= 0.0 or rear > LIDAR_MAX_M
	_check("L5-rear-clear", l5, "rear_min=%.4f m(0 或 >%.1f)" % [rear, LIDAR_MAX_M])

	# ---- L7 命中段宽度 = 一个人的张角（2·asin(0.25/2.5) ≈ 11.5° → 约 11~13 条射线）
	var l7: bool = s_hits >= PERSON_RAYS_MIN and s_hits <= PERSON_RAYS_MAX
	_check("L7-person-width", l7, "扇区命中 %d 条(%d..%d) ≈ %.1f° 张角（r0.25 的胶囊 @%.2f m）"
		% [s_hits, PERSON_RAYS_MIN, PERSON_RAYS_MAX,
			rad_to_deg(2.0 * asin(CAPSULE_R / LIDAR_STANDOFF_M)), LIDAR_STANDOFF_M])

	# ---- L6 把人挪到车头前 10 m：扇区最近必须 > 3 m（或 0）——证明 L2 命中的**是这个人**
	var far: PackedFloat32Array = await _Scan_With_Ped_At(LIDAR_FAR_M)
	var far_min: float = _Sector_Min(far, 0.0, SECTOR_DEG)
	var l6: bool = far_min <= 0.0 or far_min > LIDAR_MAX_M
	_check("L6-moved-away", l6, "d=%.4f m(0 或 >%.1f) 人挪到车头前 %.1f m（→ 表面 %.2f m）"
		% [far_min, LIDAR_MAX_M, LIDAR_FAR_M, LIDAR_FAR_M - CAPSULE_R])
	return l1 and l2 and l3 and l4 and l5 and l6 and l7


## L2 的判据单独抽出来，只为让 `_check` 那一行读起来不夹三个条件。
static func _L2_Ok(got: float, want: float) -> bool:
	return got > 0.0 and absf(got - want) <= LIDAR_TOL_M


# ================================================================ 收尾 / 断言

## 逐级 queue_free + 逐级 await 两帧（漏了会打 `ERROR: resources still in use at exit`，§6 R13）。
func _Teardown() -> void:
	var nodes: Array[Node] = [_ped, _veh, _world]
	for node: Node in nodes:
		if node != null and is_instance_valid(node):
			node.queue_free()
			await process_frame
			await process_frame
	_ped = null
	_veh = null
	_world = null
	_lidar = null


## 前置失败时把后面不会再跑的断言一次性标红（保持 total 恒定 = 22，别让"少跑"伪装成"全绿"）。
func _Check_Fail_All(labels: Array) -> void:
	for label: Variant in labels:
		_check(String(label), false, "前置失败：本组未执行")


func _check(label: String, ok: bool, detail: String = "") -> bool:
	_total += 1
	if not ok:
		_failed += 1
	var suffix: String = "" if detail.is_empty() else " " + detail
	if ok:
		print("[check] %s ok%s" % [label, suffix])
	else:
		print("[check] %s FAIL%s" % [label, suffix])
	return ok


func _Finish() -> void:
	print("[check] env display=%s adapter='%s'" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
