# Presented by KeJi
# Created Date ： 2026-10-24
# Modified Date ： 2026-10-24
#
# 支线 human_model Step 5 — 人物取景拍照自检（真 World + 真 CameraSensor -> 2/4/8 m 三张 JPEG 落盘）。
#
# 做什么：载入**真** `World.tscn`（体素/地面/空气墙/程序化摆好的 person_01），把一台
#   CameraSensor 摆到 person_01 **正前方** 2 m / 4 m / 8 m，每个距离拍一张 1280×720 JPEG 到
#   tmp/human_model/shots/（绝对路径），并就地量出"人在画面里的包围框 / 占比"打印出来，
#   供 tmp/human_model/yolo_report.md 做「期望框 vs YOLO 检测框」的人工核对。
#   本 step 的最终目的：证明"这个世界里的人，YOLOv8n 认得出"（检测在 rust_uav_camera_test 里跑）。
#
# 跑法 ①（纯 headless，结构档：S 组 7 项；R 组 15 项全部 [skip] 且**不计入 total**）：
#   timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_person_shots.gd
#   期望：total=7 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR + 无 "[ws] listening"。
#
# 跑法 ②（**本 step 的真验收**：现成的 Xvfb :99 + 软件 OpenGL，S+R 全跑，出 3 张图）：
#   DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot \
#     --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
#     --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_person_shots.gd
#   期望：total=22 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR。
#   **绝不加 --headless**：那是 dummy 渲染器，`frame_post_draw` 不触发、`get_image()` 返回 null
#   （COMPONENTS.md §9.1 E26），物理上出不了图。
#
# 机位口径（世界 Z-up；person_01 世界 (60,76,0)、yaw 0 = 朝世界 +X，模型原生正脸朝 +X）：
#   机位 = PosToGodot(PERSON_WORLD + (dist, 0, 0)) + Godot 高度 EYE_H
#   看向 = PosToGodot(PERSON_WORLD) + Godot 高度 EYE_H
#   EYE_H = 0.87 m（≈ 人身高一半）：实测 2 m 处整身不裁切（头顶 114 px、脚底 608 px，都在 720 内）。
#   `Camera3D.fov` 是**垂直**角（keep_aspect=KEEP_HEIGHT），fov 70 → 垂直 70°、水平 102.5°；
#   本文件不改 fov（与世界相机同参数），后人别"按水平角"重算又改坏机位。
#
# 阈值来源（**全部是这台机器上实测的，不是估计**；三档实测值见 yolo_report.md）：
#   字节 42317 / 33244 / 29973（quality 0.85 的规划期探针）；人屏内 341×493 / 162×234 / 79×114 px；
#   高度占比 0.685 / 0.325 / 0.158（相邻比 2.11 / 2.06）；
#   灰度 var 0.0093~0.0159、mean 0.543~0.563、spread ≈ 0.996、white ≤ 0.0001；
#   两两差异 byte_ratio 0.858/0.844/0.687、mean_abs 0.0214/0.0234/**0.0051**。
#   ⚠️ 4 m~8 m 只差沿同一轴的推拉，背景几乎不动 → mean_abs 只有 0.0051，
#   所以**不套用** check_camera_shots 的 DIFF_MEAN_MIN=0.02（那会假红）；
#   本文件用 `byte_ratio > 0.10 + mean_abs > 0.002`，另用 R7（高度占比单调）证明三档距离是真的。
#
# 为什么分两档跑（与本工程既有拍照自检同一条铁律）：
#   `--headless` = dummy 渲染器，能出图物理不可能；本脚本用 `CameraSensor.Can_Capture()` 在
#   **碰渲染器之前**分流，R 组整体 SKIP。真渲染必须 `--audio-driver Dummy`（本机无声卡，
#   ALSA 打不开会多一行 ERROR，把"0 ERROR"验收搞假红）。
#   本脚本自身**不 push_error**（失败一律 `_check(..., false, detail)` + `quit(1)`）。
#
# 坐标：位置一律 WorldTransform.PosToGodot()、朝向一律 WorldTransform.YawToGodot()；
#   本文件**不出现**任何 (x, z, y) 换轴字面量（COMPONENTS.md §2.3）。
#
# 注意（GDScript 4.7）：`load()` / `instantiate()` 的返回值一律显式标注类型；
#   退出前必须 `queue_free()` + 逐级 await 两帧，否则报 `ERROR: resources still in use at exit`。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldGd := preload("res://src/world/World.gd")

