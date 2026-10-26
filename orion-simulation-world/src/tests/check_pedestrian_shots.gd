# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# task_35 Step 5 第 6 项 —— **走 / 转 / 停** 三帧真渲染自检（A 组结构 4 + R 组渲染 10 = 14 项）。
#
# 被拍的人 = `World.PEDESTRIANS[0]` = `pedestrian_01`（世界 (44,96)、yaw 0，`Pedestrian.tscn`）；
# 机位沿用 `check_person_walk_shots.gd` 已经实战过的几何（(48,101) 抬 1.5 m，瞄 (44.9, 96.5, 1.0)）。
# 驱动**只有三发** `Pedestrian.SetCommand`（这也是 R10 lint 数出来的）：
#   ① {"v":1.0, "dir":0.0}      朝东走 40 帧 → 拍 ped_walk.jpg
#   ② {"v":1.0, "dir":π/2}      转 90°（180 °/s ⇒ 30 帧）再走 15 帧 → 拍 ped_turn.jpg（此帧 yaw ≈ 90°）
#   ③ {"v":0.0}                 再走 20 帧后停 12 帧 → 拍 ped_stop.jpg → 再等 15 帧冻结取证
#
# 跑法 ①（**本 step 的真验收**，Xvfb 真渲染）：
#   cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion
#   DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot \
#     --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
#     --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_pedestrian_shots.gd
#   期望：total=14 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR + 三张 >10 KB 的 1280×720 JPEG。
#   **绝不加 --headless**（dummy 渲染器下 `frame_post_draw` 不触发、`get_image()` 返回 null，
#   COMPONENTS.md §9.1 E26）；**必须 --audio-driver Dummy**（本机无声卡，ALSA 打不开 = 一行 ERROR）。
#
# 跑法 ②（headless 冒烟，只跑 A 组 4 项，**不得**出现假绿）：
#   timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_pedestrian_shots.gd
#   期望：`[skip] R1..R10` 恰 10 条 + skipped=10 + total=4 failed=0 + SELF-CHECK OK。
#   R 组**不计入 total**（否则 headless 基线会和 Xvfb 档错位）。
#
# 关键取舍（每条都有实测依据）：
#   ① `physics_frame` 由本文件**自己计数**（`_counter`）：`Capture_To_File()` 的 warmup 会偷 1~2 帧，
#      用"我 await 了几次"数不准 → 全部用**绝对目标帧号** + FRAME_TOL=2 兜底。
#   ② 投影测量必须放在 `Capture_To_File()` **之后**（E26：`Lens.global_transform` 只在抓拍内部同步）。
#   ③ 机位只用 `look_at_from_position()` 摆**一次**：这样源码里既没有 `global_position =`
#      也没有 `position =`（R10 的白盒 lint 要数它们 == 0），三次抓拍的机位也不可能漂。
#   ④ "转"和"停"都各有一把**物理尺子**，不只看像素：
#      R7 用逐帧 `GetPose().yaw` 拟合角速度（60 Hz 采样，内部点拟合，实测 180.0 °/s）；
#      R8 用冻结窗口内的位姿位移 + `AnimationPlayer.speed_scale == 0`（Step 1 的"定格"语义）。
#   ⑤ 本文件**不 push_error**（失败一律 `_check(..., false, ...)` + `quit(1)`），
#      退出前 `queue_free()` + 逐级 await 两帧，避免 `resources still in use at exit`。
#
# 注意（GDScript 4.7）：`load()` / `instantiate()` / `call()` 的返回值一律显式标注类型。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldGd := preload("res://src/world/World.gd")

# ---------------------------------------------------------------- 场景（唯一来源）
const WORLD_SCENE := "res://src/scenes/World.tscn"
const CAMERA_SCENE := "res://src/scenes/sensors/CameraSensor.tscn"
const PEDESTRIAN_SCENE := "res://src/scenes/props/Pedestrian.tscn"
const PEDESTRIAN_SCRIPT := "res://src/chassis/Pedestrian.gd"
const SELF_PATH := "res://src/tests/check_pedestrian_shots.gd"

# ---------------------------------------------------------------- 出图（绝对路径）
const OUT_DIR := "/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/task35/shots"
const SHOT_NAMES := ["ped_walk.jpg", "ped_turn.jpg", "ped_stop.jpg"]
const WANT_SIZE := Vector2i(1280, 720)
const WANT_FOV := 70.0
const WANT_QUALITY := 0.8
const MIN_BYTES := 10 * 1024
const VAR_MIN := 0.002
const MEAN_MIN := 0.05
const MEAN_MAX := 0.95
const SPREAD_MIN := 0.15
const WHITE_MAX := 0.50

