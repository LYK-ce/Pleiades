# Presented by KeJi
# Created Date ： 2026-11-06
# Modified Date ： 2026-11-06
#
# 室内视觉件（task_49 §五 Step 4）：地板 overlay / 踢脚线 / 门框，**纯视觉、零碰撞**，
# 与 StreetLayout / Facade 同构（运行时建、不进 .tscn、不写 shader）。
# 节点结构（**3 个 MeshInstance3D；`def.ceiling == true` 时第 4 个**；`Interior` 子树里没有任何 CollisionObject3D）：
#   Interior (Node3D)
#   ├── Floor     (MeshInstance3D)  3 个四边形：走廊 96 m² + 门槛 0.5 m² + 房间 18 m²，顶面 z = 0.01
#   ├── Skirting  (MeshInstance3D)  5 个四边形：走廊四面墙脚（东墙被门洞断成两段），z ∈ [0, 0.15]
#   ├── DoorFrame (MeshInstance3D)  4 个四边形：门洞两侧各 0.5 m 竖框（走廊面 + 房间面），z ∈ [0, 3.0]
#   └── Ceiling   (MeshInstance3D)  6 个四边形（仅 ceiling 开）：3 块净空矩形 × 上下两面，z = 3.0
# 坐标：常量一律**世界坐标（Z-up，米）**；顶点直接烘成 Godot 坐标（PosToGodot），节点 transform 恒等。
# 换轴只在 WorldTransform 里发生（task_30 §7.3），本文件无换轴字面量。
#
# 体素对齐（task_49 §3.2 四条硬规则）：
#   ① 面内（切向）坐标全为 0.5 的整数倍，且与体素外表面共面：x ∈ {50, 74, 74.5, 79}、y ∈ {62, 63.5, 64.5, 66}；
#   ② 墙厚 0.5 m（1 格）是**数据**的事（SceneDefs），本文件不碰体素、不改数据；
#   ③ 门洞仍是 1.0 m（y ∈ [63.5, 64.5] = 2 格）：门框**只做在洞口两侧的实墙**上
#      （y ∈ [63.0,63.5] 与 [64.5,65.0]），一个顶点都不落进洞口 ⇒ 门洞净宽不变；
#   ④ 沿"法向"只有 1~2 cm 偏移用于抗 z-fighting（与 Step 4-1 明写的"地板顶面 y = 0.01"、
#      与 Facade 的 SHEET_OUT_M = 0.01 同一手法）：Floor 抬 0.01 / Skirting 内缩 0.01 / DoorFrame 内缩 0.02，
#      三者都 ≤ 0.03（Step 4 坑 5 的凸出上限；口径 #6「凸出物不进体素就不做」在此由 ≤3 cm 装饰性误差放行）。
# 薄片没有 UV ⇒ 材质一律经 WorldLook 工厂（内部世界空间 triplanar），否则整面只采一个 texel（E31）。
# 幂等：Build_All 先摘同名旧容器（remove_child 后再 queue_free）→ 连调两次节点数与 mesh 数不变。
# 天花板（task_49 §五 Step 5）：`def.ceiling == true` 才建，`false` 时一个节点都不建；
# 下表面 z = def.ceiling_z_m = 3.0（= 墙顶 = 体素外表面，共面）；上下两面都建 ⇒ 走廊里看得见顶、
# 俯视观察相机看到"屋顶"（验收 Step 5-3）；**纯 mesh、零碰撞、不进 OBSTACLES**（进体素会挡 UAV 上升与雷达上层）；
# cast_shadow 沿用 _Add_Mesh_Node 的 OFF ⇒ 天花板不挡太阳/环境光，室内亮度不因开顶掉档（口径 #7）。

extends RefCounted

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldLook := preload("res://src/world/WorldLook.gd")

