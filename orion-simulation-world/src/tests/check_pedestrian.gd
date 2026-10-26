# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# 行人底盘 pedestrian 自检（task_35 Step 1）：场景形态 / 目录装配 / SetCommand->Step 控制量 /
# 老协议 action/speed 回退 / 真 World 自宿主推进 + 走路动画 / 白盒边界。
# 用法：timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
#          --script res://src/tests/check_pedestrian.gd
# 期望：[check] total=40 failed=0 + SELF-CHECK OK + EXIT=0，且日志无 ERROR 行。
#
# 决定本文件写法的实测事实（plan.md §1.5 / §6）：
#   P1  CharacterBody3D 根节点可以挂 Node3D 基类的脚本；但 `self as CharacterBody3D` 在
#       **Pedestrian.gd 内部**是 Parse Error，必须经 Node 中转（那是被测代码的事，本文件不涉及）。
#   P2  `Step()` 是**纯控制量**、树外可调：树外调它不会出 !is_inside_tree / body->get_space ERROR。
#       控制量与推进分家是硬约定 —— 本文件 C/D 组全部在树外调 Step，E 组才进真 World。
#   P3  GROUNDED 是 CharacterBody3D 的默认值：断言"Configure 真的设过"靠先置 FLOATING 再比对。
#   P6  未入树访问 global_position 会报 ERROR -> 一律先 add_child 再设 global_position。
#   P15 含 await 的辅助函数必须被 await 调用；本文件除 _wait_physics 外都不含 await。
#   P16 一次 null 解引用会让协程中断、quit() 不执行 -> 入口先判空，命令带 timeout。
#
# 数值口径：DT = 1/60（--fixed-fps 60），max_turn_rate = PI rad/s（= 180 °/s），
#   每帧最大转角 = PI/60 = 0.0523599 rad。所有期望值都是解析值，容差取 ±1e-3 ~ ±10%。

extends SceneTree

const WorldTransform := preload("res://src/common/WorldTransform.gd")

const PED_SCENE := "res://src/scenes/props/Pedestrian.tscn"
const PED_SCRIPT := "res://src/chassis/Pedestrian.gd"
const BASE_SCRIPT := "res://src/chassis/ChassisModel.gd"
const CATALOG_SCRIPT := "res://src/chassis/ChassisCatalog.gd"
const WORLD_SCENE := "res://src/scenes/World.tscn"

const CHASSIS_NAME := "pedestrian"
const TOL := 1.0e-3
## dt = 1/60；每帧最大转角（rad）。
const DT := 1.0 / 60.0
const MAX_STEP_RAD := PI / 60.0
## 世界坐标（Z-up）出生点：与 check_diff_drive / check_vehicle 同一块空地。
const SPAWN_WORLD := Vector3(64.0, 64.0, 0.0)
## 60 帧直行位移：1.26 m/s × 1 s（规划期实测 1.260223）。
const WANT_WALK_60 := 1.2602
## 目录表 / GetDefaultParams() 的期望值（逐键）。
const DEFAULT_PARAMS := {"max_speed": 1.26, "max_turn_rate": 180.0}

## 白盒 lint：Pedestrian.gd 里**不允许**出现的符号（越界 / 违反 D8 / 会把 0 ERROR 验收搞红）。
const BANNED_TOKENS := ["global_position", "position =", "push_error"]

var _total: int = 0
var _failed: int = 0
var _harness_errors: int = 0


func _initialize() -> void:
	_run()


