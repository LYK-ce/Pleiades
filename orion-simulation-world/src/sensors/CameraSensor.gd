# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-24
#
# 传感器组件 camera（task_33 Step 2 / task_34 Step 3 / COMPONENTS.md §4.8）：
#   离屏渲染一台相机 —— 给一个挂载位姿，它渲染"当前对着的画面"并交出一段 JPEG 字节；
#   task_34 Step 3 起它同时是**可装配零件**：按 `rate_hz` 把每张图组 `type=2` 帧下发给 sim。
#
# 节点形态（.tscn 见 src/scenes/sensors/CameraSensor.tscn）：
#   CameraSensor (Node3D)            ← 挂载点 = 光心（D8）；position / rotation 由挂载方写
#   └── Viewport (SubViewport)       ← 1280×720、own_world_3d=false、update_mode=DISABLED
#       └── Lens (Camera3D)          ← fov=70 / near=0.05 / far=200 / current=true
#
# 做什么 / 不做什么：
#   - 两种用法，同一个抓拍内核：
#     ① **按需单拍**（task_33 的交付）：`Capture_Jpeg()` / `Capture_To_File()` 调一次拍一张；
#     ② **按频率出帧**（task_34 Step 3）：`SetStream()` 绑流后 `_process` 里的累加器按 `rate_hz`
#        到点 -> `Capture_Jpeg()` -> `WireProtocol.BuildCameraFrame()` -> `link.SendBinary()`。
#   - **不做** 请求 / 应答（无 `camera.capture()`）、**不做** 队列积压、**不碰 socket**
#     （只鸭子调用 link 的 NextSeq / IsOpen / SendBinary）。
#   - **不在脚本里建模型**（`Body` 是 `.tscn` 里的静态 glb 实例，task_33 Step 3）、无碰撞体（D8：传感器纯视觉）。
#   - 不写 position / 不推物理：外参由 Vehicle.AttachSensor() 写进来。
#
# 三条实测事实（4.7.2 + 本机 llvmpipe，违反任一条都会静默出错 → COMPONENTS.md §9.1 E26）：
#   ① `--headless` 是 **dummy 渲染器**：`frame_post_draw` 永不触发、`get_texture().get_image()`
#      返回 null 并打 `ERROR: Parameter "t" is null`。所以**先过渲染器闸再 await**，否则永久挂住。
#   ② `SubViewport` **不是 Node3D**：`Lens` 的 `global_transform` **不继承**本节点的
#      position / rotation（实测：本节点在 (64,6,64)、Lens 的 global 仍是 IDENTITY）。
#      所以每次抓拍前必须 `_Sync_View_Transform()` 把光心世界变换写进镜头，
#      否则拍到的永远是"世界原点的画面"（本机表现为对着地平面以下的空世界）。
#   ③ `RenderingServer.frame_post_draw` **只能 `await`、绝不能 `connect()`**：
#      连 Callable 会让进程退出时 SIGABRT（EXIT=134）。
#
# 不声明全局类名（§9.1 E5）：场景按路径挂脚本。

extends Node3D

const WorldClock := preload("res://src/common/WorldClock.gd")
const WireProtocol := preload("res://src/net/WireProtocol.gd")

const TAG := "[camera]"

# ---------------------------------------------------------------- 默认值（唯一来源）
## 默认分辨率（step 硬性 1280×720；.tscn 只是初始声明，以本文件常量为准）。
const DEFAULT_WIDTH := 1280
const DEFAULT_HEIGHT := 720
## 默认水平视场角（度）。
const DEFAULT_FOV_DEG := 70.0
## 默认 JPEG 质量（0~1，Godot 的 save_jpg 口径）。
const DEFAULT_QUALITY := 0.8
## 默认近 / 远裁剪面（米）：near 0.05 让"贴车身也不穿模"（Step 4 车鼻占比断言依赖它）。
const DEFAULT_NEAR := 0.05
const DEFAULT_FAR := 200.0

## 分辨率边界：下限防 0/负数把 SubViewport 打崩，上限防一次申请 8K×8K 显存。
const MIN_SIZE := 16
const MAX_SIZE := 4096
## FOV 边界（度）：0 与 180 都是退化值。
const MIN_FOV_DEG := 1.0
const MAX_FOV_DEG := 179.0
## JPEG 质量边界：`save_jpg` 只接受 (0, 1]，越界一律回退默认值（不是夹取）。
const MIN_QUALITY := 0.01
const MAX_QUALITY := 1.0

