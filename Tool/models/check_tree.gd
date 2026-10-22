extends Node3D
## 资产验收（CheckTree）：实例化两个 .glb，逐个 MeshInstance3D 打印三角面与世界 AABB，
## 断言「尺寸 / 原点 / 朝向」与仿真参数一致，全部通过 quit(0)，否则 quit(1)。
##
## 只在 /tmp 最小工程里跑：verify_assets.py 会把本文件 + check_tree.tscn 复制到临时工程，
## 再把两个 .glb 拷到同目录（res:// 根），因此这里用固定文件名而不是绝对路径。
##
## 必须用「主场景 + _ready()」而不是 `--script` + `extends SceneTree` 的 `_initialize()`：
## 后者节点还没进树，global_transform 会报 !is_inside_tree() 并退化成单位阵，
## 轮/旋翼的平移会丢（UAV 尺寸会从 0.64 掉到 0.42）。
##
## 期望值出处（task_32 重构后的当前行号，1 BU = 1 m，Godot Y-up，前进 = 局部 +X）：
##   UGV 轮半径 0.06   src/chassis/DiffDrive.gd:33  (WHEEL_RADIUS)
##       wheel_base 0.30 / track_width 0.24 / height 0.16   DiffDrive.gd:41-43
##       轮心 x=±0.15、z=∓0.12、y=0.06                     DiffDrive.gd:159-161
##   UAV size 0.40 (X=Z)         src/chassis/Quadrotor.gd:68
##       BODY_HEIGHT 0.12 / ROTOR_LIFT 0.01                 Quadrotor.gd:50,52
##       旋翼 半径 0.12 / 厚 0.02 / 16 段                   src/scenes/chassis/Quadrotor.tscn:17-20
##       旋翼中心 (±0.20, 0.13, ∓0.20)                      Quadrotor.gd:248-250
## 由此推出（见 tmp/models/plan.md §3）：
##   UGV 车身包络 = (0.30, 0.16, 0.24) = (wheel_base, height, track_width)
##   UGV 整机 AABB = (0.42, 0.16, 0.28) = (wheel_base + 2*WHEEL_RADIUS, height, track_width + 轮宽)
##   UAV 整机 AABB = (0.64, 0.14, 0.64) = (2*(size/2 + 旋翼半径), BODY_HEIGHT+ROTOR_LIFT+厚/2, 同 X)

const UGV_PATH := "res://ugv_x3plus_lowpoly.glb"
const UAV_PATH := "res://uav_quadrotor_lowpoly.glb"

## 车型：对象名集合、对象数、三角面、车身包络、整机包络、容差、机头位置下限
const UGV_OBJECTS := ["Body", "WheelFL", "WheelFR", "WheelRL", "WheelRR"]
const UAV_OBJECTS := ["Body", "RotorFL", "RotorFR", "RotorRL", "RotorRR", "ArmFL", "ArmFR", "ArmRL", "ArmRR"]

var _fail_count := 0

## tag -> 关键数字，供 verify_assets.py 二次核对
var _summary := {}


func _ready() -> void:
	_check_ugv()
	_check_uav()
	if _fail_count == 0:
		print("[RESULT] PASS")
		get_tree().quit(0)
	else:
		print("[RESULT] FAIL %d" % _fail_count)
		get_tree().quit(1)


# ── UGV ──────────────────────────────────────────────────────────────────────


