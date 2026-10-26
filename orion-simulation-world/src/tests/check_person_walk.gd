# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# 支线 walking_person / task_35 Step 3 —— 人形资产 + 世界预置行人布局自检（headless 15 项）。
#
# 跑法（结构档，无显示）——**本 step 的主验收**：
#   timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#     --script res://src/tests/check_person_walk.gd
#   期望：total=15 failed=0 + SELF-CHECK OK + EXIT=0 + 日志 0 ERROR + 无 "[ws] listening"。
#
#   task_35 Step 3 起本文件**没有渲染组**（两档 total 都是 15）：世界不再自动让人绕圈，
#   旧 R 组 6 项（三帧 JPEG）的机位是对着**旧环路**摆的（世界 (37,101) 看 (50,101)），
#   人不再绕圈后它没有主语 —— 行人的"三帧真渲染"由 `check_person_walk_shots.gd` 承担
#   （它为 pedestrian_01 保留了同一套机位几何）。
#
# 断言分组（15 = A 8 + B 4 + C 3）：
#   A(8) 资产：glb 能 load / 有动画(≥1) / 有蒙皮(≥60 骨) / 时长 2.0~2.5 s / 人高 1.60~1.85 /
#             脚底 |y|≤2 mm / 朝 +X（x 跨度 < z 跨度）/ 三角数 ≤150k
#   B(4) 场景：PersonWalking.tscn 能载且根脚本是 Person.gd / Body 挂走路 glb 且 transform=IDENTITY /
#             脚本常量 WALK_GLB 与 .tscn 里 Body 的 glb 逐字一致（且 .tscn 不含旧目录）/ 整棵子树零物理节点
#   C(3) 世界（**task_35 Step 3 新写**）：
#             world-pedestrians-count      容器名/父节点/孩子数/孩子名/场景/脚本 + 底盘契约面
#             world-pedestrians-spawn-clear 每个出生点：不压障碍、离边界、离 spawn、离静态两人、两人互距
#             world-pedestrians-idempotent  BuildPedestrians() 调两次节点数不变、容器恰 1 个、孩子数不变
#
# task_35 Step 3 删掉的 11 项（本次 total 20 → 15，理由逐条）：
#   C 旧 3（Walkers 里恰 1 个 walker_01 / Path_Hits_Rect 净空 / BuildWalkers 幂等）→ 换成上面的 C 新 3；
#   D 5（run-anim-playing / run-progress-monotonic / run-lap-close / run-path-dev / run-hip-drift）
#     —— 「人真的在走」改为由 `check_pedestrian.gd`（60 帧真推进证明 v/dir 生效）与
#     `check_ws_manager.gd` 的 K 组（网络驱动）覆盖；世界侧已无路径可量；
#   R 6（三帧 JPEG）—— 见上，机位主语消失，改由 `check_person_walk_shots.gd` 覆盖。
#   `total` 变化的来龙去脉写进 tmp/task_35/impl.md。
#
# 关键取舍（都有实测依据，别"优化"掉）：
#   - 资产 AABB 用**局部变换累加**（树外读 global_* 会 ERROR，check_models.gd P8）；
#     入树后才有 global_transform。
#   - `MeshInstance3D.get_aabb()` 是 **mesh 局部** AABB（不含节点旋转/缩放）。
#   - 世界行人**站住不动**是预期（无外部连接时无人驱动）：本文件**不**断言它们移动，
#     也不调 SetCommand —— 那是 `check_person_walk_shots.gd` 的事。
#   - 本文件**不 push_error**（失败一律 `_check(..., false, ...)` + quit(1)），
#     退出前 queue_free + 逐级 await 两帧，避免 `resources still in use at exit`。
#
# 注意（GDScript 4.7）：`load()` / `instantiate()` / `call()` 的返回值一律显式标注类型。

extends SceneTree

const WorldGd := preload("res://src/world/World.gd")
const PersonGd := preload("res://src/props/Person.gd")

