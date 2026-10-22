# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 传感器组件 lidar_2d（Step 8 / task_30 §7.4 / COMPONENTS.md §4.7）：
#   从挂载点出发、在**水平面**投 ray_count 条射线（默认 360 条 / 圈、10 Hz），
#   每圈产出一份 LaserScan（task_30 §7.4）并按 §8.2 的二进制帧下发给 sim。
#
# 做什么 / 不做什么：
#   - 只模拟「周围一圈距离」；**不复刻真机协议 / 三角测距 / 噪声**（§7.4）。
#     noise_sigma 默认 0.0，> 0 才叠加高斯噪声（§10：噪声默认关闭）。
#   - 不发 JSON（§8.2 大数据一律二进制）；不碰 socket（只鸭子调用 link.SendBinary）。
#   - 不写 position / 不推物理：外参由 Vehicle.AttachSensor() 写进来（S1），本文件只读 global_*。
#   - 不缓存场景、不查目录：目录在 SensorCatalog，装配在 Vehicle / manager。
#
# 坐标（S2）：index i -> 局部方向 (cos a, 0, sin a)，a = TAU * i / ray_count，局部 +X = 车头。
#   LaserPoint.angle = a（弧度，从 +X 起、世界 yaw 增大方向）→ sim 侧 ob_angle = yaw + angle 直接可用。
#   角度全部在**雷达局部系**，换轴由车辆变换负责，所以本文件不引 WorldTransform。
#
# 约定（§8.2 / §6.5）：
#   - 命中 -> 距离（米）；未命中 -> **0.0**（不是 range、不是 inf）。
#   - 每圈 seq = ComponentLink.NextSeq()（**每连接**独立递增，§8.2）；无 link 时用内部计数（仅自检可观察）。
#   - world_time = WorldClock.Now()（§6.3-3 统一时间基）。
#
# 不声明全局类名（§9.1 E5）：场景按路径挂脚本。

extends Node3D

const WorldClock := preload("res://src/common/WorldClock.gd")
const WireProtocol := preload("res://src/net/WireProtocol.gd")

# ---------------------------------------------------------------- 参数默认值（唯一来源）
## 量程（米）：与真机一致（task_30 §7.4）。
const DEFAULT_RANGE := 12.0
## 一圈射线数。
const DEFAULT_RAYS := 360
## 扫描频率（Hz）：与真机 10 Hz 一致，影响 sim 侧的时序假设。
const DEFAULT_RATE_HZ := 10.0
## 命中强度（0-255，§7.4 允许给常数）。
const DEFAULT_INTENSITY := 255.0
## 射线碰撞掩码 = world_ground(1) | world_obstacle(2) | vehicle(4)：其它 avatar 可见（S3）。
const DEFAULT_SCAN_MASK := 1 | 2 | 4

## 量程下限（米）：避免 0 长度射线（R14：< 0.01 m 的命中会被读成无回波）。
const MIN_RANGE := 0.01
## 频率下限（Hz）：避免除零。
const MIN_RATE := 1e-3
## 一圈射线数上限：防止外部参数把一帧撑爆（4096 条 = 16 KB payload）。
const MAX_RAYS := 4096
## 累加器容差（S6 / P6）：60 fps 下 6 × 1/60 = 0.09999999999999999 < 0.1，
##   不加容差 1 s 只扫 9 次；加了之后帧 6/12/…/60 恰好 10 次。
const ACCUM_EPS := 1e-6

## 一圈完成（无论有无 link 都发；自检时序 + 可视化都靠它）。
signal scan_produced(ranges: PackedFloat32Array, world_time: float, seq: int)

# ---------------------------------------------------------------- 传感器参数（Configure 后生效）
## 量程（米）：超过它的回波一律记 0.0。
var range_limit: float = DEFAULT_RANGE
## 一圈射线数。
var ray_count: int = DEFAULT_RAYS
## 扫描频率（Hz）。
var rate_hz: float = DEFAULT_RATE_HZ
## 距离噪声标准差（米）：v1 默认 0（不复刻噪声）。
var noise_sigma: float = 0.0
## 命中强度（0-255）；未命中一律 0。
var intensity_hit: float = DEFAULT_INTENSITY
## 射线碰撞掩码（S3）。
var scan_mask: int = DEFAULT_SCAN_MASK

# ---------------------------------------------------------------- 节点引用（E16：靠 node_paths 绑定）
## 扫描点可视化（MeshInstance3D，默认 visible=false → 零开销）。
@export var debug_points: MeshInstance3D = null
## 是否画点云（SetDebugVisible 同步到 debug_points.visible）。
@export var debug_visible: bool = false

