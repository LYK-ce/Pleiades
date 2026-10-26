# Presented by KeJi
# Created Date ： 2026-10-25
# Modified Date ： 2026-10-25
#
# 行人底盘 pedestrian（task_35 Step 1 / COMPONENTS.md §4.11）：
#   CharacterBody3D + MOTION_MODE_GROUNDED + "线速度 + 世界朝向角" 的平面运动学。
#
# 为什么它是"底盘"而不是 Vehicle：行人没有轮式外观、没有 Chassis/Sensors 挂载点，
#   但外部脚本要用**与车同一套** chassis_hello / motion_cmd / pose 协议驱动它，
#   所以它实现同一个 ChassisModel 契约（Configure / GetDefaultParams / SetCommand / Step）。
#
# 与本文件相关的三条硬边界：
#   ① 本文件**不写 position**：位置只由 _physics_process() 的 move_and_slide() 推进（D8）。
#   ② 本文件不碰网络 / 传感器 / 世界几何；pose 由 Step 2 的 manager 调 GetPose() 取。
#   ③ 找不到动画只 push_warning（--quit 的 0 ERROR 验收靠这条）。
#
# 指令语义（task_35 §二 Step 2 定义的 motion_cmd 字段，Step 1 先把语义落进底盘）：
#   v   : 线速度 m/s，clamp 0~max_speed（行人**不倒着走**：负值一律 0）
#   dir : 世界系朝向角（弧度，0 = +X 东，逆时针为正）；内部一律存 rotation.y = -dir
#   v 与 dir **各自独立**：{v:1.2} 只改速度（保持原朝向直着走）；{dir:1.57} 只改朝向。
#   缺 v/dir 时回退老协议 action/speed：
#     forward       -> v = max_speed * speed/100
#     left / right  -> **只改朝向**（v 不动，所以"边走边转"不用两条指令）
#     stop / backward / 未知 / 缺 action -> v = 0
#
# 为什么**没有**加速度斜坡（与 DiffDrive 的 Approach() 不同，是有意为之）：
#   step.md 要求 v=0 **立刻**归零、dir 阶跃第一帧就到 max_turn_rate（180 °/s）；
#   人的起停没有"世界侧速度环"这层机械，硬加斜坡会让本 step 的
#   v-clamp / dir-rate-limit / dir-step-180 三条断言全红（plan.md §6 R6）。
#
# 朝向口径（最容易抄反的一处）：世界 yaw 逆时针为正（+X -> +Y），rotation.y = -yaw。
#   Step() 返回的 yaw_rate 是 **Godot rotation.y** 的角速度（正 = 左转）。
#   用 rotation.y 算误差的前提 = 父节点变换为恒等（Step 3 的 World/Pedestrians 是新建 Node3D，
#   恒等），与 Vehicle 的假设完全一致。
#
# 不声明全局类名（COMPONENTS.md §9.1 E5）。

extends "res://src/chassis/ChassisModel.gd"

const WorldTransform := preload("res://src/common/WorldTransform.gd")

## 目录名（ChassisCatalog.SCENES / DEFAULTS 的键）。
const CHASSIS_NAME := "pedestrian"
## 资产自带步速（m/s）：speed_scale 的基准，与 Person.gd::CLIP_MPS 同值同口径。
const CLIP_MPS := 1.26
## 默认最大线速度（m/s）= CLIP_MPS：走得跟动画一样快，脚不滑。
const DEFAULT_MAX_SPEED := 1.26
## 默认最大转向角速度（度/秒）：与目录表同口径（内部 rad/s）。
const DEFAULT_MAX_TURN_RATE_DPS := 180.0

# ---------------------------------------------------------------- 生效参数
var _max_speed := DEFAULT_MAX_SPEED          # m/s（params.max_speed 覆盖；clamp 上限）
var _max_turn_rate := PI                     # rad/s（params.max_turn_rate 单位是**度/秒**）

# ---------------------------------------------------------------- 运动状态
var _v_target := 0.0        # 目标线速度（m/s，SetCommand 写，clamp 0.._max_speed）
var _speed_now := 0.0       # Step 算出的生效线速度（Play_Walk / Get_Speed 用）
var _yaw_target := 0.0      # 目标 rotation.y（= -世界 yaw，SetCommand 写）
var _legacy_turn := 0.0     # 老协议 action=left/right 的转向意图（+1/-1 × pct，0 = 不转）

var _anim: AnimationPlayer = null
var _warned: Dictionary = {}


