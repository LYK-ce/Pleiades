# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# task_35 Step 3（原支线 walking_person Step 4）—— 行人**真的在走**的三帧真渲染自检
# （14 项：A 组结构 4 + R 组渲染 10）。
#
# task_35 Step 3 的改动（total **不变**：headless 4 / Xvfb 14）：
#   世界不再自动让人绕圈（`World.WALKERS` / `BuildWalkers()` 已删），本文件的主语从
#   `PersonWalking.tscn` 的 `walker_01` 换成 `World.PEDESTRIANS[0]` 的 `pedestrian_01`
#   （`Pedestrian.tscn`，同一个 glb、索引 0 的出生点**就是旧环路首点** (44,96)），
#   驱动方式从 `Person.Set_Progress_M()` 换成**一次** `Pedestrian.SetCommand({"v": 1.26, "dir": 0.0})`
#   —— 于是本文件回答的问题变成："**外部指令**能不能让世界里的人真的逐帧走过去，且动画一直播"。
#   机位几何与 walking_person 时期**逐位相同**（旧机位就是按 (44,96) 这个人摆的）。
#
# 跑法 ①（**主验收**，真渲染档，逐字来自 step.md）：
#   cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion
#   DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot \
#     --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
#     --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_person_walk_shots.gd
#   期望：total=14 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR，
#         三张 JPEG 落在 tmp/walking_person/shots/walk_t{0,1,2}.jpg（沿用旧目录/文件名）。
#   **绝不加 --headless**：dummy 渲染器下 `frame_post_draw` 不触发、`get_image()` 返回 null
#   （COMPONENTS.md §9.1 E26）。headless 时 R 组整体 `[skip]` 且**不计入 total**。
#
# 跑法 ②（headless 冒烟，只跑 A 组 4 项，**不得**出现假绿）：
#   timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_person_walk_shots.gd
#   期望：`[skip] R1..R10` 恰 10 条 + skipped=10 + total=4 failed=0 + SELF-CHECK OK。
#
# 断言分组：
#   A(4) 结构：世界能载 + `Pedestrians` 里恰 `World.PEDESTRIANS.size()` 个、
#             第 0 个是 `pedestrian_01`（来自 Pedestrian.tscn / src/chassis/Pedestrian.gd）/
#             **站着时**动画在播且 `speed_scale == 0.0`（Step 1 的 "v=0 定格" 语义）/
#             出生位姿 == `PEDESTRIANS[0]` 的 pos/yaw（≤1 mm / 1 mrad ⇒ 世界摆位真的按表落地）。
#   R(10) 渲染：三张文件（体积/魔数/SOF0 尺寸）/ 非全黑 / 机位三次逐位相同 /
#             人体 AABB 8 角点全在画面内且框高占比 ∈[0.15,0.45] /
#             物理帧数 40±2 且 Δx 0.840±0.05 m 且 Δx/Δframes == v/60（±2%）+ 源码 lint /
#             框水平中心位移 ≥30 px（**位置在变**）/ 框内差分 ≥3× 框外（**不是背景在晃**）/
#             框裁齐到 64×64 后仍有显著差（**平移被消掉 ⇒ 姿态在变，不是滑行**）。
#
# 关键取舍（每条都有实测依据，别"优化"掉）：
#   ① `physics_frame` 由本文件**自己计数**（`_counter`），断言用「计数器差」而不是"我数了几次 await"：
#      `Capture_To_File()` 内部 warmup 会偷 1~2 个物理帧，用计数器才数得准（FRAME_TOL=2 兜底）。
#   ② 三次读 `GetPose().x` 都发生在 `await physics_frame` 恢复后的同一相位，且用**绝对目标帧号**
#      （`_c0 + 40k`）等待 → 读-读之间的物理帧差**恰好** 40/40，Δx 因此可直接对 v/60。
#   ③ 投影测量**必须**放在 `Capture_To_File()` **之后**（E26：`Lens.global_transform`
#      只在抓拍内部同步，提前取会拿到上一帧/单位阵）。
#   ④ `MeshInstance3D.get_aabb()` 是**绑定姿态**的 mesh 局部 AABB（蒙皮变形是运行时的事），
#      所以投影框是稳定可复现的 —— 这正是我们要的"位置尺子"，不追求贴合剪影。
#   ⑤ 本文件**不 push_error**（失败一律 `_check(..., false, ...)` + `quit(1)`），
#      退出前逐级 queue_free + await 两帧，避免 `resources still in use at exit`。
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
const SELF_PATH := "res://src/tests/check_person_walk_shots.gd"

