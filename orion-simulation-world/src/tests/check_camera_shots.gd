# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-23
#
# task_33 Step 4 — 模拟世界拍照自检（真 World + 静止 UGV + 建模雷达/相机 -> 3 张 JPEG 落盘）。
#
# 做什么：把一台**静止**的 UGV 摆进真 World.tscn（体素/地面/空气墙），挂 Step 1 建模出来的
#   Lidar2D + CameraSensor 两件传感器，然后用**同一台相机**换机位拍 3 张 1280×720 JPEG 到
#   tmp/task33_shots/（绝对路径），就地把 35 项断言跑完。
#   **不启动 pleiades-sim、不连 WS、不跑决策链 / on_tick、不造 motion_cmd**（task_33 §九 非目标）。
#
# 跑法 ①（纯 headless，结构档：S 组 18 项；R 组 17 项全部 [skip] 且**不计入 total**）：
#   timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_camera_shots.gd
#   期望：total=18 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR + 无 "[ws] listening"。
#
# 跑法 ②（**本 step 的真验收**：虚拟显示 + 软件 OpenGL，S+R 全跑，出 3 张图）：
#   bash tmp/task33/xvfb/start_xvfb.sh
#   DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot \
#     --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
#     --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_camera_shots.gd
#   期望：total=35 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR。
#   实测基线（Xvfb :99 + llvmpipe，见 COMPONENTS.md §4.8.2）：
#     字节 26315 / 24462 / 29576，var 0.0096~0.0126，mean 0.49~0.57，
#     车体投影占比 0.0000 / 0.0000 / 0.0689，pillar_e 屏内角点 8 / 0。
#
# 为什么必须分两档跑（实测事实，COMPONENTS.md §9.1 E26）：
#   ① `--headless` = `--display-driver headless`，只支持 dummy 渲染驱动：
#      `frame_post_draw` 永不触发、`get_texture().get_image()` 返回 null 并打
#      `ERROR: Parameter "t" is null`。=> headless 下"能出图"物理不可能。
#      本脚本用 CameraSensor.Can_Capture() 在**碰渲染器之前**分流，R 组整体 SKIP。
#   ② 真渲染必须 `--audio-driver Dummy`：本机无声卡，ALSA 打不开会打一行 ERROR，
#      把"0 ERROR"验收搞假红。
#   ③ 本脚本自身**不 push_error**（失败一律 `_check(..., false, detail)` + `quit(1)`），
#      否则测试脚本自己就把"日志 0 ERROR"验收搞红。
#
# 坐标：位置一律 WorldTransform.PosToGodot()、朝向一律 WorldTransform.YawToGodot()；
#   本文件**不出现**任何 (x, z, y) 换轴字面量（COMPONENTS.md §2.3）。
#
# 注意（GDScript 4.7）：`load()` / `instantiate()` 的返回值一律显式标注类型；
#   退出前必须 `queue_free()` + 逐级 await 两帧，否则报 `ERROR: resources still in use at exit`。
#   首次运行若模型 load() 静默返回 null，先跑 `godot --headless --path orion-simulation-world --import`。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")

# ---------------------------------------------------------------- 场景（唯一来源）
const WORLD_SCENE := "res://src/scenes/World.tscn"
const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const LIDAR_SCENE := "res://src/scenes/sensors/Lidar2D.tscn"
const CAMERA_SCENE := "res://src/scenes/sensors/CameraSensor.tscn"
const SCRIPT_PATH := "res://src/tests/check_camera_shots.gd"

# ---------------------------------------------------------------- 落盘目录（绝对路径，step 硬性）
## 图片必须落在仓库下的 tmp/task33_shots/，**不得**写进工程目录（res:// 会把图塞进仓库）。
const OUT_DIR := "/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/task33_shots"
## 后缀用于断言"没写进工程目录"（S15）。
const OUT_SUFFIX := "tmp/task33_shots"

# ---------------------------------------------------------------- 机位（世界坐标，Z-up，米）
## 出生空地：wall_c（世界 y∈[92,93]、x∈[36,66]）以北、pillar_e（世界 x∈[78,80]、y∈[78,80]）以西。
const SPOT_WORLD := Vector3(60.0, 84.0, 0.0)
## 世界 yaw 0 = 车头 +x（东）→ 正对 8 m 高的 pillar_e。
const HEADING_A := 0.0
## A + 90°（南）→ 正对 30 m 长的 wall_c；pillar_e 落到车后。
const HEADING_B := PI * 0.5

# ---------------------------------------------------------------- 外参（车体局部、世界语义）
## 雷达：车顶中心略偏后。
const LIDAR_MOUNT := {"x": -0.05, "y": 0.0, "z": 0.25, "yaw": 0.0}
## 相机：车鼻前表面（x=0.15 = 车身半长），**前视** = mount.yaw 取 +π/2。
##   单位提醒（实测，COMPONENTS.md §4.8 / §9.1 E26）：`yaw` 是**弧度**、`pitch` 是**度**。
##   光轴 = 传感器局部 −Z，车头 = 车体局部 +X，所以"看车头方向"等价于 yaw = +π/2；
##   写 0 会侧看、把车自己的壳拍满画面（实测车体投影占比 1.3767），S11/R6 立刻红。
const CAM_MOUNT := {"x": 0.15, "y": 0.0, "z": 0.12, "yaw": PI * 0.5, "pitch": 0.0}

## 第三机位：外部相机 = 车头 3/4 视角（世界几何 + 车体 + 车顶雷达同框）。
const EXT_FWD_M := 0.62
const EXT_SIDE_M := 0.40
const EXT_UP_M := 0.48
const EXT_LOOK_UP_M := 0.14

## 机位表（顺序即文件名前缀；R 组断言按这张表驱动）。
const SHOTS := [
	{"id": 0, "file": "shot_00_heading_a.jpg", "kind": "onboard", "heading": HEADING_A},
	{"id": 1, "file": "shot_01_heading_a_plus_90.jpg", "kind": "onboard", "heading": HEADING_B},
	{"id": 2, "file": "shot_02_external_3q.jpg", "kind": "external", "heading": HEADING_A},
]

