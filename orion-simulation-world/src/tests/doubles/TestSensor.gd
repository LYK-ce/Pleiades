# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 传感器测试替身（Step 4 自检专用，**不是组件**）：
#   只实现 Vehicle 期望的鸭子契约 —— 根节点 Node3D + Configure(params) + Sample()。
#   返回固定结构（不读物理世界），让 SampleSensors() 的转发结果可逐字段比对。
#
# 放在 src/tests/doubles/ 而非 src/sensors/：不改产品代码、不新增 .tscn（§9.4）。
# 由 src/tests/check_vehicle.gd 用 PackedScene.pack() 现场打包成场景，不落盘为 .tscn。
#
# 不声明 class_name（COMPONENTS.md §9.1 E5）。

extends Node3D

## 自检读取的调用记录。
var configured_params: Dictionary = {}
var sample_calls: int = 0


## 可选契约：挂载时 Vehicle.AttachSensor() 会把 params 交给传感器。
func Configure(params: Dictionary) -> void:
	configured_params = params.duplicate(true)


## 主采样契约（Step 8 的 Lidar2D 同名方法）。
func Sample() -> Dictionary:
	sample_calls += 1
	return {"slot": str(name), "rays": int(configured_params.get("rays", 0))}


## 备选采样契约（Step 8 的 Lidar2D.ScanOnce()）：语义同 Sample()，仅证明两条分支都能用。
func ScanOnce() -> Dictionary:
	return Sample()
