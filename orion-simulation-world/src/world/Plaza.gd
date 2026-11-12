# Presented by KeJi
# Created Date ： 2026-11-07
# Modified Date ： 2026-11-07
#
# 场地视觉件（task_50 §五 Step 4 / 口径 #15）：路面 / 斑马线 / 人行道 / 场馆 / 背景楼，
# **纯视觉、零碰撞**，与 StreetLayout / Facade / Interior 同构（运行时建、不进 .tscn、不写 shader）。
# 节点结构（6 个直接子节点；`Plaza` 子树里没有任何 CollisionObject3D / CollisionShape3D / GridMap）：
#   Plaza (Node3D)
#   ├── Floor     (MeshInstance3D)  1 个四边形：道路 x∈[56,64] × y∈[0,128]，顶面 z = 0.01
#   ├── Markings  (MeshInstance3D)  6 个四边形：斑马线白条（0.5 m 条 + 1.0 m 空，横跨 x∈[56,64]），z = 0.03
#   ├── Sidewalk  (MeshInstance3D)  4 个四边形：两侧人行道（2 m 宽，顶面 z = 0.06）+ 2 条临路立面
#   ├── Venue     (MeshInstance3D)  7 个四边形：南/北/东墙 + 西墙 2 段 + 门洞过梁 + 屋顶（z = 8.0）
#   ├── VenueDoor (MeshInstance3D)  1 个四边形：门洞内的暗面板（x = 70.02，法线 -x）
#   └── Buildings (Node3D)          按 def.buildings 实例化背景楼（每栋 1 个 glb 实例、零碰撞）
# 坐标：常量一律**世界坐标（Z-up，米）**；顶点直接烘成 Godot 坐标（PosToGodot），节点 transform 恒等。
# 换轴只在 WorldTransform 里发生（task_30 §7.3），本文件无换轴字面量。
# 体素对齐（task_49 §3.2）：面内坐标全为 0.5 的整数倍；本文件不碰体素、不写障碍表。
# 深度分层（防 z-fighting，与 StreetLayout 同源）：地面 z=0 / 路面 0.01 / 标线 0.03 / 人行道 0.06。
# 幂等：Build_All 先摘同名旧容器（remove_child 后再 queue_free）→ 连调两次节点数与 mesh 数不变。

extends RefCounted

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldLook := preload("res://src/world/WorldLook.gd")

const ROOT_NAME := "Plaza"
const FLOOR_NODE := "Floor"
const ZEBRA_NODE := "Markings"
const SIDEWALK_NODE := "Sidewalk"
const VENUE_NODE := "Venue"
const DOOR_NODE := "VenueDoor"
const BUILDINGS_NODE := "Buildings"

## 路面顶面 z（step.md:16「顶面 z 等于 0.01」；与 StreetLayout.ROAD_TOP_Z 同值）。
const ROAD_TOP_Z := 0.01
## 标线顶面 = 路面 + 0.02（与 StreetLayout.MARK_TOP_Z 同值）。
const MARK_TOP_Z := 0.03
## 人行道顶面 = 路面 + 0.05（任务文档 §5.4-3「略高于路面 0.05 m」）。
const SIDEWALK_TOP_Z := 0.06

## 道路：x∈[56,64]（8 m 宽，口径 #1），南北贯通 y∈[0,128]。
const ROAD := Rect2(56.0, 0.0, 8.0, 128.0)
## 人行道：路两侧各 2 m（x∈[54,56] 与 x∈[64,66]），同样南北贯通。
const SIDEWALKS := [
	Rect2(54.0, 0.0, 2.0, 128.0),
	Rect2(64.0, 0.0, 2.0, 128.0),
]
## 斑马线：y∈[58,66]（8 m），6 条白条 = 0.5 m 条 + 1.0 m 空（6×0.5 + 5×1.0 = 8.0），横跨整个路宽。
const ZEBRA := {"x0": 56.0, "x1": 64.0, "y0": 58.0, "y1": 66.0,
	"bar_d": 0.5, "gap_d": 1.0, "count": 6}

## 场馆（任务文档 §5.3）：x∈[70,100] / y∈[70,100]，墙高 8 m（区间 6~8 m 取上界）；
## 西墙 x=70 留门洞 y∈[80,84]（净高 4 m）；出生点 (68,82) 就在门洞外 2 m。
const VENUE_X0 := 70.0
const VENUE_X1 := 100.0
const VENUE_Y0 := 70.0
const VENUE_Y1 := 100.0
const VENUE_H := 8.0
const DOOR_Y0 := 80.0
const DOOR_Y1 := 84.0
const DOOR_H := 4.0
## 门洞暗面板：贴在墙面**内侧** 0.02 m（≤ 3 cm 装饰性偏移，与 Interior 的 INSET 同一手法）⇒
## 从街上透过门洞看到的是"暗的门口"，而不是被背面剔除后一路穿到天空。
const DOOR_PANEL_INSET_M := 0.02
const DOOR_PANEL_TINT := 0.25

