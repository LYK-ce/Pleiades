# Presented by KeJi
# Created Date ： 2026-10-28
# Modified Date ： 2026-10-28
#
# 街道几何（COMPONENTS.md §4.1.2 / task_38 §三.2）：路面 + 标线 + 路缘，运行时建、不进 .tscn。
# 节点结构（**恰好 3 个 MeshInstance3D**，`Street` 下无任何 CollisionObject3D）：
#   Street (Node3D)
#   ├── Road     (MeshInstance3D) 2 个四边形（主路 → 支路，顺序固定）
#   ├── Markings (MeshInstance3D) 28 个四边形（18 主虚线 + 2 边线 + 4 支虚线 + 4 斑马线）
#   └── Curbs    (MeshInstance3D) 35 个四边形（7 段 × 5 面，**不出底面**）
# 坐标：常量一律**世界坐标（Z-up，米）**；顶点直接烘成 Godot 坐标（PosToGodot），节点 transform 恒等。
# 换轴只经 WorldTransform（task_30 §7.3），本文件无换轴字面量。
# 确定性：无 RNG、无数值抖动；标线/斑马线由「线长 3 + 空 3」「白条 0.5 + 间隔 0.5」递推。
# 幂等：Build_All 先摘同名旧容器（remove_child 后再 queue_free）→ 连调两次节点数与 mesh 数不变。
# 深度分层（防 z-fighting）：地面 z=0 / 路面 z=0.01 / 标线 z=0.03；路缘底面不出（与地面共面会闪）。
# 薄片没有 UV ⇒ 材质必须经 WorldLook.Make_Pbr_Material（内部世界空间 triplanar），否则整面一个 texel。

extends RefCounted

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldLook := preload("res://src/world/WorldLook.gd")

const ROOT_NAME := "Street"
const ROAD_NODE := "Road"
const MARK_NODE := "Markings"
const CURB_NODE := "Curbs"

## 路面顶面 z（task_38 §三.6-1 硬性：0.01 ± 0.005）。
const ROAD_TOP_Z := 0.01
## 标线顶面 = 路面 + 0.02（比路面高 2 cm：D24 近裁面 0.05 下 64 m 处深度精度 ≈ 4.9 mm，2 cm = 4 倍余量）。
const MARK_TOP_Z := 0.03
const CURB_H := 0.15
const CURB_W := 0.2

const DASH_LEN := 3.0
const DASH_GAP := 3.0
const DASH_W := 0.15
const MAIN_CENTER_Y := 64.0
const BRANCH_CENTER_X := 48.0
## 主路虚线在路口（支路）与斑马线段跳过（有交即跳过，贴着边界不算相交）。
const MAIN_SKIP_X := [Vector2(44.0, 52.0), Vector2(56.0, 60.0)]
## 主路边线中心距主路边缘 0.30 m（线段宽 0.15 ⇒ y = 60.30 / 67.70）。
const EDGE_LINE_INSET := 0.30

## 材质（贴图接线；ASPHALT / CURB 的米制循环在 WorldLook 里定死）。
const ASPHALT_PREFIX := "asphalt"
const CURB_PREFIX := "curb"
## 标线：纯参数白（不用贴图），roughness 0.55、无 metallic（避免镜面反光把白条打爆）。
const MARK_COLOR := Color(0.85, 0.85, 0.83)
const MARK_ROUGHNESS := 0.55

## 路面 2 块：Rect2(x0, y0, 宽, 深)，世界坐标 Z-up / 米。顺序 = 主路 → 支路（自检按下标取）。
const ROADS := [
	{"name": "main", "rect": Rect2(4.0, 60.0, 120.0, 8.0)},
	{"name": "branch", "rect": Rect2(44.0, 68.0, 8.0, 24.0)},
]

## 路缘 7 段（高 CURB_H、宽 CURB_W）：斑马线 x∈[56,60] 与支路口 x∈[44,52] 处断开。
const CURBS := [
	{"name": "curb_south_west", "rect": Rect2(4.0, 60.0, 52.0, 0.2)},
	{"name": "curb_south_east", "rect": Rect2(60.0, 60.0, 64.0, 0.2)},
	{"name": "curb_north_west", "rect": Rect2(4.0, 67.8, 40.0, 0.2)},
	{"name": "curb_north_mid", "rect": Rect2(52.0, 67.8, 4.0, 0.2)},
	{"name": "curb_north_east", "rect": Rect2(60.0, 67.8, 64.0, 0.2)},
	{"name": "curb_branch_west", "rect": Rect2(44.0, 68.0, 0.2, 24.0)},
	{"name": "curb_branch_east", "rect": Rect2(51.8, 68.0, 0.2, 24.0)},
]

## 斑马线：x∈[56,60] 跨主路 y∈[60,68]，白条 0.5 + 间隔 0.5 ⇒ 4 条。
const CROSSWALK := {"x0": 56.0, "x1": 60.0, "y0": 60.0, "y1": 68.0,
	"bar_w": 0.5, "gap": 0.5}