# ---------------------------------------------------------------- 场景（唯一来源）
const WORLD_SCENE := "res://src/scenes/World.tscn"
const CAMERA_SCENE := "res://src/scenes/sensors/CameraSensor.tscn"

# ---------------------------------------------------------------- 落盘目录（绝对路径，step 硬性）
## 图片必须落在仓库下的 tmp/human_model/shots/，**不得**写进工程目录（res:// 会把图塞进仓库）。
const OUT_DIR := "/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/human_model/shots"
## 后缀用于断言"没写进工程目录"（S6）。
const OUT_SUFFIX := "tmp/human_model/shots"

# ---------------------------------------------------------------- 机位（世界坐标，Z-up，米）
## 被拍的人：世界 (60,76,0)、yaw 0（朝世界 +X）—— 与 World.gd PERSONS[0] 同一份事实，
## S3 会拿本文件的常量去和 World.gd 的 PERSONS[0] 逐位对照，防两处漂移。
const PERSON_NAME := "person_01"
const PERSON_WORLD := Vector3(60.0, 76.0, 0.0)
const PERSON_YAW_DEG := 0.0
## 光心高度（米）：≈ 人身高一半。实测 2 m 处整身不裁切；换成 1.35 m 会切到小腿。
const EYE_H := 0.87

## 机位表：三档距离，同一台相机、同一个人（R7 的"高度占比单调"要可比）。
## `h_lo` / `h_hi` = R4 的"人体在屏高占比"合法带（实测 0.685 / 0.325 / 0.158，两侧留 ≥30% 余量）。
const SHOTS := [
	{"dist": 2.0, "tag": "2m", "file": "person_2m.jpg", "h_lo": 0.45, "h_hi": 0.95},
	{"dist": 4.0, "tag": "4m", "file": "person_4m.jpg", "h_lo": 0.18, "h_hi": 0.50},
	{"dist": 8.0, "tag": "8m", "file": "person_8m.jpg", "h_lo": 0.08, "h_hi": 0.25},
]

# ---------------------------------------------------------------- 结构判据
## 相机参数期望值（CameraSensor.gd 的 DEFAULT_* 与 step 硬性 1280×720）。
const WANT_SIZE := Vector2i(1280, 720)
const WANT_FOV := 70.0
const WANT_QUALITY := 0.8
## 人数（World.gd PERSONS 实测 2 个）。
const WANT_PERSONS := 2
## 摆位 / 朝向容差。
const POS_TOL := 1.0e-4
const YAW_TOL := 1.0e-4
## 人高判据（米，与 Person.gd HEIGHT_MIN_M / HEIGHT_MAX_M 同口径）。
const HEIGHT_MIN_M := 1.60
const HEIGHT_MAX_M := 1.85
## 脚底贴地容差（米）：AABB.position.y 应 ≈ 0。
const FEET_TOL := 1.0e-3

# ---------------------------------------------------------------- 图像判据（实测值 + 余量）
const MIN_BYTES := 10 * 1024        # step 硬性：单张 > 10 KB（实测 29~43 KiB）
const VAR_MIN := 0.002              # 灰度方差下限（实测 0.0093~0.0159）
const MEAN_MIN := 0.05              # 非全黑
const MEAN_MAX := 0.95              # 非全白
const SPREAD_MIN := 0.15            # max - min（实测 ≈ 0.996）
const WHITE_MAX := 0.50             # 近白像素占比上限（实测 ≤ 0.0001）
const DIFF_BYTE_MIN := 0.10         # 两两字节差异占比（实测 0.687~0.858）
const DIFF_MEAN_MIN := 0.002        # 两两灰度平均绝对差（实测 0.0051~0.0234；**不是 0.02**）
const DIFF_STEP := 8                # 灰度差抽样步长（像素）
## R7：相邻档的高度占比至少放大这么多倍（实测 2.11 / 2.06）。
const MONO_RATIO_MIN := 1.5
## 落盘时间容差（秒）：防拿旧图充数。
const FILE_FRESH_SLACK_S := 2

