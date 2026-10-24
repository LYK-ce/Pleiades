# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-24
#
# 相机组件自检：A 组 18 项结构 / 参数 / 出帧断言 + B 组 10 项渲染断言。
#   task_33 Step 2 落地 A1~A12 + B1~B9；task_34 Step 3 追加 A13~A18（下行流 / 帧头 / 频率 /
#   无渲染器闸门 / 目录登记）与 B10（真抓拍 -> 真帧）。
#
# 跑法 ①（纯 headless，只跑 A 组；B 组全部 [skip] 且**不计入 total**）：
#   timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_camera_sensor.gd
#   期望：total=18 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR。
#
# 跑法 ②（**真渲染验收**：虚拟显示 + 软件 OpenGL，A+B 全跑）：
#   DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot \
#     --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
#     --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_camera_sensor.gd
#   期望：total=28 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR。
#
# 为什么必须分两档跑（实测事实，COMPONENTS.md §9.1 E26）：
#   ① `--headless` = `--display-driver headless`，它**只支持 dummy 渲染驱动**：
#      `RenderingServer.frame_post_draw` 永不触发、`get_texture().get_image()` 返回 null
#      并打 `ERROR: Parameter "t" is null`。=> headless 下"能出图"是**物理不可能**，
#      本脚本用 CameraSensor.Can_Capture() 在**碰渲染器之前**分流，B 组整体 SKIP。
#      同理 A 组的出帧断言只用**假 JPEG**（`FF D8 FF ...`）走 `Emit_Frame()` 真路径，
#      频率断言用**手工 Tick_Stream()**，一律不依赖真渲染（task_34 Step 3）。
#   ② 真渲染必须 `--audio-driver Dummy`：本机无声卡，ALSA 打不开会打一行
#      `ERROR: ... init_output_device`，把"0 ERROR"验收搞假红。
#   ③ 渲染信号只能 `await`，**绝不能 `connect()`**：连 Callable 会让进程退出时 SIGABRT（EXIT=134）。
#
# 注意（GDScript 4.7）：`load()` / `instantiate()` 的返回值一律显式标注类型，
#   否则 `var x := ...` 的类型推断会把 Variant 推断成具体类而编译报错。
#   退出前必须 `queue_free()` + await 两帧，否则报 `ERROR: resources still in use at exit`。

extends SceneTree

const WireProtocol := preload("res://src/net/WireProtocol.gd")

## ── 被测对象 ────────────────────────────────────────────────
const SCENE := "res://src/scenes/sensors/CameraSensor.tscn"
const SCRIPT_PATH := "res://src/sensors/CameraSensor.gd"
const WORLD_SCENE := "res://src/scenes/World.tscn"
const CATALOG_SCRIPT := "res://src/sensors/SensorCatalog.gd"

## ── task_34 Step 3：出帧口径 ────────────────────────────────
const CAMERA_SCENE_FROM_CATALOG := "res://src/scenes/sensors/CameraSensor.tscn"
## 目录默认参数（装配线口径，与组件常量 1280×720 是两回事，见 COMPONENTS.md §7.6）。
const CATALOG_CAMERA_DEFAULTS := {
	"width": 640, "height": 480, "fov_deg": 70.0, "quality": 0.8, "rate_hz": 1.0,
}
## `--fixed-fps 60` 下 60 步 = 引擎时间 1 s；频率断言一律用**手工推步**，不用墙钟。
const STEPS_PER_SECOND := 60

## ── 期望值（与 CameraSensor.gd 的 DEFAULT_* 双重断言）─────────
const WANT_SIZE := Vector2i(1280, 720)
const WANT_FOV := 70.0
const WANT_NEAR := 0.05
const WANT_FAR := 200.0
const WANT_QUALITY := 0.8
const ALT_SIZE := Vector2i(640, 360)
const ALT_FOV := 40.0
const ALT_FOV_B := 90.0

## ── 机位（世界中心上方 6 m、俯角 -20°：与规划期探针 probe3 同机位）──
const MOUNT_POS := Vector3(64.0, 6.0, 64.0)
const MOUNT_ROT_DEG := Vector3(-20.0, 0.0, 0.0)

## ── 图像判据（"非全黑"）────────────────────────────────────
const VAR_MIN := 0.002          # 灰度方差下限（实测 0.0187）
const MEAN_MIN := 0.05          # 灰度均值区间（实测 0.5658）
const MEAN_MAX := 0.95
const SPREAD_MIN := 0.15        # max - min 下限（实测 0.84）

