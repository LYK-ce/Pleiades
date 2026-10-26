# Presented by KeJi
# Created Date ： 2026-10-24
# Modified Date ： 2026-10-25
#
# 人物道具脚本（**唯一**一份；静态站立 + 沿路径走动，默认静态）。
#
#   静态（支线 human_model Step 4）：`Person.tscn` → `Body = rp_carla_rigged_001/model.glb`，
#     不给 `walk_speed` / `path` 就**站着不动**（导出字段的默认值 = 不动），行为与 Step 4 一致。
#   走动（支线 walking_person Step 3）：`PersonWalking.tscn` → `Body = rp_nathan_animated_003_walk/model.glb`
#     （88 骨蒙皮 + 1 条走路动画），由 `World.gd::BuildWalkers()` 调 `Configure()` 给一条闭合路径
#     与 `walk_speed`，脚本在 `_physics_process` 里沿路径推进、朝向对齐行进方向、动画跟着播。
#     走动的三个字段名与 step.md 逐字一致：`walk_speed` / `path` / `loop`。
#
# 纯视觉、**零碰撞**、**不进装配线**：
#   一旦带 CollisionShape3D，`check_lidar` 的 F7-real-wall-distance / F8-real-wall-hits 会多出回波，
#   UGV 也会把行人当障碍绕行 —— COMPONENTS.md §9.1 E27 的延伸。人只是"给相机看的一具壳"。
#
# 坐标口径（静态 / 走动完全一致）：模型原生 **米制、人朝 +X、脚底 y=0（glTF/Godot Y-up）**，
# 节点上不补 scale、不补 y 偏移（否则 accessor min/max 与 AABB 打架，见 check_person_assets）。
# 路径点一律是**世界坐标（Z-up，x=东 / y=南 / z=高）**；换轴只发生在 `WorldTransform.MakeTransform()`，
# 本文件里不出现任何 (x, z, y) 换轴字面量。
#
# 动画三条硬规则（任何一条都别"优化"掉，plan.md §4.2）：
#   ① `find_child("AnimationPlayer", true, false)` —— `owned=false` 才看得见 glb 实例**内部**的子节点
#      （导入器给的节点名/owner 会随版本变，所以再按**类型**递归兜底）；
#   ② 播 `get_animation_list()` 的**第 0 条**，不硬编码 "Animation" / "Take 001"；
#   ③ `loop_mode` 只在 LOOP_NONE 时设一次 LOOP_LINEAR；`speed_scale = walk_speed / CLIP_MPS`
#      （改速度时步频跟着变，脚不滑）。`walk_speed <= 0` 时 `speed_scale = 1.0`（静止但动画不冻住）。
#   找不到 AnimationPlayer / 动画列表为空：**只 push_warning（每个实例一次），绝不 push_error** ——
#   静态人形本来就没有 AnimationPlayer；push_error 会把 `--quit` 的 "0 ERROR" 验收搞成假红。
#
# 为什么 `_physics_process` 里用 `fposmod` 而不是 Tween：走一圈 ~35 s，Tween 需要每圈重建；
# fposmod 是纯函数、无累积漂移，自检可以逐帧对比"节点位置 == 路径采样点"（误差 ≤1e-3）。

extends Node3D

const WorldTransform := preload("res://src/common/WorldTransform.gd")

## 静态人形（唯一来源）：Person.tscn 的 Body 实例必须指向同一路径
## （check_person_assets 的 script-model-const 断言锁住两处一致 —— 不许改此行）。
const MODEL_GLB := "res://assets/humans/rp_carla_rigged_001/model.glb"
## 走路人形（唯一来源）：PersonWalking.tscn 的 Body 实例必须指向同一路径
## （check_person_walk.gd 的 scene-const-consistent 断言锁住两处一致）。
const WALK_GLB := "res://assets/humans/rp_nathan_animated_003_walk/model.glb"
## 人高判据（米，与 step.md 验收同口径）：AABB 的 y 尺寸必须落在 [MIN, MAX]。
const HEIGHT_MIN_M := 1.60
const HEIGHT_MAX_M := 1.85
## 资产自带步速（m/s）= 源动画剥掉根位移后的前进速度；这是"滑步判据"的基准。
## 实测值 = normalized/.../convert.json 的 `after.speed_mps` = **1.2694 m/s**
## （`root 前进极差 2.865983 m / 保留时长 2.233333 s × k 0.989205`）。
## 这里保留 1.260：与实测差 **0.74%**（< 1% 判据）→ 按 plan.md §6 **不改常量**，
## 残差写在 tmp/walking_person/normalized/README.md §2.2（脚相对地面慢 0.74%，肉眼不可辨）。
const CLIP_MPS := 1.260
## 允许的路径速度偏差（±10%）；超了只在日志里提示"有滑步风险"，不阻断。
const SPEED_TOL_RATIO := 0.10
## 脚底贴地容差（米）：与 check_person_walk 的 asset-feet 同判据。
const FEET_TOL_M := 2.0e-3