# ---------------------------------------------------------------- 内部状态
var _accum: float = 0.0                     # 时间累加器（秒）
var _seq: int = 0                           # 无 link 时的自增序号（只用于 signal）
var _link: Variant = null                   # ComponentLink（RefCounted）；未绑定 = null
var _sensor_id: int = 0                     # u16 语义的传感器编号（写进帧头 slot_id）
var _last_ranges := PackedFloat32Array()    # 最近一圈距离（米，0.0 = 无回波）
var _last_scan: Dictionary = {}             # 最近一圈 LaserScan（task_30 §7.4）
var _scan_count: int = 0
var _rng := RandomNumberGenerator.new()
var _warned_no_space: bool = false          # 世界未就绪只告警一次
var _warned_params: bool = false            # 非法参数只告警一次
var _debug_material: StandardMaterial3D = null


# ================================================================ 生命周期

## 只做引用兜底 + 同步可视化开关：**不重置参数、不扫描**（R7）。
## 时序原因：Vehicle.AttachSensor() 先 add_child（触发本回调）后 Configure(params)，
##   在 _ready 里重置参数会盖掉 Configure 的结果。
func _ready() -> void:
	_ensure_refs()
	_SyncDebugVisible()


## 累加器 -> 一圈扫描 -> 发布（S5 / S6）。卡顿后最多补一圈，不"追赶爆扫"。
func _physics_process(delta: float) -> void:
	var period := 1.0 / maxf(rate_hz, MIN_RATE)
	_accum += maxf(delta, 0.0)
	if _accum + ACCUM_EPS < period:
		return
	_accum = minf(_accum - period, period)
	ScanOnce()
	_Publish(_last_ranges)


## 出树即断流：防悬挂引用（manager 释放 ComponentLink 后本节点仍可能存活一帧）。
func _exit_tree() -> void:
	_link = null


# ================================================================ 装配接口

## 装配参数：range / rays / rate_hz / noise_sigma / intensity_hit / scan_mask。
## 键缺失 = 保留当前值；值非法 = 回退**默认常量**并 push_warning 一次（F1）。
func Configure(params: Dictionary) -> void:
	var p: Dictionary = params if params != null else {}
	if p.has("rays"):
		ray_count = _RaysOr(p["rays"], DEFAULT_RAYS)
	if p.has("range"):
		range_limit = _RangeOr(p["range"], DEFAULT_RANGE)
	if p.has("rate_hz"):
		rate_hz = _RateOr(p["rate_hz"], DEFAULT_RATE_HZ)
	if p.has("noise_sigma") or p.has("noise"):
		noise_sigma = _NoiseOr(p.get("noise_sigma", p.get("noise", 0.0)))
	if p.has("intensity_hit") or p.has("intensity"):
		intensity_hit = clampf(
			_NumberOr(p.get("intensity_hit", p.get("intensity", DEFAULT_INTENSITY)),
				DEFAULT_INTENSITY), 0.0, 255.0)
	if p.has("scan_mask") or p.has("mask"):
		var raw: Variant = p.get("scan_mask", p.get("mask", DEFAULT_SCAN_MASK))
		if raw is int or raw is float:
			var mask := int(raw)
			scan_mask = mask if mask > 0 else DEFAULT_SCAN_MASK
		else:
			_warn_param("scan_mask=%s" % str(raw))
			scan_mask = DEFAULT_SCAN_MASK
	# 重新起跑：周期可能变了，累加器清零；缓存按新射线数重建（避免旧长度混进新一圈）。
	_accum = 0.0
	_last_ranges = _ZeroRanges()
	_last_scan = _MakeLaserScan(_last_ranges)


## 生效参数（副本；自检 / manager 用）。
func GetParams() -> Dictionary:
	return {
		"range": range_limit,
		"rays": ray_count,
		"rate_hz": rate_hz,
		"noise_sigma": noise_sigma,
		"intensity_hit": intensity_hit,
		"scan_mask": scan_mask,
	}


# ================================================================ 数据流（manager 在 sensor_ready 之后调用）

## 绑定下行流（§5.3 时序铁律：**必须**在 sensor_ready 之后）。
## 参数写 Variant + 鸭子调用（R6）：不 preload ComponentLink（net 层与传感器解耦，自检可换替身）。
func SetStream(link: Variant, sensor_id: int) -> void:
	_link = link
	_sensor_id = maxi(sensor_id, 0)


func ClearStream() -> void:
	_link = null
	_sensor_id = 0


func HasStream() -> bool:
	return _link != null


# ================================================================ 采样

