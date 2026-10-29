# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-28
#
# 世界根脚本（COMPONENTS.md §4.1）：启动世界时钟、按 [sim_world] 程序化生成体素世界、
# 提供尺度/坐标/容器访问。D4 程序化生成，布局确定性、无 RNG。
#
# task_38 Step 1b：世界观感（地面/体素材质 + 天空 + 太阳方向）全部**运行时**建，
# 入口是 WorldLook.gd（COMPONENTS.md §4.1.1）；World.tscn 零改动。
#
# 尺度唯一来源是 project.godot 的 [sim_world]（COMPONENTS.md §0.2-5），本文件不写字面量尺度。
# 例外（task_32 §三.2 明令"写死，不得自行发挥"）：不可见空气墙 / 顶盖的 5 个 box 用字面量几何表
# AIR_WALL_BOXES，与现有设置数值一致（256×0.5=128 m、64×0.5=32 m），一致性由 check_bounds.gd 的
# B1b/B1c 数值断言兜住；改世界尺度必须同步改该表 + E24 + check_bounds 常量（三处一起）。
# 坐标换轴只经 WorldTransform（task_30 §7.3）：WorldToCell 内部即 PosToGodot，本文件无换轴字面量。

extends Node3D

const WorldClock := preload("res://src/common/WorldClock.gd")
const WorldTransform := preload("res://src/common/WorldTransform.gd")
## 观感（材质 + 天空 + 太阳）唯一入口（task_38 §二.3 / COMPONENTS.md §4.1.1）。
const WorldLook := preload("res://src/world/WorldLook.gd")
## 街道几何 / 建筑立面的唯一入口（task_38 §三.4 / COMPONENTS.md §4.1.2）；运行期建、不进 .tscn。
const StreetLayout := preload("res://src/world/StreetLayout.gd")
const Facade := preload("res://src/world/Facade.gd")

const SETTING_VOXEL_SIZE := "sim_world/world/voxel_size"
const SETTING_CELLS_X := "sim_world/world/cells_x"
const SETTING_CELLS_Y := "sim_world/world/cells_y"
const SETTING_LAYERS := "sim_world/world/layers"

const VOXEL_ITEM := 0
const VOXEL_COLOR := Color(0.55, 0.58, 0.62, 1.0)

## 观感总开关（task_38 §二.3）：false = 不换地面材质 / 不换天空 / 不动太阳，世界回到 task_37 的单色观感。
## 缺资产时**不用**改它：WorldLook 自己降级（只 warn）。check_world_look 直接断言它为 true。
const LOOK_ENABLED := true

## 出生点（世界坐标，米），与 pleiades-sim 的 DEFAULT_ORIGIN (64, 64, 0) 一致。
const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)
## 出生点净空半径（米，水平）与净空高度（米）——Step 6 直接把 avatar 放在这里。
const SPAWN_CLEAR_RADIUS_M := 3.0
const SPAWN_CLEAR_HEIGHT_M := 4.0