func _check_ugv() -> void:
	var tag := "ugv"
	var label := UGV_PATH.get_file()
	var root := _instantiate(UGV_PATH, tag)
	if root == null:
		return
	var stats := _measure(root, label)

	print("[ASSET] %s objects=%d tris=%d min=(%.6f,%.6f,%.6f) max=(%.6f,%.6f,%.6f) size=(%.6f,%.6f,%.6f)" % [
		label, stats.objects.size(), stats.tris,
		stats.min.x, stats.min.y, stats.min.z, stats.max.x, stats.max.y, stats.max.z,
		stats.size.x, stats.size.y, stats.size.z,
	])

	_print_detail(stats)

	# A1 / A2 对象数与名字集合
	_assert_int(tag, "objects", UGV_OBJECTS.size(), stats.objects.size())
	var names := _sorted_names(stats.objects)
	_assert_names(tag, names, UGV_OBJECTS)

	# A3 三角面
	_assert_int(tag, "tris", 260, stats.tris)

	# A4 车身包络（step.md 字面说的 "X=0.30" 指这个：Body 单对象 = 碰撞盒口径）
	var body: Dictionary = stats.by_name.get("Body", {})
	_assert_vec3(tag, "body_size", Vector3(0.30, 0.16, 0.24), body.get("size", Vector3.ZERO), 0.005)

	# A5 整机 AABB（含轮子的真实视觉包络）
	_assert_vec3(tag, "full_size", Vector3(0.42, 0.16, 0.28), stats.size, 0.01)

	# A6 原点 = 底面中心 → min.y ≈ 0
	_assert_range(tag, "min_y", stats.min.y, -0.01, 0.01)

	# A10/A11/A12 机头红面朝 +X
	_assert_nose(tag, stats, stats.size.x)

	print("[BODY] %s size=(%.6f,%.6f,%.6f)" % [
		label, body.get("size", Vector3.ZERO).x,
		body.get("size", Vector3.ZERO).y, body.get("size", Vector3.ZERO).z,
	])
	print("[SPEC] ugv full X: step.md=0.30 body-only=0.30 -> full=0.42 (= wheel_base 0.30 + 2*WHEEL_RADIUS 0.06, DiffDrive.gd:41,33,159)")

	_summary[tag] = {
		"objects": stats.objects.size(),
		"tris": stats.tris,
		"size": stats.size,
		"nose_x": stats.nose_center_x,
		"body_size": body.get("size", Vector3.ZERO),
	}


# ── UAV ──────────────────────────────────────────────────────────────────────


func _check_uav() -> void:
	var tag := "uav"
	var label := UAV_PATH.get_file()
	var root := _instantiate(UAV_PATH, tag)
	if root == null:
		return
	var stats := _measure(root, label)

	print("[ASSET] %s objects=%d tris=%d min=(%.6f,%.6f,%.6f) max=(%.6f,%.6f,%.6f) size=(%.6f,%.6f,%.6f)" % [
		label, stats.objects.size(), stats.tris,
		stats.min.x, stats.min.y, stats.min.z, stats.max.x, stats.max.y, stats.max.z,
		stats.size.x, stats.size.y, stats.size.z,
	])

	_print_detail(stats)

	_assert_int(tag, "objects", UAV_OBJECTS.size(), stats.objects.size())
	var names := _sorted_names(stats.objects)
	_assert_names(tag, names, UAV_OBJECTS)
	_assert_int(tag, "tris", 308, stats.tris)

	# A7 整机 AABB：X=Z=2*(size/2 + 旋翼半径) = 0.64，Y = 0.12+0.01+0.01 = 0.14
	_assert_vec3(tag, "full_size", Vector3(0.64, 0.14, 0.64), stats.size, 0.01)

	# A8 机高落在 [0.11, 0.15]
	_assert_range(tag, "height", stats.size.y, 0.11, 0.15)

	# A9 原点 = 底面 → min.y ≈ 0
	_assert_range(tag, "min_y", stats.min.y, -0.01, 0.01)

	# A10/A11/A12 机头红面朝 +X
	_assert_nose(tag, stats, stats.size.x)

	_summary[tag] = {
		"objects": stats.objects.size(),
		"tris": stats.tris,
		"size": stats.size,
		"nose_x": stats.nose_center_x,
	}


# ── 度量 ─────────────────────────────────────────────────────────────────────


## 递归收 MeshInstance3D（不含根节点以外层级遗漏；glTF 导入结果通常是扁平/两层）
func _collect(node: Node, out: Array) -> void:
	if node is MeshInstance3D:
		out.append(node)
	for child in node.get_children():
		_collect(child, out)


func _instantiate(path: String, tag: String) -> Node3D:
	var packed := load(path) as PackedScene
	if packed == null:
		_fail_count += 1
		print("[FAIL] %s load expect=PackedScene(%s) got=null diff=无法加载" % [tag, path])
		return null
	var root := packed.instantiate() as Node3D
	if root == null:
		_fail_count += 1
		print("[FAIL] %s instantiate expect=Node3D got=null diff=根节点不是 Node3D" % tag)
		return null
	add_child(root)
	return root