# ---------------------------------------------------------------- 走动字段（Step 3）
# 三个字段都**不写自定义 setter**（plan.md R1）：setter 里再写 `path = v` 会自赋值递归/丢值。
# 弧长表只在 `_ready()`（读 path）与 `Configure()`（写完 path 之后）各重建一次。
## 沿路径前进速度（m/s）。**<= 0 表示不走**（静态站立 = 默认值，向后兼容 Person.tscn）。
@export var walk_speed: float = 0.0
## 路径点（**世界坐标 Z-up、米**）；隐式闭合：末点自动接回首点。
@export var path: Array[Vector3] = []
## true = 走完一圈回绕首点；false = 到终点停在末点。
@export var loop: bool = true

## 预计算的弧长表（米）：_seg_len[i] = 第 i 段长度，_cum_len[i] = 第 i 段起点的累计弧长。
var _seg_len: PackedFloat32Array = PackedFloat32Array()
var _cum_len: PackedFloat32Array = PackedFloat32Array()
var _path_len_m: float = 0.0
## 沿路径的弧长进度（米，loop 时永远在 [0, _path_len_m)）。
var _s_m: float = 0.0
## 动画步频倍率 = walk_speed / CLIP_MPS（<=0 时取 1.0）。
var _anim_scale: float = 1.0
var _anim: AnimationPlayer = null
var _warned: Dictionary = {}


func _ready() -> void:
	_Rebuild_Path()
	_Ensure_Anim_Player()
	_Update_Anim_Speed()
	_Start_Anim()
	# 场景里没配路径时（静态人形 / 编辑器里单独打开本场景）保持原地不动。
	if _path_len_m > 0.0:
		_Place_At(_s_m)


## 推进（step.md 指定：只在 `_physics_process` 推，`_process` 里**不**再推一次，否则速度翻倍）。
func _physics_process(delta: float) -> void:
	if _path_len_m <= 0.0 or walk_speed <= 0.0:
		return
	if loop:
		_s_m = fposmod(_s_m + walk_speed * delta, _path_len_m)
	else:
		_s_m = clampf(_s_m + walk_speed * delta, 0.0, _path_len_m)
	_Place_At(_s_m)


# ---------------------------------------------------------------- 装配

## spec: {"path": Array[Vector3]（世界 Z-up 米）, "walk_speed": float（别名 speed_mps）,
##        "loop": bool, "phase_m": float}
## 必须在**入树之后**调用（内部写 global_transform；树外写 global_* 会 ERROR）。
func Configure(spec: Dictionary) -> void:
	var raw: Variant = spec.get("path", [])
	var pts: Array[Vector3] = []
	if raw is PackedVector3Array:
		for v: Vector3 in (raw as PackedVector3Array):
			pts.append(v)
	elif raw is Array:
		for v: Variant in (raw as Array):
			if v is Vector3:
				pts.append(v)
	path = pts
	_Rebuild_Path()

	# 速度：优先 step.md 的字段名 `walk_speed`，兼容旧调用方的 `speed_mps` 别名。
	var sp: float = float(spec.get("walk_speed", spec.get("speed_mps", CLIP_MPS)))
	if not is_finite(sp) or sp < 0.0:
		_Warn_Once("speed", "[person] walk_speed=%s 非法，回退 %.3f" % [sp, CLIP_MPS])
		sp = CLIP_MPS
	walk_speed = sp
	loop = bool(spec.get("loop", true))

	var phase: float = float(spec.get("phase_m", 0.0))
	_s_m = 0.0 if _path_len_m <= 0.0 else fposmod(phase, _path_len_m)

	_Update_Anim_Speed()
	_Start_Anim()
	_Place_At(_s_m)

	var slide: String = "" if not _Slide_Risk() else " (偏离 CLIP_MPS 超过 %.0f%%，有滑步风险)" \
		% (SPEED_TOL_RATIO * 100.0)
	print("[person] ", name, " configured: points=", path.size(),
		" path_len=%.3f m walk_speed=%.3f m/s loop=%s anim_scale=%.3f anim=%s%s" % [
			_path_len_m, walk_speed, str(loop), _anim_scale,
			_anim.current_animation if _anim != null else "<none>", slide])


