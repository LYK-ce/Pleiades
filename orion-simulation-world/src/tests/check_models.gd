# Presented by KeJi
# Created Date ： 2026-10-22
# Modified Date ： 2026-10-22
#
# 低模外观自检（Step 2）：模型静态挂在 Vehicle.tscn 上 / 显隐正确 / 尺寸朝向正确。
# 用法：timeout 180 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_models.gd
# 期望：total=16 failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR。
#
# 决定本文件写法的实测事实（plan.md §0 / §3）：
#   - glb 里**没有**名字含 Nose 的节点：红色车头是 Body 这个 MeshInstance3D 的第 2 个
#     surface（材质名 UGV_NoseRed / UAV_NoseRed），所以朝向断言必须"节点名 + 材质名"双通道。
#   - UGV 模型在 Vehicle 坐标系的整体 AABB = P(-0.21, 0.00, -0.14) S(0.42, 0.16, 0.28)；
#     UAV = P(-0.32, 0.00, -0.32) S(0.64, 0.14, 0.64)。
#   - 节点总数：空车 22、装一个底盘 28（模型节点是静态实例，装配只改 visible）。
#   - mi.get_aabb() 是 **mesh 局部 AABB**（不含节点自身旋转/缩放）→ 必须 8 角点过变换。
#   - 树外读 global_transform 会 ERROR（P8）→ AABB / 朝向断言只能在 add_child 之后。
#   - _ensure_refs() 会在 _ready 里按名补齐引用 → E16 真绑定只能用**树外探针**证明。
#
# 注意（GDScript 4.7 类型推断陷阱）：load() / instantiate() / get() 的返回值一律显式标注类型。
# 注意（P15）：含 await 的辅助函数必须被 await 调用 → 本文件所有辅助函数都不含 await，
#   await 只内联在 _run 里。

extends SceneTree

const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const UGV_MODEL_SCENE := "res://assets/ugv_x3plus_lowpoly.glb"
const UAV_MODEL_SCENE := "res://assets/uav_quadrotor_lowpoly.glb"
const UGV_CHASSIS := "diff_drive_ugv"
const UAV_CHASSIS := "quadrotor_uav"

## M3/M5：模型整体 AABB 期望尺寸（Vehicle 坐标系，米）+ 容差。
const UGV_SIZE := Vector3(0.42, 0.16, 0.28)
const UAV_SIZE_XZ := 0.64
const UAV_SIZE_Y_MIN := 0.11
const UAV_SIZE_Y_MAX := 0.15
const TOL := 0.01

## M6：碰撞形状尺寸必须仍是底盘参数（模型只是外观层，一行物理都不许跟着变）。
const UGV_HULL := Vector3(0.30, 0.16, 0.24)
const UAV_HULL := Vector3(0.40, 0.12, 0.40)

## M2/M5：Wheels / Rotors 下的视觉件个数（全部应被模型接管而隐藏）。
const EXPECTED_MESH_COUNT := 4

## M4：车头几何中心实测 0.125（UGV）/ 0.1925（UAV），用 0.02 只判"确实在 +X 半轴"。
const NOSE_MIN_X := 0.02

var _total: int = 0
var _failed: int = 0