## ── 落盘判据 ────────────────────────────────────────────────
const FILE_MIN_BYTES := 5000
const USER_DIR := "user://check_camera_sensor"
const BAD_PATH := "/proc/orion_no_such_dir/never.jpg"

## ── 白盒 lint（去掉整行注释后不得出现）────────────────────────
const BANNED_TOKENS := ["UPDATE_ALWAYS", "intersect_ray", "move_and_slide",
	"CollisionShape3D", "CollisionPolygon3D", "PhysicsBody3D"]

var _total: int = 0
var _failed: int = 0
var _skipped: int = 0
var _aborted: bool = false

var _cam: Node3D = null
var _world: Node3D = null
## A13~A16 共用的下行流替身（A14/A15 断言它收到的帧）。
var _stub: Variant = null


# ================================================================ 测试替身

## 帧下行替身（鸭子契约：`NextSeq` / `IsOpen` / `SendBinary`，与 `ComponentLink` 同名同义）。
## 只记帧、不入树、不碰 socket：本自检因此完全不需要真 WS 连接（task_34 Step 3）。
class StubLink:
	## 按到达顺序存下的二进制帧（PackedByteArray）。
	var frames: Array = []
	var seq: int = 0
	var open: bool = true

	func NextSeq() -> int:
		seq += 1
		return seq

	func IsOpen() -> bool:
		return open

	func SendBinary(bytes: PackedByteArray) -> Error:
		if not open:
			return ERR_UNCONFIGURED
		frames.append(bytes)
		return OK

	## 最后一帧；一帧都没有 -> 空数组。
	func Last_Frame() -> PackedByteArray:
		if frames.is_empty():
			return PackedByteArray()
		return frames[frames.size() - 1]


## 手工推 n 步（每步 delta = 1/60）：完全脱离引擎计时，headless / Xvfb 都确定（R4）。
## 为什么不用墙钟：`--fixed-fps 60` 固定 delta 但**不限制实际帧率**，墙钟毫秒在引擎时间里不可控。
func _Step_Ticks(cam: Node3D, steps: int) -> void:
	for _i in range(steps):
		cam.Tick_Stream(1.0 / 60.0)


func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ ⓪ 载入 + 入树
	var packed: PackedScene = load(SCENE) as PackedScene
	if not _check("A1-scene-loads", packed != null, SCENE):
		_finish()
		return
	var cam: Node3D = packed.instantiate() as Node3D
	if cam == null:
		_check("A2-root", false, "instantiate() 返回 null")
		_finish()
		return
	_cam = cam
	root.add_child(cam)
	await process_frame

	# ============================================================ ① A 组：结构 / 参数
	_Check_A2_Root(cam)
	_Check_A3_Viewport(cam)
	_Check_A4_Lens(cam)
	_Check_A5_NoCollision(cam)
	_Check_A6_ConfigureEmpty(cam)
	_Check_A7_ConfigureSize(cam)
	_Check_A8_ConfigurePartial(cam)
	_Check_A9_ConfigureInvalid(cam)
	_Check_A10_ConfigureNull(cam)
	_Check_A11_CanCapture(cam)
	_Check_A12_WhiteBoxLint()

	# ---- task_34 Step 3：下行流 / 帧格式 / 频率 / 无渲染器闸门 / 目录登记
	_Check_A13_StreamBind(cam)
	_Check_A14_FrameHeader(cam)
	_Check_A15_FrameSeqEmpty(cam)
	_Check_A16_RateHz(cam)
	_Check_A17_NoRenderGate()
	_Check_A18_CatalogCamera()

	# ============================================================ ② B 组：真渲染（仅真渲染器）
	if not _aborted:
		if cam.Can_Capture():
			await _Run_B(cam)
		else:
			_Skip_B()

	# ============================================================ ③ 释放 + 退出
	await _Teardown()
	_finish()


# ================================================================ A 组

## A2：根节点名 / 类型 / 脚本路径（按路径挂脚本，不用全局类名）。
func _Check_A2_Root(cam: Node3D) -> void:
	var script_obj: Script = cam.get_script() as Script
	var script_path: String = script_obj.resource_path if script_obj != null else ""
	_check("A2-root",
		cam.name == StringName("CameraSensor") and cam is Node3D and script_path == SCRIPT_PATH,
		"name=%s type=%s script=%s" % [cam.name, cam.get_class(), script_path])


