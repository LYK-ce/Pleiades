# Presented by KeJi
# Created Date ： 2026-10-21
# Modified Date ： 2026-10-24
#
# 单条连接封装 ComponentLink（COMPONENTS.md §4.5 / §7.4）：RefCounted，**不进场景树**。
#   一条 TCP+WS 连接 = 一个本对象；它只回答三件事：
#     1) 我是谁（conn_id / peer_id / role / slot / sensor_id / vehicle）
#     2) 现在什么状态（IsOpen / IsConnecting / closed / handshaken）
#     3) 怎么收发（Poll / TakePackets / SendJson / SendBinary / NextSeq / Close）
#
# 它**不做业务判断**（不认 msg 名、不查目录、不建车）：保持「哑管道 + 状态」，
#   业务分派全部在 WebSocketManager —— 这样连接层可以被任何未来组件（相机流等）复用。
#
# 实测（4.7.2，COMPONENTS.md §9.1 E15 / E19）：
#   - WebSocketPeer 没有「发二进制」的快捷方法（E15）：文本 send_text()，
#     二进制 send(bytes, WebSocketPeer.WRITE_MODE_BINARY)。
#   - close(code, reason) 返回 **void**（写成 var e := peer.close(...) 直接 Parse Error）。
#   - 握手未完成（STATE_CONNECTING）时 close() 有效：下一帧 get_ready_state() == CLOSED。
#
# 不声明全局类名（§9.1 E5）：WebSocketManager preload 本脚本后 new()。

extends RefCounted

const WireProtocol := preload("res://src/net/WireProtocol.gd")

## WS 心跳间隔（秒）：只靠它 + get_ready_state() 判死，不做应用层 ping/pong（§8.1-5）。
const HEARTBEAT_INTERVAL_S := 10.0

## 单帧缓冲区上限（字节）：`WebSocketPeer` 默认只有 **64 KiB**，一次 720p 的 JPEG
##   就能把它撑爆（task_34 §二 Step 3 硬要点）。提到 4 MiB 一次到位：
##   1280×720 quality 0.8 实测 ≈ 29 KB，640×480 ≈ 8.9 KB，高熵画面可到数百 KB。
## 代价：每连接常驻 2 × 4 MiB（世界侧连接数 = sim 数，个位数，可接受）。
const MAX_FRAME_BUFFER_BYTES := 4 * 1024 * 1024

# ---------------------------------------------------------------- 身份 / 状态
## 与 WS 共用同一条 TCP（accept_stream 后由 peer 接管）。
var tcp: StreamPeerTCP = null
var peer: WebSocketPeer = null
## manager 单调分配的连接号（仅日志 / 自检用，不上线）。
var conn_id: int = 0
## sim 自报的 peer_id（**原样**保存；节点名净化在 manager 侧做）。
var peer_id: String = ""
## &"chassis" / &"sensor"；未握手 = &""。
var role: StringName = &""
## 传感器连接的槽位（= Vehicle/Sensors 下的节点名）；底盘连接 = &""。
var slot: StringName = &""
## u16 语义的传感器编号（sensor_ready 回给 sim，二进制帧头 slot_id 用它）。
var sensor_id: int = 0
## 装配后指向该 peer 的 avatar（Vehicle 实例）；未装配 = null。
var vehicle: Node3D = null
## 本连接下行二进制帧的序号（NextSeq() 从 1 起）。
var out_seq: int = 0
## 已收到合法 hello。
var handshaken: bool = false
## 已判死（下一帧由 manager 的 _SweepClosed() 移除）。
var closed: bool = false
## 建链时刻（墙钟毫秒，握手超时用；R20：不用帧数）。
var created_ms: int = 0
## 关闭原因 / 对端关闭码（manager 一行日志用）。
var last_error: String = ""

## 本帧从 socket 抽出的包：[{bytes: PackedByteArray, was_string: bool}]。
var _packets: Array = []


# ================================================================ 生命周期