## 材质前缀（WorldLook 的贴图组）与标线纯色（与 StreetLayout 逐值一致）。
const ROAD_PREFIX := "asphalt"
const SIDEWALK_PREFIX := "curb"
const VENUE_PREFIX := "concrete"
const MARK_COLOR := Color(0.85, 0.85, 0.83)
const MARK_ROUGHNESS := 0.55

const EPS := 1e-6


# ================================================================ 对外入口

## 建 venue 场地（幂等）：先摘同名旧容器，再建 Floor/Markings/Sidewalk/Venue/VenueDoor + Buildings。
## `def` = SceneDefs.Get("venue") 的深拷贝；只读一个键：`buildings`（缺键 / 非数组 ⇒ 一栋都不建）。
static func Build_All(world: Node3D, def: Dictionary) -> Node3D:
	if world == null:
		return null
	_Remove_Old(world)

	var root := Node3D.new()
	root.name = ROOT_NAME
	world.add_child(root)

	var road_quads: Array = [_Quad_Up(ROAD, ROAD_TOP_Z)]
	var zebra_quads: Array = _Collect_Zebra_Quads()
	var sidewalk_quads: Array = _Collect_Sidewalk_Quads()
	var venue_quads: Array = _Collect_Venue_Quads()
	var door_quads: Array = _Collect_Door_Quads()

	_Add_Mesh_Node(root, FLOOR_NODE, road_quads,
		WorldLook.Make_Pbr_Material(ROAD_PREFIX, WorldLook.ASPHALT_TILE_M))
	_Add_Mesh_Node(root, ZEBRA_NODE, zebra_quads, _Make_Marking_Material())
	_Add_Mesh_Node(root, SIDEWALK_NODE, sidewalk_quads,
		WorldLook.Make_Pbr_Material(SIDEWALK_PREFIX, WorldLook.CURB_TILE_M))
	# 2026-11-08：场馆 / 门洞已移除（quad 数组为空）⇒ 不再建空 MeshInstance 节点。
	if not venue_quads.is_empty():
		_Add_Mesh_Node(root, VENUE_NODE, venue_quads,
			WorldLook.Make_Pbr_Material(VENUE_PREFIX, WorldLook.VOXEL_TILE_M))
	if not door_quads.is_empty():
		_Add_Mesh_Node(root, DOOR_NODE, door_quads,
			WorldLook.Make_Pbr_Material(VENUE_PREFIX, WorldLook.VOXEL_TILE_M, DOOR_PANEL_TINT))
	# 背景楼最后建 ⇒ 子节点顺序与头注释的节点树逐行一致（Floor…Buildings）。
	var built: int = _Build_Buildings(root, def)

	print("[plaza] built: road=%d zebra=%d sidewalk=%d venue=%d door=%d buildings=%d quads=%d nodes=%d missing_textures=%s" % [
		road_quads.size(), zebra_quads.size(), sidewalk_quads.size(), venue_quads.size(),
		door_quads.size(), built,
		road_quads.size() + zebra_quads.size() + sidewalk_quads.size()
			+ venue_quads.size() + door_quads.size(),
		root.get_child_count(), str(WorldLook.Get_Missing_Assets())])
	return root


## 场地容器（探针 / 后续步用）；不存在返回 null。
static func Get_Root(world: Node3D) -> Node3D:
	if world == null:
		return null
	return world.get_node_or_null(ROOT_NAME) as Node3D


# ================================================================ 几何

## 斑马线 6 条：y 从 58.0 起「0.5 条 + 1.0 空」递推，横跨整条路（x∈[56,64]）。
## 无 RNG、无浮点抖动；参数自相矛盾时只 push_warning（0 ERROR 验收靠这条）。
static func _Collect_Zebra_Quads() -> Array:
	var bar_d: float = ZEBRA["bar_d"]
	var gap_d: float = ZEBRA["gap_d"]
	var count: int = int(ZEBRA["count"])
	var span: float = float(ZEBRA["y1"]) - float(ZEBRA["y0"])
	if absf(span - (count * bar_d + (count - 1) * gap_d)) > EPS:
		push_warning("[plaza] 斑马线参数不自洽：span=%.3f vs %d×%.3f + %d×%.3f"
			% [span, count, bar_d, count - 1, gap_d])
	var width: float = float(ZEBRA["x1"]) - float(ZEBRA["x0"])
	var x0: float = float(ZEBRA["x0"])
	var quads: Array = []
	var y: float = float(ZEBRA["y0"])
	for i: int in range(count):
		quads.append(_Quad_Up(Rect2(x0, y, width, bar_d), MARK_TOP_Z))
		y += bar_d + gap_d
	return quads