# ---------------------------------------------------------------- 出图
const OUT_DIR := "/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/walking_person/shots"
const SHOT_NAMES := ["walk_t0.jpg", "walk_t1.jpg", "walk_t2.jpg"]
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
## 机位**逐位沿用** walking_person 时期：被拍的人仍在世界 (44,96)（= 旧环路首点 = 现在
## `World.PEDESTRIANS[0].pos`），机位在其 (+4, +5) 处 → 与朝向夹角 atan2(5,4)=51°，
## 既能看清正脸+侧身，又让 40 帧 0.84 m 的走位主要**横向**投在画面上（不会被"迎面走近"吃掉）。
## y=101 在空地内侧，与 wall_c（y∈[92,93]）隔着 8 m。
const CAM_EYE_W := Vector3(48.0, 101.0, 0.0)
const CAM_EYE_H_M := 1.50
const CAM_AIM_W := Vector3(44.8, 96.0, 0.92)

# ---------------------------------------------------------------- 帧计划 / 判据
## t0 / t1 / t2 = 0 / 40 / 80 物理帧（step.md 指定，不许改）。
const SHOT_FRAMES := [0, 40, 80]
## `Capture_To_File` 内部 warmup 会偷 1~2 帧 → 用 ±2。
const FRAME_TOL := 2
## 指令速度（m/s）：= World.PEDESTRIANS 的行人默认 max_speed = 走路 glb 自带步速。
## 开拍前**只下发一次** `SetCommand({"v": CMD_V_MPS, "dir": 0.0})`（R7④ 的 lint 锁死次数）。
const CMD_V_MPS := 1.26
## 朝东（+X）：pedestrian_01 的表内 yaw_deg = 0。
const CMD_DIR_RAD := 0.0
## 40 帧 × 1.260/60 = 0.840 m。
const STEP_M_NOMINAL := 0.84
const STEP_M_TOL := 0.05
## Δx/Δframes 与 v/60 的相对误差上限。
const STEP_RATE_TOL := 0.02
## 出生位姿容差：1 mm / 1 mrad。
const POSE_TOL_M := 1.0e-3
const YAW_TOL_RAD := 1.0e-3

## 框**水平**中心位移下限（像素）：预测 58 / 68 / 126 px，留 1.9× 余量。
const BOX_DX_MIN_PX := 30.0
## 框内灰度平均绝对差下限。
const IN_DIFF_MIN := 0.02
## 框内 / 框外 的倍数下限。
const IN_OUT_RATIO_MIN := 3.0
## 框内 64×64 对齐裁剪差下限（平移不变量 → 证"姿态在变"）。
const CROP_DIFF_MIN := 0.010
const CROP_N := 64
## 全图差分抽样步长。
const DIFF_STEP := 4
const BOX_PAD_PX := 2.0
## 框高占画面高的比例窗口（walking_person 时期实测 0.241→0.260→0.286，同一机位同一个人）。
const H_FRAC_MIN := 0.15
const H_FRAC_MAX := 0.45
## 机位逐位相等的判据（`is_equal_approx` 太松，用 1e-9）。
const XF_TOL := 1.0e-9

# ---------------------------------------------------------------- 状态
var _total: int = 0
var _failed: int = 0
var _skipped: int = 0

var _world: Node3D = null
var _ped: Node3D = null
var _pedestrians_root: Node3D = null
var _cam: Node3D = null
var _ws: Node = null

## 物理帧计数器（每收到一次 `SceneTree.physics_frame` +1；见头注释①）。
var _counter: int = 0
## t0 的读帧基准（R7 的绝对目标帧号 = _c0 + SHOT_FRAMES[k]）。
var _c0: int = 0
## 每次读位姿时的绝对帧号（相对 _c0）与世界 x（米）。
var _frame_reads: Array[int] = []
var _x_reads: Array[float] = []

var _shots: Array[Dictionary] = []
var _imgs: Array[Image] = []


func _initialize() -> void:
	_run()


func _run() -> void:
	physics_frame.connect(_on_physics_frame)
	var ok: bool = await _build_world()
	if ok:
		ok = _check_anim()
	if ok:
		ok = await _build_camera()
	# A4 只读位姿、**不瞬移**（世界把人按表摆好之后没人动过它）。
	if ok:
		ok = _check_spawn_pose()
	if ok and _cam != null and _cam.Can_Capture():
		await _run_render()
	else:
		_skip(_render_labels())
	await _teardown()
	_finish()


func _on_physics_frame() -> void:
	_counter += 1


