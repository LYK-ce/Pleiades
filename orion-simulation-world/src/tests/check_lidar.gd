# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# Lidar2D 传感器自检（Step 8）：场景形态 / 目录 / 装配 / **一圈距离的几何正确性（核心验收）** /
# 10 Hz 时序 / 二进制帧格式 / 边界与白盒 lint。
# 用法：timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_lidar.gd
# 期望：failed=0 + SELF-CHECK OK + EXIT=0，且日志无 ERROR / SCRIPT ERROR / Parse Error
#       （COMPONENTS.md §11 Step 8）。
#
# 决定本文件写法的实测事实（plan.md §3）：
#   P1  在 `await physics_frame` 之后从 Vehicle/Sensors/<slot> 调 intersect_ray 正常、无 ERROR。
#   P2  车在 world (60,90,0)、rotation.y = YawToGodot(+90°) → i=0 命中 wall_c（y=92）距离 2.000 m；
#       该墙一圈的命中数实测 152（f8）。
#   P4  起点落在 BoxShape3D 内部时 hit_from_inside 默认 false → 不报自命中。
#   P5  未命中时 intersect_ray() 返回**空字典 {}**（不是 null）。
#   P6  朴素累加器 60 帧只扫 9 次；+1e-6 容差 → 帧 6/12/…/60 恰好 10 次（E1）。
#   P7  ImmediateMesh **没有** surface_get_array_len()/surface_get_primitive_type()，误用会
#       SCRIPT ERROR 且进程不退出 → 所有命令带 timeout，F4 直接禁止这两个符号。
#   P9  9999 被占用时加载 World.tscn 会打 [ws] listen WARNING（不是 ERROR），本文件只看 ERROR。
#   P10 World.gd 每帧建 12k 体素；Ground=layer1 / VoxelWorld=layer2 / Vehicle=layer4+mask7。
#
# 本文件不新增任何 .tscn：自建障碍用 StaticBody3D + BoxShape3D 现场 new()；
# 假连接用内嵌 FakeLink 类（与 src/tests/doubles/ 同风格）。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WireProtocol := preload("res://src/net/WireProtocol.gd")

const LIDAR_SCENE := "res://src/scenes/sensors/Lidar2D.tscn"
const LIDAR_SCRIPT := "res://src/sensors/Lidar2D.gd"
const CATALOG_SCRIPT := "res://src/sensors/SensorCatalog.gd"
const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const WORLD_SCENE := "res://src/scenes/World.tscn"

const SLOT := "lidar_front"
const SENSOR_NAME := "lidar_2d"
const LIDAR_NODE := "Lidar2D"

const DEFAULT_RANGE := 12.0
const DEFAULT_RAYS := 360
const DEFAULT_RATE_HZ := 10.0
const DEFAULT_PARAMS := {"range": 12.0, "rays": 360, "rate_hz": 10.0}

## 空旷区：世界坐标 (100, 60, 0) 附近 12 m 内无任何静态障碍（P10）。
const OPEN_WORLD := Vector3(100.0, 60.0, 0.0)
## 真实墙 wall_c（世界 y=92 的墙面）走廊：车在 (60,90,0)、yaw=+90° 时车头正对墙（P2）。
const WALL_WORLD := Vector3(60.0, 90.0, 0.0)
const WALL_HIT_M := 2.0
## 自建 box 的命中距离：原点 (100.1, ·, 60)，box 面 x=103.5；侧向 box 面 z=63.0。
const BOX_FWD_M := 3.4
const BOX_SIDE_M := 3.0
## D6 前进 1 m 后的距离（3.4 - 1.0）。
const BOX_FWD_AFTER_MOVE_M := 2.4
## D8 第二台车的车体面（world x=101.0 - 0.15 = 100.85，原点 100.1）。
const VEHICLE_B_M := 0.75

## 帧格式：360 线 = 15 + 4×360。
const FRAME_BYTES_360 := 1455
const SENSOR_ID := 7

const TOL := 0.02

## 白盒 lint：传感器脚本里**不允许**出现的符号（越界 / 违反接口分工 / 不存在的 API）。
const BANNED_TOKENS := [
	"class_name",
	"move_and_slide",
	"intersect_shape",
	"surface_get_array_len",
	"surface_get_primitive_type",
	"global_position =",
	"SendJson",
	"res://src/net/ComponentLink.gd",
]


## 假连接（RefCounted）：只实现 Lidar2D 鸭子调用的三个方法。
class FakeLink:
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


var _total: int = 0
var _failed: int = 0
var _harness_errors: int = 0