const ROOT_NAME := "Interior"
const FLOOR_NODE := "Floor"
const SKIRTING_NODE := "Skirting"
const DOOR_FRAME_NODE := "DoorFrame"
const CEILING_NODE := "Ceiling"

## 走廊 / 房间 / 门洞的内表面边界（世界 Z-up，米）——与 SceneDefs.SCENES["corridor_indoor"] 的
## 8 条 AABB 逐值对应（§3.3），改一处必须同步另一处。
const CORRIDOR_X0 := 50.0
const CORRIDOR_X1 := 74.0
const CORRIDOR_Y0 := 62.0
const CORRIDOR_Y1 := 66.0
const WALL_T_M := 0.5
const ROOM_X0 := 74.5
const DOOR_Y0 := 63.5
const DOOR_Y1 := 64.5

## 天花板下表面高度（米）的兜底值：正常一律读数据的 `ceiling_z_m`（§3.1 = 3.0 = 6 格，正对墙顶）。
const CEILING_Z_M := 3.0

## 地板 3 块矩形（右开区间），顶点全为 0.5 整数倍。
const FLOOR_RECTS := [
	{"name": "corridor", "rect": Rect2(50.0, 62.0, 24.0, 4.0)},
	{"name": "door", "rect": Rect2(74.0, 63.5, 0.5, 1.0)},
	{"name": "room", "rect": Rect2(74.5, 62.0, 4.5, 4.0)},
]
## 地板顶面 z：与 StreetLayout.ROAD_TOP_Z 同值（Step 4-1 写死 0.01），避免与 Ground 顶面（z=0）共面闪烁。
const FLOOR_TOP_Z := 0.01

## 踢脚线：高 0.15 m（Step 4-1 写死）、零厚度薄片、沿墙面**向内** 0.01 m（≤ 0.03）。
const SKIRT_H_M := 0.15
const SKIRT_INSET_M := 0.01

## 门框：竖框宽 0.5 m（0.5 整数倍）、高 3.0 m（= 墙高 6 格）、沿墙面内缩 0.02 m（≤ 0.03）。
const FRAME_W_M := 0.5
const FRAME_H_M := 3.0
const FRAME_INSET_M := 0.02

const EPS := 1e-6


# ================================================================ 对外入口

## 建室内视觉件（幂等）：先摘同名旧容器，再建 Floor / Skirting / DoorFrame 三个合并 mesh，
## `def.ceiling == true` 时再加 Ceiling 一个。返回 `Interior` 根节点。
## `def` = SceneDefs.Get(scene_name) 的深拷贝，只读 `ceiling` / `ceiling_z_m` 两个键（task_49 §3.1）。
static func Build_All(world: Node3D, def: Dictionary) -> Node3D:
	if world == null:
		return null
	_Remove_Old(world)

	var root := Node3D.new()
	root.name = ROOT_NAME
	world.add_child(root)

	var floor_quads: Array = _Collect_Floor_Quads()
	var skirting_quads: Array = _Collect_Skirting_Quads()
	var frame_quads: Array = _Collect_Door_Frame_Quads()

	_Add_Mesh_Node(root, FLOOR_NODE, floor_quads, WorldLook.Make_Indoor_Floor_Material())
	_Add_Mesh_Node(root, SKIRTING_NODE, skirting_quads, WorldLook.Make_Indoor_Skirting_Material())
	_Add_Mesh_Node(root, DOOR_FRAME_NODE, frame_quads, WorldLook.Make_Indoor_Frame_Material())

	# 天花板（task_49 §五 Step 5）：**只在 def.ceiling == true 时建**（false ⇒ 一个节点都不建）。
	# 纯 mesh、零碰撞、不进 OBSTACLES（进体素会挡 UAV 上升与雷达上层，step.md:16）。
	var ceiling_on: bool = bool(def.get("ceiling", false))
	var ceiling_quads: Array = []
	if ceiling_on:
		ceiling_quads = _Collect_Ceiling_Quads(_Ceiling_Z(def))
		_Add_Mesh_Node(root, CEILING_NODE, ceiling_quads, WorldLook.Make_Indoor_Ceiling_Material())

	# missing_textures=[] = 贴图路径；非空 = 纯色降级路径（口径 #22，两条都算 PASS）
	print("[interior] built: floor=%d skirting=%d frame=%d ceiling_quads=%d nodes=%d missing_textures=%s ceiling=%s" % [
		floor_quads.size(), skirting_quads.size(), frame_quads.size(), ceiling_quads.size(),
		root.get_child_count(), str(WorldLook.Get_Missing_Assets()), str(ceiling_on)])
	return root


