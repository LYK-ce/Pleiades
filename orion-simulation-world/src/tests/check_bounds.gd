# Presented by KeJi
# Created Date ： 2026-10-22
# Modified Date ： 2026-10-22
#
# 空气墙 / 顶盖自检（task_32 Step 2）：不可见硬边界（32 m 四面空气墙 + 32 m 顶盖）的结构、
# 几何、代码构建性、幂等性，以及"20 m/s 冲撞出不去 / 5 m/s 上升被顶盖挡住 / 地面仍有效 /
# 可见 2 m 矮墙仍在"。断言清单见 Task/task_32_world_refactor_bounds.md §三.3 与
# COMPONENTS.md §4.1（空气墙几何表）/ §9.1 E24。
#
# 用法：timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 \
#           --script res://src/tests/check_bounds.gd
# 期望：[check] total=12 failed=0 + SELF-CHECK OK + EXIT=0
#       （任何 Godot 命令都必须带 timeout，COMPONENTS.md §9.1 E22）
#
# 本文件赖以成立的实测事实：
#   P1  World.gd 的 `_Build_Bounds()` 在 `_ready()` 里、`BuildVoxelWorld()` 之后同步执行 →
#       `add_child(World)` 后等 2 个物理帧即可查到 `World/Bounds`。
#   P2  `Bounds` 的 5 个 box 内侧面贴世界边界平面（x=0/128、y=0/128、z=32），厚度向外 →
#       撞墙后车中心停在 [0,128] 内（体半宽 0.1 + 默认 safe_margin）。
#   P3  冲撞体用 MOTION_MODE_FLOATING（无重力）+ collision_mask=1（只撞地面 / 空气墙）+
#       起点世界 z=3.0（高于 2 m 可见矮墙）→ 只可能与空气墙相互作用，撞停即"空气墙挡住了"。
#   P4  世界水平跨度 128 m = [sim_world] 256 × 0.5；竖直 32 m = 64 × 0.5（与几何表数值一致，
#       由 B1b/B1c 的数值断言兜住）。
#   P5  `OBSTACLES` 是**脚本常量**，运行时经 `get_script_constant_map()` 读取（同
#       check_lidar.gd B6 手法），不复制障碍表。
#
# 注意（GDScript 4.7 类型推断陷阱）：凡 instantiate() / WorldToCell() 等动态返回值的变量
# 一律显式标注类型，否则 Parse Error: Cannot infer the type of ...

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")

const WORLD_SCENE := "res://src/scenes/World.tscn"   # 同一路径既 load() 也读原文（B1d）
const BOUNDS_NODE := "Bounds"

const AIR_WALL_HEIGHT_M := 32.0
const AIR_WALL_THICKNESS_M := 1.0
const AIR_WALL_LAYER := 1
const WORLD_SPAN_M := 128.0          # 世界水平跨度（[sim_world] 256 × 0.5）
const LID_INNER_Y := 32.0            # 世界 z=32 的 Godot y

const CHARGE_SPEED_MPS := 20.0       # B2~B5
const CHARGE_FRAMES := 600           # 10 s @ 60 Hz
const CHARGE_WORLD_Z := 3.0          # 高于 2 m 可见矮墙 → 只可能被空气墙挡住
const CLIMB_SPEED_MPS := 5.0         # B6
const CLIMB_START_WORLD := Vector3(64.0, 64.0, 2.0)
const CLIMB_FRAMES := 600
const DROP_START_WORLD := Vector3(64.0, 64.0, 5.0)   # B7
const DROP_FRAMES := 180
const BODY_HALF_M := 0.1             # 冲撞体半边长（0.2³ 立方）
const TOL := 1e-3
const WALL_TOUCH_MIN_M := 127.0      # "真的贴到墙"下界（否则"没动"也会 PASS）