## 人行道 4 个四边形 = 2 个顶面 + 2 条临路立面（5 cm 台阶的立面，法线朝路）。
static func _Collect_Sidewalk_Quads() -> Array:
	var quads: Array = []
	for rect: Rect2 in SIDEWALKS:
		quads.append(_Quad_Up(rect, SIDEWALK_TOP_Z))
	quads.append(_Face_X(ROAD.position.x, ROAD.position.y, ROAD.position.y + ROAD.size.y,
		ROAD_TOP_Z, SIDEWALK_TOP_Z, 1.0))                    # x=56，法线 +x（朝路）
	quads.append(_Face_X(ROAD.position.x + ROAD.size.x, ROAD.position.y,
		ROAD.position.y + ROAD.size.y, ROAD_TOP_Z, SIDEWALK_TOP_Z, -1.0))  # x=64，法线 -x（朝路）
	return quads


## 场馆 7 个四边形：单面朝外（俯视机位只看得见"屋顶 + 西墙 + 北墙"，正好合成一个封闭体块）。
## 2026-11-08 反馈：场馆体块贴路太近、把演示空间挤没了 ⇒ **本场景不再建场馆**（返回空数组，
## 常量与旧几何保留作历史）；道路两侧改由 `SceneDefs["venue"].buildings` 的街墙承担城市观感。
static func _Collect_Venue_Quads() -> Array:
	return []


## 门洞暗面板 1 个：x = 70.02（墙面内侧 2 cm），法线 -x（朝街）。
## 2026-11-08：场馆已移除 ⇒ 门洞面板一并取消（返回空数组）。
static func _Collect_Door_Quads() -> Array:
	return []


# ================================================================ 背景楼

## 按 `def.buildings` 实例化背景楼（口径 #15：纯视觉、零碰撞、不进体素、不进障碍表）。
## 该键是 venue 独有的**新增字段**：`def.get("buildings", [])`；缺键 / 非数组 ⇒ 空数组 ⇒ 一栋都不建，
## 且**绝不 push_error**（step.md:59：city 与 corridor_indoor 不带该键也必须能跑）。
## 每项 {model: String, pos: Vector3(世界坐标 Z-up), rot_y_deg: float, scale: float}；
## 缺项按缺省值（rot 0 / scale 1）；任一项不合法 ⇒ 只 warn 跳过该项，其余照建。
static func _Build_Buildings(root: Node3D, def: Dictionary) -> int:
	var holder := Node3D.new()
	holder.name = BUILDINGS_NODE
	root.add_child(holder)

	var raw: Variant = def.get("buildings", [])
	if not (raw is Array):
		push_warning("[plaza] def.buildings 不是数组，按空数组处理")
		return 0
	var specs: Array = raw
	var built: int = 0
	for i: int in range(specs.size()):
		var spec: Variant = specs[i]
		if not (spec is Dictionary):
			push_warning("[plaza] buildings[%d] 不是 Dictionary，跳过" % i)
			continue
		var dict: Dictionary = spec
		var model_path := String(dict.get("model", ""))
		var pos_v: Variant = dict.get("pos", Vector3.ZERO)
		if not (pos_v is Vector3):
			push_warning("[plaza] buildings[%d].pos 不是 Vector3，跳过" % i)
			continue
		var pos: Vector3 = pos_v
		if model_path == "" or not ResourceLoader.exists(model_path):
			push_warning("[plaza] buildings[%d] 模型缺失 / 未 import：%s（跳过）" % [i, model_path])
			continue
		var packed: PackedScene = load(model_path) as PackedScene
		if packed == null:
			push_warning("[plaza] buildings[%d] load() 返回 null：%s（跳过）" % [i, model_path])
			continue
		var inst: Node3D = packed.instantiate() as Node3D
		if inst == null:
			push_warning("[plaza] buildings[%d] 实例根不是 Node3D：%s（跳过）" % [i, model_path])
			continue
		var rot_deg := float(dict.get("rot_y_deg", 0.0))
		var factor := float(dict.get("scale", 1.0))
		inst.name = "Building_%02d" % (i + 1)
		inst.position = WorldTransform.PosToGodot(pos)
		# rotation.y = -world_yaw（WorldTransform.YawToGodot 是唯一换轴入口，本文件不写符号）
		inst.rotation = Vector3(0.0, WorldTransform.YawToGodot(deg_to_rad(rot_deg)), 0.0)
		inst.scale = Vector3(factor, factor, factor)
		_Set_Shadows_Off(inst)
		holder.add_child(inst)
		built += 1
	return built