## 室内容器（探针 / Step 5 用）；不存在返回 null。
static func Get_Root(world: Node3D) -> Node3D:
	if world == null:
		return null
	return world.get_node_or_null(ROOT_NAME) as Node3D


# ================================================================ 几何

## 地板 3 个四边形（顶面朝上，z = FLOOR_TOP_Z）。三块各自独立，不拼成一个大多边形：
## 分开写才能保证每条边的 x/y 都是 0.5 的整数倍（拼起来会出现 74.0/74.5 的斜接边）。
static func _Collect_Floor_Quads() -> Array:
	var quads: Array = []
	for spec: Dictionary in FLOOR_RECTS:
		quads.append(_Quad_Up(spec["rect"], FLOOR_TOP_Z))
	return quads


## 天花板 6 个四边形 = 3 块净空矩形（复用 FLOOR_RECTS：天花板的覆盖范围**就是**地板覆盖范围）
## × 上下两面。下表面 z = `_Ceiling_Z(def)` = 3.0：
##   ① 切向坐标沿用 FLOOR_RECTS ⇒ x ∈ {50, 74, 74.5, 79}、y ∈ {62, 63.5, 64.5, 66}，全为 0.5 整数倍；
##   ② 下表面 z = 3.0 = 体素墙顶（6 层 = 3 m）⇒ 与体素外表面**共面**（§3.2 规则 4）；
##   ③ 覆盖范围 = 净空并集，**不与墙体重叠** ⇒ 和墙顶面没有共面重叠，不需要 0.01 抗闪偏移；
##   ④ 上下两面**同 z、绕序相反**：背面剔除各管一侧，从走廊里看到下表面（朝下），
##      从俯视观察相机看到上表面（"屋顶"）。两片不共面重叠 ⇒ 不会 z-fighting。
static func _Collect_Ceiling_Quads(z: float) -> Array:
	var quads: Array = []
	for spec: Dictionary in FLOOR_RECTS:
		quads.append(_Quad_Down(spec["rect"], z))
	for spec: Dictionary in FLOOR_RECTS:
		quads.append(_Quad_Up(spec["rect"], z))
	return quads


## 天花板下表面高度（米）：取 def 的 `ceiling_z_m`（§3.1 规定 3.0），缺键 / 非法值回退 CEILING_Z_M。
## 不是 0.5 整数倍时**只 push_warning**（§3.2 规则 1/4 的现场提醒）——绝不 push_error（0 ERROR 验收靠这条）。
static func _Ceiling_Z(def: Dictionary) -> float:
	var v: Variant = def.get("ceiling_z_m", CEILING_Z_M)
	var z: float = float(v) if v is float or v is int else CEILING_Z_M
	if z <= 0.0:
		z = CEILING_Z_M
	if absf(z * 2.0 - roundf(z * 2.0)) > 1e-6:
		push_warning("[interior] ceiling_z_m=%.3f 不是 0.5 的整数倍（task_49 §3.2 规则 1/4）" % z)
	return z