# ================================================================ A 组：世界与行人结构（4 项）

## A1 + A2：载真 `World.tscn`（入树前关 WS autolisten，避免连 9999），
## 断言 `Pedestrians` 容器里恰 `World.PEDESTRIANS.size()` 个人、第 0 个是表里的名字，
## 来自 `Pedestrian.tscn`、根脚本 `src/chassis/Pedestrian.gd`。
func _build_world() -> bool:
	var labels: Array = ["A1-world-loads", "A2-pedestrian-instance"]
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	var world: Node3D = packed.instantiate() as Node3D if packed != null else null
	if world == null:
		for label: Variant in labels:
			_check(String(label), false, "前置失败：World.tscn 不可加载/实例化")
		return false

	_ws = world.get_node_or_null("Systems/WebSocketManager")
	if _ws != null:
		_ws.set("autolisten", false)
	root.add_child(world)
	_world = world
	await process_frame
	await physics_frame
	await process_frame

	_pedestrians_root = world.call("GetPedestriansRoot") as Node3D
	var nodes: int = _count_nodes(world)
	var autolisten: String = "<none>"
	if _ws != null:
		autolisten = str(_ws.get("autolisten"))
	_check("A1-world-loads", _pedestrians_root != null,
		"nodes=%d pedestrians_root=%s ws_autolisten=%s" % [nodes,
			str(_pedestrians_root != null), autolisten])

	var kids: Array[Node] = []
	if _pedestrians_root != null:
		kids = _pedestrians_root.get_children()
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
		_check("A3-anim-playing", false, "前置失败：Pedestrians 里没有实例")
		return false
	return true


## A3：**站着时**动画在播且步频倍率 == 0.0。
## 这是 Step 1 定的语义（`Pedestrian.Play_Walk()`：`v=0 → speed_scale=0`，定格但仍 `is_playing`），
## 与 walking_person 时期 `walk_speed>0 → speed_scale≈1` 的断言**相反**，别照抄旧判据（plan.md §7 R6）。
func _check_anim() -> bool:
	var ap: AnimationPlayer = _find_anim_player(_ped)
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
	_check("A3-anim-playing",
		ap != null and clips >= 1 and playing and not clip.is_empty()
			and absf(speed_scale) <= 1.0e-6 and absf(idle_speed) <= 1.0e-6,
		"clip=%s speed_scale=%.3f(want 0.000) clips=%d playing=%s v=%.3f(want 0 = 未驱动时站着)"
			% [clip, speed_scale, clips, str(playing), idle_speed])
	# 拿不到动画/片子的话渲染段就没有"姿态在变"可测，直接判前置失败。
	return ap != null and clips >= 1


## 相机 = `CameraSensor.tscn` 实例**直接挂 World 下**（不挂车、不建 UGV）：
## `Can_Capture()` 是渲染组的**唯一**闸门。机位只在这里摆一次（三次抓拍位置全等 → R5）。
func _build_camera() -> bool:
	if _world == null or not is_instance_valid(_world):
		return false
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
	_place_camera()
	await process_frame
	return true


## 机位：世界 (48, 101) 地面、光心 1.5 m，瞄 (44.8, 96, 0.92)。
## 只摆**一次**；之后三次抓拍期间不再动它（R5 会逐位比对）。
func _place_camera() -> void:
	if _cam == null or not is_instance_valid(_cam):
		return
	var eye: Vector3 = WorldTransform.PosToGodot(CAM_EYE_W) + Vector3(0.0, CAM_EYE_H_M, 0.0)
	var aim: Vector3 = WorldTransform.PosToGodot(CAM_AIM_W)
	_cam.global_position = eye
	_cam.look_at(aim, Vector3.UP)


## A4：出生位姿 == `World.PEDESTRIANS[0]` 的 pos / yaw（≤1 mm / 1 mrad）。
## 这是"世界摆位真的按表落地"的断言 —— 尤其 `yaw_deg`：`Pedestrian.Configure()` 会把
## `_yaw_target` 对齐到当前 `rotation.y`，世界侧若漏调它，`yaw_deg≠0` 的人第一帧就被拧回 0。
## 本条**不写任何位姿**（R7④ 的 lint 会数"瞬移调用"与"直接写行人节点位置"的命中数 == 0）。
func _check_spawn_pose() -> bool:
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
		"pos=(%.4f, %.4f, %.4f)(want (%.1f, %.1f, %.1f)) Δ=%.6f m(<=%.4f) "
			% [got.x, got.y, got.z, want_pos.x, want_pos.y, want_pos.z, d_pos, POSE_TOL_M]
		+ "yaw=%.6f(rad, want %.6f=%.1f°) Δ=%.6f(<=%.4f)" % [got_yaw, want_yaw,
			float(WorldGd.PEDESTRIANS[0]["yaw_deg"]), d_yaw, YAW_TOL_RAD])
	return ok


