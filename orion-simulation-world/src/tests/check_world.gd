# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 世界组件自检（Step 2）：结构 / 体素渲染与碰撞 / 地面顶面 / 物理层分离 / 掉落静止 /
# 环境光照 / 相机视锥 / 坐标转换 / 时间基。断言清单见 COMPONENTS.md §2 的不变量。
# Step 3：§11 的相机断言改指 ObserverCamera（DebugCamera 已删除），断言数 25 → 26。
# 用法：godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_world.gd
#
# 注意（GDScript 4.7 类型推断陷阱）：凡 instantiate() / intersect_ray() / WorldToCell() 等
# 动态返回值的变量一律显式标注类型，否则 Parse Error: Cannot infer the type of ...

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldClock := preload("res://src/common/WorldClock.gd")

const VOXEL_SIZE := 0.5
const CELLS_XZ := 256
const CELL_LAYERS := 64
const CELLS_MIN := 2000
const CELLS_MAX := 20000
const TOL := 0.001

var _total: int = 0
var _failed: int = 0


func _initialize() -> void:
	_run()


func _run() -> void:
	# ---- 1. 场景可加载 / 根节点 / 脚本
	var packed: PackedScene = load("res://src/scenes/World.tscn")
	if not _check("scene-loads", packed != null, "res://src/scenes/World.tscn"):
		_finish()
		return
	var world: Node3D = packed.instantiate()
	root.add_child(world)
	# 物理空间在 add_child 后第一个物理帧才同步，射线/掉落至少等两帧（R7）
	await physics_frame
	await physics_frame

	_check("root-name", world.name == "World", str(world.name))
	_check("root-script", world.get_script() != null, str(world.get_script().resource_path))

	# ---- 2. 体素渲染数据真实生成（headless 下"可见"的等价代理，R1）
	var voxel: GridMap = world.get_node("VoxelWorld")
	var lib: MeshLibrary = voxel.mesh_library
	_check("voxel-library", lib != null)
	var meshes: Array = voxel.get_meshes()
	_check("voxel-render-data", lib != null and meshes.size() > 0, "octants=%d" % meshes.size())
	var item_mesh: Mesh = lib.get_item_mesh(0) if lib != null else null
	var shapes: Array = lib.get_item_shapes(0) if lib != null else []
	_check("voxel-item-shape", item_mesh != null and shapes.size() >= 2 and shapes[0] is BoxShape3D,
		"shapes=%d" % shapes.size())

	# ---- 3. 稀疏填充（不是 419 万格全填，R6）
	var used: Array = voxel.get_used_cells()
	_check("voxel-count", used.size() > CELLS_MIN and used.size() < CELLS_MAX, "cells=%d" % used.size())

	# ---- 4. 尺度：水平 256×256 格、垂直 64 层（0~32 m）
	var bad_cells := 0
	var max_layer := -1
	for raw_cell in used:
		var cell: Vector3i = raw_cell
		if cell.x < 0 or cell.x >= CELLS_XZ or cell.z < 0 or cell.z >= CELLS_XZ:
			bad_cells += 1
		elif cell.y < 0 or cell.y >= CELL_LAYERS:
			bad_cells += 1
		max_layer = maxi(max_layer, cell.y)
	_check("voxel-bounds", bad_cells == 0 and max_layer < CELL_LAYERS,
		"max_layer=%d out_of_range=%d" % [max_layer, bad_cells])

	# ---- 5. 地面：一整块 BoxShape3D(128,1,128)，中心 y=-0.5 → 顶面 y=0
	var ground: StaticBody3D = world.get_node("Ground")
	var ground_shape_node: CollisionShape3D = ground.get_node("CollisionShape3D")
	var ground_shape: Shape3D = ground_shape_node.shape
	var is_box: bool = ground_shape is BoxShape3D
	var box_size: Vector3 = (ground_shape as BoxShape3D).size if is_box else Vector3.ZERO
	var ground_top: float = ground.global_position.y + box_size.y * 0.5
	_check("ground-block", is_box and box_size.is_equal_approx(Vector3(128, 1, 128))
		and absf(ground_top) < TOL,
		"size=%s top=%s" % [box_size, ground_top])
	_check("ground-layers", ground.collision_layer == 1 and ground.collision_mask == 0,
		"layer=%d mask=%d" % [ground.collision_layer, ground.collision_mask])
	_check("voxel-layers", voxel.collision_layer == 2 and voxel.collision_mask == 0,
		"layer=%d mask=%d" % [voxel.collision_layer, voxel.collision_mask])

	# ---- 6. 地面无洞：mask=1 向下射线在四角 / 中心 / 空场点全部命中 y≈0
	var space: PhysicsDirectSpaceState3D = world.get_world_3d().direct_space_state
	var ground_samples := [
		Vector2(0.25, 0.25), Vector2(127.75, 0.25),
		Vector2(0.25, 127.75), Vector2(127.75, 127.75),
		Vector2(64.0, 64.0), Vector2(50.0, 20.0), Vector2(110.0, 60.0),
	]
	var ground_hits := 0
	var worst_error := 0.0
	for sample: Vector2 in ground_samples:
		var hit_y := _ray_down(space, sample.x, sample.y, 5.0, 1)
		if absf(hit_y) < TOL:
			ground_hits += 1
			worst_error = maxf(worst_error, absf(hit_y))
	_check("ground-no-hole", ground_hits == ground_samples.size(),
		"hits=%d/%d worst=%s" % [ground_hits, ground_samples.size(), worst_error])

	# ---- 7. 体素真碰撞 + 物理层 1/2 分离：建筑 A 顶面 4 m，地面 0 m
	var building_x := 29.0   # 世界 (29, 28) -> Godot x=29, z=28
	var building_z := 28.0
	var voxel_hit_y := _ray_down(space, building_x, building_z, 10.0, 2)
	var under_hit_y := _ray_down(space, building_x, building_z, 10.0, 1)
	_check("voxel-collision", absf(voxel_hit_y - 4.0) < TOL, "top=%s (want 4.0)" % voxel_hit_y)
	_check("layer-separation", absf(under_hit_y) < TOL, "ground=%s (want 0.0)" % under_hit_y)

	# ---- 8. 出生点：WorldToCell(64,64,0)=(128,0,128)，±6 格 × 8 层全空
	var spawn := Vector3(64.0, 64.0, 0.0)
	var spawn_cell: Vector3i = world.WorldToCell(spawn)
	_check("spawn-cell", spawn_cell == Vector3i(128, 0, 128), str(spawn_cell))
	var blocked := 0
	for dx in range(-6, 7):
		for dz in range(-6, 7):
			for dy in range(0, 8):
				var probe := Vector3i(spawn_cell.x + dx, dy, spawn_cell.z + dz)
				if voxel.get_cell_item(probe) != GridMap.INVALID_CELL_ITEM:
					blocked += 1
	_check("spawn-clear", blocked == 0, "occupied=%d" % blocked)

	# ---- 9. 验收核心：RigidBody 从 5 m 静止释放，被地面挡住并静止（不会无限下坠）
	var body := RigidBody3D.new()
	var body_col := CollisionShape3D.new()
	var body_box := BoxShape3D.new()
	body_box.size = Vector3(0.3, 0.3, 0.3)
	body_col.shape = body_box
	body.add_child(body_col)
	body.position = Vector3(64.0, 5.0, 64.0)
	body.mass = 1.0
	body.collision_layer = 4
	body.collision_mask = 1
	world.add_child(body)
	for _i in range(180):
		await physics_frame
	var rest_y := body.global_position.y
	var rest_speed := body.linear_velocity.length()
	_check("drop-rests", rest_y > 0.10 and rest_y < 0.25 and rest_speed < 0.1,
		"y=%s |v|=%s (want y in [0.10, 0.25])" % [rest_y, rest_speed])
	body.queue_free()

	# ---- 10. 环境 / 光照真的生效（不靠肉眼看）
	var world_env: WorldEnvironment = world.get_node("WorldEnvironment")
	var env: Environment = world_env.environment
	_check("environment-sky", env != null
		and env.background_mode == Environment.BG_SKY
		and env.ambient_light_source == Environment.AMBIENT_SOURCE_SKY,
		"bg=%d ambient=%d" % [env.background_mode, env.ambient_light_source] if env != null else "null")
	var sun: DirectionalLight3D = world.get_node("Sun")
	_check("sun-shadow", sun.shadow_enabled and sun.visible, "shadow=%s" % sun.shadow_enabled)

	# ---- 11. 相机：Step 3 起由 ObserverCamera 承担（DebugCamera 已移除）
	var cam: Camera3D = world.get_node_or_null("ObserverCamera")
	_check("camera-frustum", cam != null and cam.current
		and cam.is_position_in_frustum(Vector3(64.0, 0.0, 64.0)),
		"fov=%s far=%s" % [cam.fov, cam.far] if cam != null else "null")
	_check("debug-camera-removed", world.get_node_or_null("DebugCamera") == null)

	# ---- 12. 世界->格 映射与 COMPONENTS.md §2.3 公式一致（4 个点，含边界与半格）
	var probes := [
		Vector3(0.0, 0.0, 0.0),
		Vector3(64.3, 63.9, 12.7),
		Vector3(127.9, 127.4, 31.9),
		Vector3(1.2, 2.5, 3.7),
	]
	var mapping_bad := 0
	for probe: Vector3 in probes:
		var got: Vector3i = world.WorldToCell(probe)
		var want := Vector3i(floori(probe.x / VOXEL_SIZE), floori(probe.z / VOXEL_SIZE),
			floori(probe.y / VOXEL_SIZE))
		if got != want:
			mapping_bad += 1
			print("  probe ", probe, " got ", got, " want ", want)
	_check("world-to-cell", mapping_bad == 0, "mismatch=%d" % mapping_bad)

	# ---- 13. 位姿转换往返 + yaw 符号（唯一入口自洽）
	var pose_t: Transform3D = WorldTransform.MakeTransform(Vector3(12.5, -3.75, 2.25), 0.6)
	var pose_back: Dictionary = WorldTransform.PoseFromTransform(pose_t)
	var pose_ok := absf(float(pose_back["x"]) - 12.5) < TOL \
		and absf(float(pose_back["y"]) + 3.75) < TOL \
		and absf(float(pose_back["z"]) - 2.25) < TOL \
		and absf(float(pose_back["yaw"]) - 0.6) < TOL
	_check("pose-roundtrip", pose_ok, str(pose_back))
	var yaw_ok := absf(WorldTransform.YawToGodot(PI / 2.0) + PI / 2.0) < TOL \
		and absf(WorldTransform.YawToWorld(-PI / 2.0) - PI / 2.0) < TOL
	_check("yaw-sign", yaw_ok, "YawToGodot(PI/2)=%s" % WorldTransform.YawToGodot(PI / 2.0))

	# ---- 14. 时间基：非负且单调不降
	var t0 := WorldClock.Now()
	var t0_usec := WorldClock.NowUsec()
	var t1 := WorldClock.Now()
	_check("clock-monotonic", t0 >= 0.0 and t0_usec >= 0 and t1 >= t0,
		"t0=%s t1=%s" % [t0, t1])

	# ---- 15. 容器挂载点就位（Step 4/6 依赖）
	var vehicles_root: Node3D = world.GetVehiclesRoot()
	_check("containers", vehicles_root != null and vehicles_root.name == "Vehicles"
		and world.has_node("Systems"), str(vehicles_root.name))

	_finish()


# ---------------------------------------------------------------- 私有

func _ray_down(space: PhysicsDirectSpaceState3D, x: float, z: float, from_y: float, mask: int) -> float:
	var query: PhysicsRayQueryParameters3D = PhysicsRayQueryParameters3D.create(
		Vector3(x, from_y, z), Vector3(x, -5.0, z), mask)
	var hit: Dictionary = space.intersect_ray(query)
	if hit.is_empty():
		return -1000.0
	var pos: Vector3 = hit["position"]
	return pos.y


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