# ---------------------------------------------------------------- 结构判据
## 体素格数下限（实测 12104；只做"世界真的建了"的弱断言）。
const MIN_CELLS := 12000
## 空气墙 / 顶盖的 box 数（World.gd AIR_WALL_BOXES，实测 5）。
const WANT_BOUNDS := 5
## 静止性观测帧数（无指令 -> 位移应为 0）。
const STILL_FRAMES := 12
## 位移容差（米）。
const STILL_EPS := 1e-6
## 相机参数期望值（CameraSensor.gd 的 DEFAULT_*）。
const WANT_SIZE := Vector2i(1280, 720)
const WANT_FOV := 70.0
const WANT_QUALITY := 0.8
## "光轴 = 车头"的几何契约阈值（实测 1.000000）。
const NOSE_DOT_MIN := 0.999
## 前视挂载的 rotation.y 期望值（= -π/2，锁定 mount.yaw 的单位是弧度）。
const WANT_MOUNT_ROT_Y := -PI * 0.5

# ---------------------------------------------------------------- 图像判据（计划期实测值的 3~5 倍余量）
const MIN_BYTES := 10 * 1024        # step 硬性：单张 > 10 KB
const VAR_MIN := 0.002              # 灰度方差下限（实测 0.0083~0.0126）
const MEAN_MIN := 0.05              # 非全黑
const MEAN_MAX := 0.95              # 非全白
const SPREAD_MIN := 0.15            # max - min（实测 0.686~0.957）
const WHITE_MAX := 0.50             # 近白像素占比上限（实测 0.000~0.005）
const GRID_COLS := 16               # 网格列数
const GRID_ROWS := 9                # 网格行数（144 格）
const GRID_STEP := 3                # 格内抽样步长（像素）
const GRID_CONTRAST_MIN := 20       # 格内 max-min>0.05 的格数（实测 42~56）
const GRID_MIDBAND_MIN := 10        # |格均值-全图均值|>0.10 的格数（实测 28~30）
const DIFF_BYTE_MIN := 0.10         # 两两字节差异占比（实测 0.823~0.963）
const DIFF_MEAN_MIN := 0.02         # 两两灰度平均绝对差（实测 0.070~0.115）
const DIFF_STEP := 8                # 灰度差抽样步长（像素）

# ---------------------------------------------------------------- 几何判据
## step 硬性：车鼻占比 < 50%；前视机位实测 0.0000，取 0.05 做强化版。
const CAR_RATIO_MAX := 0.05
## 外部机位必须真的拍到车（实测 0.0689）。
const CAR_RATIO_EXT_MIN := 0.02
const CAR_RATIO_EXT_MAX := 0.30
## pillar_e 的屏幕角点数：朝东 >= 4（实测 8）；朝南 <= 2（实测 0）。
const PILLAR_IN_MIN := 4
const PILLAR_IN_MAX_B := 2
## pillar_e 世界包围盒（World.gd OBSTACLES：origin (78,78,0)、size (2,2,8)）。
const PILLAR_ORIGIN := Vector3(78.0, 78.0, 0.0)
const PILLAR_SIZE := Vector3(2.0, 2.0, 8.0)
## 落盘时间容差（秒）：防拿旧图充数。
const FILE_FRESH_SLACK_S := 2

# ---------------------------------------------------------------- 白盒 lint
## 脚本自身**不得**出现这些符号（证明"不跑决策链、不连 WS"）。
## 字面量必须**拼接**着写：本数组就在被检查的源文件里，写整串会自己命中自己。
const BANNED_TOKENS := ["on" + "_tick", "WebSocket" + "Peer", "connect" + "_to", "HTTP" + "Client"]

# ---------------------------------------------------------------- 状态
var _total: int = 0
var _failed: int = 0
var _skipped: int = 0

var _world: Node3D = null
var _veh: Node3D = null
var _lidar: Node3D = null
var _cam: Node3D = null
var _ext_cam: Node3D = null
var _ext_holder: Node3D = null
var _ws: Node = null
var _chassis_ok: bool = false
var _start_unix_s: int = 0
var _capture_total: int = 0

## 逐张实测数据（抓拍后就地测量，投影类断言必须在抓拍之后才有意义）。
var _shots: Array[Dictionary] = []


func _initialize() -> void:
	_run()


func _run() -> void:
	_start_unix_s = int(Time.get_unix_time_from_system())

	# ============================================================ ① 组世界（关 WS）
	var world_ok: bool = await _Build_World()
	if not world_ok:
		await _Teardown()
		_abort("载入/实例化 %s 失败（首次运行先跑 --import）" % WORLD_SCENE)
		return

	# ============================================================ ② 摆车 + 挂传感器
	var veh_ok: bool = await _Spawn_Vehicle()
	if not veh_ok:
		await _Teardown()
		_abort("载入/实例化 %s 失败（首次运行先跑 --import）" % VEHICLE_SCENE)
		return
	var sensors_ok: bool = await _Attach_Sensors()
	if not sensors_ok:
		await _Teardown()
		_abort("AttachSensor 失败：%s / %s" % [LIDAR_SCENE, CAMERA_SCENE])
		return

	# ============================================================ ③ S 组：结构（两档都跑）
	await _Check_Structural()

	# ============================================================ ④ R 组：真渲染（仅真渲染器）
	var can_capture: bool = _cam.Can_Capture()
	if can_capture:
		await _Run_Render()
	else:
		_skip(_Render_Labels())

	# ============================================================ ⑤ 释放 + 退出
	await _Teardown()
	_finish()


# ================================================================ 组场景

## 载 World.tscn：**add_child 之前**把自带的 WebSocketManager.autolisten 关掉。
## 为什么：World.tscn 的 Systems 下挂着 WebSocketManager（autolisten=true），一入树就 listen(9999)，
##   与 step「不连 WS」冲突（历史日志里出现过 `[ws] listening on port 9999`）。
##   只写导出位、不改 .tscn、不 queue_free 该节点（那会打 `[ws] stopped` 噪音）。
func _Build_World() -> bool:
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	if packed == null:
		return false
	var node: Node = packed.instantiate()
	if node == null:
		return false
	_world = node as Node3D
	if _world == null:
		return false
	_ws = _world.get_node_or_null("Systems/WebSocketManager")
	if _ws != null:
		_ws.set("autolisten", false)
	root.add_child(_world)
	await process_frame
	await process_frame
	return _world.is_inside_tree()