# ---------------------------------------------------------------- 固定机位（世界 Z-up 米）
## 机位是本 step 唯一调整过的"几何"（plan.md 允许：只动 CAM_EYE_W / CAM_AIM_W）。
## 为什么不能沿用 `check_person_walk_shots.gd` 的 (48,101)：那条机位的视线方向与走位几乎重合
## （人朝东走、相机在东偏南 51°），位移主要落在**深度**上 —— 本轮实测三帧投影框水平中心只差
## 4.1 / 13.6 / 17.7 px，够不到判据的 20 px。把机位挪到"路径的侧面"（人从 (44,96) 朝东走 0.667 m、
## 再朝 +y 转 90° 走 0.57 m），同样的 40/45/20 帧就能投出 74 / 50 / 124 px（几何推算）：
##   相机 (48.37, 96.79) 抬 1.50 m，距人 3.5 m；人高投影 ≈ 249 px（h_frac 0.35，窗口 [0.15, 0.45]）。
## 顺带的好处：第 1 帧是**正脸**（相机朝向与人的朝向夹角 12°），转完变成**侧身**（86~92°），
## "走 → 转 → 停"的姿态差在画面上一眼能看出来。
const CAM_EYE_W := Vector3(48.37, 96.79, 0.0)
const CAM_EYE_H_M := 1.50
## 瞄点沿用 plan.md 给的值（(44.9, 96.5, 1.00)）：三帧（走 / 转 / 停）都在框内。
const CAM_AIM_W := Vector3(44.9, 96.5, 1.00)

# ---------------------------------------------------------------- 帧计划（--fixed-fps 60）
## 指令速度（m/s）：世界 clamp 0~1.26，资产步速也是 1.26；1.0 便于"40 帧 = 0.667 m"心算。
const CMD_V := 1.0
const TURN_DIR := PI * 0.5
const WALK_FRAMES := 40          # 走：0.667 s → 0.667 m
const TURN_FRAMES := 45          # 转：30 帧转完 90° + 15 帧直行
const STOP_WALK_FRAMES := 20     # 停车前再多走 0.333 m（保证三帧位置不同）
const STOP_SETTLE_FRAMES := 12   # 停住后再等 0.2 s 才拍
const FREEZE_FRAMES := 15        # 拍完再等 15 帧，证明"真的不动"
## `Capture_To_File` 内部 warmup 会偷 1~2 帧 → 用 ±2。
const FRAME_TOL := 2

# ---------------------------------------------------------------- 判据
## 框**水平**中心两两相距的下限（像素）。预测 60~150 px，留余量。
const BOX_DX_MIN_PX := 20.0
## 两两字节差异占比 / 灰度平均绝对差。
const DIFF_BYTE_MIN := 0.10
const DIFF_MEAN_MIN := 0.02
const DIFF_STEP := 4
## 框内差分的边界余量（像素）：把投影框的越界/抖动兜进去。
const BOX_PAD_PX := 4.0
## 转向段：实测角速度 180 ±10%；此帧 yaw 与 90° 的差 <= 3°。
const TURN_RATE_MIN_DPS := 162.0
const TURN_RATE_MAX_DPS := 198.0
const TURN_YAW_TOL_DEG := 3.0
## 冻结段：位姿位移 < 1 mm。
const FREEZE_MOVE_MAX_M := 1.0e-3
## 位姿容差：1 mm / 1 mrad（A4）。
const POSE_TOL_M := 1.0e-3
const YAW_TOL_RAD := 1.0e-3
## 落盘时间容差（秒）：防拿旧图充数。
const FILE_FRESH_SLACK_S := 2
## 框高占画面高的比例窗口：只做"人还在画面里、大小合理"的弱断言。
## 本轮机位（离人 3.5 m）实测 0.375 / 0.525 / 0.530 —— 比 walking_person 时期（6.4 m，0.241~0.286）
## 大得多，所以上界取 0.65；"转身让 AABB 变高"（0.375 → 0.525）是模型绑定姿态的必然现象，不是错。
const H_FRAC_MIN := 0.15
const H_FRAC_MAX := 0.65

# ---------------------------------------------------------------- 状态
var _total: int = 0
var _failed: int = 0
var _skipped: int = 0

var _world: Node3D = null
var _ped: Node3D = null
var _cam: Node3D = null
var _ws: Node = null
var _start_unix_s: int = 0

## 物理帧计数器（每收到一次 `SceneTree.physics_frame` +1）。
var _counter: int = 0
var _c0: int = 0
var _c1: int = 0
var _c2: int = 0
var _c3: int = 0
var _c4: int = 0
## 转向段逐帧 yaw 采样：[{f: 帧号, yaw: 度}]
var _turn_samples: Array[Dictionary] = []
## 三帧抓拍记录（投影框必须在抓拍之后才测，E26）。
var _shots: Array[Dictionary] = []
var _turned_pose: Dictionary = {}
var _stop_pose: Dictionary = {}
var _freeze_pose: Dictionary = {}


func _initialize() -> void:
	_run()


func _run() -> void:
	_start_unix_s = int(Time.get_unix_time_from_system())
	physics_frame.connect(_On_Physics_Frame)
	var ok: bool = await _Build_World()
	if ok:
		ok = _Check_Anim()
	if ok:
		ok = await _Build_Camera()
	if ok:
		ok = _Check_Spawn_Pose()
	if ok and _cam != null and _cam.Can_Capture():
		await _Run_Render()
	else:
		_skip(_Render_Labels())
	await _Teardown()
	_Finish()


func _On_Physics_Frame() -> void:
	_counter += 1


# ================================================================ A 组：结构与出生位姿（4 项）