func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ 0. 场景探测（树外）
	var packed: PackedScene = load(VEHICLE_SCENE) as PackedScene
	if packed == null:
		_fatal("scene-loads", "%s 不可加载" % VEHICLE_SCENE)
		_finish()
		return
	var probe: Node = packed.instantiate()
	if probe == null:
		_fatal("scene-loads", "%s 实例化失败" % VEHICLE_SCENE)
		_finish()
		return
	_check("scene-loads", packed != null, VEHICLE_SCENE)

	# M1-exports-bound（E16 护栏）：树外 = _ready 的 _ensure_refs 兜底还没机会跑，
	# 此时两个 @export 非 null 且等于同名子节点，才证明 node_paths 真的写了。
	var probe_ugv: Variant = probe.get("model_ugv")
	var probe_uav: Variant = probe.get("model_uav")
	var probe_ugv_node: Node = probe.get_node_or_null("ModelUGV")
	var probe_uav_node: Node = probe.get_node_or_null("ModelUAV")
	_check("M1-exports-bound", probe_ugv != null and probe_ugv == probe_ugv_node
		and probe_uav != null and probe_uav == probe_uav_node,
		"model_ugv=%s model_uav=%s" % [probe_ugv, probe_uav])
	probe.free()

	# ============================================================ 1. 入树
	var vehicle: CharacterBody3D = packed.instantiate() as CharacterBody3D
	if vehicle == null:
		_fatal("M1-model-nodes", "%s 根节点不是 CharacterBody3D" % VEHICLE_SCENE)
		_finish()
		return
	root.add_child(vehicle)
	await process_frame

	var model_ugv: Node3D = vehicle.get_node_or_null("ModelUGV") as Node3D
	var model_uav: Node3D = vehicle.get_node_or_null("ModelUAV") as Node3D
	var body: MeshInstance3D = vehicle.get_node_or_null("Body") as MeshInstance3D
	var shape_node: CollisionShape3D = vehicle.get_node_or_null("CollisionShape3D") as CollisionShape3D
	var chassis_root: Node3D = vehicle.get_node_or_null("Chassis") as Node3D
	if model_ugv == null or model_uav == null or body == null or shape_node == null \
			or chassis_root == null:
		_fatal("M1-model-nodes", "节点缺失 ugv=%s uav=%s body=%s shape=%s chassis=%s"
			% [model_ugv, model_uav, body, shape_node, chassis_root])
		vehicle.queue_free()
		await process_frame
		_finish()
		return

	# M1-model-nodes：静态挂载（scene_file_path 指向 .glb）+ 未缩放未平移（原点 = 底面中心）
	_check("M1-model-nodes", model_ugv.scene_file_path == UGV_MODEL_SCENE
		and model_uav.scene_file_path == UAV_MODEL_SCENE
		and model_ugv.transform.is_equal_approx(Transform3D.IDENTITY)
		and model_uav.transform.is_equal_approx(Transform3D.IDENTITY),
		"ugv=%s uav=%s ugv_xf=%s uav_xf=%s" % [model_ugv.scene_file_path,
			model_uav.scene_file_path, model_ugv.transform, model_uav.transform])

	# M1-default-visibility：空车默认态 = 只有程序化占位车身可见
	_check("M1-default-visibility", not model_ugv.visible and not model_uav.visible
		and body.visible and body.is_visible_in_tree()
		and not model_ugv.is_visible_in_tree() and not model_uav.is_visible_in_tree(),
		"ugv=%s uav=%s body=%s body_in_tree=%s" % [model_ugv.visible, model_uav.visible,
			body.visible, body.is_visible_in_tree()])

	# ============================================================ 2. 装配 diff_drive_ugv（M2/M3/M4/M6）
	var applied_ugv: bool = vehicle.ApplyChassis(UGV_CHASSIS, {}, null)
	await process_frame
	_check("M2-diff-drive-visibility", applied_ugv and model_ugv.visible
		and not model_uav.visible and not body.visible
		and model_ugv.is_visible_in_tree() and not body.is_visible_in_tree()
		and not model_uav.is_visible_in_tree(),
		"applied=%s ugv=%s uav=%s body=%s" % [applied_ugv, model_ugv.visible,
			model_uav.visible, body.visible])

	var chassis_ugv: Node = _first_child(chassis_root)
	var wheels: Node = chassis_ugv.get_node_or_null("Wheels") if chassis_ugv != null else null
	var wheels_stat: Dictionary = _all_meshes_hidden(wheels)
	_check("M2-wheels-hidden", wheels != null
		and int(wheels_stat["count"]) == EXPECTED_MESH_COUNT
		and bool(wheels_stat["hidden"]),
		"wheels=%s count=%d hidden=%s" % [wheels, int(wheels_stat["count"]),
			bool(wheels_stat["hidden"])])

	var ugv_aabb: AABB = _model_aabb(vehicle, model_ugv)
	_check("M3-ugv-aabb", absf(ugv_aabb.size.x - UGV_SIZE.x) <= TOL
		and absf(ugv_aabb.size.y - UGV_SIZE.y) <= TOL
		and absf(ugv_aabb.size.z - UGV_SIZE.z) <= TOL
		and ugv_aabb.position.y >= -TOL and ugv_aabb.position.y <= TOL,
		_fmt(ugv_aabb))

	var ugv_hits: Array[Dictionary] = _nose_hits(vehicle, model_ugv)
	_check("M4-ugv-nose-plus-x", _hits_all_plus_x(ugv_hits), _fmt_hits(ugv_hits))

	var ugv_box: BoxShape3D = shape_node.shape as BoxShape3D
	_check("M6-ugv-hull-box", ugv_box != null and ugv_box.size.is_equal_approx(UGV_HULL),
		"size=%s" % [ugv_box.size if ugv_box != null else shape_node.shape])

	# ============================================================ 3. 幂等（M7）：同一底盘再装一次
	var nodes_before: int = _count_nodes(vehicle)
	var children_before: int = chassis_root.get_child_count()
	var applied_again: bool = vehicle.ApplyChassis(UGV_CHASSIS, {}, null)
	await process_frame
	var nodes_after: int = _count_nodes(vehicle)
	var chassis_again: Node = _first_child(chassis_root)
	var wheels_again: Node = null
	if chassis_again != null:
		wheels_again = chassis_again.get_node_or_null("Wheels")
	var wheels_again_stat: Dictionary = _all_meshes_hidden(wheels_again)
	_check("M7-reapply-idempotent", applied_again
		and nodes_before == nodes_after
		and children_before == 1 and chassis_root.get_child_count() == 1
		and model_ugv.visible and not model_uav.visible and not body.visible
		and int(wheels_again_stat["count"]) == EXPECTED_MESH_COUNT
		and bool(wheels_again_stat["hidden"]),
		"nodes=%d->%d children=%d->%d ugv=%s uav=%s body=%s" % [nodes_before, nodes_after,
			children_before, chassis_root.get_child_count(), model_ugv.visible,
			model_uav.visible, body.visible])

	# ============================================================ 4. 装配 quadrotor_uav（M5/M4/M6）
	var applied_uav: bool = vehicle.ApplyChassis(UAV_CHASSIS, {}, null)
	await process_frame
	_check("M5-uav-visibility", applied_uav and model_uav.visible
		and not model_ugv.visible and not body.visible
		and model_uav.is_visible_in_tree() and not body.is_visible_in_tree()
		and not model_ugv.is_visible_in_tree(),
		"applied=%s ugv=%s uav=%s body=%s" % [applied_uav, model_ugv.visible,
			model_uav.visible, body.visible])

	var chassis_uav: Node = _first_child(chassis_root)
	var rotors: Node = chassis_uav.get_node_or_null("Rotors") if chassis_uav != null else null
	var rotors_stat: Dictionary = _all_meshes_hidden(rotors)
	_check("M5-rotors-hidden", rotors != null
		and int(rotors_stat["count"]) == EXPECTED_MESH_COUNT
		and bool(rotors_stat["hidden"]),
		"rotors=%s count=%d hidden=%s" % [rotors, int(rotors_stat["count"]),
			bool(rotors_stat["hidden"])])

	var uav_aabb: AABB = _model_aabb(vehicle, model_uav)
	_check("M5-uav-aabb", absf(uav_aabb.size.x - UAV_SIZE_XZ) <= TOL
		and uav_aabb.size.y >= UAV_SIZE_Y_MIN and uav_aabb.size.y <= UAV_SIZE_Y_MAX
		and absf(uav_aabb.size.z - UAV_SIZE_XZ) <= TOL
		and absf(uav_aabb.position.y) <= TOL,
		_fmt(uav_aabb))

	var uav_hits: Array[Dictionary] = _nose_hits(vehicle, model_uav)
	_check("M4-uav-nose-plus-x", _hits_all_plus_x(uav_hits), _fmt_hits(uav_hits))

	var uav_box: BoxShape3D = shape_node.shape as BoxShape3D
	_check("M6-uav-hull-box", uav_box != null and uav_box.size.is_equal_approx(UAV_HULL),
		"size=%s" % [uav_box.size if uav_box != null else shape_node.shape])

	# ============================================================ 5. 换底盘循环（M8）：uav -> ugv -> uav
	var cycle_ok: bool = true
	var cycle_detail: String = ""
	var expect_nodes: int = nodes_before
	for chassis_name: String in [UGV_CHASSIS, UAV_CHASSIS]:
		var want_ugv: bool = chassis_name == UGV_CHASSIS
		var switched: bool = vehicle.ApplyChassis(chassis_name, {}, null)
		await process_frame
		var first: Node = _first_child(chassis_root)
		var container: Node = null
		if first != null:
			container = first.get_node_or_null("Wheels" if want_ugv else "Rotors")
		var stat: Dictionary = _all_meshes_hidden(container)
		var step_ok: bool = (switched and model_ugv.visible == want_ugv
			and model_uav.visible == (not want_ugv)
			and model_ugv.is_visible_in_tree() == want_ugv
			and model_uav.is_visible_in_tree() == (not want_ugv)
			and not body.visible and not body.is_visible_in_tree()
			and chassis_root.get_child_count() == 1
			and _count_nodes(vehicle) == expect_nodes
			and int(stat["count"]) == EXPECTED_MESH_COUNT
			and bool(stat["hidden"]))
		if not step_ok:
			cycle_ok = false
		cycle_detail += "%s:%s " % [chassis_name, step_ok]
	_check("M8-switch-cycle", cycle_ok, "%s(ugv=%s uav=%s body=%s nodes=%d)" % [
		cycle_detail, model_ugv.visible, model_uav.visible, body.visible,
		_count_nodes(vehicle)])

	# ============================================================ 6. 清理（P12：退出时无 ERROR）
	vehicle.queue_free()
	await process_frame
	await process_frame
	_finish()