## A3：Viewport 是 SubViewport、**共享 World3D**、尺寸 1280×720、**不每帧渲**。
func _Check_A3_Viewport(cam: Node3D) -> void:
	var sv: SubViewport = cam.get_node_or_null("Viewport") as SubViewport
	var ok: bool = sv != null and not sv.own_world_3d \
		and sv.size == WANT_SIZE \
		and sv.render_target_update_mode == SubViewport.UPDATE_DISABLED
	_check("A3-viewport-node", ok, "sub=%s own_world_3d=%s size=%s mode=%d" % [
		str(sv != null),
		str(sv.own_world_3d) if sv != null else "n/a",
		str(sv.size) if sv != null else "n/a",
		sv.render_target_update_mode if sv != null else -1])


## A4：镜头参数 + **必须挂在 SubViewport 下**（防抢 ObserverCamera 的 current）+ 光心在原点。
func _Check_A4_Lens(cam: Node3D) -> void:
	var lens: Camera3D = cam.get_node_or_null("Viewport/Lens") as Camera3D
	var parent_is_sub: bool = lens != null and lens.get_parent() is SubViewport
	var ok: bool = lens != null and is_equal_approx(lens.fov, WANT_FOV) \
		and is_equal_approx(lens.near, WANT_NEAR) and is_equal_approx(lens.far, WANT_FAR) \
		and lens.current and parent_is_sub and lens.position.is_equal_approx(Vector3.ZERO)
	_check("A4-lens-node", ok, "fov=%.2f near=%.3f far=%.1f current=%s parent_sub=%s pos=%s" % [
		lens.fov if lens != null else -1.0,
		lens.near if lens != null else -1.0,
		lens.far if lens != null else -1.0,
		str(lens.current) if lens != null else "n/a",
		str(parent_is_sub),
		str(lens.position) if lens != null else "n/a"])


## A5：子树里零碰撞体 / 零物理体（D8：传感器纯视觉）。
func _Check_A5_NoCollision(cam: Node3D) -> void:
	var n: int = _Count_Physics(cam)
	_check("A5-no-collision", n == 0, "physics_nodes=%d" % n)


## A6：`Configure({})` 不改任何参数（缺键 = 保持当前值）。
func _Check_A6_ConfigureEmpty(cam: Node3D) -> void:
	cam.Configure({})
	var lens: Camera3D = cam.get_node_or_null("Viewport/Lens") as Camera3D
	var params: Dictionary = cam.Get_Params()
	var ok: bool = cam.Get_Image_Size() == WANT_SIZE \
		and is_equal_approx(float(params["fov_deg"]), WANT_FOV) \
		and is_equal_approx(float(params["quality"]), WANT_QUALITY) \
		and lens != null and is_equal_approx(lens.fov, WANT_FOV)
	_check("A6-configure-empty", ok, "size=%s params=%s lens.fov=%.2f" % [
		str(cam.Get_Image_Size()), str(params), lens.fov if lens != null else -1.0])


## A7：`Configure({width, height})` 真的重设了 SubViewport.size。
func _Check_A7_ConfigureSize(cam: Node3D) -> void:
	cam.Configure({"width": ALT_SIZE.x, "height": ALT_SIZE.y})
	var sv: SubViewport = cam.get_node_or_null("Viewport") as SubViewport
	var ok: bool = cam.Get_Image_Size() == ALT_SIZE and sv != null and sv.size == ALT_SIZE
	_check("A7-configure-size", ok, "image_size=%s viewport.size=%s" % [
		str(cam.Get_Image_Size()), str(sv.size) if sv != null else "n/a"])


## A8：只改 fov_deg → 尺寸不动（缺键容错）。
func _Check_A8_ConfigurePartial(cam: Node3D) -> void:
	cam.Configure({"fov_deg": ALT_FOV_B})
	var lens: Camera3D = cam.get_node_or_null("Viewport/Lens") as Camera3D
	var ok: bool = lens != null and is_equal_approx(lens.fov, ALT_FOV_B) \
		and cam.Get_Image_Size() == ALT_SIZE
	_check("A8-configure-partial", ok, "lens.fov=%.2f size=%s（缺键不动）" % [
		lens.fov if lens != null else -1.0, str(cam.Get_Image_Size())])


