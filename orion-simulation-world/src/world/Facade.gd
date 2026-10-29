# Presented by KeJi
# Created Date ： 2026-10-28
# Modified Date ： 2026-10-28
#
# 建筑立面（COMPONENTS.md §4.1.2 / task_38 §三.3）：窗 / 基座 / 屋顶压顶，全用 StandardMaterial3D
# + SurfaceTool 合并 mesh，**不写 shader**、不进 .tscn、不加任何碰撞。
# 节点结构（**恰好 3 个 MeshInstance3D**，`Facade` 下无任何 CollisionObject3D）：
#   Facade (Node3D)
#   ├── Windows (MeshInstance3D) 60 个四边形：4 个边界矮墙之外 11 个障碍的全部窗
#   ├── Plinths (MeshInstance3D) 44 个四边形：11 × 4 面，z∈[0, 0.6]
#   └── Caps    (MeshInstance3D) 15 个四边形：15 个障碍各 1 个，外扩 0.05
# 输入：**只读** `World.OBSTACLES`（经 node 的脚本常量表取，不 preload World.gd ⇒ 无循环依赖）。
# 常量一律世界坐标（Z-up，米）；顶点直接烘成 Godot 坐标（PosToGodot），节点 transform 恒等。
# 薄片没有 UV ⇒ 材质必须经 WorldLook.Make_Pbr_Material（内部世界空间 triplanar），否则整面一个 texel。
# 幂等：Build_All 先摘同名旧容器（remove_child 后再 queue_free）→ 连调两次节点数与 mesh 数不变。
# 层数：§三.3 的字面公式 floor((h-0.9)/3.2) 会让 h=4 的 building_a 也算 0 层（全表只剩 2 栋有窗），
#   故按 Floors_Of() 加"一档下限"：h ≥ SILL+WIN_H（2.5 m）却不足一整层时按 1 层算（推导见 impl.md）。

extends RefCounted

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const WorldLook := preload("res://src/world/WorldLook.gd")

const ROOT_NAME := "Facade"
const WINDOW_NODE := "Windows"
const PLINTH_NODE := "Plinths"
const CAP_NODE := "Caps"

const BAY_M := 2.4
const WIN_W_M := 1.8
const WIN_H_M := 1.6
const SILL_M := 0.9
const FLOOR_M := 3.2
## 面宽不足 2.0 m 不开窗（1.8 m 窗放不下，两侧各留 0.1 m）。
const FACE_MIN_W_M := 2.0
## 窗 / 基座薄片一律向墙外偏移 0.01 m（既不穿墙也不留缝）。
const SHEET_OUT_M := 0.01
const PLINTH_Z0 := 0.0
const PLINTH_Z1 := 0.6
## 压顶外扩 0.05 m；z = h + CAP_LIFT_M + CAP_LIFT_STEP_M × 障碍下标（逐栋不同 ⇒ 相邻压顶不共面）。
const CAP_OUT_M := 0.05
const CAP_LIFT_M := 0.01
const CAP_LIFT_STEP_M := 0.001
## 只做压顶的 4 面边界矮墙（窗/基座一律跳过）。
const BOUNDARY_WALLS := ["wall_south", "wall_north", "wall_west", "wall_east"]

## 窗 = 深色玻璃（task_38 §三.3 写死：albedo ≈ 0.07、roughness 0.15、metallic 0.3）。
const GLASS_COLOR := Color(0.07, 0.08, 0.09)
const GLASS_ROUGHNESS := 0.15
const GLASS_METALLIC := 0.3
## 基座 = 混凝土压暗一档（tint 0.7）；压顶 = 路缘石压暗一档（tint 0.6）。
const PLINTH_PREFIX := "concrete"
const PLINTH_TINT := 0.7
const CAP_PREFIX := "curb"
const CAP_TINT := 0.6

const EPS := 1e-6


# ================================================================ 对外入口