## 投射一圈并更新缓存：**不发包、不递增 seq、不发 signal**（Vehicle.SampleSensors 走这里）。
## 树外调用返回全 0（R5：树外访问 global_* 会报 ERROR）。
func ScanOnce() -> PackedFloat32Array:
	var n := clampi(ray_count, 1, MAX_RAYS)
	var ranges := PackedFloat32Array()
	ranges.resize(n)                                  # PackedFloat32Array 默认全 0.0 = 无回波
	_scan_count += 1
	if not is_inside_tree():
		_last_ranges = ranges
		_last_scan = _MakeLaserScan(ranges)
		return ranges
	var space := _SpaceState()
	if space == null:                                 # 世界未就绪：全 0，告警一次
		_last_ranges = ranges
		_last_scan = _MakeLaserScan(ranges)
		return ranges
	var origin: Vector3 = global_position
	var basis: Basis = global_basis
	var exclude := _SelfRidArray()                    # 自车不可见（P4 双保险）
	var limit := maxf(range_limit, MIN_RANGE)
	for i in range(n):
		var a := TAU * float(i) / float(n)
		var dir: Vector3 = basis * Vector3(cos(a), 0.0, sin(a))   # 水平面；+X = 车头
		var query := PhysicsRayQueryParameters3D.create(origin, origin + dir * limit,
			scan_mask, exclude)
		var hit: Dictionary = space.intersect_ray(query)
		var d := 0.0
		if not hit.is_empty():                        # P5：未命中返回空字典，不是 null
			d = origin.distance_to(hit["position"])
			if noise_sigma > 0.0:
				d = maxf(d + _rng.randfn(0.0, noise_sigma), 0.0)
		ranges[i] = d
	_last_ranges = ranges
	_last_scan = _MakeLaserScan(ranges)
	_RefreshDebug(ranges)
	return ranges


## 最近一圈距离（副本）。
func GetLastRanges() -> PackedFloat32Array:
	return _last_ranges.duplicate()


## 最近一圈 LaserScan（task_30 §7.4 的 {points, stamp, scan_freq, scan_time}；深拷贝）。
func GetLaserScan() -> Dictionary:
	return _last_scan.duplicate(true)


## 累计扫描次数（ScanOnce 与 _physics_process 都算；自检用）。
func GetScanCount() -> int:
	return _scan_count


## 点云可视化开关（默认关；开时立刻用最近一圈重画）。
func SetDebugVisible(on: bool) -> void:
	debug_visible = on
	_SyncDebugVisible()
	if on:
		_RefreshDebug(_last_ranges)


# ================================================================ 私有：发布

## 一圈的单一出口：算 world_time / seq -> 有 link 就发二进制帧 -> 最后 emit signal。
## 有 link 但未 OPEN 时**不发帧**，signal 照发（自检可观察，R13）。
func _Publish(ranges: PackedFloat32Array) -> void:
	var world_time := WorldClock.Now()
	var seq := 0
	if _link != null and not is_instance_valid(_link):
		_link = null                                  # 对端对象已释放：就地断引用
	if _link == null:
		_seq += 1
		seq = _seq
	else:
		if _link.has_method("NextSeq"):
			seq = int(_link.call("NextSeq"))          # 每连接独立递增（§8.2）
		var streaming := true
		if _link.has_method("IsOpen"):
			streaming = bool(_link.call("IsOpen"))
		if streaming and _link.has_method("SendBinary"):
			_link.call("SendBinary",
				WireProtocol.BuildLidar2DFrame(_sensor_id, seq, world_time, ranges))
	scan_produced.emit(ranges, world_time, seq)


# ================================================================ 私有：射线 / 结构

## 物理空间（带守卫）；树外 / 世界未就绪 -> null（R5）。
func _SpaceState() -> PhysicsDirectSpaceState3D:
	if not is_inside_tree():
		return null
	var world := get_world_3d()
	if world == null:
		_warn_no_space("get_world_3d() 为空")
		return null
	var space: PhysicsDirectSpaceState3D = world.direct_space_state
	if space == null:
		_warn_no_space("direct_space_state 为空")
		return null
	return space


func _warn_no_space(reason: String) -> void:
	if _warned_no_space:
		return
	_warned_no_space = true
	push_warning("[lidar] 物理空间未就绪（%s），本圈按无回波处理" % reason)


## 宿主碰撞体（自车）：向上找第一个 CollisionObject3D（= Vehicle）。
func _HostBody() -> CollisionObject3D:
	var node: Node = get_parent()
	while node != null:
		var body := node as CollisionObject3D
		if body != null:
			return body
		node = node.get_parent()
	return null