## A9：键在但值非法 → 逐键回退**默认常量**（不是夹取、不是保留旧值）。
func _Check_A9_ConfigureInvalid(cam: Node3D) -> void:
	cam.Configure({"width": -5, "height": "x", "quality": 3.0, "fov_deg": 0})
	var lens: Camera3D = cam.get_node_or_null("Viewport/Lens") as Camera3D
	var params: Dictionary = cam.Get_Params()
	var ok: bool = cam.Get_Image_Size() == WANT_SIZE \
		and is_equal_approx(float(params["quality"]), WANT_QUALITY) \
		and is_equal_approx(float(params["fov_deg"]), WANT_FOV) \
		and lens != null and is_equal_approx(lens.fov, WANT_FOV)
	_check("A9-configure-invalid", ok, "size=%s quality=%.2f fov=%.2f" % [
		str(cam.Get_Image_Size()), float(params["quality"]), float(params["fov_deg"])])


## A10：`Configure(null)` 不崩、参数保持当前值。
## 组件形参写的是 `Variant`（不是 `Dictionary`）——形参若是 `Dictionary`，
##   `Configure(null)` 会在**编译期**报 `Cannot pass a value of type "null" as "Dictionary"`（实测）。
func _Check_A10_ConfigureNull(cam: Node3D) -> void:
	var before: Dictionary = cam.Get_Params()
	cam.Configure(null)
	var after: Dictionary = cam.Get_Params()
	_check("A10-configure-null", before == after, "before=%s after=%s" % [str(before), str(after)])


## A11：`Can_Capture()` 与"是否有真显示 / 真渲染器"一致（headless 必须 false）。
func _Check_A11_CanCapture(cam: Node3D) -> void:
	var headless: bool = DisplayServer.get_name() == "headless"
	var adapter: String = RenderingServer.get_video_adapter_name()
	_check("A11-can-capture", cam.Can_Capture() == (not headless),
		"can_capture=%s display=%s adapter='%s'" % [
			str(cam.Can_Capture()), DisplayServer.get_name(), adapter])


## A12：源码白盒 lint —— 去掉**整行**注释后不得出现每帧渲 / 射线 / 物理 / 碰撞体符号。
func _Check_A12_WhiteBoxLint() -> void:
	var src: String = _Read_Text(SCRIPT_PATH)
	if src.is_empty():
		_check("A12-no-physics-lint", false, "读不到 %s" % SCRIPT_PATH)
		return
	var code: String = _Strip_Line_Comments(src)
	var hits: Array[String] = []
	for token in BANNED_TOKENS:
		if code.contains(token):
			hits.append(token)
	_check("A12-no-physics-lint", hits.is_empty(),
		"banned=%s" % ("none" if hits.is_empty() else str(hits)))


## A13：`SetStream` / `ClearStream` / `HasStream` 的绑解绑语义（与 Lidar2D 同名同义）。
## 绑完后立刻 `set_process(false)`：后续所有计时都靠手工推步，避免引擎 `_process` 污染（R6）。
func _Check_A13_StreamBind(cam: Node3D) -> void:
	var link := StubLink.new()
	cam.SetStream(link, 7)
	var bound: bool = cam.HasStream() and int(cam._sensor_id) == 7
	cam.ClearStream()
	var cleared: bool = not cam.HasStream()
	cam.SetStream(link, 7)
	var rebound: bool = cam.HasStream() and int(cam._sensor_id) == 7
	cam.set_process(false)
	_stub = link
	_check("A13-stream-bind", bound and cleared and rebound,
		"has=%s cleared=%s sensor_id=%d" % [str(bound), str(cleared), int(cam._sensor_id)])


