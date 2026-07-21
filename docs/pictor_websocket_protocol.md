# WebSocket 通信协议（Pictor 参考）

> 来源：`/vepfs-mlp2/c20250205/240804016/GodotProject/Pictor/docs/websocket_protocol.md`
> 复制日期：2026-07-21
> 用途：Orion 接入 Pictor 可视化时的协议参考

---

## 基本信息

| 项目 | 值 |
|------|------|
| 传输协议 | WebSocket |
| 数据格式 | JSON 文本消息（map_full 为二进制帧） |
| 编码 | UTF-8 |
| 角色 | 小车 = Server，PC = Client |
| 默认端口 | 9001 |

每条消息为单行 JSON，顶层必有 `type` 字段。

## 坐标系

| 项目 | 值 |
|------|------|
| 1 cell | 0.5m × 0.5m |
| Godot 缩放 | 1m = 32px |
| 2D 轴 | `x`（东/右）, `y`（南/下） |
| 3D 轴 | 高度用 `z` |
| 网格坐标 | `gx = floor(x / 0.5)`, `gy = floor(y / 0.5)` |

Chunk 大小：256×256 cell = 128m×128m。

---

## 连接流程

| 阶段 | 触发条件 | 含义 |
|------|---------|------|
| WebSocket 握手完成 | TCP 升级为 WS | 物理通道建立 |
| `hello` 包收到 | 小车发送身份 | **正式建立连接** |

`hello` 之前收到的任何消息将被丢弃。

```
小车 ── TCP 握手 ──→ PC
小车 ── hello ──→ PC          ← 必须第一帧
小车 ── map_full ──→ PC       ← 可选
小车 ── pose ──→ PC
```

---

## 上行：小车 → PC

### hello — 注册身份

```json
{
    "type": "hello",
    "vehicle_id": "car_0",
    "address": "ws://192.168.1.10:9090"
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `vehicle_id` | string | 车辆唯一标识 |
| `address` | string | 本连接地址 |

### pose — 车辆位姿

实时发送车辆位置、朝向和速度。

```json
{
    "type": "pose",
    "ts": 1717800000.123,
    "x": 1.5,
    "y": 3.2,
    "z": 0.0,
    "yaw": 0.785,
    "vx": 0.5,
    "vy": 0.0
}
```

| 字段 | 类型 | 单位 | 说明 |
|------|------|------|------|
| `ts` | f64 | 秒 | Unix 时间戳 |
| `x`, `y` | f32 | 米 | 2D 世界坐标 |
| `z` | f32 | 米 | 高度 |
| `yaw` | f32 | 弧度 | 偏航角 |
| `vx`, `vy` | f32 | 米/秒 | 2D 速度分量 |

### map_full — 全量地图（二进制帧）

连接建立后发送完整 Chunk。**使用 WebSocket 二进制帧**，不走 JSON。

```
字节布局:
  [0]      type:    u8 = 0 (map_full)
  [1..4]   chunk_x: int32 (big-endian)
  [5..8]   chunk_y: int32 (big-endian)
  [9..]    cells:   PackedByteArray, 65536 bytes (256×256)

  总大小: 65545 bytes
```

每个 cell: 0=可通行, 1=不可通行, 2=未知，行优先 `index = y * 256 + x`。

### map_delta — 增量地图（文本帧）

仅发送变化的格子，JSON 格式。

```json
{
    "type": "map_delta",
    "voxels": [
        {"gx": 2, "gy": 1, "state": 1},
        {"gx": 3, "gy": 2, "state": 0}
    ]
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `voxels` | array | 变化的格子列表 |
| `gx`, `gy` | i32 | 网格坐标 |
| `state` | u8 | 0=可通行, 1=不可通行, 2=未知 |

---

## 下行：PC → 小车

### cmd — 控制命令

```json
{
    "cmd": "forward"
}
```

| 命令 | 说明 |
|------|------|
| `forward` | 前进 |
| `backward` | 后退 |
| `spin_left` | 左旋 |
| `spin_right` | 右旋 |
| `stop` | 停止 |

---

## 消息一览

```
上行 (小车 → PC)          下行 (PC → 小车)
─────────────────         ─────────────────
hello                      cmd
pose
map_full
map_delta
```