## 浮点比较余量（几何常量都是 0.05 的整数倍，1e-6 只用来吸收 binary 误差）。
const EPS := 1e-6


# ================================================================ 对外入口

## 建街道（幂等）：先摘同名旧容器，再建 `Street` + 3 个合并 mesh。返回 `Street` 根节点。
static func Build_All(world: Node3D) -> Node3D:
	if world == null:
		return null
	_Remove_Old(world)

	var root := Node3D.new()
	root.name = ROOT_NAME
	world.add_child(root)

	var road_quads: Array = _Collect_Road_Quads()
	var mark_quads: Array = _Collect_Marking_Quads()
	var curb_quads: Array = _Collect_Curb_Quads()

	_Add_Mesh_Node(root, ROAD_NODE, road_quads,
		WorldLook.Make_Pbr_Material(ASPHALT_PREFIX, WorldLook.ASPHALT_TILE_M))
	_Add_Mesh_Node(root, MARK_NODE, mark_quads, _Make_Marking_Material())
	_Add_Mesh_Node(root, CURB_NODE, curb_quads,
		WorldLook.Make_Pbr_Material(CURB_PREFIX, WorldLook.CURB_TILE_M))

	print("[street] built: roads=%d markings=%d curbs=%d nodes=%d" % [
		ROADS.size(), mark_quads.size(), CURBS.size(), root.get_child_count()])
	return root


## 街道容器（自检 / 后续任务用）；不存在返回 null。
static func Get_Root(world: Node3D) -> Node3D:
	if world == null:
		return null
	return world.get_node_or_null(ROOT_NAME) as Node3D


# ================================================================ 几何：路面 / 标线 / 路缘

## 路面 2 个四边形（顶面朝上，无侧面无底面 —— 路面是 1 cm 高的薄片）。
static func _Collect_Road_Quads() -> Array:
	var quads: Array = []
	for spec: Dictionary in ROADS:
		quads.append(_Quad_Up(spec["rect"], ROAD_TOP_Z))
	return quads


## 标线 28 个四边形，顺序固定：主虚线 → 边线 → 支虚线 → 斑马线。
static func _Collect_Marking_Quads() -> Array:
	var quads: Array = []
	for rect: Rect2 in _Main_Center_Dashes():
		quads.append(_Quad_Up(rect, MARK_TOP_Z))
	for rect: Rect2 in _Edge_Lines():
		quads.append(_Quad_Up(rect, MARK_TOP_Z))
	for rect: Rect2 in _Branch_Center_Dashes():
		quads.append(_Quad_Up(rect, MARK_TOP_Z))
	for rect: Rect2 in _Crosswalk_Bars():
		quads.append(_Quad_Up(rect, MARK_TOP_Z))
	return quads


## 主路中心虚线：从 x=4 起 3 m 线 + 3 m 空，落不进主路或与跳过段相交就不要（无 RNG）。
static func _Main_Center_Dashes() -> Array:
	var main: Rect2 = ROADS[0]["rect"]
	var dashes: Array = []
	var x: float = main.position.x
	var x_end: float = main.position.x + main.size.x
	var y0: float = MAIN_CENTER_Y - DASH_W * 0.5
	while x + DASH_LEN <= x_end + EPS:
		if not _Overlaps_Main_Skip(x, x + DASH_LEN):
			dashes.append(Rect2(x, y0, DASH_LEN, DASH_W))
		x += DASH_LEN + DASH_GAP
	return dashes


## 主路边线 2 条（南 / 北，120 m × 0.15 m 实线）。
static func _Edge_Lines() -> Array:
	var main: Rect2 = ROADS[0]["rect"]
	var y_south: float = main.position.y + EDGE_LINE_INSET - DASH_W * 0.5
	var y_north: float = main.position.y + main.size.y - EDGE_LINE_INSET - DASH_W * 0.5
	return [
		Rect2(main.position.x, y_south, main.size.x, DASH_W),
		Rect2(main.position.x, y_north, main.size.x, DASH_W),
	]


## 支路中心虚线：从 y=68 起 3 m 线 + 3 m 空（x = 48 ± 0.075）。
static func _Branch_Center_Dashes() -> Array:
	var branch: Rect2 = ROADS[1]["rect"]
	var dashes: Array = []
	var x0: float = BRANCH_CENTER_X - DASH_W * 0.5
	var y: float = branch.position.y
	var y_end: float = branch.position.y + branch.size.y
	while y + DASH_LEN <= y_end + EPS:
		dashes.append(Rect2(x0, y, DASH_W, DASH_LEN))
		y += DASH_LEN + DASH_GAP
	return dashes