## 摆一台静止 UGV：父节点 = World/Vehicles，位置/朝向一律经 WorldTransform。
## 返回值只表示"载入 + 入树"成功；底盘装配结果存在 `_chassis_ok`（装配失败不中断，让 S5 红）。
func _Spawn_Vehicle() -> bool:
	var packed: PackedScene = load(VEHICLE_SCENE) as PackedScene
	if packed == null:
		return false
	var node: Node = packed.instantiate()
	if node == null:
		return false
	_veh = node as Node3D
	if _veh == null:
		return false
	_world.GetVehiclesRoot().add_child(_veh)
	_veh.global_position = WorldTransform.PosToGodot(SPOT_WORLD)
	_veh.rotation.y = WorldTransform.YawToGodot(HEADING_A)
	await process_frame
	# 显示建模底盘（ModelUGV = assets/chassis/ugv_chassis.glb），关掉橙色占位盒。
	_chassis_ok = _veh.ApplyChassis("diff_drive_ugv", {}, null)
	await physics_frame
	return true


## 挂两件传感器：雷达（建模 lidar_tmini.glb）+ 相机（建模 camera_module.glb）。
func _Attach_Sensors() -> bool:
	var lidar_packed: PackedScene = load(LIDAR_SCENE) as PackedScene
	var cam_packed: PackedScene = load(CAMERA_SCENE) as PackedScene
	if lidar_packed == null or cam_packed == null:
		return false
	_lidar = _veh.AttachSensor("lidar_top", lidar_packed, LIDAR_MOUNT) as Node3D
	_cam = _veh.AttachSensor("camera_nose", cam_packed, CAM_MOUNT) as Node3D
	await process_frame
	await process_frame
	return _lidar != null and _cam != null


# ================================================================ S 组：结构（恒跑）

