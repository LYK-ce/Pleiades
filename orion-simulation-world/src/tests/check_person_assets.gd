# Presented by KeJi
# Created Date ： 2026-10-24
# Modified Date ： 2026-10-24
#
# 静态人物自检（支线 human_model Step 4）：两件人形资产可 load / AABB 人高 / 脚底在地面 /
# 零碰撞 / 材质与贴图在 / 三角数预算 / sha256 与 Step 3 证据链一致；道具场景 Person.tscn
# 可实例化且零碰撞；脚本常量与场景自洽；世界程序化摆位 2 人、净空、幂等、不污染 Vehicles。
# 用法：timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_person_assets.gd
# 期望：total=32 failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR。
#
# 决定本文件写法的实测事实：
#   - glb 导入后的节点形态（实测）：根 `Node3D`（名 = glb 文件名 stem）→ 1 个 `MeshInstance3D`；
#     单 mesh / 单 surface / 无骨骼 / 无动画 / 无 CollisionShape3D。
#   - `mi.get_aabb()` 是 **mesh 局部 AABB**（不含节点自身的旋转/缩放）→ 必须 8 角点过变换
#     （与 check_models.gd 同款 `_aabb_of_local_in`）。
#   - 树外读 `global_*` 会 ERROR（check_models.gd 头注释 P8）→ A 组一律用**局部变换累加**，
#     只有入树后的 B/D 组才用 `global_transform`。
#   - 轴口径：glTF/Godot 是 Y-up（x=前后、y=高、z=左右），step.md 的"脚底 z≈0"是**世界坐标**
#     （Z-up）口径。D4 两边都锁：Godot AABB `position.y ≈ 0` + `PosToWorld(pos).z ≈ 0`。
#   - 世界坐标 (x, y, z) → Godot (x, z, y)；Godot AABB 的 z 跨度 = 世界 y 跨度（D8 足迹换算）。
#
# 注意（GDScript 4.7 类型推断陷阱）：load() / instantiate() / call() 的返回值一律显式标注类型。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldGd := preload("res://src/world/World.gd")
const PersonGd := preload("res://src/props/Person.gd")

const PERSON_SCENE := "res://src/scenes/props/Person.tscn"
const PERSON_SCRIPT := "res://src/props/Person.gd"
const WORLD_SCENE := "res://src/scenes/World.tscn"

## 人高容差 / 摆位容差 / 脚底容差（米）。
const HEIGHT_TOL := 0.01
const POS_TOL := 1.0e-4
const FEET_TOL := 1.0e-3
## 单模型三角数上限（design.md 口径，远超实际 ~20k，防回归到"高模塞仓库"）。
const TRI_CAP := 150000

## A 组的两件资产：期望人高 / 三角数 / sha256 全部取自 Step 3 的 metrics.json + SOURCES.tsv
## （tmp/human_model/normalized/），这里逐位钉死，防"仓库资产被换掉而没人发现"。
const MODELS := [
	{
		"name": "rp_carla_rigged_001",
		"glb": "res://assets/humans/rp_carla_rigged_001/model.glb",
		"height": 1.7307,
		"tris": 19988,
		"sha256": "bc9677b8d990b2016fd88759b7d999a525990730584b731a1f4b4cced4d24690",
	},
	{
		"name": "rp_eric_rigged_001",
		"glb": "res://assets/humans/rp_eric_rigged_001/model.glb",
		"height": 1.8400,
		"tris": 20542,
		"sha256": "12e959ba199c4f599ee8c7db4d60ddd9a9af1d5fdee9c5e7d7c4f52873458538",
	},
]

var _total: int = 0
var _failed: int = 0
## B 组测得的 Body.scene_file_path，供 C1 复用。
var _scene_body_path: String = ""


func _initialize() -> void:
	_run()


func _run() -> void:
	_run_assets()
	await _run_scene()
	_run_script_consts()
	await _run_world()
	_finish()


# ================================================================ A 组：两件资产（8 项 / 件）

