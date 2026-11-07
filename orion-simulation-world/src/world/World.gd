# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-11-07
#
# 世界根脚本（COMPONENTS.md §4.1）：启动世界时钟、按 [sim_world] 程序化生成体素世界、
# 提供尺度/坐标/容器访问。D4 程序化生成，布局确定性、无 RNG。
#
# task_38 Step 1b：世界观感（地面/体素材质 + 天空 + 太阳方向）全部**运行时**建，
# 入口是 WorldLook.gd（COMPONENTS.md §4.1.1）；World.tscn 零改动。
#
# 尺度唯一来源是 project.godot 的 [sim_world]（COMPONENTS.md §0.2-5），本文件不写字面量尺度。
# 例外（task_32 §三.2 明令"写死，不得自行发挥"）：不可见空气墙 / 顶盖的 5 个 box 用字面量几何表
# AIR_WALL_BOXES，与现有设置数值一致（256×0.5=128 m、64×0.5=32 m）；改世界尺度必须同步改该表 + E24（两处一起）。
# 坐标换轴只经 WorldTransform（task_30 §7.3）：WorldToCell 内部即 PosToGodot，本文件无换轴字面量。
#
# task_49 Step 1（§五 Step 1）：世界几何 / 出生点 / 观感档的唯一数据来源改为 SceneDefs.SCENES，
# 本文件只留"按名字取数据"；场景名解析顺序 = 导出属性 scene_name → 项目设置 sim_world/world/scene → "city"。
# SPAWN_WORLD / OBSTACLES 两个 const **保留**为 city 兜底（Facade.gd 经 get_script_constant_map() 读 OBSTACLES，
# 而 Facade 只在 city 档跑），数值与数据表逐条相等。
#
# task_49 Step 4：`_Look() == "indoor"` 时补建 Interior（地板 / 踢脚线 / 门框，纯视觉零碰撞）并把体素材质
# 换成室内抹灰贴图；city 档 `_Look()` 恒为 "city" ⇒ 走原分支，渲染结果逐像素不变（sha256 对照）。
#
# task_50 Step 4：`_Look() == "venue"` 时补建 Plaza（路面 / 斑马线 / 人行道 / 场馆 / 背景楼，纯视觉零碰撞）；
# city 与 corridor 两条分支一字不改 ⇒ 渲染与格数都不变（验收按 sha256 + 日志计数双对照）。

extends Node3D

const WorldClock := preload("res://src/common/WorldClock.gd")
const WorldTransform := preload("res://src/common/WorldTransform.gd")
## 观感（材质 + 天空 + 太阳）唯一入口（task_38 §二.3 / COMPONENTS.md §4.1.1）。
const WorldLook := preload("res://src/world/WorldLook.gd")
## 街道几何 / 建筑立面的唯一入口（task_38 §三.4 / COMPONENTS.md §4.1.2）；运行期建、不进 .tscn。
const StreetLayout := preload("res://src/world/StreetLayout.gd")
const Facade := preload("res://src/world/Facade.gd")
## 室内视觉件（task_49 Step 4）：纯视觉零碰撞，只在 look=="indoor" 档建（city 档不建）。
const Interior := preload("res://src/world/Interior.gd")
## 场景数据表（task_49 §三）：世界几何 / 出生点 / 观感档的唯一数据来源。
const SceneDefs := preload("res://src/world/SceneDefs.gd")
## venue 场地视觉件（task_50 Step 4）：纯视觉零碰撞，只在 look=="venue" 档建（其它档不建）。
const Plaza := preload("res://src/world/Plaza.gd")

const SETTING_VOXEL_SIZE := "sim_world/world/voxel_size"
const SETTING_CELLS_X := "sim_world/world/cells_x"
const SETTING_CELLS_Y := "sim_world/world/cells_y"
const SETTING_LAYERS := "sim_world/world/layers"
## 场景名（task_49 §五 Step 1-2）：导出属性未填时的第二来源；两面都不填 = DEFAULT_SCENE。
const SETTING_SCENE := "sim_world/world/scene"
const DEFAULT_SCENE := "city"