func _run() -> void:
	# ============================================================ A. 树外：场景形态（7 项）
	var packed: PackedScene = load(PED_SCENE) as PackedScene
	if packed == null:
		_fatal("scene-loads", PED_SCENE)
		_finish()
		return
	var probe: Node = packed.instantiate()
	if probe == null:
		_fatal("scene-instantiates", PED_SCENE)
		_finish()
		return
	_check("scene-loads", true, "%s -> %s" % [PED_SCENE, probe.get_class()])

	# A2 根节点：名字 / 类型（CharacterBody3D）/ 脚本
	var probe_script: Script = probe.get_script()
	_check("root-type", probe.name == "Pedestrian" and probe is CharacterBody3D
		and probe_script != null and probe_script.resource_path == PED_SCRIPT,
		"name=%s class=%s script=%s" % [probe.name, probe.get_class(),
			probe_script.resource_path if probe_script != null else "null"])

	# A3 碰撞层：layer=2（雷达 scan_mask 含 2）、mask=5=1|4（地面 + 车辆）
	var probe_body: CharacterBody3D = probe as CharacterBody3D
	var layers_ok := false
	var layers_detail := "root 不是 CharacterBody3D"
	if probe_body != null:
		layers_ok = probe_body.collision_layer == 2 and probe_body.collision_mask == 5 \
			and (probe_body.collision_mask & 4) != 0 and (probe_body.collision_mask & 1) != 0
		layers_detail = "layer=%d mask=%d" % [probe_body.collision_layer, probe_body.collision_mask]
	_check("collision-layers", layers_ok, layers_detail)

	# A4 胶囊：r=0.25 / h=1.75 / 中心 y=0.875（底面正好在脚底 y=0）
	var cs: CollisionShape3D = probe.get_node_or_null("CollisionShape3D") as CollisionShape3D
	var capsule_ok := false
	var capsule_detail := "CollisionShape3D=%s" % cs
	if cs != null and cs.shape is CapsuleShape3D:
		var cap: CapsuleShape3D = cs.shape as CapsuleShape3D
		capsule_ok = absf(cap.radius - 0.25) < 1.0e-4 and absf(cap.height - 1.75) < 1.0e-4 \
			and cs.position.distance_to(Vector3(0.0, 0.875, 0.0)) < 1.0e-4
		capsule_detail = "r=%.4f h=%.4f pos=%s" % [cap.radius, cap.height, cs.position]
	_check("capsule-shape", capsule_ok, capsule_detail)

	# A5 整个子树只有 1 个 CollisionShape3D（glb 不带形状；两套碰撞体会让人被自己卡住）
	var shape_count: int = _count_shapes(probe)
	_check("physics-shape-count", shape_count == 1, "shapes=%d (want=1)" % shape_count)

	# A6 Body = glb 实例：存在、是 Node3D、**不是** PhysicsBody3D、变换恒等（原点 = 脚底中心）
	var glb: Node3D = probe.get_node_or_null("Body") as Node3D
	var glb_ok: bool = glb != null and not (glb is PhysicsBody3D) \
		and glb.transform.is_equal_approx(Transform3D.IDENTITY)
	_check("body-glb", glb_ok, "Body=%s class=%s xform=%s" % [glb,
		glb.get_class() if glb != null else "null",
		glb.transform if glb != null else "null"])

	# A7 glb 内嵌 AnimationPlayer + 至少 1 条 clip（find_child owned=false 才看得见实例内部）
	var ap: Node = probe.find_child("AnimationPlayer", true, false)
	var clips_ok := ap is AnimationPlayer \
		and (ap as AnimationPlayer).get_animation_list().size() >= 1
	_check("anim-player-clips", clips_ok, "player=%s clips=%s" % [ap,
		(ap as AnimationPlayer).get_animation_list() if ap is AnimationPlayer else "n/a"])

	# ============================================================ B. 目录装配（6 项）
	var catalog: Script = load(CATALOG_SCRIPT) as Script
	var has_ped: bool = bool(catalog.call("Has", CHASSIS_NAME)) if catalog != null else false
	var has_bogus: bool = bool(catalog.call("Has", "no_such_chassis")) if catalog != null else true
	var bogus_defaults: Variant = null
	if catalog != null:
		bogus_defaults = catalog.call("GetDefaults", "no_such_chassis")
	_check("catalog-has", catalog != null and has_ped and not has_bogus
		and bogus_defaults is Dictionary and (bogus_defaults as Dictionary).is_empty(),
		"ped=%s bogus=%s defaults=%s" % [has_ped, has_bogus, bogus_defaults])

	var scene_a: Variant = catalog.call("GetScene", CHASSIS_NAME) if catalog != null else null
	_check("catalog-scene", scene_a is PackedScene
		and (scene_a as PackedScene).resource_path == PED_SCENE,
		"scene=%s path=%s" % [scene_a,
			(scene_a as PackedScene).resource_path if scene_a is PackedScene else "null"])

	var scene_b: Variant = catalog.call("GetScene", CHASSIS_NAME) if catalog != null else null
	_check("catalog-scene-cache", scene_a is PackedScene and scene_b is PackedScene
		and scene_a == scene_b, "a=%s b=%s same=%s" % [scene_a, scene_b, scene_a == scene_b])

	var defs_any: Variant = probe.call("GetDefaultParams")
	var defs: Dictionary = defs_any if defs_any is Dictionary else {}
	var catalog_defs: Variant = catalog.call("GetDefaults", CHASSIS_NAME) if catalog != null else null
	var catalog_dict: Dictionary = catalog_defs if catalog_defs is Dictionary else {}
	_check("default-params", _dict_equal(defs, DEFAULT_PARAMS)
		and _dict_equal(defs, catalog_dict),
		"obj=%s catalog=%s want=%s" % [defs, catalog_dict, DEFAULT_PARAMS])

	var has_dd: bool = bool(catalog.call("Has", "diff_drive_ugv")) if catalog != null else false
	var has_quad: bool = bool(catalog.call("Has", "quadrotor_uav")) if catalog != null else false
	_check("catalog-others-intact", has_dd and has_quad,
		"diff_drive_ugv=%s quadrotor_uav=%s" % [has_dd, has_quad])

	var methods_ok: bool = probe.has_method("Configure") and probe.has_method("GetDefaultParams") \
		and probe.has_method("SetCommand") and probe.has_method("Step")
	var base: Script = probe_script.get_base_script() if probe_script != null else null
	_check("contract-methods", methods_ok and base != null and base.resource_path == BASE_SCRIPT,
		"methods=%s base=%s" % [methods_ok, base.resource_path if base != null else "null"])

	probe.free()

	# ============================================================ C. SetCommand{v,dir} -> Step（11 项）
	# C1 直行：v=1.26 / dir=0 -> velocity=(1.26,0,0)、yaw_rate=0
	var ped: Node3D = packed.instantiate() as Node3D
	ped.call("Configure", ped, DEFAULT_PARAMS)
	ped.call("SetCommand", {"v": 1.26, "dir": 0.0})
	var ctl1: Dictionary = _ctl(ped.call("Step", Vector3.ZERO, DT))
	var v1: Vector3 = ctl1.get("velocity", Vector3.ZERO)
	_check("step-straight", v1.distance_to(Vector3(1.26, 0.0, 0.0)) < TOL
		and absf(float(ctl1.get("yaw_rate", 9.0))) < TOL,
		"velocity=%s yaw_rate=%.6f" % [v1, float(ctl1.get("yaw_rate", 0.0))])

	# C2 v 上 clamp：5.0 -> max_speed（1.26）
	ped.call("SetCommand", {"v": 5.0})
	var ctl2: Dictionary = _ctl(ped.call("Step", Vector3.ZERO, DT))
	var v2: Vector3 = ctl2.get("velocity", Vector3.ZERO)
	_check("step-v-clamp-high", absf(v2.length() - 1.26) < TOL, "|v|=%.6f" % v2.length())

	# C3 v 下 clamp：-3.0 -> 0（行人没有倒着走）
	ped.call("SetCommand", {"v": -3.0})
	var ctl3: Dictionary = _ctl(ped.call("Step", Vector3.ZERO, DT))
	var v3: Vector3 = ctl3.get("velocity", Vector3.ZERO)
	_check("step-v-clamp-low", v3.length() < TOL, "|v|=%.6f" % v3.length())

	# C4 v=0 -> 完全静止（无加速度斜坡，Step 当帧就归零）
	ped.call("SetCommand", {"v": 0.0})
	var ctl4: Dictionary = _ctl(ped.call("Step", Vector3.ZERO, DT))
	var v4: Vector3 = ctl4.get("velocity", Vector3.ZERO)
	_check("step-v-zero", v4.is_zero_approx()
		and absf(float(ctl4.get("yaw_rate", 9.0))) < TOL, "velocity=%s" % [v4])

	# C5 v 非数字：不崩、当 0
	ped.call("SetCommand", {"v": "abc"})
	var ctl5: Dictionary = _ctl(ped.call("Step", Vector3.ZERO, DT))
	var v5: Vector3 = ctl5.get("velocity", Vector3.ZERO)
	_check("step-v-tolerant", v5.length() < TOL and is_instance_valid(ped),
		"|v|=%.6f valid=%s" % [v5.length(), is_instance_valid(ped)])
	ped.free()

	# C6 dir 阶跃：第一帧就到 180 °/s（不是慢慢加）
	var ped6: Node3D = packed.instantiate() as Node3D
	ped6.call("Configure", ped6, DEFAULT_PARAMS)
	ped6.call("SetCommand", {"v": 1.0, "dir": PI / 2.0})
	var ctl6: Dictionary = _integrate(ped6, 1)
	var rate6: float = float(ctl6.get("yaw_rate", 0.0))
	_check("step-dir-rate-limit", absf(rate6 + PI) < PI * 0.01,
		"yaw_rate=%.6f (want≈%.6f)" % [rate6, -PI])

	# C7 30 帧积分到 -PI/2（自检自己当宿主），**第 31 帧**不再转（到位即停，不抖）
	_integrate(ped6, 29)
	var rot7: float = ped6.rotation.y
	var rate7: float = absf(float(_ctl(ped6.call("Step", Vector3.ZERO, DT)).get("yaw_rate", 9.0)))
	_check("step-dir-converge-30f", absf(rot7 + PI / 2.0) < TOL and rate7 < 1.0e-3,
		"rot_y=%.6f (want≈%.6f) 31st_rate=%.6f" % [rot7, -PI / 2.0, rate7])
	ped6.free()

	# C8 dir=PI 阶跃：首帧步长恰好 max_turn_rate*DT、60 帧到位且**不过冲**
	var ped8: Node3D = packed.instantiate() as Node3D
	ped8.call("Configure", ped8, DEFAULT_PARAMS)
	ped8.call("SetCommand", {"dir": PI})
	var rot_before8: float = ped8.rotation.y
	_integrate(ped8, 1)
	var d_rot8: float = absf(ped8.rotation.y - rot_before8)
	_integrate(ped8, 59)
	_check("step-dir-step-180", absf(d_rot8 - MAX_STEP_RAD) < 1.0e-4
		and absf(ped8.rotation.y + PI) < TOL,
		"first_step=%.7f (want=%.7f) rot_y=%.7f (want≈%.7f)" % [d_rot8, MAX_STEP_RAD,
			ped8.rotation.y, -PI])
	ped8.free()

	# C9 小角差**不**饱和：证明是"限速"不是"恒速"
	var ped9: Node3D = packed.instantiate() as Node3D
	ped9.call("Configure", ped9, DEFAULT_PARAMS)
	ped9.call("SetCommand", {"dir": 0.01})
	var rate9: float = float(_ctl(ped9.call("Step", Vector3.ZERO, DT)).get("yaw_rate", 0.0))
	_check("step-small-error-no-saturate", absf(rate9 + 0.01 / DT) < 1.0e-3,
		"yaw_rate=%.6f (want≈%.6f)" % [rate9, -0.01 / DT])
	ped9.free()

	# C10 沿**当前** yaw 前进（世界 +90° 时 Godot 前向 = +Z）
	var ped10: Node3D = packed.instantiate() as Node3D
	ped10.call("Configure", ped10, DEFAULT_PARAMS)
	ped10.rotation.y = -PI / 2.0
	ped10.call("SetCommand", {"v": 1.0, "dir": PI / 2.0})
	var v10: Vector3 = _ctl(ped10.call("Step", Vector3.ZERO, DT)).get("velocity", Vector3.ZERO)
	_check("velocity-along-current-yaw", v10.distance_to(Vector3(0.0, 0.0, 1.0)) < TOL
		and absf(v10.y) < TOL, "velocity=%s" % [v10])
	ped10.free()

	# C11 符号：dir=+PI/2 -> 世界 yaw **正**、rotation.y **负**（抄反了整台人会走反）
	var ped11: Node3D = packed.instantiate() as Node3D
	ped11.call("Configure", ped11, DEFAULT_PARAMS)
	ped11.call("SetCommand", {"v": 0.0, "dir": PI / 2.0})
	_integrate(ped11, 30)
	var target_w: float = float(ped11.call("Get_Dir_Target_W"))
	_check("world-yaw-sign", absf(target_w - PI / 2.0) < TOL
		and ped11.rotation.y < 0.0 and absf(ped11.rotation.y + PI / 2.0) < TOL,
		"dir_target_w=%.6f rot_y=%.6f" % [target_w, ped11.rotation.y])
	ped11.free()

	# ============================================================ D. 老协议 action/speed 回退（5 项）
	var pedd: Node3D = packed.instantiate() as Node3D
	pedd.call("Configure", pedd, DEFAULT_PARAMS)

	# D1 forward + speed=50 -> v = 1.26*0.5
	pedd.call("SetCommand", {"action": "forward", "speed": 50})
	var dv1: Vector3 = _ctl(pedd.call("Step", Vector3.ZERO, DT)).get("velocity", Vector3.ZERO)
	_check("legacy-forward", absf(dv1.length() - 0.63) < TOL, "|v|=%.6f (want≈0.63)" % dv1.length())

	# D2 stop -> 速度和角速度都归零
	pedd.call("SetCommand", {"action": "stop"})
	var dctl2: Dictionary = _ctl(pedd.call("Step", Vector3.ZERO, DT))
	_check("legacy-stop", (dctl2.get("velocity", Vector3.ONE) as Vector3).length() < TOL
		and absf(float(dctl2.get("yaw_rate", 9.0))) < TOL, "ctl=%s" % [dctl2])

	# D3 backward / 未知 action -> 都不动（行人没有倒着走）
	var back_len := -1.0
	var unk_len := -1.0
	pedd.call("SetCommand", {"action": "backward", "speed": 100})
	back_len = (_ctl(pedd.call("Step", Vector3.ZERO, DT)).get("velocity", Vector3.ONE) as Vector3).length()
	pedd.call("SetCommand", {"action": "teleport", "speed": 100})
	unk_len = (_ctl(pedd.call("Step", Vector3.ZERO, DT)).get("velocity", Vector3.ONE) as Vector3).length()
	_check("legacy-backward-unknown", back_len < TOL and unk_len < TOL,
		"backward=%.6f unknown=%.6f" % [back_len, unk_len])

	# D4 left 15 帧 -> 世界 yaw +0.7854（= 180 °/s × 0.25 s）
	pedd.call("SetCommand", {"action": "left", "speed": 100})
	_integrate(pedd, 15)
	var yaw_left: float = -pedd.rotation.y
	_check("legacy-left-turns", absf(yaw_left - PI / 4.0) < PI / 4.0 * 0.02,
		"world_yaw=%.6f (want≈%.6f, ±2%%)" % [yaw_left, PI / 4.0])

	# D5 left **只改朝向**：先 forward 再 left，线速度保持 1.26
	pedd.call("SetCommand", {"action": "forward", "speed": 100})
	pedd.call("Step", Vector3.ZERO, DT)
	pedd.call("SetCommand", {"action": "left", "speed": 100})
	var dv5: Vector3 = _ctl(pedd.call("Step", Vector3.ZERO, DT)).get("velocity", Vector3.ZERO)
	_check("legacy-left-keeps-v", absf(dv5.length() - 1.26) < TOL,
		"|v|=%.6f (want≈1.26)" % dv5.length())
	pedd.free()

	# ============================================================ E. 真 World：自宿主推进 + 动画（8 项）
	var world_packed: PackedScene = load(WORLD_SCENE) as PackedScene
	if world_packed == null:
		_fatal("world-loads", WORLD_SCENE)
		_finish()
		return
	var world: Node3D = world_packed.instantiate() as Node3D
	if world == null:
		_fatal("world-instantiates", WORLD_SCENE)
		_finish()
		return
	root.add_child(world)
	await _wait_physics(2)

	var walker: Node3D = packed.instantiate() as Node3D
	world.add_child(walker)
	walker.global_position = WorldTransform.PosToGodot(SPAWN_WORLD)   # 先入树再写 global_*
	walker.rotation.y = 0.0
	await _wait_physics(5)

	# E1 落地：is_on_floor / y≈0 / 朝向 0
	var wbody: CharacterBody3D = walker as CharacterBody3D
	_check("world-spawn", wbody != null and wbody.is_on_floor()
		and absf(walker.global_position.y) < TOL and absf(walker.rotation.y) < TOL,
		"on_floor=%s pos=%s rot_y=%.6f" % [wbody.is_on_floor() if wbody != null else "n/a",
			walker.global_position, walker.rotation.y])

	# E2 直行 60 帧 -> 1.26 m（±10%），不横向漂、不抬头
	var p0: Vector3 = walker.global_position
	walker.call("SetCommand", {"v": 1.26, "dir": 0.0})
	await _wait_physics(60)
	var dx2: float = walker.global_position.x - p0.x
	var dy2: float = walker.global_position.y - p0.y
	var dz2: float = walker.global_position.z - p0.z
	_check("world-walk-60f", dx2 > WANT_WALK_60 * 0.90 and dx2 < WANT_WALK_60 * 1.10
		and absf(dy2) < TOL and absf(dz2) < TOL and absf(walker.rotation.y) < TOL,
		"dx=%.4f (want≈%.4f) dy=%.6f dz=%.6f rot_y=%.6f" % [dx2, WANT_WALK_60, dy2, dz2,
			walker.rotation.y])

	# E3 转向**限速**：v=0 + dir=PI/2 的第一帧，世界 yaw 只涨一个 max_turn_rate*DT
	var yaw_before3: float = -walker.rotation.y
	walker.call("SetCommand", {"v": 0.0, "dir": PI / 2.0})
	await _wait_physics(1)
	var d_yaw3: float = -walker.rotation.y - yaw_before3
	_check("world-turn-limited-1f", d_yaw3 > 0.05 and d_yaw3 <= MAX_STEP_RAD * 1.02,
		"d_world_yaw=%.7f (limit=%.7f)" % [d_yaw3, MAX_STEP_RAD])

	# E4 再 29 帧（共 30 帧 = 0.5 s）-> 世界 yaw = PI/2 ±2°
	await _wait_physics(29)
	var yaw4: float = -walker.rotation.y
	_check("world-turn-converge-30f", absf(yaw4 - PI / 2.0) < deg_to_rad(2.0),
		"world_yaw=%.6f (want≈%.6f)" % [yaw4, PI / 2.0])

	# E5 v=0 之后 30 帧：真的停住（位置不动、y 不抖）
	var stop_pos: Vector3 = walker.global_position
	walker.call("SetCommand", {"v": 0.0})
	await _wait_physics(30)
	var moved5: float = (walker.global_position - stop_pos).length()
	_check("world-stop-still", moved5 < TOL and absf(wbody.velocity.y) < TOL,
		"moved=%.6f velocity.y=%.6f" % [moved5, wbody.velocity.y])

	# E6 GetPose() 与 WorldTransform 口径逐键一致，yaw ≈ PI/2
	var pose: Dictionary = walker.call("GetPose") if walker.has_method("GetPose") else {}
	var pose_want: Dictionary = WorldTransform.PoseFromTransform(walker.global_transform)
	_check("pose-getter", _dict_equal(pose, pose_want)
		and absf(float(pose.get("yaw", 0.0)) - PI / 2.0) < 0.04,
		"pose=%s want=%s" % [pose, pose_want])

	# E7 走路动画：clip = 第一条、loop、speed_scale = v / 1.26（v=0 定格但仍 is_playing）
	var wap: AnimationPlayer = walker.find_child("AnimationPlayer", true, false) as AnimationPlayer
	var anim_ok := wap != null
	var anim_detail := "player=%s" % wap
	if wap != null:
		var list: PackedStringArray = wap.get_animation_list()
		walker.call("SetCommand", {"v": 1.26})
		walker.call("Step", Vector3.ZERO, DT)
		walker.call("Play_Walk")
		var scale_full: float = wap.speed_scale
		var clip_name := String(wap.current_animation)
		var clip_obj: Animation = wap.get_animation(clip_name)
		var loop_ok: bool = clip_obj != null and clip_obj.loop_mode == Animation.LOOP_LINEAR
		walker.call("SetCommand", {"v": 0.63})
		walker.call("Step", Vector3.ZERO, DT)
		walker.call("Play_Walk")
		var scale_half: float = wap.speed_scale
		walker.call("SetCommand", {"v": 0.0})
		walker.call("Step", Vector3.ZERO, DT)
		walker.call("Play_Walk")
		var scale_zero: float = wap.speed_scale
		anim_ok = not list.is_empty() and clip_name == String(list[0]) and loop_ok \
			and absf(scale_full - 1.0) < 0.01 and absf(scale_half - 0.5) < 0.005 \
			and absf(scale_zero) < 1.0e-6 and wap.is_playing()
		anim_detail = "clip=%s loop=%s scale=%.3f/%.3f/%.3f playing=%s" % [clip_name, loop_ok,
			scale_full, scale_half, scale_zero, wap.is_playing()]
	_check("anim-play-walk", anim_ok, anim_detail)

	# E8 缺动画容错：摘掉 Body 再 Play_Walk 只 warning，不崩、不 push_error
	var orphan: Node3D = packed.instantiate() as Node3D
	var orphan_body: Node = orphan.get_node_or_null("Body")
	if orphan_body != null:
		orphan.remove_child(orphan_body)
		orphan_body.free()
	orphan.call("Play_Walk")
	orphan.call("Play_Walk")
	_check("anim-missing-tolerant", is_instance_valid(orphan)
		and orphan.get_node_or_null("Body") == null, "valid=%s body=%s"
		% [is_instance_valid(orphan), orphan.get_node_or_null("Body")])
	orphan.free()

	# ============================================================ F. 白盒 / 收尾（3 项）
	# F1 只产出控制量：不写 position、不 push_error（D8 + 0 ERROR 验收）
	var ped_code: String = _code_of(PED_SCRIPT)
	var banned_hits: Array[String] = []
	for token: String in BANNED_TOKENS:
		if ped_code.find(token) >= 0:
			banned_hits.append(token)
	_check("white-box-no-position-write", banned_hits.is_empty(),
		"banned=%s" % [banned_hits])

	# F2 唯一位置更新点在 _physics_process 的 move_and_slide
	var mover_count: int = _count_token(ped_code, "move_and_slide")
	_check("white-box-single-mover", mover_count == 1
		and ped_code.find("_physics_process") >= 0,
		"move_and_slide x%d physics_process=%s" % [mover_count,
			ped_code.find("_physics_process") >= 0])

	# F3 全程无自检装置层面的错误
	_check("no-harness-errors", _harness_errors == 0 and is_instance_valid(walker),
		"harness_errors=%d walker_valid=%s" % [_harness_errors, is_instance_valid(walker)])

	# ============================================================ 清理（P12：退出时无 ERROR）
	walker.call("SetCommand", {"v": 0.0})
	walker.queue_free()
	await process_frame
	await process_frame
	world.queue_free()
	await process_frame
	await process_frame
	_finish()