func _run_assets() -> void:
	for raw: Variant in MODELS:
		var spec: Dictionary = raw
		var tag: String = String(spec["name"])
		var glb: String = String(spec["glb"])
		var want_h: float = float(spec["height"])

		# A1 asset-loads：PackedScene 能 load()（.godot 缓存缺失时这里会红 → 先跑 --import）。
		var packed: PackedScene = load(glb) as PackedScene
		_check("asset-loads-" + tag, packed != null, glb)

		# A2 asset-instantiates：根 Node3D + 唯一子节点是带 mesh 的 MeshInstance3D。
		var body: Node3D = null
		if packed != null:
			body = packed.instantiate() as Node3D
		var mi: MeshInstance3D = null
		if body != null and body.get_child_count() == 1:
			mi = body.get_child(0) as MeshInstance3D
		_check("asset-instantiates-" + tag, body != null and mi != null and mi.mesh != null,
			"children=%d child=%s" % [body.get_child_count() if body != null else -1,
				mi.get_class() if mi != null else "null"])

		# A3 asset-aabb：局部变换累加后的人高 / 区间 / "前后薄于左右"（朝 +X 的弱几何证据）。
		var box: AABB = _aabb_of_local_tree(body)
		_check("asset-aabb-" + tag,
			absf(box.size.y - want_h) <= HEIGHT_TOL
				and box.size.y >= PersonGd.HEIGHT_MIN_M and box.size.y <= PersonGd.HEIGHT_MAX_M
				and box.size.x < box.size.z,
			"%s want_y=%.4f range=[%.2f, %.2f]" % [_fmt(box), want_h,
				PersonGd.HEIGHT_MIN_M, PersonGd.HEIGHT_MAX_M])

		# A4 asset-feet：脚底 = 模型局部地面（glTF POSITION.min[1] == 0）。
		_check("asset-feet-" + tag, absf(box.position.y) <= FEET_TOL,
			"position.y=%.6f" % box.position.y)

		# A5 asset-no-collision：纯视觉，子树必须在引擎物理层完全不可见（E27 同口径）。
		var physics: int = _count_physics(body)
		_check("asset-no-collision-" + tag, physics == 0, "physics_nodes=%d" % physics)

		# A6 asset-material：单 surface + StandardMaterial3D + basecolor/normal 贴图都在。
		var surf: int = -1
		var mat: Material = null
		var std: StandardMaterial3D = null
		if mi != null and mi.mesh != null:
			surf = mi.mesh.get_surface_count()
			mat = mi.get_active_material(0)
			std = mat as StandardMaterial3D
		_check("asset-material-" + tag,
			surf == 1 and std != null and std.albedo_texture != null
				and std.normal_enabled and std.normal_texture != null,
			"surfaces=%d mat=%s albedo=%s normal=%s" % [surf,
				std.get_class() if std != null else (mat.get_class() if mat != null else "null"),
				std.albedo_texture != null if std != null else false,
				(std.normal_enabled and std.normal_texture != null) if std != null else false])

		# A7 asset-tri-budget：三角数与 Step 3 逐位一致，且远低于 150k 预算。
		var tris: int = -1
		if mi != null and mi.mesh != null:
			tris = mi.mesh.get_faces().size() / 3
		_check("asset-tri-budget-" + tag,
			tris == int(spec["tris"]) and tris > 0 and tris <= TRI_CAP,
			"tris=%d want=%d cap=%d" % [tris, int(spec["tris"]), TRI_CAP])

		# A8 asset-sha256：仓库里的字节 == Step 3 记录的字节（证据链闭环）。
		var got: String = _sha256_of(glb)
		_check("asset-sha256-" + tag, got == String(spec["sha256"]), got)

		if body != null:
			body.free()   # 树外实例：free() 即可（未入树，无排队释放语义）


# ================================================================ B 组：Person.tscn（5 项）