## S1~S18：全部在**碰渲染器之前**能判的条件（headless / Xvfb 两档都跑）。
func _Check_Structural() -> void:
	# ---- S1 世界载入 + 入树
	_check("S1-world-loads", _world != null and _world.is_inside_tree(),
		"scene=%s inside_tree=%s" % [WORLD_SCENE, str(_world != null and _world.is_inside_tree())])

	# ---- S2 体素世界 + 空气墙真的建起来了
	var voxel: GridMap = _world.get_node_or_null("VoxelWorld") as GridMap
	var cells: int = voxel.get_used_cells().size() if voxel != null else -1
	var bounds: Node = _world.get_node_or_null("Bounds")
	var bounds_children: int = bounds.get_child_count() if bounds != null else -1
	_check("S2-world-built", cells >= MIN_CELLS and bounds_children == WANT_BOUNDS,
		"cells=%d(>=%d) bounds_children=%d(==%d)" % [
			cells, MIN_CELLS, bounds_children, WANT_BOUNDS])

	# ---- S3 WS 惰性：节点在、autolisten 关了、没 listen(9999)
	var autolisten: bool = bool(_ws.get("autolisten")) if _ws != null else true
	var listening: bool = bool(_ws.call("IsListening")) if _ws != null else true
	_check("S3-ws-inert", _ws != null and not autolisten and not listening,
		"node=%s autolisten=%s IsListening=%s（不监听 9999，日志不得出现 ws 监听行）" % [
			str(_ws != null), str(autolisten), str(listening)])

	# ---- S4 车的父节点 / 位置 / 朝向
	var want_pos: Vector3 = WorldTransform.PosToGodot(SPOT_WORLD)
	# `+ 0.0` 只为把 YawToGodot(0) 的 -0.0 归一成 0.0，避免日志打 "-0.000000"。
	var want_rot_y: float = WorldTransform.YawToGodot(HEADING_A) + 0.0
	var parent_ok: bool = _veh.get_parent() == _world.GetVehiclesRoot()
	_check("S4-vehicle-spawn",
		parent_ok and _veh.global_position.is_equal_approx(want_pos)
			and is_equal_approx(_veh.rotation.y, want_rot_y),
		"parent=World/Vehicles:%s pos=%s(期望 %s) rotation.y=%.6f(期望 %.6f)" % [
			str(parent_ok), str(_veh.global_position), str(want_pos),
			_veh.rotation.y, want_rot_y])

	# ---- S5 画面里是建模底盘（ModelUGV），不是橙色占位盒（Body）
	var model: Node3D = _veh.get_node_or_null("ModelUGV") as Node3D
	var body: MeshInstance3D = _veh.get_node_or_null("Body") as MeshInstance3D
	_check("S5-chassis-model-shown",
		_chassis_ok and model != null and model.visible and body != null and not body.visible,
		"ApplyChassis=%s ModelUGV.visible=%s Body.visible=%s" % [
			str(_chassis_ok),
			str(model != null and model.visible),
			str(body != null and body.visible)])

	# ---- S6 静止性：12 个物理帧、无指令、无决策链 -> 位移 0
	var pos0: Vector3 = _veh.global_position
	for _i in range(STILL_FRAMES):
		await physics_frame
	var drift: float = _veh.global_position.distance_to(pos0)
	_check("S6-vehicle-still", drift < STILL_EPS,
		"frames=%d drift=%.6f(<%.6f) velocity=%s" % [
			STILL_FRAMES, drift, STILL_EPS, str(_veh.get("velocity"))])

	# ---- S7 / S8 雷达：挂在 Vehicle/Sensors 下、外参正确
	var sensors_root: Node = _veh.get_node_or_null("Sensors")
	var lidar_parent_ok: bool = _lidar != null and _lidar.get_parent() == sensors_root
	_check("S7-lidar-attached",
		_lidar != null and lidar_parent_ok and _lidar.name == StringName("lidar_top"),
		"slot=%s parent=Vehicle/Sensors:%s" % [
			str(_lidar.name) if _lidar != null else "null", str(lidar_parent_ok)])
	var lidar_want: Vector3 = WorldTransform.PosToGodot(Vector3(
		float(LIDAR_MOUNT["x"]), float(LIDAR_MOUNT["y"]), float(LIDAR_MOUNT["z"])))
	_check("S8-lidar-mount",
		_lidar != null and _lidar.position.is_equal_approx(lidar_want)
			and is_zero_approx(_lidar.rotation.y),
		"position=%s(期望 %s) rotation.y=%.6f(期望 0)" % [
			str(_lidar.position) if _lidar != null else "null",
			str(lidar_want), _lidar.rotation.y if _lidar != null else -1.0])

	# ---- S9 / S10 相机：挂在 Vehicle/Sensors 下、外参正确（rotation.x = pitch = 0）
	var cam_parent_ok: bool = _cam != null and _cam.get_parent() == sensors_root
	_check("S9-camera-attached",
		_cam != null and cam_parent_ok and _cam.name == StringName("camera_nose"),
		"slot=%s parent=Vehicle/Sensors:%s" % [
			str(_cam.name) if _cam != null else "null", str(cam_parent_ok)])
	var cam_want: Vector3 = WorldTransform.PosToGodot(Vector3(
		float(CAM_MOUNT["x"]), float(CAM_MOUNT["y"]), float(CAM_MOUNT["z"])))
	_check("S10-camera-mount",
		_cam != null and _cam.position.is_equal_approx(cam_want)
			and is_zero_approx(_cam.rotation.x),
		"position=%s(期望 %s) rotation.x=%.6f(期望 0 = pitch 0)" % [
			str(_cam.position) if _cam != null else "null",
			str(cam_want), _cam.rotation.x if _cam != null else -1.0])

	# ---- S11 光轴对着车头（"对着哪拍哪"的几何契约）
	var optic: Vector3 = -_cam.global_basis.z if _cam != null else Vector3.ZERO
	var fwd: Vector3 = _veh.global_basis.x.normalized()
	var dot: float = optic.dot(fwd)
	_check("S11-camera-looks-nose", dot > NOSE_DOT_MIN,
		"optic=%s veh_fwd=%s dot=%.6f(>%.3f)" % [str(optic), str(fwd), dot, NOSE_DOT_MIN])

	# ---- S12 mount.yaw 的单位是**弧度**：前视 = rotation.y 恰为 -π/2
	var rot_y: float = _cam.rotation.y if _cam != null else 0.0
	_check("S12-camera-yaw-unit", is_equal_approx(rot_y, WANT_MOUNT_ROT_Y),
		"rotation.y=%.6f(期望 %.6f = -PI/2；若有人改成度会变成 -90 rad)" % [
			rot_y, WANT_MOUNT_ROT_Y])

	# ---- S13 传感器纯视觉：两棵子树里零碰撞体 / 零物理体
	var physics_nodes: int = 0
	if _lidar != null:
		physics_nodes += _Count_Physics(_lidar)
	if _cam != null:
		physics_nodes += _Count_Physics(_cam)
	_check("S13-sensors-no-collision", physics_nodes == 0,
		"physics_nodes=%d（CollisionShape3D/Polygon3D/PhysicsBody3D/Area3D 必须为 0）"
			% physics_nodes)

	# ---- S14 相机参数 + Viewport 共享世界
	var sub: SubViewport = _cam.get_node_or_null("Viewport") as SubViewport if _cam != null else null
	var params: Dictionary = _cam.Get_Params() if _cam != null else {}
	var image_size: Vector2i = _cam.Get_Image_Size() if _cam != null else Vector2i(-1, -1)
	var own_world: bool = sub.own_world_3d if sub != null else true
	_check("S14-camera-params",
		image_size == WANT_SIZE and sub != null and not own_world
			and is_equal_approx(float(params.get("fov_deg", -1.0)), WANT_FOV)
			and is_equal_approx(float(params.get("quality", -1.0)), WANT_QUALITY),
		"size=%s own_world_3d=%s fov=%.1f quality=%.2f" % [
			str(image_size), str(own_world),
			float(params.get("fov_deg", -1.0)), float(params.get("quality", -1.0))])

	# ---- S15 落盘目录：绝对路径 + 不在工程目录 + 建得出来
	var absolute: bool = OUT_DIR.is_absolute_path()
	var suffix_ok: bool = OUT_DIR.ends_with(OUT_SUFFIX)
	var not_res: bool = not OUT_DIR.begins_with("res://")
	var made: bool = _Ensure_Out_Dir()
	_check("S15-out-dir",
		absolute and suffix_ok and not_res and made and DirAccess.dir_exists_absolute(OUT_DIR),
		"dir=%s absolute=%s suffix=%s not_res=%s exists=%s" % [
			OUT_DIR, str(absolute), str(suffix_ok), str(not_res),
			str(DirAccess.dir_exists_absolute(OUT_DIR))])

	# ---- S16 机位表：三个不同文件、覆盖 onboard×2 + external×1
	var seen: Array[String] = []
	var onboard: int = 0
	var external: int = 0
	var plan_ok: bool = SHOTS.size() >= 3
	for i in range(SHOTS.size()):
		var file: String = String(SHOTS[i]["file"])
		var kind: String = String(SHOTS[i]["kind"])
		if not file.begins_with("shot_0") or not file.ends_with(".jpg") or seen.has(file):
			plan_ok = false
		seen.append(file)
		if kind == "onboard":
			onboard += 1
		elif kind == "external":
			external += 1
		else:
			plan_ok = false
	_check("S16-shot-plan", plan_ok and onboard == 2 and external == 1,
		"shots=%d onboard=%d external=%d files=%s" % [
			SHOTS.size(), onboard, external, str(seen)])

	# ---- S17 渲染器闸：Can_Capture() 必须与"有没有真显示"一致（防 headless 悄悄跳过真问题）
	var headless: bool = DisplayServer.get_name() == "headless"
	_check("S17-renderer-gate", _cam.Can_Capture() == (not headless),
		"can_capture=%s display=%s adapter='%s'" % [
			str(_cam.Can_Capture()), DisplayServer.get_name(),
			RenderingServer.get_video_adapter_name()])

	# ---- S18 白盒 lint：自身源码不得出现决策链 / WS 客户端符号
	_Check_WhiteBox_Lint()


## S18：读自身源码，剥掉**整行**注释后不得出现决策链 / WS 客户端符号。
func _Check_WhiteBox_Lint() -> void:
	var src: String = _Read_Text(SCRIPT_PATH)
	if src.is_empty():
		_check("S18-whitebox-lint", false, "读不到 %s" % SCRIPT_PATH)
		return
	var code: String = _Strip_Line_Comments(src)
	var hits: Array[String] = []
	for token in BANNED_TOKENS:
		if code.contains(token):
			hits.append(token)
	_check("S18-whitebox-lint", hits.is_empty(),
		"banned=%s" % ("none" if hits.is_empty() else str(hits)))