## 射线排除表：[宿主 RID] 或 []（树外 / 没有宿主）。
func _SelfRidArray() -> Array:
	var body := _HostBody()
	if body == null:
		return []
	return [body.get_rid()]


## 距离数组 -> LaserScan（task_30 §7.4；字段与 Rust LaserPoint 对齐）。
## angle = TAU * i / n（从局部 +X 起、世界 yaw 增大方向）；intensity 命中 = intensity_hit、未命中 = 0。
## stamp 用**世界时钟基**（纳秒），只用于相对对齐，不是 Unix 时间。
func _MakeLaserScan(ranges: PackedFloat32Array) -> Dictionary:
	var n := ranges.size()
	var points: Array = []
	points.resize(n)
	var step := TAU / float(n) if n > 0 else 0.0
	for i in range(n):
		var d := ranges[i]
		points[i] = {
			"angle": step * float(i),
			"range": d,
			"intensity": intensity_hit if d > 0.0 else 0.0,
		}
	var period := 1.0 / maxf(rate_hz, MIN_RATE)
	return {
		"points": points,
		"stamp": WorldClock.NowUsec() * 1000,          # 微秒 -> 纳秒
		"scan_freq": rate_hz,
		"scan_time": period,
	}


# ================================================================ 私有：可视化

## 点云重画（E22 / P7）：只用 surface_begin / surface_add_vertex / surface_end / clear_surfaces。
## ImmediateMesh **没有** surface_get_array_len() / surface_get_primitive_type()（4.7.2 实测），
##   误用会 SCRIPT ERROR 且进程不退出 —— 本函数与自检都禁止出现这两个名字。
func _RefreshDebug(ranges: PackedFloat32Array) -> void:
	if debug_points == null or not is_instance_valid(debug_points):
		return
	debug_points.visible = debug_visible
	if not debug_visible:
		return
	var mesh: ImmediateMesh = debug_points.mesh as ImmediateMesh
	if mesh == null:
		mesh = ImmediateMesh.new()
		debug_points.mesh = mesh
	if _debug_material == null:
		var mat := StandardMaterial3D.new()
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mat.albedo_color = Color(1.0, 0.28, 0.2, 1.0)
		_debug_material = mat
	mesh.clear_surfaces()
	var n := ranges.size()
	if n == 0:
		return
	var step := TAU / float(n)
	mesh.surface_begin(Mesh.PRIMITIVE_POINTS, _debug_material)
	for i in range(n):
		var d := ranges[i]
		if d <= 0.0:
			continue
		var a := step * float(i)
		mesh.surface_add_vertex(Vector3(cos(a) * d, 0.0, sin(a) * d))
	mesh.surface_end()


func _SyncDebugVisible() -> void:
	if debug_points != null and is_instance_valid(debug_points):
		debug_points.visible = debug_visible


# ================================================================ 私有：参数校验

## E16 兜底：node_paths 漏写时 @export 引用会静默为 null，这里按子节点名回退一次。
func _ensure_refs() -> void:
	if debug_points == null:
		debug_points = get_node_or_null("DebugPoints") as MeshInstance3D


func _ZeroRanges() -> PackedFloat32Array:
	var ranges := PackedFloat32Array()
	ranges.resize(clampi(ray_count, 1, MAX_RAYS))
	return ranges


func _RaysOr(value: Variant, fallback: int) -> int:
	if not (value is int or value is float):
		_warn_param("rays=%s" % str(value))
		return fallback
	var count := int(value)
	if count < 1:                                     # 0 / 负数 = 非法（不是"改成 1 条"）
		_warn_param("rays=%s" % str(value))
		return fallback
	return clampi(count, 1, MAX_RAYS)


func _RangeOr(value: Variant, fallback: float) -> float:
	var v := _NumberOr(value, fallback)
	if v <= 0.0:
		_warn_param("range=%s" % str(value))
		return fallback
	return maxf(v, MIN_RANGE)


func _RateOr(value: Variant, fallback: float) -> float:
	var v := _NumberOr(value, fallback)
	if v <= 0.0:
		_warn_param("rate_hz=%s" % str(value))
		return fallback
	return maxf(v, MIN_RATE)


func _NoiseOr(value: Variant) -> float:
	return maxf(_NumberOr(value, 0.0), 0.0)


## int/float -> float；其它类型 -> fallback（并告警一次）。
func _NumberOr(value: Variant, fallback: float) -> float:
	if value is int or value is float:
		return float(value)
	_warn_param("value=%s" % str(value))
	return fallback


func _warn_param(detail: String) -> void:
	if _warned_params:
		return
	_warned_params = true
	push_warning("[lidar] 参数非法（%s），已回退默认值" % detail)