## warmup 帧数：前 2 帧是黑的（step 明令）；改分辨率后要重新 warmup。
const WARMUP_FRAMES := 2

# ---------------------------------------------------------------- 出帧节奏（task_34 Step 3）
## 出帧频率默认值（Hz）：task_34 §二 Step 3 的 1 Hz（UAV 相机口径）。
const DEFAULT_RATE_HZ := 1.0
## 频率下限（Hz）：防 1/0。`rate_hz` 由 Configure 夹到这里，绝不出现 NaN / INF。
const MIN_RATE := 1e-3
## 累加器容差（与 Lidar2D 同款 1e-6）：60 × 1/60 = 0.9999999999999999 < 1.0，
##   不加容差 1 Hz 会一个整周期只出 0 帧。
const ACCUM_EPS := 1e-6

## 一张图组好并（可能）下发后发出：seq / world_time / JPEG 字节数。
## 有 link 但未 OPEN 时**也发**（与 Lidar2D 的 scan_produced 同口径，自检可观察）。
## 传字节数而不是 PackedByteArray：避免每帧复制大数组 / 避免信号持有大缓冲引用。
signal frame_produced(seq: int, world_time: float, size_bytes: int)

# ---------------------------------------------------------------- 节点引用（E16：靠 node_paths 绑定）
## 离屏渲染目标（SubViewport）。漏写 node_paths 会静默为 null → _Ensure_Refs() 兜底。
@export var viewport: SubViewport = null
## 镜头（Camera3D），必须是 Viewport 的子节点（绝不放根视口，否则抢 ObserverCamera 的 current）。
@export var lens: Camera3D = null

# ---------------------------------------------------------------- 生效参数（Configure 后）
## 成图宽度（像素）。
var width: int = DEFAULT_WIDTH
## 成图高度（像素）。
var height: int = DEFAULT_HEIGHT
## 水平视场角（度）。
var fov_deg: float = DEFAULT_FOV_DEG
## JPEG 质量。
var quality: float = DEFAULT_QUALITY
## 出帧频率（Hz）：`SetStream` 之后由累加器按它到点出帧（task_34 Step 3）。
var rate_hz: float = DEFAULT_RATE_HZ

# ---------------------------------------------------------------- 内部状态
var _warmed: bool = false          # warmup 是否已过（Configure 后置回 false）
var _warned_params: bool = false   # 非法参数只告警一次（与 Lidar2D 同款 F1）
var _capture_count: int = 0        # 成功出图次数（自检 / Step 4 可观察）
var _last_error: String = ""       # 最近一次失败原因（空 = 没失败过）

# ---- 出帧状态（task_34 Step 3）
var _accum: float = 0.0            # 时间累加器（秒）；SetStream / Configure 时清零
var _tick_count: int = 0           # "到点"次数（不区分是否真的出了图；自检用）
var _frame_count: int = 0          # 真正 SendBinary 成功的帧数（自检 / 排障用）
var _capturing: bool = false       # 抓拍互斥：上一张没回来时到点也不叠拍（R13）
var _warned_no_render: bool = false  # 无渲染器只告警一次（E26-③，避免每帧刷屏）
var _link: Variant = null          # ComponentLink（RefCounted，鸭子契约：NextSeq/IsOpen/SendBinary）
var _sensor_id: int = 0            # u16 语义，写进帧头 slot_id
var _seq: int = 0                  # 无 link 时的内部序号（仅 signal 可观察）


# ================================================================ 生命周期

## 只做引用兜底 + 把参数应用到节点：**不重置参数**（AttachSensor 先 add_child 后 Configure）。
func _ready() -> void:
	_Ensure_Refs()
	_Apply_To_Nodes()


## 每帧一次：只把 delta 交给累加器（引擎回调里**不阻塞、绝不 await**）。
## 未绑流时 `Tick_Stream()` 首行直接 return -> 全工程既有的相机实例零开销（R6）。
func _process(delta: float) -> void:
	Tick_Stream(delta)


## 出树即断流：manager 释放 ComponentLink（RefCounted）后本节点可能还在树里存活一帧（R11）。
func _exit_tree() -> void:
	_link = null
	_capturing = false


# ================================================================ 装配接口