# ================================================================ 几何（全部无 await）

## 收集子树里所有 MeshInstance3D（含自身；跳过 mesh == null 的节点）。
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


## mesh 的 8 个局部角点过 space * global_transform 后重新求包围盒。
## （get_aabb() 不含节点自身的旋转/缩放，直接变换 position/size 会得到偏大的盒子。）
func _aabb_of_mesh_in(space: Transform3D, mi: MeshInstance3D) -> AABB:
	var local: AABB = mi.get_aabb()
	return _aabb_of_local_in(space * mi.global_transform, local)


func _aabb_of_local_in(xf: Transform3D, local: AABB) -> AABB:
	var out: AABB = AABB()
	var first: bool = true
	for i in range(8):
		var corner: Vector3 = xf * local.get_endpoint(i)
		if first:
			out = AABB(corner, Vector3.ZERO)   # 首个角点定型，避免 AABB() 原点污染
			first = false
		else:
			out = out.expand(corner)
	return out


## 模型在 Vehicle 局部坐标系下的整体 AABB；没有任何 mesh = 零点零尺寸（断言据此 FAIL）。
func _model_aabb(vehicle: Node3D, model: Node3D) -> AABB:
	var out: AABB = AABB()
	if vehicle == null or model == null or not model.is_inside_tree():
		return out
	var space: Transform3D = vehicle.global_transform.affine_inverse()
	var first: bool = true
	for mi: MeshInstance3D in _collect_meshes(model):
		var box: AABB = _aabb_of_mesh_in(space, mi)
		if first:
			out = box
			first = false
		else:
			out = out.merge(box)
	return out