## 静态障碍布局：origin / size 均为**世界坐标（Z-up，米）**。
## 只做边界墙 + 稀疏块（≈1.2 万格），绝不填满 256×256×64（419 万格）。
const OBSTACLES := [
	# 边界墙 ×4：标出 128×128 m 世界范围，高 2 m
	{"name": "wall_south", "origin": Vector3(0.0, 0.0, 0.0), "size": Vector3(128.0, 0.5, 2.0)},
	{"name": "wall_north", "origin": Vector3(0.0, 127.5, 0.0), "size": Vector3(128.0, 0.5, 2.0)},
	{"name": "wall_west", "origin": Vector3(0.0, 0.0, 0.0), "size": Vector3(0.5, 128.0, 2.0)},
	{"name": "wall_east", "origin": Vector3(127.5, 0.0, 0.0), "size": Vector3(0.5, 128.0, 2.0)},
	# 建筑 A / B：低层块状障碍
	{"name": "building_a", "origin": Vector3(24.0, 24.0, 0.0), "size": Vector3(10.0, 8.0, 4.0)},
	{"name": "building_b", "origin": Vector3(88.0, 28.0, 0.0), "size": Vector3(8.0, 10.0, 5.0)},
	# 长墙 C / D：走廊与直角绕行场景
	{"name": "wall_c", "origin": Vector3(36.0, 92.0, 0.0), "size": Vector3(30.0, 1.0, 2.0)},
	{"name": "wall_d", "origin": Vector3(34.0, 100.0, 0.0), "size": Vector3(1.0, 24.0, 3.0)},
	# 柱 E：8 m 高，UAV 高度参照
	{"name": "pillar_e", "origin": Vector3(78.0, 78.0, 0.0), "size": Vector3(2.0, 2.0, 8.0)},
	# 三级退台 F1/F2/F3：视觉层次
	{"name": "step_f1", "origin": Vector3(102.0, 100.0, 0.0), "size": Vector3(8.0, 8.0, 1.0)},
	{"name": "step_f2", "origin": Vector3(103.0, 101.0, 0.0), "size": Vector3(6.0, 6.0, 2.0)},
	{"name": "step_f3", "origin": Vector3(104.0, 102.0, 0.0), "size": Vector3(4.0, 4.0, 3.0)},
	# 散块 ×3：小障碍点缀
	{"name": "block_1", "origin": Vector3(60.0, 100.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
	{"name": "block_2", "origin": Vector3(66.0, 100.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
	{"name": "block_3", "origin": Vector3(72.0, 104.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
]

# ---------------------------------------------------------------- 空气墙 / 顶盖（task_32 Step 2）
## 不可见硬边界（task_32 §三.2 写死）：可见 2 m 体素矮墙保留，外面再套 32 m 空气墙 + 32 m 顶盖。
## 碰撞面**内侧面正好贴世界边界平面**（世界 x=0/128、y=0/128、z=32），厚度一律**向外**，
## 车中心因此永远 ≥ 边界平面，体素格坐标不会变负（COMPONENTS.md §9.1 E24）。
const AIR_WALL_HEIGHT_M := 32.0
const AIR_WALL_THICKNESS_M := 1.0
const AIR_WALL_LAYER := 1

## 5 个 BoxShape3D 的 (中心, 尺寸)，**Godot 坐标（Y-up，米）**：godot = (world.x, world.z, world.y)。
## 顺序 = 南墙 / 北墙 / 西墙 / 东墙 / 顶盖，与 task_32 §三.2 的表格逐行一一对应（勿调整顺序）。
const AIR_WALL_BOXES := [
	# 南墙：世界 x∈[0,128], y∈[-1,0], z∈[0,32] —— 内侧面 godot z=0
	[Vector3(64.0, 16.0, -0.5), Vector3(128.0, 32.0, 1.0)],
	# 北墙：世界 y∈[128,129] —— 内侧面 godot z=128
	[Vector3(64.0, 16.0, 128.5), Vector3(128.0, 32.0, 1.0)],
	# 西墙：世界 x∈[-1,0] —— 内侧面 godot x=0
	[Vector3(-0.5, 16.0, 64.0), Vector3(1.0, 32.0, 128.0)],
	# 东墙：世界 x∈[128,129] —— 内侧面 godot x=128
	[Vector3(128.5, 16.0, 64.0), Vector3(1.0, 32.0, 128.0)],
	# 顶盖：世界 z∈[32,33] —— 内表面（下表面）godot y=32
	[Vector3(64.0, 32.5, 64.0), Vector3(128.0, 1.0, 128.0)],
]

# ---------------------------------------------------------------- 外部驱动的行人（task_35 Step 3）
## 行人底盘场景：与 `ChassisCatalog.SCENES["pedestrian"]` 同一路径。
## 用 load() 而非 preload()（与 PERSON_SCENE 同一取舍）：glb/场景尚未 import 时只 warn，
## 不让整个世界脚本解析失败（COMPONENTS.md §9.1 E8 的反面）。
const PEDESTRIAN_SCENE := "res://src/scenes/props/Pedestrian.tscn"
## 容器名**必须**与 `WebSocketManager.PEDESTRIANS_NODE` 逐字一致（"Pedestrians"）：
## 同一个容器既装世界预置的人（本表），也装 `chassis_hello{kind:"pedestrian"}` 动态装配的人。
const PEDESTRIANS_NODE := "Pedestrians"
## 街道 / 立面容器名（task_38 §三.4，自检 Get_Street_Root() / Get_Facade_Root() 用）。
const STREET_NODE := "Street"
const FACADE_NODE := "Facade"

## 行人布局表：**只有**出生点（世界坐标 Z-up，米）+ 初始朝向 + 数量。
## **没有**路径 / 巡逻 / 计时 / 速度 —— 世界不再自己让人走（task_35 Step 3）：
## 人由外部（Python / 自检 / Step 5 的车撞人）通过 Pedestrian.SetCommand 驱动；
## **没有外部连接时人就站着不动，这是预期行为，不是故障**。
## yaw_deg：0 = 朝 +X（东），180 = 朝 −X（西）。入树后由 Configure() 把 _yaw_target 对齐到该朝向。
##
## 选点依据（逐条按 OBSTACLES 实算；比的是**度**不是"看起来空"）：
##   ① 无障碍：pedestrian_01 最近的障碍是 wall_c（y∈[92,93]、x∈[36,66]）→ 3.000 m；
##      pedestrian_02 最近的是 block_1（x∈[60,61]、y∈[100,101]）→ 8.062 m。
##      判据 > PED_CLEAR_M(0.5) = 胶囊半径 0.25 + 0.25 余量。
##   ② 离世界边界：min(44, 96, 128−44, 128−96) = 32 m / min(52,102,76,26) = 26 m，判据 ≥ 1.0 m。
##   ③ 离出生点 SPAWN_WORLD(64,64)：37.736 / 39.850 m，判据 > SPAWN_CLEAR_RADIUS_M(3.0)
##      （也就远于 manager 的 8 方位 × 2 m/环 avatar 排位）。
##   ⑤ 两人互相 10.000 m ≥ 1.0 m（镜像 manager 的 PEDESTRIAN_SPAWN_MIN_GAP_M）。
##   ⑥ 离雷达热区：check_lidar 的真墙热区 (60,90) 17.088 / 14.422 m、空场热区 (100,60) 66.573 / 63.781 m、
##      check_camera_shots 的停车点 (60,84) 20.000 / 19.698 m —— 全部 > 12 m 的 radar range。
##      行人 collision_layer=2 会被 `Lidar2D`（scan_mask=1|2|4）扫到，这是刻意保留的（"雷达看得见人"），
##      但**绝不能**落进既有雷达/拍照自检的 12 m 热区，否则 F7/F8 的 2.000 m / 152 命中会红。
##   ⑦ 离协议 spawn 点：默认 (64,56) 44.7/47.5 m、check_ws_manager K1 (70,70) 36.8/36.7 m、
##      K14 (72,74) / K16 (72,78) 均 > 30 m —— 动态行人与世界行人**不会叠人**。
##   ⑧ 离车辆路径：距 (64,64) 37.7/39.8 m、距 (126.5,64) > 70 m；人**有碰撞**（Step 1 取舍，
##      Vehicle.mask=7 含 layer 2），落在车道上会挡车 —— 这几个点不在任何既有自检的行车路线上。
##   x∈[44,56] × y∈[96,106] 这块 12×10 m 空地是 walking_person 时期就用 1.0 m 臂展余量对 15 个
##   OBSTACLES 全量断言过的区域，现在正好"站在广场上、离长墙 3 m"。yaw 一东一西，
##   用来证明初始朝向按表落地（yaw=180° 那个也是 Configure() 对齐逻辑的活体证据）。
const PEDESTRIANS := [
	{"name": "pedestrian_01", "pos": Vector3(44.0, 96.0, 0.0), "yaw_deg": 0.0},
	{"name": "pedestrian_02", "pos": Vector3(52.0, 102.0, 0.0), "yaw_deg": 180.0},
]

var _voxel_size: float = 0.5
var _cells_x: int = 256
var _cells_y: int = 256
var _layers: int = 64
var _built_cells: int = 0

@onready var _voxel_world: GridMap = $VoxelWorld
@onready var _vehicles_root: Node3D = $Vehicles
var _pedestrians_root: Node3D = null


func _ready() -> void:
	WorldClock.EnsureStarted()
	_read_scale_settings()
	_log_scale()
	BuildVoxelWorld()
	if LOOK_ENABLED:
		Apply_World_Look()
	_Build_Bounds()
	BuildPedestrians()
	# 街道几何 / 立面（task_38 §三.4 的末尾两行）：只建渲染 mesh、零碰撞、可幂等重建。
	StreetLayout.Build_All(self)
	Facade.Build_All(self)


## 应用观感（地面材质 + 天空 + 太阳方向）：幂等；缺资产只 warn。
## 返回 false = 至少一样降级（缺失清单见 WorldLook.Get_Missing_Assets()）。
func Apply_World_Look() -> bool:
	var ok: bool = WorldLook.Apply_All(self)
	if not ok:
		print("[world] look degraded（缺资产，保持原单色观感）：", str(WorldLook.Get_Missing_Assets()))
	return ok


## 程序化重建体素世界：清空 -> 建 MeshLibrary -> 稀疏填格。可重复调用（幂等）。
func BuildVoxelWorld() -> void:
	var started_usec := Time.get_ticks_usec()
	_voxel_world.clear()
	_built_cells = 0

	var size := Vector3(_voxel_size, _voxel_size, _voxel_size)
	if not _voxel_world.cell_size.is_equal_approx(size):
		push_warning("[world] VoxelWorld.cell_size=%s 与 [sim_world] 的 %s 不一致，以设置为准"
			% [_voxel_world.cell_size, size])
	_voxel_world.cell_size = size
	_voxel_world.mesh_library = _make_mesh_library()

	for obstacle: Dictionary in OBSTACLES:
		_fill_block_world(obstacle["origin"], obstacle["size"])

	var elapsed_ms := float(Time.get_ticks_usec() - started_usec) / 1000.0
	print("[world] voxel world built: ", _built_cells, " cells in ", "%.2f" % elapsed_ms, " ms")
	_warn_if_spawn_blocked()


## 代码构建不可见硬边界：1 个 StaticBody3D(Bounds) + 5 个 CollisionShape3D。
## 不写进 .tscn（避开 COMPONENTS.md §9.1 E16 的 node_paths 坑）；**不建任何 Mesh 渲染节点**
## （必须不可见，§9.1 E24）。
## 幂等：重复调用先摘掉旧的 Bounds 再重建，不会累积重复形状（check_bounds.gd 的 B1e 直接断言）。
func _Build_Bounds() -> void:
	var old: Node = get_node_or_null("Bounds")
	if old != null:
		push_warning("[world] Bounds 已存在，按代码重建（若来自 .tscn 请删掉该节点）")
		remove_child(old)
		old.queue_free()

	var body := StaticBody3D.new()
	body.name = "Bounds"
	body.collision_layer = AIR_WALL_LAYER
	body.collision_mask = 0
	for box: Array in AIR_WALL_BOXES:
		var center: Vector3 = box[0]
		var extent: Vector3 = box[1]
		var shape := BoxShape3D.new()
		shape.size = extent
		var node := CollisionShape3D.new()
		node.shape = shape
		node.position = center
		body.add_child(node)
	add_child(body)
	print("[world] bounds built: ", AIR_WALL_BOXES.size(), " shapes, layer=", AIR_WALL_LAYER,
		", mask=0, height=", AIR_WALL_HEIGHT_M, " m, thickness=", AIR_WALL_THICKNESS_M, " m")


## 程序化摆放静态人物（纯视觉、无碰撞、无 RNG）：先摘掉同名旧容器再重建（幂等）。
## 容器由代码建、不写进 World.tscn —— 与 Bounds 同策略（代码建、幂等重建），
## 这样 check_bounds.gd 的 B1d（读 .tscn 文本数节点）不受影响。
## 缺资产/缺场景只 push_warning（绝不 push_error：否则 --quit 的 "0 ERROR" 验收假红）。
## 程序化摆放"外部驱动的行人"（task_35 Step 3）——
## 先摘掉同名旧容器再重建（幂等，与 Bounds 同策略）。
## **不内置任何行为**：不调 SetCommand、不设速度、不建路径；人站着等外部驱动
## （无 Python 连接时站住不动 = 预期）。容器代码建、不写进 World.tscn。
## 缺场景/缺资产只 push_warning（**绝不 push_error**：--quit 的 0 ERROR 验收靠这条）。
##
## 注意：运行期重建会**连带移除**已装配的网络行人（manager 的缓存指针会因
## is_instance_valid()==false 自动重解析，不会悬空，但那个 peer 的 avatar 就没了）。
## 因此本函数只应在 `_ready()` 与自检里调用（COMPONENTS.md §4.11）。
func BuildPedestrians() -> void:
	var old: Node = get_node_or_null(PEDESTRIANS_NODE)
	if old != null:
		remove_child(old)
		old.queue_free()

	_pedestrians_root = Node3D.new()
	_pedestrians_root.name = PEDESTRIANS_NODE
	add_child(_pedestrians_root)

	# 前置判断：glb / 场景没 import 时 load() 会打 ERROR；exists() 先探一次，缺了只 warn
	# （既有自检也会载入 World，不能因为它们没 import 就红）。
	if not ResourceLoader.exists(PEDESTRIAN_SCENE):
		push_warning("[world] 行人场景缺失（未 import？）：%s" % PEDESTRIAN_SCENE)
		return
	var packed: PackedScene = load(PEDESTRIAN_SCENE) as PackedScene
	if packed == null:
		push_warning("[world] 行人场景不可加载：%s" % PEDESTRIAN_SCENE)
		return

	for spec: Dictionary in PEDESTRIANS:
		var ped: Node3D = packed.instantiate() as Node3D
		if ped == null:
			push_warning("[world] 行人实例化失败：%s" % spec.get("name", "?"))
			continue
		ped.name = String(spec["name"])
		# 先入树再写 global_transform：树外写 global_* 会 ERROR（check_models.gd P8）。
		_pedestrians_root.add_child(ped)
		ped.global_transform = WorldTransform.MakeTransform(
			spec["pos"], deg_to_rad(float(spec.get("yaw_deg", 0.0))))
		# **必须**在写完 transform 之后 Configure：Pedestrian.Configure() 末尾把
		# `_yaw_target = rotation.y`；不调它的话 yaw_deg≠0 的人第一帧会被 Step() 拧回 yaw=0
		# （plan.md §7 R1）。参数用 `{}`：目录默认值（max_speed 1.26 / max_turn_rate 180 °/s）。
		ped.call("Configure", ped, {})

	print("[world] pedestrians built: ", _pedestrians_root.get_child_count(),
		" (", PEDESTRIANS.size(), " specs, idle until driven)")


## 行人容器（自检 / Step 4/5 / manager 用；与 GetVehiclesRoot 同风格）。
## 每次调用都复检 is_instance_valid：容器可能被 BuildPedestrians() 重建。
func GetPedestriansRoot() -> Node3D:
	if _pedestrians_root == null or not is_instance_valid(_pedestrians_root):
		_pedestrians_root = get_node_or_null(PEDESTRIANS_NODE) as Node3D
	return _pedestrians_root


## 街道容器（自检 / 后续任务用；与 GetPedestriansRoot 同风格，每次重新解析）。
func Get_Street_Root() -> Node3D:
	return StreetLayout.Get_Root(self)


## 立面容器（同上）。
func Get_Facade_Root() -> Node3D:
	return Facade.Get_Root(self)


## 世界坐标（Z-up，米）-> Godot 体素索引。换轴只在 WorldTransform 里发生。
## 等价于 COMPONENTS.md §2.3 的 (floor(wx/v), floor(wz/v), floor(wy/v))。
func WorldToCell(world_pos: Vector3) -> Vector3i:
	var v := _voxel_size
	var g := WorldTransform.PosToGodot(world_pos)
	return Vector3i(floori(g.x / v), floori(g.y / v), floori(g.z / v))


## 所有 avatar 的父节点（Step 4/6 由 WebSocketManager 填充）。
func GetVehiclesRoot() -> Node3D:
	return _vehicles_root


# ---------------------------------------------------------------- 私有

## 从 [sim_world] 读尺度；缺键时保留默认值并告警（唯一来源原则）。
func _read_scale_settings() -> void:
	_voxel_size = float(ProjectSettings.get_setting(SETTING_VOXEL_SIZE, _voxel_size))
	_cells_x = int(ProjectSettings.get_setting(SETTING_CELLS_X, _cells_x))
	_cells_y = int(ProjectSettings.get_setting(SETTING_CELLS_Y, _cells_y))
	_layers = int(ProjectSettings.get_setting(SETTING_LAYERS, _layers))
	if _voxel_size <= 0.0:
		push_warning("[world] %s=%s 非法，回退 0.5" % [SETTING_VOXEL_SIZE, _voxel_size])
		_voxel_size = 0.5
	if _cells_x <= 0 or _cells_y <= 0 or _layers <= 0:
		push_warning("[world] [sim_world] cells/layers 非法，回退 256x256x64")
		_cells_x = 256
		_cells_y = 256
		_layers = 64


func _log_scale() -> void:
	print("[world] voxel=", _voxel_size, " m, cells=", _cells_x, "x", _cells_y, "x", _layers,
		" -> ", _cells_x * _voxel_size, " x ", _cells_y * _voxel_size, " x ", _layers * _voxel_size,
		" m, spawn=(", SPAWN_WORLD.x, ", ", SPAWN_WORLD.y, ", ", SPAWN_WORLD.z, ")")


## 立方体 mesh + 立方体碰撞形状：渲染与物理共用同一边长（= voxel_size）。
func _make_mesh_library() -> MeshLibrary:
	var size := Vector3(_voxel_size, _voxel_size, _voxel_size)

	var mesh := BoxMesh.new()
	mesh.size = size
	# 材质由 WorldLook 工厂给（task_38 §二.3）：LOOK_ENABLED=false 时取原单色材质。
	var mat: StandardMaterial3D = WorldLook.Make_Voxel_Material(VOXEL_COLOR)
	if not LOOK_ENABLED:
		mat = WorldLook.Make_Fallback_Voxel_Material(VOXEL_COLOR)
	mesh.material = mat

	var shape := BoxShape3D.new()
	shape.size = size

	var lib := MeshLibrary.new()
	lib.create_item(VOXEL_ITEM)
	lib.set_item_name(VOXEL_ITEM, "voxel")
	lib.set_item_mesh(VOXEL_ITEM, mesh)
	lib.set_item_shapes(VOXEL_ITEM, [shape, Transform3D.IDENTITY])
	return lib


## 用世界坐标（Z-up）的轴对齐矩形填格；右开区间，单格厚度的墙也至少填 1 层。
func _fill_block_world(world_origin: Vector3, world_size: Vector3) -> void:
	var start := WorldToCell(world_origin)
	var end := WorldToCell(world_origin + world_size)
	for cx in range(start.x, maxi(end.x, start.x + 1)):
		for cz in range(start.z, maxi(end.z, start.z + 1)):
			for cy in range(start.y, maxi(end.y, start.y + 1)):
				_set_voxel(Vector3i(cx, cy, cz))


func _set_voxel(cell: Vector3i) -> void:
	if _voxel_world.get_cell_item(cell) == GridMap.INVALID_CELL_ITEM:
		_built_cells += 1
	_voxel_world.set_cell_item(cell, VOXEL_ITEM)


## 出生点净空自证（Step 6 直接落 (64,64,0) 不会卡在墙里）。
func _warn_if_spawn_blocked() -> void:
	var start := WorldToCell(SPAWN_WORLD - Vector3(SPAWN_CLEAR_RADIUS_M, SPAWN_CLEAR_RADIUS_M, 0.0))
	var end := WorldToCell(SPAWN_WORLD
		+ Vector3(SPAWN_CLEAR_RADIUS_M, SPAWN_CLEAR_RADIUS_M, SPAWN_CLEAR_HEIGHT_M))
	for cx in range(start.x, end.x + 1):
		for cz in range(start.z, end.z + 1):
			for cy in range(maxi(start.y, 0), end.y + 1):
				if _voxel_world.get_cell_item(Vector3i(cx, cy, cz)) != GridMap.INVALID_CELL_ITEM:
					push_warning("[world] 出生点净空被体素占用：cell=(%d, %d, %d)" % [cx, cy, cz])
					return
