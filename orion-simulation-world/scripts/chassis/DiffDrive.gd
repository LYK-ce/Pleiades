# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# 差速底盘 diff_drive_ugv（Step 5 / COMPONENTS.md §4.4）：
#   CharacterBody3D + MOTION_MODE_GROUNDED（D3）+ 差速运动学 + 世界侧速度环。
#
# 控制接口与真机一致（§6.3-1）：只吃 motion_cmd{action, speed(百分比)}，
#   速度环在底盘内部跑 —— sim 侧的控制器组件与真机完全相同。
#
# 分工（D8）：本文件只产出**控制量** {velocity(全局), yaw_rate(rotation.y 角速度)}；
#   位置一律由 Vehicle._physics_process() 推进（唯一位置更新点）。
#   因此这里不写 position、不调物理推进 API、不做堵转检测。
#
# action 语义（§4.4 表，逐行照做；left/right 线速度为 0 = **原地转**，是设计）：
#   forward  -> v_target = +max_speed·pct , yaw_target = 0
#   backward -> v_target = -max_speed·pct , yaw_target = 0
#   left     -> v_target = 0              , yaw_target = +max_turn_rate·pct
#   right    -> v_target = 0              , yaw_target = -max_turn_rate·pct
#   up/down  -> 0（UGV 无竖直自由度）；stop / 未知 / 缺 action -> 0
#
# 坐标：前进 = 局部 +X，局部 +Z = 车体右侧；velocity = global_basis * (v, 0, 0)。
# yaw_rate 是 **Godot rotation.y** 的角速度（正 = 左转），世界 yaw = -rotation.y。
#
# 不声明全局类名（§9.1 E5）。

extends "res://scripts/chassis/ChassisModel.gd"


## 零件名（hello / motion 里用的 chassis 标识）。
const CHASSIS_NAME := "diff_drive_ugv"
## 轮子视觉半径（米）：轮底贴 y=0（Vehicle 原点 = 底面中心）。
const WHEEL_RADIUS := 0.06
## 轮子节点名（与 chassis/DiffDrive.tscn 一一对应；顺序 = 左前/右前/左后/右后）。
const WHEEL_NAMES := ["WheelFL", "WheelFR", "WheelRL", "WheelRR"]


# 生效参数（Configure 从 params 读，带默认回退）
var _max_speed := 1.5              # m/s
var _max_turn_rate := PI * 0.5     # rad/s（= 90 °/s）
var _wheel_base := 0.30            # m（纵向，X）
var _track_width := 0.24           # m（横向，Z）
var _height := 0.16                # m

# 速度环状态（世界侧）：当前值与目标值，按加速度上限斜坡逼近
var _speed := 0.0                  # 当前线速度（局部 +X 正向，m/s）
var _yaw_rate := 0.0               # 当前 rotation.y 角速度（rad/s，正 = 左转）
var _speed_target := 0.0
var _yaw_target := 0.0

var _warned_wheels := false        # 轮子缺失只告警一次（每帧刷屏没有意义）


# ================================================================ 契约

## 默认参数：与 ChassisCatalog.DEFAULTS["diff_drive_ugv"] 逐键一致（自检 A6 比对）。
## 入树前也会被 Vehicle._merge_params() 调用 → 只返回常量，不碰场景树。
func GetDefaultParams() -> Dictionary:
	return {
		"max_speed": 1.5,
		"max_turn_rate": 90.0,
		"wheel_base": 0.30,
		"track_width": 0.24,
		"height": 0.16,
	}


## 装配：参数夹到合法域 -> 速度环清零 -> GROUNDED -> 车身尺寸 -> 轮子摆位。
func Configure(p_vehicle: Node3D, p_params: Dictionary) -> void:
	super.Configure(p_vehicle, p_params)
	_max_speed = maxf(GetParam("max_speed", 1.5), 0.0)
	_max_turn_rate = maxf(RateToRad(GetParam("max_turn_rate", 90.0)), 0.0)
	_wheel_base = maxf(GetParam("wheel_base", 0.30), MIN_SIZE)
	_track_width = maxf(GetParam("track_width", 0.24), MIN_SIZE)
	_height = maxf(GetParam("height", 0.16), MIN_SIZE)
	# 换底盘不继承旧速度：状态一律清零
	_speed = 0.0
	_yaw_rate = 0.0
	_speed_target = 0.0
	_yaw_target = 0.0
	# 地面运动：motion_mode 是 UGV 的语义（UAV 用 FLOATING），floor_snap_length 用默认
	ApplyMotionMode(CharacterBody3D.MOTION_MODE_GROUNDED)
	# 车身/碰撞形状唯一入口：只传尺寸，抬升由 Vehicle 负责（R5：绝不双重抬升）
	ApplyBodySize(Vector3(_wheel_base, _height, _track_width))
	_layout_wheels()