## A1 + A2：载真 `World.tscn`（入树前关 WS autolisten，避免抢 9999），
## 断言 `Pedestrians` 里恰 `World.PEDESTRIANS.size()` 个人、第 0 个是表里的名字。
func _Build_World() -> bool:
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	var world: Node3D = packed.instantiate() as Node3D if packed != null else null
	if world == null:
		_Check_Fail_All(["A1-world-loads", "A2-pedestrian-instance", "A3-anim-playing",
			"A4-spawn-pose"])
		return false
	# 全文件**唯一**一处 WS 相关字样就在这里，而且是"关掉监听"（R10 的 lint 会数它 == 1）：
	# 不关的话本自检会去抢 9999 端口，把"0 ERROR"验收搞红（plan.md §6 R3）。
	_ws = world.get_node_or_null("Systems/WebSocketManager")
	if _ws != null:
		_ws.set("autolisten", false)
	root.add_child(world)
	_world = world
	await process_frame
	await physics_frame
	await process_frame

	var root_node: Node3D = world.call("GetPedestriansRoot") as Node3D
	var kids: Array[Node] = root_node.get_children() if root_node != null else []
	_check("A1-world-loads", root_node != null,
		"nodes=%d pedestrians_root=%s ws_autolisten=%s" % [_Count_Nodes(world),
			str(root_node != null), str(_ws.get("autolisten")) if _ws != null else "<none>"])

	var kid: Node3D = kids[0] as Node3D if kids.size() > 0 else null
	var spec_name: String = String(WorldGd.PEDESTRIANS[0]["name"])
	var kid_name: String = String(kid.name) if kid != null else "<none>"
	var kid_scene: String = kid.scene_file_path if kid != null else "<none>"
	var script_path: String = ""
	if kid != null and kid.get_script() != null:
		script_path = String(kid.get_script().resource_path)
	_ped = kid
	_check("A2-pedestrian-instance",
		kids.size() == WorldGd.PEDESTRIANS.size() and kid != null and kid_name == spec_name
			and kid_scene == PEDESTRIAN_SCENE and script_path == PEDESTRIAN_SCRIPT,
		"count=%d(%d) name=%s(%s) scene=%s script=%s" % [kids.size(),
			WorldGd.PEDESTRIANS.size(), kid_name, spec_name, kid_scene, script_path])
	if _ped == null:
		_Check_Fail_All(["A3-anim-playing", "A4-spawn-pose"])
		return false
	return true


## A3：**还没被驱动时**动画在播且步频倍率 == 0.0（Step 1 的 "v=0 定格" 语义）。
func _Check_Anim() -> bool:
	var ap: AnimationPlayer = _Find_Anim_Player(_ped)
	var clip: String = ""
	var speed_scale: float = -1.0
	var clips: int = -1
	var playing: bool = false
	if ap != null:
		clip = String(ap.current_animation)
		speed_scale = ap.speed_scale
		clips = ap.get_animation_list().size()
		playing = ap.is_playing()
	var idle_speed: float = float(_ped.call("Get_Speed"))
	var ok: bool = ap != null and clips >= 1 and playing and not clip.is_empty() \
		and absf(speed_scale) <= 1.0e-6 and absf(idle_speed) <= 1.0e-6
	_check("A3-anim-playing", ok,
		"clip=%s speed_scale=%.3f(want 0.000) clips=%d playing=%s v=%.3f(want 0 = 未驱动时站着)"
		% [clip, speed_scale, clips, str(playing), idle_speed])
	return ap != null and clips >= 1


## 相机 = `CameraSensor.tscn` 实例**直接挂 World 下**（不挂车、不建 UGV）。
## `Can_Capture()` 是 R 组的**唯一**闸门（在碰渲染器之前分流，§6 R4）。
func _Build_Camera() -> bool:
	var packed: PackedScene = load(CAMERA_SCENE) as PackedScene
	var node: Node = packed.instantiate() if packed != null else null
	_cam = node as Node3D
	if _cam == null:
		return false
	_world.add_child(_cam)
	_cam.Configure({"width": WANT_SIZE.x, "height": WANT_SIZE.y,
		"fov_deg": WANT_FOV, "quality": WANT_QUALITY})
	await process_frame
	await process_frame
	# 机位只摆**一次**：look_at_from_position = 同时写位置与朝向（源码里没有 "position ="）。
	var eye: Vector3 = WorldTransform.PosToGodot(CAM_EYE_W) + Vector3(0.0, CAM_EYE_H_M, 0.0)
	var aim: Vector3 = WorldTransform.PosToGodot(CAM_AIM_W)
	_cam.look_at_from_position(eye, aim, Vector3.UP)
	await process_frame
	return true


## A4：出生位姿 == `World.PEDESTRIANS[0]` 的 pos / yaw（≤1 mm / 1 mrad）。
## 本条**不写任何位姿**（R10 的 lint 会数"直接写行人节点位姿"的命中数 == 0）。
func _Check_Spawn_Pose() -> bool:
	var pose: Dictionary = _ped.call("GetPose")
	var want_pos: Vector3 = WorldGd.PEDESTRIANS[0]["pos"]
	var want_yaw: float = deg_to_rad(float(WorldGd.PEDESTRIANS[0]["yaw_deg"]))
	var got := Vector3(float(pose.get("x", -999.0)), float(pose.get("y", -999.0)),
		float(pose.get("z", -999.0)))
	var got_yaw: float = float(pose.get("yaw", -999.0))
	var d_pos: float = (got - want_pos).length()
	var d_yaw: float = absf(wrapf(got_yaw - want_yaw, -PI, PI))
	var ok: bool = d_pos <= POSE_TOL_M and d_yaw <= YAW_TOL_RAD
	_check("A4-spawn-pose", ok,
		"pos=(%.4f, %.4f) Δ=%.6f m(<=%.4f) yaw=%.4f°(want %.1f°) Δ=%.6f rad(<=%.4f)"
		% [got.x, got.y, d_pos, POSE_TOL_M, rad_to_deg(got_yaw),
			float(WorldGd.PEDESTRIANS[0]["yaw_deg"]), d_yaw, YAW_TOL_RAD])
	return ok