# ---------------------------------------------------------------- 状态
var _total: int = 0
var _failed: int = 0
var _skipped: int = 0

var _world: Node3D = null
var _cam: Node3D = null
var _person: Node3D = null
var _persons_root: Node3D = null
var _ws: Node = null
var _start_unix_s: int = 0
var _cam_ready: bool = false

## 逐张实测数据（抓拍后就地测量，投影类断言必须在抓拍之后才有意义 —— E26）。
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

	# ============================================================ ② 取人 + 摆相机
	_persons_root = _world.call("GetPersonsRoot") as Node3D
	if _persons_root != null:
		_person = _persons_root.get_node_or_null(PERSON_NAME) as Node3D
	_cam_ready = await _Build_Camera()
	if not _cam_ready:
		await _Teardown()
		_abort("载入/实例化 %s 失败（首次运行先跑 --import）" % CAMERA_SCENE)
		return

	# ============================================================ ③ S 组：结构（两档都跑）
	await _Check_Structural()

	# ============================================================ ④ R 组：真渲染（仅真渲染器）
	if _cam.Can_Capture():
		await _Run_Render()
	else:
		_skip(_Render_Labels())

	# ============================================================ ⑤ 释放 + 退出
	await _Teardown()
	_finish()


# ================================================================ 组场景

## 载 World.tscn：**add_child 之前**把自带的 WebSocketManager.autolisten 关掉。
## 为什么：World.tscn 的 Systems 下挂着 WebSocketManager（autolisten=true），一入树就 listen(9999)，
##   与 step「不连 WS」冲突。只写导出位、不改 .tscn、不 queue_free 该节点（那会打 "[ws] stopped" 噪音）。
## World._ready() 里会程序化 BuildPersons()，所以 person_01 入树即有（无需本脚本摆人）。
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


## 相机 = CameraSensor.tscn 实例直接挂 World 下（**不挂车**、不建 UGV），
## add_child 之后才 Configure（_ready 里已做一次 refs 兜底，之后再写参数最稳），
## 并预先摆到第一档机位，让 S7 的 Can_Capture() 在有渲染器时立刻为真。
func _Build_Camera() -> bool:
	var packed: PackedScene = load(CAMERA_SCENE) as PackedScene
	if packed == null:
		return false
	var node: Node = packed.instantiate()
	if node == null:
		return false
	_cam = node as Node3D
	if _cam == null:
		return false
	_world.add_child(_cam)
	_cam.Configure({
		"width": WANT_SIZE.x, "height": WANT_SIZE.y,
		"fov_deg": WANT_FOV, "quality": WANT_QUALITY,
	})
	await process_frame
	await process_frame
	_Place_Camera(SHOTS[0])
	await process_frame
	return _cam.is_inside_tree()


## 摆机位：机位在人的**正前方**（世界 +X 方向）dist 米、光心高 EYE_H，光轴对着人。
## 只在入树后写 global_*（树外读 global_* 会 ERROR）。
func _Place_Camera(shot: Dictionary) -> void:
	if _cam == null or not is_instance_valid(_cam):
		return
	var base: Vector3 = WorldTransform.PosToGodot(PERSON_WORLD)
	_cam.global_position = base + Vector3(float(shot["dist"]), EYE_H, 0.0)
	_cam.look_at(base + Vector3(0.0, EYE_H, 0.0), Vector3.UP)


# ================================================================ S 组：结构（恒跑）