## A14：`Emit_Frame()` 出的帧头字段（type / slot_id / seq / world_time）+ payload 原样 + JPEG 魔数。
## 用假 JPEG（首三字节 `FF D8 FF`）走**真路径**：帧头与 payload 拼装是同一个 `BuildCameraFrame()`。
func _Check_A14_FrameHeader(cam: Node3D) -> void:
	if _stub == null:
		_check("A14-frame-header", false, "A13 未建立 StubLink")
		return
	var link: Variant = _stub
	var fake := PackedByteArray([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46])
	var sent: bool = cam.Emit_Frame(fake)
	var frame: PackedByteArray = link.Last_Frame()
	var header: Dictionary = WireProtocol.ParseFrameHeader(frame)
	var world_time: float = float(header.get("world_time", -1.0))
	var ok: bool = sent and frame.size() == WireProtocol.FRAME_HEADER_SIZE + fake.size() \
		and int(header.get("type", -1)) == WireProtocol.FRAME_TYPE_CAMERA \
		and int(header.get("slot_id", -1)) == 7 \
		and int(header.get("seq", -1)) == 1 \
		and is_finite(world_time) and world_time >= 0.0 \
		and frame.slice(WireProtocol.FRAME_HEADER_SIZE, 18).hex_encode() == "ffd8ff" \
		and frame.slice(WireProtocol.FRAME_HEADER_SIZE) == fake \
		and link.frames.size() == 1
	_check("A14-frame-header", ok, "size=%d type=%d slot=%d seq=%d" % [
		frame.size(), int(header.get("type", -1)), int(header.get("slot_id", -1)),
		int(header.get("seq", -1))])


## A15：seq 递增 + 字节跟着变 + **空图不出帧**（`Emit_Frame(空)` 返回 false 且不追加、不递增 seq）。
func _Check_A15_FrameSeqEmpty(cam: Node3D) -> void:
	if _stub == null:
		_check("A15-frame-seq-empty", false, "A13 未建立 StubLink")
		return
	var link: Variant = _stub
	var fake2 := PackedByteArray([0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00])
	var sent_second: bool = cam.Emit_Frame(fake2)
	var frame2: PackedByteArray = link.Last_Frame()
	var header2: Dictionary = WireProtocol.ParseFrameHeader(frame2)
	var frames_before: int = link.frames.size()
	var seq_before: int = int(link.seq)
	var sent_empty: bool = cam.Emit_Frame(PackedByteArray())
	var frames_after: int = link.frames.size()
	var seq_after: int = int(link.seq)
	_check("A15-frame-seq-empty",
		sent_second and int(header2.get("seq", -1)) == 2
		and frame2.size() == WireProtocol.FRAME_HEADER_SIZE + fake2.size()
		and frames_before == 2
		and not sent_empty and frames_after == frames_before and seq_after == seq_before,
		"seq=%d empty=%s frames=%d" % [
			int(header2.get("seq", -1)), str(sent_empty), frames_after])


## A16：`rate_hz` 的出帧节奏（1 Hz 每 60 步 1 次、2 Hz 每 60 步 2 次）+ 参数往返。
## 全部手工推步：引擎帧与墙钟都不参与判定（R4）。
func _Check_A16_RateHz(cam: Node3D) -> void:
	var rate_ok: bool = is_equal_approx(float(cam.Get_Params().get("rate_hz", -1.0)), 1.0)
	var tick0: int = cam.Get_Tick_Count()
	_Step_Ticks(cam, STEPS_PER_SECOND)
	var one_hz: int = cam.Get_Tick_Count() - tick0
	_Step_Ticks(cam, STEPS_PER_SECOND)
	var second_period: int = cam.Get_Tick_Count() - tick0 - one_hz
	cam.Configure({"rate_hz": 2.0})
	_Step_Ticks(cam, STEPS_PER_SECOND)
	var two_hz: int = cam.Get_Tick_Count() - tick0 - one_hz - second_period
	cam.Configure({"rate_hz": 1.0})
	var restored: bool = is_equal_approx(float(cam.Get_Params().get("rate_hz", -1.0)), 1.0)
	_check("A16-rate-hz", rate_ok and one_hz == 1 and second_period == 1
		and two_hz == 2 and restored,
		"1hz#1=%d/1 1hz#2=%d/1 2hz=%d/2 restored=%s" % [
			one_hz, second_period, two_hz, str(restored)])