## 装配参数：width / height / fov_deg / quality / rate_hz。
##   缺键 = 保持当前值；键在但值非法 = 回退**默认常量**并 push_warning 一次（F1）。
## 签名说明：形参写 `Variant` 而不是 `Dictionary`，因为 `Configure(null)` 若形参是
##   `Dictionary` 会**编译期报错**（`Cannot pass a value of type "null" as "Dictionary"`，
##   实测），而 task 文档要求缺键 / 空字典容错、自检 A10 直接传 null。鸭子契约不受影响。
func Configure(params: Variant = null) -> void:
	var p: Dictionary = params if params is Dictionary else {}
	if p.has("width"):
		width = _Size_Or(p["width"], DEFAULT_WIDTH)
	if p.has("height"):
		height = _Size_Or(p["height"], DEFAULT_HEIGHT)
	if p.has("fov_deg"):
		fov_deg = _Fov_Or(p["fov_deg"], DEFAULT_FOV_DEG)
	if p.has("quality"):
		quality = _Quality_Or(p["quality"], DEFAULT_QUALITY)
	if p.has("rate_hz"):
		rate_hz = _Rate_Or(p["rate_hz"], DEFAULT_RATE_HZ)
	_Apply_To_Nodes()
	# 分辨率 / FOV 变了 -> 新尺寸首帧可能还没绘制，重新 warmup（R7）。
	_warmed = false
	# 重新起跑：周期可能变了，累加器清零（不带着旧相位起跑，R12）。
	_accum = 0.0


## 生效参数（副本；自检 / Step 3 / Step 4 用）。
## 前 4 键的顺序与值沿用 task_33 的口径不变，`rate_hz` 追加在末尾。
func Get_Params() -> Dictionary:
	return {
		"width": width,
		"height": height,
		"fov_deg": fov_deg,
		"quality": quality,
		"rate_hz": rate_hz,
	}


# ================================================================ 数据流（manager 在 sensor_ready 之后调用）

## 绑定下行流（§5.3 时序铁律：**必须**在 `sensor_ready` 之后；与 `Lidar2D.SetStream` 同签名同时序）。
## 参数写 Variant + 鸭子调用（R6）：不 preload ComponentLink（net 层与传感器解耦，自检可换替身）。
## 绑流后累加器清零 = 整周期后出第一张图（自检与 sim 侧都能看到首帧）。
func SetStream(link: Variant, sensor_id: int) -> void:
	_link = link
	_sensor_id = maxi(sensor_id, 0)
	_accum = 0.0


## 解绑：清引用 + 清累加器（不重置参数、不打扰正在进行的抓拍）。
func ClearStream() -> void:
	_link = null
	_sensor_id = 0
	_accum = 0.0


## 有下行流吗（未绑 = false）。
func HasStream() -> bool:
	return _link != null


## 出帧节奏的唯一实现（照抄 Lidar2D 的累加器写法）：到点 -> 计数 -> 抓拍（协程，**不 await**）。
## `_link == null` 时**直接 return**：没有收端时不做无谓抓拍（一次抓拍有成本）。
## 到点计数刻意放在 `Can_Capture()` 闸**之前**：这样 headless 下"1 Hz 到点 1 次"仍可被观测，
##   而"绝不出帧"同时成立 —— 两条要求不互相掩盖。
func Tick_Stream(delta: float) -> void:
	if _link == null:
		return
	var period := 1.0 / maxf(rate_hz, MIN_RATE)
	_accum += maxf(delta, 0.0)
	if _accum + ACCUM_EPS < period:
		return
	_accum = minf(_accum - period, period)      # 卡顿后最多补一张，不"追赶爆拍"
	_tick_count += 1
	_Capture_Tick()                             # 协程：**这里绝不 await**（_process 不能阻塞）


## 到点次数（不区分有没有真的出图；自检用，对应 `Lidar2D.GetScanCount()`）。
func Get_Tick_Count() -> int:
	return _tick_count


## 真正 `SendBinary` 成功的帧数（恒 `<= Get_Tick_Count()`：抓拍慢于周期时到点多于帧，见 §4.8.3）。
func Get_Frame_Count() -> int:
	return _frame_count


