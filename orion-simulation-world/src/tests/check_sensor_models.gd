# Presented by KeJi
# Created Date ： 2026-10-23
# Modified Date ： 2026-10-23
#
# task_33 Step 1 — 传感器 / 底盘模型资产自检（29 项断言）。
# 用法：timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_sensor_models.gd
# 期望：total=29 failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR。
#
# 前置条件（顺序固定）：① build_models.py → ② export_all.sh → ③ `--import` → ④ 本脚本。
#   缺 `.import` 时 `load()` **静默返回 null**（不报错）→ 本脚本把它判成 STRUCT 失败并提示
#   先跑 `godot --headless --path orion-simulation-world --import`。
#
# 决定本文件写法的实测事实：
#   - glb 导入后**根节点是 Node3D、transform == IDENTITY**，子节点是 MeshInstance3D，
#     每个 MeshInstance3D 一个 surface、一个材质（导出时一对象一材质）。
#   - glTF 是 Y-up，Godot 也是 Y-up → 不做二次换轴：
#       Blender (x, y, z) = Godot (x, z, −y)，即 **Godot x = 长、y = 高、z = 宽**。
#   - `Vector3` 是**值类型**：AABB 累加不能用"传出参数"，必须写进 Dictionary（R6）。
#   - `quit()` 前不 `queue_free()` → 退出时报 `ERROR: resources still in use at exit`（R7）。
#     固定序列：add_child → await process_frame → 断言 → queue_free → await process_frame → quit。
#
# 注意（GDScript 4.7 类型推断陷阱）：load() / instantiate() / get() 的返回值一律显式标注类型。

extends SceneTree

## ── 资产路径 ──────────────────────────────────────────────
const LIDAR_GLB := "res://assets/sensors/lidar_tmini.glb"
const CAMERA_GLB := "res://assets/sensors/camera_module.glb"
const UGV_GLB := "res://assets/chassis/ugv_chassis.glb"
const UAV_GLB := "res://assets/uav_quadrotor_lowpoly.glb"

## ── 期望 AABB（Godot 轴序：x 长 / y 高 / z 宽，米）──────────
const LIDAR_SIZE := Vector3(0.080, 0.054, 0.080)
const LIDAR_CAP := Vector3(0.09, 0.06, 0.09)      # step 验收上限（≤）
const CAMERA_SIZE := Vector3(0.055, 0.036, 0.030)
const CAMERA_CAP := Vector3(0.06, 0.04, 0.04)     # step 验收上限（≤）
const UGV_SIZE := Vector3(0.30, 0.25, 0.25)       # step 硬性：逐轴相等
const UAV_SIZE := Vector3(0.64, 0.14, 0.64)       # Step 1 复核锁定值（防误删/误改）

## ── 容差 / 预算 ───────────────────────────────────────────
const TOL_ABS := 2.0e-4          # 传感器尺寸 / 原点绝对容差（米）
const UGV_TOL_REL := 0.02        # 底盘 ±2%（step 允许 ±5%）
const UAV_TOL_REL := 0.02        # 复核值 ±2%
const TRI_LIMIT := 2000          # 单件三角面预算
const GLB_LIMIT_B := 100 * 1024  # 单件 glb 体积上限
const CAMERA_BRACKET_DROP := -0.021  # 支架底面 y（机身下方，证明"装得低"而不是居中悬挂）

## ── 无碰撞体判据（D8 硬性）────────────────────────────────
const FORBIDDEN_NODE_TYPES := ["CollisionShape3D", "CollisionPolygon3D", "StaticBody3D",
	"RigidBody3D", "Area3D", "CharacterBody3D", "AnimatableBody3D", "PhysicsBody3D"]

var _total: int = 0
var _failed: int = 0
var _aborted: bool = false


func _initialize() -> void:
	_run()