## 接管一条已 accept 的 TCP：关 Nagle、建 peer、握手超时起点。
## accept_stream 失败 -> closed=true（下一帧由 manager 清理，不抛异常）。
func Init(p_tcp: StreamPeerTCP, p_conn_id: int) -> void:
	tcp = p_tcp
	conn_id = p_conn_id
	created_ms = Time.get_ticks_msec()
	peer = WebSocketPeer.new()
	# 必须在 accept_stream() **之前**设（连上后再改，对已分配的收发缓冲无效）。
	peer.inbound_buffer_size = MAX_FRAME_BUFFER_BYTES
	peer.outbound_buffer_size = MAX_FRAME_BUFFER_BYTES
	peer.set_heartbeat_interval(HEARTBEAT_INTERVAL_S)
	if tcp == null:
		last_error = "null tcp"
		closed = true
		return
	tcp.set_no_delay(true)                 # 控制指令要低延迟，关 Nagle
	var err: Error = peer.accept_stream(tcp)
	if err != OK:
		last_error = "accept_stream failed: %s" % error_string(err)
		closed = true


## 每帧一次：poll -> **先抽包**（OPEN/CLOSING 都抽，先关后抽会丢最后几帧）-> 再看 CLOSED。
func Poll() -> void:
	if peer == null or closed:
		return
	peer.poll()
	var state: WebSocketPeer.State = peer.get_ready_state()
	if state == WebSocketPeer.STATE_OPEN or state == WebSocketPeer.STATE_CLOSING:
		while peer.get_available_packet_count() > 0:
			var bytes: PackedByteArray = peer.get_packet()
			_packets.append({"bytes": bytes, "was_string": peer.was_string_packet()})
	if state == WebSocketPeer.STATE_CLOSED:
		closed = true
		if last_error.is_empty():
			last_error = "peer closed (code=%d reason='%s')" % [
				peer.get_close_code(), peer.get_close_reason()]


## 取出并清空本帧抽到的包（调用方持有 Array 的所有权）。
func TakePackets() -> Array:
	var out: Array = _packets
	_packets = []
	return out


# ================================================================ 发送

## 文本帧（JSON）。未 OPEN -> ERR_UNCONFIGURED（不崩、不打印）。
func SendJson(msg: Dictionary) -> Error:
	if not IsOpen():
		return ERR_UNCONFIGURED
	return peer.send_text(WireProtocol.MakeJsonText(msg))


## 二进制帧（LiDAR 等大数据；§8.2）。未 OPEN -> ERR_UNCONFIGURED。
func SendBinary(bytes: PackedByteArray) -> Error:
	if not IsOpen():
		return ERR_UNCONFIGURED
	return peer.send(bytes, WebSocketPeer.WRITE_MODE_BINARY)


## 下行二进制帧序号：先自增再返回（首条 = 1）。
func NextSeq() -> int:
	out_seq += 1
	return out_seq


# ================================================================ 状态

func IsOpen() -> bool:
	return peer != null and not closed and peer.get_ready_state() == WebSocketPeer.STATE_OPEN


func IsConnecting() -> bool:
	return peer != null and not closed \
		and peer.get_ready_state() == WebSocketPeer.STATE_CONNECTING


## 主动关闭：CONNECTING / OPEN 才调 peer.close()（E19：返回 void，不取值）。
## 只置状态，真正的移除交给 manager 的下一帧 _SweepClosed()（R7：遍历中不删）。
func Close(code: int = 1000, reason: String = "") -> void:
	if closed:
		return
	if peer != null:
		var state: WebSocketPeer.State = peer.get_ready_state()
		if state == WebSocketPeer.STATE_CONNECTING or state == WebSocketPeer.STATE_OPEN:
			peer.close(code, reason)
	closed = true
	if last_error.is_empty():
		last_error = "close %d '%s'" % [code, reason]


## 一行可读身份（日志 / 自检）。例："conn=3 peer=simA role=sensor slot=lidar_front open=true"
func Describe() -> String:
	return "conn=%d peer='%s' role=%s slot=%s open=%s handshaken=%s" % [
		conn_id, peer_id, str(role), str(slot), IsOpen(), handshaken]