func _run_scene() -> void:
	var packed: PackedScene = load(PERSON_SCENE) as PackedScene
	var probe: Node3D = null
	if packed != null:
		probe = packed.instantiate() as Node3D
	var probe_body: Node3D = null
	if probe != null:
		probe_body = probe.get_node_or_null("Body") as Node3D

	# B1 scene-loads：场景可加载、根是 Node3D 名为 Person、根脚本是 Person.gd。
	var script_path: String = ""
	if probe != null and probe.get_script() != null:
		script_path = String(probe.get_script().resource_path)
	_check("scene-loads", probe != null and probe.name == "Person" and script_path == PERSON_SCRIPT,
		"%s root=%s script=%s" % [PERSON_SCENE, probe.name if probe != null else "null",
			script_path])

	# B2 scene-body-mount：Body 存在、静态挂 glb、局部变换 = IDENTITY（原点 = 脚底中心）。
	var body_path: String = ""
	if probe_body != null:
		body_path = probe_body.scene_file_path
	_check("scene-body-mount", probe_body != null and body_path == PersonGd.MODEL_GLB
		and probe_body.transform.is_equal_approx(Transform3D.IDENTITY),
		"body=%s glb=%s xf=%s" % [probe_body, body_path,
			probe_body.transform if probe_body != null else "null"])

	if probe == null:
		_check("scene-no-collision", false, "前置失败：Person.tscn 不可实例化")
		_check("scene-instantiable", false, "前置失败：Person.tscn 不可实例化")
		_check("scene-material", false, "前置失败：Person.tscn 不可实例化")
		return

	root.add_child(probe)
	await process_frame

	# B3 scene-no-collision：整棵子树零物理节点（人只是"给 YOLO 看的一张图"）。
	var physics: int = _count_physics(probe)
	_check("scene-no-collision", physics == 0, "physics_nodes=%d" % physics)

	# B4 scene-instantiable：入树后仍有 1 个 mesh，人高 ≈ 1.7307 m、脚底 y ≈ 0。
	var box: AABB = _aabb_of_global_tree(probe)
	_check("scene-instantiable",
		_count_meshes(probe) == 1
			and absf(box.size.y - float(MODELS[0]["height"])) <= HEIGHT_TOL
			and absf(box.position.y) <= FEET_TOL,
		"meshes=%d %s" % [_count_meshes(probe), _fmt(box)])

	# B5 scene-material：实例化后 surface 0 的材质与 basecolor 贴图仍在（没被抽取丢图）。
	var inst_mi: MeshInstance3D = null
	var meshes: Array[MeshInstance3D] = _collect_meshes(probe)
	if meshes.size() == 1:
		inst_mi = meshes[0]
	var inst_mat: Material = inst_mi.get_active_material(0) if inst_mi != null else null
	var inst_std: StandardMaterial3D = inst_mat as StandardMaterial3D
	_check("scene-material", inst_std != null and inst_std.albedo_texture != null,
		"mat=%s albedo=%s" % [
			inst_std.get_class() if inst_std != null else (inst_mat.get_class() if inst_mat != null else "null"),
			inst_std.albedo_texture != null if inst_std != null else false])

	# 供 C1 复用：Body 的 scene_file_path。
	_scene_body_path = body_path


# ================================================================ C 组：脚本常量自洽（2 项）

func _run_script_consts() -> void:
	# C1 script-model-const：脚本常量 == .tscn 里 Body 的 glb 路径（防两处漂移）。
	_check("script-model-const", _scene_body_path == PersonGd.MODEL_GLB,
		"script=%s scene=%s" % [PersonGd.MODEL_GLB, _scene_body_path])

	# C2 script-height-const：人高判据与 step.md / 本文件 A3 同口径。
	_check("script-height-const",
		PersonGd.HEIGHT_MIN_M == 1.60 and PersonGd.HEIGHT_MAX_M == 1.85,
		"min=%.2f max=%.2f" % [PersonGd.HEIGHT_MIN_M, PersonGd.HEIGHT_MAX_M])


# ================================================================ D 组：世界摆位（9 项）