# ================================================================ R 组：三帧真渲染（10 项）

## 帧推进时序（`await` 语义是坑，照抄）：
##   ① 先下发**唯一一次** `SetCommand`（v=1.26 朝东）→ 之后每一帧都由 `Pedestrian._physics_process()`
##      自己 `move_and_slide()` 前进，本文件**绝不**写位姿；
##   ② t0 的读与 t1/t2 用**绝对目标帧号** `_c0 + 40k` 等待（计数器自己数，把抓拍偷走的帧算进去）；
##   ③ 每拍都在 `Capture_To_File()` **之后**才做投影测量（E26）。
func _run_render() -> void:
	_ensure_out_dir()
	_frame_reads = []
	_x_reads = []
	# 唯一一次驱动：v = 资产步速、dir = 0（朝 +X 东，与表内 yaw_deg 一致）。
	# 不下发就是站着（A3 已证明 idle 语义），下发后由底盘自己的 `_physics_process()` 推进。
	_ped.call("SetCommand", {"v": CMD_V_MPS, "dir": CMD_DIR_RAD})
	# 相位对齐（沿用 walking_person 实测结论）：`SceneTree.physics_frame` 在 `_physics_process`
	# **之前**发出，所以这里恢复时本帧还没推进；三次读因此落在同一相位，Δx 可直接对 v/60。
	await physics_frame
	_c0 = _counter
	_x_reads.append(float((_ped.call("GetPose") as Dictionary).get("x", 0.0)))
	_frame_reads.append(0)
	await _capture(0)
	for k in range(1, SHOT_FRAMES.size()):
		await _wait_until(_c0 + int(SHOT_FRAMES[k]))
		_x_reads.append(float((_ped.call("GetPose") as Dictionary).get("x", 0.0)))
		_frame_reads.append(_counter - _c0)
		await _capture(k)

	# ---- R1~R3：三张文件。存在 + 体积 + JPEG 魔数 + SOF0 尺寸。
	for i in range(SHOT_FRAMES.size()):
		var d: Dictionary = _shots[i]
		var bytes: int = int(d["bytes"])
		var head: String = String(d["head"])
		var sof: Vector2i = _sof_size(d["jpg"])
		var ok: bool = bool(d["ok"]) and bytes > MIN_BYTES and head == "ffd8ff" \
			and sof == WANT_SIZE
		_check("R%d-t%d-file" % [i + 1, i], ok,
			"bytes=%d(>%d) head=%s sof0=%s want=%s saved=%s" % [bytes, MIN_BYTES, head,
				str(sof), str(WANT_SIZE), str(d["ok"])])

	# ---- R4：三张都非全黑（灰度方差 / 均值窗口 / 极差 / 高光占比）。
	var blank_ok: bool = true
	var blank_detail: Array[String] = []
	for i in range(SHOT_FRAMES.size()):
		var img: Image = _decode(_shots[i]["jpg"])
		if img == null or img.is_empty():
			blank_ok = false
			blank_detail.append("t%d:decode-fail" % i)
			continue
		var st: Dictionary = _gray_stats(img)
		var spread: float = float(st["max"]) - float(st["min"])
		var ok: bool = float(st["var"]) > VAR_MIN and float(st["mean"]) > MEAN_MIN \
			and float(st["mean"]) < MEAN_MAX and spread > SPREAD_MIN \
			and float(st["white"]) < WHITE_MAX
		if not ok:
			blank_ok = false
		blank_detail.append("t%d:var=%.4f mean=%.3f spread=%.2f white=%.4f" % [
			i, float(st["var"]), float(st["mean"]), spread, float(st["white"])])
	_check("R4-not-blank", blank_ok, "；".join(blank_detail))

	# ---- R5：机位固定。三次抓拍记录下的**光心**与**镜头**世界变换必须逐位相等。
	var cam_dev: float = 0.0
	var lens_dev: float = 0.0
	for i in range(1, SHOT_FRAMES.size()):
		cam_dev = maxf(cam_dev, _xf_diff(_shots[0]["cam_xf"], _shots[i]["cam_xf"]))
		lens_dev = maxf(lens_dev, _xf_diff(_shots[0]["lens_xf"], _shots[i]["lens_xf"]))
	_check("R5-camera-fixed", cam_dev <= XF_TOL and lens_dev <= XF_TOL,
		"cam_max_dev=%.6f lens_max_dev=%.6f(<=%.6f) eye_world=(%.2f, %.2f, %.2f)+h%.2f" % [
			cam_dev, lens_dev, XF_TOL, CAM_EYE_W.x, CAM_EYE_W.y, CAM_EYE_W.z, CAM_EYE_H_M])

	# ---- R6：三帧人体 AABB 8 角点全在画面内，且框高占比落在 [0.15, 0.45]。
	var frame_ok: bool = true
	var frame_detail: Array[String] = []
	for i in range(SHOT_FRAMES.size()):
		var box: Rect2 = _shots[i]["box"]
		var h_frac: float = box.size.y / float(WANT_SIZE.y)
		var ok: bool = int(_shots[i]["front"]) == 8 and int(_shots[i]["inside"]) == 8 \
			and h_frac >= H_FRAC_MIN and h_frac <= H_FRAC_MAX
		if not ok:
			frame_ok = false
		frame_detail.append("t%d:front=%d/8 inside=%d/8 h_frac=%.3f" % [
			i, int(_shots[i]["front"]), int(_shots[i]["inside"]), h_frac])
	_check("R6-pedestrian-in-frame", frame_ok, "；".join(frame_detail))

	# ---- R7：帧是真的在推进（**堵死瞬移**：帧数 / Δx / Δx-per-frame 三把尺子 + 源码 lint）。
	#      尺子从 walking_person 的「弧长 Δs」换成「`GetPose().x` 的 Δ」—— 世界侧已无路径弧长可读，
	#      而 +X 位移就是 dir=0 时这条指令**唯一**该产生的世界位移。
	var want_rate: float = CMD_V_MPS / 60.0
	var frames_ok: bool = true
	var step_ok: bool = true
	var rate_ok: bool = true
	var r7_detail: Array[String] = []
	for k in range(1, SHOT_FRAMES.size()):
		var df: int = _frame_reads[k] - _frame_reads[k - 1]
		var dx: float = _x_reads[k] - _x_reads[k - 1]
		var nominal: int = int(SHOT_FRAMES[k]) - int(SHOT_FRAMES[k - 1])
		if absi(df - nominal) > FRAME_TOL:
			frames_ok = false
		if absf(dx - STEP_M_NOMINAL) > STEP_M_TOL:
			step_ok = false
		var per_frame: float = dx / float(df) if df > 0 else -1.0
		var rel: float = absf(per_frame - want_rate) / want_rate if want_rate > 0.0 else 1.0
		if rel > STEP_RATE_TOL or df <= 0 or dx <= 0.0:
			rate_ok = false
		r7_detail.append("t%d→t%d frames=%d(==%d±%d) Δx=%.4f m(%.3f±%.2f) Δx/frame=%.6f"
			% [k - 1, k, df, nominal, FRAME_TOL, dx, STEP_M_NOMINAL, STEP_M_TOL, per_frame]
			+ "(want %.6f 偏差 %.2f%%)" % [want_rate, rel * 100.0])
	# ④ 源码 lint：本文件里**不许**出现任何位姿写入，且 `SetCommand` 恰 1 处。
	#    针串在此拼出来，避免把本行自身算进命中数。
	var needle_progress: String = "call(\"Set_Progress_" + "M\""
	var needle_ped_pos: String = "_ped.global_posi" + "tion"
	var needle_cmd: String = "call(\"SetCom" + "mand\""
	var src: String = _read_text(SELF_PATH)
	var hits_progress: int = src.count(needle_progress) if not src.is_empty() else -1
	var hits_pos: int = src.count(needle_ped_pos) if not src.is_empty() else -1
	var hits_cmd: int = src.count(needle_cmd) if not src.is_empty() else -1
	r7_detail.append("lint Set_Progress_M=%d(==0) " % hits_progress
		+ "_ped." + "global_position=%d(==0) " % hits_pos
		+ "SetCommand=%d(==1) 源码 %d B" % [hits_cmd, src.length()])
	_check("R7-frames-real", frames_ok and step_ok and rate_ok
		and hits_progress == 0 and hits_pos == 0 and hits_cmd == 1,
		"；".join(r7_detail))

	# ---- R8：**位置在变** —— 框水平中心位移（预测 58 / 68 / 126 px）。
	var moved_ok: bool = true
	var moved_detail: Array[String] = []
	for i in range(SHOT_FRAMES.size()):
		for j in range(i + 1, SHOT_FRAMES.size()):
			var a: Rect2 = _shots[i]["box"]
			var b: Rect2 = _shots[j]["box"]
			var dx: float = absf(b.get_center().x - a.get_center().x)
			if dx < BOX_DX_MIN_PX:
				moved_ok = false
			moved_detail.append("t%d→t%d Δx=%.1f px(>=%.0f) 中心 %.1f→%.1f" % [
				i, j, dx, BOX_DX_MIN_PX, a.get_center().x, b.get_center().x])
	_check("R8-box-moved", moved_ok, "；".join(moved_detail))

	# ---- R9：**姿态在变，不是整体平移**（第一把尺子）：框内差分 ≫ 框外差分。
	_imgs = []
	for i in range(SHOT_FRAMES.size()):
		_imgs.append(_decode(_shots[i]["jpg"]))
	var pose_ok: bool = true
	var pose_detail: Array[String] = []
	for i in range(SHOT_FRAMES.size()):
		for j in range(i + 1, SHOT_FRAMES.size()):
			var d: Dictionary = _diff_in_out(_imgs[i], _imgs[j],
				_shots[i]["box"], _shots[j]["box"])
			var din: float = float(d["in"])
			var dout: float = float(d["out"])
			var ratio: float = float(d["ratio"])
			if din < IN_DIFF_MIN or ratio < IN_OUT_RATIO_MIN:
				pose_ok = false
			pose_detail.append("t%d→t%d in=%.4f(>=%.2f) out=%.4f ratio=%.1f(>=%.1f)" % [
				i, j, din, IN_DIFF_MIN, dout, ratio, IN_OUT_RATIO_MIN])
	_check("R9-pose-in-vs-out", pose_ok, "；".join(pose_detail))

	# ---- R10：**姿态在变**（第二把尺子，step.md 只要求 R9，这条更硬）：
	#      两框各裁到同一 64×64 再比 —— 平移（乃至轻度缩放）被归一化掉，
	#      剩下的差只可能来自剪影/四肢变化。整体平移的"滑行"在这里会掉到 ~0。
	var crop_ok: bool = true
	var crop_detail: Array[String] = []
	var crops: Array[PackedFloat32Array] = []
	for i in range(SHOT_FRAMES.size()):
		crops.append(_crop_gray(_imgs[i], _shots[i]["box"]))
	for i in range(SHOT_FRAMES.size()):
		for j in range(i + 1, SHOT_FRAMES.size()):
			if crops[i].is_empty() or crops[j].is_empty():
				crop_ok = false
				crop_detail.append("t%d→t%d:empty-crop" % [i, j])
				continue
			var diff: float = _mean_abs(crops[i], crops[j])
			if diff < CROP_DIFF_MIN:
				crop_ok = false
			crop_detail.append("t%d→t%d mean_abs=%.5f(>=%.3f)" % [
				i, j, diff, CROP_DIFF_MIN])
	_check("R10-silhouette-diff", crop_ok, "；".join(crop_detail))