## A17：无渲染器闸门 —— `Can_Capture()==false` 时**只告警一次、绝不 await、绝不出帧**，
## 但"到点"照常计数（两条要求互不掩盖）。用**树外**实例：`Can_Capture()` 必为 false，
## headless / Xvfb 两档一致（否则 Xvfb 下会真的去抓拍，把 headless 语义测丢）。
func _Check_A17_NoRenderGate() -> void:
	var packed: PackedScene = load(SCENE) as PackedScene
	if packed == null:
		_check("A17-no-render-gate", false, "场景加载失败")
		return
	var solo: Node3D = packed.instantiate() as Node3D
	if solo == null:
		_check("A17-no-render-gate", false, "instantiate() 返回 null")
		return
	var link := StubLink.new()
	solo.SetStream(link, 3)
	var can: bool = solo.Can_Capture()
	var tick0: int = solo.Get_Tick_Count()
	_Step_Ticks(solo, STEPS_PER_SECOND)
	var ticks: int = solo.Get_Tick_Count() - tick0
	var frames_first: int = link.frames.size()
	var warned: bool = bool(solo._warned_no_render)
	var last_err: String = solo.Get_Last_Error()
	_Step_Ticks(solo, STEPS_PER_SECOND)
	var frames_second: int = link.frames.size()
	solo.free()                       # 树外实例：没有树替它回收，直接 free（队列无用）
	_check("A17-no-render-gate",
		(not can) and ticks == 1 and frames_first == 0 and warned
		and last_err.is_empty() and frames_second == 0,
		"can=%s ticks=%d frames=%d warned=%s last_err='%s'" % [
			str(can), ticks, frames_second, str(warned), last_err])


## A18：`SensorCatalog` 登记了 `camera`（场景路径 + 5 键默认参数逐键比对，防 int/float 混淆）。
func _Check_A18_CatalogCamera() -> void:
	var catalog: Script = load(CATALOG_SCRIPT) as Script
	if catalog == null:
		_check("A18-catalog-camera", false, "读不到 %s" % CATALOG_SCRIPT)
		return
	var has_camera: bool = bool(catalog.call("Has", "camera"))
	var scene_from_catalog: Variant = catalog.call("GetScene", "camera")
	var scene_path: String = ""
	if scene_from_catalog is PackedScene:
		scene_path = (scene_from_catalog as PackedScene).resource_path
	var defaults_variant: Variant = catalog.call("GetDefaults", "camera")
	var defaults: Dictionary = defaults_variant if defaults_variant is Dictionary else {}
	var keys_ok: bool = defaults.size() == CATALOG_CAMERA_DEFAULTS.size()
	for key: Variant in CATALOG_CAMERA_DEFAULTS.keys():
		if not defaults.has(key):
			keys_ok = false
			continue
		if typeof(defaults[key]) != typeof(CATALOG_CAMERA_DEFAULTS[key]) \
				or defaults[key] != CATALOG_CAMERA_DEFAULTS[key]:
			keys_ok = false
	_check("A18-catalog-camera",
		has_camera and scene_path == CAMERA_SCENE_FROM_CATALOG and keys_ok,
		"has=%s scene=%s defaults=%s" % [str(has_camera), scene_path, str(defaults)])


# ================================================================ B 组（真渲染）