## 渲染档/自检用：瞬移到指定弧长进度（避免傻等 2095 帧）。
func Set_Progress_M(s: float) -> void:
	if _path_len_m <= 0.0:
		return
	_s_m = fposmod(s, _path_len_m) if loop else clampf(s, 0.0, _path_len_m)
	_Place_At(_s_m)


func Get_Progress_M() -> float:
	return _s_m


func Get_Path_Length() -> float:
	return _path_len_m


func Get_Speed_Mps() -> float:
	return walk_speed


## 世界坐标（Z-up，米）里的当前位置 —— 自检据此与路径采样点逐位对比。
func Get_Position_W() -> Vector3:
	return WorldTransform.PosToWorld(global_position)


## 世界 yaw（绕 +z，逆时针为正，与 WorldTransform 同口径）。
func Get_Yaw_W() -> float:
	return WorldTransform.YawToWorld(rotation.y)


## 模型根（glb 实例）；缺失返回 null（自检据此报红）。
func Get_Body() -> Node3D:
	return get_node_or_null("Body") as Node3D


func Get_Anim_Player() -> AnimationPlayer:
	return _Ensure_Anim_Player()


func Is_Anim_Playing() -> bool:
	var ap: AnimationPlayer = _Ensure_Anim_Player()
	return ap != null and ap.is_playing() and not String(ap.current_animation).is_empty()


## 路径是否与给定的世界 XY 矩形相撞（供自检检查障碍净空；矩形 = origin/size，Z-up 世界）。
func Path_Hits_Rect(origin_xy: Vector2, size_xy: Vector2, margin_m: float) -> bool:
	var n := path.size()
	if n < 2:
		return false
	var x0: float = origin_xy.x - margin_m
	var x1: float = origin_xy.x + size_xy.x + margin_m
	var y0: float = origin_xy.y - margin_m
	var y1: float = origin_xy.y + size_xy.y + margin_m
	for i in range(n):
		var a: Vector3 = path[i]
		var b: Vector3 = path[(i + 1) % n]
		# 轴对齐矩形 vs 线段：先做包围盒快速排除，再做 slab 判定（线段与 AABB 相交）
		var s_min_x: float = minf(a.x, b.x)
		var s_max_x: float = maxf(a.x, b.x)
		var s_min_y: float = minf(a.y, b.y)
		var s_max_y: float = maxf(a.y, b.y)
		if s_max_x < x0 or s_min_x > x1 or s_max_y < y0 or s_min_y > y1:
			continue
		if _Segment_Hits_Aabb(a, b, x0, x1, y0, y1):
			return true
	return false


# ---------------------------------------------------------------- 路径采样

func _Rebuild_Path() -> void:
	_seg_len = PackedFloat32Array()
	_cum_len = PackedFloat32Array()
	_path_len_m = 0.0
	var n := path.size()
	if n < 2:
		return
	for i in range(n):
		var a: Vector3 = path[i]
		var b: Vector3 = path[(i + 1) % n]
		var seg: float = a.distance_to(b)
		_seg_len.append(seg)
		_cum_len.append(_path_len_m)
		_path_len_m += seg


## 弧长 -> (world_pos, world_yaw)。闭区间折线：最后一段接回首点。
func _Sample_Path(s: float) -> Dictionary:
	var n := path.size()
	if n == 0:
		return {"pos": Vector3.ZERO, "yaw": 0.0}
	if n == 1 or _path_len_m <= 0.0:
		return {"pos": path[0], "yaw": 0.0}
	# loop=false 时 s == L 必须停在**末点**（fposmod(L, L) 会回到首点，plan.md §4.1）。
	var t: float = fposmod(s, _path_len_m) if loop else clampf(s, 0.0, _path_len_m)
	var k: int = 0
	for i in range(n):
		var seg: float = _seg_len[i]
		var end: float = _cum_len[i] + seg
		if t < end or i == n - 1:
			k = i
			break
	var a: Vector3 = path[k]
	var b: Vector3 = path[(k + 1) % n]
	var seg_len: float = _seg_len[k]
	var f: float = 0.0 if seg_len <= 0.0 else clampf((t - _cum_len[k]) / seg_len, 0.0, 1.0)
	var d: Vector3 = b - a
	return {
		"pos": a.lerp(b, f),
		"yaw": 0.0 if d.length() <= 0.0 else atan2(d.y, d.x),
	}