## 一张图组好并（可能）下发：算 world_time / seq -> 有 link 且 OPEN 就发 `type=2` 帧 -> 最后 emit signal。
## 返回 **true** = 已 `SendBinary` 成功；**false** = 空图 / 未绑流 / 未 OPEN
##   （三者都**不抛、不打 ERROR、不递增 `_frame_count`**）。
## 有 link 但未 OPEN 时 **signal 照发**（与 `Lidar2D.scan_produced` 同口径，自检可观察）。
## 这是**出帧的唯一出口**（生产路径 `_Capture_Tick()` 也走它，不是测试专用分支）：
##   与 `Lidar2D._Publish()` 同构，只是"可注入 payload"——因为相机的 payload 来自一次耗时的 await 抓拍。
func Emit_Frame(jpeg: PackedByteArray) -> bool:
	if jpeg.is_empty():
		return false                              # 空图 = 什么都没发生（不发帧、不发 signal、不递增 seq）
	var world_time := WorldClock.Now()
	if _link != null and not is_instance_valid(_link):
		_link = null                              # 对端对象已释放：就地断引用（R11）
	var seq := 0
	if _link != null and _link.has_method("NextSeq"):
		seq = int(_link.call("NextSeq"))          # 每连接独立递增（§8.2）
	else:
		_seq += 1
		seq = _seq
	var sent := false
	if _link != null:
		var streaming := true
		if _link.has_method("IsOpen"):
			streaming = bool(_link.call("IsOpen"))
		if streaming and _link.has_method("SendBinary"):
			_link.call("SendBinary",
				WireProtocol.BuildCameraFrame(_sensor_id, seq, world_time, jpeg))
			_frame_count += 1
			sent = true
	frame_produced.emit(seq, world_time, jpeg.size())
	return sent


# ================================================================ 拍照（本 step 的唯一交付能力）

## 渲染一帧 -> JPEG 字节。**顺序不能变**（task_33 §五-4）：
##   ① warmup 未过 -> await 两次 process_frame
##   ② viewport.render_target_update_mode = UPDATE_ONCE
##   ③ await RenderingServer.frame_post_draw
##   ④ img = viewport.get_texture().get_image()
##   ⑤ return img.save_jpg_to_buffer(quality)
## 失败一律 `push_error` + 返回**空** `PackedByteArray()`（原因写清，见 Get_Last_Error()）。
func Capture_Jpeg() -> PackedByteArray:
	_Ensure_Refs()
	# 闸 1：无渲染器（headless=dummy）。**必须先判**，绝不触碰 get_texture()（否则打 ERROR）。
	if not _Has_Renderer():
		return _Fail("当前环境无渲染器（display=%s adapter='%s'）：--headless 用 dummy 渲染器，无法抓拍"
			% [DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])
	# 闸 2：场景绑定丢失（E16 漏写 node_paths）。
	if viewport == null or lens == null:
		return _Fail("场景绑定丢失：viewport/lens 为 null（检查 .tscn 的 node_paths）")
	# 闸 3：树外调用（树外没有 Viewport 树 / render target，与 Lidar2D.ScanOnce 的 R5 同款）。
	if not is_inside_tree():
		return _Fail("不在场景树内，无法抓拍")

	# ① warmup：前 2 帧是黑的
	if not _warmed:
		for _i in range(WARMUP_FRAMES):
			await get_tree().process_frame
		_warmed = true

	# ①② 之间：把本节点（光心）的世界变换写进镜头（见文件头事实 ②）
	_Sync_View_Transform()

	# ② 只渲这一帧（绝不用 UPDATE_ALWAYS）
	viewport.render_target_update_mode = SubViewport.UPDATE_ONCE

	# ③ 必须等这一帧画完，否则读到上一帧 / 空缓冲
	await RenderingServer.frame_post_draw

	# ④ GPU -> CPU 回读
	var tex: ViewportTexture = viewport.get_texture()
	var img: Image = tex.get_image() if tex != null else null
	if img == null or img.is_empty():
		return _Fail("get_image 返回空（渲染未就绪）")

	# ⑤ JPEG 编码（1280×720 实测 ~10 ms）
	var jpg: PackedByteArray = img.save_jpg_to_buffer(quality)
	if jpg.is_empty():
		return _Fail("save_jpg_to_buffer 返回空（quality=%.3f）" % quality)
	_capture_count += 1
	_last_error = ""
	return jpg