# ================================================================ R 组：真渲染（仅 Xvfb 档）

## 三机位拍照 + R1~R9（17 项）。只在 `_cam.Can_Capture()==true` 时调用。
func _Run_Render() -> void:
	# ① 逐张抓拍（投影类测量在 _Capture 内部于**抓拍之后**完成）
	for i in range(SHOTS.size()):
		await _Capture(i)
	# ② 逐张 R1~R4（读落盘文件，验证"交付物"本身）
	for i in range(SHOTS.size()):
		_Check_Shot(i)
	# ③ 全局 R5~R9
	_Check_Pairs()
	_Check_Car_Ratios()
	_Check_Pillar()
	_Check_Bookkeeping()


## 单机位：摆位 -> 等 2 帧 -> 抓拍落盘 -> 就地测量（车体投影 / pillar_e 屏幕角点）。
## 返回 false = 抓拍或落盘失败（原因已由组件记在 Get_Last_Error()，本脚本不 push_error）。
func _Capture(idx: int) -> bool:
	var shot: Dictionary = SHOTS[idx]
	var tag: String = _Tag_Of(idx)
	var path: String = "%s/%s" % [OUT_DIR, String(shot["file"])]
	var heading: float = float(shot["heading"])
	var kind: String = String(shot["kind"])

	# 摆位：onboard = 转车（相机跟着车转，这就是"对着哪拍哪"）；external = 另建世界级机位。
	_veh.rotation.y = WorldTransform.YawToGodot(heading)
	await process_frame
	var cam: Node3D = _cam
	if kind == "external":
		_ext_holder = _Make_External_Holder()
		cam = _ext_cam
	if cam == null:
		_shots.append({"ok": false, "path": path, "bytes": -1, "head": "",
			"car_ratio": -1.0, "pillar_front": -1, "pillar_inside": -1,
			"pillar_lo": Vector2.ZERO, "pillar_hi": Vector2.ZERO,
			"cam_world": Vector3.ZERO, "jpg": PackedByteArray()})
		return false
	await process_frame
	await process_frame

	var saved: bool = await cam.Capture_To_File(path)

	# 测量（必须在抓拍之后：SubViewport 断了继承链，Lens.global_transform 只在抓拍内同步）
	var car_ratio: float = _Car_Ratio(cam)
	var pillar: Dictionary = _Pillar_Screen(cam)
	_shots.append({
		"ok": saved,
		"path": path,
		"bytes": _File_Size(path),
		"head": _File_Head(path),
		"car_ratio": car_ratio,
		"pillar_front": int(pillar["front"]),
		"pillar_inside": int(pillar["inside"]),
		"pillar_lo": pillar["lo"],
		"pillar_hi": pillar["hi"],
		"cam_world": cam.global_position,
		"jpg": FileAccess.get_file_as_bytes(path),
	})
	print("[check] capture %s ok=%s heading=%.1f° cam_world=%s" % [
		tag, str(saved), rad_to_deg(heading), str(cam.global_position)])
	return saved


## 外部机位 holder：车头右前上方 3/4 视角，望向车体（不挂车上，否则跟着车动）。
func _Make_External_Holder() -> Node3D:
	var fwd: Vector3 = _veh.global_basis.x.normalized()
	var side: Vector3 = fwd.cross(Vector3.UP).normalized()
	var holder := Node3D.new()
	holder.name = "ExternalCamera"
	_world.add_child(holder)
	holder.global_position = _veh.global_position + fwd * EXT_FWD_M \
		- side * EXT_SIDE_M + Vector3.UP * EXT_UP_M
	holder.look_at(_veh.global_position + Vector3.UP * EXT_LOOK_UP_M, Vector3.UP)
	var packed: PackedScene = load(CAMERA_SCENE) as PackedScene
	if packed != null:
		_ext_cam = packed.instantiate() as Node3D
		if _ext_cam != null:
			holder.add_child(_ext_cam)
	return holder


## 单张 R1~R4：对**落盘文件**断言（不是内存里的 Image）。
func _Check_Shot(idx: int) -> void:
	var data: Dictionary = _shots[idx]
	var labels: Array = _Shot_Labels(idx)
	var path: String = String(data["path"])
	var bytes: int = int(data["bytes"])
	var head: String = String(data["head"])
	var jpg: PackedByteArray = data["jpg"]

	# ---- R1 文件存在 + 大小 + JPEG 魔数
	_check(String(labels[0]),
		bool(data["ok"]) and bytes > MIN_BYTES and head == "ffd8ff",
		"bytes=%d(>%d) head=%s saved=%s" % [bytes, MIN_BYTES, head, str(data["ok"])])

	# ---- R2 SOF0 尺寸 + 解码尺寸
	var sof: Vector2i = _Sof0_Size(jpg)
	var img: Image = _Decode(jpg)
	var img_size: Vector2i = img.get_size() if img != null and not img.is_empty() \
		else Vector2i(-1, -1)
	_check(String(labels[1]), sof == WANT_SIZE and img_size == WANT_SIZE,
		"sof0=%dx%d decoded=%dx%d want=%dx%d" % [
			sof.x, sof.y, img_size.x, img_size.y, WANT_SIZE.x, WANT_SIZE.y])
	if img == null or img.is_empty():
		_check(String(labels[2]), false, "JPEG 解码失败：%s" % path)
		_check(String(labels[3]), false, "JPEG 解码失败：%s" % path)
		return

	# ---- R3 非全黑 / 非全白
	var st: Dictionary = _Gray_Stats(img)
	var mean: float = float(st["mean"])
	var variance: float = float(st["var"])
	var spread: float = float(st["max"]) - float(st["min"])
	var white: float = float(st["white"])
	_check(String(labels[2]),
		variance > VAR_MIN and mean > MEAN_MIN and mean < MEAN_MAX
			and spread > SPREAD_MIN and white < WHITE_MAX,
		"var=%.6f(>%.3f) mean=%.4f(%.2f~%.2f) spread=%.3f(>%.2f) white=%.3f(<%.2f)" % [
			variance, VAR_MIN, mean, MEAN_MIN, MEAN_MAX, spread, SPREAD_MIN,
			white, WHITE_MAX])

	# ---- R4 画面里能读到世界几何（144 格里的亮暗对比格数）
	var grid: Dictionary = _Grid_Stats(img, mean)
	_check(String(labels[3]),
		int(grid["contrast"]) >= GRID_CONTRAST_MIN and int(grid["midband"]) >= GRID_MIDBAND_MIN,
		"contrast=%d/%d(>=%d) midband=%d(>=%d)" % [
			int(grid["contrast"]), GRID_COLS * GRID_ROWS, GRID_CONTRAST_MIN,
			int(grid["midband"]), GRID_MIDBAND_MIN])