## 建立面（幂等）：先摘同名旧容器，再按 OBSTACLES 逐栋生成并合并成 3 个 mesh。返回 `Facade` 根节点。
static func Build_All(world: Node3D) -> Node3D:
	if world == null:
		return null
	_Remove_Old(world)

	var win_quads: Array = []
	var plinth_quads: Array = []
	var cap_quads: Array = []
	var obstacles: Array = _Obstacles_Of(world)
	for i: int in range(obstacles.size()):
		var spec: Dictionary = obstacles[i]
		var origin: Vector3 = spec["origin"]
		var size: Vector3 = spec["size"]
		var x0: float = origin.x
		var y0: float = origin.y
		var x1: float = origin.x + size.x
		var y1: float = origin.y + size.y
		var h: float = size.z
		# 压顶：15 个障碍全做（含 4 面边界矮墙，解决"俯视看到墙皮"）。
		cap_quads.append(_Cap_Quad(x0, y0, x1, y1, h, i))
		if BOUNDARY_WALLS.has(String(spec["name"])):
			continue
		_Add_Plinth_Quads(plinth_quads, x0, y0, x1, y1)
		_Add_Window_Quads(win_quads, x0, y0, x1, y1, h)

	var root := Node3D.new()
	root.name = ROOT_NAME
	world.add_child(root)
	_Add_Mesh_Node(root, WINDOW_NODE, win_quads, _Make_Glass_Material())
	_Add_Mesh_Node(root, PLINTH_NODE, plinth_quads,
		WorldLook.Make_Pbr_Material(PLINTH_PREFIX, WorldLook.VOXEL_TILE_M, PLINTH_TINT))
	_Add_Mesh_Node(root, CAP_NODE, cap_quads,
		WorldLook.Make_Pbr_Material(CAP_PREFIX, WorldLook.CURB_TILE_M, CAP_TINT))

	print("[facade] built: windows=%d plinths=%d caps=%d nodes=%d" % [
		win_quads.size(), plinth_quads.size(), cap_quads.size(), root.get_child_count()])
	return root


## 立面容器（自检 / 后续任务用）；不存在返回 null。
static func Get_Root(world: Node3D) -> Node3D:
	if world == null:
		return null
	return world.get_node_or_null(ROOT_NAME) as Node3D


# ================================================================ 纯函数（自检独立复算同一公式）

## 层数：§三.3 字面公式 floor((h - SILL) / FLOOR)，再加"一档下限"：
## h ≥ SILL + WIN_H（= 2.5 m）却不足一整层时按 1 层算（否则 h=4 的 building_a 一扇窗都没有）。
## h=4 → 1 层、h=5 → 1 层、h=8 → 2 层、h=3 → 1 层、h=2 / h=1 → 0 层。
static func Floors_Of(h: float) -> int:
	var n: int = int(floor((h - SILL_M) / FLOOR_M))
	if n == 0 and h >= SILL_M + WIN_H_M - EPS:
		n = 1
	return n


## 每面窗数：n = max(1, floor(w / BAY))，但面宽 < 2.0 m 的面不开窗（1.8 m 窗放不下）。
static func Bays_Of(w: float) -> int:
	if w < FACE_MIN_W_M - EPS:
		return 0
	return maxi(1, int(floor(w / BAY_M + EPS)))


# ================================================================ 窗 / 基座 / 压顶

## 窗：4 个面各 Bays_Of(面宽) 扇/层 × Floors_Of(h) 层；窗面在墙平面外扩 SHEET_OUT_M。
## ±x 面的面宽 = size.y，±y 面的面宽 = size.x；沿面方向按等间距 pitch = 面宽 / n 排布。
static func _Add_Window_Quads(out: Array, x0: float, y0: float, x1: float, y1: float, h: float) -> void:
	var floors: int = Floors_Of(h)
	if floors <= 0:
		return
	var half_h: float = WIN_H_M * 0.5
	var half_w: float = WIN_W_M * 0.5

	# ±x 面（面宽 = size.y）
	var bays_y: int = Bays_Of(y1 - y0)
	if bays_y > 0:
		var pitch_y: float = (y1 - y0) / float(bays_y)
		for k: int in range(floors):
			var zc: float = SILL_M + float(k) * FLOOR_M + half_h
			for i: int in range(bays_y):
				var uc: float = y0 + (float(i) + 0.5) * pitch_y
				out.append(_Face_X(x0 - SHEET_OUT_M, uc - half_w, uc + half_w,
					zc - half_h, zc + half_h, -1.0))
				out.append(_Face_X(x1 + SHEET_OUT_M, uc - half_w, uc + half_w,
					zc - half_h, zc + half_h, 1.0))

	# ±y 面（面宽 = size.x）
	var bays_x: int = Bays_Of(x1 - x0)
	if bays_x > 0:
		var pitch_x: float = (x1 - x0) / float(bays_x)
		for k: int in range(floors):
			var zc2: float = SILL_M + float(k) * FLOOR_M + half_h
			for i2: int in range(bays_x):
				var uc2: float = x0 + (float(i2) + 0.5) * pitch_x
				out.append(_Face_Y(y0 - SHEET_OUT_M, uc2 - half_w, uc2 + half_w,
					zc2 - half_h, zc2 + half_h, -1.0))
				out.append(_Face_Y(y1 + SHEET_OUT_M, uc2 - half_w, uc2 + half_w,
					zc2 - half_h, zc2 + half_h, 1.0))