## 车头判定（双通道，因为 glb 里没有 Nose 节点，车头是 Body 的一个 surface）。
## 返回 [{source, center}]；通道 1 = 节点名含 nose，通道 2 = surface 材质名含 nose。
func _nose_hits(vehicle: Node3D, model: Node3D) -> Array[Dictionary]:
	var hits: Array[Dictionary] = []
	if vehicle == null or model == null or not model.is_inside_tree():
		return hits
	var space: Transform3D = vehicle.global_transform.affine_inverse()
	for mi: MeshInstance3D in _collect_meshes(model):
		var xf: Transform3D = space * mi.global_transform
		if String(mi.name).to_lower().contains("nose"):
			var whole: AABB = _aabb_of_local_in(xf, mi.get_aabb())
			hits.append({"source": "by-name:" + String(mi.name), "center": whole.get_center()})
		var mesh: Mesh = mi.mesh
		if mesh == null:
			continue
		for surface in range(mesh.get_surface_count()):
			var mat: Material = mi.get_active_material(surface)
			if mat == null or not mat.resource_name.to_lower().contains("nose"):
				continue
			var part: AABB = _surface_aabb_in(xf, mesh, surface)
			if part.size.is_zero_approx():
				continue                          # 该 surface 没有顶点数组：跳过不崩
			hits.append({"source": "by-mat:%s surf=%d mat=%s"
				% [mi.name, surface, mat.resource_name], "center": part.get_center()})
	return hits