# ---------------------------------------------------------------- 场景（唯一来源）
const WALKER_SCENE := "res://src/scenes/props/PersonWalking.tscn"
const PERSON_SCRIPT := "res://src/props/Person.gd"
const WORLD_SCENE := "res://src/scenes/World.tscn"
const PEDESTRIAN_SCENE := "res://src/scenes/props/Pedestrian.tscn"
const PEDESTRIAN_SCRIPT := "res://src/chassis/Pedestrian.gd"
## 改名前的资产目录（不可再出现在 .tscn 里；`_walk` 后缀是本 step 的资产口径）。
const OLD_ASSET_DIR := "/rp_nathan_animated_003/"

# ---------------------------------------------------------------- A 组判据
const HEIGHT_TOL := 0.01
const HEIGHT_MIN_M := 1.60
const HEIGHT_MAX_M := 1.85
## 目标人高（normalized/…/convert.json 的 target_height_m；A5 逐位对到 ±1 cm）。
const WANT_HEIGHT_M := 1.84
const FEET_TOL_M := 2.0e-3
const TRI_CAP := 150000
const WANT_JOINTS := 60
const CLIP_SEC_MIN := 2.0
const CLIP_SEC_MAX := 2.5

# ---------------------------------------------------------------- C 组判据（task_35 Step 3）
## 出生点离最近障碍矩形的最小距离（米）= 胶囊半径 0.25 + 0.25 余量。
const PED_CLEAR_M := 0.5
## 出生点离 4 面世界边界的最小距离（米）：胶囊半径 + 余量。
const BORDER_MIN_M := 1.0
## 出生点离静态人物 PERSONS 的最小距离（米）：不污染 person 系列取景。
const PERSON_MIN_GAP_M := 2.0
## 两个行人互相的最小距离（米）：镜像 manager 的 PEDESTRIAN_SPAWN_MIN_GAP_M。
const PED_GAP_MIN_M := 1.0

# ---------------------------------------------------------------- 状态
var _total: int = 0
var _failed: int = 0
var _warned: Dictionary = {}

var _world: Node3D = null
var _pedestrians_root: Node3D = null
var _peds: Array[Node3D] = []
var _ws: Node = null

## B 组测得的 Body.scene_file_path，供 scene-const-consistent 复用。
var _scene_body_path: String = ""


func _initialize() -> void:
	_run()


func _run() -> void:
	_run_assets()
	await _run_scene()
	await _run_world()
	await _teardown()
	_finish()


# ================================================================ A 组：资产（8 项）

func _run_assets() -> void:
	var fails: Array = ["asset-animations", "asset-skins", "asset-clip-duration",
		"asset-aabb-height", "asset-feet", "asset-facing-plus-x", "asset-tri-budget"]

	# A1 asset-loads：glb 是 PackedScene 且能实例化（.godot 缓存缺失时会红 → 先跑 --import）。
	var packed: PackedScene = load(PersonGd.WALK_GLB) as PackedScene
	var body: Node3D = packed.instantiate() as Node3D if packed != null else null
	_check("asset-loads", packed != null and body != null, PersonGd.WALK_GLB)
	if body == null:
		for label in fails:
			_check(String(label), false, "前置失败：glb 不可实例化")
		return

	# A2 asset-animations：AnimationPlayer 在且至少有 1 条动画。
	var ap: AnimationPlayer = _find_anim_player(body)
	var clips: PackedStringArray = PackedStringArray()
	if ap != null:
		clips = ap.get_animation_list()
	_check("asset-animations", ap != null and clips.size() >= 1,
		"AnimationPlayer=%s clips=%s" % [ap != null, clips])

	# A3 asset-skins：蒙皮骨架在，骨数 ≥ 60（源 88 骨；掉到 <60 说明蒙皮没导进来）。
	var sk: Skeleton3D = _find_skeleton(body)
	var bones: int = sk.get_bone_count() if sk != null else -1
	_check("asset-skins", sk != null and bones >= WANT_JOINTS,
		"Skeleton3D=%s bones=%d(>=%d)" % [sk != null, bones, WANT_JOINTS])

	# A4 asset-clip-duration：时长 ∈ [2.0, 2.5] s（现 glb = 68 键 @30 fps = 2.2333 s；
	# 源 69 帧 = 2.2667 s，其中 frame 1 是零位 T-pose，已在资产侧裁掉 —— 两者都落在窗口内）。
	var dur: float = -1.0
	var clip_name: String = ""
	if ap != null and clips.size() >= 1:
		clip_name = String(clips[0])
		var a: Animation = ap.get_animation(clip_name)
		if a != null:
			dur = a.length
	_check("asset-clip-duration", dur >= CLIP_SEC_MIN and dur <= CLIP_SEC_MAX,
		"clip=%s length=%.4f s want=[%.1f, %.1f]" % [clip_name, dur, CLIP_SEC_MIN, CLIP_SEC_MAX])

	# A5~A8：几何三件 + 三角预算（局部变换累加，树外不用 global_*）。
	var box: AABB = _aabb_local_tree(body)
	_check("asset-aabb-height",
		box.size.y >= HEIGHT_MIN_M and box.size.y <= HEIGHT_MAX_M
			and absf(box.size.y - WANT_HEIGHT_M) <= HEIGHT_TOL,
		"%s want≈%.2f range=[%.2f, %.2f]" % [_fmt(box), WANT_HEIGHT_M,
			HEIGHT_MIN_M, HEIGHT_MAX_M])
	_check("asset-feet", absf(box.position.y) <= FEET_TOL_M,
		"position.y=%.6f(<%.4f)" % [box.position.y, FEET_TOL_M])
	_check("asset-facing-plus-x", box.size.x < box.size.z,
		"x=%.4f < z=%.4f（前后薄于左右 = 朝 ±X）" % [box.size.x, box.size.z])

	var tris: int = 0
	for mi: MeshInstance3D in _collect_meshes(body):
		tris += int(mi.mesh.get_faces().size() / 3)
	_check("asset-tri-budget", tris > 0 and tris <= TRI_CAP,
		"tris=%d(<=%d)" % [tris, TRI_CAP])

	body.free()