## R5：三张两两不同（字节差异 + 灰度差异都要过）——"真的对着拍，不是固定背景"。
func _Check_Pairs() -> void:
	var pairs: Array = [[0, 1], [0, 2], [1, 2]]
	var details: Array[String] = []
	var ok: bool = _shots.size() == SHOTS.size()
	for pair: Array in pairs:
		var a: PackedByteArray = _shots[pair[0]]["jpg"]
		var b: PackedByteArray = _shots[pair[1]]["jpg"]
		var diff: Dictionary = _Pair_Diff(a, b)
		var byte_ratio: float = float(diff["byte_ratio"])
		var mean_abs: float = float(diff["mean_abs"])
		if byte_ratio <= DIFF_BYTE_MIN or mean_abs <= DIFF_MEAN_MIN:
			ok = false
		details.append("%s~%s byte=%.4f(>%.2f) mean_abs=%.5f(>%.2f)" % [
			_Tag_Of(pair[0]), _Tag_Of(pair[1]), byte_ratio, DIFF_BYTE_MIN,
			mean_abs, DIFF_MEAN_MIN])
	_check("R5-shots-differ", ok, "；".join(details))


## R6 / R7：车体投影占屏比例（R6 防"拍到自己的壳"，R7 证明外部机位真的框到了车）。
func _Check_Car_Ratios() -> void:
	var ratio_0: float = float(_shots[0]["car_ratio"])
	var ratio_1: float = float(_shots[1]["car_ratio"])
	var ratio_2: float = float(_shots[2]["car_ratio"])
	_check("R6-nose-not-occluding", ratio_0 < CAR_RATIO_MAX and ratio_1 < CAR_RATIO_MAX,
		"shot_00=%.4f(<%.2f) shot_01=%.4f(<%.2f)" % [
			ratio_0, CAR_RATIO_MAX, ratio_1, CAR_RATIO_MAX])
	_check("R7-external-shows-car",
		ratio_2 > CAR_RATIO_EXT_MIN and ratio_2 < CAR_RATIO_EXT_MAX,
		"shot_02=%.4f(%.2f~%.2f)" % [ratio_2, CAR_RATIO_EXT_MIN, CAR_RATIO_EXT_MAX])


## R8：同一个世界地标（pillar_e）随车头转向离场 —— "对着哪拍哪"的像素级证据。
func _Check_Pillar() -> void:
	var in_a: int = int(_shots[0]["pillar_inside"])
	var in_b: int = int(_shots[1]["pillar_inside"])
	var lo_a: Vector2 = _shots[0]["pillar_lo"]
	var hi_a: Vector2 = _shots[0]["pillar_hi"]
	_check("R8-pillar-follows-heading", in_a >= PILLAR_IN_MIN and in_b <= PILLAR_IN_MAX_B,
		"shot_00 inside=%d/8(>=%d, bbox=%.0f,%.0f-%.0f,%.0f) shot_01 inside=%d/8(<=%d) front_01=%d" % [
			in_a, PILLAR_IN_MIN, lo_a.x, lo_a.y, hi_a.x, hi_a.y,
			in_b, PILLAR_IN_MAX_B, int(_shots[1]["pillar_front"])])


## R9：抓拍簿记（总次数 / 无错误 / 文件确实是本次写的，防拿旧图充数）。
func _Check_Bookkeeping() -> void:
	var nose_count: int = int(_cam.Get_Capture_Count())
	var ext_count: int = int(_ext_cam.Get_Capture_Count()) if _ext_cam != null else 0
	_capture_total = nose_count + ext_count
	var last_error: String = String(_cam.Get_Last_Error())
	if _ext_cam != null and not String(_ext_cam.Get_Last_Error()).is_empty():
		last_error = String(_ext_cam.Get_Last_Error())
	var fresh: bool = true
	var stamps: Array[String] = []
	for i in range(_shots.size()):
		var path: String = String(_shots[i]["path"])
		var stamp: int = FileAccess.get_modified_time(path)
		if stamp < _start_unix_s - FILE_FRESH_SLACK_S:
			fresh = false
		stamps.append("%s@%d" % [_Tag_Of(i), stamp])
	_check("R9-capture-bookkeeping",
		nose_count + ext_count == SHOTS.size() and last_error.is_empty() and fresh,
		"captures nose=%d ext=%d total=%d(==%d) last_error='%s' fresh=%s=%s" % [
			nose_count, ext_count, nose_count + ext_count, SHOTS.size(),
			last_error, str(fresh), str(stamps)])


## headless：R 组整体 SKIP（**不计入 total**，否则断言数会和基线错位）。
func _skip(labels: Array) -> void:
	for label in labels:
		print("[skip] %s（display=%s adapter='%s'：无真渲染器，--headless 是 dummy）" % [
			String(label), DisplayServer.get_name(),
			RenderingServer.get_video_adapter_name()])
		_skipped += 1


## R 组全部 label（17 项），与 _Check_Shot / 全局断言逐字同源，防两处漂移。
func _Render_Labels() -> Array:
	var labels: Array = []
	for i in range(SHOTS.size()):
		labels.append_array(_Shot_Labels(i))
	labels.append_array([
		"R5-shots-differ", "R6-nose-not-occluding", "R7-external-shows-car",
		"R8-pillar-follows-heading", "R9-capture-bookkeeping",
	])
	return labels


## 单张的 4 条 label（R1~R4）。
func _Shot_Labels(idx: int) -> Array:
	var tag: String = _Tag_Of(idx)
	return ["R1-%s-file" % tag, "R2-%s-size" % tag, "R3-%s-not-blank" % tag, "R4-%s-grid" % tag]


func _Tag_Of(idx: int) -> String:
	return "shot-%02d" % int(SHOTS[idx]["id"])


# ================================================================ 测量：车体投影 / 世界地标