## 世界 AABB / 三角面：按 surface 累加（Body 是 2 个 surface，按对象算会少数）
func _measure(root: Node3D, label: String) -> Dictionary:
	var meshes: Array = []
	_collect(root, meshes)

	var mn := Vector3(INF, INF, INF)
	var mx := Vector3(-INF, -INF, -INF)
	var tris_total := 0
	var by_name := {}
	var nose_min := Vector3(INF, INF, INF)
	var nose_max := Vector3(-INF, -INF, -INF)
	var nose_surfaces := 0
	var obj_lines: Array = []

	for node in meshes:
		var mi := node as MeshInstance3D
		var obj_min := Vector3(INF, INF, INF)
		var obj_max := Vector3(-INF, -INF, -INF)
		var obj_tris := 0
		var mats: Array = []

		if mi.mesh != null:
			for s in range(mi.mesh.get_surface_count()):
				var arrays: Array = mi.mesh.surface_get_arrays(s)
				var verts_raw = arrays[Mesh.ARRAY_VERTEX]
				var idx_raw = arrays[Mesh.ARRAY_INDEX]
				if verts_raw == null:
					continue
				var verts: PackedVector3Array = verts_raw
				var idx := PackedInt32Array()
				if idx_raw != null:
					idx = idx_raw
				var surface_tris := 0
				if idx.size() > 0:
					surface_tris = int(idx.size() / 3)
					_assert_index_multiple(mi.name, s, idx.size())
				else:
					surface_tris = int(verts.size() / 3)
				obj_tris += surface_tris

				var mat := mi.mesh.surface_get_material(s)
				var mat_name := ""
				if mat != null:
					mat_name = String(mat.resource_name)
				mats.append(mat_name)

				var is_nose := _is_nose_material(mat, mat_name)
				if is_nose:
					nose_surfaces += 1

				for v in verts:
					var w: Vector3 = mi.global_transform * v
					obj_min = _vmin(obj_min, w)
					obj_max = _vmax(obj_max, w)
					mn = _vmin(mn, w)
					mx = _vmax(mx, w)
					if is_nose:
						nose_min = _vmin(nose_min, w)
						nose_max = _vmax(nose_max, w)

		if obj_min.x == INF:
			obj_min = Vector3.ZERO
			obj_max = Vector3.ZERO
		var obj_size := obj_max - obj_min
		obj_lines.append("[OBJ]   %s %-10s tris=%-4d world=(%.6f,%.6f,%.6f)..(%.6f,%.6f,%.6f) mats=[%s]" % [
			label, mi.name, obj_tris,
			obj_min.x, obj_min.y, obj_min.z, obj_max.x, obj_max.y, obj_max.z,
			", ".join(PackedStringArray(mats)),
		])
		by_name[String(mi.name)] = {
			"tris": obj_tris,
			"min": obj_min,
			"max": obj_max,
			"size": obj_size,
		}
		tris_total += obj_tris

	if mn.x == INF:
		mn = Vector3.ZERO
		mx = Vector3.ZERO

	if nose_surfaces > 0:
		var nose_center := (nose_min + nose_max) * 0.5
		obj_lines.append("[NOSE]  %s surfaces=%d center_x=%.6f (x in [%.6f, %.6f])" % [
			label, nose_surfaces, nose_center.x, nose_min.x, nose_max.x,
		])
	elif meshes.size() > 0:
		obj_lines.append("[NOSE]  %s surfaces=0 center_x=n/a (未识别到红色机头材质)" % label)

	return {
		"objects": meshes,
		"tris": tris_total,
		"min": mn,
		"max": mx,
		"size": mx - mn,
		"by_name": by_name,
		"nose_surfaces": nose_surfaces,
		"nose_center_x": ((nose_min + nose_max) * 0.5).x,
		"obj_lines": obj_lines,
	}


## Vector3.min()/max() 在 Godot 4.7 里是实例方法（不是静态），这里用标量 minf/maxf 自己拼
func _vmin(a: Vector3, b: Vector3) -> Vector3:
	return Vector3(minf(a.x, b.x), minf(a.y, b.y), minf(a.z, b.z))


func _vmax(a: Vector3, b: Vector3) -> Vector3:
	return Vector3(maxf(a.x, b.x), maxf(a.y, b.y), maxf(a.z, b.z))


