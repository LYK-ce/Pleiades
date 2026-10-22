# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# 四旋翼底盘 quadrotor_uav（Step 7 / COMPONENTS.md §4.6）：
#   CharacterBody3D + MOTION_MODE_FLOATING（D3）+ 四旋翼运动学 + 世界侧速度环。
#
# 为什么 FLOATING 就是"无重力悬停"（§4.6 / D3）：
#   本文件**从不**施加任何重力（不调 get_gravity / apply_gravity / 手写 y -= g*dt），
#   CharacterBody3D 也不会自动加重力 → 没有竖向指令时竖直速度恒为 0，机身悬在原高度。
#   竖直自由度完全由 up/down 指令给出（速度环 -> 爬升率），与 default_gravity 无关。
#
# 控制接口与真机一致（§6.3-1）：只吃 motion_cmd{action, speed(百分比)}，
#   速度环在底盘内部跑 —— sim 侧的控制器组件与真机完全相同。
#
# 分工（D8）：本文件只产出**控制量** {velocity(全局), yaw_rate(rotation.y 角速度)}；
#   位置一律由 Vehicle._physics_process() 推进（唯一位置更新点）。
#   因此这里不写 position、不调物理推进 API、不做堵转检测。
#
# action 语义（§4.6；一次 motion_cmd 只有**一个** action，所以不会"边前进边爬升"）：
#   forward  -> 水平 v_target = +max_speed·pct          （竖直 0）
#   backward -> 水平 v_target = -max_speed·pct          （竖直 0）
#   left     -> 原地偏航 yaw_target = +max_yaw_rate·pct （水平速度 0，同 UGV）
#   right    -> 原地偏航 yaw_target = -max_yaw_rate·pct
#   up       -> 爬升 = +max_climb_rate·pct              （水平速度 0）
#   down     -> 爬升 = -max_climb_rate·pct              （水平速度 0）
#   stop / 未知 / 缺 action -> 三轴目标 0（= 悬停）
#   单 action 的直接后果：从 forward 切到 up，水平速度会按 3 m/s² 斜坡降到 0（约 0.65 m 漂移）。
#
# 坐标：Godot Y-up，前进 = 局部 +X，局部 +Z = 机头右侧；速度 = global_basis * (v, 0, 0)，
#   再把 y 分量换成爬升率（Godot +y = 世界 +z = 高度，**不换轴**）。
# yaw_rate 是 **Godot rotation.y** 的角速度（正 = 左转），世界 yaw = -rotation.y。
#
# 不声明全局类名（§9.1 E5）。

extends "res://scripts/chassis/ChassisModel.gd"


## 零件名（hello / motion 里用的 chassis 标识）。
const CHASSIS_NAME := "quadrotor_uav"
## 机身厚度（米）：碰撞形状 = BoxShape3D(size, BODY_HEIGHT, size)，原点 = 底面中心。
const BODY_HEIGHT := 0.12
## 旋翼盘面比机顶再高 0.01 m（纯视觉，不参与物理）。
const ROTOR_LIFT := 0.01
## 旋翼节点名（与 chassis/Quadrotor.tscn 一一对应；顺序 = 左前/右前/左后/右后）。
const ROTOR_NAMES := ["RotorFL", "RotorFR", "RotorRL", "RotorRR"]


# 生效参数（Configure 从 params 读，带默认回退）
var _max_speed := 2.0              # m/s（水平）
var _max_climb_rate := 1.0         # m/s（竖直，Godot +y）
var _max_yaw_rate := PI * 0.5      # rad/s（= 90 °/s）
var _size := 0.40                  # m（机身水平边长，X = Z = size）

# 速度环状态（世界侧）：当前值与目标值，按加速度上限斜坡逼近
var _speed := 0.0                  # 当前水平速度（局部 +X 正向，m/s）
var _climb := 0.0                  # 当前爬升率（Godot +y，m/s）
var _yaw_rate := 0.0               # 当前 rotation.y 角速度（rad/s，正 = 左转）
var _speed_target := 0.0
var _climb_target := 0.0
var _yaw_target := 0.0

var _warned_rotors := false        # 旋翼缺失只告警一次（每帧刷屏没有意义）


# ================================================================ 契约

## 默认参数：与 ChassisCatalog.DEFAULTS["quadrotor_uav"] 逐键一致（自检 A6 比对）。
## 入树前也会被 Vehicle._merge_params() 调用 → 只返回常量，不碰场景树。
func GetDefaultParams() -> Dictionary:
	return {
		"max_speed": 2.0,
		"max_climb_rate": 1.0,
		"max_yaw_rate": 90.0,
		"size": 0.40,
	}


## 装配：参数夹到合法域 -> 三轴速度环清零 -> FLOATING -> 车身尺寸 -> 旋翼摆位。
func Configure(p_vehicle: Node3D, p_params: Dictionary) -> void:
	super.Configure(p_vehicle, p_params)
	_max_speed = maxf(GetParam("max_speed", 2.0), 0.0)
	_max_climb_rate = maxf(GetParam("max_climb_rate", 1.0), 0.0)
	_max_yaw_rate = maxf(RateToRad(GetParam("max_yaw_rate", 90.0)), 0.0)
	_size = maxf(GetParam("size", 0.40), MIN_SIZE)
	# 换底盘不继承旧状态：三轴一律清零（新底盘因此"从未收到过指令"）
	_speed = 0.0
	_climb = 0.0
	_yaw_rate = 0.0
	_speed_target = 0.0
	_climb_target = 0.0
	_yaw_target = 0.0
	# 空中运动：FLOATING（UAV 用 FLOATING、UGV 用 GROUNDED，D3）
	ApplyMotionMode(CharacterBody3D.MOTION_MODE_FLOATING)
	# 车身/碰撞形状唯一入口：只传尺寸，抬升由 Vehicle 负责（绝不双重抬升）
	ApplyBodySize(Vector3(_size, BODY_HEIGHT, _size))
	_layout_rotors()


