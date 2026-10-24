# Presented by KeJi
# Created Date ： 2026-10-22
# Modified Date ： 2026-10-23
#
# 低模外观自检（Step 2 + task_33 Step 3）：模型静态挂在 Vehicle.tscn / 两个传感器场景上，
# 显隐正确 / 尺寸朝向正确 / 模型 = 碰撞盒 / 传感器模型零碰撞。
# 用法：timeout 180 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_models.gd
# 期望：total=24 failed=0 + SELF-CHECK OK + EXIT=0，且 stderr 无 ERROR。
#
# 决定本文件写法的实测事实（plan.md §0 / §3）：
#   - 旧 UGV 低模（ugv_x3plus_lowpoly.glb）里**没有**名字含 Nose 的节点：红色车头是 Body 这个
#     MeshInstance3D 的第 2 个 surface（材质名 UGV_NoseRed / UAV_NoseRed），所以 UAV 的朝向断言
#     仍走"节点名 + 材质名"双通道。task_33 Step 3 起 ModelUGV 改用 assets/chassis/ugv_chassis.glb，
#     它的车头是**独立节点 NosePlate**（新 glb 没有 NoseRed 材质通道）→ UGV 命中走 by-name。
#   - UGV 模型在 Vehicle 坐标系的整体 AABB = P(-0.15, 0.00, -0.13) S(0.30, 0.25, 0.25)（= 碰撞盒）；
#     UAV = P(-0.32, 0.00, -0.32) S(0.64, 0.14, 0.64)。
#   - 传感器模型（task_33 Step 3，纯视觉、无碰撞）：挂在 Lidar2D/CameraSensor 场景的 Body 上，
#     Body 是 glb 实例根（Node3D），子弹节点才是 MeshInstance3D（E27 见 COMPONENTS.md §9.1）。
#     雷达 AABB（holder 系）= P(-0.04, 0.00, -0.04) S(0.08, 0.054, 0.08)，扫描面 = 原点；
#     相机 AABB（Body 系）= P(-0.055, -0.021, -0.015) S(0.055, 0.036, 0.030)，镜片前表面 = 光心。
#   - 节点总数（实测）：空车 25、装一个底盘 31（ugv_chassis 模型子树 9 个节点；
#     模型节点是静态实例，装配只改 visible，所以 M7/M8 的 _count_nodes 基线恒定）。
#   - mi.get_aabb() 是 **mesh 局部 AABB**（不含节点自身旋转/缩放）→ 必须 8 角点过变换。
#   - 树外读 global_transform 会 ERROR（P8）→ AABB / 朝向断言只能在 add_child 之后。
#   - _ensure_refs() 会在 _ready 里按名补齐引用 → E16 真绑定只能用**树外探针**证明。
#
# 注意（GDScript 4.7 类型推断陷阱）：load() / instantiate() / get() 的返回值一律显式标注类型。
# 注意（P15）：含 await 的辅助函数必须被 await 调用 → 本文件所有辅助函数都不含 await，
#   await 只内联在 _run 里。

extends SceneTree

const VEHICLE_SCENE := "res://src/scenes/Vehicle.tscn"
const UGV_MODEL_SCENE := "res://assets/chassis/ugv_chassis.glb"
const UAV_MODEL_SCENE := "res://assets/uav_quadrotor_lowpoly.glb"
const LIDAR_SCENE := "res://src/scenes/sensors/Lidar2D.tscn"
const CAMERA_SCENE := "res://src/scenes/sensors/CameraSensor.tscn"
const LIDAR_GLB := "res://assets/sensors/lidar_tmini.glb"
const CAMERA_GLB := "res://assets/sensors/camera_module.glb"
const UGV_CHASSIS := "diff_drive_ugv"
const UAV_CHASSIS := "quadrotor_uav"

## M3/M5：模型整体 AABB 期望尺寸（Vehicle 坐标系，米）+ 容差。
const UGV_SIZE := Vector3(0.30, 0.25, 0.25)
const UAV_SIZE_XZ := 0.64
const UAV_SIZE_Y_MIN := 0.11
const UAV_SIZE_Y_MAX := 0.15
const TOL := 0.01