## S1~S7：全部在**碰渲染器之前**能判的条件（headless / Xvfb 两档都跑）。
func _Check_Structural() -> void:
	# ---- S1 世界载入 + 入树 + WS 惰性
	var autolisten: bool = bool(_ws.get("autolisten")) if _ws != null else true
	var listening: bool = bool(_ws.call("IsListening")) if _ws != null else true
	_check("S1-world-loads",
		_world != null and _world.is_inside_tree()
			and _ws != null and not autolisten and not listening,
		"scene=%s inside_tree=%s ws_autolisten=%s IsListening=%s（不监听 9999）" % [
			WORLD_SCENE, str(_world != null and _world.is_inside_tree()),
			str(autolisten), str(listening)])

	# ---- S2 人在场：Persons 容器存在、名对、2 个孩子、person_01 在其中
	var count: int = _persons_root.get_child_count() if _persons_root != null else -1
	var has_person: bool = _person != null and is_instance_valid(_person)
	_check("S2-persons-present",
		_persons_root != null and _persons_root.name == StringName(WorldGd.PERSONS_NODE)
			and count == WANT_PERSONS and has_person,
		"root=%s count=%d(==%d) has_%s=%s" % [
			str(_persons_root.name) if _persons_root != null else "null",
			count, WANT_PERSONS, PERSON_NAME, str(has_person)])

	# ---- S3 人的位姿 + 身高 + 脚底贴地（与 World.gd PERSONS[0] 逐位对照）
	var spec: Dictionary = WorldGd.PERSONS[0]
	var spec_pos: Vector3 = spec["pos"]
	var spec_yaw: float = WorldTransform.YawToGodot(deg_to_rad(float(spec["yaw_deg"]))) + 0.0
	var want_pos: Vector3 = WorldTransform.PosToGodot(spec_pos)
	var pos_ok: bool = spec_pos.is_equal_approx(PERSON_WORLD) \
		and absf(float(spec["yaw_deg"]) - PERSON_YAW_DEG) < 1.0e-6 \
		and has_person and _person.global_position.distance_to(want_pos) < POS_TOL
	var yaw_ok: bool = has_person and absf(_person.rotation.y - spec_yaw) < YAW_TOL
	var box: AABB = _Aabb_Of(_person) if has_person else AABB()
	var feet_ok: bool = absf(box.position.y) < FEET_TOL
	var height_ok: bool = box.size.y >= HEIGHT_MIN_M and box.size.y <= HEIGHT_MAX_M
	_check("S3-person-pose", pos_ok and yaw_ok and feet_ok and height_ok,
		"pos=%s(期望 %s) rotation.y=%.6f(期望 %.6f) aabb.position.y=%.6f(<%.4f) aabb.size.y=%.4f(%.2f~%.2f)" % [
			str(_person.global_position) if has_person else "null", str(want_pos),
			_person.rotation.y if has_person else -1.0, spec_yaw,
			box.position.y, FEET_TOL, box.size.y, HEIGHT_MIN_M, HEIGHT_MAX_M])

	# ---- S4 相机场景 + 参数（1280×720 / fov 70 / quality 0.8）
	var params: Dictionary = _cam.Get_Params() if _cam != null else {}
	var image_size: Vector2i = _cam.Get_Image_Size() if _cam != null else Vector2i(-1, -1)
	_check("S4-camera-scene",
		_cam != null and _cam.is_inside_tree()
			and image_size == WANT_SIZE
			and is_equal_approx(float(params.get("fov_deg", -1.0)), WANT_FOV)
			and is_equal_approx(float(params.get("quality", -1.0)), WANT_QUALITY),
		"size=%s(期望 %s) fov=%.1f quality=%.2f" % [
			str(image_size), str(WANT_SIZE),
			float(params.get("fov_deg", -1.0)), float(params.get("quality", -1.0))])

	# ---- S5 机位表：三档距离不同、文件名 person_<d>m.jpg、互不相同、.jpg 结尾
	var seen: Array[String] = []
	var dists: Array[float] = []
	var reasons: Array[String] = []
	if SHOTS.size() != 3:
		reasons.append("count=%d" % SHOTS.size())
	for i in range(SHOTS.size()):
		var file: String = String(SHOTS[i]["file"])
		var dist: float = float(SHOTS[i]["dist"])
		var tag: String = String(SHOTS[i]["tag"])
		var want_file: String = "person_%s.jpg" % tag   # tag 自带 "m"（如 "2m"）
		if not file.begins_with("person_"):
			reasons.append("%d:no_prefix" % i)
		if not file.ends_with(".jpg"):
			reasons.append("%d:no_suffix" % i)
		if seen.has(file):
			reasons.append("%d:dup_file" % i)
		if dists.has(dist):
			reasons.append("%d:dup_dist" % i)
		if file != want_file:
			reasons.append("%d:file=%s!=%s" % [i, file, want_file])
		seen.append(file)
		dists.append(dist)
	_check("S5-shot-plan", reasons.is_empty(),
		"shots=%d dists=%s files=%s reasons=%s" % [
			SHOTS.size(), str(dists), str(seen),
			"none" if reasons.is_empty() else str(reasons)])

	# ---- S6 落盘目录：绝对路径 + 后缀对 + 不在工程目录 + 建得出来
	var absolute: bool = OUT_DIR.is_absolute_path()
	var suffix_ok: bool = OUT_DIR.ends_with(OUT_SUFFIX)
	var not_res: bool = not OUT_DIR.begins_with("res://")
	var made: bool = _Ensure_Out_Dir()
	_check("S6-out-dir",
		absolute and suffix_ok and not_res and made and DirAccess.dir_exists_absolute(OUT_DIR),
		"dir=%s absolute=%s suffix=%s not_res=%s exists=%s" % [
			OUT_DIR, str(absolute), str(suffix_ok), str(not_res),
			str(DirAccess.dir_exists_absolute(OUT_DIR))])

	# ---- S7 渲染器闸：Can_Capture() 必须与"有没有真显示"一致
	## 防 headless 悄悄跳过真问题（也防有人加了 --headless 还以为出了图）。
	var headless: bool = DisplayServer.get_name() == "headless"
	_check("S7-renderer-gate", _cam.Can_Capture() == (not headless),
		"can_capture=%s display=%s adapter='%s'" % [
			str(_cam.Can_Capture()), DisplayServer.get_name(),
			RenderingServer.get_video_adapter_name()])