## 期望的 5 个 box 的世界 AABB（Godot 坐标，米）——独立写出，不从 World.gd 导入，
## 这样才能真正发现几何放反 / 厚度朝内（§9.1 E24）。顺序 = 南 / 北 / 西 / 东 / 顶盖。
const WALL_AABB_MIN := [
	Vector3(0.0, 0.0, -1.0),        # 南墙：世界 y∈[-1,0]
	Vector3(0.0, 0.0, 128.0),       # 北墙：世界 y∈[128,129]
	Vector3(-1.0, 0.0, 0.0),        # 西墙：世界 x∈[-1,0]
	Vector3(128.0, 0.0, 0.0),       # 东墙：世界 x∈[128,129]
	Vector3(0.0, 32.0, 0.0),        # 顶盖：世界 z∈[32,33]
]
const WALL_AABB_MAX := [
	Vector3(128.0, 32.0, 0.0),
	Vector3(128.0, 32.0, 129.0),
	Vector3(0.0, 32.0, 128.0),
	Vector3(129.0, 32.0, 128.0),
	Vector3(128.0, 33.0, 128.0),
]
const LID_CENTER := Vector3(64.0, 32.5, 64.0)
const LID_SIZE := Vector3(128.0, 1.0, 128.0)
const WALL_NAMES := ["wall_south", "wall_north", "wall_west", "wall_east"]
const WALL_CELLS_EACH := 1024        # 128×0.5 m 长 × 2 m 高（0.5 m 格）

var _total: int = 0
var _failed: int = 0


## 恒速冲撞体：每物理帧把 drive 速度交给 move_and_slide()（FLOATING，无重力、不落地）。
## 只在本文件内使用，不进 §3 的组件清单、不落 .tscn。
class Mover extends CharacterBody3D:
	var drive_velocity: Vector3 = Vector3.ZERO

	func _physics_process(_delta: float) -> void:
		velocity = drive_velocity
		move_and_slide()


func _initialize() -> void:
	_run()