# ================================================================ B 组：道具场景（4 项）

func _run_scene() -> void:
	var packed: PackedScene = load(WALKER_SCENE) as PackedScene
	var probe: Node3D = packed.instantiate() as Node3D if packed != null else null

	# B1 scene-loads：可加载、根名对、根脚本是 src/props/Person.gd。
	var script_path: String = ""
	if probe != null and probe.get_script() != null:
		script_path = String(probe.get_script().resource_path)
	_check("scene-loads",
		probe != null and probe.name == "PersonWalking" and script_path == PERSON_SCRIPT,
		"%s root=%s script=%s" % [WALKER_SCENE,
			probe.name if probe != null else "null", script_path])

	# B2 scene-body-mount：Body 存在、静态挂走路 glb、局部变换 = IDENTITY（原点 = 脚底中心）。
	var body: Node3D = probe.get_node_or_null("Body") as Node3D if probe != null else null
	var body_path: String = body.scene_file_path if body != null else ""
	_scene_body_path = body_path
	_check("scene-body-mount",
		body != null and body_path == PersonGd.WALK_GLB
			and body.transform.is_equal_approx(Transform3D.IDENTITY),
		"body=%s glb=%s xf=%s" % [body != null, body_path,
			body.transform if body != null else "null"])

	# B3 scene-const-consistent：脚本常量 == .tscn 里 Body 的 glb 路径（防两处漂移），
	#    且 .tscn 文本里**不许**再出现改名前的目录（plan.md R2/R11：改名必须彻底）。
	var txt: String = _read_text(WALKER_SCENE)
	var has_const_path: bool = txt.contains(PersonGd.WALK_GLB)
	var has_old_dir: bool = txt.contains(OLD_ASSET_DIR)
	_check("scene-const-consistent",
		_scene_body_path == PersonGd.WALK_GLB and has_const_path and not has_old_dir,
		"script=%s scene=%s 文本含新路径=%s 含旧目录=%s" % [PersonGd.WALK_GLB,
			_scene_body_path, str(has_const_path), str(has_old_dir)])

	if probe == null:
		_check("scene-no-collision", false, "前置失败：PersonWalking.tscn 不可实例化")
		return
	root.add_child(probe)
	await process_frame
	# B4 scene-no-collision：整棵子树零物理节点（E27：PersonWalking 只是给相机看的一具壳）。
	#    注意：**世界预置行人**（Pedestrian.tscn）恰恰**有**胶囊碰撞，那是有意为之（§4.11），
	#    所以本断言只针对 PersonWalking.tscn 这一个道具场景。
	var physics: int = _count_physics(probe)
	_check("scene-no-collision", physics == 0, "physics_nodes=%d" % physics)
	probe.queue_free()
	await process_frame