## 某个 surface 的顶点包围盒，按 xf 变换后取包围盒（角点法）。
func _surface_aabb_in(xf: Transform3D, mesh: Mesh, surface: int) -> AABB:
	var out: AABB = AABB()
	if not (mesh is ArrayMesh):
		return out
	var arrays: Array = (mesh as ArrayMesh).surface_get_arrays(surface)
	if arrays.size() <= Mesh.ARRAY_VERTEX:
		return out
	if not (arrays[Mesh.ARRAY_VERTEX] is PackedVector3Array):
		return out
	var verts: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	if verts.is_empty():
		return out
	var lo: Vector3 = verts[0]
	var hi: Vector3 = verts[0]
	for v: Vector3 in verts:
		lo = lo.min(v)
		hi = hi.max(v)
	return _aabb_of_local_in(xf, AABB(lo, hi - lo))


## 所有命中点都在 +X 半轴且至少命中一次（否则"找不到 Nose"会退化成恒真）。
func _hits_all_plus_x(hits: Array[Dictionary]) -> bool:
	if hits.is_empty():
		return false
	for hit: Dictionary in hits:
		var center: Vector3 = hit["center"]
		if center.x <= NOSE_MIN_X:
			return false
	return true


func _fmt_hits(hits: Array[Dictionary]) -> String:
	if hits.is_empty():
		return "hits=[]"
	var parts: PackedStringArray = PackedStringArray()
	for hit: Dictionary in hits:
		var center: Vector3 = hit["center"]
		parts.append("{%s center=(%.3f, %.3f, %.3f)}"
			% [hit["source"], center.x, center.y, center.z])
	return "hits=[" + ", ".join(parts) + "]"


func _fmt(a: AABB) -> String:
	return "P(%.2f, %.2f, %.2f) S(%.2f, %.2f, %.2f)" % [a.position.x, a.position.y,
		a.position.z, a.size.x, a.size.y, a.size.z]


# ================================================================ 树遍历（无 await）

## 节点总数（含自身）。
func _count_nodes(node: Node) -> int:
	if node == null:
		return 0
	var n: int = 1
	for child: Node in node.get_children():
		n += _count_nodes(child)
	return n


func _first_child(parent: Node) -> Node:
	if parent == null or parent.get_child_count() == 0:
		return null
	return parent.get_child(0)


## {count, hidden}：hidden 要求 count > 0（"0 个也算全隐藏"会让断言写废）。
func _all_meshes_hidden(node: Node) -> Dictionary:
	var meshes: Array[MeshInstance3D] = _collect_meshes(node)
	var hidden: bool = meshes.size() > 0
	for mi: MeshInstance3D in meshes:
		if mi.visible:
			hidden = false
	return {"count": meshes.size(), "hidden": hidden}


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