func _run() -> void:
	var packed: PackedScene = load(WORLD_SCENE)
	if packed == null:
		_fatal("scene-loads", WORLD_SCENE)
		return
	var world: Node3D = packed.instantiate()
	root.add_child(world)
	# 物理空间在 add_child 后第一个物理帧才同步（P1 / check_world.gd R7）
	await _wait_physics(2)

	var bounds: Node = world.get_node_or_null(BOUNDS_NODE)

	# ============================================================ B1. 结构 / 层 / 不可见
	var is_body := bounds is StaticBody3D
	var layer := -1
	var mask := -1
	var shapes: Array = []
	var all_box := false
	if is_body:
		layer = (bounds as StaticBody3D).collision_layer
		mask = (bounds as StaticBody3D).collision_mask
		shapes = _bounds_shapes(bounds)
		all_box = true
		for raw: Node in shapes:
			var cs: CollisionShape3D = raw
			if not (cs.shape is BoxShape3D):
				all_box = false
	var meshes := _count_by_class(bounds, &"MeshInstance3D") if bounds != null else -1
	_check("B1-bounds-structure", is_body and layer == AIR_WALL_LAYER and mask == 0
		and shapes.size() == 5 and all_box and meshes == 0,
		"shapes=%d layer=%d mask=%d meshes=%d" % [shapes.size(), layer, mask, meshes])

	# ============================================================ B1b. 四面墙几何（内侧面贴边界）
	var walls_ok := shapes.size() == 5
	var inner: Array = []
	var tops: Array = []
	var thickness_bad := 0
	for i in range(4):
		if i >= shapes.size():
			walls_ok = false
			break
		var aabb: Array = _box_aabb(shapes[i])
		var lo: Vector3 = aabb[0]
		var hi: Vector3 = aabb[1]
		if not lo.is_equal_approx(WALL_AABB_MIN[i]) or not hi.is_equal_approx(WALL_AABB_MAX[i]):
			walls_ok = false
		inner.append(lo if i % 2 == 1 else hi)   # 北/东取 min 面，南/西取 max 面
		tops.append(hi.y)
		var thin := minf(hi.x - lo.x, hi.z - lo.z)
		if absf(thin - AIR_WALL_THICKNESS_M) > TOL:
			thickness_bad += 1
	var inner_ok := shapes.size() == 5
	if inner_ok:
		inner_ok = absf((inner[0] as Vector3).z) < TOL \
			and absf((inner[1] as Vector3).z - WORLD_SPAN_M) < TOL \
			and absf((inner[2] as Vector3).x) < TOL \
			and absf((inner[3] as Vector3).x - WORLD_SPAN_M) < TOL
	var tops_ok := shapes.size() == 5
	if tops_ok:
		for raw_top: float in tops:
			if absf(raw_top - AIR_WALL_HEIGHT_M) > TOL:
				tops_ok = false
	_check("B1b-walls-geometry", walls_ok and inner_ok and tops_ok and thickness_bad == 0,
		"inner=[%s,%s,%s,%s] tops=%s thickness=%s"
		% [_f((inner[0] as Vector3).z if shapes.size() == 5 else -1.0),
			_f((inner[1] as Vector3).z if shapes.size() == 5 else -1.0),
			_f((inner[2] as Vector3).x if shapes.size() == 5 else -1.0),
			_f((inner[3] as Vector3).x if shapes.size() == 5 else -1.0),
			_f(tops[0] as float if shapes.size() == 5 else -1.0),
			_f(AIR_WALL_THICKNESS_M if thickness_bad == 0 else -1.0)])

	# ============================================================ B1c. 顶盖几何（下表面 32 m）
	var lid_ok := false
	var lid_detail := "shapes=%d" % shapes.size()
	if shapes.size() == 5:
		var aabb: Array = _box_aabb(shapes[4])
		var lo: Vector3 = aabb[0]
		var hi: Vector3 = aabb[1]
		var center: Vector3 = (lo + hi) * 0.5
		var size: Vector3 = hi - lo
		lid_ok = center.is_equal_approx(LID_CENTER) and size.is_equal_approx(LID_SIZE) \
			and absf(lo.y - LID_INNER_Y) < TOL
		lid_detail = "center=(%s,%s,%s) size=(%s,%s,%s) inner_y=%s" % [
			_f(center.x), _f(center.y), _f(center.z),
			_f(size.x), _f(size.y), _f(size.z), _f(lo.y)]
	_check("B1c-lid-geometry", lid_ok, lid_detail)

	# ============================================================ B1d. 必须由代码构建（.tscn 里没有）
	var tscn := _read_text(WORLD_SCENE)
	var hits := tscn.count("Bounds") + tscn.count("AIR_WALL")
	var embedded := tscn.count("[sub_resource type=\"GDScript\"")
	_check("B1d-built-by-code", tscn.length() > 0 and hits == 0 and embedded == 0,
		"tscn=%d chars Bounds/AIR_WALL hits=%d gdscript_subres=%d" % [tscn.length(), hits, embedded])

	# ============================================================ B1e. 幂等重建（不累积 / 不改名）
	world.call("_Build_Bounds")
	await _wait_physics(2)
	var bounds_nodes := 0
	for child: Node in world.get_children():
		if child.name == BOUNDS_NODE:
			bounds_nodes += 1
	var rebuilt: Node = world.get_node_or_null(BOUNDS_NODE)
	var rebuilt_shapes: int = _bounds_shapes(rebuilt).size() if rebuilt != null else -1
	var rebuilt_meshes: int = _count_by_class(rebuilt, &"MeshInstance3D") if rebuilt != null else -1
	_check("B1e-idempotent", bounds_nodes == 1 and rebuilt_shapes == 5 and rebuilt_meshes == 0,
		"bounds_nodes=%d shapes=%d meshes=%d" % [bounds_nodes, rebuilt_shapes, rebuilt_meshes])

	# ============================================================ B2~B5. 20 m/s 四面冲撞 10 s
	var start_world := Vector3(64.0, 64.0, CHARGE_WORLD_Z)
	var east: Vector3 = await _charge(world, start_world, Vector3(CHARGE_SPEED_MPS, 0.0, 0.0), CHARGE_FRAMES)
	_check("B2-east-blocked", east.x > WALL_TOUCH_MIN_M and east.x < WORLD_SPAN_M
		and absf(east.z - CHARGE_WORLD_Z) < TOL,
		"x=%s z=%s (want %s<x<%s)" % [_f(east.x), _f(east.z), _f(WALL_TOUCH_MIN_M), _f(WORLD_SPAN_M)])

	var west: Vector3 = await _charge(world, start_world, Vector3(-CHARGE_SPEED_MPS, 0.0, 0.0), CHARGE_FRAMES)
	_check("B3-west-blocked", west.x > 0.0 and west.x < 1.0 and absf(west.z - CHARGE_WORLD_Z) < TOL,
		"x=%s z=%s (want 0<x<1)" % [_f(west.x), _f(west.z)])

	var south: Vector3 = await _charge(world, start_world, Vector3(0.0, -CHARGE_SPEED_MPS, 0.0), CHARGE_FRAMES)
	_check("B4-south-blocked", south.y > 0.0 and south.y < 1.0 and absf(south.z - CHARGE_WORLD_Z) < TOL,
		"y=%s z=%s (want 0<y<1)" % [_f(south.y), _f(south.z)])

	var north: Vector3 = await _charge(world, start_world, Vector3(0.0, CHARGE_SPEED_MPS, 0.0), CHARGE_FRAMES)
	_check("B5-north-blocked", north.y > WALL_TOUCH_MIN_M and north.y < WORLD_SPAN_M
		and absf(north.z - CHARGE_WORLD_Z) < TOL,
		"y=%s z=%s (want %s<y<%s)" % [_f(north.y), _f(north.z), _f(WALL_TOUCH_MIN_M), _f(WORLD_SPAN_M)])

	# ============================================================ B6. 顶盖：5 m/s 上升 10 s
	var up: Vector3 = await _charge(world, CLIMB_START_WORLD, Vector3(0.0, 0.0, CLIMB_SPEED_MPS),
		CLIMB_FRAMES)
	_check("B6-lid-blocks", up.z > LID_INNER_Y - 1.0 and up.z <= LID_INNER_Y + TOL,
		"z=%s (want %s<z<=%s)" % [_f(up.z), _f(LID_INNER_Y - 1.0), _f(LID_INNER_Y)])

	# ============================================================ B7. 地面仍有效（自由落体静止）
	var body := RigidBody3D.new()
	var body_col := CollisionShape3D.new()
	var body_box := BoxShape3D.new()
	body_box.size = Vector3(0.3, 0.3, 0.3)
	body_col.shape = body_box
	body.add_child(body_col)
	body.mass = 1.0
	body.collision_layer = 4
	body.collision_mask = 1
	world.add_child(body)
	# 入树后才写 global_position（入树前写会触发 `!is_inside_tree()` 的 ERROR，E4 要求日志干净）
	body.global_position = WorldTransform.PosToGodot(DROP_START_WORLD)
	await _wait_physics(DROP_FRAMES)
	var rest_world: Vector3 = WorldTransform.PosToWorld(body.global_position)
	var rest_speed := body.linear_velocity.length()
	_check("B7-ground-still", rest_world.z > 0.10 and rest_world.z < 0.25 and rest_speed < 0.1,
		"z=%s speed=%s (want z in [0.10, 0.25])" % [_f(rest_world.z), _f(rest_speed)])
	body.queue_free()
	await _wait_physics(2)

	# ============================================================ B8. 可见 2 m 矮墙仍在
	var consts: Dictionary = world.get_script().get_script_constant_map()
	var obstacles: Variant = consts.get("OBSTACLES", null)
	var voxel: GridMap = world.get_node_or_null("VoxelWorld")
	var wall_cells: Array = []
	var walls_intact := obstacles is Array and voxel != null
	if walls_intact:
		var table: Array = obstacles
		if table.size() < 4:
			walls_intact = false
		for i in range(4):
			if not walls_intact:
				break
			var item: Dictionary = table[i]
			if String(item.get("name", "")) != WALL_NAMES[i]:
				walls_intact = false
				break
			var filled: int = _occupied_cells(world, voxel, item["origin"], item["size"])
			wall_cells.append(filled)
			if filled <= 0:
				walls_intact = false
	_check("B8-visible-walls", walls_intact and wall_cells.size() == 4
		and int(wall_cells[0]) == WALL_CELLS_EACH and int(wall_cells[1]) == WALL_CELLS_EACH
		and int(wall_cells[2]) == WALL_CELLS_EACH and int(wall_cells[3]) == WALL_CELLS_EACH,
		"south=%d north=%d west=%d east=%d (want each %d)"
		% [int(wall_cells[0]) if wall_cells.size() > 0 else -1,
			int(wall_cells[1]) if wall_cells.size() > 1 else -1,
			int(wall_cells[2]) if wall_cells.size() > 2 else -1,
			int(wall_cells[3]) if wall_cells.size() > 3 else -1,
			WALL_CELLS_EACH])

	_finish()