const VOXEL_ITEM := 0
const VOXEL_COLOR := Color(0.55, 0.58, 0.62, 1.0)

## 观感总开关（task_38 §二.3）：false = 不换地面材质 / 不换天空 / 不动太阳，世界回到 task_37 的单色观感。
## 缺资产时**不用**改它：WorldLook 自己降级（只 warn）。
const LOOK_ENABLED := true

## 出生点（世界坐标，米），与 pleiades-sim 的 DEFAULT_ORIGIN (64, 64, 0) 一致。
## **保留**：city 档的兜底来源（_def 未就绪时用）；实际取值走 _Spawn()（task_49 Step 1）。
const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)
## 出生点净空半径（米，水平）与净空高度（米）——Step 6 直接把 avatar 放在这里。
## **保留**：同上，实际取值走 _Spawn_Clear_Radius_M() / _Spawn_Clear_Height_M()。
const SPAWN_CLEAR_RADIUS_M := 3.0
const SPAWN_CLEAR_HEIGHT_M := 4.0

## 静态障碍布局：origin / size 均为**世界坐标（Z-up，米）**。
## 只做边界墙 + 稀疏块（≈1.2 万格），绝不填满 256×256×64（419 万格）。
## **保留**：city 档的兜底来源，且 Facade.gd 经 get_script_constant_map() 读它（E31-⑤：不能 preload 本文件）；
## 数值与 SceneDefs.SCENES["city"].obstacles 逐条相等（task_49 Step 1，改一处必须同步另一处）。
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

# ---------------------------------------------------------------- 行人容器名（与 manager 共用）
## 容器名**必须**与 `WebSocketManager.PEDESTRIANS_NODE` 逐字一致（"Pedestrians"）。
## 世界**不建**这个容器、也不预置任何人：容器由 `WebSocketManager._Pedestrians_Root()` 在首个
## `chassis_hello{kind:"pedestrian"}` 到达时惰性创建（无人连入 = 节点树零变化）。
const PEDESTRIANS_NODE := "Pedestrians"
## 街道 / 立面容器名（task_38 §三.4）。
const STREET_NODE := "Street"
const FACADE_NODE := "Facade"

## 场景名（task_49 Step 1）：空 = 走项目设置 SETTING_SCENE，再空 = "city"。
## `.tscn` 里显式写一行即可（不需要 node_paths：E16 只约束节点引用）。
@export var scene_name: String = ""

var _scene: String = ""
var _def: Dictionary = {}
var _voxel_size: float = 0.5
var _cells_x: int = 256
var _cells_y: int = 256
var _layers: int = 64
var _built_cells: int = 0

@onready var _voxel_world: GridMap = $VoxelWorld
@onready var _vehicles_root: Node3D = $Vehicles


func _ready() -> void:
	_Resolve_Scene()
	WorldClock.EnsureStarted()
	_read_scale_settings()
	_log_scale()
	BuildVoxelWorld()
	if LOOK_ENABLED:
		Apply_World_Look()
	_Build_Bounds()
	# 室内视觉件（task_49 Step 4）：地板 overlay / 踢脚线 / 门框；纯 mesh、零碰撞、幂等。
	if _Look() == "indoor":
		Interior.Build_All(self, _def)
	# 场地视觉件（task_50 Step 4 / 口径 #15）：路面 / 斑马线 / 人行道 / 场馆 / 背景楼；
	# 纯 mesh、零碰撞、不进障碍表；幂等（先摘同名旧容器）。venue 档 props_street/props_facade 均为 false
	# ⇒ 下面两个 city 档分支都不会跑。
	if _Look() == "venue":
		Plaza.Build_All(self, _def)
	# 街道几何 / 立面（task_38 §三.4 的末尾两行）：只建渲染 mesh、零碰撞、可幂等重建。
	# task_49 §二 #17：只在 city 档跑，由场景数据的 props_street / props_facade 门控。
	if _Props_Street():
		StreetLayout.Build_All(self)
	if _Props_Facade():
		Facade.Build_All(self)