func _run_world() -> void:
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	var world: Node3D = null
	if packed != null:
		world = packed.instantiate() as Node3D
	if world == null:
		_fatal("world-persons-root", "%s 不可加载/实例化" % WORLD_SCENE)
		for label in ["world-persons-count", "world-persons-placement", "world-persons-feet",
				"world-persons-no-collision", "world-persons-idempotent",
				"world-persons-spawn-clear", "world-persons-obstacle-clear",
				"world-persons-not-vehicles"]:
			_fatal(label, "前置失败：World.tscn 不可实例化")
		return

	root.add_child(world)
	# 物理空间在 add_child 后第一个物理帧才同步（与 check_world.gd R7 同款，这里只读变换，两帧足够）。
	await physics_frame
	await physics_frame

	# D1 world-persons-root：容器存在、名字正确、直接挂在 world 下。
	var persons: Node3D = _persons_root(world)
	_check("world-persons-root",
		persons != null and persons.name == "Persons" and persons.get_parent() == world,
		"root=%s parent=%s" % [persons, persons.get_parent() if persons != null else "null"])

	# D2 world-persons-count：数量 == 布局表条数 == 2，名字与顺序与常量表一致，实例都来自 Person.tscn。
	var specs: Array = WorldGd.PERSONS
	var want: int = specs.size()
	var names_ok: bool = true
	var scene_ok: bool = true

	var kids: Array[Node] = []
	if persons != null:
		kids = persons.get_children()
	for i in range(kids.size()):
		var kid: Node = kids[i]
		if i >= want:
			names_ok = false
			break
		var spec: Dictionary = specs[i]
		if String(kid.name) != String(spec["name"]):
			names_ok = false
		var kid3: Node3D = kid as Node3D
		if kid3 == null or kid3.scene_file_path != WorldGd.PERSON_SCENE:
			scene_ok = false
	_check("world-persons-count",
		persons != null and kids.size() == want and want == 2 and names_ok and scene_ok,
		"count=%d want=%d names_ok=%s scene_ok=%s scene=%s"
			% [kids.size(), want, names_ok, scene_ok, WorldGd.PERSON_SCENE])

	# D3 world-persons-placement：逐人锁死世界坐标与 yaw（换轴只经 WorldTransform）。
	var place_ok: bool = true
	var place_detail: String = ""
	for i in range(mini(kids.size(), want)):
		var spec: Dictionary = specs[i]
		var person: Node3D = kids[i] as Node3D
		if person == null:
			place_ok = false
			continue
		var want_pos: Vector3 = WorldTransform.PosToGodot(spec["pos"])
		var want_yaw: float = WorldTransform.YawToGodot(deg_to_rad(float(spec["yaw_deg"])))
		var dp: float = (person.global_position - want_pos).length()
		var dy: float = absf(wrapf(person.rotation.y - want_yaw, -PI, PI))
		if dp > POS_TOL or dy > POS_TOL:
			place_ok = false
		place_detail += "%s pos=%s d=%.6f yaw=%.6f dy=%.6f; " % [
			person.name, person.global_position, dp, person.rotation.y, dy]
	_check("world-persons-placement", place_ok and kids.size() == want,
		place_detail if not place_detail.is_empty() else "no persons")

	# D4 world-persons-feet：世界系脚底 z == 0（= Godot AABB position.y == 0），两边都锁。
	var feet_ok: bool = true
	var feet_detail: String = ""
	for kid: Node in kids:
		var person: Node3D = kid as Node3D
		if person == null:
			feet_ok = false
			continue
		var box: AABB = _aabb_of_global_tree(person)
		var world_pos: Vector3 = WorldTransform.PosToWorld(person.global_position)
		if absf(box.position.y) > FEET_TOL or absf(world_pos.z) > FEET_TOL:
			feet_ok = false
		feet_detail += "%s aabb_y=%.6f world_z=%.6f; " % [
			person.name, box.position.y, world_pos.z]
	_check("world-persons-feet", feet_ok and kids.size() > 0,
		feet_detail if not feet_detail.is_empty() else "no persons")

	# D5 world-persons-no-collision：整个 Persons 子树零物理节点（check_lidar F7/F8 的硬前提）。
	var physics: int = _count_physics(persons)
	_check("world-persons-no-collision", physics == 0, "physics_nodes=%d" % physics)

	# D6 world-persons-idempotent：连调两次 BuildPersons()，不重复实例化、节点总数不增长。
	var nodes_before: int = _count_nodes(world)
	world.call("BuildPersons")
	await process_frame
	world.call("BuildPersons")
	await process_frame
	var nodes_after: int = _count_nodes(world)
	var same_name: int = 0
	for child: Node in world.get_children():
		if String(child.name) == String(WorldGd.PERSONS_NODE):
			same_name += 1
	var persons_after: Node3D = _persons_root(world)
	var count_after: int = persons_after.get_child_count() if persons_after != null else -1
	_check("world-persons-idempotent",
		count_after == want and same_name == 1 and nodes_after == nodes_before,
		"count=%d want=%d same_name_nodes=%d nodes=%d->%d"
			% [count_after, want, same_name, nodes_before, nodes_after])

	# 重建后重取容器（旧容器已 queue_free，断言要用新的这一份）。
	persons = _persons_root(world)
	kids = []
	if persons != null:
		kids = persons.get_children()

	# D7 world-persons-spawn-clear：水平距出生点 > 净空半径（3 m）。
	var spawn_ok: bool = true
	var spawn_detail: String = ""
	var spawn: Vector3 = WorldGd.SPAWN_WORLD
	var clear_r: float = WorldGd.SPAWN_CLEAR_RADIUS_M
	for kid: Node in kids:
		var person: Node3D = kid as Node3D
		if person == null:
			spawn_ok = false
			continue
		var w: Vector3 = WorldTransform.PosToWorld(person.global_position)
		var dist: float = Vector2(w.x - spawn.x, w.y - spawn.y).length()
		if dist <= clear_r:
			spawn_ok = false
		spawn_detail += "%s dist=%.3f; " % [person.name, dist]
	_check("world-persons-spawn-clear", spawn_ok and kids.size() > 0,
		"%s(clear=%.1f m)" % [spawn_detail, clear_r])

	# D8 world-persons-obstacle-clear：人的世界 xy 足迹与 OBSTACLES 全部盒子无交。
	#   Godot AABB 的 x 跨度 = 世界 x 跨度、z 跨度 = 世界 y 跨度（换轴规则 (x, z, y)）。
	var obstacles: Array = WorldGd.OBSTACLES
	var obs_ok: bool = true
	var obs_detail: String = ""
	for kid: Node in kids:
		var person: Node3D = kid as Node3D
		if person == null:
			obs_ok = false
			continue
		var box: AABB = _aabb_of_global_tree(person)
		var px0: float = box.position.x
		var px1: float = box.position.x + box.size.x
		var py0: float = box.position.z
		var py1: float = box.position.z + box.size.z
		for raw: Variant in obstacles:
			var obs: Dictionary = raw
			var origin: Vector3 = obs["origin"]
			var size: Vector3 = obs["size"]
			var ox0: float = origin.x
			var ox1: float = origin.x + size.x
			var oy0: float = origin.y
			var oy1: float = origin.y + size.y
			var hit: bool = px0 < ox1 and px1 > ox0 and py0 < oy1 and py1 > oy0
			if hit:
				obs_ok = false
				obs_detail += "%s hits %s; " % [person.name, String(obs.get("name", "?"))]
		obs_detail += "%s rect=[%.3f, %.3f]x[%.3f, %.3f]; " % [person.name, px0, px1, py0, py1]
	_check("world-persons-obstacle-clear", obs_ok and kids.size() > 0,
		"%s(obstacles=%d)" % [obs_detail, obstacles.size()])

	# D9 world-persons-not-vehicles：父节点是 Persons 根；Vehicles 装配容器仍为空
	#    （WebSocketManager 的 avatar 计数 / check_ws_manager 的 vehicle 子树计数不受影响）。
	var vehicles: Node3D = world.get_node_or_null("Vehicles") as Node3D
	var parent_ok: bool = persons != null
	for kid: Node in kids:
		if kid.get_parent() != persons:
			parent_ok = false
	var veh_count: int = vehicles.get_child_count() if vehicles != null else -1
	_check("world-persons-not-vehicles",
		parent_ok and kids.size() > 0 and vehicles != null and veh_count == 0,
		"parent_ok=%s vehicles_children=%d" % [parent_ok, veh_count])

	world.queue_free()
	await process_frame
	await process_frame