func _Place_At(s: float) -> void:
	if path.size() == 0:
		return
	var sample: Dictionary = _Sample_Path(s)
	if not is_inside_tree():
		# 树外写 global_transform 会 ERROR（与 check_models.gd 头注释 P8 同款）；
		# 这里退化成写本地变换（父节点恒等时等价，且不产生 ERROR）。
		transform = WorldTransform.MakeTransform(sample["pos"], float(sample["yaw"]))
		return
	global_transform = WorldTransform.MakeTransform(sample["pos"], float(sample["yaw"]))


# ---------------------------------------------------------------- 动画

## 按 step.md 的字面调用找 AnimationPlayer：`owned=false` 才能看见 glb 实例内部的子节点。
## 找不到再按**类型**递归兜底（导入器给的节点名会随版本变）。
func _Ensure_Anim_Player() -> AnimationPlayer:
	if _anim != null and is_instance_valid(_anim):
		return _anim
	var by_name: Node = find_child("AnimationPlayer", true, false)
	if by_name is AnimationPlayer:
		_anim = by_name as AnimationPlayer
	if _anim == null:
		_anim = _Find_Anim_Player(self)
	if _anim == null:
		# 静态人形（Person.tscn）没有 AnimationPlayer —— **静态属正常**，只 warning。
		_Warn_Once("no-anim", "[person] 子树里找不到 AnimationPlayer（静态人形属正常；走动模型请查 glb 导入）")
	return _anim


func _Find_Anim_Player(root: Node) -> AnimationPlayer:
	if root == null:
		return null
	var hit: AnimationPlayer = root as AnimationPlayer
	if hit != null:
		return hit
	for child: Node in root.get_children():
		var found: AnimationPlayer = _Find_Anim_Player(child)
		if found != null:
			return found
	return null


## 播第 0 条动画；loop_mode 只在 LOOP_NONE 时设一次。
func _Start_Anim() -> void:
	var ap: AnimationPlayer = _Ensure_Anim_Player()
	if ap == null:
		return
	var list: PackedStringArray = ap.get_animation_list()
	if list.is_empty():
		_Warn_Once("no-clip", "[person] AnimationPlayer 里没有任何动画")
		return
	var nm: String = String(list[0])
	var anim: Animation = ap.get_animation(nm)
	if anim != null and anim.loop_mode == Animation.LOOP_NONE:
		anim.loop_mode = Animation.LOOP_LINEAR
	ap.speed_scale = _anim_scale
	if ap.current_animation != nm or not ap.is_playing():
		ap.play(nm)


## 步频倍率：改速度不滑步；速度 <=0 时锁 1.0（静止但动画不冻在第一帧，plan.md R5）。
func _Update_Anim_Speed() -> void:
	_anim_scale = 1.0 if walk_speed <= 0.0 else walk_speed / CLIP_MPS
	var ap: AnimationPlayer = _Ensure_Anim_Player()
	if ap != null:
		ap.speed_scale = _anim_scale


func _Slide_Risk() -> bool:
	return absf(walk_speed - CLIP_MPS) > CLIP_MPS * SPEED_TOL_RATIO


func _Warn_Once(key: String, msg: String) -> void:
	if _warned.has(key):
		return
	_warned[key] = true
	# 一律 push_warning（绝不 push_error）：--quit 的 "0 ERROR" 验收会被 ERROR 搞假红。
	push_warning(msg)


# ---------------------------------------------------------------- 几何（矩形净空）

## 线段 (a, b) 是否与 XY 轴对齐矩形 [x0,x1]×[y0,y1] 相交（slab 法；含端点接触）。
func _Segment_Hits_Aabb(a: Vector3, b: Vector3, x0: float, x1: float,
		y0: float, y1: float) -> bool:
	var t_min: float = 0.0
	var t_max: float = 1.0
	var dx: float = b.x - a.x
	var dy: float = b.y - a.y
	for axis: Array in [[a.x, dx, x0, x1], [a.y, dy, y0, y1]]:
		var p: float = axis[0]
		var d: float = axis[1]
		var lo: float = axis[2]
		var hi: float = axis[3]
		if absf(d) < 1.0e-9:
			if p < lo or p > hi:
				return false
			continue
		var t1: float = (lo - p) / d
		var t2: float = (hi - p) / d
		if t1 > t2:
			var tmp: float = t1
			t1 = t2
			t2 = tmp
		t_min = maxf(t_min, t1)
		t_max = minf(t_max, t2)
		if t_min > t_max:
			return false
	return true