func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ A. 树外：场景与脚本形态
	var packed: PackedScene = load(LIDAR_SCENE) as PackedScene
	if packed == null:
		_fatal("A1-scene-loads", LIDAR_SCENE)
		_finish()
		return
	var probe: Node = packed.instantiate()
	if probe == null:
		_fatal("A2-scene-instantiates", LIDAR_SCENE)
		_finish()
		return
	_check("A1-scene-loads", probe is Node3D, "%s -> %s" % [LIDAR_SCENE, probe.get_class()])

	# A2 根节点：名字 / 类型 / 脚本
	var probe_script: Script = probe.get_script()
	var probe_path := probe_script.resource_path if probe_script != null else "null"
	_check("A2-root-type", probe.name == LIDAR_NODE and probe is Node3D
		and probe_path == LIDAR_SCRIPT,
		"name=%s class=%s script=%s" % [probe.name, probe.get_class(), probe_path])

	# A3 DebugPoints：存在 / MeshInstance3D / 默认不显示
	var points: Node = probe.get_node_or_null("DebugPoints")
	_check("A3-debug-points", points != null and points is MeshInstance3D
		and not (points as MeshInstance3D).visible,
		"node=%s class=%s visible=%s" % [points,
			points.get_class() if points != null else "null",
			(points as MeshInstance3D).visible if points != null else "?"])

	# A4 E16：@export 节点引用靠 node_paths 真绑定（漏写会静默 null）
	var bound: Variant = probe.get("debug_points")
	_check("A4-debug-binding", bound != null and bound == points,
		"debug_points=%s DebugPoints=%s" % [bound, points])

	# A5 默认参数（唯一来源 = Lidar2D.gd；与 SensorCatalog.DEFAULTS 逐键一致，B4 再比目录）
	var params_default: Dictionary = probe.call("GetParams")
	var defaults_ok := absf(float(params_default.get("range", -1.0)) - DEFAULT_RANGE) < 1e-9 \
		and int(params_default.get("rays", -1)) == DEFAULT_RAYS \
		and absf(float(params_default.get("rate_hz", -1.0)) - DEFAULT_RATE_HZ) < 1e-9
	_check("A5-default-params", defaults_ok, "params=%s" % [params_default])

	# A6 文件形态：.tscn 无内嵌脚本 + 两侧头注释齐全 + .gd 无 class_name
	var tscn_text := _read_text(LIDAR_SCENE)
	var gd_source: String = probe_script.source_code if probe_script != null else ""
	_check("A6-file-shape",
		not tscn_text.contains("[sub_resource type=\"GDScript\"")
		and tscn_text.contains("Presented by KeJi")
		and tscn_text.contains("Created Date")
		and tscn_text.contains("Modified Date")
		and gd_source.begins_with("# Presented by KeJi")
		and not _code_of(LIDAR_SCRIPT).contains("class_name"),
		"tscn=%d B gd=%d B" % [tscn_text.length(), gd_source.length()])

	# A7 树外 free 不产生 ERROR（场景可安全丢弃）
	probe.free()
	_check("A7-free-outside-tree", not is_instance_valid(probe), "树外实例已释放")

	# ============================================================ B. 目录（SensorCatalog）
	var catalog: Script = load(CATALOG_SCRIPT) as Script
	if catalog == null:
		_fatal("B0-catalog-loads", CATALOG_SCRIPT)
		_finish()
		return
	var has_lidar: bool = bool(catalog.call("Has", SENSOR_NAME))
	var has_imu: bool = bool(catalog.call("Has", "imu"))
	_check("B1-catalog-has", has_lidar and not has_imu,
		"lidar_2d=%s imu=%s" % [has_lidar, has_imu])

	var scene_from_catalog: Variant = catalog.call("GetScene", SENSOR_NAME)
	_check("B2-catalog-scene", scene_from_catalog is PackedScene
		and (scene_from_catalog as PackedScene).resource_path == LIDAR_SCENE,
		"scene=%s path=%s" % [scene_from_catalog,
			(scene_from_catalog as PackedScene).resource_path
				if scene_from_catalog is PackedScene else "null"])

	var scene_again: Variant = catalog.call("GetScene", SENSOR_NAME)
	_check("B3-catalog-scene-cache", scene_again is PackedScene
		and scene_again == scene_from_catalog, "same=%s" % [scene_again == scene_from_catalog])

	var catalog_defaults: Variant = catalog.call("GetDefaults", SENSOR_NAME)
	var catalog_dict: Dictionary = catalog_defaults if catalog_defaults is Dictionary else {}
	var catalog_keys_ok := _dict_equal(catalog_dict, DEFAULT_PARAMS)
	for key: Variant in DEFAULT_PARAMS.keys():
		if typeof(params_default.get(key)) != typeof(DEFAULT_PARAMS[key]) \
				or params_default.get(key) != DEFAULT_PARAMS[key]:
			catalog_keys_ok = false
	_check("B4-catalog-defaults", catalog_keys_ok,
		"catalog=%s object=%s" % [catalog_dict, params_default])

	var unknown_defaults: Variant = catalog.call("GetDefaults", "imu")
	_check("B5-catalog-unknown-defaults", unknown_defaults is Dictionary
		and (unknown_defaults as Dictionary).is_empty(), "imu defaults=%s" % [unknown_defaults])

	# B6 manager 的 _CatalogSceneExists() 依赖 SCENES 是**脚本常量**
	var consts: Dictionary = catalog.get_script_constant_map()
	var scenes_table: Variant = consts.get("SCENES", null)
	_check("B6-catalog-scenes-constant", scenes_table is Dictionary
		and (scenes_table as Dictionary).get(SENSOR_NAME, "") == LIDAR_SCENE,
		"SCENES=%s" % [scenes_table])

	# ============================================================ C. 装配（真实 World + Vehicle）
	var world_packed: PackedScene = load(WORLD_SCENE) as PackedScene
	var vehicle_packed: PackedScene = load(VEHICLE_SCENE) as PackedScene
	var lidar_packed: PackedScene = load(LIDAR_SCENE) as PackedScene
	if world_packed == null or vehicle_packed == null or lidar_packed == null:
		_fatal("C0-scenes-load", "world=%s vehicle=%s lidar=%s"
			% [world_packed, vehicle_packed, lidar_packed])
		_finish()
		return
	var world: Node3D = world_packed.instantiate() as Node3D
	root.add_child(world)
	await _wait_physics(2)
	var vehicles_root: Node3D = world.get_node_or_null("Vehicles") as Node3D
	if vehicles_root == null:
		_fatal("C0-vehicles-root", "World/Vehicles 缺失")
		_finish()
		return
	var vehicle: CharacterBody3D = vehicle_packed.instantiate() as CharacterBody3D
	vehicles_root.add_child(vehicle)
	vehicle.global_position = WorldTransform.PosToGodot(OPEN_WORLD)
	vehicle.rotation.y = 0.0
	await _wait_physics(2)

	# C1 挂载（显式场景 + mount 外参）
	var sensor: Node3D = _attach(vehicle, lidar_packed, {"x": 0.1, "y": 0.0, "z": 0.2}, {})
	if sensor == null:
		_fatal("C1-attach", "AttachSensor 返回 null")
		_finish()
		return
	_check("C1-attach", sensor is Node3D and sensor.get_script() != null
		and (sensor.get_script() as Script).resource_path == LIDAR_SCRIPT,
		"sensor=%s script=%s" % [sensor, sensor.get_script()])

	# C2 节点名 = slot，父 = Vehicle/Sensors
	_check("C2-slot-node", sensor.name == SLOT and sensor.get_parent() == vehicle.sensors_root,
		"name=%s parent=%s" % [sensor.name, sensor.get_parent()])

	# C3 mount {x,z} -> Godot position (x, z, y)
	_check("C3-mount-position", sensor.position.distance_to(Vector3(0.1, 0.2, 0.0)) < 1e-5,
		"position=%s" % [sensor.position])

	# C4 mount.yaw -> rotation.y = YawToGodot(yaw)（换轴唯一入口）
	var yawed: Node3D = _attach(vehicle, lidar_packed,
		{"x": 0.1, "y": 0.0, "z": 0.2, "yaw": 0.5}, {})
	var want_rot := WorldTransform.YawToGodot(0.5)
	_check("C4-mount-yaw", yawed != null and absf(yawed.rotation.y - want_rot) < 1e-5,
		"rotation.y=%.6f want=%.6f" % [
			yawed.rotation.y if yawed != null else 99.0, want_rot])

	# C5 Configure 生效（rays / range / rate_hz）
	if yawed != null:
		yawed.call("Configure", {"rays": 720, "range": 8.0, "rate_hz": 20.0})
	var tuned: Dictionary = yawed.call("GetParams") if yawed != null else {}
	_check("C5-configure", int(tuned.get("rays", -1)) == 720
		and absf(float(tuned.get("range", -1.0)) - 8.0) < 1e-9
		and absf(float(tuned.get("rate_hz", -1.0)) - 20.0) < 1e-9, "params=%s" % [tuned])

	# C6 Vehicle.SampleSensors() 走 ScanOnce 分支（返回 PackedFloat32Array，长度 = 720）
	var samples: Dictionary = vehicle.SampleSensors()
	var sample: Variant = samples.get(SLOT, null)
	_check("C6-sample-sensors", sample is PackedFloat32Array
		and (sample as PackedFloat32Array).size() == 720,
		"type=%s size=%s" % [typeof(sample),
			(sample as PackedFloat32Array).size() if sample is PackedFloat32Array else -1])

	# C7 同 slot 重挂：先卸后挂，Sensors 下只留 1 个子节点（旧节点被释放）
	var old: Node3D = yawed
	sensor = _attach(vehicle, lidar_packed, {"x": 0.1, "y": 0.0, "z": 0.2},
		{"range": DEFAULT_RANGE, "rays": DEFAULT_RAYS, "rate_hz": DEFAULT_RATE_HZ})
	await _wait_physics(1)
	_check("C7-reattach-single", sensor != null and vehicle.sensors_root.get_child_count() == 1
		and sensor != old and not is_instance_valid(old),
		"children=%d same=%s old_valid=%s" % [vehicle.sensors_root.get_child_count(),
			sensor == old, is_instance_valid(old)])

	# ============================================================ D. 几何（核心验收：一圈距离）
	# D1 空旷：全 0.0（未命中 = 0.0，不是 range、不是 inf）
	var ranges: PackedFloat32Array = sensor.call("ScanOnce")
	var all_zero := ranges.size() == DEFAULT_RAYS
	var max_open := 0.0
	for v: float in ranges:
		max_open = maxf(max_open, v)
	_check("D1-open-space", all_zero and max_open == 0.0,
		"size=%d max=%.4f" % [ranges.size(), max_open])

	# D2 正前方 3.4 m 的 box（面 x=103.5，原点 x=100.1）
	var box_fwd: StaticBody3D = _add_box(world, Vector3(104.0, 60.0, 0.5),
		Vector3(1.0, 1.0, 1.0), 2)
	await _wait_physics(2)
	ranges = sensor.call("ScanOnce")
	_check("D2-forward-distance", absf(ranges[0] - BOX_FWD_M) <= TOL,
		"ranges[0]=%.4f want=%.2f" % [ranges[0], BOX_FWD_M])

	# D3 角度覆盖：命中 index ≈ [-8..8]（box 张角 atan(0.5/3.4)=8.4°）
	var hit_indices: Array = []
	for i in range(ranges.size()):
		if ranges[i] > 0.0:
			hit_indices.append(i)
	var span_ok := hit_indices.size() >= 13 and hit_indices.size() <= 21
	var span_detail := "hits=%d" % [hit_indices.size()]
	if not hit_indices.is_empty():
		var min_rel := 999
		var max_rel := -999
		for idx: int in hit_indices:
			var rel: int = idx if idx <= ranges.size() / 2 else idx - ranges.size()
			min_rel = mini(min_rel, rel)
			max_rel = maxi(max_rel, rel)
		span_ok = span_ok and min_rel >= -10 and max_rel <= 10
		span_detail += " rel=[%d..%d]" % [min_rel, max_rel]
	else:
		span_ok = false
	_check("D3-angle-span", span_ok, span_detail)

	# D4 侧向 3.0 m（i=90 -> 局部 +Z -> world +y）
	var box_side: StaticBody3D = _add_box(world, Vector3(100.0, 63.5, 0.5),
		Vector3(1.0, 1.0, 1.0), 2)
	await _wait_physics(2)
	ranges = sensor.call("ScanOnce")
	_check("D4-side-distance", absf(ranges[90] - BOX_SIDE_M) <= TOL,
		"ranges[90]=%.4f want=%.2f" % [ranges[90], BOX_SIDE_M])

	# D5 背向 / 另一侧无回波
	_check("D5-no-echo-behind", ranges[180] == 0.0 and ranges[270] == 0.0,
		"ranges[180]=%.4f ranges[270]=%.4f" % [ranges[180], ranges[270]])

	# D6 车前进 1 m：距离同步减 1（扫描跟着挂载点走）
	vehicle.global_position += Vector3(1.0, 0.0, 0.0)
	await _wait_physics(2)
	ranges = sensor.call("ScanOnce")
	_check("D6-follows-position", absf(ranges[0] - BOX_FWD_AFTER_MOVE_M) <= TOL,
		"ranges[0]=%.4f want=%.2f" % [ranges[0], BOX_FWD_AFTER_MOVE_M])

	# D7 mount.yaw=+90°：前向转到 world +y，命中侧向 box（3.0 m）
	# （先退回 OPEN_WORLD：D6 已把车推到 x=101，侧向 box 的 x 范围是 99.5..100.5）
	vehicle.global_position = WorldTransform.PosToGodot(OPEN_WORLD)
	await _wait_physics(1)
	sensor = _attach(vehicle, lidar_packed, {"x": 0.1, "y": 0.0, "z": 0.2, "yaw": PI * 0.5}, {})
	await _wait_physics(1)
	ranges = sensor.call("ScanOnce")
	_check("D7-mount-yaw-rotates", absf(ranges[0] - BOX_SIDE_M) <= TOL,
		"ranges[0]=%.4f want=%.2f" % [ranges[0], BOX_SIDE_M])

	# D8 其它 avatar 可见：第二台真车（车体高 0.16）放在前 0.75 m，ray 高度 0.1 m
	var vehicle_b: CharacterBody3D = vehicle_packed.instantiate() as CharacterBody3D
	vehicles_root.add_child(vehicle_b)
	vehicle_b.global_position = WorldTransform.PosToGodot(Vector3(101.0, 60.0, 0.0))
	vehicle_b.rotation.y = 0.0
	sensor = _attach(vehicle, lidar_packed, {"x": 0.1, "y": 0.0, "z": 0.1}, {})
	await _wait_physics(2)
	ranges = sensor.call("ScanOnce")
	_check("D8-other-vehicle-visible", ranges[0] > 0.0 and ranges[0] < BOX_SIDE_M
		and absf(ranges[0] - VEHICLE_B_M) <= 0.1,
		"ranges[0]=%.4f want≈%.2f" % [ranges[0], VEHICLE_B_M])

	# D9 自车不可见：排除表 = [宿主 RID]，且 mount 落进自车体内部时没有 ~0 m 命中
	var rids: Variant = sensor.call("_SelfRidArray")
	var rid_ok: bool = rids is Array and (rids as Array).size() == 1 \
		and (rids as Array)[0] == vehicle.get_rid()
	vehicle_b.queue_free()
	await _wait_physics(2)
	sensor = _attach(vehicle, lidar_packed, {"x": 0.0, "y": 0.0, "z": 0.08}, {})
	await _wait_physics(1)
	ranges = sensor.call("ScanOnce")
	var min_hit := 999.0
	for v: float in ranges:
		if v > 0.0:
			min_hit = minf(min_hit, v)
	_check("D9-self-excluded", rid_ok and min_hit >= 0.05,
		"rids=%s self_rid=%s min_hit=%.4f" % [rids, vehicle.get_rid(), min_hit])

	# D10 LaserScan（task_30 §7.4）：points / angle / intensity / scan_freq / scan_time / stamp
	sensor = _attach(vehicle, lidar_packed, {"x": 0.1, "y": 0.0, "z": 0.2}, {})
	await _wait_physics(1)
	ranges = sensor.call("ScanOnce")
	var scan: Dictionary = sensor.call("GetLaserScan")
	var points_list: Array = scan.get("points", [])
	var angle_ok := points_list.size() == DEFAULT_RAYS
	var intensity_ok := angle_ok
	var angle_detail := "points=%d" % [points_list.size()]
	if angle_ok:
		for i in range(points_list.size()):
			var point: Dictionary = points_list[i]
			var want_angle := TAU * float(i) / float(DEFAULT_RAYS)
			if absf(float(point.get("angle", 9.0)) - want_angle) > 1e-6:
				angle_ok = false
				angle_detail += " i=%d angle=%.6f want=%.6f" % [i,
					float(point.get("angle", 0.0)), want_angle]
				break
			var is_hit := float(ranges[i]) > 0.0
			var want_intensity := 255.0 if is_hit else 0.0
			if absf(float(point.get("intensity", -1.0)) - want_intensity) > 1e-6:
				intensity_ok = false
				break
	var freq_ok := absf(float(scan.get("scan_freq", -1.0)) - DEFAULT_RATE_HZ) < 1e-9
	var time_ok := absf(float(scan.get("scan_time", -1.0)) - 0.1) < 1e-9
	var stamp_ok := int(scan.get("stamp", 0)) > 0
	_check("D10-laserscan", angle_ok and intensity_ok and freq_ok and time_ok and stamp_ok,
		"%s intensity=%s freq=%s time=%.4f stamp=%s" % [angle_detail, intensity_ok,
			scan.get("scan_freq"), float(scan.get("scan_time", 0.0)), scan.get("stamp")])

	# D11 量程裁剪：range=1.0 时 2.4 m 的 box 不再命中
	sensor.call("Configure", {"range": 1.0})
	await _wait_physics(1)
	ranges = sensor.call("ScanOnce")
	var clipped := true
	for v: float in ranges:
		if v > 0.0:
			clipped = false
	_check("D11-range-clipping", clipped and ranges[0] == 0.0,
		"ranges[0]=%.4f max=%.4f" % [ranges[0], _max_of(ranges)])

	# D12 全部 0 ≤ range ≤ range_limit
	sensor.call("Configure", {"range": DEFAULT_RANGE})
	await _wait_physics(1)
	ranges = sensor.call("ScanOnce")
	var bounded := true
	for v: float in ranges:
		if v < 0.0 or v > DEFAULT_RANGE + 1e-6:
			bounded = false
	_check("D12-range-bounded", bounded, "max=%.4f limit=%.2f" % [_max_of(ranges),
		DEFAULT_RANGE])

	# 清理几何（后续 F7 用真实墙体，不能被自建 box 干扰）
	box_fwd.queue_free()
	box_side.queue_free()
	await _wait_physics(2)

	# ============================================================ E. 时序 / 帧格式
	var holder := Node3D.new()
	world.add_child(holder)
	holder.global_position = WorldTransform.PosToGodot(Vector3(64.0, 64.0, 0.2))
	var timing: Node3D = lidar_packed.instantiate() as Node3D
	holder.add_child(timing)
	timing.call("Configure", {"rays": DEFAULT_RAYS, "rate_hz": DEFAULT_RATE_HZ})
	var counter := {"n": 0}
	var counter_cb := func(_r: PackedFloat32Array, _t: float, _s: int) -> void:
		counter["n"] = int(counter["n"]) + 1
	timing.connect("scan_produced", counter_cb)
	await _wait_physics(60)
	# E1 10 Hz：60 帧（= 1 s）恰好 10 圈（P6：容差 ACCUM_EPS 让第 6 帧就能触发）
	_check("E1-rate-10hz", int(counter["n"]) == 10, "scans=%d want=10" % [int(counter["n"])])

	# E2 20 Hz：60 帧 ≈ 20 圈（±1）
	counter["n"] = 0
	timing.call("Configure", {"rate_hz": 20.0})
	await _wait_physics(60)
	var scans_20 := int(counter["n"])
	# E3 无 link 时仍 emit，且全程无 harness 错误
	_check("E2-rate-20hz", absi(scans_20 - 20) <= 1, "scans=%d want≈20" % [scans_20])
	_check("E3-emit-without-link", scans_20 > 0 and _harness_errors == 0
		and not bool(timing.call("HasStream")),
		"scans=%d harness_errors=%d has_stream=%s" % [scans_20, _harness_errors,
			timing.call("HasStream")])

	# E4/E5 有 link：每圈 1 帧 1455 B，帧头 type/slot_id/seq/world_time 正确，payload 与距离一致
	var link := FakeLink.new()
	timing.call("SetStream", link, SENSOR_ID)
	timing.call("Configure", {"rate_hz": 10.0})
	var matching := {"ok": true, "checked": 0}
	var match_cb := func(scan_ranges: PackedFloat32Array, _t: float, _s: int) -> void:
		if link.frames.is_empty():
			return
		var frame: PackedByteArray = link.frames[link.frames.size() - 1]
		if not _frame_matches(frame, scan_ranges):
			matching["ok"] = false
		matching["checked"] = int(matching["checked"]) + 1
	timing.connect("scan_produced", match_cb)
	await _wait_physics(60)
	var frames_ok := link.frames.size() == 10
	var header_ok := frames_ok
	var seq_ok := frames_ok
	var time_ok_e4 := frames_ok
	var last_time := -1.0
	for i in range(link.frames.size()):
		var frame: PackedByteArray = link.frames[i]
		if frame.size() != FRAME_BYTES_360:
			frames_ok = false
			continue
		var header: Dictionary = WireProtocol.ParseFrameHeader(frame)
		if int(header.get("type", -1)) != WireProtocol.FRAME_TYPE_LIDAR \
				or int(header.get("slot_id", -1)) != SENSOR_ID:
			header_ok = false
		if int(header.get("seq", -1)) != i + 1:
			seq_ok = false
		var stamp := float(header.get("world_time", -1.0))
		if stamp < last_time:
			time_ok_e4 = false
		last_time = stamp
	_check("E4-frame-format", frames_ok and header_ok and seq_ok and time_ok_e4,
		"frames=%d size=%s type=%s slot=%s seq=%s monotonic=%s" % [link.frames.size(),
			FRAME_BYTES_360 if frames_ok else "?",
			"lidar" if header_ok else "BAD", SENSOR_ID if header_ok else "BAD",
			seq_ok, time_ok_e4])
	# E5 payload f32 与同期 GetLastRanges() 一致（每圈都比对）
	_check("E5-frame-payload", bool(matching["ok"]) and int(matching["checked"]) == 10,
		"checked=%d ok=%s" % [int(matching["checked"]), matching["ok"]])

	# E6 ClearStream 后不再发帧（扫描继续）
	var frames_before := link.frames.size()
	timing.call("ClearStream")
	await _wait_physics(60)
	_check("E6-clear-stream", link.frames.size() == frames_before
		and not bool(timing.call("HasStream")),
		"frames=%d -> %d has_stream=%s" % [frames_before, link.frames.size(),
			timing.call("HasStream")])
	holder.queue_free()
	await _wait_physics(2)

	# ============================================================ F. 边界 / 白盒
	# F1 非法参数回退默认（树外实例：不碰物理、不扫）
	var bad: Node3D = lidar_packed.instantiate() as Node3D
	var fallbacks_ok := true
	var fallback_detail := ""
	for pair: Array in [[{"rays": 0}, 360], [{"rays": -1}, 360], [{"rays": "x"}, 360],
			[{"range": 0}, 12], [{"range": -1.0}, 12], [{"rate_hz": 0}, 10]]:
		var cfg: Dictionary = pair[0]
		var want: float = float(pair[1])
		bad.call("Configure", cfg)
		var got: Dictionary = bad.call("GetParams")
		var key: String = "rays" if cfg.has("rays") else ("range" if cfg.has("range") else "rate_hz")
		var value := float(got.get(key, -999.0))
		if absf(value - want) > 1e-6:
			fallbacks_ok = false
			fallback_detail += " %s->%s(want %s)" % [cfg, value, want]
	_check("F1-invalid-params", fallbacks_ok, fallback_detail if not fallbacks_ok
		else "6 组非法值全部回退默认")
	bad.free()

	# F2 树外 ScanOnce：size=360 全 0，且不产生 ERROR（R5）
	var lone: Node3D = lidar_packed.instantiate() as Node3D
	var lone_ranges: PackedFloat32Array = lone.call("ScanOnce")
	var lone_ok := lone_ranges.size() == DEFAULT_RAYS
	for v: float in lone_ranges:
		if v != 0.0:
			lone_ok = false
	_check("F2-scan-outside-tree", lone_ok and _harness_errors == 0,
		"size=%d max=%.4f" % [lone_ranges.size(), _max_of(lone_ranges)])
	lone.free()

	# F3 DetachSensor：节点立刻消失（先 remove_child 再 queue_free）
	vehicle.DetachSensor(SLOT)
	var detached_ok := vehicle.get_node_or_null("Sensors/%s" % SLOT) == null \
		and vehicle.GetSensor(SLOT) == null
	_check("F3-detach", detached_ok, "node=%s holder=%s" % [
		vehicle.get_node_or_null("Sensors/%s" % SLOT), vehicle.GetSensor(SLOT)])

	# F4 白盒 lint：越界符号 / 不存在的 API 一律禁止
	var code := _code_of(LIDAR_SCRIPT)
	var banned_hits: Array = []
	for token: String in BANNED_TOKENS:
		if code.contains(token):
			banned_hits.append(token)
	_check("F4-white-box-lint", banned_hits.is_empty(), "banned=%s" % [banned_hits])

	# F5 头注释齐全
	var header_src := _read_text(LIDAR_SCRIPT)
	var header_ok_f5 := header_src.begins_with("# Presented by KeJi") \
		and header_src.contains("Created Date") and header_src.contains("Modified Date")
	_check("F5-file-header", header_ok_f5, "bytes=%d" % [header_src.length()])

	# F6 SetStream 不 preload ComponentLink（net 层解耦，R6）
	_check("F6-no-component-link", not header_src.contains("res://src/net/ComponentLink.gd"),
		"ComponentLink refs=%d" % [header_src.count("ComponentLink")])

	# F7 真实世界几何：车 (60,90,0)、yaw=+90° → 车头正对 wall_c（世界 y=92，P2）
	vehicle.global_position = WorldTransform.PosToGodot(WALL_WORLD)
	vehicle.rotation.y = WorldTransform.YawToGodot(PI * 0.5)
	sensor = _attach(vehicle, lidar_packed, {"x": 0.0, "y": 0.0, "z": 0.2},
		{"range": DEFAULT_RANGE, "rays": DEFAULT_RAYS, "rate_hz": DEFAULT_RATE_HZ})
	await _wait_physics(2)
	ranges = sensor.call("ScanOnce")
	_check("F7-real-wall-distance", absf(ranges[0] - WALL_HIT_M) <= TOL,
		"ranges[0]=%.4f want=%.3f" % [ranges[0], WALL_HIT_M])

	# F8 真实墙的命中数落在 [140,165]（P2 实测 152），且全部 ≤ range_limit
	var wall_hits := 0
	var wall_bounded := true
	for v: float in ranges:
		if v > 0.0:
			wall_hits += 1
		if v > DEFAULT_RANGE + 1e-6:
			wall_bounded = false
	_check("F8-real-wall-hits", wall_hits >= 140 and wall_hits <= 165 and wall_bounded,
		"hits=%d want=[140,165] bounded=%s" % [wall_hits, wall_bounded])

	# ============================================================ G. 收尾
	_check("G1-no-harness-errors", _harness_errors == 0, "errors=%d" % [_harness_errors])
	root.remove_child(world)
	world.queue_free()
	await _wait_physics(2)
	_check("G2-cleanup", not is_instance_valid(vehicle) and not is_instance_valid(world),
		"vehicle_valid=%s world_valid=%s" % [is_instance_valid(vehicle),
			is_instance_valid(world)])
	_finish()