## B1~B9：全部走真实 World.tscn + 真实 SubViewport 渲染。
func _Run_B(cam: Node3D) -> void:
	# 需要世界才有"非全黑"的画面
	var world_packed: PackedScene = load(WORLD_SCENE) as PackedScene
	_world = world_packed.instantiate() as Node3D
	root.add_child(_world)
	cam.position = MOUNT_POS
	cam.rotation_degrees = MOUNT_ROT_DEG
	await process_frame
	await process_frame

	# ---- B1 非空 + JPEG 魔数
	cam.Configure({"width": WANT_SIZE.x, "height": WANT_SIZE.y, "fov_deg": WANT_FOV,
		"quality": WANT_QUALITY})
	var jpg: PackedByteArray = await cam.Capture_Jpeg()
	var head: String = jpg.slice(0, 3).hex_encode()
	_check("B1-capture-nonempty", jpg.size() > 0 and head == "ffd8ff",
		"bytes=%d head=%s err='%s'" % [jpg.size(), head, cam.Get_Last_Error()])
	if jpg.is_empty():
		_Skip_B(1)
		return

	# ---- B2 SOF0 尺寸
	var sof0: Vector2i = _Parse_Sof0(jpg)
	_check("B2-sof0-size", sof0 == WANT_SIZE, "sof0=%dx%d want=%dx%d" % [
		sof0.x, sof0.y, WANT_SIZE.x, WANT_SIZE.y])

	# ---- B3 非全黑（对**交付物 JPEG** 解码后统计，而不是对内部 Image）
	var img: Image = _Decode_Jpg(jpg)
	if img == null or img.is_empty():
		_check("B3-not-black", false, "JPEG 解码失败")
	else:
		var stat: Dictionary = _Gray_Stats(img)
		var mean: float = float(stat["mean"])
		var variance: float = float(stat["var"])
		var spread: float = float(stat["max"]) - float(stat["min"])
		_check("B3-not-black", variance > VAR_MIN and mean > MEAN_MIN and mean < MEAN_MAX
			and spread > SPREAD_MIN,
			"var=%.6f mean=%.4f spread=%.3f（阈值 var>%.3f / %.2f<mean<%.2f / spread>%.2f）" % [
				variance, mean, spread, VAR_MIN, MEAN_MIN, MEAN_MAX, SPREAD_MIN])

	# ---- B4 改 fov 后同机位成图不同
	cam.Configure({"fov_deg": ALT_FOV})
	var jpg_fov: PackedByteArray = await cam.Capture_Jpeg()
	_check("B4-fov-changes-image", jpg_fov.size() > 0 and jpg_fov != jpg,
		"fov=%.0f bytes=%d vs fov=%.0f bytes=%d same=%s" % [
			ALT_FOV, jpg_fov.size(), WANT_FOV, jpg.size(), str(jpg_fov == jpg)])

	# ---- B5 连拍稳定性：断言"每次都能出图"；逐字节是否相同**只记录、不判定**
	#         （渲染器/驱动可能有随机抖动，把它做成 FAIL 会造成假红）
	var jpg_again: PackedByteArray = await cam.Capture_Jpeg()
	_check("B5-determinism", jpg_again.size() > 0,
		"bytes=%d same_as_prev=%s（逐字节相同仅记录，不参与判定）" % [
			jpg_again.size(), str(jpg_again == jpg_fov)])

	# ---- B6 落盘
	var out_path: String = ProjectSettings.globalize_path(USER_DIR + "/cap_check.jpg")
	var saved: bool = await cam.Capture_To_File(out_path)
	var file_bytes: int = _File_Size(out_path)
	var file_head: String = _File_Head(out_path)
	_check("B6-capture-to-file",
		saved and file_bytes > FILE_MIN_BYTES and file_head == "ffd8ff",
		"ok=%s path=%s bytes=%d head=%s" % [str(saved), out_path, file_bytes, file_head])

	# ---- B7 坏路径：返回 false、不崩、**不打印 ERROR**（只用 push_warning）
	var bad_ok: bool = await cam.Capture_To_File(BAD_PATH)
	_check("B7-capture-to-file-bad-path", not bad_ok, "path=%s returned=%s" % [
		BAD_PATH, str(bad_ok)])

	# ---- B8 Configure 的分辨率真的生效（SOF0 跟着变）
	cam.Configure({"width": ALT_SIZE.x, "height": ALT_SIZE.y})
	var jpg_small: PackedByteArray = await cam.Capture_Jpeg()
	var sof0_small: Vector2i = _Parse_Sof0(jpg_small)
	_check("B8-resolution-applied", sof0_small == ALT_SIZE, "sof0=%dx%d want=%dx%d bytes=%d" % [
		sof0_small.x, sof0_small.y, ALT_SIZE.x, ALT_SIZE.y, jpg_small.size()])

	# ---- B9 抓拍后更新模式仍是 UPDATE_ONCE（没有偷偷变 ALWAYS）
	var sv: SubViewport = cam.get_node_or_null("Viewport") as SubViewport
	var mode: int = sv.render_target_update_mode if sv != null else -1
	_check("B9-update-mode-restored", mode == SubViewport.UPDATE_ONCE,
		"mode=%d（UPDATE_ONCE=%d，UPDATE_ALWAYS=%d 禁止）" % [
			mode, SubViewport.UPDATE_ONCE, SubViewport.UPDATE_ALWAYS])

	# ---- B10（task_34 Step 3）：真抓拍 -> **真帧**（帧头 + 整段 JPEG 原样 + SOF0 = 当前成图尺寸）
	var jpg_frame: PackedByteArray = await cam.Capture_Jpeg()
	var sent: bool = cam.Emit_Frame(jpg_frame)
	var whole: PackedByteArray = PackedByteArray()
	if _stub != null:
		whole = _stub.Last_Frame()
	var payload: PackedByteArray = whole.slice(WireProtocol.FRAME_HEADER_SIZE)
	var frame_header: Dictionary = WireProtocol.ParseFrameHeader(whole)
	var sof0_payload: Vector2i = _Parse_Sof0(payload)
	var want_size: Vector2i = cam.Get_Image_Size()
	_check("B10-frame-from-real-capture",
		not jpg_frame.is_empty() and sent
		and whole.size() == WireProtocol.FRAME_HEADER_SIZE + jpg_frame.size()
		and payload.slice(0, 3).hex_encode() == "ffd8ff"
		and sof0_payload == want_size
		and int(frame_header.get("type", -1)) == WireProtocol.FRAME_TYPE_CAMERA,
		"bytes=%d+%d=%d sof0=%dx%d want=%dx%d type=%d" % [
			WireProtocol.FRAME_HEADER_SIZE, jpg_frame.size(), whole.size(),
			sof0_payload.x, sof0_payload.y, want_size.x, want_size.y,
			int(frame_header.get("type", -1))])

	print("[check] B-summary captures=%d frames=%d last_error='%s'" % [
		cam.Get_Capture_Count(), cam.Get_Frame_Count(), cam.Get_Last_Error()])


