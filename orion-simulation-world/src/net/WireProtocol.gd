# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-22
#
# 消息协议唯一入口（task_30 §六 / COMPONENTS.md §7.3、§8）：
#   消息名常量 + JSON 文本帧编解码 + LiDAR 二进制帧构造 / 帧头解析。
#
# 只做「协议事实」：不碰 socket、不碰场景树、不查零件目录、不打业务日志。
#   收发在 ComponentLink，业务分派在 WebSocketManager。
#
# 二进制帧（小端，COMPONENTS.md §8.2）：
#   [0] u8 type | [1] u16 slot_id | [3] u32 seq | [7] f64 world_time | [15] payload
#   lidar payload = f32[rays] 距离（米，0.0 = 无回波）；总长 = 15 + 4·rays（360 线 = 1455 B）。
#
# 实测（4.7.2，COMPONENTS.md §9.1 E21）：JSON.parse_string() 遇非法输入会向 stderr
#   打印 "ERROR: Parse JSON failed"（core/io/json.cpp），污染「grep ERROR 必须为空」的验收；
#   这里一律用 JSON.new().parse()（返回 Error，**不打印**），失败返回 {}。
#
# 不声明全局类名（§9.1 E5）：调用方 preload / load 本脚本后调静态方法。

extends RefCounted


# ---------------------------------------------------------------- 文本帧消息名（§8.1）
const MSG_CHASSIS_HELLO := "chassis_hello"
const MSG_CHASSIS_READY := "chassis_ready"
const MSG_SENSOR_HELLO := "sensor_hello"
const MSG_SENSOR_READY := "sensor_ready"
const MSG_MOTION_CMD := "motion_cmd"
const MSG_ASSEMBLY_READY := "assembly_ready"
## 位姿下行（task_31 §三；`imu` / `gnss` 在 v1 **不做**，位姿走本条消息，见 §10）。
const MSG_POSE := "pose"

# ---------------------------------------------------------------- 二进制帧（§8.2）
## 1 = lidar_2d；2 = camera（预留）
const FRAME_TYPE_LIDAR := 1
const FRAME_TYPE_CAMERA := 2
## u8 type + u16 slot_id + u32 seq + f64 world_time
const FRAME_HEADER_SIZE := 15
## lidar payload 每个距离占 f32
const LIDAR_SAMPLE_BYTES := 4


# ================================================================ 文本帧

## bytes -> Dictionary。非 JSON / 非对象（数组、标量、null）一律返回 {}（不抛、不打印）。
static func ParseJson(bytes: PackedByteArray) -> Dictionary:
	if bytes.is_empty():
		return {}
	var json := JSON.new()
	if json.parse(bytes.get_string_from_utf8()) != OK:
		return {}
	var parsed: Variant = json.data
	if typeof(parsed) != TYPE_DICTIONARY:
		return {}
	var dict: Dictionary = parsed
	return dict


## Dictionary -> UTF-8 字节（可直接进 send() / 落盘比对）。
static func MakeJson(msg: Dictionary) -> PackedByteArray:
	return MakeJsonText(msg).to_utf8_buffer()


## Dictionary -> JSON 文本（ComponentLink.SendJson 用 send_text 时取这一份）。
static func MakeJsonText(msg: Dictionary) -> String:
	return JSON.stringify(msg)


## 位姿下行帧（世界坐标 Z-up、米、yaw 弧度绕 +z、归一化 [-PI,PI)）。
## 位置/朝向必须来自 Vehicle.GetPose()（§9.1 E23），不得传 global_position / rotation.y。
## 固定 8 键：{msg, world_time, x, y, z, yaw, vx, vy}；`vz` / `valid` / `sub_gx` / `sub_gy` / `rtk_fixed` 一律不发。
## `velocity` 由调用方传 WorldTransform.VelToWorld(vehicle.velocity) 的结果（本文件不 preload WorldTransform）。
static func BuildPose(world_time: float, pose: Dictionary, velocity: Vector3) -> Dictionary:
	return {
		"msg": MSG_POSE, "world_time": world_time,
		"x": float(pose.get("x", 0.0)), "y": float(pose.get("y", 0.0)),
		"z": float(pose.get("z", 0.0)), "yaw": float(pose.get("yaw", 0.0)),
		"vx": velocity.x, "vy": velocity.y,
	}


# ================================================================ 二进制帧

## LiDAR 帧：15 字节头 + ranges 的 f32 小端数组。
## 长度 = FRAME_HEADER_SIZE + LIDAR_SAMPLE_BYTES * ranges.size()。
static func BuildLidar2DFrame(sensor_id: int, seq: int, world_time: float,
		ranges: PackedFloat32Array) -> PackedByteArray:
	var frame := PackedByteArray()
	frame.resize(FRAME_HEADER_SIZE)
	frame.encode_u8(0, FRAME_TYPE_LIDAR)
	frame.encode_u16(1, sensor_id & 0xFFFF)
	frame.encode_u32(3, seq & 0xFFFFFFFF)
	frame.encode_double(7, world_time)
	frame.append_array(ranges.to_byte_array())
	return frame


## 解析帧头 -> {type, slot_id, seq, world_time}；短于 15 字节 -> {}（调用方按丢弃处理）。
static func ParseFrameHeader(bytes: PackedByteArray) -> Dictionary:
	if bytes.size() < FRAME_HEADER_SIZE:
		return {}
	return {
		"type": bytes.decode_u8(0),
		"slot_id": bytes.decode_u16(1),
		"seq": bytes.decode_u32(3),
		"world_time": bytes.decode_double(7),
	}


## 一帧的 payload 字节数（= 总长 - 15）；短包 -> 负数不返回，一律 0。
static func FramePayloadSize(bytes: PackedByteArray) -> int:
	return maxi(bytes.size() - FRAME_HEADER_SIZE, 0)