## 背景楼一律不投影：软渲染（llvmpipe ~11 fps 基线）下 20 m 高楼的长影子会把帧率吃掉，
## 且与既有街道 / 立面 / 室内件「cast_shadow = OFF」的做法一致（COMPONENTS.md §2.1）。
static func _Set_Shadows_Off(node: Node) -> void:
	for child: Node in node.find_children("*", "MeshInstance3D", true, false):
		var mi := child as MeshInstance3D
		if mi != null:
			mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF


# ================================================================ 四边形 / 网格工具（与 Interior / StreetLayout 逐字同源）

## 水平四边形（顶面朝上，世界 +z）：四角按"从上方看逆时针"给 —— 实测该绕序的法线 = +Y（Godot）。
static func _Quad_Up(rect: Rect2, z: float) -> Array:
	var x0: float = rect.position.x
	var y0: float = rect.position.y
	var x1: float = rect.position.x + rect.size.x
	var y1: float = rect.position.y + rect.size.y
	return [Vector3(x0, y0, z), Vector3(x1, y0, z), Vector3(x1, y1, z), Vector3(x0, y1, z)]


## 竖面四边形：平面 y = plane，沿 x ∈ [u0, u1]，世界 z ∈ [z0, z1]；sign = 外法线朝向（-1 = -y）。
static func _Face_Y(plane: float, u0: float, u1: float, z0: float, z1: float, sign: float) -> Array:
	if sign > 0.0:
		return [Vector3(u1, plane, z0), Vector3(u0, plane, z0),
			Vector3(u0, plane, z1), Vector3(u1, plane, z1)]
	return [Vector3(u0, plane, z0), Vector3(u1, plane, z0),
		Vector3(u1, plane, z1), Vector3(u0, plane, z1)]


## 竖面四边形：平面 x = plane，沿 y ∈ [u0, u1]，世界 z ∈ [z0, z1]；sign = 外法线朝向（-1 = -x）。
static func _Face_X(plane: float, u0: float, u1: float, z0: float, z1: float, sign: float) -> Array:
	if sign > 0.0:
		return [Vector3(plane, u0, z0), Vector3(plane, u1, z0),
			Vector3(plane, u1, z1), Vector3(plane, u0, z1)]
	return [Vector3(plane, u1, z0), Vector3(plane, u0, z0),
		Vector3(plane, u0, z1), Vector3(plane, u1, z1)]


## 把 n 个四边形合成 1 个 MeshInstance3D（**每四边形独立 4 顶点、不焊接、不共享索引**，E31-②）。
## 顶点直接烘成 Godot 坐标 ⇒ 节点 transform 恒等；材质挂在 mesh 的 surface 上（material_override 保持 null）。
static func _Add_Mesh_Node(root: Node3D, node_name: String, quads: Array, mat: Material) -> MeshInstance3D:
	var st := SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	if mat != null:
		st.set_material(mat)
	var base: int = 0
	for quad: Array in quads:
		if _Add_Quad(st, quad, base):
			base += 4
	var mesh: ArrayMesh = st.commit()

	var mi := MeshInstance3D.new()
	mi.name = node_name
	mi.mesh = mesh
	mi.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	mi.transform = Transform3D.IDENTITY
	root.add_child(mi)
	return mi


## 提交 1 个四边形（4 个世界坐标角点，按绕序给）：法线取 (c-a)×(b-a)（Godot 正面 = 顺时针，E31-③）。
static func _Add_Quad(st: SurfaceTool, quad: Array, base: int) -> bool:
	var a: Vector3 = WorldTransform.PosToGodot(quad[0])
	var b: Vector3 = WorldTransform.PosToGodot(quad[1])
	var c: Vector3 = WorldTransform.PosToGodot(quad[2])
	var d: Vector3 = WorldTransform.PosToGodot(quad[3])
	var n: Vector3 = (c - a).cross(b - a)
	if n.length_squared() <= EPS:
		return false
	n = n.normalized()
	st.set_normal(n)
	for p: Vector3 in [a, b, c, d]:
		st.add_vertex(p)
	for idx: int in [base + 0, base + 1, base + 2, base + 0, base + 2, base + 3]:
		st.add_index(idx)
	return true


## 标线材质：纯参数白（不用贴图），roughness 0.55、无 metallic —— 与 StreetLayout 逐值一致。
static func _Make_Marking_Material() -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	mat.albedo_color = MARK_COLOR
	mat.roughness = MARK_ROUGHNESS
	return mat


## 幂等：先出树再释放（queue_free 是延迟的，留在树上会撞名成 Plaza2）。
static func _Remove_Old(world: Node3D) -> void:
	var old: Node = world.get_node_or_null(ROOT_NAME)
	if old != null:
		world.remove_child(old)
		old.queue_free()
