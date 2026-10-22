# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 底盘基类（COMPONENTS.md §4.4 / §7.5）：只定义契约 + 公共工具，不含任何运动学。
#   子类（DiffDrive = Step 5，Quadrotor = Step 7）覆写 4 个契约方法。
#
# 契约（由 Vehicle 以鸭子类型调用，§4.3）：
#   Configure(vehicle, params)                 装配：记宿主/参数，设 motion_mode、车身尺寸、视觉件
#   GetDefaultParams() -> Dictionary            默认参数（可能在 add_child **之前**被调用）
#   SetCommand(cmd)                            接收 motion_cmd（只保留最新，不排队）
#   Step(current_velocity, delta) -> Dictionary {velocity: Vector3(全局), yaw_rate: float}
#
# 本文件**不做**的事：不写 position、不调物理推进 API（D8：位置只由 Vehicle 推进）、
#   不做传感器/网络/世界几何（§10）；Step() 的 current_velocity 也不在此使用。
#
# 坐标约定：Godot Y-up，前进轴 = 局部 +X，局部 +Z = 车体右侧。
#   Step() 返回的 yaw_rate 是 **Godot rotation.y** 的角速度（正 = 左转）；
#   世界 yaw = -rotation.y（GetPose().yaw，§6.5）。
#
# 不声明全局类名（COMPONENTS.md §9.1 E5）：子类用 extends "res://src/chassis/ChassisModel.gd"。

extends Node3D


## 尺寸下限（米）：防零尺寸 shape / 除零。
const MIN_SIZE := 0.01
## 线加速度上限（m/s²）：世界侧速度环的斜坡斜率。
const MAX_ACCEL_MPS2 := 3.0
## 偏航角加速度上限（度/秒²）：世界侧速度环的斜坡斜率。
const MAX_YAW_ACCEL_DPS2 := 180.0


## Configure 记下的宿主（Vehicle 本体）；未装配 = null。
var vehicle: Node3D = null
## 生效参数（= 默认值 <- 传入 params）；未装配 = {}。
var params: Dictionary = {}
## 最新一条 motion_cmd（只保留最新，不排队）。
var last_cmd: Dictionary = {}


# ================================================================ 契约（子类覆写）

## 装配：只记宿主与参数；motion_mode / 车身尺寸 / 视觉件由**具体底盘**决定（UGV 地面、UAV 悬停）。
func Configure(p_vehicle: Node3D, p_params: Dictionary) -> void:
	vehicle = p_vehicle
	params = p_params.duplicate(true)


## 默认参数：返回**常量字典**，不得访问场景树 / global_* / 子节点（可能入树前被调用）。
func GetDefaultParams() -> Dictionary:
	return {}


## 接收 motion_cmd（由 Vehicle 转发）：基类只存最新一条，不解释语义。
func SetCommand(cmd: Dictionary) -> void:
	last_cmd = cmd.duplicate(true)


## 基类 = 不动的底盘：零速度、零角速度（子类覆写为真实运动学）。
func Step(_current_velocity: Vector3, _delta: float) -> Dictionary:
	return {"velocity": Vector3.ZERO, "yaw_rate": 0.0}


# ================================================================ 公共工具（DiffDrive / Quadrotor 共用）

## 读参数并转 float（容错：缺失 / 字符串 / 非法类型都回退，不报错、不崩）。
func GetParam(key: String, fallback: float) -> float:
	if not params.has(key):
		return fallback
	return _to_float(params[key], fallback)


## motion_cmd.speed 是 **0~100 百分比**（§8.1-2），不是 m/s；clamp 后 /100。
## 容错：缺失 / 非数字 / 非法类型 → 0.0（不 push_error、不崩）。
func SpeedPct(cmd: Dictionary) -> float:
	if not cmd.has("speed"):
		return 0.0
	return clampf(_to_float(cmd["speed"], 0.0), 0.0, 100.0) / 100.0


## motion_cmd.action：去空白 + 小写；缺失 / 空 / 非法类型 → "stop"。
func ActionOf(cmd: Dictionary) -> String:
	if not cmd.has("action"):
		return "stop"
	var raw: Variant = cmd["action"]
	var action := str(raw).strip_edges().to_lower() if raw != null else ""
	return action if not action.is_empty() else "stop"


## 度/秒 -> 弧度/秒（*_rate 参数的入参单位是度/秒，内部一律用 rad/s）。
func RateToRad(dps: float) -> float:
	return deg_to_rad(dps)


## 一阶斜坡：朝 target 前进，单步最多 max_delta（dt 由调用方乘好，天然与帧率无关）。
func Approach(current: float, target: float, max_delta: float) -> float:
	if max_delta <= 0.0:
		return current
	return clampf(target, current - max_delta, current + max_delta)


## 设宿主物理模式（is CharacterBody3D 守卫：宿主类型不对时返回 false，不崩）。
func ApplyMotionMode(mode: int) -> bool:
	var body: CharacterBody3D = vehicle as CharacterBody3D
	if body == null or not is_instance_valid(body):
		return false
	body.motion_mode = mode
	return true


## 设车身尺寸（唯一入口是 Vehicle.SetBodySize()：mesh + shape + "原点 = 底面中心"抬升）。
## 底盘**只传尺寸**，绝不自己写 body.position.y（否则双重抬升）。
func ApplyBodySize(size: Vector3) -> bool:
	if vehicle == null or not is_instance_valid(vehicle):
		return false
	if not vehicle.has_method("SetBodySize"):
		return false
	vehicle.call("SetBodySize", Vector3(
		maxf(size.x, MIN_SIZE), maxf(size.y, MIN_SIZE), maxf(size.z, MIN_SIZE)))
	return true


# ================================================================ 私有

## 宽容的数值解析：int/float 直接用；字符串走 to_float()；其它类型回退。
func _to_float(value: Variant, fallback: float) -> float:
	match typeof(value):
		TYPE_INT, TYPE_FLOAT:
			return float(value)
		TYPE_STRING, TYPE_STRING_NAME:
			var text := str(value).strip_edges()
			return text.to_float() if text.is_valid_float() else fallback
		_:
			return fallback