## 车体（CollisionShape3D 的 BoxShape3D）8 角投影到屏幕，返回包围框面积占 1280×720 的比例。
## 全部角点在镜头后方 -> 返回 0.0（合法值，不是失败：前视机位实测就是 0）。
## **必须在一次抓拍之后调用**：Lens.global_transform 只在 Capture_Jpeg() 内同步（E26）。
func _Car_Ratio(cam: Node3D) -> float:
	var lens: Camera3D = cam.get_node_or_null("Viewport/Lens") as Camera3D
	var cs: CollisionShape3D = _veh.get_node_or_null("CollisionShape3D") as CollisionShape3D
	if lens == null or cs == null:
		return -1.0
	var box: BoxShape3D = cs.shape as BoxShape3D
	if box == null:
		return -1.0
	var half: Vector3 = box.size * 0.5
	var lo := Vector2(1e9, 1e9)
	var hi := Vector2(-1e9, -1e9)
	var front: int = 0
	for i in range(8):
		var local: Vector3 = cs.position + Vector3(
			half.x if (i & 1) != 0 else -half.x,
			half.y if (i & 2) != 0 else -half.y,
			half.z if (i & 4) != 0 else -half.z)
		var world_pos: Vector3 = _veh.global_transform * local
		if lens.is_position_behind(world_pos):
			continue
		front += 1
		var sp: Vector2 = lens.unproject_position(world_pos)
		lo = Vector2(minf(lo.x, sp.x), minf(lo.y, sp.y))
		hi = Vector2(maxf(hi.x, sp.x), maxf(hi.y, sp.y))
	if front == 0:
		return 0.0
	return maxf(hi.x - lo.x, 0.0) * maxf(hi.y - lo.y, 0.0) \
		/ float(WANT_SIZE.x * WANT_SIZE.y)


## pillar_e 的 8 个世界角点投到屏幕：`{front, inside, lo, hi}`。
## `inside` = 落在 [0,1280]×[0,720] 内的角点数（实测朝东 8、朝南 0）。
## **必须在一次抓拍之后调用**（同上）。
func _Pillar_Screen(cam: Node3D) -> Dictionary:
	var lens: Camera3D = cam.get_node_or_null("Viewport/Lens") as Camera3D
	if lens == null:
		return {"front": -1, "inside": -1, "lo": Vector2.ZERO, "hi": Vector2.ZERO}
	var lo := Vector2(1e9, 1e9)
	var hi := Vector2(-1e9, -1e9)
	var front: int = 0
	var inside: int = 0
	for i in range(8):
		var world_pos: Vector3 = WorldTransform.PosToGodot(PILLAR_ORIGIN + Vector3(
			PILLAR_SIZE.x if (i & 1) != 0 else 0.0,
			PILLAR_SIZE.y if (i & 2) != 0 else 0.0,
			PILLAR_SIZE.z if (i & 4) != 0 else 0.0))
		if lens.is_position_behind(world_pos):
			continue
		front += 1
		var sp: Vector2 = lens.unproject_position(world_pos)
		lo = Vector2(minf(lo.x, sp.x), minf(lo.y, sp.y))
		hi = Vector2(maxf(hi.x, sp.x), maxf(hi.y, sp.y))
		if sp.x >= 0.0 and sp.x <= float(WANT_SIZE.x) \
			and sp.y >= 0.0 and sp.y <= float(WANT_SIZE.y):
			inside += 1
	if front == 0:
		lo = Vector2.ZERO
		hi = Vector2.ZERO
	return {"front": front, "inside": inside, "lo": lo, "hi": hi}


# ================================================================ 测量：图像统计

## 灰度统计（2 像素步长抽样：1280×720 -> 23 万点，够稳且快）：`{mean, var, min, max, white}`。
func _Gray_Stats(img: Image) -> Dictionary:
	var sum_l: float = 0.0
	var sum2: float = 0.0
	var count: int = 0
	var lo: float = 1.0
	var hi: float = 0.0
	var white: int = 0
	for y in range(0, img.get_height(), 2):
		for x in range(0, img.get_width(), 2):
			var l: float = _Lum(img.get_pixel(x, y))
			sum_l += l
			sum2 += l * l
			lo = minf(lo, l)
			hi = maxf(hi, l)
			if l > 0.95:
				white += 1
			count += 1
	if count == 0:
		return {"mean": -1.0, "var": -1.0, "min": -1.0, "max": -1.0, "white": -1.0}
	var mean: float = sum_l / float(count)
	return {
		"mean": mean,
		"var": maxf(sum2 / float(count) - mean * mean, 0.0),
		"min": lo,
		"max": hi,
		"white": float(white) / float(count),
	}


## 网格统计（16×9 = 144 格）：`contrast` = 格内 max-min>0.05 的格数、
## `midband` = |格均值-全图均值|>0.10 的格数（"画面里亮暗格数"的弱断言）。
func _Grid_Stats(img: Image, global_mean: float) -> Dictionary:
	var cell_w: int = maxi(img.get_width() / GRID_COLS, 1)
	var cell_h: int = maxi(img.get_height() / GRID_ROWS, 1)
	var contrast: int = 0
	var midband: int = 0
	for gy in range(GRID_ROWS):
		for gx in range(GRID_COLS):
			var cmin: float = 1.0
			var cmax: float = 0.0
			var csum: float = 0.0
			var cn: int = 0
			for y in range(gy * cell_h, mini((gy + 1) * cell_h, img.get_height()), GRID_STEP):
				for x in range(gx * cell_w, mini((gx + 1) * cell_w, img.get_width()), GRID_STEP):
					var l: float = _Lum(img.get_pixel(x, y))
					cmin = minf(cmin, l)
					cmax = maxf(cmax, l)
					csum += l
					cn += 1
			if cn == 0:
				continue
			if cmax - cmin > 0.05:
				contrast += 1
			var cmean: float = csum / float(cn)
			if absf(cmean - global_mean) > 0.10:
				midband += 1
	return {"contrast": contrast, "midband": midband}