# ================================================================ R 组：三帧真渲染（10 项）

## 帧计划（**自己数帧**，别数 await）：
##   ① SetCommand(v, dir=0) → 等到 c0+40 → 拍"走"
##   ② SetCommand(v, dir=π/2) → 逐帧采 yaw 直到 c1+45 → 拍"转"
##   ③ 等到 c2+20 → SetCommand(v=0) → 等到 c3+12 → 拍"停" → 再等 15 帧做冻结取证
func _Run_Render() -> void:
	_ensure_out_dir()
	# ---- ① 走
	_ped.call("SetCommand", {"v": CMD_V, "dir": 0.0})
	await physics_frame
	_c0 = _counter
	await _Wait_Until(_c0 + WALK_FRAMES)
	var walk_frames: int = _counter - _c0
	await _Capture(0)

	# ---- ② 转（逐帧采 yaw，用于 R7 的角速度拟合）
	_c1 = _counter
	_ped.call("SetCommand", {"v": CMD_V, "dir": TURN_DIR})
	_turn_samples = []
	await _Wait_Sampling(_c1 + TURN_FRAMES)
	var turn_frames: int = _counter - _c1
	await _Capture(1)
	_turned_pose = _ped.call("GetPose")

	# ---- ③ 停
	_c2 = _counter
	await _Wait_Until(_c2 + STOP_WALK_FRAMES)
	var stop_walk_frames: int = _counter - _c2
	_ped.call("SetCommand", {"v": 0.0})
	_c3 = _counter
	await _Wait_Until(_c3 + STOP_SETTLE_FRAMES)
	await _Capture(2)
	_stop_pose = _ped.call("GetPose")
	_c4 = _counter
	await _Wait_Until(_c4 + FREEZE_FRAMES)
	var freeze_frames: int = _counter - _c4
	_freeze_pose = _ped.call("GetPose")

	var ap: AnimationPlayer = _Find_Anim_Player(_ped)
	var speed_scale: float = ap.speed_scale if ap != null else -1.0

	# ---- R1/R2/R3 逐张：文件存在 / 体积 / JPEG 魔数 / SOF0 尺寸 / 解码尺寸
	for i in range(SHOT_NAMES.size()):
		var d: Dictionary = _shots[i]
		var bytes: int = int(d["bytes"])
		var head: String = String(d["head"])
		var sof: Vector2i = _Sof_Size(d["jpg"])
		var img: Image = _decode(d["jpg"])
		var dec: Vector2i = img.get_size() if img != null and not img.is_empty() else Vector2i(-1, -1)
		var ok: bool = bool(d["ok"]) and bytes > MIN_BYTES and head == "ffd8ff" \
			and sof == WANT_SIZE and dec == WANT_SIZE
		_check("R%d-t%d-file" % [i + 1, i], ok,
			"%s bytes=%d(>%d) head=%s sof0=%s decoded=%s want=%s saved=%s"
			% [SHOT_NAMES[i], bytes, MIN_BYTES, head, str(sof), str(dec), str(WANT_SIZE),
				str(d["ok"])])

	# ---- R4 三张都非全黑（灰度方差 / 均值窗口 / 极差 / 高光占比）
	var blank_ok: bool = true
	var blank_detail: Array[String] = []
	for i in range(SHOT_NAMES.size()):
		var img: Image = _decode(_shots[i]["jpg"])
		if img == null or img.is_empty():
			blank_ok = false
			blank_detail.append("t%d:decode-fail" % i)
			continue
		var st: Dictionary = _Gray_Stats(img)
		var spread: float = float(st["max"]) - float(st["min"])
		var ok: bool = float(st["var"]) > VAR_MIN and float(st["mean"]) > MEAN_MIN \
			and float(st["mean"]) < MEAN_MAX and spread > SPREAD_MIN \
			and float(st["white"]) < WHITE_MAX
		if not ok:
			blank_ok = false
		blank_detail.append("t%d:var=%.4f mean=%.3f spread=%.2f white=%.4f"
			% [i, float(st["var"]), float(st["mean"]), spread, float(st["white"])])
	_check("R4-not-blank", blank_ok, "；".join(blank_detail))

	# ---- R5 三张两两不同（逐字节差异占比 + 灰度平均绝对差）
	#      灰度差必须量在**两框并集**里：人物只占画面 2~4%，量全图会被背景稀释成 0.002 级
	#      （本轮第一版就是这样，实测 0.0025 —— 那是"背景没变"，不是"人没变"）。
	var differ_ok: bool = true
	var differ_detail: Array[String] = []
	for i in range(SHOT_NAMES.size()):
		for j in range(i + 1, SHOT_NAMES.size()):
			var byte_ratio: float = _Byte_Diff_Ratio(_shots[i]["jpg"], _shots[j]["jpg"])
			var img_a: Image = _decode(_shots[i]["jpg"])
			var img_b: Image = _decode(_shots[j]["jpg"])
			var region: Rect2 = (_shots[i]["box"] as Rect2).merge(_shots[j]["box"] as Rect2)
			var gray: float = _Gray_Diff_In(img_a, img_b, region)
			var ok: bool = byte_ratio > DIFF_BYTE_MIN and gray > DIFF_MEAN_MIN
			if not ok:
				differ_ok = false
			differ_detail.append("t%d→t%d bytes=%.3f(>%.2f) 框内gray=%.4f(>%.2f) 区域 %dx%d"
				% [i, j, byte_ratio, DIFF_BYTE_MIN, gray, DIFF_MEAN_MIN,
					int(region.size.x), int(region.size.y)])
	_check("R5-frames-differ", differ_ok, "；".join(differ_detail))

	# ---- R6 人物投影框在动（水平中心两两相距 >= 20 px）+ 三帧的人都在画面内、大小合理
	var moved_ok: bool = true
	var moved_detail: Array[String] = []
	for i in range(SHOT_NAMES.size()):
		var box: Rect2 = _shots[i]["box"]
		var h_frac: float = box.size.y / float(WANT_SIZE.y)
		if int(_shots[i]["front"]) != 8 or int(_shots[i]["inside"]) != 8 \
				or h_frac < H_FRAC_MIN or h_frac > H_FRAC_MAX:
			moved_ok = false
			moved_detail.append("t%d:front=%d/8 inside=%d/8 h_frac=%.3f FAIL"
				% [i, int(_shots[i]["front"]), int(_shots[i]["inside"]), h_frac])
	for i in range(SHOT_NAMES.size()):
		for j in range(i + 1, SHOT_NAMES.size()):
			var a: Rect2 = _shots[i]["box"]
			var b: Rect2 = _shots[j]["box"]
			var dx: float = absf(b.get_center().x - a.get_center().x)
			if dx < BOX_DX_MIN_PX:
				moved_ok = false
			moved_detail.append("t%d→t%d Δx=%.1f px(>=%.0f) 中心 %.1f→%.1f"
				% [i, j, dx, BOX_DX_MIN_PX, a.get_center().x, b.get_center().x])
	_check("R6-box-moved", moved_ok, "；".join(moved_detail))

	# ---- R7 "转"是真转：此帧 yaw ≈ 90°±3°，且**转向段**实测角速度 ≈ 180 °/s ±10%
	#         时间基 = 物理帧号/60（**不用**墙钟：--fixed-fps 下墙钟与物理帧无关）。
	var turn_yaw: float = float(_turned_pose.get("yaw", -999.0))
	var yaw_err: float = absf(wrapf(turn_yaw - TURN_DIR, -PI, PI))
	var rate_dps: float = _Fit_Turn_Rate(_turn_samples)
	var rate_ok: bool = is_finite(rate_dps) \
		and rate_dps >= TURN_RATE_MIN_DPS and rate_dps <= TURN_RATE_MAX_DPS
	var yaw_ok: bool = rad_to_deg(yaw_err) <= TURN_YAW_TOL_DEG
	_check("R7-turn-real", rate_ok and yaw_ok and absi(turn_frames - TURN_FRAMES) <= FRAME_TOL,
		"yaw=%.2f°(want %.1f ±%.1f) rate=%.1f °/s([%.0f, %.0f]) 采样 %d 点 转段 %d 帧(==%d±%d)"
		% [rad_to_deg(turn_yaw), rad_to_deg(TURN_DIR), TURN_YAW_TOL_DEG, rate_dps,
			TURN_RATE_MIN_DPS, TURN_RATE_MAX_DPS, _turn_samples.size(), turn_frames,
			TURN_FRAMES, FRAME_TOL])

	# ---- R8 "停"是真停：冻结 15 帧内 |Δpose| < 1 mm、speed_scale == 0，且画面位置与前两帧不同
	var stop_p := Vector3(float(_stop_pose.get("x", 0.0)), float(_stop_pose.get("y", 0.0)), 0.0)
	var freeze_p := Vector3(float(_freeze_pose.get("x", 0.0)), float(_freeze_pose.get("y", 0.0)), 0.0)
	var freeze_d: float = (freeze_p - stop_p).length()
	var stop_yaw_d: float = absf(wrapf(float(_freeze_pose.get("yaw", 0.0))
		- float(_stop_pose.get("yaw", 0.0)), -PI, PI))
	# "停 = 画面冻住 / 只是不再推进"这种假绿要否掉：停帧的框必须与前两帧都不同。
	var stop_box: Rect2 = _shots[2]["box"]
	var stop_dx_walk: float = absf(stop_box.get_center().x - (_shots[0]["box"] as Rect2).get_center().x)
	var stop_dx_turn: float = absf(stop_box.get_center().x - (_shots[1]["box"] as Rect2).get_center().x)
	var stop_ok: bool = freeze_d < FREEZE_MOVE_MAX_M and stop_yaw_d < YAW_TOL_RAD \
		and absf(speed_scale) <= 1.0e-6 \
		and stop_dx_walk >= BOX_DX_MIN_PX and stop_dx_turn >= BOX_DX_MIN_PX \
		and absi(freeze_frames - FREEZE_FRAMES) <= FRAME_TOL
	_check("R8-stop-real", stop_ok,
		"freeze=%d 帧 Δpose=%.6f m(<%.4f) Δyaw=%.6f speed_scale=%.3f(want 0.000) "
		% [freeze_frames, freeze_d, FREEZE_MOVE_MAX_M, stop_yaw_d, speed_scale]
		+ "停帧 vs 走 %.1f px / vs 转 %.1f px(>=%.0f)"
		% [stop_dx_walk, stop_dx_turn, BOX_DX_MIN_PX])

	# ---- R9 抓拍簿记：次数 == 3、无错误、三张 mtime 不早于本次启动
	var count: int = int(_cam.call("Get_Capture_Count"))
	var last_err: String = String(_cam.call("Get_Last_Error"))
	var fresh_ok: bool = true
	var mtimes: Array[String] = []
	for i in range(SHOT_NAMES.size()):
		var path: String = "%s/%s" % [OUT_DIR, SHOT_NAMES[i]]
		var mt: int = int(FileAccess.get_modified_time(path))
		if mt + FILE_FRESH_SLACK_S < _start_unix_s:
			fresh_ok = false
		mtimes.append("%d" % mt)
	_check("R9-capture-book", count == SHOT_NAMES.size() and last_err.is_empty() and fresh_ok,
		"captures=%d(==%d) last_error='%s' mtime=[%s] start=%d"
		% [count, SHOT_NAMES.size(), last_err, ", ".join(mtimes), _start_unix_s])

	# ---- R10 白盒 lint：本文件**不许**连网 / 不许 on_tick / 不许直接写行人位姿 / SetCommand 恰 3 发
	#      两处"自己数自己"的坑（本轮实测踩过，别退回去）：
	#        ① 针串必须拼出来（"Web" + "Socket"），整串字面量出现会命中自己；
	#        ② 诊断文案里也**不能**写完整针串 —— 上一版把日志里的 "on_tick=%d" 也数进了命中数。
	#      注释整行剥掉后再数（否则本文件头注释里全是禁用词）。
	var src: String = _Read_Code_Only(SELF_PATH)
	var hits_ws: int = src.count("Web" + "Socket") if not src.is_empty() else -1
	var hits_tick: int = src.count("on_" + "tick") if not src.is_empty() else -1
	var hits_gpos: int = src.count("global_" + "position" + " =") if not src.is_empty() else -1
	var hits_pos: int = src.count("position" + " =") if not src.is_empty() else -1
	var hits_cmd: int = src.count("call(\"SetCom" + "mand\"") if not src.is_empty() else -1
	var guard_ok: bool = src.count("autolisten\", false") == 1 if not src.is_empty() else false
	# WS 字样**允许且仅允许 1 处**：`Systems/WebSocketManager` 的 autolisten=false 守门
	# （拿掉它就会去抢 9999，见 §6 R3）。其余针串必须为 0。
	_check("R10-source-lint",
		hits_ws == 1 and guard_ok and hits_tick == 0 and hits_gpos == 0 and hits_pos == 0
			and hits_cmd == 3,
		"code=%d B ws=%d(==1 且是 autolisten=false 守门:%s) tick=%d(==0) "
		% [src.length(), hits_ws, str(guard_ok), hits_tick]
		+ "写位姿=%d+%d(==0) SetCommand=%d(==3)" % [hits_gpos, hits_pos, hits_cmd])

	# 逐帧帧数（人工核对帧计划；不算断言，只打日志）。
	print("[check] frames walk=%d turn=%d stop_walk=%d stop_settle=%d freeze=%d（计划 %d/%d/%d/%d/%d ±%d）"
		% [walk_frames, turn_frames, stop_walk_frames, STOP_SETTLE_FRAMES, freeze_frames,
			WALK_FRAMES, TURN_FRAMES, STOP_WALK_FRAMES, STOP_SETTLE_FRAMES, FREEZE_FRAMES,
			FRAME_TOL])


