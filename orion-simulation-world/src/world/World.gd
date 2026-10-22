# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 世界根脚本（COMPONENTS.md §4.1）：启动世界时钟、按 [sim_world] 程序化生成体素世界、
# 提供尺度/坐标/容器访问。D4 程序化生成，布局确定性、无 RNG。
#
# 尺度唯一来源是 project.godot 的 [sim_world]（COMPONENTS.md §0.2-5），本文件不写字面量尺度。
# 例外（task_32 §三.2 明令"写死，不得自行发挥"）：不可见空气墙 / 顶盖的 5 个 box 用字面量几何表
# AIR_WALL_BOXES，与现有设置数值一致（256×0.5=128 m、64×0.5=32 m），一致性由 check_bounds.gd 的
# B1b/B1c 数值断言兜住；改世界尺度必须同步改该表 + E24 + check_bounds 常量（三处一起）。
# 坐标换轴只经 WorldTransform（task_30 §7.3）：WorldToCell 内部即 PosToGodot，本文件无换轴字面量。

extends Node3D

const WorldClock := preload("res://src/common/WorldClock.gd")
const WorldTransform := preload("res://src/common/WorldTransform.gd")

const SETTING_VOXEL_SIZE := "sim_world/world/voxel_size"
const SETTING_CELLS_X := "sim_world/world/cells_x"
const SETTING_CELLS_Y := "sim_world/world/cells_y"
const SETTING_LAYERS := "sim_world/world/layers"

const VOXEL_ITEM := 0
const VOXEL_COLOR := Color(0.55, 0.58, 0.62, 1.0)

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

var _voxel_size: float = 0.5
var _cells_x: int = 256
var _cells_y: int = 256
var _layers: int = 64
var _built_cells: int = 0

@onready var _voxel_world: GridMap = $VoxelWorld
@onready var _vehicles_root: Node3D = $Vehicles


func _ready() -> void:
	WorldClock.EnsureStarted()
	_read_scale_settings()
	_log_scale()
	BuildVoxelWorld()
	_Build_Bounds()


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
	var mat := StandardMaterial3D.new()
	mat.albedo_color = VOXEL_COLOR
	mat.roughness = 0.85
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