## 指令 -> 速度环目标（只存最新，不排队；pct 缺失/非法一律 0）。
func SetCommand(cmd: Dictionary) -> void:
	super.SetCommand(cmd)
	var pct := SpeedPct(cmd)
	match ActionOf(cmd):
		"forward":
			_speed_target = _max_speed * pct
			_yaw_target = 0.0
		"backward":
			_speed_target = -_max_speed * pct
			_yaw_target = 0.0
		"left":
			_speed_target = 0.0
			_yaw_target = _max_turn_rate * pct
		"right":
			_speed_target = 0.0
			_yaw_target = -_max_turn_rate * pct
		_:
			# stop / up / down / 未知 action / 缺 action：UGV 只认平面运动
			_speed_target = 0.0
			_yaw_target = 0.0


## 世界侧速度环 + 差速运动学：返回控制量，不改位置（D8）。
## current_velocity 本 step 不使用（开环斜坡；撞墙后引擎自己会把 velocity 归零）。
func Step(_current_velocity: Vector3, delta: float) -> Dictionary:
	var dt := maxf(delta, 0.0)                          # 暂停 / 0 帧不推进
	_speed = Approach(_speed, _speed_target, MAX_ACCEL_MPS2 * dt)
	_yaw_rate = Approach(_yaw_rate, _yaw_target, RateToRad(MAX_YAW_ACCEL_DPS2) * dt)
	var speed := clampf(_speed, -_max_speed, _max_speed)
	var yaw_rate := clampf(_yaw_rate, -_max_turn_rate, _max_turn_rate)
	# 树外（自检直接 new 出来调 Step）不能访问 global_*，否则 ERROR（P8）
	var basis_now: Basis = global_basis if is_inside_tree() else basis
	var vel: Vector3 = basis_now * Vector3(speed, 0.0, 0.0)
	vel.y = 0.0                                          # 平面运动学：不爬墙、不跟重力抢 y
	return {"velocity": vel, "yaw_rate": yaw_rate}


# ================================================================ 查询（自检 / 调试用，不改变状态）

## 当前线速度（local +X 正向，m/s）。
func GetSpeed() -> float:
	return clampf(_speed, -_max_speed, _max_speed)


## 当前 rotation.y 角速度（rad/s，正 = 左转）。
func GetYawRate() -> float:
	return clampf(_yaw_rate, -_max_turn_rate, _max_turn_rate)


## 生效车身尺寸（米，Vector3(wheel_base, height, track_width)）。
func GetBodySize() -> Vector3:
	return Vector3(_wheel_base, _height, _track_width)


# ================================================================ 私有

## 轮子摆位：x = ±wheel_base/2（前 +，后 −）、z = ±track_width/2（左 −，右 +）、y = 半径。
## 纯视觉，不参与物理；子节点缺失只告警一次并跳过（不崩）。
func _layout_wheels() -> void:
	var wheels: Node3D = get_node_or_null("Wheels") as Node3D
	if wheels == null:
		_warn_wheels("Wheels 容器缺失")
		return
	var missing: Array[String] = []
	for i in range(WHEEL_NAMES.size()):
		var wheel_name: String = WHEEL_NAMES[i]
		var wheel: Node3D = wheels.get_node_or_null(wheel_name) as Node3D
		if wheel == null:
			missing.append(wheel_name)
			continue
		var x := _wheel_base * 0.5 if i < 2 else -_wheel_base * 0.5
		var z := -_track_width * 0.5 if (i % 2) == 0 else _track_width * 0.5
		wheel.position = Vector3(x, WHEEL_RADIUS, z)
	if not missing.is_empty():
		_warn_wheels("轮子节点缺失：%s" % str(missing))


func _warn_wheels(message: String) -> void:
	if _warned_wheels:
		return
	_warned_wheels = true
	push_warning("[diff_drive] %s（视觉件缺失不影响运动学）" % message)