# ================================================================ C 组：世界预置行人（3 项）

func _run_world() -> void:
	var packed: PackedScene = load(WORLD_SCENE) as PackedScene
	var world: Node3D = packed.instantiate() as Node3D if packed != null else null
	if world == null:
		for label in ["world-pedestrians-count", "world-pedestrians-spawn-clear",
				"world-pedestrians-idempotent"]:
			_check(String(label), false, "前置失败：World.tscn 不可加载/实例化")
		return

	# 与 check_person_shots 同款：入树前关掉 WebSocketManager.autolisten（不连 9999）。
	_ws = world.get_node_or_null("Systems/WebSocketManager")
	if _ws != null:
		_ws.set("autolisten", false)
	root.add_child(world)
	_world = world
	await process_frame
	await physics_frame

	_run_pedestrians_count(world)
	_run_pedestrians_spawn_clear()
	await _run_pedestrians_idempotent(world)


## C1 world-pedestrians-count：容器名对、父节点对、孩子数 == PEDESTRIANS.size()、
## 孩子名与常量表逐字一致、孩子来自 Pedestrian.tscn、根脚本是本 step 的行人底盘，
## 且**具备底盘契约面**（SetCommand / GetPose）—— 外部脚本靠这两条驱动它。
func _run_pedestrians_count(world: Node3D) -> void:
	_pedestrians_root = world.call("GetPedestriansRoot") as Node3D
	var kids: Array[Node] = []
	if _pedestrians_root != null:
		kids = _pedestrians_root.get_children()
	var specs: Array = WorldGd.PEDESTRIANS
	var names_ok: bool = kids.size() == specs.size()
	var scene_ok: bool = kids.size() == specs.size()
	var script_ok: bool = kids.size() == specs.size()
	var contract_ok: bool = kids.size() == specs.size()
	_peds = []
	for i in range(kids.size()):
		var kid3: Node3D = kids[i] as Node3D
		if kid3 == null:
			names_ok = false
			scene_ok = false
			script_ok = false
			contract_ok = false
			continue
		_peds.append(kid3)
		if i < specs.size():
			var spec: Dictionary = specs[i]
			if String(kid3.name) != String(spec["name"]):
				names_ok = false
		else:
			names_ok = false
		if kid3.scene_file_path != PEDESTRIAN_SCENE:
			scene_ok = false
		var kid_script_path: String = ""
		if kid3.get_script() != null:
			kid_script_path = String(kid3.get_script().resource_path)
		if kid_script_path != PEDESTRIAN_SCRIPT:
			script_ok = false
		if not (kid3.has_method("SetCommand") and kid3.has_method("GetPose")):
			contract_ok = false
	_check("world-pedestrians-count",
		_pedestrians_root != null
			and String(_pedestrians_root.name) == String(WorldGd.PEDESTRIANS_NODE)
			and _pedestrians_root.get_parent() == world
			and not kids.is_empty()
			and names_ok and scene_ok and script_ok and contract_ok,
		"root=%s parent_ok=%s count=%d(want %d) names_ok=%s scene_ok=%s script_ok=%s "
			% [String(_pedestrians_root.name) if _pedestrians_root != null else "null",
				str(_pedestrians_root.get_parent() == world if _pedestrians_root != null else false),
				kids.size(), specs.size(), str(names_ok), str(scene_ok), str(script_ok)]
		+ "contract_ok=%s scene=%s script=%s" % [str(contract_ok), PEDESTRIAN_SCENE,
			PEDESTRIAN_SCRIPT])