func _run() -> void:
	var specs: Array[Dictionary] = [
		{"label": "lidar", "path": LIDAR_GLB, "size": LIDAR_SIZE, "cap": LIDAR_CAP,
			"tol": TOL_ABS},
		{"label": "camera", "path": CAMERA_GLB, "size": CAMERA_SIZE, "cap": CAMERA_CAP,
			"tol": TOL_ABS},
		{"label": "ugv", "path": UGV_GLB, "size": UGV_SIZE, "cap": UGV_SIZE,
			"tol": UGV_SIZE.x * UGV_TOL_REL},
	]

	# ============================================================ ① 载入 + 入树
	var roots: Array[Node] = []
	for spec in specs:
		var label: String = spec["label"]
		var path: String = spec["path"]
		var packed: PackedScene = load(path) as PackedScene
		if packed == null:
			_fatal("%s-loads" % label, "%s 不可加载（请先跑 godot --headless --path "
				% path + "orion-simulation-world --import）")
			break
		_check("%s-loads" % label, true, path)
		var inst: Node = packed.instantiate()
		if inst == null:
			_fatal("%s-instantiates" % label, "%s 实例化失败" % path)
			break
		_check("%s-instantiates" % label, inst != null, inst.get_class())
		root.add_child(inst)
		roots.append(inst)
	if not _aborted:
		await process_frame

	# ============================================================ ② 度量 + 断言
	var stats: Array[Dictionary] = []
	if not _aborted:
		for i in range(specs.size()):
			stats.append(_measure(roots[i]))

	if not _aborted:
		for i in range(specs.size()):
			var spec: Dictionary = specs[i]
			var label: String = spec["label"]
			var acc: Dictionary = stats[i]
			var mn: Vector3 = acc["min"]
			var mx: Vector3 = acc["max"]
			var got_size: Vector3 = mx - mn
			var want_size: Vector3 = spec["size"]
			var cap: Vector3 = spec["cap"]
			var tol: float = spec["tol"]
			var center: Vector3 = (mn + mx) * 0.5

			# 3) 根节点 transform 必须是 IDENTITY（原点约定靠它落地）
			var root3d: Node3D = roots[i] as Node3D
			var root_xf: Transform3D = root3d.transform if root3d != null else Transform3D.IDENTITY
			_check("%s-root-identity" % label,
				root3d != null and root_xf.is_equal_approx(Transform3D.IDENTITY),
				"xf=%s" % root_xf)

			# 4) AABB 尺寸：逐轴命中期望值 **且** 不超过 step 上限
			var size_ok: bool = true
			for axis in range(3):
				if absf(got_size[axis] - want_size[axis]) > tol:
					size_ok = false
				if got_size[axis] > cap[axis] + TOL_ABS:
					size_ok = false
			_check("%s-aabb-size" % label, size_ok,
				"size=%s want=%s cap=%s" % [got_size, want_size, cap])

			# 5) 原点：逐件的"钉死点"不同
			var origin_ok: bool = _origin_ok(label, mn, mx, center)
			_check("%s-origin" % label, origin_ok,
				"min=%s max=%s center=%s" % [mn, mx, center])

			# 6) 无碰撞体 / 无物理体
			var shapes: int = int(acc["shapes"])
			var bodies: int = int(acc["bodies"])
			_check("%s-no-collision" % label, shapes == 0 and bodies == 0,
				"shapes=%d bodies=%d" % [shapes, bodies])

			# 7) 材质非空（每个 surface 都有材质）
			var meshes: int = int(acc["meshes"])
			var null_mats: int = int(acc["null_mats"])
			_check("%s-materials" % label, meshes > 0 and null_mats == 0,
				"meshes=%d null_mats=%d" % [meshes, null_mats])

			# 8) 三角面预算
			var tris: int = int(acc["tris"])
			_check("%s-tri-budget" % label, tris > 0 and tris <= TRI_LIMIT,
				"tris=%d limit=%d" % [tris, TRI_LIMIT])

			# 9) 文件体积
			var size_b: int = _file_size(spec["path"])
			_check("%s-file-size" % label, size_b > 0 and size_b <= GLB_LIMIT_B,
				"bytes=%d limit=%d" % [size_b, GLB_LIMIT_B])

	# ============================================================ ③ UAV 复核锁定
	var uav_packed: PackedScene = load(UAV_GLB) as PackedScene
	if _check("uav-loads", uav_packed != null, UAV_GLB) and uav_packed != null:
		var uav_node: Node = uav_packed.instantiate()
		root.add_child(uav_node)
		await process_frame
		var uav_acc: Dictionary = _measure(uav_node)
		var uav_size: Vector3 = (uav_acc["max"] as Vector3) - (uav_acc["min"] as Vector3)
		var uav_ok: bool = true
		for axis in range(3):
			if absf(uav_size[axis] - UAV_SIZE[axis]) > UAV_SIZE[axis] * UAV_TOL_REL:
				uav_ok = false
		_check("uav-aabb", uav_ok, "size=%s want=%s(±2%%)" % [uav_size, UAV_SIZE])
		uav_node.queue_free()
		await process_frame

	# ============================================================ ④ 释放 + 退出
	for node in roots:
		node.queue_free()
	await process_frame
	_finish()