# ---------------------------------------------------------------- 私有

## 生成恒速冲撞体（P3）：layer=4 / mask=1（只撞地面与空气墙）、FLOATING（无重力）。
func _spawn_mover(world: Node3D, world_pos: Vector3, world_vel: Vector3) -> Mover:
	var mover := Mover.new()
	mover.collision_layer = 4
	mover.collision_mask = 1
	mover.motion_mode = CharacterBody3D.MOTION_MODE_FLOATING
	var shape := BoxShape3D.new()
	shape.size = Vector3(BODY_HALF_M * 2.0, BODY_HALF_M * 2.0, BODY_HALF_M * 2.0)
	var col := CollisionShape3D.new()
	col.shape = shape
	mover.add_child(col)
	world.add_child(mover)
	mover.global_position = WorldTransform.PosToGodot(world_pos)
	mover.drive_velocity = WorldTransform.VelToGodot(world_vel)
	return mover


## 冲撞 frames 个物理帧，返回停下后的**世界坐标**（Z-up）；用完即释放，等 2 帧再返回。
func _charge(world: Node3D, world_pos: Vector3, world_vel: Vector3, frames: int) -> Vector3:
	var mover: Mover = _spawn_mover(world, world_pos, world_vel)
	await _wait_physics(frames)
	var result: Vector3 = WorldTransform.PosToWorld(mover.global_position)
	mover.queue_free()
	await _wait_physics(2)
	return result