## C2 world-pedestrians-spawn-clear：逐点五条判据（表里的出生点 = 世界的唯一真相）。
##   ① 不落在任何 OBSTACLES 矩形外扩 PED_CLEAR_M 内（胶囊 0.25 + 0.25 余量）
##   ② 距 4 面世界边界 ≥ BORDER_MIN_M
##   ③ 水平距 SPAWN_WORLD > SPAWN_CLEAR_RADIUS_M（3 m）
##   ④ 距 PERSONS 每个人 ≥ PERSON_MIN_GAP_M
##   ⑤ 两人互相 ≥ PED_GAP_MIN_M
func _run_pedestrians_spawn_clear() -> void:
	var extent: float = _world_extent_m()
	var specs: Array = WorldGd.PEDESTRIANS
	var bad: Array[String] = []
	var spawn: Vector3 = WorldGd.SPAWN_WORLD
	var radius: float = WorldGd.SPAWN_CLEAR_RADIUS_M
	for raw: Variant in specs:
		var spec: Dictionary = raw
		var p: Vector3 = spec["pos"]
		var who: String = String(spec.get("name", "?"))
		# ① 障碍：外扩 0.5 m 的矩形包含判定（比"路径整段"更直接，也就是本 step 的全部要求）
		for raw_obs: Variant in WorldGd.OBSTACLES:
			var obs: Dictionary = raw_obs
			var o: Vector3 = obs["origin"]
			var s: Vector3 = obs["size"]
			if p.x > o.x - PED_CLEAR_M and p.x < o.x + s.x + PED_CLEAR_M \
					and p.y > o.y - PED_CLEAR_M and p.y < o.y + s.y + PED_CLEAR_M:
				bad.append("%s 压 %s" % [who, String(obs.get("name", "?"))])
		# ② 世界边界（4 面；世界是 [0, extent] × [0, extent] 的方形）
		var border: float = minf(minf(p.x, p.y), minf(extent - p.x, extent - p.y))
		if border < BORDER_MIN_M:
			bad.append("%s 距边界 %.3f m" % [who, border])
		# ③ 出生点净空
		var d_spawn: float = Vector2(p.x - spawn.x, p.y - spawn.y).length()
		if d_spawn <= radius:
			bad.append("%s 距 spawn %.3f m" % [who, d_spawn])
		# ④ 静态两人
		for raw_p: Variant in WorldGd.PERSONS:
			var person: Dictionary = raw_p
			var pw: Vector3 = person["pos"]
			var d_person: float = Vector2(p.x - pw.x, p.y - pw.y).length()
			if d_person < PERSON_MIN_GAP_M:
				bad.append("%s 距 %s %.3f m" % [who, String(person.get("name", "?")), d_person])
	# ⑤ 两人互距
	for i in range(specs.size()):
		for j in range(i + 1, specs.size()):
			var a: Vector3 = (specs[i] as Dictionary)["pos"]
			var b: Vector3 = (specs[j] as Dictionary)["pos"]
			var gap: float = Vector2(a.x - b.x, a.y - b.y).length()
			if gap < PED_GAP_MIN_M:
				bad.append("%s~%s 仅 %.3f m" % [String((specs[i] as Dictionary)["name"]),
					String((specs[j] as Dictionary)["name"]), gap])

	# 实测落位（只进 detail，真正的"位姿 == 表"由 check_person_walk_shots A4 断言）
	var live: Array[String] = []
	for i in range(mini(_peds.size(), specs.size())):
		var t: Dictionary = _peds[i].call("GetPose")
		var want: Vector3 = (specs[i] as Dictionary)["pos"]
		live.append("%s(%.3f, %.3f) yaw=%.4f" % [String(_peds[i].name),
			float(t.get("x", -999.0)), float(t.get("y", -999.0)), float(t.get("yaw", -999.0))]
			+ "(want %.1f, %.1f)" % [want.x, want.y])
	_check("world-pedestrians-spawn-clear", bad.is_empty(),
		"specs=%d clear=%.2f m border>=%.2f spawn>%.2f persons>=%.2f gap>=%.2f bad=%s live=[%s]" % [
			specs.size(), PED_CLEAR_M, BORDER_MIN_M, radius, PERSON_MIN_GAP_M,
			PED_GAP_MIN_M, bad, "；".join(live)])