## 抓一张 + 记录。**投影必须在抓拍之后**（E26）。`x_m` / `frames` 取抓拍**之前**读到的值。
func _capture(idx: int) -> void:
	var path: String = "%s/%s" % [OUT_DIR, SHOT_NAMES[idx]]
	var saved: bool = await _cam.Capture_To_File(path)
	var lens: Camera3D = _cam.get_node_or_null("Viewport/Lens") as Camera3D
	var screen: Dictionary = _project_ped()
	var lens_xf := Transform3D()
	if lens != null:
		lens_xf = lens.global_transform
	_shots.append({
		"ok": saved, "path": path, "bytes": _file_size(path), "head": _file_head(path),
		"jpg": FileAccess.get_file_as_bytes(path),
		"box": screen["box"], "inside": int(screen["inside"]), "front": int(screen["front"]),
		"x_m": _x_reads[idx], "frames": _frame_reads[idx],
		"cam_xf": _cam.global_transform, "lens_xf": lens_xf,
		"playing": bool(_ped.call("Get_Speed") > 0.0),
	})
	var box: Rect2 = _shots[idx]["box"]
	print("[check] shot t%d x=%.3f m frames=%d box=P(%.1f, %.1f) S(%.1f, %.1f) "
		% [idx, _x_reads[idx], _frame_reads[idx], box.position.x, box.position.y,
			box.size.x, box.size.y]
		+ "center=(%.1f, %.1f) inside=%d/8 front=%d/8 moving=%s bytes=%d" % [
			box.get_center().x, box.get_center().y, int(_shots[idx]["inside"]),
			int(_shots[idx]["front"]), str(_shots[idx]["playing"]), int(_shots[idx]["bytes"])])