# ================================================================ 契约

## 默认参数：与 ChassisCatalog.DEFAULTS["pedestrian"] **逐键一致**（自检 B4 比对）。
## 入树前也会被调用 → 只返回常量，不碰场景树。
func GetDefaultParams() -> Dictionary:
	return {"max_speed": DEFAULT_MAX_SPEED, "max_turn_rate": DEFAULT_MAX_TURN_RATE_DPS}


## 装配：参数夹到合法域 -> 运动状态清零 -> GROUNDED。
## **不调 ApplyBodySize()**：胶囊是场景给的形状（r0.25 / h1.75 / 中心 y0.875），
##   行人不是 Vehicle，没有"模型 = 碰撞盒"那套重摆逻辑。
func Configure(p_vehicle: Node3D, p_params: Dictionary) -> void:
	super.Configure(p_vehicle, p_params)
	_max_speed = maxf(GetParam("max_speed", DEFAULT_MAX_SPEED), 0.0)
	_max_turn_rate = maxf(RateToRad(GetParam("max_turn_rate", DEFAULT_MAX_TURN_RATE_DPS)), 0.0)
	# 换底盘/重装配不继承旧状态
	_v_target = 0.0
	_speed_now = 0.0
	_legacy_turn = 0.0
	_yaw_target = rotation.y
	ApplyMotionMode(CharacterBody3D.MOTION_MODE_GROUNDED)


## 指令 -> 目标（只存最新，不排队）。v / dir 各路独立解析，缺省回退 action/speed。
func SetCommand(cmd: Dictionary) -> void:
	super.SetCommand(cmd)
	var action := ActionOf(cmd)

	# ---- 线速度
	if cmd.has("v"):
		var raw_v: float = _to_float(cmd["v"], 0.0)
		_v_target = clampf(raw_v, 0.0, _max_speed) if is_finite(raw_v) else 0.0
	else:
		match action:
			"forward":
				_v_target = _max_speed * SpeedPct(cmd)
			"left", "right":
				pass                       # 「只改 dir」：v 不动
			_:
				_v_target = 0.0            # stop / backward / 未知 / 缺 action

	# ---- 朝向（世界系弧度 -> rotation.y = -dir）
	if cmd.has("dir"):
		var dir_w: float = _Parse_Angle(cmd["dir"])
		if is_finite(dir_w):
			_yaw_target = wrapf(-dir_w, -PI, PI)
			_legacy_turn = 0.0
		else:
			_Warn_Once("bad-dir", "[pedestrian] motion_cmd.dir 非法（%s），保留上一目标朝向"
				% str(cmd["dir"]))
	else:
		match action:
			"left":
				_legacy_turn = SpeedPct(cmd)
			"right":
				_legacy_turn = -SpeedPct(cmd)
			_:
				_legacy_turn = 0.0


## 纯控制量：{velocity: Vector3(全局), yaw_rate: float(rotation.y 角速度，正 = 左转)}。
## **不写 position、不调物理推进 API、树外可调**（树外走 basis 而非 global_basis，否则 ERROR）。
func Step(_current_velocity: Vector3, delta: float) -> Dictionary:
	var dt := maxf(delta, 1.0e-6)                                 # 0 帧不除零
	if not is_zero_approx(_legacy_turn):
		# 老协议 left/right：目标朝向自己往前推 max_turn_rate（rad/s），不是 Instant 转过去
		_yaw_target = wrapf(_yaw_target - _legacy_turn * _max_turn_rate * dt, -PI, PI)
	var err := wrapf(_yaw_target - rotation.y, -PI, PI)            # 最短角差（Godot rotation.y）
	var step := clampf(err, -_max_turn_rate * dt, _max_turn_rate * dt)   # 限速、不过冲
	_speed_now = clampf(_v_target, 0.0, _max_speed)                # v clamp 0~max_speed
	var basis_now: Basis = global_basis if is_inside_tree() else basis   # 树外不能碰 global_*
	var vel: Vector3 = basis_now * Vector3(_speed_now, 0.0, 0.0)   # 沿**当前** yaw 前进
	vel.y = 0.0                                                   # 平面运动：不跟重力抢 y
	return {"velocity": vel, "yaw_rate": step / dt}