## 指令 -> 三轴速度环目标（只存最新，不排队；pct 缺失/非法一律 0）。
## 单 action：任何一个 action 都先把三轴目标全部清零，再只设其中一轴。
func SetCommand(cmd: Dictionary) -> void:
	super.SetCommand(cmd)
	var pct := SpeedPct(cmd)
	_speed_target = 0.0
	_climb_target = 0.0
	_yaw_target = 0.0
	match ActionOf(cmd):
		"forward":
			_speed_target = _max_speed * pct
		"backward":
			_speed_target = -_max_speed * pct
		"left":
			# 原地偏航：与 UGV 同规则（§4.6），水平速度保持 0
			_yaw_target = _max_yaw_rate * pct
		"right":
			_yaw_target = -_max_yaw_rate * pct
		"up":
			_climb_target = _max_climb_rate * pct
		"down":
			_climb_target = -_max_climb_rate * pct
		_:
			# stop / 未知 action / 缺 action：三轴目标 0 = 悬停（高度保持）
			pass


## 世界侧速度环 + 四旋翼运动学：返回控制量，不改位置（D8）。
## current_velocity 本 step 不使用（开环斜坡；撞墙/触地后引擎自己会把 velocity 归零）。
func Step(_current_velocity: Vector3, delta: float) -> Dictionary:
	var dt := maxf(delta, 0.0)                                      # 暂停 / 0 帧不推进
	_speed = Approach(_speed, _speed_target, MAX_ACCEL_MPS2 * dt)
	# 竖直同斜率：复用基类的两个加速度常量，不新增参数（参数集固定为 §7.2 / §7.6）
	_climb = Approach(_climb, _climb_target, MAX_ACCEL_MPS2 * dt)
	_yaw_rate = Approach(_yaw_rate, _yaw_target, RateToRad(MAX_YAW_ACCEL_DPS2) * dt)

	var speed := clampf(_speed, -_max_speed, _max_speed)
	var climb := clampf(_climb, -_max_climb_rate, _max_climb_rate)
	var yaw_rate := clampf(_yaw_rate, -_max_yaw_rate, _max_yaw_rate)
	# 树外（自检直接 new 出来调 Step）不能访问 global_*，否则 ERROR（Step 5 的 P8）
	var basis_now: Basis = global_basis if is_inside_tree() else basis
	var vel: Vector3 = basis_now * Vector3(speed, 0.0, 0.0)
	vel.y = climb                                                    # FLOATING：竖直分量 = 爬升率
	return {"velocity": vel, "yaw_rate": yaw_rate}


# ================================================================ 查询（自检 / 调试用，不改变状态）

## 当前水平速度（local +X 正向，m/s）。
func GetSpeed() -> float:
	return clampf(_speed, -_max_speed, _max_speed)


## 当前爬升率（Godot +y，m/s；负 = 下降）。
func GetClimbRate() -> float:
	return clampf(_climb, -_max_climb_rate, _max_climb_rate)


## 当前 rotation.y 角速度（rad/s，正 = 左转）。
func GetYawRate() -> float:
	return clampf(_yaw_rate, -_max_yaw_rate, _max_yaw_rate)


## 生效车身尺寸（米，Vector3(size, BODY_HEIGHT, size)）。
func GetBodySize() -> Vector3:
	return Vector3(_size, BODY_HEIGHT, _size)


# ================================================================ 私有

## 旋翼摆位：x = ±size/2（前 +，后 −）、z = ±size/2（左 −，右 +）、
##   y = BODY_HEIGHT + ROTOR_LIFT（贴着机顶上方一点）。
## 纯视觉，不参与物理；子节点缺失只告警一次并跳过（不崩）。
func _layout_rotors() -> void:
	var rotors: Node3D = get_node_or_null("Rotors") as Node3D
	if rotors == null:
		_warn_rotors("Rotors 容器缺失")
		return
	var missing: Array[String] = []
	for i in range(ROTOR_NAMES.size()):
		var rotor_name: String = ROTOR_NAMES[i]
		var rotor: Node3D = rotors.get_node_or_null(rotor_name) as Node3D
		if rotor == null:
			missing.append(rotor_name)
			continue
		var x := _size * 0.5 if i < 2 else -_size * 0.5
		var z := -_size * 0.5 if (i % 2) == 0 else _size * 0.5
		rotor.position = Vector3(x, BODY_HEIGHT + ROTOR_LIFT, z)
	if not missing.is_empty():
		_warn_rotors("旋翼节点缺失：%s" % str(missing))


func _warn_rotors(message: String) -> void:
	if _warned_rotors:
		return
	_warned_rotors = true
	push_warning("[quadrotor] %s（视觉件缺失不影响运动学）" % message)