## 应用观感（task_49 Step 4：按档分派）——city = 地面材质 + HDRI 天空 + 太阳；indoor = 背景色 + 环境色 + 室内灯。
## 幂等；缺资产只 warn，不中断世界构建。
func Apply_World_Look() -> bool:
	var ok: bool = WorldLook.Apply_All(self, _Look())
	if not ok:
		print("[world] look degraded（city 档：缺资产保持原单色观感 / indoor 档：环境或灯没建全）：",
			str(WorldLook.Get_Missing_Assets()))
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

	for obstacle: Dictionary in _Obstacles():
		_fill_block_world(obstacle["origin"], obstacle["size"])

	var elapsed_ms := float(Time.get_ticks_usec() - started_usec) / 1000.0
	print("[world] voxel world built: ", _built_cells, " cells in ", "%.2f" % elapsed_ms, " ms")
	_warn_if_spawn_blocked()


## 代码构建不可见硬边界：1 个 StaticBody3D(Bounds) + 5 个 CollisionShape3D。
## 不写进 .tscn（避开 COMPONENTS.md §9.1 E16 的 node_paths 坑）；**不建任何 Mesh 渲染节点**
## （必须不可见，§9.1 E24）。
## 幂等：重复调用先摘掉旧的 Bounds 再重建，不会累积重复形状。
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
	# 几何按 AIR_WALL_BOXES 的写死字面量表建（E24：内侧面必须正好贴世界边界平面、顺序固定）；
	# air_wall_height_m 只用于日志 + "数据与几何表是否一致"的告警，**不按数据重建几何**。
	var height: float = _Air_Wall_Height()
	if not is_equal_approx(height, AIR_WALL_HEIGHT_M):
		push_warning("[world] air_wall_height_m=%.2f 与 AIR_WALL_BOXES 的 %.2f 不一致：几何仍按写死表建"
			% [height, AIR_WALL_HEIGHT_M])
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
		", mask=0, height=", height, " m, thickness=", AIR_WALL_THICKNESS_M, " m")


## 街道容器（后续任务用；每次调用重新解析，不缓存）。
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


# ---------------------------------------------------------------- 场景数据（task_49 Step 1）

## 当前场景名（"city" / "corridor_indoor"）；_ready 之前返回 "city"（= 2026-11-06 的既有行为）。
func Get_Scene_Name() -> String:
	return _scene if _scene != "" else DEFAULT_SCENE


## 当前场景的障碍表（**只读**；元素 {name, origin: Vector3, size: Vector3}，世界坐标 Z-up / 米）。
## 它是 SceneDefs.Get() 的深拷贝：就地改它不会落盘、只会骗自己 —— 要改数据去改 SceneDefs。
func Get_Obstacles() -> Array:
	return _Obstacles()


## 当前场景的出生点（世界坐标 / 米）。
func Get_Spawn() -> Vector3:
	return _Spawn()


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
	var spawn := _Spawn()
	print("[world] voxel=", _voxel_size, " m, cells=", _cells_x, "x", _cells_y, "x", _layers,
		" -> ", _cells_x * _voxel_size, " x ", _cells_y * _voxel_size, " x ", _layers * _voxel_size,
		" m, spawn=(", spawn.x, ", ", spawn.y, ", ", spawn.z, ")")