## 抓一张 + 记录。**投影必须在抓拍之后**（E26）。
func _Capture(idx: int) -> void:
	var path: String = "%s/%s" % [OUT_DIR, SHOT_NAMES[idx]]
	var saved: bool = bool(await _cam.Capture_To_File(path))
	var screen: Dictionary = _Project_Ped()
	_shots.append({
		"ok": saved, "path": path, "bytes": _File_Size(path), "head": _File_Head(path),
		"jpg": FileAccess.get_file_as_bytes(path),
		"box": screen["box"], "inside": int(screen["inside"]), "front": int(screen["front"]),
	})
	var box: Rect2 = _shots[idx]["box"]
	print("[check] shot t%d %s frames=%d box=P(%.1f, %.1f) S(%.1f, %.1f) center=(%.1f, %.1f) "
		% [idx, SHOT_NAMES[idx], _counter, box.position.x, box.position.y, box.size.x,
			box.size.y, box.get_center().x, box.get_center().y]
		+ "inside=%d/8 front=%d/8 bytes=%d"
		% [int(_shots[idx]["inside"]), int(_shots[idx]["front"]), int(_shots[idx]["bytes"])])


## 一直 `await physics_frame` 直到**自己数到的**绝对帧号到 `target`。
func _Wait_Until(target: int) -> void:
	var guard: int = 0
	while _counter < target and guard < 2000:
		await physics_frame
		guard += 1