# ================================================================ 辅助

## 在当前 vehicle 上重挂传感器（同 slot = 先卸后挂）。
func _attach(vehicle: Node, scene: PackedScene, mount: Dictionary,
		params: Dictionary) -> Node3D:
	var node: Node = vehicle.call("AttachSensor", SLOT, scene, mount, params)
	return node as Node3D


## 自建实心障碍（StaticBody3D + BoxShape3D）：位置 / 尺寸都用**世界坐标**（米）。
func _add_box(parent: Node3D, world_center: Vector3, size: Vector3,
		layer: int) -> StaticBody3D:
	var body := StaticBody3D.new()
	body.collision_layer = layer
	body.collision_mask = 0
	body.position = WorldTransform.PosToGodot(world_center)
	var shape := CollisionShape3D.new()
	var box := BoxShape3D.new()
	box.size = size
	shape.shape = box
	body.add_child(shape)
	parent.add_child(body)
	return body


## 帧 payload（f32 小端）是否与这一圈的距离逐点相等。
func _frame_matches(frame: PackedByteArray, ranges: PackedFloat32Array) -> bool:
	if frame.size() < WireProtocol.FRAME_HEADER_SIZE:
		return false
	if frame.size() - WireProtocol.FRAME_HEADER_SIZE != ranges.size() * 4:
		return false
	for i in range(ranges.size()):
		var value := frame.decode_float(WireProtocol.FRAME_HEADER_SIZE + i * 4)
		if absf(value - ranges[i]) > 1e-6:
			return false
	return true