## 立方体 mesh + 立方体碰撞形状：渲染与物理共用同一边长（= voxel_size）。
func _make_mesh_library() -> MeshLibrary:
	var size := Vector3(_voxel_size, _voxel_size, _voxel_size)

	var mesh := BoxMesh.new()
	mesh.size = size
	# 材质由 WorldLook 工厂给（task_38 §二.3）：LOOK_ENABLED=false 时取原单色材质。
	# task_49 Step 4：indoor 档的体素墙换成室内抹灰贴图（2 m 循环）；city 档走原分支，逐字等价。
	var mat: StandardMaterial3D
	if not LOOK_ENABLED:
		mat = WorldLook.Make_Fallback_Voxel_Material(VOXEL_COLOR)
	elif _Look() == "indoor":
		mat = WorldLook.Make_Indoor_Wall_Material()
	else:
		mat = WorldLook.Make_Voxel_Material(VOXEL_COLOR)
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
	var spawn := _Spawn()
	var clear_radius := _Spawn_Clear_Radius_M()
	var clear_height := _Spawn_Clear_Height_M()
	var start := WorldToCell(spawn - Vector3(clear_radius, clear_radius, 0.0))
	var end := WorldToCell(spawn + Vector3(clear_radius, clear_radius, clear_height))
	for cx in range(start.x, end.x + 1):
		for cz in range(start.z, end.z + 1):
			for cy in range(maxi(start.y, 0), end.y + 1):
				if _voxel_world.get_cell_item(Vector3i(cx, cy, cz)) != GridMap.INVALID_CELL_ITEM:
					push_warning("[world] 出生点净空被体素占用：cell=(%d, %d, %d)" % [cx, cy, cz])
					return


# ---------------------------------------------------------------- 场景数据私有区（task_49 Step 1）

## 场景名解析（step.md:18）：导出属性 → ProjectSettings(SETTING_SCENE) → DEFAULT_SCENE。
## 未知名字 push_warning 后回退 city；**绝不报 ERROR 级**（--quit 的 0 ERROR 验收靠这条，E4）。
## 子节点（Systems/WebSocketManager）的 _ready 早于本节点 ⇒ 访问器必须容忍 _def 为空。
func _Resolve_Scene() -> void:
	var raw := scene_name
	var source := "export"
	if raw.strip_edges() == "":
		var setting: Variant = ProjectSettings.get_setting(SETTING_SCENE, "")
		raw = String(setting) if setting is String else ""
		source = "project_setting"
	if raw.strip_edges() == "":
		raw = DEFAULT_SCENE
		source = "default"
	_scene = raw.strip_edges()
	if not SceneDefs.Has(_scene):
		push_warning("[world] 未知场景 '%s'（来源 %s），回退 '%s'；已知场景：%s"
			% [_scene, source, DEFAULT_SCENE, str(SceneDefs.Names())])
		_scene = DEFAULT_SCENE
	_def = SceneDefs.Get(_scene)
	print("[world] scene=", _scene, " (source=", source, ", obstacles=", _Obstacles().size(), ")")


## 障碍表：_def 未就绪（子节点 _ready 早于本节点 _ready）时回退 city 常量。
func _Obstacles() -> Array:
	var table: Variant = _def.get("obstacles", null)
	if table is Array:
		return table
	return OBSTACLES


func _Spawn() -> Vector3:
	var p: Variant = _def.get("spawn", null)
	if p is Vector3:
		return p
	return SPAWN_WORLD


func _Spawn_Clear_Radius_M() -> float:
	var v: Variant = _def.get("spawn_clear_radius_m", null)
	return float(v) if v is float or v is int else SPAWN_CLEAR_RADIUS_M


func _Spawn_Clear_Height_M() -> float:
	var v: Variant = _def.get("spawn_clear_height_m", null)
	return float(v) if v is float or v is int else SPAWN_CLEAR_HEIGHT_M


func _Air_Wall_Height() -> float:
	var v: Variant = _def.get("air_wall_height_m", null)
	return float(v) if v is float or v is int else AIR_WALL_HEIGHT_M


func _Props_Street() -> bool:
	if _def.is_empty():
		return true                      # 兜底 = city 行为
	return bool(_def.get("props_street", true))


func _Props_Facade() -> bool:
	if _def.is_empty():
		return true
	return bool(_def.get("props_facade", true))


## 观感档（task_49 Step 4）：取 SceneDef 的 `look` 字段；数据未就绪（子节点 _ready 早于本节点）时回退 "city"。
func _Look() -> String:
	var v: Variant = _def.get("look", null)
	var s := String(v) if v is String or v is StringName else ""
	return s if s != "" else "city"