func _persons_root(world: Node3D) -> Node3D:
	if world == null or not world.has_method("GetPersonsRoot"):
		return null
	return world.call("GetPersonsRoot") as Node3D


# ================================================================ AABB（两条路径）

## mesh 的 8 个局部角点过 xf 后重新求包围盒。
## （`get_aabb()` 不含节点自身的旋转/缩放，直接变换 position/size 会得到偏大的盒子。）
func _aabb_of_local_in(xf: Transform3D, local: AABB) -> AABB:
	var out: AABB = AABB()
	var first: bool = true
	for i in range(8):
		var corner: Vector3 = xf * local.get_endpoint(i)
		if first:
			out = AABB(corner, Vector3.ZERO)
			first = false
		else:
			out = out.expand(corner)
	return out


## 树外：按局部变换逐级累加（根节点自身的 transform 也计入）。
func _aabb_of_local_tree(node: Node) -> AABB:
	var boxes: Array[AABB] = []
	_gather_local(node, Transform3D.IDENTITY, boxes)
	return _merge(boxes)


func _gather_local(node: Node, parent_xf: Transform3D, boxes: Array[AABB]) -> void:
	if node == null:
		return
	var xf: Transform3D = parent_xf
	if node is Node3D:
		xf = parent_xf * (node as Node3D).transform
	if node is MeshInstance3D:
		var mi: MeshInstance3D = node
		if mi.mesh != null:
			boxes.append(_aabb_of_local_in(xf, mi.get_aabb()))
	for child: Node in node.get_children():
		_gather_local(child, xf, boxes)