## `bounds` 的直接子节点里的 CollisionShape3D（保序）。
func _bounds_shapes(bounds: Node) -> Array:
	var result: Array = []
	if bounds == null:
		return result
	for child: Node in bounds.get_children():
		if child is CollisionShape3D:
			result.append(child)
	return result


## 递归数某个引擎类（MeshInstance3D 用）。
func _count_by_class(node: Node, cls: StringName) -> int:
	if node == null:
		return 0
	var total := 1 if node.is_class(cls) else 0
	for child: Node in node.get_children():
		total += _count_by_class(child, cls)
	return total


## CollisionShape3D 的**世界** AABB：[Vector3 min, Vector3 max]。
func _box_aabb(shape_node: Node) -> Array:
	var cs: CollisionShape3D = shape_node
	var half: Vector3 = (cs.shape as BoxShape3D).size * 0.5
	var center: Vector3 = cs.global_position
	return [center - half, center + half]


## 与 World.gd `_fill_block_world` 同款右开区间遍历，数实填格数（B8）。
func _occupied_cells(world: Node3D, voxel: GridMap, origin: Vector3, size: Vector3) -> int:
	var start: Vector3i = world.WorldToCell(origin)
	var end: Vector3i = world.WorldToCell(origin + size)
	var filled := 0
	for cx in range(start.x, maxi(end.x, start.x + 1)):
		for cz in range(start.z, maxi(end.z, start.z + 1)):
			for cy in range(start.y, maxi(end.y, start.y + 1)):
				if voxel.get_cell_item(Vector3i(cx, cy, cz)) != GridMap.INVALID_CELL_ITEM:
					filled += 1
	return filled


func _read_text(path: String) -> String:
	var file := FileAccess.open(path, FileAccess.READ)
	if file == null:
		return ""
	var text := file.get_as_text()
	file.close()
	return text


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		await physics_frame


## 定点格式化（detail 用；Vector3 不能直接吃 % 占位符）。
func _f(value: float) -> String:
	return "%.4f" % value


## 致命前置失败（World.tscn 加载不到）：**只有这一条**会中断后续断言，记 1 条 FAIL 后直接收尾。
func _fatal(label: String, detail: String) -> void:
	_total += 1
	_failed += 1
	print("[check] ", label, " FAIL ", detail, "（致命，后续断言跳过）")
	_finish()


func _check(label: String, ok: bool, detail: String = "") -> bool:
	_total += 1
	if ok:
		print("[check] ", label, " ok", (" " + detail) if not detail.is_empty() else "")
		return true
	_failed += 1
	print("[check] ", label, " FAIL", (" " + detail) if not detail.is_empty() else "")
	return false


func _finish() -> void:
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