# ================================================================ R 组：真渲染（仅 Xvfb 档）

## 三档拍照 + R1~R7（15 项）。只在 `_cam.Can_Capture()==true` 时调用。
func _Run_Render() -> void:
	# ① 逐张抓拍（投影类测量在 _Capture 内部于**抓拍之后**完成）
	for i in range(SHOTS.size()):
		await _Capture(i)
	# ② 逐张 R1~R4（读落盘文件，验证"交付物"本身）
	for i in range(SHOTS.size()):
		_Check_Shot(i)
	# ③ 全局 R5~R7
	_Check_Pairs()
	_Check_Bookkeeping()
	_Check_Monotonic()


## 单机位：摆位 -> 等 2 帧 -> 抓拍落盘 -> 就地测量（人体屏内包围框）。
## 返回 false = 抓拍或落盘失败（原因已由组件记在 Get_Last_Error()，本脚本不 push_error）。
func _Capture(idx: int) -> bool:
	var shot: Dictionary = SHOTS[idx]
	var tag: String = String(shot["tag"])
	var path: String = "%s/%s" % [OUT_DIR, String(shot["file"])]

	_Place_Camera(shot)
	await process_frame
	await process_frame

	var saved: bool = await _cam.Capture_To_File(path)

	# 测量（必须在抓拍之后：SubViewport 断了继承链，Lens.global_transform 只在抓拍内同步）
	var screen: Dictionary = _Person_Screen(_cam)
	_shots.append({
		"ok": saved,
		"tag": tag,
		"dist": float(shot["dist"]),
		"h_lo": float(shot["h_lo"]),
		"h_hi": float(shot["h_hi"]),
		"path": path,
		"bytes": _File_Size(path),
		"head": _File_Head(path),
		"front": int(screen["front"]),
		"inside": int(screen["inside"]),
		"lo": screen["lo"],
		"hi": screen["hi"],
		"cover": float(screen["cover"]),
		"h_frac": float(screen["h_frac"]),
		"person_y_lo": float(screen["person_y_lo"]),
		"person_y_hi": float(screen["person_y_hi"]),
		"jpg": FileAccess.get_file_as_bytes(path),
	})
	# 这一行是 yolo_report.md 的人工核对依据（浮点固定 3 位，格式钉死不要再改）。
	print("[check] shot %s person_screen front=%d inside=%d bbox=(%.1f,%.1f)-(%.1f,%.1f) cover=%.3f h_frac=%.3f" % [
		tag, int(screen["front"]), int(screen["inside"]),
		Vector2(screen["lo"]).x, Vector2(screen["lo"]).y,
		Vector2(screen["hi"]).x, Vector2(screen["hi"]).y,
		float(screen["cover"]), float(screen["h_frac"])])
	return saved