## 入树后：用 global_transform（树外读 global_* 会 ERROR，故只在 B/D 组用）。
func _aabb_of_global_tree(node: Node) -> AABB:
	var boxes: Array[AABB] = []
	_gather_global(node, boxes)
	return _merge(boxes)


func _gather_global(node: Node, boxes: Array[AABB]) -> void:
	if node == null:
		return
	if node is MeshInstance3D:
		var mi: MeshInstance3D = node
		if mi.mesh != null:
			boxes.append(_aabb_of_local_in(mi.global_transform, mi.get_aabb()))
	for child: Node in node.get_children():
		_gather_global(child, boxes)


func _merge(boxes: Array[AABB]) -> AABB:
	if boxes.is_empty():
		return AABB()
	var out: AABB = boxes[0]
	for i in range(1, boxes.size()):
		out = out.merge(boxes[i])
	return out


# ================================================================ 统计 / 工具

## 物理节点计数（E27：人物资产与道具场景必须是 0）。
func _count_physics(node: Node) -> int:
	if node == null:
		return 0
	var n: int = 0
	if node is CollisionShape3D or node is CollisionPolygon3D \
			or node is PhysicsBody3D or node is Area3D:
		n += 1
	for child: Node in node.get_children():
		n += _count_physics(child)
	return n


## 节点总数（含自身）。
func _count_nodes(node: Node) -> int:
	if node == null:
		return 0
	var n: int = 1
	for child: Node in node.get_children():
		n += _count_nodes(child)
	return n


func _collect_meshes(node: Node) -> Array[MeshInstance3D]:
	var out: Array[MeshInstance3D] = []
	if node == null:
		return out
	if node is MeshInstance3D:
		var mi: MeshInstance3D = node
		if mi.mesh != null:
			out.append(mi)
	for child: Node in node.get_children():
		out.append_array(_collect_meshes(child))
	return out


func _count_meshes(node: Node) -> int:
	return _collect_meshes(node).size()


## 文件 sha256（十六进制小写）；打不开返回 "open-failed:<code>"（断言会红，不崩）。
func _sha256_of(path: String) -> String:
	var f: FileAccess = FileAccess.open(path, FileAccess.READ)
	if f == null:
		return "open-failed:%d" % FileAccess.get_open_error()
	var hc := HashingContext.new()
	hc.start(HashingContext.HASH_SHA256)
	var chunk: PackedByteArray = f.get_buffer(65536)
	while chunk.size() > 0:
		hc.update(chunk)
		chunk = f.get_buffer(65536)
	f.close()
	return hc.finish().hex_encode()


func _fmt(a: AABB) -> String:
	return "P(%.3f, %.3f, %.3f) S(%.3f, %.3f, %.3f)" % [a.position.x, a.position.y,
		a.position.z, a.size.x, a.size.y, a.size.z]


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
	print("[check] ", label, " FAIL ", detail)


func _finish() -> void:
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