## 抓一张并存盘（供 Step 4 拍照用）：先 Capture_Jpeg()，再落盘（自动建目录）。
## 抓拍失败的 `push_error` 由 Capture_Jpeg() 打；这里的路径问题用 **push_warning**
##   （可预期的调用方错误，且验收要求日志 0 ERROR —— task 文档未规定级别，见计划 C5）。
## 只接受**绝对路径**（相对路径在 headless 下没有稳定基准，直接拒绝）。
func Capture_To_File(abs_path: String) -> bool:
	var jpg: PackedByteArray = await Capture_Jpeg()
	if jpg.is_empty():
		return false
	if abs_path.is_empty() or not abs_path.is_absolute_path():
		push_warning("%s Capture_To_File 需要绝对路径，收到 '%s'" % [TAG, abs_path])
		return false
	var handle: FileAccess = FileAccess.open(abs_path, FileAccess.WRITE)
	if handle == null:
		# 目录可能还不存在：静默逐级建目录后重试一次。
		if not _Make_Dir_Chain(abs_path.get_base_dir()):
			push_warning("%s 无法创建目录 %s" % [TAG, abs_path.get_base_dir()])
			return false
		handle = FileAccess.open(abs_path, FileAccess.WRITE)
	if handle == null:
		push_warning("%s 无法写入 %s（err=%d）" % [TAG, abs_path, FileAccess.get_open_error()])
		return false
	handle.store_buffer(jpg)
	handle.close()
	# 落盘校验：读回的字节数必须与要写的一致（半截文件 = 失败）
	var written: PackedByteArray = FileAccess.get_file_as_bytes(abs_path)
	return written.size() == jpg.size()


# ================================================================ 只读查询

## 当前成图尺寸：`viewport.size`；refs 缺失时回退参数值（白盒断言用，永不失败）。
func Get_Image_Size() -> Vector2i:
	_Ensure_Refs()
	if viewport != null:
		return viewport.size
	return Vector2i(width, height)


## 环境 + 装配探测：能不能出图（headless / 无渲染器 / refs 丢失 / 树外 → false）。
## 这是**环境探测**、不是 task 文档列的交付能力（计划 C3）：
##   自检与 Step 4 的拍照脚本要在**碰 dummy 渲染器之前**分流，否则每次都会打
##   `ERROR: Parameter "t" is null` 把"0 ERROR"验收搞红。
func Can_Capture() -> bool:
	_Ensure_Refs()
	return is_inside_tree() and viewport != null and lens != null and _Has_Renderer()


## 最近一次失败原因（空串 = 没失败过；自检打印用）。
func Get_Last_Error() -> String:
	return _last_error


## 成功出图次数（自检 / Step 4 可观察）。
func Get_Capture_Count() -> int:
	return _capture_count


# ================================================================ 私有：环境 / 结构

## 有真渲染器吗：`--headless` 的 display 名是 "headless"、adapter 名是空串（实测）。
func _Has_Renderer() -> bool:
	if DisplayServer.get_name() == "headless":
		return false
	return not RenderingServer.get_video_adapter_name().is_empty()


## 生产路径（唯一的定时抓拍入口）：`Can_Capture()` 闸 -> `await Capture_Jpeg()` -> `Emit_Frame()`。
## `Can_Capture() == false`（--headless 的 dummy 渲染器）时**只告警一次、绝不出帧、绝不 await**
##   —— 否则协程会永久挂在 `frame_post_draw` 上（E26-③，本 step 最硬的一条）。
## `_capturing` 互斥：上一张还没回来时到点只计数、不叠拍（帧率上限 = 抓拍耗时，R13）。
func _Capture_Tick() -> void:
	if not Can_Capture():
		_Warn_No_Renderer()
		return
	if _capturing:
		return
	_capturing = true
	var jpeg: PackedByteArray = await Capture_Jpeg()
	_capturing = false
	if jpeg.is_empty():
		return                                    # 失败原因已由 Capture_Jpeg() 记好并 push_error
	Emit_Frame(jpeg)


## 无渲染器只告警**一次**（E26-③）：绝不 await、绝不 push_error，
##   否则 headless 下每帧刷屏，还会把「日志 0 ERROR」的验收搞红。
func _Warn_No_Renderer() -> void:
	if _warned_no_render:
		return
	_warned_no_render = true
	push_warning("%s 无渲染器（display=%s adapter='%s'）：不抓拍、不出帧"
		% [TAG, DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])


## 把本节点（光心）的世界变换写进镜头。
## 必须手工同步：`SubViewport` 不是 `Node3D`，中间隔了它，`Lens.global_transform`
##   就断了继承链（实测本节点在 (64,6,64) 时 Lens 的 global 仍是 IDENTITY）。
func _Sync_View_Transform() -> void:
	if lens == null or not is_instance_valid(lens):
		return
	if not is_inside_tree() or not lens.is_inside_tree():
		return
	lens.global_transform = global_transform