## 单张 R1~R4：对**落盘文件**断言（不是内存里的 Image）。
func _Check_Shot(idx: int) -> void:
	var data: Dictionary = _shots[idx]
	var tag: String = String(data["tag"])
	var path: String = String(data["path"])
	var bytes: int = int(data["bytes"])
	var head: String = String(data["head"])
	var jpg: PackedByteArray = data["jpg"]

	# ---- R1 文件存在 + 大小 + JPEG 魔数
	_check("R1-%s-file" % tag,
		bool(data["ok"]) and bytes > MIN_BYTES and head == "ffd8ff",
		"bytes=%d(>%d) head=%s saved=%s" % [bytes, MIN_BYTES, head, str(data["ok"])])

	# ---- R2 SOF0 尺寸 + 解码尺寸
	var sof: Vector2i = _Sof0_Size(jpg)
	var img: Image = _Decode(jpg)
	var img_size: Vector2i = img.get_size() if img != null and not img.is_empty() \
		else Vector2i(-1, -1)
	_check("R2-%s-size" % tag, sof == WANT_SIZE and img_size == WANT_SIZE,
		"sof0=%dx%d decoded=%dx%d want=%dx%d" % [
			sof.x, sof.y, img_size.x, img_size.y, WANT_SIZE.x, WANT_SIZE.y])
	if img == null or img.is_empty():
		_check("R3-%s-not-blank" % tag, false, "JPEG 解码失败：%s" % path)
		_check("R4-%s-person-in-frame" % tag, false, "JPEG 解码失败：%s" % path)
		return

	# ---- R3 非全黑 / 非全白
	var st: Dictionary = _Gray_Stats(img)
	var mean: float = float(st["mean"])
	var variance: float = float(st["var"])
	var spread: float = float(st["max"]) - float(st["min"])
	var white: float = float(st["white"])
	_check("R3-%s-not-blank" % tag,
		variance > VAR_MIN and mean > MEAN_MIN and mean < MEAN_MAX
			and spread > SPREAD_MIN and white < WHITE_MAX,
		"var=%.6f(>%.3f) mean=%.4f(%.2f~%.2f) spread=%.3f(>%.2f) white=%.3f(<%.2f)" % [
			variance, VAR_MIN, mean, MEAN_MIN, MEAN_MAX, spread, SPREAD_MIN,
			white, WHITE_MAX])

	# ---- R4 人真的在画面里：8 角点全在镜头前方 + 全落在框内 + 高度占比在实测带内
	var front: int = int(data["front"])
	var inside: int = int(data["inside"])
	var h_frac: float = float(data["h_frac"])
	var h_lo: float = float(data["h_lo"])
	var h_hi: float = float(data["h_hi"])
	var lo: Vector2 = data["lo"]
	var hi: Vector2 = data["hi"]
	_check("R4-%s-person-in-frame" % tag,
		front == 8 and inside == 8 and h_frac >= h_lo and h_frac <= h_hi,
		"front=%d/8 inside=%d/8 bbox=(%.1f,%.1f)-(%.1f,%.1f) h_frac=%.3f(%.2f~%.2f)" % [
			front, inside, lo.x, lo.y, hi.x, hi.y, h_frac, h_lo, h_hi])