func _is_nose_material(mat: Material, mat_name: String) -> bool:
	if mat == null:
		return false
	if mat_name.contains("NoseRed"):
		return true
	if mat is StandardMaterial3D:
		var c := (mat as StandardMaterial3D).albedo_color
		if c.r > 0.6 and c.g < 0.35 and c.b < 0.35:
			return true
	return false


func _print_detail(stats: Dictionary) -> void:
	for line in stats.obj_lines:
		print(line)


func _sorted_names(meshes: Array) -> Array:
	var names: Array = []
	for node in meshes:
		names.append(String(node.name))
	names.sort()
	return names


# ── 断言小工具 ────────────────────────────────────────────────────────────────


func _ok(tag: String, name: String, detail: String) -> void:
	print("[ASSERT] %-3s %-11s PASS  %s" % [tag, name, detail])


func _bad(tag: String, name: String, detail: String) -> void:
	_fail_count += 1
	print("[FAIL] %s %s %s" % [tag, name, detail])


func _assert_int(tag: String, name: String, expect: int, got: int) -> void:
	if expect == got:
		_ok(tag, name, "expect=%d got=%d" % [expect, got])
	else:
		_bad(tag, name, "expect=%d got=%d diff=%+d" % [expect, got, got - expect])


func _assert_vec3(tag: String, name: String, expect: Vector3, got: Vector3, tol: float) -> void:
	var diff := got - expect
	var bad := absf(diff.x) > tol or absf(diff.y) > tol or absf(diff.z) > tol
	var detail := "expect=(%.6f,%.6f,%.6f) got=(%.6f,%.6f,%.6f) tol=%.4f diff=(%+.6f,%+.6f,%+.6f)" % [
		expect.x, expect.y, expect.z, got.x, got.y, got.z, tol, diff.x, diff.y, diff.z,
	]
	if bad:
		_bad(tag, name, detail)
	else:
		_ok(tag, name, detail)


func _assert_range(tag: String, name: String, got: float, lo: float, hi: float) -> void:
	var detail := "expect=[%+.6f,%+.6f] got=%.6f diff=%+.6f" % [lo, hi, got, maxf(lo - got, got - hi)]
	if got < lo or got > hi:
		_bad(tag, name, detail)
	else:
		_ok(tag, name, detail)


func _assert_names(tag: String, got: Array, expect: Array) -> void:
	var expected := expect.duplicate()
	expected.sort()
	var missing: Array = []
	for n in expected:
		if not got.has(n):
			missing.append(n)
	var extra: Array = []
	for n in got:
		if not expected.has(n):
			extra.append(n)
	var detail := "expect=%s got=%s" % [str(expected), str(got)]
	if missing.is_empty() and extra.is_empty() and got.size() == expected.size():
		_ok(tag, "names", detail)
	else:
		_bad(tag, "names", detail + " missing=%s extra=%s" % [str(missing), str(extra)])


## A12：红面必须存在；A10：中心 x > 0；A11：中心 x > 0.25 * 整机长（防「整机镜像」）
func _assert_nose(tag: String, stats: Dictionary, full_x: float) -> void:
	if stats.nose_surfaces < 1:
		_bad(tag, "nose_surfaces", "expect>=1 got=%d diff=无红面，朝向无法判定" % stats.nose_surfaces)
		return
	_ok(tag, "nose_surfaces", "expect>=1 got=%d" % stats.nose_surfaces)

	var x := float(stats.nose_center_x)
	var floor_x := 0.25 * full_x
	if x > 0.0:
		_ok(tag, "nose_x", "expect>0 got=%.6f" % x)
	else:
		_bad(tag, "nose_x", "expect>0 got=%.6f diff=车头朝 -X（模型镜像）" % x)

	if x > floor_x:
		_ok(tag, "nose_x_frac", "expect>%.6f (=0.25*%.6f) got=%.6f" % [floor_x, full_x, x])
	else:
		_bad(tag, "nose_x_frac", "expect>%.6f got=%.6f diff=机头太靠中间或整机被镜像" % [floor_x, x])


func _assert_index_multiple(obj: String, surface: int, count: int) -> void:
	if count % 3 != 0:
		_bad("mesh", "index_triplets", "%s surface=%d indices=%d diff=不是 3 的倍数" % [obj, surface, count])