## 基座：每栋 4 个竖四边形，z∈[0, 0.6]，外扩 SHEET_OUT_M，沿面方向两端各多伸 SHEET_OUT_M（包角）。
static func _Add_Plinth_Quads(out: Array, x0: float, y0: float, x1: float, y1: float) -> void:
	var ya: float = y0 - SHEET_OUT_M
	var yb: float = y1 + SHEET_OUT_M
	var xa: float = x0 - SHEET_OUT_M
	var xb: float = x1 + SHEET_OUT_M
	out.append(_Face_X(x0 - SHEET_OUT_M, ya, yb, PLINTH_Z0, PLINTH_Z1, -1.0))
	out.append(_Face_X(x1 + SHEET_OUT_M, ya, yb, PLINTH_Z0, PLINTH_Z1, 1.0))
	out.append(_Face_Y(y0 - SHEET_OUT_M, xa, xb, PLINTH_Z0, PLINTH_Z1, -1.0))
	out.append(_Face_Y(y1 + SHEET_OUT_M, xa, xb, PLINTH_Z0, PLINTH_Z1, 1.0))


## 屋顶压顶：水平四边形，矩形外扩 CAP_OUT_M，z = h + 0.01 + 0.001 × 障碍下标（逐栋不同 ⇒ 不共面）。
static func _Cap_Quad(x0: float, y0: float, x1: float, y1: float, h: float, index: int) -> Array:
	var z: float = h + CAP_LIFT_M + CAP_LIFT_STEP_M * float(index)
	var rect := Rect2(x0 - CAP_OUT_M, y0 - CAP_OUT_M,
		(x1 - x0) + 2.0 * CAP_OUT_M, (y1 - y0) + 2.0 * CAP_OUT_M)
	return _Quad_Up(rect, z)


# ================================================================ 输入 / 四边形 / 网格工具

## 只读拿 World.OBSTACLES：const 不在属性表里，只能走 get_script_constant_map()
## （check_world_look.gd 读 LOOK_ENABLED 用的同一招）。缺常量返回空表（不报错）。
static func _Obstacles_Of(world: Node3D) -> Array:
	if world == null or world.get_script() == null:
		return []
	var consts: Dictionary = world.get_script().get_script_constant_map()
	if not consts.has("OBSTACLES"):
		return []
	return consts["OBSTACLES"] as Array


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


## 把 n 个四边形合成 1 个 MeshInstance3D（**每四边形独立 4 顶点、不焊接、不共享索引**）。
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


## 窗：深色玻璃。CULL_DISABLED —— 窗是"贴"在墙上的单片，从墙内看也不该露出黑洞。
static func _Make_Glass_Material() -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	mat.albedo_color = GLASS_COLOR
	mat.roughness = GLASS_ROUGHNESS
	mat.metallic = GLASS_METALLIC
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	return mat


## 幂等：先出树再释放（queue_free 是延迟的，留在树上会撞名成 Facade2）。
static func _Remove_Old(world: Node3D) -> void:
	var old: Node = world.get_node_or_null(ROOT_NAME)
	if old != null:
		world.remove_child(old)
		old.queue_free()