func _max_of(values: PackedFloat32Array) -> float:
	var out := 0.0
	for v: float in values:
		out = maxf(out, v)
	return out


## 字典逐键逐类型相同（键集 + 类型 + 值）。
func _dict_equal(a: Dictionary, b: Dictionary) -> bool:
	if a.size() != b.size():
		return false
	for key: Variant in a.keys():
		if not b.has(key):
			return false
		if typeof(a[key]) != typeof(b[key]):
			return false
		if a[key] != b[key]:
			return false
	return true


func _read_text(path: String) -> String:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return ""
	var text := file.get_as_text()
	file.close()
	return text


## 去掉以 # 开头的注释行，只留可执行代码（白盒 lint 用）。
func _strip_comment_lines(source: String) -> String:
	var code := ""
	for raw_line: String in source.split("\n"):
		if raw_line.strip_edges().begins_with("#"):
			continue
		code += raw_line + "\n"
	return code


func _code_of(path: String) -> String:
	var script: Script = load(path) as Script
	if script == null:
		return ""
	return _strip_comment_lines(script.source_code)


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		await physics_frame


# ================================================================ 断言与收尾

func _check(label: String, ok: bool, detail: String = "") -> bool:
	_total += 1
	if ok:
		print("[check] ", label, " ok", (" " + detail) if not detail.is_empty() else "")
		return true
	_failed += 1
	print("[check] ", label, " FAIL", (" " + detail) if not detail.is_empty() else "")
	return false


## 结构性前置条件失败：计数 + 中止（成功路径不占断言编号，保证 total 稳定）。
func _fatal(label: String, detail: String) -> void:
	_total += 1
	_failed += 1
	_harness_errors += 1
	print("[check] ", label, " FAIL ", detail)


func _finish() -> void:
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