## 一直 `await physics_frame` 直到**自己数到的**绝对帧号到 `target`（含抓拍偷走的帧）。
func _wait_until(target: int) -> void:
	var guard: int = 0
	while _counter < target and guard < 1000:
		await physics_frame
		guard += 1


## 被拍的人（pedestrian_01）的世界 AABB 8 角点投到屏幕：包围框 + 落在画面内的角点数。
func _project_ped() -> Dictionary:
	var empty: Dictionary = {"box": Rect2(), "inside": -1, "front": -1}
	if _cam == null or _ped == null or not is_instance_valid(_ped):
		return empty
	var lens: Camera3D = _cam.get_node_or_null("Viewport/Lens") as Camera3D
	if lens == null:
		return empty
	var box: AABB = _aabb_global_tree(_ped)
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
		return {"box": Rect2(), "inside": -1, "front": -1}
	return {"box": Rect2(lo, hi - lo), "inside": inside, "front": front}


func _render_labels() -> Array:
	var labels: Array = []
	for i in range(SHOT_FRAMES.size()):
		labels.append("R%d-t%d-file" % [i + 1, i])
	labels.append("R4-not-blank")
	labels.append("R5-camera-fixed")
	labels.append("R6-pedestrian-in-frame")
	labels.append("R7-frames-real")
	labels.append("R8-box-moved")
	labels.append("R9-pose-in-vs-out")
	labels.append("R10-silhouette-diff")
	return labels


