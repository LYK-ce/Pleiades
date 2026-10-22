# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 底盘测试替身（Step 4 自检专用，**不是组件**）：
#   只实现 COMPONENTS.md §4.4 的鸭子契约 —— Configure / GetDefaultParams / SetCommand / Step。
#   不写任何运动学：Step() 返回固定控制量，让自检的位移/转角断言成为"帧数 × 常量"的确定性算式。
#
# 放在 src/tests/doubles/ 而非 src/chassis/：不改产品代码、不新增 .tscn（§9.4）。
# 由 src/tests/check_vehicle.gd 用 PackedScene.pack() 现场打包成场景，不落盘为 .tscn。
#
# 不声明 class_name（COMPONENTS.md §9.1 E5）。

extends Node3D

## Step() 恒返回的线速度（Godot 坐标，前进轴 = 局部 +X）与偏航角速度（rotation.y 角速度）。
const TEST_SPEED_MPS := 0.5
const TEST_YAW_RATE := 0.2

## 自检读取的调用记录。
var configured_vehicle: Node3D = null
var configured_params: Dictionary = {}
var last_cmd: Dictionary = {}
var step_calls: int = 0


## 被 Vehicle.ApplyChassis() 调用一次；车辆把自己与合并后的参数交给底盘。
func Configure(vehicle: Node3D, params: Dictionary) -> void:
	configured_vehicle = vehicle
	configured_params = params.duplicate(true)


## 与传入参数合并的"底盘默认值"（Vehicle._merge_params 的左侧）。
func GetDefaultParams() -> Dictionary:
	return {"max_speed": 1.5, "wheel_base": 0.30}


## 接收 motion_cmd（由 Vehicle.SetMotionCommand 转发）；只留最新一条。
func SetCommand(cmd: Dictionary) -> void:
	last_cmd = cmd.duplicate(true)


## 物理步进：固定速度 + 固定偏航角速度（正 = 左转，与 rotation.y 同向）。
func Step(_current_velocity: Vector3, _delta: float) -> Dictionary:
	step_calls += 1
	return {"velocity": Vector3(TEST_SPEED_MPS, 0.0, 0.0), "yaw_rate": TEST_YAW_RATE}
