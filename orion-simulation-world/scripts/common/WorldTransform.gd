# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-21
#
# 坐标转换唯一入口（task_30 §7.3 / COMPONENTS.md §2.3、§7.2）。
# 世界：Z-up，x=东、y=南、z=高，yaw 绕 z（从 +x 逆时针为正，右手系）。
# Godot：Y-up，前进轴固定为局部 +X，rotation.y = -yaw。
#
# 全项目只有本文件允许出现 (x, z, y) 换轴字面量；其它文件一律 preload 本脚本调用。
# 不声明 class_name（COMPONENTS.md §9.1 E5）。

extends RefCounted


## 世界位置 -> Godot 位置：world (x, y, z) -> godot (x, z, y)。
static func PosToGodot(p: Vector3) -> Vector3:
	return Vector3(p.x, p.z, p.y)


## Godot 位置 -> 世界位置：godot (x, y, z) -> world (x, z, y)。
static func PosToWorld(g: Vector3) -> Vector3:
	return Vector3(g.x, g.z, g.y)


## 世界速度 -> Godot 速度（换轴规则同位置）。
static func VelToGodot(v: Vector3) -> Vector3:
	return PosToGodot(v)


## Godot 速度 -> 世界速度（换轴规则同位置）。
static func VelToWorld(v: Vector3) -> Vector3:
	return PosToWorld(v)


## 世界 yaw（绕 +z，逆时针为正）-> Godot rotation.y（绕 +y，顺时针为正）。
static func YawToGodot(world_yaw: float) -> float:
	return -world_yaw


## Godot rotation.y -> 世界 yaw。
static func YawToWorld(godot_yaw: float) -> float:
	return -godot_yaw


## 世界位姿（位置 + yaw）-> Godot 变换（仅绕 Y 旋转，无缩放）。
static func MakeTransform(world_pos: Vector3, world_yaw: float) -> Transform3D:
	return Transform3D(Basis(Vector3.UP, YawToGodot(world_yaw)), PosToGodot(world_pos))


## Godot 变换 -> 世界位姿 {x, y, z, yaw}；yaw 归一化到 [-PI, PI)。
static func PoseFromTransform(t: Transform3D) -> Dictionary:
	var w := PosToWorld(t.origin)
	return {
		"x": w.x,
		"y": w.y,
		"z": w.z,
		"yaw": wrapf(YawToWorld(t.basis.get_euler().y), -PI, PI),
	}