# ================================================================ 测试装置

## 调 Step 并取字典（返回值非字典时给空字典，避免 null 解引用中断协程）。
func _ctl(raw: Variant) -> Dictionary:
	return raw if raw is Dictionary else {}


## 自检自己当宿主：调 Step -> 把 yaw_rate 积分进 rotation.y（与 _physics_process 同序）。
func _integrate(ped: Node3D, frames: int) -> Dictionary:
	var ctl: Dictionary = {}
	for _i in range(maxi(frames, 0)):
		ctl = _ctl(ped.call("Step", Vector3.ZERO, DT))
		ped.rotation.y += float(ctl.get("yaw_rate", 0.0)) * DT
	return ctl


## 子树里 CollisionShape3D 的个数（glb 不带形状，正常应为 1）。
func _count_shapes(node: Node) -> int:
	if node == null:
		return 0
	var total: int = 1 if node is CollisionShape3D else 0
	for child: Node in node.get_children():
		total += _count_shapes(child)
	return total


## 字典逐键相同（键集 + 类型 + 值）；用于默认参数 / GetPose 比对。
func _dict_equal(a: Dictionary, b: Dictionary) -> bool:
	if a.size() != b.size():
		return false
	for key: Variant in a.keys():
		if not b.has(key):
			return false
		if typeof(a[key]) != typeof(b[key]):
			return false
		if a[key] != b[key]:
			return false
	return true


## 去掉以 # 开头的注释行，只留可执行代码（白盒 lint 用）。
func _strip_comment_lines(source: String) -> String:
	var code := ""
	for raw_line: String in source.split("\n"):
		if raw_line.strip_edges().begins_with("#"):
			continue
		code += raw_line + "\n"
	return code


## 读脚本源码并去注释（脚本不存在 -> 空串）。
func _code_of(path: String) -> String:
	var script: Script = load(path) as Script
	if script == null:
		return ""
	return _strip_comment_lines(script.source_code)


## 子串出现次数（白盒 lint：move_and_slide 只能有 1 处）。
func _count_token(code: String, token: String) -> int:
	if token.is_empty():
		return 0
	var count: int = 0
	var from: int = code.find(token)
	while from >= 0:
		count += 1
		from = code.find(token, from + token.length())
	return count


func _wait_physics(frames: int) -> void:
	for _i in range(frames):
		await physics_frame


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
	_harness_errors += 1
	print("[check] ", label, " FAIL ", detail)


func _finish() -> void:
	print("[check] total=", _total, " failed=", _failed)
	if _failed == 0:
		print("SELF-CHECK OK")
		quit(0)
	else:
		print("SELF-CHECK FAILED: ", _failed)
		quit(1)