## 逐件的原点判据（D8）。
##   lidar  → 罐体底面中心：min.y ≈ 0、x 中心 ≈ 0、z 中心 ≈ 0
##   camera → 镜片前表面中心（光心）：max.x ≈ 0、z 中心 ≈ 0（光轴在机身横向中面）、支架在机身下方
##   ugv    → 车体底面中心：min.y ≈ 0、x 中心 ≈ 0、z 中心 ≈ 0
func _origin_ok(label: String, mn: Vector3, mx: Vector3, center: Vector3) -> bool:
	match label:
		"lidar":
			return absf(mn.y) <= TOL_ABS and absf(center.x) <= TOL_ABS \
				and absf(center.z) <= TOL_ABS
		"camera":
			return absf(mx.x) <= TOL_ABS and absf(center.z) <= TOL_ABS \
				and absf(mn.y - CAMERA_BRACKET_DROP) <= TOL_ABS
		"ugv":
			return absf(mn.y) <= TOL_ABS and absf(center.x) <= TOL_ABS \
				and absf(center.z) <= TOL_ABS
		_:
			return false


## 递归度量：AABB / 三角数 / 网格数 / 空材质数 / 碰撞体数。
## 结果写进 Dictionary（Vector3 是值类型，传出参数拿不到结果 —— plan R6）。
func _measure(node: Node, xf: Transform3D = Transform3D.IDENTITY) -> Dictionary:
	var acc: Dictionary = {
		"min": Vector3(1e9, 1e9, 1e9),
		"max": Vector3(-1e9, -1e9, -1e9),
		"tris": 0,
		"shapes": 0,
		"bodies": 0,
		"meshes": 0,
		"null_mats": 0,
	}
	_measure_into(node, xf, acc)
	return acc


func _measure_into(node: Node, xf: Transform3D, acc: Dictionary) -> void:
	var local_xf: Transform3D = xf
	var node3d: Node3D = node as Node3D
	if node3d != null:
		local_xf = xf * node3d.transform
	if FORBIDDEN_NODE_TYPES.has(node.get_class()):
		if node is CollisionShape3D or node is CollisionPolygon3D:
			acc["shapes"] = int(acc["shapes"]) + 1
		else:
			acc["bodies"] = int(acc["bodies"]) + 1

	var mi: MeshInstance3D = node as MeshInstance3D
	if mi != null and mi.mesh != null:
		acc["meshes"] = int(acc["meshes"]) + 1
		var aabb: AABB = mi.mesh.get_aabb()
		for corner in range(8):
			_acc_point(acc, local_xf * aabb.get_endpoint(corner))
		var am: ArrayMesh = mi.mesh as ArrayMesh
		if am != null:
			for s in range(am.get_surface_count()):
				if am.surface_get_material(s) == null:
					acc["null_mats"] = int(acc["null_mats"]) + 1
				var arrays: Array = am.surface_get_arrays(s)
				var idx: PackedInt32Array = arrays[Mesh.ARRAY_INDEX]
				if idx.size() > 0:
					acc["tris"] = int(acc["tris"]) + idx.size() / 3
				else:
					var verts: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
					acc["tris"] = int(acc["tris"]) + verts.size() / 3
	for child in node.get_children():
		_measure_into(child, local_xf, acc)


func _acc_point(acc: Dictionary, point: Vector3) -> void:
	var mn: Vector3 = acc["min"]
	var mx: Vector3 = acc["max"]
	acc["min"] = Vector3(minf(mn.x, point.x), minf(mn.y, point.y), minf(mn.z, point.z))
	acc["max"] = Vector3(maxf(mx.x, point.x), maxf(mx.y, point.y), maxf(mx.z, point.z))


func _file_size(path: String) -> int:
	if not FileAccess.file_exists(path):
		return -1
	var handle: FileAccess = FileAccess.open(path, FileAccess.READ)
	if handle == null:
		return -2
	var size_b: int = handle.get_length()
	handle.close()
	return size_b


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


func _fatal(label: String, detail: String) -> void:
	_check(label, false, "STRUCT " + detail)
	_aborted = true


func _finish() -> void:
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