## M6：碰撞形状尺寸必须仍是底盘参数（模型只是外观层，一行物理都不许跟着变）。
## task_33 Step 3 起底盘几何默认值 = 模型 AABB（D11：模型 = 碰撞盒）。
const UGV_HULL := Vector3(0.30, 0.25, 0.25)
const UAV_HULL := Vector3(0.40, 0.12, 0.40)

## S1~S7：传感器模型（task_33 Step 3）。模型根是 glb 实例（Node3D），子节点才是 MeshInstance3D。
const CAMERA_MODEL_YAW_DEG := 90.0      # 模型 +X（镜头）→ 传感器 −Z（光轴）的补偿角
const SENSOR_TOL := 1.0e-3              # 传感器模型 AABB / 原点的绝对容差（米）
const LIDAR_CAP := Vector3(0.09, 0.06, 0.09)   # step.md 尺寸上限（≤）
const CAMERA_CAP := Vector3(0.06, 0.04, 0.04)  # step.md 尺寸上限（≤）
const CAMERA_LENS_X := 0.0              # 镜片前表面 = 光心（max.x ≈ 0）
const CAMERA_BRACKET_Y := -0.021        # 支架底面 y（机身下方）
const SENSOR_MESH_COUNT := 4            # 两件传感器模型都是 4 个 mesh（一对象一材质）

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

	# M3b 模型 = 碰撞盒（D11，task_33 Step 3）：车体模型 AABB 与 BoxShape3D 的 AABB 逐轴相等。
	#   两者都在 Vehicle 局部系；BoxShape3D 没有 position，AABB 原点 = CollisionShape3D.position - size/2。
	var hull_aabb: AABB = AABB()
	if ugv_box != null:
		hull_aabb = AABB(shape_node.position - ugv_box.size * 0.5, ugv_box.size)
	_check("M3b-ugv-model-equals-hull", ugv_box != null
		and _aabb_equal(ugv_aabb, hull_aabb, SENSOR_TOL),
		"model=%s hull=%s" % [_fmt(ugv_aabb), _fmt(hull_aabb)])

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

	# ============================================================ 6. 传感器模型接入（S1~S7）
	# 三件 glb 都已在 Step 1 的 check_sensor_models 锁过资产层；这里锁"场景里怎么接的"。
	var lidar_packed: PackedScene = load(LIDAR_SCENE) as PackedScene
	var camera_packed: PackedScene = load(CAMERA_SCENE) as PackedScene
	if lidar_packed == null or camera_packed == null:
		_fatal("S0-sensor-scenes", "lidar=%s camera=%s" % [lidar_packed, camera_packed])
		vehicle.queue_free()
		await process_frame
		_finish()
		return
	var lidar_probe: Node3D = lidar_packed.instantiate() as Node3D
	var camera_probe: Node3D = camera_packed.instantiate() as Node3D
	if lidar_probe != null:
		root.add_child(lidar_probe)
	if camera_probe != null:
		root.add_child(camera_probe)
	await process_frame

	# S1 雷达场景：Body = glb 实例、transform 恒 identity、DebugPoints 仍在且默认隐藏
	var lidar_body: Node3D = null
	var lidar_debug: MeshInstance3D = null
	if lidar_probe != null:
		lidar_body = lidar_probe.get_node_or_null("Body") as Node3D
		lidar_debug = lidar_probe.get_node_or_null("DebugPoints") as MeshInstance3D
	_check("S1-lidar-body-mount", lidar_body != null
		and lidar_body.scene_file_path == LIDAR_GLB
		and lidar_body.transform.is_equal_approx(Transform3D.IDENTITY)
		and lidar_debug != null and not lidar_debug.visible,
		"body=%s path=%s xf=%s debug=%s" % [lidar_body,
			lidar_body.scene_file_path if lidar_body != null else "null",
			lidar_body.transform if lidar_body != null else "null", lidar_debug != null])

	# S2 雷达模型 AABB（holder 局部系）：尺寸 ≤ 上限、扫描面 = 原点（min.y≈0、center.xz≈0）
	var lidar_aabb: AABB = _subtree_aabb(_space_of(lidar_probe), lidar_body)
	var lidar_center: Vector3 = lidar_aabb.get_center()
	var lidar_meshes: int = _count_meshes(lidar_body)
	_check("S2-lidar-model-aabb", lidar_meshes == SENSOR_MESH_COUNT
		and _aabb_size_le(lidar_aabb, LIDAR_CAP, SENSOR_TOL)
		and absf(lidar_aabb.position.y) <= SENSOR_TOL
		and absf(lidar_center.x) <= SENSOR_TOL and absf(lidar_center.z) <= SENSOR_TOL,
		"%s meshes=%d" % [_fmt(lidar_aabb), lidar_meshes])

	# S3 相机场景：Body = glb 实例、不写 position（光心 = 原点）、绕 Y +90° 把镜头 +X 转到光轴 −Z
	var camera_body: Node3D = null
	if camera_probe != null:
		camera_body = camera_probe.get_node_or_null("Body") as Node3D
	var cam_rot: Vector3 = camera_body.rotation_degrees if camera_body != null else Vector3.ZERO
	_check("S3-camera-body-mount", camera_body != null
		and camera_body.scene_file_path == CAMERA_GLB
		and camera_body.position.is_equal_approx(Vector3.ZERO)
		and absf(cam_rot.x) < 1e-4 and absf(cam_rot.z) < 1e-4
		and absf(cam_rot.y - CAMERA_MODEL_YAW_DEG) < 1e-4,
		"rot=(%.2f, %.2f, %.2f)" % [cam_rot.x, cam_rot.y, cam_rot.z])

	# S4 相机模型 AABB（Body 自身局部系 = 把 90° 旋掉）：尺寸 ≤ 上限、镜片前表面 = 光心（max.x≈0）、
	#    支架在机身下方（min.y≈-0.021）；且模型 +X 与光轴（-holder.basis.z）同向。
	var cam_aabb: AABB = _subtree_aabb(_space_of(camera_body), camera_body)
	var cam_meshes: int = _count_meshes(camera_body)
	var cam_axis_dot: float = 0.0
	if camera_body != null and camera_probe != null:
		cam_axis_dot = camera_body.global_basis.x.normalized().dot(
			(-camera_probe.global_basis.z).normalized())
	_check("S4-camera-optic-origin", cam_meshes == SENSOR_MESH_COUNT
		and _aabb_size_le(cam_aabb, CAMERA_CAP, SENSOR_TOL)
		and absf(cam_aabb.end.x - CAMERA_LENS_X) <= SENSOR_TOL
		and absf(cam_aabb.position.y - CAMERA_BRACKET_Y) <= SENSOR_TOL
		and cam_axis_dot > 0.9999,
		"%s meshes=%d dot=%.4f" % [_fmt(cam_aabb), cam_meshes, cam_axis_dot])

	# S5~S7 真挂到车上（雷达在车顶、相机在车鼻；pitch 只有相机给 -30）
	var lidar_attached: Node3D = vehicle.AttachSensor("lidar_top", lidar_packed,
		{"x": -0.05, "z": 0.25}) as Node3D
	var camera_attached: Node3D = vehicle.AttachSensor("camera_front", camera_packed,
		{"x": 0.15, "z": 0.12, "pitch": -30.0}) as Node3D
	await process_frame

	# S5 传感器子树里一个物理节点都不许有（E27：否则雷达扫到自己、相机被自己的壳挡）
	var lidar_physics: int = _count_physics(lidar_attached)
	var camera_physics: int = _count_physics(camera_attached)
	_check("S5-sensor-no-collision", lidar_attached != null and camera_attached != null
		and lidar_physics == 0 and camera_physics == 0,
		"lidar=%d camera=%d" % [lidar_physics, camera_physics])

	# S6 pitch=-30 ⇒ 低头：rotation.x≈-30、rotation.y=0（yaw 未给）、光轴 y≈-0.5，模型仍朝光轴
	var cam_axis: Vector3 = -camera_attached.global_basis.z if camera_attached != null else Vector3.ZERO
	cam_axis = cam_axis.normalized()
	var cam_model_dir: Vector3 = Vector3.ZERO
	if camera_attached != null:
		var cam_child: Node3D = camera_attached.get_node_or_null("Body") as Node3D
		if cam_child != null:
			cam_model_dir = cam_child.global_basis.x.normalized()
	var cam_rot_x: float = camera_attached.rotation_degrees.x if camera_attached != null else 0.0
	var cam_rot_y: float = camera_attached.rotation.y if camera_attached != null else 1.0
	_check("S6-sensor-pitch-applied", camera_attached != null
		and absf(cam_rot_x + 30.0) < 1e-3 and absf(cam_rot_y) < 1e-6
		and absf(cam_axis.y + 0.5) < 1e-3 and cam_model_dir.dot(cam_axis) > 0.9999,
		"rot_x=%.2f axis=(%.3f, %.3f, %.3f)" % [cam_rot_x, cam_axis.x, cam_axis.y, cam_axis.z])

	# S7 雷达 mount 不带 pitch ⇒ 不预置任何角度（rotation.x 精确 0），扫描面保持水平
	var lidar_plane_y: float = 0.0
	var lidar_rot_x: float = 0.0
	if lidar_attached != null:
		lidar_rot_x = lidar_attached.rotation.x
		lidar_plane_y = (lidar_attached.global_basis * Vector3(0.0, 0.0, 1.0)).y
	_check("S7-lidar-pitch-default", lidar_attached != null and lidar_rot_x == 0.0
		and absf(lidar_plane_y) < 1e-9,
		"rot_x=%.4f plane_y=%.6f" % [lidar_rot_x, lidar_plane_y])

	# 传感器探针出树（同帧释放：先 remove 再 free，避免 "already has parent" 类告警）
	if lidar_probe != null:
		root.remove_child(lidar_probe)
		lidar_probe.queue_free()
	if camera_probe != null:
		root.remove_child(camera_probe)
		camera_probe.queue_free()
	await process_frame

	# ============================================================ 7. 清理（P12：退出时无 ERROR）
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