## R5：三张两两不同（字节差异 + 灰度差异都要过）——"真的换了机位，不是同一张图复制三份"。
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
		details.append("%s~%s byte=%.4f(>%.2f) mean_abs=%.5f(>%.3f)" % [
			String(_shots[pair[0]]["tag"]), String(_shots[pair[1]]["tag"]),
			byte_ratio, DIFF_BYTE_MIN, mean_abs, DIFF_MEAN_MIN])
	_check("R5-shots-differ", ok, "；".join(details))


## R6：抓拍簿记（总次数 / 无错误 / 文件确实是本次写的，防拿旧图充数）。
func _Check_Bookkeeping() -> void:
	var count: int = int(_cam.Get_Capture_Count())
	var last_error: String = String(_cam.Get_Last_Error())
	var fresh: bool = true
	var stamps: Array[String] = []
	for i in range(_shots.size()):
		var path: String = String(_shots[i]["path"])
		var stamp: int = FileAccess.get_modified_time(path)
		if stamp < _start_unix_s - FILE_FRESH_SLACK_S:
			fresh = false
		stamps.append("%s@%d" % [String(_shots[i]["tag"]), stamp])
	_check("R6-capture-bookkeeping",
		count == SHOTS.size() and last_error.is_empty() and fresh,
		"captures=%d(==%d) last_error='%s' fresh=%s=%s" % [
			count, SHOTS.size(), last_error, str(fresh), str(stamps)])


## R7：三档距离是真的 —— 屏高占比必须严格单调下降，且相邻档至少放大 1.5 倍。
## 这一条替代"照抄 check_camera_shots 的 DIFF_MEAN_MIN=0.02"（4m~8m 实测只有 0.0051，会假红）。
func _Check_Monotonic() -> void:
	if _shots.size() != SHOTS.size():
		_check("R7-distance-monotonic", false, "shots=%d（期望 %d）" % [
			_shots.size(), SHOTS.size()])
		return
	var h0: float = float(_shots[0]["h_frac"])
	var h1: float = float(_shots[1]["h_frac"])
	var h2: float = float(_shots[2]["h_frac"])
	var ratio_01: float = h1 if h1 <= 0.0 else h0 / h1
	var ratio_12: float = h2 if h2 <= 0.0 else h1 / h2
	_check("R7-distance-monotonic",
		h0 > h1 and h1 > h2 and ratio_01 >= MONO_RATIO_MIN and ratio_12 >= MONO_RATIO_MIN,
		"h_frac %.3f > %.3f > %.3f；比值 2m/4m=%.2f(>=%.2f) 4m/8m=%.2f(>=%.2f)" % [
			h0, h1, h2, ratio_01, MONO_RATIO_MIN, ratio_12, MONO_RATIO_MIN])


## headless：R 组整体 SKIP（**不计入 total**，否则断言数会和基线错位）。
func _skip(labels: Array) -> void:
	for label in labels:
		print("[skip] %s（display=%s adapter='%s'：无真渲染器，--headless 是 dummy）" % [
			String(label), DisplayServer.get_name(),
			RenderingServer.get_video_adapter_name()])
		_skipped += 1


## R 组全部 label（15 项），与 _Check_Shot / 全局断言逐字同源，防两处漂移。
func _Render_Labels() -> Array:
	var labels: Array = []
	for i in range(SHOTS.size()):
		labels.append_array(_Shot_Labels(i))
	labels.append_array([
		"R5-shots-differ", "R6-capture-bookkeeping", "R7-distance-monotonic",
	])
	return labels


## 单张的 4 条 label（R1~R4）。
func _Shot_Labels(idx: int) -> Array:
	var tag: String = String(SHOTS[idx]["tag"])
	return ["R1-%s-file" % tag, "R2-%s-size" % tag, "R3-%s-not-blank" % tag,
		"R4-%s-person-in-frame" % tag]


# ================================================================ 测量：人体屏内包围框