## C3 world-pedestrians-idempotent：BuildPedestrians() 调两次后节点总数不变、
## World 直接子节点里名为 Pedestrians 的恰 1 个、容器孩子数仍 == PEDESTRIANS.size()。
func _run_pedestrians_idempotent(world: Node3D) -> void:
	var nodes_before: int = _count_nodes(world)
	world.call("BuildPedestrians")
	await process_frame
	world.call("BuildPedestrians")
	await process_frame
	await process_frame
	var nodes_after: int = _count_nodes(world)
	var same_name: int = 0
	for child: Node in world.get_children():
		if String(child.name) == String(WorldGd.PEDESTRIANS_NODE):
			same_name += 1
	# 重建过 → 旧实例已 queue_free，所有引用必须重取（否则拿到悬空实例）。
	_pedestrians_root = world.call("GetPedestriansRoot") as Node3D
	var count_after: int = _pedestrians_root.get_child_count() if _pedestrians_root != null else -1
	_peds = []
	if _pedestrians_root != null:
		for child: Node in _pedestrians_root.get_children():
			var kid: Node3D = child as Node3D
			if kid != null:
				_peds.append(kid)
	_check("world-pedestrians-idempotent",
		count_after == WorldGd.PEDESTRIANS.size() and same_name == 1
			and nodes_after == nodes_before,
		"count=%d want=%d same_name_nodes=%d nodes=%d->%d" % [count_after,
			WorldGd.PEDESTRIANS.size(), same_name, nodes_before, nodes_after])


## 世界边长（米）：从墙类障碍反推，不写字面量（scale 唯一来源原则）。
## wall_north / wall_east 的 origin+size 正好落在 128 —— 与 [sim_world] 的
## 256 格 × 0.5 m 一致（check_bounds 的 B1b/B1c 已在数值上兜住这一点）。
func _world_extent_m() -> float:
	var extent: float = 0.0
	for raw: Variant in WorldGd.OBSTACLES:
		var obs: Dictionary = raw
		var o: Vector3 = obs["origin"]
		var s: Vector3 = obs["size"]
		extent = maxf(extent, maxf(o.x + s.x, o.y + s.y))
	return extent


# ================================================================ 骨骼 / 网格

func _find_skeleton(node: Node) -> Skeleton3D:
	if node == null:
		return null
	var sk: Skeleton3D = node as Skeleton3D
	if sk != null:
		return sk
	for child: Node in node.get_children():
		var found: Skeleton3D = _find_skeleton(child)
		if found != null:
			return found
	return null


func _find_anim_player(node: Node) -> AnimationPlayer:
	if node == null:
		return null
	var ap: AnimationPlayer = node as AnimationPlayer
	if ap != null:
		return ap
	for child: Node in node.get_children():
		var found: AnimationPlayer = _find_anim_player(child)
		if found != null:
			return found
	return null


# ================================================================ AABB

func _aabb_of_local_in(xf: Transform3D, local: AABB) -> AABB:
	var out := AABB()
	var first: bool = true
	for i in range(8):
		var corner: Vector3 = xf * local.get_endpoint(i)
		if first:
			out = AABB(corner, Vector3.ZERO)
			first = false
		else:
			out = out.expand(corner)
	return out


func _aabb_local_tree(node: Node) -> AABB:
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


func _merge(boxes: Array[AABB]) -> AABB:
	if boxes.is_empty():
		return AABB()
	var out: AABB = boxes[0]
	for i in range(1, boxes.size()):
		out = out.merge(boxes[i])
	return out


# ================================================================ 统计 / 工具

func _count_nodes(node: Node) -> int:
	if node == null:
		return 0
	var n: int = 1
	for child: Node in node.get_children():
		n += _count_nodes(child)
	return n


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


func _fmt(a: AABB) -> String:
	return "P(%.4f, %.4f, %.4f) S(%.4f, %.4f, %.4f)" % [a.position.x, a.position.y,
		a.position.z, a.size.x, a.size.y, a.size.z]


func _read_text(res_path: String) -> String:
	var f: FileAccess = FileAccess.open(res_path, FileAccess.READ)
	if f == null:
		return ""
	var txt: String = f.get_as_text()
	f.close()
	return txt


# ================================================================ 收尾 / 断言

func _teardown() -> void:
	_peds = []
	_pedestrians_root = null
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


func _warn_once(key: String, msg: String) -> void:
	if _warned.has(key):
		return
	_warned[key] = true
	push_warning(msg)


func _finish() -> void:
	print("[check] env display=%s adapter='%s'" % [
		DisplayServer.get_name(), RenderingServer.get_video_adapter_name()])
	print("[check] total=%d failed=%d" % [_total, _failed])
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED")
		quit(1)