func _skip(labels: Array) -> void:
	for label: Variant in labels:
		print("[skip] %s（display=%s adapter='%s'：无真渲染器，--headless 是 dummy）" % [
			String(label), DisplayServer.get_name(),
			RenderingServer.get_video_adapter_name()])
		_skipped += 1


# ================================================================ 像素测量

## 框内 / 框外灰度平均绝对差（一次网格遍历，步长 DIFF_STEP）：
##   `box = union(box_i, box_j).grow(BOX_PAD_PX)` 以内记 in，以外记 out。
## "整体平移"也会让 in 变亮，所以 R9 只是第一把尺子；第二把是 R10 的对齐裁剪差。
func _diff_in_out(a: Image, b: Image, box_a: Rect2, box_b: Rect2) -> Dictionary:
	var out: Dictionary = {"in": -1.0, "out": -1.0, "ratio": -1.0, "cnt_in": 0, "cnt_out": 0}
	if a == null or b == null or a.is_empty() or b.is_empty():
		return out
	if a.get_size() != b.get_size():
		return out
	var region: Rect2 = box_a.merge(box_b).grow(BOX_PAD_PX)
	var sum_in: float = 0.0
	var cnt_in: int = 0
	var sum_out: float = 0.0
	var cnt_out: int = 0
	for y in range(0, a.get_height(), DIFF_STEP):
		for x in range(0, a.get_width(), DIFF_STEP):
			var p := Vector2(float(x), float(y))
			var d: float = absf(_lum(a.get_pixel(x, y)) - _lum(b.get_pixel(x, y)))
			if region.has_point(p):
				sum_in += d
				cnt_in += 1
			else:
				sum_out += d
				cnt_out += 1
	var din: float = sum_in / float(cnt_in) if cnt_in > 0 else -1.0
	var dout: float = sum_out / float(cnt_out) if cnt_out > 0 else -1.0
	return {"in": din, "out": dout,
		"ratio": din / maxf(dout, 1.0e-6) if din >= 0.0 and dout >= 0.0 else -1.0,
		"cnt_in": cnt_in, "cnt_out": cnt_out}


func _decode(jpg: PackedByteArray) -> Image:
	if jpg.is_empty():
		return null
	var img := Image.new()
	var err: Error = img.load_jpg_from_buffer(jpg)
	if err != OK:
		return null
	return img


func _lum(c: Color) -> float:
	return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b


func _gray_stats(img: Image) -> Dictionary:
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


## 把矩形区域内的灰度重采样成 CROP_N×CROP_N 的一维数组（最近邻，够做差分）。
## **重采样**这一步就是"平移不变量"。
func _crop_gray(img: Image, rect: Rect2) -> PackedFloat32Array:
	var out := PackedFloat32Array()
	if img == null or img.is_empty():
		return out
	var r: Rect2 = rect.intersection(Rect2(0.0, 0.0, float(img.get_width()),
		float(img.get_height())))
	if r.size.x < 4.0 or r.size.y < 4.0:
		return out
	out.resize(CROP_N * CROP_N)
	for y in range(CROP_N):
		for x in range(CROP_N):
			var sx: int = int(clampf(r.position.x + (float(x) + 0.5) * r.size.x / float(CROP_N),
				0.0, float(img.get_width() - 1)))
			var sy: int = int(clampf(r.position.y + (float(y) + 0.5) * r.size.y / float(CROP_N),
				0.0, float(img.get_height() - 1)))
			out[y * CROP_N + x] = _lum(img.get_pixel(sx, sy))
	return out