## 斑马线白条 4 条：0.5 m 白条 + 0.5 m 间隔，跨整个主路宽度（y∈[60,68]）。
static func _Crosswalk_Bars() -> Array:
	var bars: Array = []
	var x: float = CROSSWALK["x0"]
	var x1: float = CROSSWALK["x1"]
	var bar_w: float = CROSSWALK["bar_w"]
	var y0: float = CROSSWALK["y0"]
	var depth: float = CROSSWALK["y1"] - CROSSWALK["y0"]
	while x + bar_w <= x1 + EPS:
		bars.append(Rect2(x, y0, bar_w, depth))
		x += bar_w + CROSSWALK["gap"]
	return bars


## 路缘 7 段 × 5 面 = 35 个四边形（顶面 + 4 侧面；**不出底面**：与地面 z=0 共面会闪）。
static func _Collect_Curb_Quads() -> Array:
	var quads: Array = []
	for spec: Dictionary in CURBS:
		_Add_Box_Quads(quads, spec["rect"], 0.0, CURB_H)
	return quads


## 与主路跳过段（路口 / 斑马线）是否相交：贴边界不算相交（[52,55] 与 [44,52] 判不相交）。
static func _Overlaps_Main_Skip(x0: float, x1: float) -> bool:
	for skip: Vector2 in MAIN_SKIP_X:
		if x1 > skip.x + EPS and x0 < skip.y - EPS:
			return true
	return false


# ================================================================ 四边形 / 网格工具

## 水平四边形（顶面朝上，世界 +z）：四角按"从上方看逆时针"给 —— 实测该绕序的法线 = +Y（Godot）。
static func _Quad_Up(rect: Rect2, z: float) -> Array:
	var x0: float = rect.position.x
	var y0: float = rect.position.y
	var x1: float = rect.position.x + rect.size.x
	var y1: float = rect.position.y + rect.size.y
	return [Vector3(x0, y0, z), Vector3(x1, y0, z), Vector3(x1, y1, z), Vector3(x0, y1, z)]


## 盒体 5 个面（顶 + 4 侧，**不出底面**）：rect 是水平投影，z0..z1 是高度区间。
static func _Add_Box_Quads(out: Array, rect: Rect2, z0: float, z1: float) -> void:
	var x0: float = rect.position.x
	var y0: float = rect.position.y
	var x1: float = rect.position.x + rect.size.x
	var y1: float = rect.position.y + rect.size.y
	out.append(_Quad_Up(rect, z1))
	out.append(_Face_Y(y0, x0, x1, z0, z1, -1.0))
	out.append(_Face_Y(y1, x0, x1, z0, z1, 1.0))
	out.append(_Face_X(x0, y0, y1, z0, z1, -1.0))
	out.append(_Face_X(x1, y0, y1, z0, z1, 1.0))


## 竖面四边形：平面 y = plane，沿 x ∈ [u0, u1]，世界 z ∈ [z0, z1]；sign = 外法线朝向（-1 = -y）。
## 绕序已按 Godot 的顺时针正面约定排好（见 _Add_Quad 的注释）。
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


## 把 n 个四边形合成 1 个 MeshInstance3D（**每四边形独立 4 顶点、不焊接、不共享索引**）。
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


## 提交 1 个四边形（4 个世界坐标角点，按绕序给）：法线取 (c-a)×(b-a)。
## Godot 的正面是**顺时针**绕序，generate_normals() 实测其法线 = (c-a)×(b-a)，故以此为准（否则面被剔除
## 或被当成背面去受光）。法线逐顶点写死，三角形 (a,b,c) 与 (a,c,d) 同向。
static func _Add_Quad(st: SurfaceTool, quad: Array, base: int) -> bool:
	var a: Vector3 = WorldTransform.PosToGodot(quad[0])
	var b: Vector3 = WorldTransform.PosToGodot(quad[1])
	var c: Vector3 = WorldTransform.PosToGodot(quad[2])
	var d: Vector3 = WorldTransform.PosToGodot(quad[3])
	var n: Vector3 = (c - a).cross(b - a)
	if n.length_squared() <= 0.0:
		return false
	n = n.normalized()
	st.set_normal(n)
	for p: Vector3 in [a, b, c, d]:
		st.add_vertex(p)
	# 4 顶点 + 2 个三角形 6 个索引：每四边形独立 4 顶点、**不焊接、不与别的四边形共享**
	for idx: int in [base + 0, base + 1, base + 2, base + 0, base + 2, base + 3]:
		st.add_index(idx)
	return true


static func _Make_Marking_Material() -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	mat.albedo_color = MARK_COLOR
	mat.roughness = MARK_ROUGHNESS
	return mat


## 幂等：先出树再释放（queue_free 是延迟的，留在树上会撞名成 Street2）。
static func _Remove_Old(world: Node3D) -> void:
	var old: Node = world.get_node_or_null(ROOT_NAME)
	if old != null:
		world.remove_child(old)
		old.queue_free()