## 同 `_Wait_Until`，但每帧顺手采一次 `GetPose().yaw`（R7 的角速度拟合用 60 Hz 采样）。
func _Wait_Sampling(target: int) -> void:
	var guard: int = 0
	while _counter < target and guard < 2000:
		await physics_frame
		guard += 1
		var pose: Dictionary = _ped.call("GetPose")
		_turn_samples.append({"f": _counter, "yaw": rad_to_deg(float(pose.get("yaw", 0.0)))})


## 转向段角速度（度/秒）：只用**严格内部点**（0.5° < yaw < 89.5°）做最小二乘斜率。
## 为什么剔掉两端：60 Hz 采样下每帧 3°，收敛后那帧的 yaw 被 clamp 在 90°，
## 而它比"真正到 90°"的时刻晚了不到一帧 → 算进斜率会把 180 压低（e2e_drive.py 同款处理）。
func _Fit_Turn_Rate(samples: Array[Dictionary]) -> float:
	var ts: Array[float] = []
	var ys: Array[float] = []
	var want: float = rad_to_deg(TURN_DIR)
	for s: Dictionary in samples:
		var yaw: float = float(s["yaw"])
		if yaw <= 0.5 or yaw >= want - 0.5:
			continue
		ts.append(float(s["f"]) / 60.0)
		ys.append(yaw)
	if ts.size() < 2:
		return NAN
	var n: int = ts.size()
	var mean_t: float = 0.0
	var mean_y: float = 0.0
	for i in range(n):
		mean_t += ts[i]
		mean_y += ys[i]
	mean_t /= float(n)
	mean_y /= float(n)
	var sxx: float = 0.0
	var sxy: float = 0.0
	for i in range(n):
		sxx += (ts[i] - mean_t) * (ts[i] - mean_t)
		sxy += (ts[i] - mean_t) * (ys[i] - mean_y)
	if sxx <= 1.0e-12:
		return NAN
	return sxy / sxx