## 人的世界 AABB 8 角点投到屏幕：包围框 + 屏内占比。
## 返回 `{front, inside, lo, hi, cover, h_frac, person_y_lo, person_y_hi}`。
## **必须在一次抓拍之后调用**：Lens.global_transform 只在 Capture_Jpeg() 内同步（E26）。
func _Person_Screen(cam: Node3D) -> Dictionary:
	var empty: Dictionary = {
		"front": -1, "inside": -1, "lo": Vector2.ZERO, "hi": Vector2.ZERO,
		"cover": -1.0, "h_frac": -1.0, "person_y_lo": -1.0, "person_y_hi": -1.0,
	}
	if cam == null or _person == null or not is_instance_valid(_person):
		return empty
	var lens: Camera3D = cam.get_node_or_null("Viewport/Lens") as Camera3D
	if lens == null:
		return empty
	var box: AABB = _Aabb_Of(_person)
	var lo := Vector2(1e9, 1e9)
	var hi := Vector2(-1e9, -1e9)
	var front: int = 0
	var inside: int = 0
	for i in range(8):
		var world_pos: Vector3 = box.get_endpoint(i)
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
		return empty
	var rect: Rect2 = Rect2(lo, hi - lo).intersection(
		Rect2(0.0, 0.0, float(WANT_SIZE.x), float(WANT_SIZE.y)))
	return {
		"front": front,
		"inside": inside,
		"lo": lo,
		"hi": hi,
		"cover": maxf(rect.size.x, 0.0) * maxf(rect.size.y, 0.0) \
			/ float(WANT_SIZE.x * WANT_SIZE.y),
		"h_frac": maxf(rect.size.y, 0.0) / float(WANT_SIZE.y),
		"person_y_lo": maxf(lo.y, 0.0),
		"person_y_hi": minf(hi.y, float(WANT_SIZE.y)),
	}


## 节点的世界 AABB：逐个 MeshInstance3D 取 mesh 局部 AABB、过 global_transform 后合并。
## 为什么不用 `get_aabb()`：那是 **mesh 局部** AABB（不含节点旋转/缩放），与 check_person_assets 同款取舍。
func _Aabb_Of(node: Node) -> AABB:
	var boxes: Array[AABB] = []
	_Gather_Boxes(node, boxes)
	if boxes.is_empty():
		return AABB()
	var out: AABB = boxes[0]
	for i in range(1, boxes.size()):
		out = out.merge(boxes[i])
	return out


func _Gather_Boxes(node: Node, boxes: Array[AABB]) -> void:
	if node == null:
		return
	if node is MeshInstance3D:
		var mi: MeshInstance3D = node
		if mi.mesh != null:
			var local: AABB = mi.get_aabb()
			var xf: Transform3D = mi.global_transform
			var box := AABB(xf * local.get_endpoint(0), Vector3.ZERO)
			for i in range(1, 8):
				box = box.expand(xf * local.get_endpoint(i))
			boxes.append(box)
	for child: Node in node.get_children():
		_Gather_Boxes(child, boxes)


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


# ================================================================ 工具：文件

## 建落盘目录（已存在 = 成功）；失败只返回 false、**绝不打印 ERROR**
## （`make_dir_recursive_absolute` 失败会走 ERR_FAIL_V_MSG 打 ERROR，故不用）。
func _Ensure_Out_Dir() -> bool:
	if OUT_DIR.is_empty() or not OUT_DIR.is_absolute_path():
		return false
	if DirAccess.dir_exists_absolute(OUT_DIR):
		return true
	DirAccess.make_dir_absolute(OUT_DIR)
	if not DirAccess.dir_exists_absolute(OUT_DIR):
		var parent: String = OUT_DIR.get_base_dir()
		if not parent.is_empty() and not DirAccess.dir_exists_absolute(parent):
			DirAccess.make_dir_absolute(parent)
		DirAccess.make_dir_absolute(OUT_DIR)
	return DirAccess.dir_exists_absolute(OUT_DIR)


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

## 释放顺序：相机 -> 世界，每步 await 两帧（漏了会报 `ERROR: resources still in use at exit`）。
func _Teardown() -> void:
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


## 前置条件失败（世界/相机载不进来）：记一条失败后直接收尾，不抛异常、不模拟"全绿"。
func _abort(reason: String) -> void:
	_failed += 1
	print("[check] ABORT %s" % reason)
	_finish()


func _finish() -> void:
	print("[check] env display=%s adapter='%s'" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])
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