## 走路动画：播**第一条** clip（不硬编码名字）、loop、步频 = v / CLIP_MPS。
## v = 0 时 speed_scale = 0（人物定格，不闪不跳，但仍 is_playing）。
## 找不到 AnimationPlayer / 无 clip：**只 push_warning（每类一次），绝不 push_error**。
func Play_Walk() -> void:
	var ap := _Ensure_Anim()
	if ap == null:
		return
	var list: PackedStringArray = ap.get_animation_list()
	if list.is_empty():
		_Warn_Once("no-clip", "[pedestrian] AnimationPlayer 里没有任何动画")
		return
	var clip_name := String(list[0])
	if ap.current_animation != clip_name or not ap.is_playing():
		var anim: Animation = ap.get_animation(clip_name)
		if anim != null and anim.loop_mode == Animation.LOOP_NONE:
			anim.loop_mode = Animation.LOOP_LINEAR
		ap.play(clip_name)
	ap.speed_scale = 0.0 if _speed_now <= 0.0 else _speed_now / CLIP_MPS


# ================================================================ 自宿主推进（唯一位置更新点）

## 位移只在这里发生：Step() 给控制量 -> 转 yaw -> 交给 move_and_slide()。
## 宿主是别的节点（Vehicle 拨给行人用的情形）时**不重复推进**：那种情况由宿主自己 Step。
func _physics_process(delta: float) -> void:
	if vehicle != null and vehicle != self:
		return
	var body := _Body()
	if body == null:
		return
	var ctl: Dictionary = Step(body.velocity, delta)
	rotation.y += float(ctl.get("yaw_rate", 0.0)) * delta
	body.velocity = ctl.get("velocity", Vector3.ZERO)
	body.move_and_slide()
	Play_Walk()


# ================================================================ 查询（自检 / 调试 / Step 2 的 pose 下行）

## 位姿 {x, y, z, yaw}：世界坐标（Z-up，米），yaw 归一化到 [-PI, PI)。口径与 Vehicle.GetPose() 相同。
func GetPose() -> Dictionary:
	# 树外读 global_transform 会 ERROR（与 check_models.gd 头注释 P8 同款）
	return WorldTransform.PoseFromTransform(global_transform if is_inside_tree() else transform)


## 当前生效线速度（m/s）。
func Get_Speed() -> float:
	return _speed_now


## 目标朝向的世界 yaw（弧度，0 = +X，逆时针为正）。
func Get_Dir_Target_W() -> float:
	return wrapf(-_yaw_target, -PI, PI)


## 转向角速度上限（rad/s）。
func Get_Yaw_Rate_Limit() -> float:
	return _max_turn_rate


## 模型根（glb 实例）；缺失返回 null（自检据此报红）。
func Get_Body() -> Node3D:
	return get_node_or_null("Body") as Node3D


## 根节点**必须**经 Node 中转再 as：Node3D 基类的脚本直接 `self as CharacterBody3D`
## 是 **Parse Error**（plan.md §1.5 实测），这条中转不是为了好看。
func _Body() -> CharacterBody3D:
	var n: Node = self
	return n as CharacterBody3D


# ================================================================ 私有

## 宽容的角度解析：合法返回弧度（可任意实数），非法/NaN/inf 返回 NAN（调用方保留旧目标）。
func _Parse_Angle(value: Variant) -> float:
	var parsed := NAN
	match typeof(value):
		TYPE_INT, TYPE_FLOAT:
			parsed = float(value)
		TYPE_STRING, TYPE_STRING_NAME:
			var text := str(value).strip_edges()
			if text.is_valid_float():
				parsed = text.to_float()
		_:
			parsed = NAN
	return parsed if is_finite(parsed) else NAN


## 找 AnimationPlayer：优先按 step.md 的字面调用（owned=false 才看得见 glb 实例**内部**的节点），
## 再按**类型**递归兜底（导入器给的节点名会随版本变）。
func _Ensure_Anim() -> AnimationPlayer:
	if _anim != null and is_instance_valid(_anim):
		return _anim
	var by_name: Node = find_child("AnimationPlayer", true, false)
	if by_name is AnimationPlayer:
		_anim = by_name as AnimationPlayer
	if _anim == null:
		_anim = _Find_Anim_Player(self)
	if _anim == null:
		# 缺动画不是致命错误（行人照样能走，只是腿不动）：只 warning，绝不 push_error。
		_Warn_Once("no-anim", "[pedestrian] 子树里找不到 AnimationPlayer（走路 glb 是否已导入？）")
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


## 每类告警只出一次（每帧刷屏没有意义）。
func _Warn_Once(key: String, message: String) -> void:
	if _warned.has(key):
		return
	_warned[key] = true
	push_warning(message)