## 某节点自身局部系（= 世界变换的逆）；null → IDENTITY（断言会红，不崩）。
func _space_of(node: Node3D) -> Transform3D:
	if node == null:
		return Transform3D.IDENTITY
	return node.global_transform.affine_inverse()


## 子树（含自身）在 space 坐标系下的整体 AABB；无 mesh = 零点零尺寸（断言据此 FAIL）。
func _subtree_aabb(space: Transform3D, root_node: Node) -> AABB:
	var out: AABB = AABB()
	if root_node == null:
		return out
	var first: bool = true
	for mi: MeshInstance3D in _collect_meshes(root_node):
		var box: AABB = _aabb_of_mesh_in(space, mi)
		if first:
			out = box
			first = false
		else:
			out = out.merge(box)
	return out


## AABB 逐轴相等（位置 + 尺寸，绝对容差）—— 断言"模型 = 碰撞盒"用。
func _aabb_equal(a: AABB, b: AABB, tol: float) -> bool:
	for axis in range(3):
		if absf(a.position[axis] - b.position[axis]) > tol:
			return false
		if absf(a.size[axis] - b.size[axis]) > tol:
			return false
	return true


## AABB 尺寸逐轴 ≤ 上限（step.md 的"尺寸上限"口径）。
func _aabb_size_le(a: AABB, cap: Vector3, tol: float) -> bool:
	for axis in range(3):
		if a.size[axis] > cap[axis] + tol:
			return false
	return true


## 物理节点计数（E27：传感器模型必须是 0）。
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


## 子树里 MeshInstance3D 个数（glb 一个对象一个 mesh，用来证明模型真的实例化了）。
func _count_meshes(node: Node) -> int:
	return _collect_meshes(node).size()


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