## 两张 JPEG 的差异：`mean_abs` = 8 像素步长的灰度差均值、`byte_ratio` = 逐字节不同占比。
func _Pair_Diff(a: PackedByteArray, b: PackedByteArray) -> Dictionary:
	var img_a: Image = _Decode(a)
	var img_b: Image = _Decode(b)
	var mean_abs: float = -1.0
	if img_a != null and img_b != null and not img_a.is_empty() and not img_b.is_empty():
		var sum_l: float = 0.0
		var n: int = 0
		var h: int = mini(img_a.get_height(), img_b.get_height())
		var w: int = mini(img_a.get_width(), img_b.get_width())
		for y in range(0, h, DIFF_STEP):
			for x in range(0, w, DIFF_STEP):
				sum_l += absf(_Lum(img_a.get_pixel(x, y)) - _Lum(img_b.get_pixel(x, y)))
				n += 1
		mean_abs = sum_l / float(maxi(n, 1))
	var total: int = mini(a.size(), b.size())
	var diff: int = 0
	for i in range(total):
		if a[i] != b[i]:
			diff += 1
	return {
		"mean_abs": mean_abs,
		"byte_ratio": float(diff) / float(maxi(total, 1)),
	}


## 解码 JPEG；失败返回 null（调用方判空 -> FAIL，不崩）。
func _Decode(jpg: PackedByteArray) -> Image:
	if jpg.is_empty():
		return null
	var img := Image.new()
	var err: Error = img.load_jpg_from_buffer(jpg)
	if err != OK:
		return null
	return img


func _Lum(c: Color) -> float:
	return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b


## 解析 JPEG 的 SOF 段拿宽高（C0~C3；C4/C8/CC 是 DHT/JPG/DAC，不是 SOF）。
## 返回 Vector2i(-1,-1) = 没找到（不是合法 JPEG）。
func _Sof0_Size(jpg: PackedByteArray) -> Vector2i:
	var n: int = jpg.size()
	if n < 4 or jpg[0] != 0xFF or jpg[1] != 0xD8:
		return Vector2i(-1, -1)
	var i: int = 2
	while i + 9 < n:
		if jpg[i] != 0xFF:
			i += 1
			continue
		var marker: int = jpg[i + 1]
		if marker == 0xC0 or marker == 0xC1 or marker == 0xC2 or marker == 0xC3:
			var h: int = (jpg[i + 5] << 8) | jpg[i + 6]
			var w: int = (jpg[i + 7] << 8) | jpg[i + 8]
			return Vector2i(w, h)
		if marker == 0xD8 or marker == 0xD9 or marker == 0x01 \
			or (marker >= 0xD0 and marker <= 0xD7):
			i += 2
			continue
		var seg_len: int = (jpg[i + 2] << 8) | jpg[i + 3]
		i += 2 + maxi(seg_len, 2)
	return Vector2i(-1, -1)


# ================================================================ 工具：文件 / 节点

## 递归数物理节点（传感器纯视觉：必须 0）。
func _Count_Physics(node: Node) -> int:
	var n: int = 0
	if node is CollisionShape3D or node is CollisionPolygon3D \
		or node is PhysicsBody3D or node is Area3D:
		n += 1
	for child in node.get_children():
		n += _Count_Physics(child)
	return n


## 建落盘目录（已存在 = 成功）；失败只返回 false、**绝不打印 ERROR**
## （`make_dir_recursive_absolute` 失败会走 ERR_FAIL_V_MSG 打 ERROR，故不用）。
func _Ensure_Out_Dir() -> bool:
	if OUT_DIR.is_empty() or not OUT_DIR.is_absolute_path():
		return false
	if DirAccess.dir_exists_absolute(OUT_DIR):
		return true
	DirAccess.make_dir_absolute(OUT_DIR)
	if not DirAccess.dir_exists_absolute(OUT_DIR):
		# 父目录也可能缺失：补建一层父目录后再试（失败仍只返回 false）
		var parent: String = OUT_DIR.get_base_dir()
		if not parent.is_empty() and not DirAccess.dir_exists_absolute(parent):
			DirAccess.make_dir_absolute(parent)
		DirAccess.make_dir_absolute(OUT_DIR)
	return DirAccess.dir_exists_absolute(OUT_DIR)


func _Read_Text(path: String) -> String:
	if not FileAccess.file_exists(path):
		return ""
	var handle: FileAccess = FileAccess.open(path, FileAccess.READ)
	if handle == null:
		return ""
	var text: String = handle.get_as_text()
	handle.close()
	return text


## 只剥**整行**注释（# 或 ## 开头，允许前导空白）：行尾代码里的 # 不剥（与 check_quadrotor 同口径）。
func _Strip_Line_Comments(src: String) -> String:
	var out: PackedStringArray = PackedStringArray()
	for line in src.split("\n"):
		if line.strip_edges().begins_with("#"):
			continue
		out.append(line)
	return "\n".join(out)


func _File_Size(path: String) -> int:
	if not FileAccess.file_exists(path):
		return -1
	var handle: FileAccess = FileAccess.open(path, FileAccess.READ)
	if handle == null:
		return -2
	var size_b: int = handle.get_length()
	handle.close()
	return size_b


func _File_Head(path: String) -> String:
	var bytes: PackedByteArray = FileAccess.get_file_as_bytes(path)
	if bytes.size() < 3:
		return ""
	return bytes.slice(0, 3).hex_encode()


# ================================================================ 释放 / 断言 / 收尾

## 释放顺序：外部相机 -> 车 -> 世界，每步 await 两帧
## （漏了会报 `ERROR: resources still in use at exit`）。
func _Teardown() -> void:
	if _ext_cam != null and is_instance_valid(_ext_cam):
		_ext_cam.queue_free()
		_ext_cam = null
		await process_frame
		await process_frame
	if _ext_holder != null and is_instance_valid(_ext_holder):
		_ext_holder.queue_free()
		_ext_holder = null
		await process_frame
		await process_frame
	if _veh != null and is_instance_valid(_veh):
		_veh.queue_free()
		_veh = null
		await process_frame
		await process_frame
	if _world != null and is_instance_valid(_world):
		_world.queue_free()
		_world = null
		await process_frame
		await process_frame


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


## 前置条件失败（世界/车载不进来）：记一条失败后直接收尾，不抛异常、不模拟"全绿"。
func _abort(reason: String) -> void:
	_failed += 1
	print("[check] ABORT %s" % reason)
	_finish()


func _finish() -> void:
	print("[check] env display=%s adapter='%s' captures=%d" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name(), _capture_total])
	print("[check] out_dir=%s" % OUT_DIR)
	if _skipped > 0:
		print("[check] skipped=%d（R 组：无真渲染器，不计入 total）" % _skipped)
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