## 把生效参数应用到节点：SubViewport.size / Camera3D 的 fov + near/far。
## near / far 不在 Configure 的参数表里（task 文档只给 4 个键），但**唯一来源仍是本文件常量**
## （`.tscn` 里那两行只是初始声明；常量改了这里会跟着变，避免两处打架）。
func _Apply_To_Nodes() -> void:
	if viewport != null:
		viewport.size = Vector2i(width, height)
	if lens != null:
		lens.fov = fov_deg
		lens.near = DEFAULT_NEAR
		lens.far = DEFAULT_FAR


## E16 兜底：node_paths 漏写时 @export 引用会静默为 null，这里按子节点名回退一次。
func _Ensure_Refs() -> void:
	if viewport == null:
		viewport = get_node_or_null("Viewport") as SubViewport
	if lens == null:
		lens = get_node_or_null("Viewport/Lens") as Camera3D


## 统一失败出口：记原因 + push_error + 返回空数组。
func _Fail(reason: String) -> PackedByteArray:
	_last_error = reason
	push_error("%s %s" % [TAG, reason])
	return PackedByteArray()


## 逐级建目录（已存在 = 成功），失败只返回 false、**绝不打印 ERROR**。
## 为什么不用 `DirAccess.make_dir_recursive_absolute()`：它在失败时走 `ERR_FAIL_V_MSG`，
##   会打印 `ERROR: Could not create directory: ...`（实测，B7 的坏路径用例直接命中），
##   把"日志 0 ERROR"验收搞红；`DirAccess.make_dir_absolute()` 失败只返回 Error 码、不打印（实测）。
func _Make_Dir_Chain(abs_dir: String) -> bool:
	if abs_dir.is_empty():
		return false
	if DirAccess.dir_exists_absolute(abs_dir):
		return true
	var missing: PackedStringArray = PackedStringArray()
	var probe: String = abs_dir
	while not probe.is_empty() and not DirAccess.dir_exists_absolute(probe):
		missing.append(probe)
		var parent: String = probe.get_base_dir()
		if parent == probe:
			break
		probe = parent
	for i in range(missing.size() - 1, -1, -1):
		var made: Error = DirAccess.make_dir_absolute(missing[i])
		if made != OK and made != ERR_ALREADY_EXISTS:
			return false
	return DirAccess.dir_exists_absolute(abs_dir)


# ================================================================ 私有：参数校验

func _Size_Or(value: Variant, fallback: int) -> int:
	if not (value is int or value is float):
		_Warn_Params("size=%s" % str(value))
		return fallback
	var v: int = int(value)
	if v < MIN_SIZE:
		_Warn_Params("size=%s" % str(value))
		return fallback
	return mini(v, MAX_SIZE)


func _Fov_Or(value: Variant, fallback: float) -> float:
	if not (value is int or value is float):
		_Warn_Params("fov_deg=%s" % str(value))
		return fallback
	var v: float = float(value)
	if v < MIN_FOV_DEG or v > MAX_FOV_DEG:
		_Warn_Params("fov_deg=%s" % str(value))
		return fallback
	return v


func _Quality_Or(value: Variant, fallback: float) -> float:
	if not (value is int or value is float):
		_Warn_Params("quality=%s" % str(value))
		return fallback
	var v: float = float(value)
	if v < MIN_QUALITY or v > MAX_QUALITY:
		_Warn_Params("quality=%s" % str(value))
		return fallback
	return v


## 非法参数只告警一次（F1：避免每帧刷屏）。
func _Warn_Params(detail: String) -> void:
	if _warned_params:
		return
	_warned_params = true
	push_warning("%s 参数非法（%s），已回退默认值" % [TAG, detail])


## 出帧频率：合法 -> `maxf(v, MIN_RATE)`；非数字 / `<= 0` -> fallback + 告警一次（R12）。
## 与 `_Size_Or / _Fov_Or / _Quality_Or` 同款（不是夹取，是回退）。
func _Rate_Or(value: Variant, fallback: float) -> float:
	if not (value is int or value is float):
		_Warn_Params("rate_hz=%s" % str(value))
		return fallback
	var v: float = float(value)
	if v <= 0.0:
		_Warn_Params("rate_hz=%s" % str(value))
		return fallback
	return maxf(v, MIN_RATE)