## 被拍的人（pedestrian_01）的世界 AABB 8 角点投到屏幕：包围框 + 落在画面内的角点数。
func _Project_Ped() -> Dictionary:
	var empty: Dictionary = {"box": Rect2(), "inside": -1, "front": -1}
	if _cam == null or _ped == null or not is_instance_valid(_ped):
		return empty
	var lens: Camera3D = _cam.get_node_or_null("Viewport/Lens") as Camera3D
	if lens == null:
		return empty
	var box: AABB = _Aabb_Global_Tree(_ped)
	var lo := Vector2(1e9, 1e9)
	var hi := Vector2(-1e9, -1e9)
	var front: int = 0
	var inside: int = 0
	for i in range(8):
		var wp: Vector3 = box.get_endpoint(i)
		if lens.is_position_behind(wp):
			continue
		front += 1
		var sp: Vector2 = lens.unproject_position(wp)
		lo = Vector2(minf(lo.x, sp.x), minf(lo.y, sp.y))
		hi = Vector2(maxf(hi.x, sp.x), maxf(hi.y, sp.y))
		if sp.x >= 0.0 and sp.x <= float(WANT_SIZE.x) \
			and sp.y >= 0.0 and sp.y <= float(WANT_SIZE.y):
			inside += 1
	if front == 0:
		return empty
	return {"box": Rect2(lo, hi - lo), "inside": inside, "front": front}


func _Render_Labels() -> Array:
	var labels: Array = []
	for i in range(SHOT_NAMES.size()):
		labels.append("R%d-t%d-file" % [i + 1, i])
	labels.append("R4-not-blank")
	labels.append("R5-frames-differ")
	labels.append("R6-box-moved")
	labels.append("R7-turn-real")
	labels.append("R8-stop-real")
	labels.append("R9-capture-book")
	labels.append("R10-source-lint")
	return labels


func _skip(labels: Array) -> void:
	for label: Variant in labels:
		print("[skip] %s（display=%s adapter='%s'：无真渲染器，--headless 是 dummy）" % [
			String(label), DisplayServer.get_name(),
			RenderingServer.get_video_adapter_name()])
		_skipped += 1


# ================================================================ 像素测量

func _decode(jpg: PackedByteArray) -> Image:
	if jpg.is_empty():
		return null
	var img := Image.new()
	if img.load_jpg_from_buffer(jpg) != OK:
		return null
	return img


func _lum(c: Color) -> float:
	return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b


func _Gray_Stats(img: Image) -> Dictionary:
	var sum_l: float = 0.0
	var sum2: float = 0.0
	var count: int = 0
	var lo: float = 1.0
	var hi: float = 0.0
	var white: int = 0
	for y in range(0, img.get_height(), 2):
		for x in range(0, img.get_width(), 2):
			var l: float = _lum(img.get_pixel(x, y))
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
	return {"mean": mean, "var": maxf(sum2 / float(count) - mean * mean, 0.0),
		"min": lo, "max": hi, "white": float(white) / float(count)}


## 两段 JPEG 的逐字节差异占比（长度不同也照算：差额部分记为"不同"）。
func _Byte_Diff_Ratio(a: PackedByteArray, b: PackedByteArray) -> float:
	var n: int = mini(a.size(), b.size())
	var total: int = maxi(a.size(), b.size())
	if total == 0:
		return 0.0
	var diff: int = maxi(a.size(), b.size()) - n
	for i in range(n):
		if a[i] != b[i]:
			diff += 1
	return float(diff) / float(total)


