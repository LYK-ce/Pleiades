# Presented by KeJi
# Created Date ： 2026-10-28
# Modified Date ： 2026-10-28
#
# 街道几何 + 建筑立面自检（task_38 Step 2，COMPONENTS.md §4.1.2）：
#   S 组 28 项结构（headless 也全跑）+ R 组 13 项渲染（仅 Xvfb；headless 打印 13 条 [skip] 且不计入 total）。
# 把关：路网 AABB 与路面顶 z / Street+Facade 子树零碰撞 / 标线与斑马线条数 / 路缘段数与断口 /
#   立面 mesh 数、逐栋逐面窗数、基座与压顶几何 / 连调两次节点数不变 / OBSTACLES 与尺度碰撞层零改动 /
#   3 张 1600×900 街景图（路口平视 / 建筑侧视 / UAV 俯视）+ 窗暗度 + 斑马线对比度。
# 几何一律从 mesh 顶点反推（每四边形独立 4 顶点、不焊接 ⇒ verts.size() == 4 × quads），
#   不读任何自报数字；坐标换轴只经 WorldTransform（本文件无换轴字面量）。
# 本脚本**不 push_error**（失败一律 _check(..., false, detail) + quit(1)），否则 0 ERROR 验收假红。
#
# 跑法 ①（headless，结构档：total=28 + 13 条 [skip]）：
#   godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_street.gd
# 跑法 ②（Xvfb + 软件 OpenGL，S+R 全跑：total=41 + 3 张 jpg）：
#   xvfb-run -a -s "-screen 0 1600x900x24" env LIBGL_ALWAYS_SOFTWARE=1 godot \
#     --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
#     --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_street.gd
#
# 图片只落 tmp/task38/shots/（绝对路径），**绝不**写进 res://。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")
const World := preload("res://src/world/World.gd")
const StreetLayout := preload("res://src/world/StreetLayout.gd")
const Facade := preload("res://src/world/Facade.gd")

const WORLD_SCENE := "res://src/scenes/World.tscn"
const CAMERA_SCENE := "res://src/scenes/sensors/CameraSensor.tscn"

## 图片落 tmp/task38/shots/（**绝不**写进 res://，否则污染仓库）
const OUT_DIR := "/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/task38/shots"
const SHOT_SIZE := Vector2i(1600, 900)
const SHOT_FOV := 70.0
const SHOT_QUALITY := 0.85
const MIN_BYTES := 10 * 1024

## 三机位（**世界坐标 Z-up / 米**，一律经 WorldTransform.PosToGodot 换轴）
const SHOTS := [
	# 路口平视：站在支路上（x=48）朝 T 型路口看，斑马线 4 条 + 两侧路缘断口 + 主路虚线
	{"id": 0, "file": "street_00_junction.jpg",
		"eye": Vector3(48.0, 74.0, 1.70), "aim": Vector3(57.0, 63.0, 0.25)},
	# 建筑侧视：building_a（世界 x∈[24,34]、y∈[24,32]、高 4 m）西南两面 3/4 视角
	{"id": 1, "file": "street_01_building_side.jpg",
		"eye": Vector3(17.0, 21.0, 2.10), "aim": Vector3(31.0, 31.0, 1.50)},
	# UAV 俯视：高 20 m 斜下（不写纯垂直：look_at 与 UP 平行会退化）
	{"id": 2, "file": "street_02_uav_top.jpg",
		"eye": Vector3(62.0, 52.0, 20.0), "aim": Vector3(50.0, 70.0, 0.0)},
]

## 像素判据（与 check_world_look / check_camera_shots **同值，不得放宽**）
const VAR_MIN := 0.002
const MEAN_MIN := 0.05
const MEAN_MAX := 0.95
const SPREAD_MIN := 0.15
const WHITE_MAX := 0.50
const DIFF_BYTE_MIN := 0.10
const DIFF_MEAN_MIN := 0.02
const DIFF_STEP := 8