## headless：B 组整体 SKIP（**不计入 total**，否则断言数会和基线错位）。
func _Skip_B(from_index: int = 0) -> void:
	var labels: Array[String] = [
		"B1-capture-nonempty", "B2-sof0-size", "B3-not-black", "B4-fov-changes-image",
		"B5-determinism", "B6-capture-to-file", "B7-capture-to-file-bad-path",
		"B8-resolution-applied", "B9-update-mode-restored",
		"B10-frame-from-real-capture",
	]
	for i in range(from_index, labels.size()):
		print("[skip] %s（display=%s adapter='%s'：无真渲染器，--headless 是 dummy）" % [
			labels[i], DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])
		_skipped += 1


# ================================================================ 工具：图像 / JPEG

## 解析 JPEG 的 SOF0 段拿宽高（`FFC0`；`C4/C8/CC` 是 DHT/JPG/DAC，不是 SOF）。
## 返回 Vector2i(-1, -1) = 没找到（不是合法 JPEG）。
func _Parse_Sof0(jpg: PackedByteArray) -> Vector2i:
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


func _Decode_Jpg(jpg: PackedByteArray) -> Image:
	var img := Image.new()
	var err: Error = img.load_jpg_from_buffer(jpg)
	if err != OK:
		return null
	return img


## 灰度统计（2 像素步长抽样：1280×720 -> 23 万点，够稳且快）。
func _Gray_Stats(img: Image) -> Dictionary:
	var sum: float = 0.0
	var sum2: float = 0.0
	var count: int = 0
	var lo: float = 1.0
	var hi: float = 0.0
	for y in range(0, img.get_height(), 2):
		for x in range(0, img.get_width(), 2):
			var l: float = _Lum(img.get_pixel(x, y))
			sum += l
			sum2 += l * l
			lo = minf(lo, l)
			hi = maxf(hi, l)
			count += 1
	if count == 0:
		return {"mean": -1.0, "var": -1.0, "min": -1.0, "max": -1.0}
	var mean: float = sum / float(count)
	return {"mean": mean, "var": maxf(sum2 / float(count) - mean * mean, 0.0),
		"min": lo, "max": hi}


func _Lum(c: Color) -> float:
	return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b


# ================================================================ 工具：文件 / 节点

func _Count_Physics(node: Node) -> int:
	var n: int = 0
	if node is CollisionShape3D or node is CollisionPolygon3D \
		or node is PhysicsBody3D or node is Area3D:
		n += 1
	for child in node.get_children():
		n += _Count_Physics(child)
	return n


func _Read_Text(path: String) -> String:
	if not FileAccess.file_exists(path):
		return ""
	var handle: FileAccess = FileAccess.open(path, FileAccess.READ)
	if handle == null:
		return ""
	var text: String = handle.get_as_text()
	handle.close()
	return text


## 只剥**整行**注释（# 或 ## 开头，允许前导空白）：行尾代码里的 # 不剥（与 check_quadrotor 同款口径）。
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


# ================================================================ 工具：断言 / 收尾

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


## R9：退出前必须 queue_free + await 两帧，否则 `ERROR: resources still in use at exit`。
func _Teardown() -> void:
	if _cam != null and is_instance_valid(_cam):
		_cam.queue_free()
		await process_frame
		await process_frame
	if _world != null and is_instance_valid(_world):
		_world.queue_free()
		await process_frame
		await process_frame


func _finish() -> void:
	print("[check] env display=%s adapter='%s' renders=%s" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name(),
		str(DisplayServer.get_name() != "headless")])
	if _skipped > 0:
		print("[check] skipped=%d（B 组：无真渲染器，不计入 total）" % _skipped)
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
