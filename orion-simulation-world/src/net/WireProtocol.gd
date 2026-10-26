# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-25
#
# 消息协议唯一入口（task_30 §六 / COMPONENTS.md §7.3、§8）：
#   消息名常量 + JSON 文本帧编解码 + LiDAR / camera 二进制帧构造 + 帧头解析。
#
# 只做「协议事实」：不碰 socket、不碰场景树、不查零件目录、不打业务日志。
#   收发在 ComponentLink，业务分派在 WebSocketManager。
#
# 二进制帧（小端，COMPONENTS.md §8.2）：
#   [0] u8 type | [1] u16 slot_id | [3] u32 seq | [7] f64 world_time | [15] payload
#   lidar  (type=1) payload = f32[rays] 距离（米，0.0 = 无回波）；总长 = 15 + 4·rays（360 线 = 1455 B）。
#   camera (type=2) payload = **JPEG 原始字节**（不重编码、无长度前缀、无分隔符）；
#                   宽高从 JPEG 的 SOF0 段读，不上线；总长 = 15 + N（N = jpeg.size()）。
#
# 实测（4.7.2，COMPONENTS.md §9.1 E21）：JSON.parse_string() 遇非法输入会向 stderr
#   打印 "ERROR: Parse JSON failed"（core/io/json.cpp），污染「grep ERROR 必须为空」的验收；
#   这里一律用 JSON.new().parse()（返回 Error，**不打印**），失败返回 {}。
#
# 不声明全局类名（§9.1 E5）：调用方 preload / load 本脚本后调静态方法。

extends RefCounted


# ---------------------------------------------------------------- 文本帧消息名（§8.1）
## chassis_hello{peer_id, chassis, params, manifest}（task_35 Step 2 追加两个**可选**键）：
##   kind  : "robot"（缺省）| "pedestrian"；非法值由 manager warn 后按 robot 处理。
##   spawn : {x, y, z, yaw}，**只有** kind="pedestrian" 读；世界坐标 Z-up、米、yaw 弧度。
const MSG_CHASSIS_HELLO := "chassis_hello"
const MSG_CHASSIS_READY := "chassis_ready"
const MSG_SENSOR_HELLO := "sensor_hello"
const MSG_SENSOR_READY := "sensor_ready"
const MSG_MOTION_CMD := "motion_cmd"
const MSG_ASSEMBLY_READY := "assembly_ready"
## 位姿下行（task_31 §三；`imu` / `gnss` 在 v1 **不做**，位姿走本条消息，见 §10）。
const MSG_POSE := "pose"

# ---------------------------------------------------------------- chassis_hello.kind（task_35 Step 2）
## 两个**合法** kind 值：robot = 车 / 无人机（缺省，老 sim 一字不改也走这条），
## pedestrian = 行人（不进 Vehicles 容器、不占 _PickSpawn 的 avatar 排位）。
const KIND_ROBOT := "robot"
const KIND_PEDESTRIAN := "pedestrian"

# ---------------------------------------------------------------- 二进制帧（§8.2）
## 1 = lidar_2d；2 = camera（task_34 Step 3 落地，payload = JPEG 原始字节）
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


## 规范化 `chassis_hello.kind`（task_35 Step 2）：strip + lower，只认 robot / pedestrian。
## 缺失 / 空 / 非字符串 / 其它值一律返回 ""（**本文件不打日志**：调用方 warn 之后按 robot 处理）。
static func NormalizeKind(raw: Variant) -> String:
	if raw == null:
		return ""
	var text := str(raw).strip_edges().to_lower()
	if text == KIND_ROBOT or text == KIND_PEDESTRIAN:
		return text
	return ""


## `motion_cmd` 的「速度 + 方向」形态（task_35 Step 2）：{msg, seq, v} / {msg, seq, v, dir}。
##   v   : m/s（行人侧 clamp 0~1.26，不倒着走）
##   dir : 世界系朝向角，弧度，0 = +X（东），逆时针为正
##   dir 省略（缺省 NAN）= 只改速度、**保持原朝向** —— 超时停车就是这么发的（BuildMotionCommand(0.0)）
## **不含 action/speed**：两套字段并存、互不覆盖（§8.1-9）。
static func BuildMotionCommand(v: float, dir_rad: float = NAN, seq: int = 0) -> Dictionary:
	var msg := {"msg": MSG_MOTION_CMD, "seq": seq, "v": v}
	if is_finite(dir_rad):
		msg["dir"] = dir_rad
	return msg


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


## camera 帧（task_34 Step 3）：15 字节头 + **JPEG 原始字节**。
## 与 BuildLidar2DFrame **同一套头**（同样的端序 / 同样的字段偏移），只是 payload 语义不同：
##   不重编码、不加长度前缀——长度由 WS 帧本身给出，宽高从 JPEG 的 SOF0 段读（§8.2）。
## 长度 = FRAME_HEADER_SIZE + jpeg.size()；jpeg 为空时长度恰为 15（调用方应自行丢弃，别上线）。
static func BuildCameraFrame(sensor_id: int, seq: int, world_time: float,
		jpeg: PackedByteArray) -> PackedByteArray:
	var frame := PackedByteArray()
	frame.resize(FRAME_HEADER_SIZE)
	frame.encode_u8(0, FRAME_TYPE_CAMERA)
	frame.encode_u16(1, sensor_id & 0xFFFF)
	frame.encode_u32(3, seq & 0xFFFFFFFF)
	frame.encode_double(7, world_time)
	frame.append_array(jpeg)
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