# ---------------------------------------------------------------- 规格常量（task_38 §三.2 / §三.3）
## 以下数值**独立复算**自任务文档，不 import StreetLayout / Facade 的常量表（防自证）。
const ROAD_MAIN := Rect2(4.0, 60.0, 120.0, 8.0)
const ROAD_BRANCH := Rect2(44.0, 68.0, 8.0, 24.0)
const ROAD_TOP_Z := 0.01
const ROAD_TOP_TOL := 0.005
const MARK_TOP_Z := 0.03
const CURB_H := 0.15
const CURB_W := 0.2
const DASH_LEN := 3.0
const DASH_GAP := 3.0
const DASH_W := 0.15
const MAIN_CENTER_Y := 64.0
const BRANCH_CENTER_X := 48.0
const EDGE_LINE_Y := [60.30, 67.70]
const EDGE_LINE_LEN := 120.0
const CROSSWALK_X0 := 56.0
const CROSSWALK_Y0 := 60.0
const CROSSWALK_W := 4.0
const CROSSWALK_BAR_W := 0.5
const CROSSWALK_D := 8.0
const MAIN_SKIP_X := [Vector2(44.0, 52.0), Vector2(56.0, 60.0)]
## 7 段路缘（与 §三.2 表格逐行一致）
const CURB_RECTS := [
	Rect2(4.0, 60.0, 52.0, 0.2),
	Rect2(60.0, 60.0, 64.0, 0.2),
	Rect2(4.0, 67.8, 40.0, 0.2),
	Rect2(52.0, 67.8, 4.0, 0.2),
	Rect2(60.0, 67.8, 64.0, 0.2),
	Rect2(44.0, 68.0, 0.2, 24.0),
	Rect2(51.8, 68.0, 0.2, 24.0),
]
## 立面参数（§三.3）
const BAY_M := 2.4
const WIN_W_M := 1.8
const WIN_H_M := 1.6
const SILL_M := 0.9
const FLOOR_M := 3.2
const FACE_MIN_W_M := 2.0
const SHEET_OUT_M := 0.01
const PLINTH_Z0 := 0.0
const PLINTH_Z1 := 0.6
const CAP_OUT_M := 0.05
const CAP_LIFT_M := 0.01
const CAP_LIFT_STEP_M := 0.001
const BOUNDARY_WALLS := ["wall_south", "wall_north", "wall_west", "wall_east"]
## 逐栋窗数（§三.3 实算表，实现必须逐栋对上）
const WINDOW_EXPECT := {
	"wall_south": 0, "wall_north": 0, "wall_west": 0, "wall_east": 0,
	"building_a": 14, "building_b": 14, "wall_c": 0, "wall_d": 20,
	"pillar_e": 8, "step_f1": 0, "step_f2": 0, "step_f3": 4,
	"block_1": 0, "block_2": 0, "block_3": 0,
}
## 四面都有窗的 4 栋（四个面宽度都 ≥ 2.0 m）
const FOUR_FACE_BUILDINGS := ["building_a", "building_b", "pillar_e", "step_f3"]
## OBSTACLES 逐位快照（§三.6-6：零改动）；origin / size 为世界坐标（Z-up，米）
const OBSTACLE_TABLE := [
	{"name": "wall_south", "origin": Vector3(0.0, 0.0, 0.0), "size": Vector3(128.0, 0.5, 2.0)},
	{"name": "wall_north", "origin": Vector3(0.0, 127.5, 0.0), "size": Vector3(128.0, 0.5, 2.0)},
	{"name": "wall_west", "origin": Vector3(0.0, 0.0, 0.0), "size": Vector3(0.5, 128.0, 2.0)},
	{"name": "wall_east", "origin": Vector3(127.5, 0.0, 0.0), "size": Vector3(0.5, 128.0, 2.0)},
	{"name": "building_a", "origin": Vector3(24.0, 24.0, 0.0), "size": Vector3(10.0, 8.0, 4.0)},
	{"name": "building_b", "origin": Vector3(88.0, 28.0, 0.0), "size": Vector3(8.0, 10.0, 5.0)},
	{"name": "wall_c", "origin": Vector3(36.0, 92.0, 0.0), "size": Vector3(30.0, 1.0, 2.0)},
	{"name": "wall_d", "origin": Vector3(34.0, 100.0, 0.0), "size": Vector3(1.0, 24.0, 3.0)},
	{"name": "pillar_e", "origin": Vector3(78.0, 78.0, 0.0), "size": Vector3(2.0, 2.0, 8.0)},
	{"name": "step_f1", "origin": Vector3(102.0, 100.0, 0.0), "size": Vector3(8.0, 8.0, 1.0)},
	{"name": "step_f2", "origin": Vector3(103.0, 101.0, 0.0), "size": Vector3(6.0, 6.0, 2.0)},
	{"name": "step_f3", "origin": Vector3(104.0, 102.0, 0.0), "size": Vector3(4.0, 4.0, 3.0)},
	{"name": "block_1", "origin": Vector3(60.0, 100.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
	{"name": "block_2", "origin": Vector3(66.0, 100.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
	{"name": "block_3", "origin": Vector3(72.0, 104.0, 0.0), "size": Vector3(1.0, 1.0, 1.0)},
]
## 尺度 / 碰撞层快照（§三.6-6）
const SETTING_VOXEL_SIZE := "sim_world/world/voxel_size"
const SETTING_CELLS_X := "sim_world/world/cells_x"
const SETTING_CELLS_Y := "sim_world/world/cells_y"
const SETTING_LAYERS := "sim_world/world/layers"
const WANT_VOXEL_SIZE := 0.5
const WANT_CELLS_X := 256
const WANT_CELLS_Y := 256
const WANT_LAYERS := 64
const WANT_GROUND_LAYER := 1
const WANT_VOXEL_LAYER := 2
const WANT_BOUNDS_SHAPES := 5

## 几何容差
const GEO_TOL := 1e-3
const AABB_TOL := 1e-4

## R11/R12：窗暗度与"无斑驳"
const WINDOW_DARK_MAX := 0.25
const WINDOW_DARK_RATIO := 0.8
const WINDOW_MIN_ONSCREEN := 6
## 只统计屏幕尺寸 ≥ 12 px 的窗：更远的窗 3×3 采样会落到墙面/背景上（实测 70 m 外的窗只有几像素，
## 采到的是墙皮 ⇒ 把"窗暗度"测成墙暗度）。这是采样有效性下限，不是阈值放宽。
const WINDOW_MIN_PX := 12.0
const WINDOW_CLEAN_MAX_DEV := 0.12
const WINDOW_CLEAN_RATIO := 0.10
## R13：斑马线白条 vs 间隔的亮度差
const CROSSWALK_CONTRAST_MIN := 0.10
const CROSSWALK_MIN_SAMPLES := 100

var _total: int = 0
var _failed: int = 0
var _skipped: int = 0
var _world: Node3D = null
var _cam: Node3D = null
var _holder: Node3D = null
var _shots: Array = []
var _imgs: Array = []


func _initialize() -> void:
	_run()


func _run() -> void:
	# ① 载 World（入树前关掉 WebSocketManager.autolisten，照 check_world_look.gd:116-119）
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	if not _check("scene-loads", packed != null, WORLD_SCENE):
		_finish()
		return
	var node: Node = packed.instantiate()
	_world = node as Node3D
	if not _check("world-instance", _world != null):
		_finish()
		return
	var ws: Node = _world.get_node_or_null("Systems/WebSocketManager")
	if ws != null:
		ws.set("autolisten", false)          # 不监听 9999、不连 WS
	root.add_child(_world)
	await process_frame
	await process_frame

	# ② S 组其余 26 项（前 2 项 scene-loads / world-instance 已在上面的载入段断言）
	_Run_S_Group()

	# ③ R 组：13 项（仅真渲染器；headless 打印 [skip] 且不计入 total）
	var can_capture: bool = await _Cam_Can_Capture()
	if can_capture:
		await _Run_R13()
	else:
		_Skip_R13()

	# ④ 释放 + 退出（否则 "resources still in use at exit" 变成 ERROR）
	await _Teardown()
	_finish()


# ================================================================ S 组：结构 28 项

func _Run_S_Group() -> void:
	var street: Node3D = _world.Get_Street_Root()
	var facade: Node3D = _world.Get_Facade_Root()

	# ---- S3 街道容器
	_check("street-root", street != null and street.name == "Street",
		"node=%s name=%s" % [str(street != null), street.name if street != null else "null"])

	# ---- S4 街道只有 3 个 MeshInstance3D
	var names: Array[String] = []
	var all_mi := true
	if street != null:
		for child: Node in street.get_children():
			if child is MeshInstance3D:
				names.append(String(child.name))
			else:
				all_mi = false
	names.sort()
	var want_names: Array[String] = ["Curbs", "Markings", "Road"]
	_check("street-nodes", street != null and street.get_child_count() == 3 and all_mi
		and names == want_names,
		"children=%d（%d 个是 MeshInstance3D）names=%s 期望=%s" % [
			street.get_child_count() if street != null else -1, names.size(),
			str(names), str(want_names)])

	var road: MeshInstance3D = street.get_node_or_null("Road") as MeshInstance3D \
		if street != null else null
	var marks: MeshInstance3D = street.get_node_or_null("Markings") as MeshInstance3D \
		if street != null else null
	var curbs: MeshInstance3D = street.get_node_or_null("Curbs") as MeshInstance3D \
		if street != null else null
	var road_quads: Array = _Quads_Of(road)
	var mark_quads: Array = _Quads_Of(marks)
	var curb_quads: Array = _Quads_Of(curbs)

	# ---- S5 街道零碰撞
	var zero: Dictionary = _Collision_Count(street)
	var layers_ok := true
	var layer_detail: Array[String] = []
	for mi: MeshInstance3D in [road, marks, curbs]:
		if mi == null:
			layers_ok = false
			layer_detail.append("null")
			continue
		var raw: Variant = mi.get("collision_layer")
		var ok_mi: bool = raw == null or int(raw) == 0
		layers_ok = layers_ok and ok_mi
		layer_detail.append("%s.collision_layer=%s" % [mi.name, "null" if raw == null else str(raw)])
	_check("street-zero-collision",
		int(zero["objects"]) == 0 and int(zero["shapes"]) == 0 and layers_ok,
		"CollisionObject3D=%d CollisionShape3D=%d %s" % [
			int(zero["objects"]), int(zero["shapes"]), "；".join(layer_detail)])

	# ---- S6 主路 AABB (120, 8)
	var r0: Rect2 = _Rect_Of(road_quads[0]) if road_quads.size() > 0 else Rect2()
	_check("road-main-aabb", road_quads.size() > 0
		and absf(r0.size.x - ROAD_MAIN.size.x) < AABB_TOL
		and absf(r0.size.y - ROAD_MAIN.size.y) < AABB_TOL
		and absf(r0.position.x - ROAD_MAIN.position.x) < AABB_TOL
		and absf(r0.position.y - ROAD_MAIN.position.y) < AABB_TOL,
		"quad0=%s 期望=%s" % [str(r0), str(ROAD_MAIN)])

	# ---- S7 支路 AABB (8, 24)
	var r1: Rect2 = _Rect_Of(road_quads[1]) if road_quads.size() > 1 else Rect2()
	_check("road-branch-aabb", road_quads.size() > 1
		and absf(r1.size.x - ROAD_BRANCH.size.x) < AABB_TOL
		and absf(r1.size.y - ROAD_BRANCH.size.y) < AABB_TOL
		and absf(r1.position.x - ROAD_BRANCH.position.x) < AABB_TOL
		and absf(r1.position.y - ROAD_BRANCH.position.y) < AABB_TOL,
		"quad1=%s 期望=%s" % [str(r1), str(ROAD_BRANCH)])

	# ---- S8 路面顶 z=0.01（且只有 2 个四边形、法线朝上 ⇒ 绕序没错、不会被背面剔除）
	var z_ok := true
	var z_min := 1e9
	var z_max := -1e9
	var n_up := true
	for quad: Dictionary in road_quads:
		var mn: Vector3 = quad["min"]
		var mx: Vector3 = quad["max"]
		z_min = minf(z_min, mn.z)
		z_max = maxf(z_max, mx.z)
		if absf(mn.z - ROAD_TOP_Z) > ROAD_TOP_TOL or absf(mx.z - ROAD_TOP_Z) > ROAD_TOP_TOL:
			z_ok = false
		if Vector3(quad["normal"]).z < 0.99:
			n_up = false
	_check("road-top-z", road_quads.size() == 2 and z_ok and n_up,
		"quads=%d z∈[%.4f, %.4f]（期望 %.2f±%.3f）法线朝上=%s" % [
			road_quads.size(), z_min, z_max, ROAD_TOP_Z, ROAD_TOP_TOL, str(n_up)])

	# ---- S9 主路中心虚线 18（§三.6-3 的 20±2 ⇒ [18,22]）
	var dashes_main: Array = _Select(mark_quads, Vector2(DASH_LEN, DASH_W))
	var dm_ok: bool = dashes_main.size() == 18 \
		and dashes_main.size() >= 18 and dashes_main.size() <= 22
	var dm_detail: Array[String] = []
	for quad: Dictionary in dashes_main:
		var c: Vector3 = quad["center"]
		if absf(c.y - MAIN_CENTER_Y) > GEO_TOL:
			dm_ok = false
		dm_detail.append("%.2f" % c.x)
	_check("dashes-main", dm_ok, "count=%d（判据 18 且 ∈[18,22]）中心 y=%.2f x=%s" % [
		dashes_main.size(), MAIN_CENTER_Y, ", ".join(dm_detail)])

	# ---- S10 主路边线 2 条（120 × 0.15，中心 y = 60.30 / 67.70）
	var edges: Array = _Select(mark_quads, Vector2(EDGE_LINE_LEN, DASH_W))
	var edge_y: Array[float] = []
	for quad: Dictionary in edges:
		edge_y.append(Vector3(quad["center"]).y)
	edge_y.sort()
	var e_ok: bool = edges.size() == 2 and edge_y.size() == 2 \
		and absf(edge_y[0] - EDGE_LINE_Y[0]) < 0.01 and absf(edge_y[1] - EDGE_LINE_Y[1]) < 0.01
	_check("edge-lines", e_ok, "count=%d 中心 y=%s 期望=%s" % [
		edges.size(), str(edge_y), str(EDGE_LINE_Y)])

	# ---- S11 支路中心虚线 4（§三.6-3 的 4±1 ⇒ [3,5]）
	var dashes_branch: Array = _Select(mark_quads, Vector2(DASH_W, DASH_LEN))
	var db_ok: bool = dashes_branch.size() == 4 \
		and dashes_branch.size() >= 3 and dashes_branch.size() <= 5
	var db_x: Array[float] = []
	for quad: Dictionary in dashes_branch:
		var c2: Vector3 = quad["center"]
		db_x.append(c2.x)
		if absf(c2.x - BRANCH_CENTER_X) > GEO_TOL:
			db_ok = false
	_check("dashes-branch", db_ok, "count=%d（判据 4 且 ∈[3,5]）中心 x=%s 期望=%.2f" % [
		dashes_branch.size(), str(db_x), BRANCH_CENTER_X])

	# ---- S12 斑马线 4 条（0.5 × 8.0，x 起点 56/57/58/59）
	var bars: Array = _Select(mark_quads, Vector2(CROSSWALK_BAR_W, CROSSWALK_D))
	var bar_x: Array[float] = []
	for quad: Dictionary in bars:
		bar_x.append(Vector3(quad["min"]).x)
	bar_x.sort()
	var bars_ok: bool = bars.size() == 4 and bar_x.size() == 4
	for i: int in range(mini(bar_x.size(), 4)):
		if absf(bar_x[i] - (CROSSWALK_X0 + float(i))) > 0.01:
			bars_ok = false
	for quad: Dictionary in bars:
		var mn2: Vector3 = quad["min"]
		var mx2: Vector3 = quad["max"]
		if absf(mn2.y - CROSSWALK_Y0) > GEO_TOL or absf(mx2.y - (CROSSWALK_Y0 + CROSSWALK_D)) > GEO_TOL:
			bars_ok = false
	_check("crosswalk-bars", bars_ok, "count=%d（期望 4）x 起点=%s 期望=[56,57,58,59] y∈[%.1f,%.1f]" % [
		bars.size(), str(bar_x), CROSSWALK_Y0, CROSSWALK_Y0 + CROSSWALK_D])

	# ---- S13 28 个标线全部落在两条路面矩形内（无一落到地面）
	var out_road: Array[String] = []
	for quad: Dictionary in mark_quads:
		var r: Rect2 = _Rect_Of(quad)
		if not _Rect_Inside(r, ROAD_MAIN, 1e-6) and not _Rect_Inside(r, ROAD_BRANCH, 1e-6):
			out_road.append(str(r))
	_check("markings-inside-road", mark_quads.size() == 28 and out_road.is_empty(),
		"markings=%d（期望 28）落到地面的=%d %s" % [
			mark_quads.size(), out_road.size(), str(out_road)])

	# ---- S14 主虚线跳过路口 / 斑马线；标线不压路缘
	var in_skip: Array[String] = []
	for quad: Dictionary in dashes_main:
		var mn3: Vector3 = quad["min"]
		var mx3: Vector3 = quad["max"]
		for skip: Vector2 in MAIN_SKIP_X:
			if mx3.x > skip.x + GEO_TOL and mn3.x < skip.y - GEO_TOL:
				in_skip.append("%.2f~%.2f 撞 [%.0f,%.0f]" % [mn3.x, mx3.x, skip.x, skip.y])
	var on_curb: Array[String] = []
	for quad: Dictionary in mark_quads:
		var rm: Rect2 = _Rect_Of(quad)
		for crect: Rect2 in CURB_RECTS:
			if _Rect_Overlap(rm, crect, GEO_TOL):
				on_curb.append("%s ∩ %s" % [str(rm), str(crect)])
	# 边线 x∈[56,60] 与斑马线白条同色同材质共面重叠 = 显式允许项（像素输出完全相同）
	var coplanar := 0
	for quad: Dictionary in edges:
		var re: Rect2 = _Rect_Of(quad)
		for quad2: Dictionary in bars:
			if _Rect_Overlap(re, _Rect_Of(quad2), GEO_TOL):
				coplanar += 1
	_check("markings-skip", in_skip.is_empty() and on_curb.is_empty(),
		"撞跳过段=%d %s；压路缘=%d %s；允许项：边线×斑马线共面重叠 %d 处（同色，无可见伪影）" % [
			in_skip.size(), str(in_skip), on_curb.size(), str(on_curb), coplanar])

	# ---- S15 路缘 7 段 × 5 面 = 35（每段 1 个顶面在 z≈0.15 且覆盖该段矩形）
	var seg_counts: Array[int] = []
	var seg_detail: Array[String] = []
	var seg_ok: bool = curb_quads.size() == 35
	for i: int in range(CURB_RECTS.size()):
		var crect2: Rect2 = CURB_RECTS[i]
		var grown: Rect2 = crect2.grow(GEO_TOL)
		var group: Array = []
		for quad: Dictionary in curb_quads:
			if _Rect_Inside(_Rect_Of(quad), grown, 0.0):
				group.append(quad)
		seg_counts.append(group.size())
		var tops: int = 0
		for quad: Dictionary in group:
			if Vector3(quad["normal"]).z > 0.99 and absf(Vector3(quad["center"]).z - CURB_H) < GEO_TOL \
				and _Rect_Inside(crect2, _Rect_Of(quad), GEO_TOL):
				tops += 1
		if group.size() != 5 or tops != 1:
			seg_ok = false
		seg_detail.append("段%d=%d面/顶面%d" % [i, group.size(), tops])
	_check("curb-segments", seg_ok, "总面数=%d（期望 35）%s" % [
		curb_quads.size(), " ".join(seg_detail)])

	# ---- S16 路缘断口：斑马线 [56,60] 与支路口 [44,52] 处覆盖率为 0，其余为 1
	var tops_rects: Array[Rect2] = []
	for quad: Dictionary in curb_quads:
		if Vector3(quad["normal"]).z > 0.99 and absf(Vector3(quad["center"]).z - CURB_H) < GEO_TOL:
			tops_rects.append(_Rect_Of(quad))
	var break_bad: Array[String] = []
	var south_hits := 0
	var north_hits := 0
	var x: float = 4.05
	while x <= 123.95 + 1e-6:
		var cov_s: int = _Coverage(tops_rects, x, 60.1)
		var want_s: int = 0 if (x > CROSSWALK_X0 + 0.01 and x < CROSSWALK_X0 + CROSSWALK_W - 0.01) else 1
		if cov_s != want_s:
			break_bad.append("y=60.1 x=%.2f 覆盖=%d 期望=%d" % [x, cov_s, want_s])
		south_hits += 1
		var cov_n: int = _Coverage(tops_rects, x, 67.9)
		var want_n: int = 0
		if not (x > 44.01 and x < 51.99) and not (x > CROSSWALK_X0 + 0.01 and x < 59.99):
			want_n = 1
		if cov_n != want_n:
			break_bad.append("y=67.9 x=%.2f 覆盖=%d 期望=%d" % [x, cov_n, want_n])
		north_hits += 1
		x += 0.1
	_check("curb-breaks", break_bad.is_empty() and south_hits > 1000 and north_hits > 1000,
		"南线探针=%d 北线探针=%d 不符=%d %s" % [
			south_hits, north_hits, break_bad.size(),
			str(break_bad.slice(0, mini(6, break_bad.size())))])

	# ---- S17 路缘立面剖面：顶点 z∈[0,0.15]，顶面恰好 z=0.15，**无底面**
	var cz_min := 1e9
	var cz_max := -1e9
	var cprof_ok := true
	var down_faces := 0
	var top_faces := 0
	for quad: Dictionary in curb_quads:
		var mn4: Vector3 = quad["min"]
		var mx4: Vector3 = quad["max"]
		cz_min = minf(cz_min, mn4.z)
		cz_max = maxf(cz_max, mx4.z)
		if mn4.z < -GEO_TOL or mx4.z > CURB_H + GEO_TOL:
			cprof_ok = false
		var nz: float = Vector3(quad["normal"]).z
		if nz < -0.5:
			down_faces += 1
		if nz > 0.99 and absf(mx4.z - CURB_H) < GEO_TOL:
			top_faces += 1
	_check("curb-profile", cprof_ok and down_faces == 0 and top_faces == 7,
		"z∈[%.4f, %.4f]（期望 [0, %.2f]）顶面=%d（期望 7）朝下底面=%d（期望 0）" % [
			cz_min, cz_max, CURB_H, top_faces, down_faces])

	# ---- S18 立面只有 3 个 MeshInstance3D
	var fnames: Array[String] = []
	var fall_mi := true
	if facade != null:
		for child: Node in facade.get_children():
			if child is MeshInstance3D:
				fnames.append(String(child.name))
			else:
				fall_mi = false
	fnames.sort()
	var want_fnames: Array[String] = ["Caps", "Plinths", "Windows"]
	_check("facade-root", facade != null and facade.name == "Facade"
		and facade.get_child_count() == 3 and fall_mi and fnames == want_fnames,
		"node=%s name=%s children=%d names=%s 期望=%s" % [
			str(facade != null), facade.name if facade != null else "null",
			facade.get_child_count() if facade != null else -1, str(fnames), str(want_fnames)])

	var windows: MeshInstance3D = facade.get_node_or_null("Windows") as MeshInstance3D \
		if facade != null else null
	var plinths: MeshInstance3D = facade.get_node_or_null("Plinths") as MeshInstance3D \
		if facade != null else null
	var caps: MeshInstance3D = facade.get_node_or_null("Caps") as MeshInstance3D \
		if facade != null else null
	var win_quads: Array = _Quads_Of(windows)
	var plinth_quads: Array = _Quads_Of(plinths)
	var cap_quads: Array = _Quads_Of(caps)

	# ---- S19 立面零碰撞
	var fzero: Dictionary = _Collision_Count(facade)
	var flayers_ok := true
	var flayer_detail: Array[String] = []
	for mi2: MeshInstance3D in [windows, plinths, caps]:
		if mi2 == null:
			flayers_ok = false
			flayer_detail.append("null")
			continue
		var raw2: Variant = mi2.get("collision_layer")
		var ok2: bool = raw2 == null or int(raw2) == 0
		flayers_ok = flayers_ok and ok2
		flayer_detail.append("%s.collision_layer=%s" % [mi2.name, "null" if raw2 == null else str(raw2)])
	_check("facade-zero-collision",
		int(fzero["objects"]) == 0 and int(fzero["shapes"]) == 0 and flayers_ok,
		"CollisionObject3D=%d CollisionShape3D=%d %s" % [
			int(fzero["objects"]), int(fzero["shapes"]), "；".join(flayer_detail)])

	# ---- S20 窗总数 60（>0）
	_check("window-total", win_quads.size() == 60 and win_quads.size() > 0,
		"windows=%d（期望 60，判据 >0）" % win_quads.size())

	# ---- S21 逐栋窗数（按"墙面平面 ± 外扩 0.01"精确归属；嵌套退台 step_f1~f3 靠平面距离区分）
	var per_obstacle: Dictionary = {}
	var owner_missing: Array[String] = []
	for quad: Dictionary in win_quads:
		var owner: Dictionary = _Sheet_Owner(quad, OBSTACLE_TABLE, 0.0, false)
		var idx: int = int(owner["index"])
		if idx < 0:
			owner_missing.append(str(_Rect_Of(quad)))
			continue
		var oname: String = String(OBSTACLE_TABLE[idx]["name"])
		per_obstacle[oname] = int(per_obstacle.get(oname, 0)) + 1
	var s21_ok: bool = owner_missing.is_empty()
	var s21_detail: Array[String] = []
	for spec: Dictionary in OBSTACLE_TABLE:
		var nm: String = String(spec["name"])
		var got: int = int(per_obstacle.get(nm, 0))
		var want: int = int(WINDOW_EXPECT.get(nm, -1))
		if got != want:
			s21_ok = false
		s21_detail.append("%s=%d(期望 %d)" % [nm, got, want])
	_check("window-per-obstacle", s21_ok,
		"未归属=%d %s；%s" % [owner_missing.size(), str(owner_missing), " ".join(s21_detail)])

	# ---- S22 逐栋逐面窗数（自检独立复算公式）+ 四栋四面都有窗
	var face_actual: Dictionary = {}
	for quad: Dictionary in win_quads:
		var owner2: Dictionary = _Sheet_Owner(quad, OBSTACLE_TABLE, 0.0, false)
		var idx2: int = int(owner2["index"])
		if idx2 < 0:
			continue
		var key: String = "%s/%s" % [String(OBSTACLE_TABLE[idx2]["name"]), String(owner2["face"])]
		face_actual[key] = int(face_actual.get(key, 0)) + 1
	var s22_ok: bool = true
	var s22_detail: Array[String] = []
	for spec2: Dictionary in OBSTACLE_TABLE:
		var nm2: String = String(spec2["name"])
		var want_faces: Dictionary = _Expected_Faces(spec2)
		for face: String in ["x_neg", "x_pos", "y_neg", "y_pos"]:
			var got2: int = int(face_actual.get("%s/%s" % [nm2, face], 0))
			var want2: int = int(want_faces[face])
			if got2 != want2:
				s22_ok = false
				s22_detail.append("%s/%s=%d(期望 %d)" % [nm2, face, got2, want2])
		if FOUR_FACE_BUILDINGS.has(nm2):
			for face2: String in ["x_neg", "x_pos", "y_neg", "y_pos"]:
				if int(face_actual.get("%s/%s" % [nm2, face2], 0)) < 1:
					s22_ok = false
					s22_detail.append("%s/%s 无窗（四栋四面都必须有）" % [nm2, face2])
	# wall_d 两个 1 m 端面几何上放不下 1.8 m 窗 ⇒ 必须为 0
	for face3: String in ["y_neg", "y_pos"]:
		if int(face_actual.get("wall_d/%s" % face3, 0)) != 0:
			s22_ok = false
			s22_detail.append("wall_d/%s 应为 0（1 m 端面）" % face3)
	_check("window-four-walls", s22_ok,
		"不符=%d %s" % [s22_detail.size(), str(s22_detail)])

	# ---- S23 基座：44 面，z∈[0,0.6]，距墙面 0.01 且朝外
	var s23_ok: bool = plinth_quads.size() == 44
	var s23_detail: Array[String] = []
	for quad: Dictionary in plinth_quads:
		var mn5: Vector3 = quad["min"]
		var mx5: Vector3 = quad["max"]
		if mn5.z < PLINTH_Z0 - GEO_TOL or mx5.z > PLINTH_Z1 + GEO_TOL \
			or absf(mn5.z - PLINTH_Z0) > GEO_TOL or absf(mx5.z - PLINTH_Z1) > GEO_TOL:
			s23_ok = false
			s23_detail.append("z∈[%.4f,%.4f]" % [mn5.z, mx5.z])
		var owner3: Dictionary = _Sheet_Owner(quad, OBSTACLE_TABLE, SHEET_OUT_M, true)
		if int(owner3["index"]) < 0:
			s23_ok = false
			s23_detail.append("未归属 %s" % str(_Rect_Of(quad)))
		elif absf(float(owner3["dist"])) > GEO_TOL:
			# dist = 薄片平面 − (墙面 ± SHEET_OUT_M) ⇒ 0 表示"正好外扩 0.01 m 且朝外"
			s23_ok = false
			s23_detail.append("距墙偏差=%.4f" % float(owner3["dist"]))
	_check("plinth", s23_ok, "plinths=%d（期望 44）不符=%d %s" % [
		plinth_quads.size(), s23_detail.size(), str(s23_detail.slice(0, mini(6, s23_detail.size())))])

	# ---- S24 压顶：15 面，水平，z = h + [0.010, 0.024]，外扩 0.05，15 个 z 两两不同
	var s24_ok: bool = cap_quads.size() == 15
	var s24_detail: Array[String] = []
	var cap_z: Array[float] = []
	for quad: Dictionary in cap_quads:
		if Vector3(quad["normal"]).z < 0.99:
			s24_ok = false
			s24_detail.append("非水平")
		cap_z.append(Vector3(quad["center"]).z)
	# 15 个 z 两两不同（相邻压顶不共面）
	for i2: int in range(cap_z.size()):
		for j2: int in range(i2 + 1, cap_z.size()):
			if absf(cap_z[i2] - cap_z[j2]) < CAP_LIFT_STEP_M * 0.5:
				s24_ok = false
				s24_detail.append("z 重复 %.4f / %.4f" % [cap_z[i2], cap_z[j2]])
	var matched: Array[int] = []
	for i3: int in range(OBSTACLE_TABLE.size()):
		var spec3: Dictionary = OBSTACLE_TABLE[i3]
		var o3: Vector3 = spec3["origin"]
		var s3: Vector3 = spec3["size"]
		var want_z: float = s3.z + CAP_LIFT_M + CAP_LIFT_STEP_M * float(i3)
		var want_w: float = s3.x + 2.0 * CAP_OUT_M
		var want_d: float = s3.y + 2.0 * CAP_OUT_M
		var found := -1
		for k: int in range(cap_quads.size()):
			if matched.has(k):
				continue
			var q: Dictionary = cap_quads[k]
			var rr: Rect2 = _Rect_Of(q)
			if absf(Vector3(q["center"]).z - want_z) < GEO_TOL \
				and absf(rr.size.x - want_w) < GEO_TOL and absf(rr.size.y - want_d) < GEO_TOL \
				and absf(rr.position.x - (o3.x - CAP_OUT_M)) < GEO_TOL \
				and absf(rr.position.y - (o3.y - CAP_OUT_M)) < GEO_TOL:
				found = k
				break
		if found < 0:
			s24_ok = false
			s24_detail.append("%s 无对应压顶（z 期望 %.4f）" % [String(spec3["name"]), want_z])
		else:
			matched.append(found)
	_check("cap", s24_ok and matched.size() == 15,
		"caps=%d（期望 15）匹配=%d 不符=%d %s" % [
			cap_quads.size(), matched.size(), s24_detail.size(),
			str(s24_detail.slice(0, mini(6, s24_detail.size())))])

	# ---- S25 幂等：连调两次，节点数 / mesh 数不变，且不出现 Street2 / Facade2
	var before: Dictionary = _Structure_Counts()
	var s_root: Node3D = StreetLayout.Build_All(_world)
	var f_root: Node3D = Facade.Build_All(_world)
	var after: Dictionary = _Structure_Counts()
	var idem_ok: bool = before["street_children"] == after["street_children"] \
		and before["facade_children"] == after["facade_children"] \
		and before["street_mesh"] == after["street_mesh"] \
		and before["facade_mesh"] == after["facade_mesh"] \
		and int(after["street_named"]) == 1 and int(after["facade_named"]) == 1 \
		and _world.get_node_or_null("Street2") == null \
		and _world.get_node_or_null("Facade2") == null \
		and s_root != null and f_root != null
	_check("idempotent", idem_ok,
		"street 子节点 %d→%d、mesh %d→%d；facade 子节点 %d→%d、mesh %d→%d；同名节点 Street=%d Facade=%d；Street2=%s Facade2=%s" % [
			int(before["street_children"]), int(after["street_children"]),
			int(before["street_mesh"]), int(after["street_mesh"]),
			int(before["facade_children"]), int(after["facade_children"]),
			int(before["facade_mesh"]), int(after["facade_mesh"]),
			int(after["street_named"]), int(after["facade_named"]),
			str(_world.get_node_or_null("Street2") != null),
			str(_world.get_node_or_null("Facade2") != null)])

	# ---- S26 OBSTACLES 逐位未变
	var live: Array = World.OBSTACLES
	var s26_ok: bool = live.size() == OBSTACLE_TABLE.size()
	var s26_detail: Array[String] = []
	for i4: int in range(mini(live.size(), OBSTACLE_TABLE.size())):
		var a: Dictionary = live[i4]
		var b: Dictionary = OBSTACLE_TABLE[i4]
		if String(a["name"]) != String(b["name"]) \
			or Vector3(a["origin"]) != Vector3(b["origin"]) \
			or Vector3(a["size"]) != Vector3(b["size"]):
			s26_ok = false
			s26_detail.append("#%d %s" % [i4, String(a["name"])])
	_check("obstacles-unchanged", s26_ok, "count=%d（期望 %d）不符=%s" % [
		live.size(), OBSTACLE_TABLE.size(), str(s26_detail)])

	# ---- S27 尺度 / 碰撞层未变
	var vs: float = float(ProjectSettings.get_setting(SETTING_VOXEL_SIZE, -1.0))
	var cx: int = int(ProjectSettings.get_setting(SETTING_CELLS_X, -1))
	var cy: int = int(ProjectSettings.get_setting(SETTING_CELLS_Y, -1))
	var ly: int = int(ProjectSettings.get_setting(SETTING_LAYERS, -1))
	var ground: StaticBody3D = _world.get_node_or_null("Ground") as StaticBody3D
	var voxel: GridMap = _world.get_node_or_null("VoxelWorld") as GridMap
	var bounds: StaticBody3D = _world.get_node_or_null("Bounds") as StaticBody3D
	var bounds_shapes: int = 0
	if bounds != null:
		for child2: Node in bounds.get_children():
			if child2 is CollisionShape3D:
				bounds_shapes += 1
	var s27_ok: bool = absf(vs - WANT_VOXEL_SIZE) < 1e-6 and cx == WANT_CELLS_X \
		and cy == WANT_CELLS_Y and ly == WANT_LAYERS \
		and ground != null and ground.collision_layer == WANT_GROUND_LAYER \
		and voxel != null and voxel.collision_layer == WANT_VOXEL_LAYER \
		and bounds != null and bounds.collision_layer == WANT_GROUND_LAYER \
		and bounds_shapes == WANT_BOUNDS_SHAPES
	_check("scale-layers-unchanged", s27_ok,
		"voxel=%.2f cells=%dx%dx%d；Ground.layer=%d VoxelWorld.layer=%d Bounds.layer=%d shapes=%d" % [
			vs, cx, cy, ly,
			ground.collision_layer if ground != null else -1,
			voxel.collision_layer if voxel != null else -1,
			bounds.collision_layer if bounds != null else -1, bounds_shapes])

	# ---- S28 地面观感（Step 1b）未被本步破坏
	var gmi: MeshInstance3D = _world.get_node_or_null("Ground/MeshInstance3D") as MeshInstance3D
	var gmat: StandardMaterial3D = null
	if gmi != null:
		gmat = gmi.get_surface_override_material(0) as StandardMaterial3D
	_check("ground-look-intact", gmat != null and gmat.albedo_texture != null,
		"override=%s albedo=%s" % [
			gmat.get_class() if gmat != null else "null",
			gmat.albedo_texture.resource_path if gmat != null and gmat.albedo_texture != null else "null"])


# ================================================================ R 组：真渲染 13 项

## 先闸后 await（E26-③）：先建机位拿到 CameraSensor，再问 Can_Capture()。
func _Cam_Can_Capture() -> bool:
	var shot: Dictionary = SHOTS[0]
	_Make_Holder(shot["eye"], shot["aim"])
	await process_frame
	await process_frame
	return _cam != null and bool(_cam.Can_Capture())


func _Run_R13() -> void:
	_Ensure_Out_Dir()
	for i: int in range(SHOTS.size()):
		await _Capture_Shot(i)
		# **必须**在刚抓完这一张时投影：CameraSensor 只在 Capture_Jpeg() 里 _Sync_View_Transform，
		# 光挪 holder 不会同步镜头（挪了反而用上一张的机位去投影，实测斑马线对比度会假红）。
		if i == 0:
			await _Check_Crosswalk()
		elif i == 1:
			await _Check_Windows()
	for i: int in range(SHOTS.size()):
		var labels: Array[String] = _Shot_Labels(i)
		_Check_Shot_File(i, labels[0])
		_Check_Shot_Size(i, labels[1])
		_Check_Shot_Pixels(i, labels[2])

	_Check_Pairs()


## 单机位：移 holder -> Configure -> 等 2 帧 -> 抓拍落盘。
func _Capture_Shot(idx: int) -> void:
	var shot: Dictionary = SHOTS[idx]
	var path: String = "%s/%s" % [OUT_DIR, String(shot["file"])]
	_Place_Holder(shot["eye"], shot["aim"])
	_cam.Configure({"width": SHOT_SIZE.x, "height": SHOT_SIZE.y,
		"fov_deg": SHOT_FOV, "quality": SHOT_QUALITY})
	await process_frame
	await process_frame
	var saved: bool = await _cam.Capture_To_File(path)
	_shots.append({
		"id": int(shot["id"]),
		"path": path,
		"saved": saved,
		"bytes": _File_Size(path),
		"jpg": FileAccess.get_file_as_bytes(path),
	})
	_imgs.append(_Decode(FileAccess.get_file_as_bytes(path)))
	print("[check] capture shot-%02d saved=%s cam=%s" % [int(shot["id"]), str(saved), path])


func _Check_Shot_File(idx: int, label: String) -> void:
	var data: Dictionary = _shots[idx]
	var path: String = String(data["path"])
	var bytes: int = int(data["bytes"])
	var jpg: PackedByteArray = data["jpg"]
	var head: String = jpg.slice(0, 3).hex_encode() if jpg.size() >= 3 else ""
	_check(label, bool(data["saved"]) and bytes > MIN_BYTES and head == "ffd8ff",
		"bytes=%d(>%d) head=%s saved=%s path=%s" % [
			bytes, MIN_BYTES, head, str(data["saved"]), path])


func _Check_Shot_Size(idx: int, label: String) -> void:
	var img: Image = _Decode(_shots[idx]["jpg"])
	var size: Vector2i = img.get_size() if img != null and not img.is_empty() else Vector2i(-1, -1)
	_check(label, size == SHOT_SIZE, "decoded=%dx%d(期望 %dx%d)" % [
		size.x, size.y, SHOT_SIZE.x, SHOT_SIZE.y])


func _Check_Shot_Pixels(idx: int, label: String) -> void:
	var img: Image = _Decode(_shots[idx]["jpg"])
	if img == null or img.is_empty():
		_check(label, false, "JPEG 解码失败：%s" % String(_shots[idx]["path"]))
		return
	var st: Dictionary = _Gray_Stats(img)
	var mean: float = float(st["mean"])
	var variance: float = float(st["var"])
	var spread: float = float(st["max"]) - float(st["min"])
	var white: float = float(st["white"])
	_check(label, variance > VAR_MIN and mean > MEAN_MIN and mean < MEAN_MAX
		and spread > SPREAD_MIN and white < WHITE_MAX,
		"var=%.6f(>%.3f) mean=%.4f(%.2f~%.2f) spread=%.3f(>%.2f) white=%.3f(<%.2f)" % [
			variance, VAR_MIN, mean, MEAN_MIN, MEAN_MAX, spread, SPREAD_MIN,
			white, WHITE_MAX])


## 三张两两不同（字节差异 + 灰度差异都要过）。
func _Check_Pairs() -> void:
	var pairs: Array = [[0, 1], [0, 2], [1, 2]]
	var details: Array[String] = []
	var ok: bool = _shots.size() == SHOTS.size()
	for pair: Array in pairs:
		var a: PackedByteArray = _shots[pair[0]]["jpg"]
		var b: PackedByteArray = _shots[pair[1]]["jpg"]
		var diff: Dictionary = _Pair_Diff(a, b)
		var byte_ratio: float = float(diff["byte_ratio"])
		var mean_abs: float = float(diff["mean_abs"])
		if byte_ratio <= DIFF_BYTE_MIN or mean_abs <= DIFF_MEAN_MIN:
			ok = false
		details.append("%d~%d byte=%.4f(>%.2f) mean_abs=%.5f(>%.2f)" % [
			pair[0], pair[1], byte_ratio, DIFF_BYTE_MIN, mean_abs, DIFF_MEAN_MIN])
	_check("shots-distinct", ok, "；".join(details))


## R11/R12：把 `Windows` 的四边形按 shot-01 的相机投影上屏，3×3 内插采样。
## 判据 ① 完全在屏内的窗四边形 ≥ 6；② 平均亮度 < 0.25 的占比 ≥ 0.8 且聚合均值 < 0.25（窗真是暗玻璃）；
##      ③ 单个窗内 |lum − 窗均值| > 0.12 的采样占比 < 0.10（窗面没有与墙体互穿的斑驳 = 无 z-fighting）。
func _Check_Windows() -> void:
	var facade: Node3D = _world.Get_Facade_Root()
	var windows: MeshInstance3D = facade.get_node_or_null("Windows") as MeshInstance3D \
		if facade != null else null
	var quads: Array = _Quads_Of(windows)
	var img: Image = _imgs[1] if _imgs.size() > 1 else null
	var lens: Camera3D = _Camera_Lens()
	# 注意：quad["normal"] / quad["center"] 都是**世界坐标**，eye 也必须用世界坐标（同一坐标系才能点乘）
	var eye: Vector3 = SHOTS[1]["eye"]

	var onscreen: int = 0
	var too_small: int = 0
	var dark: int = 0
	var sum_all: float = 0.0
	var n_all: int = 0
	var dev_bad: int = 0
	var n_dev: int = 0
	var detail: Array[String] = []
	if img != null and lens != null:
		for quad: Dictionary in quads:
			# 只取**朝向相机**的窗：背面那批被自己那面墙挡住，采到的是墙皮不是窗
			var center_w: Vector3 = Vector3(quad["center"])
			if Vector3(quad["normal"]).dot(eye - center_w) <= 0.0:
				continue
			var corners: Array[Vector2] = []
			var visible: bool = true
			for p: Vector3 in quad["pts"]:
				var gp: Vector3 = WorldTransform.PosToGodot(p)
				if lens.is_position_behind(gp):
					visible = false
					break
				var sp: Vector2 = lens.unproject_position(gp)
				if sp.x < 0.0 or sp.x > float(SHOT_SIZE.x) or sp.y < 0.0 or sp.y > float(SHOT_SIZE.y):
					visible = false
					break
				corners.append(sp)
			if not visible or corners.size() != 4:
				continue
			var bbox := Rect2(corners[0], Vector2.ZERO)
			for corner: Vector2 in corners:
				bbox = bbox.expand(corner)
			if bbox.size.x < WINDOW_MIN_PX or bbox.size.y < WINDOW_MIN_PX:
				too_small += 1
				continue
			onscreen += 1
			var quad_sum: float = 0.0
			var samples: Array[float] = []
			for u: float in [0.25, 0.5, 0.75]:
				for v: float in [0.25, 0.5, 0.75]:
					var top: Vector2 = corners[0].lerp(corners[1], u)
					var bot: Vector2 = corners[3].lerp(corners[2], u)
					var sp2: Vector2 = top.lerp(bot, v)
					var lum: float = _Sample_Lum(img, sp2)
					samples.append(lum)
					quad_sum += lum
			n_all += samples.size()
			sum_all += quad_sum
			var quad_mean: float = quad_sum / float(maxi(samples.size(), 1))
			if quad_mean < WINDOW_DARK_MAX:
				dark += 1
			else:
				detail.append("窗均值=%.3f@%s(%.0fpx)" % [quad_mean,
					str(Vector3(quad["center"])), bbox.size.x])
			for lum2: float in samples:
				n_dev += 1
				if absf(lum2 - quad_mean) > WINDOW_CLEAN_MAX_DEV:
					dev_bad += 1

	var ratio: float = float(dark) / float(maxi(onscreen, 1))
	var agg: float = sum_all / float(maxi(n_all, 1))
	_check("windows-dark", onscreen >= WINDOW_MIN_ONSCREEN
		and ratio >= WINDOW_DARK_RATIO and agg < WINDOW_DARK_MAX,
		"屏内窗=%d(≥%d，另有 %d 个 <%.0fpx 不计) 暗窗占比=%.2f(≥%.2f) 聚合均值=%.4f(<%.2f) %s%s" % [
			onscreen, WINDOW_MIN_ONSCREEN, too_small, WINDOW_MIN_PX,
			ratio, WINDOW_DARK_RATIO, agg, WINDOW_DARK_MAX,
			str(detail.slice(0, mini(4, detail.size()))), "" if img != null else "影像是空"])
	var dev_ratio: float = float(dev_bad) / float(maxi(n_dev, 1))
	_check("window-clean", n_dev > 0 and dev_ratio < WINDOW_CLEAN_RATIO,
		"采样=%d 偏窗均值>%.2f 的占比=%.4f(<%.2f)" % [
			n_dev, WINDOW_CLEAN_MAX_DEV, dev_ratio, WINDOW_CLEAN_RATIO])


## R13：斑马线 4 条白条 vs 3 条间隔的亮度差（同一张 shot-00，按 MARK_TOP_Z 投影）。
func _Check_Crosswalk() -> void:
	var img: Image = _imgs[0] if _imgs.size() > 0 else null
	var lens: Camera3D = _Camera_Lens()
	var bars: Array[float] = []
	var gaps: Array[float] = []
	if img != null and lens != null:
		# 白条：4 条各 0.5 m 宽，横跨 y∈[60,68]；采样避开边线所在的 y≈60.3 / 67.7
		for i: int in range(4):
			for u: float in [0.15, 0.5, 0.85]:
				var px: float = CROSSWALK_X0 + float(i) + 0.5 * u
				for k: int in range(30):
					var py: float = 61.2 + 5.6 * (float(k) / 29.0)
					var lum: float = _Project_Lum(img, lens, Vector3(px, py, MARK_TOP_Z))
					if lum >= 0.0:
						bars.append(lum)
		# 间隔：x∈[56.5,57] / [57.5,58] / [58.5,59]
		for g: int in range(3):
			var gx0: float = CROSSWALK_X0 + 0.5 + float(g)
			for u2: float in [0.25, 0.5, 0.75]:
				var px2: float = gx0 + 0.5 * u2
				for k2: int in range(30):
					var py2: float = 61.2 + 5.6 * (float(k2) / 29.0)
					var lum2: float = _Project_Lum(img, lens, Vector3(px2, py2, MARK_TOP_Z))
					if lum2 >= 0.0:
						gaps.append(lum2)
	var bar_mean: float = _Mean(bars)
	var gap_mean: float = _Mean(gaps)
	_check("crosswalk-contrast",
		bars.size() >= CROSSWALK_MIN_SAMPLES and gaps.size() >= CROSSWALK_MIN_SAMPLES
		and bar_mean - gap_mean >= CROSSWALK_CONTRAST_MIN,
		"白条采样=%d 间隔采样=%d 白条均值=%.4f 间隔均值=%.4f 差=%.4f(≥%.2f)" % [
			bars.size(), gaps.size(), bar_mean, gap_mean, bar_mean - gap_mean,
			CROSSWALK_CONTRAST_MIN])


## headless：R 组整体 SKIP（**不计入 total**，否则断言数和基线错位）。
func _Skip_R13() -> void:
	for label: String in _Render_Labels():
		print("[skip] %s（无渲染器：display=%s adapter='%s'）" % [
			label, DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])
		_skipped += 1


## R 组全部 label（13 项），与 _Check_Shot_* / 全局断言逐字同源，防两处漂移。
func _Render_Labels() -> Array[String]:
	var labels: Array[String] = []
	for i: int in range(SHOTS.size()):
		labels.append_array(_Shot_Labels(i))
	labels.append_array(["shots-distinct", "windows-dark", "window-clean", "crosswalk-contrast"])
	return labels


func _Shot_Labels(idx: int) -> Array[String]:
	var tag: String = "shot-%02d" % int(SHOTS[idx]["id"])
	return ["%s-file" % tag, "%s-size" % tag, "%s-pixels" % tag]


# ================================================================ 机位 / 抓拍工具

## 机位 = 世界级 holder（不挂车、不挂世界外），CameraSensor 作为其子节点。
func _Make_Holder(eye_world: Vector3, aim_world: Vector3) -> Node3D:
	var holder := Node3D.new()
	holder.name = "StreetCamera"
	_world.add_child(holder)
	_holder = holder
	_Place_Holder(eye_world, aim_world)
	var packed: PackedScene = load(CAMERA_SCENE) as PackedScene
	if packed != null:
		_cam = packed.instantiate() as Node3D
		if _cam != null:
			_cam.name = "CameraSensor"
			holder.add_child(_cam)
	return holder


## 移动既有 holder（同一台相机换机位）：位置换轴 + look_at。
func _Place_Holder(eye_world: Vector3, aim_world: Vector3) -> void:
	if _holder == null or not is_instance_valid(_holder):
		return
	_holder.global_position = WorldTransform.PosToGodot(eye_world)
	_holder.look_at(WorldTransform.PosToGodot(aim_world), Vector3.UP)


func _Camera_Lens() -> Camera3D:
	if _cam == null or not is_instance_valid(_cam):
		return null
	return _cam.get_node_or_null("Viewport/Lens") as Camera3D


## 世界点 -> 屏幕亮度（在屏外 / 在相机背后返回 -1）。
func _Project_Lum(img: Image, lens: Camera3D, world_pos: Vector3) -> float:
	var gp: Vector3 = WorldTransform.PosToGodot(world_pos)
	if lens.is_position_behind(gp):
		return -1.0
	var sp: Vector2 = lens.unproject_position(gp)
	if sp.x < 0.0 or sp.x > float(SHOT_SIZE.x) or sp.y < 0.0 or sp.y > float(SHOT_SIZE.y):
		return -1.0
	return _Sample_Lum(img, sp)


func _Sample_Lum(img: Image, sp: Vector2) -> float:
	var px: int = clampi(int(roundf(sp.x)), 0, img.get_width() - 1)
	var py: int = clampi(int(roundf(sp.y)), 0, img.get_height() - 1)
	return _Lum(img.get_pixel(px, py))


# ================================================================ 几何 / 统计工具

## 从一个 MeshInstance3D 取出全部四边形：**每四边形独立 4 顶点、不焊接** ⇒ verts.size() == 4 × quads。
## 返回 `{pts(4 个世界坐标角点), min, max, size, center, normal(世界)}`（全部量都换回世界坐标）。
func _Quads_Of(mi: MeshInstance3D) -> Array:
	var out: Array = []
	if mi == null or mi.mesh == null or mi.mesh.get_surface_count() == 0:
		return out
	var arrays: Array = mi.mesh.surface_get_arrays(0)
	var verts: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX] as PackedVector3Array
	var norms: PackedVector3Array = PackedVector3Array()
	if arrays[Mesh.ARRAY_NORMAL] != null:
		norms = arrays[Mesh.ARRAY_NORMAL] as PackedVector3Array
	var quads: int = verts.size() / 4
	for qi: int in range(quads):
		var pts: Array[Vector3] = []
		var lo := Vector3(1e9, 1e9, 1e9)
		var hi := Vector3(-1e9, -1e9, -1e9)
		for k: int in range(4):
			var w: Vector3 = WorldTransform.PosToWorld(verts[qi * 4 + k])
			pts.append(w)
			lo = Vector3(minf(lo.x, w.x), minf(lo.y, w.y), minf(lo.z, w.z))
			hi = Vector3(maxf(hi.x, w.x), maxf(hi.y, w.y), maxf(hi.z, w.z))
		var nrm := Vector3.UP
		if qi * 4 < norms.size():
			nrm = WorldTransform.PosToWorld(norms[qi * 4])
		out.append({"pts": pts, "min": lo, "max": hi, "size": hi - lo,
			"center": (lo + hi) * 0.5, "normal": nrm})
	return out


## 四边形在世界 XY 平面上的矩形（所有薄片都是轴对齐矩形）。
func _Rect_Of(quad: Dictionary) -> Rect2:
	var mn: Vector3 = quad["min"]
	var mx: Vector3 = quad["max"]
	return Rect2(mn.x, mn.y, mx.x - mn.x, mx.y - mn.y)


## 从 quads 里挑出尺寸 == want_size 的（want_y < 0 表示不看 y 方向尺寸）。
## 用于把标线按"主虚线 3×0.15 / 边线 120×0.15 / 支虚线 0.15×3 / 白条 0.5×8"分桶。
func _Select(quads: Array, want_size: Vector2) -> Array:
	var picked: Array = []
	for quad: Dictionary in quads:
		var size: Vector3 = quad["size"]
		if absf(size.x - want_size.x) > GEO_TOL or absf(size.y - want_size.y) > GEO_TOL:
			continue
		if absf(size.z) > GEO_TOL:
			continue
		picked.append(quad)
	return picked


func _Rect_Inside(inner: Rect2, outer: Rect2, tol: float) -> bool:
	return inner.position.x >= outer.position.x - tol \
		and inner.position.y >= outer.position.y - tol \
		and inner.end.x <= outer.end.x + tol \
		and inner.end.y <= outer.end.y + tol


## 正面积重叠（tol 内贴边不算重叠）。
func _Rect_Overlap(a: Rect2, b: Rect2, tol: float) -> bool:
	return a.position.x < b.end.x - tol and b.position.x < a.end.x - tol \
		and a.position.y < b.end.y - tol and b.position.y < a.end.y - tol


func _Coverage(rects: Array[Rect2], x: float, y: float) -> int:
	var n: int = 0
	for r: Rect2 in rects:
		if x >= r.position.x - 1e-6 and x <= r.end.x + 1e-6 \
			and y >= r.position.y - 1e-6 and y <= r.end.y + 1e-6:
			n += 1
	return n


## 子树里 CollisionObject3D / CollisionShape3D 的个数（§三.6-2：必须都为 0）。
func _Collision_Count(root_node: Node) -> Dictionary:
	var objects: int = 0
	var shapes: int = 0
	if root_node == null:
		return {"objects": -1, "shapes": -1}
	var stack: Array[Node] = [root_node]
	while not stack.is_empty():
		var n: Node = stack.pop_back()
		if n is CollisionObject3D:
			objects += 1
		if n is CollisionShape3D:
			shapes += 1
		for child: Node in n.get_children():
			stack.append(child)
	return {"objects": objects, "shapes": shapes}


## Street / Facade 的结构计数（幂等断言用）。
func _Structure_Counts() -> Dictionary:
	var street: Node3D = _world.get_node_or_null("Street") as Node3D
	var facade: Node3D = _world.get_node_or_null("Facade") as Node3D
	var street_named: int = 0
	var facade_named: int = 0
	for child: Node in _world.get_children():
		if String(child.name) == "Street":
			street_named += 1
		if String(child.name) == "Facade":
			facade_named += 1
	return {
		"street_children": street.get_child_count() if street != null else -1,
		"facade_children": facade.get_child_count() if facade != null else -1,
		"street_mesh": _Mesh_Count(street),
		"facade_mesh": _Mesh_Count(facade),
		"street_named": street_named,
		"facade_named": facade_named,
	}


func _Mesh_Count(root_node: Node) -> int:
	var n: int = 0
	if root_node == null:
		return -1
	var stack: Array[Node] = [root_node]
	while not stack.is_empty():
		var node: Node = stack.pop_back()
		if node is MeshInstance3D:
			n += 1
		for child: Node in node.get_children():
			stack.append(child)
	return n


## 层数（**自检独立复算** task_38 §三.3 的公式 + 一档下限 → 与 Facade.Floors_Of 同判据）。
func _Floors_Of(h: float) -> int:
	var n: int = int(floor((h - SILL_M) / FLOOR_M))
	if n == 0 and h >= SILL_M + WIN_H_M - 1e-6:
		n = 1
	return n


## 每面窗数（同上，独立复算）。
func _Bays_Of(w: float) -> int:
	if w < FACE_MIN_W_M - 1e-6:
		return 0
	return maxi(1, int(floor(w / BAY_M + 1e-6)))


## 某栋逐面期望窗数（= 层数 × 面宽能排下的 bay 数）；边界矮墙一律 0。
func _Expected_Faces(spec: Dictionary) -> Dictionary:
	var name: String = String(spec["name"])
	var size: Vector3 = spec["size"]
	if BOUNDARY_WALLS.has(name):
		return {"x_neg": 0, "x_pos": 0, "y_neg": 0, "y_pos": 0}
	var floors: int = _Floors_Of(size.z)
	var n_y: int = floors * _Bays_Of(size.y)     # ±x 面的面宽 = size.y
	var n_x: int = floors * _Bays_Of(size.x)     # ±y 面的面宽 = size.x
	return {"x_neg": n_y, "x_pos": n_y, "y_neg": n_x, "y_pos": n_x}


## 薄片归属：哪个障碍的哪一面（平面距墙面正好等于外扩量、且范围落在该面内）。
## 嵌套退台 step_f1~f3 用"平面距离"而不是"矩形包含"来区分（step_f3 的窗全在 step_f2/step_f1 的体积内）。
## 返回 `{index, face, dist}`；`index < 0` = 没归上。
func _Sheet_Owner(quad: Dictionary, obstacles: Array, u_extend: float, is_plinth: bool) -> Dictionary:
	var n: Vector3 = Vector3(quad["normal"])
	var mn: Vector3 = quad["min"]
	var mx: Vector3 = quad["max"]
	var best: Dictionary = {"index": -1, "face": "", "dist": -1.0}
	var best_area: float = 1e18
	if absf(n.z) > 0.5:
		return best                                  # 窗 / 基座都是竖面
	for i: int in range(obstacles.size()):
		var spec: Dictionary = obstacles[i]
		var o: Vector3 = spec["origin"]
		var s: Vector3 = spec["size"]
		var x0: float = o.x
		var y0: float = o.y
		var x1: float = o.x + s.x
		var y1: float = o.y + s.y
		var face := ""
		var plane := 0.0
		var want := 0.0
		var u0 := 0.0
		var u1 := 0.0
		var umin := 0.0
		var umax := 0.0
		if absf(n.x) > 0.5:
			if n.x > 0.0:
				plane = mx.x
				want = x1 + SHEET_OUT_M
				face = "x_pos"
			else:
				plane = mn.x
				want = x0 - SHEET_OUT_M
				face = "x_neg"
			u0 = y0
			u1 = y1
			umin = mn.y
			umax = mx.y
		else:
			if n.y > 0.0:
				plane = mx.y
				want = y1 + SHEET_OUT_M
				face = "y_pos"
			else:
				plane = mn.y
				want = y0 - SHEET_OUT_M
				face = "y_neg"
			u0 = x0
			u1 = x1
			umin = mn.x
			umax = mx.x
		var dist: float = plane - want
		if absf(dist) > GEO_TOL:
			continue
		if umin < u0 - u_extend - GEO_TOL or umax > u1 + u_extend + GEO_TOL:
			continue
		if is_plinth:
			if absf(mn.z - PLINTH_Z0) > GEO_TOL or absf(mx.z - PLINTH_Z1) > GEO_TOL:
				continue
		else:
			if mn.z < -GEO_TOL or mx.z > s.z + GEO_TOL:
				continue
		var area: float = s.x * s.y
		if area < best_area:
			best_area = area
			best = {"index": i, "face": face, "dist": dist}
	return best


func _Mean(values: Array[float]) -> float:
	if values.is_empty():
		return -1.0
	var sum_v: float = 0.0
	for v: float in values:
		sum_v += v
	return sum_v / float(values.size())


# ================================================================ 图像统计 / 文件工具

## 灰度统计（2 像素步长抽样）：`{mean, var, min, max, white}`。
func _Gray_Stats(img: Image) -> Dictionary:
	var sum_l: float = 0.0
	var sum2: float = 0.0
	var count: int = 0
	var lo: float = 1.0
	var hi: float = 0.0
	var white: int = 0
	for y in range(0, img.get_height(), 2):
		for x in range(0, img.get_width(), 2):
			var l: float = _Lum(img.get_pixel(x, y))
			sum_l += l
			sum2 += l * l
			lo = minf(lo, l)
			hi = maxf(hi, l)
			if l > 0.95:
				white += 1
			count += 1
	if count == 0:
		return {"mean": -1.0, "var": -1.0, "min": -1.0, "max": -1.0, "white": -1.0}
	var mean: float = sum_l / float(count)
	return {
		"mean": mean,
		"var": maxf(sum2 / float(count) - mean * mean, 0.0),
		"min": lo,
		"max": hi,
		"white": float(white) / float(count),
	}


## 两张 JPEG 的差异：`mean_abs` = 8 像素步长的灰度差均值、`byte_ratio` = 逐字节不同占比。
func _Pair_Diff(a: PackedByteArray, b: PackedByteArray) -> Dictionary:
	var img_a: Image = _Decode(a)
	var img_b: Image = _Decode(b)
	var mean_abs: float = -1.0
	if img_a != null and img_b != null and not img_a.is_empty() and not img_b.is_empty():
		var sum_l: float = 0.0
		var n: int = 0
		var h: int = mini(img_a.get_height(), img_b.get_height())
		var w: int = mini(img_a.get_width(), img_b.get_width())
		for y in range(0, h, DIFF_STEP):
			for x in range(0, w, DIFF_STEP):
				sum_l += absf(_Lum(img_a.get_pixel(x, y)) - _Lum(img_b.get_pixel(x, y)))
				n += 1
		mean_abs = sum_l / float(maxi(n, 1))
	var total: int = mini(a.size(), b.size())
	var diff: int = 0
	for i: int in range(total):
		if a[i] != b[i]:
			diff += 1
	return {"mean_abs": mean_abs, "byte_ratio": float(diff) / float(maxi(total, 1))}


func _Decode(jpg: PackedByteArray) -> Image:
	if jpg.is_empty():
		return null
	var img := Image.new()
	var err: Error = img.load_jpg_from_buffer(jpg)
	if err != OK:
		return null
	return img


func _Lum(c: Color) -> float:
	return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b


## 建落盘目录（已存在 = 成功）；失败只返回 false、**绝不打印 ERROR**。
func _Ensure_Out_Dir() -> bool:
	if OUT_DIR.is_empty() or not OUT_DIR.is_absolute_path():
		return false
	if DirAccess.dir_exists_absolute(OUT_DIR):
		return true
	DirAccess.make_dir_absolute(OUT_DIR)
	if not DirAccess.dir_exists_absolute(OUT_DIR):
		var parent: String = OUT_DIR.get_base_dir()
		if not parent.is_empty() and not DirAccess.dir_exists_absolute(parent):
			DirAccess.make_dir_absolute(parent)
		DirAccess.make_dir_absolute(OUT_DIR)
	return DirAccess.dir_exists_absolute(OUT_DIR)


func _File_Size(path: String) -> int:
	if not FileAccess.file_exists(path):
		return -1
	var handle: FileAccess = FileAccess.open(path, FileAccess.READ)
	if handle == null:
		return -2
	var size_b: int = handle.get_length()
	handle.close()
	return size_b


# ================================================================ 释放 / 断言 / 收尾

## 释放顺序：相机 -> holder -> 世界，每步 await 两帧
## （漏了会报 `ERROR: resources still in use at exit`，照 check_camera_shots.gd:944-964）。
func _Teardown() -> void:
	if _cam != null and is_instance_valid(_cam):
		_cam.queue_free()
		_cam = null
		await process_frame
		await process_frame
	if _holder != null and is_instance_valid(_holder):
		_holder.queue_free()
		_holder = null
		await process_frame
		await process_frame
	if _world != null and is_instance_valid(_world):
		_world.queue_free()
		_world = null
		await process_frame
		await process_frame


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


func _finish() -> void:
	print("[check] env display=%s adapter='%s' shots=%d" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name(), _shots.size()])
	print("[check] out_dir=%s" % OUT_DIR)
	if _skipped > 0:
		print("[check] skipped=%d（R 组：无真渲染器，不计入 total）" % _skipped)
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