## 两图在 `region` 里的灰度平均绝对差（步长 DIFF_STEP 抽样；region 先夹到画面内）。
## 为什么限定区域：人物只占画面 2~4%，量全图等于在量"背景有没有变"（实测 0.0025 < 0.02）。
func _Gray_Diff_In(a: Image, b: Image, region: Rect2) -> float:
	if a == null or b == null or a.is_empty() or b.is_empty() or a.get_size() != b.get_size():
		return -1.0
	var r: Rect2 = region.grow(BOX_PAD_PX).intersection(
		Rect2(0.0, 0.0, float(a.get_width()), float(a.get_height())))
	if r.size.x < 2.0 or r.size.y < 2.0:
		return -1.0
	var sum_l: float = 0.0
	var count: int = 0
	for y in range(int(r.position.y), int(r.position.y + r.size.y), DIFF_STEP):
		for x in range(int(r.position.x), int(r.position.x + r.size.x), DIFF_STEP):
			sum_l += absf(_lum(a.get_pixel(x, y)) - _lum(b.get_pixel(x, y)))
			count += 1
	return sum_l / float(count) if count > 0 else -1.0


## JPEG 里的 SOF0 尺寸（宽高）。
func _Sof_Size(jpg: PackedByteArray) -> Vector2i:
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
			return Vector2i((jpg[i + 7] << 8) | jpg[i + 8], (jpg[i + 5] << 8) | jpg[i + 6])
		if marker == 0xD8 or marker == 0xD9 or marker == 0x01 \
				or (marker >= 0xD0 and marker <= 0xD7):
			i += 2
			continue
		var seg_len: int = (jpg[i + 2] << 8) | jpg[i + 3]
		i += 2 + maxi(seg_len, 2)
	return Vector2i(-1, -1)


# ================================================================ AABB / 文件 / 统计

func _Aabb_Of_Local_In(xf: Transform3D, local: AABB) -> AABB:
	var out := AABB()
	var first: bool = true
	for i in range(8):
		var corner: Vector3 = xf * local.get_endpoint(i)
		if first:
			out = AABB(corner, Vector3.ZERO)
			first = false
		else:
			out = out.expand(corner)
	return out


func _Aabb_Global_Tree(node: Node) -> AABB:
	var boxes: Array[AABB] = []
	_Gather_Global(node, boxes)
	if boxes.is_empty():
		return AABB()
	var out: AABB = boxes[0]
	for i in range(1, boxes.size()):
		out = out.merge(boxes[i])
	return out


func _Gather_Global(node: Node, boxes: Array[AABB]) -> void:
	if node == null:
		return
	if node is MeshInstance3D:
		var mi: MeshInstance3D = node
		if mi.mesh != null:
			boxes.append(_Aabb_Of_Local_In(mi.global_transform, mi.get_aabb()))
	for child: Node in node.get_children():
		_Gather_Global(child, boxes)


## 找 AnimationPlayer（按**类型**递归：导入器给的节点名会随版本变）。
func _Find_Anim_Player(node: Node) -> AnimationPlayer:
	if node == null:
		return null
	var ap: AnimationPlayer = node as AnimationPlayer
	if ap != null:
		return ap
	for child: Node in node.get_children():
		var found: AnimationPlayer = _Find_Anim_Player(child)
		if found != null:
			return found
	return null


func _Count_Nodes(node: Node) -> int:
	if node == null:
		return 0
	var n: int = 1
	for child: Node in node.get_children():
		n += _Count_Nodes(child)
	return n


## 读源码并**剥掉整行注释**（R10 的 lint 只看代码；本文件头注释里全是禁用词）。
func _Read_Code_Only(res_path: String) -> String:
	var f: FileAccess = FileAccess.open(res_path, FileAccess.READ)
	if f == null:
		return ""
	var raw: String = f.get_as_text()
	f.close()
	var kept := PackedStringArray()
	for line: String in raw.split("\n"):
		if line.strip_edges().begins_with("#"):
			continue
		kept.append(line)
	return "\n".join(kept)


func _ensure_out_dir() -> bool:
	if OUT_DIR.is_empty() or not OUT_DIR.is_absolute_path():
		return false
	if DirAccess.dir_exists_absolute(OUT_DIR):
		return true
	DirAccess.make_dir_absolute(OUT_DIR)
	return DirAccess.dir_exists_absolute(OUT_DIR)


func _File_Size(path: String) -> int:
	if not FileAccess.file_exists(path):
		return -1
	var f: FileAccess = FileAccess.open(path, FileAccess.READ)
	if f == null:
		return -2
	var n: int = f.get_length()
	f.close()
	return n


func _File_Head(path: String) -> String:
	var bytes: PackedByteArray = FileAccess.get_file_as_bytes(path)
	if bytes.size() < 3:
		return ""
	return bytes.slice(0, 3).hex_encode()


# ================================================================ 收尾 / 断言

func _Teardown() -> void:
	var nodes: Array[Node] = [_cam, _world]
	for node: Node in nodes:
		if node != null and is_instance_valid(node):
			node.queue_free()
			await process_frame
			await process_frame
	_cam = null
	_world = null
	_ped = null


## 前置失败时把后面不会再跑的断言一次性标红（保持 total 恒定，别让"少跑"伪装成"全绿"）。
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
	print("[check] env display=%s adapter='%s' frames_total=%d" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name(), _counter])
	if _skipped > 0:
		print("[check] skipped=%d（R 组：无真渲染器，不计入 total）" % _skipped)
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