func _mean_abs(a: PackedFloat32Array, b: PackedFloat32Array) -> float:
	var n: int = mini(a.size(), b.size())
	if n == 0:
		return 0.0
	var sum_l: float = 0.0
	for i in range(n):
		sum_l += absf(a[i] - b[i])
	return sum_l / float(n)


func _sof_size(jpg: PackedByteArray) -> Vector2i:
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


# ================================================================ AABB / 变换 / 节点

func _aabb_of_local_in(xf: Transform3D, local: AABB) -> AABB:
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


func _aabb_global_tree(node: Node) -> AABB:
	var boxes: Array[AABB] = []
	_gather_global(node, boxes)
	if boxes.is_empty():
		return AABB()
	var out: AABB = boxes[0]
	for i in range(1, boxes.size()):
		out = out.merge(boxes[i])
	return out


func _gather_global(node: Node, boxes: Array[AABB]) -> void:
	if node == null:
		return
	if node is MeshInstance3D:
		var mi: MeshInstance3D = node
		if mi.mesh != null:
			boxes.append(_aabb_of_local_in(mi.global_transform, mi.get_aabb()))
	for child: Node in node.get_children():
		_gather_global(child, boxes)


## 两个 Transform3D 的逐分量最大偏差（origin 用距离，basis 用分量）。
func _xf_diff(a: Transform3D, b: Transform3D) -> float:
	var d: float = (a.origin - b.origin).length()
	d = maxf(d, absf(a.basis.x.x - b.basis.x.x))
	d = maxf(d, absf(a.basis.x.y - b.basis.x.y))
	d = maxf(d, absf(a.basis.x.z - b.basis.x.z))
	d = maxf(d, absf(a.basis.y.x - b.basis.y.x))
	d = maxf(d, absf(a.basis.y.y - b.basis.y.y))
	d = maxf(d, absf(a.basis.y.z - b.basis.y.z))
	d = maxf(d, absf(a.basis.z.x - b.basis.z.x))
	d = maxf(d, absf(a.basis.z.y - b.basis.z.y))
	d = maxf(d, absf(a.basis.z.z - b.basis.z.z))
	return d


## 找 AnimationPlayer（按**类型**递归：导入器给的节点名会随版本变）。
func _find_anim_player(node: Node) -> AnimationPlayer:
	if node == null:
		return null
	var ap: AnimationPlayer = node as AnimationPlayer
	if ap != null:
		return ap
	for child: Node in node.get_children():
		var found: AnimationPlayer = _find_anim_player(child)
		if found != null:
			return found
	return null


# ================================================================ 统计 / 文件

func _count_nodes(node: Node) -> int:
	if node == null:
		return 0
	var n: int = 1
	for child: Node in node.get_children():
		n += _count_nodes(child)
	return n


func _read_text(res_path: String) -> String:
	var f: FileAccess = FileAccess.open(res_path, FileAccess.READ)
	if f == null:
		return ""
	var txt: String = f.get_as_text()
	f.close()
	return txt


func _ensure_out_dir() -> bool:
	if OUT_DIR.is_empty() or not OUT_DIR.is_absolute_path():
		return false
	if DirAccess.dir_exists_absolute(OUT_DIR):
		return true
	DirAccess.make_dir_absolute(OUT_DIR)
	return DirAccess.dir_exists_absolute(OUT_DIR)


func _file_size(path: String) -> int:
	if not FileAccess.file_exists(path):
		return -1
	var f: FileAccess = FileAccess.open(path, FileAccess.READ)
	if f == null:
		return -2
	var n: int = f.get_length()
	f.close()
	return n


func _file_head(path: String) -> String:
	var bytes: PackedByteArray = FileAccess.get_file_as_bytes(path)
	if bytes.size() < 3:
		return ""
	return bytes.slice(0, 3).hex_encode()


# ================================================================ 收尾 / 断言

func _teardown() -> void:
	if _cam != null and is_instance_valid(_cam):
		_cam.queue_free()
		_cam = null
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


func _finish() -> void:
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