## 踢脚线 5 个四边形：南 / 北 / 西端 / 东墙下段 / 东墙上段（东墙被 1.0 m 门洞断开）。
## 平面 = 体素内表面向内 SKIRT_INSET_M；切向范围就是内表面边界（0.5 整数倍）。
static func _Collect_Skirting_Quads() -> Array:
	var z0 := 0.0
	var z1 := SKIRT_H_M
	return [
		_Face_Y(CORRIDOR_Y0 + SKIRT_INSET_M, CORRIDOR_X0, CORRIDOR_X1, z0, z1, 1.0),
		_Face_Y(CORRIDOR_Y1 - SKIRT_INSET_M, CORRIDOR_X0, CORRIDOR_X1, z0, z1, -1.0),
		_Face_X(CORRIDOR_X0 + SKIRT_INSET_M, CORRIDOR_Y0, CORRIDOR_Y1, z0, z1, 1.0),
		_Face_X(CORRIDOR_X1 - SKIRT_INSET_M, CORRIDOR_Y0, DOOR_Y0, z0, z1, -1.0),
		_Face_X(CORRIDOR_X1 - SKIRT_INSET_M, DOOR_Y1, CORRIDOR_Y1, z0, z1, -1.0),
	]


## 门框 4 个四边形：门洞两侧各一根竖框（宽 0.5 m，**紧贴洞口外侧的实墙**）+ 走廊面 / 房间面各一片。
## 不侵入门洞 ⇒ 门洞净宽仍是 1.0 m（§3.2 规则 3）。
static func _Collect_Door_Frame_Quads() -> Array:
	var z0 := 0.0
	var z1 := FRAME_H_M
	var posts: Array = [
		[DOOR_Y0 - FRAME_W_M, DOOR_Y0],
		[DOOR_Y1, DOOR_Y1 + FRAME_W_M],
	]
	var x_corridor: float = CORRIDOR_X1 - FRAME_INSET_M          # 73.98：朝走廊，法线 -x
	var x_room: float = ROOM_X0 + FRAME_INSET_M                  # 74.52：朝房间，法线 +x
	var quads: Array = []
	for post: Array in posts:
		quads.append(_Face_X(x_corridor, post[0], post[1], z0, z1, -1.0))
		quads.append(_Face_X(x_room, post[0], post[1], z0, z1, 1.0))
	return quads


# ================================================================ 四边形 / 网格工具（与 StreetLayout 逐字同源）

## 水平四边形（顶面朝上，世界 +z）：四角按"从上方看逆时针"给 —— 实测该绕序的法线 = +Y（Godot）。
static func _Quad_Up(rect: Rect2, z: float) -> Array:
	var x0: float = rect.position.x
	var y0: float = rect.position.y
	var x1: float = rect.position.x + rect.size.x
	var y1: float = rect.position.y + rect.size.y
	return [Vector3(x0, y0, z), Vector3(x1, y0, z), Vector3(x1, y1, z), Vector3(x0, y1, z)]


## 水平四边形（**下表面朝下**，世界 -z）：与 `_Quad_Up` 同参、只把四角顺序反过来 ⇒
## 绕序反向 ⇒ `_Add_Quad` 用 (c-a)×(b-a) 算出的法线随之反号 = (0, -1, 0)（Godot），从下方可见。
## 规划期实测（tmp/task49/logs/s5_probe_winding.log）：法线 (0.0, -1.0, -0.000015)，走廊内可见。
static func _Quad_Down(rect: Rect2, z: float) -> Array:
	var x0: float = rect.position.x
	var y0: float = rect.position.y
	var x1: float = rect.position.x + rect.size.x
	var y1: float = rect.position.y + rect.size.y
	return [Vector3(x0, y0, z), Vector3(x0, y1, z), Vector3(x1, y1, z), Vector3(x1, y0, z)]


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


## 幂等：先出树再释放（queue_free 是延迟的，留在树上会撞名成 Interior2）。
static func _Remove_Old(world: Node3D) -> void:
	var old: Node = world.get_node_or_null(ROOT_NAME)
	if old != null:
		world.remove_child(old)
		old.queue_free()
