# Task 5: Map

> 状态：实现中（二值栅格 → 概率占据栅格）
> 创建日期：2026-07-21
> 最后更新：2026-07-27

## 目标

实现单 Chunk 占据栅格建图，通过 Robot 的 broadcast channel 推送到 Pictor 可视化。

## 当前状态

- `map_delta`: 200ms 增量发送 ✅
- `map_full`: WebSocket 连接时发送 ✅
- LiDAR 帧组装（零位包触发全帧） ✅
- **概率占据栅格** ❌ 待实现

---

## 🔴 二值栅格闪烁问题

### 现象

地图上同一格子 Occupied/Free 反复横跳，即使 ROS2 rviz2 点云本身是稳定的。

### 根因

二值占据栅格 `set(gx, gy, 0/1)` 直接覆盖，无法容忍传感器噪声：

```
Tmini 距离抖动: ±1~3cm（物理特性，非驱动 Bug）
分辨率: 0.5m/格

圈1: 距离 1.52m → Brensenham 终点在格子 A → Occupied
圈2: 距离 1.49m → Bresenham 终点在格子 B → 射线穿过格子 A → Free  ← 💥
圈3: 距离 1.51m → 格子 A 又是 Occupied                          ← 💥
→ 反复横跳
```

1~3cm 的噪声在 0.5m 栅格上足够让 Bresenham 终点跳到相邻格子，导致墙壁被自己的射线"擦除"。

### 修复：概率占据栅格（log-odds）

每个格子存概率分 `i8`（-128..127），多次观测累积置信度：

| 操作 | 分值变化 | 说明 |
|------|:---:|------|
| 观测到 Occupied | `cell += 15` | 快速建墙 |
| 观测到 Free | `cell -= 5` | 缓慢拆墙 |
| `cell > 0` | 认为是 Occupied | 需要多次确认 |
| `cell <= 0` | 认为是 Free | |
| 初始值 | `0` | 未知 |

**关键**: 建墙 (+15) 比拆墙 (-5) 更激进，偶尔的噪声漏打只是 -5 不会把墙抹掉。

### 数值示例

```
圈1: 打中格子 → 15   Occupied
圈2: 噪声漏打 → 10   Occupied (15-5)  ← 没消失！
圈3: 又打中   → 25   Occupied (10+15)
...
连续 3 次都漏打 → 0   Free (25-5-5-5-5-5)  ← 真的消失了
```

### 改动范围

```
grid.rs:
  cells: Box<[u8]> → Box<[i8]>   （概率分）
  set(gx, gy, u8) → update(gx, gy, bool) + state(gx, gy)
  build_map_full(): i8 > 0 → Occupied, i8 <= 0 → Free

lidar_mapper.rs:
  grid.set(cgx, cgy, 0/1) → grid.update(cgx, cgy, true/false)
  delta 对比用 grid.state() 而非 get()
```

不改驱动层、不改 Bresenham 逻辑。

---

## 背景

Pictor 协议：

- `map_full` — 全量地图（二进制帧，65545 bytes/chunk）
- `map_delta` — 增量地图（JSON `{gx, gy, state}`）

Orion 传感器：

- STM32 — 编码器 → 位姿
- LiDAR — 360° 点云 → 障碍物检测

## 设计

### 地图规格

| 项目 | 值 |
|------|------|
| Chunk 数量 | 1 个（后续扩展） |
| Chunk 大小 | 256×256 = 65536 cells |
| 分辨率 | 0.5m/cell → 128m×128m |
| 初始状态 | 全部 0（log-odds 中值 = 未知） |
| 小车初始位置 | (128, 128) — Chunk 正中心 |

### LiDAR 输出

`LidarState.scan: Option<LaserScan>`，每圈扫描包含：

```rust
LaserScan {
    points: Vec<LaserPoint>,  // 一圈点云（极坐标）
    stamp: u64,               // 纳秒时间戳
    scan_freq: f32,           // 扫描频率 Hz
    scan_time: f32,           // 一圈耗时 秒
}

LaserPoint {
    angle: f32,     // 弧度（雷达坐标系）
    range: f32,     // 米
    intensity: f32, // 0-255 信号强度
}
```

坐标转换（雷达极坐标 → 世界笛卡尔坐标）：

```
world_x = pose.x + range × cos(pose.yaw + point.angle)
world_y = pose.y + range × sin(pose.yaw + point.angle)
```

网格化：

```
gx = floor(world_x / 0.5)
gy = floor(world_y / 0.5)
```

SLAM task 对每圈扫描做射线填充：

1. 坐标转换：2000 点 → 世界坐标 → 网格坐标
2. HashSet 去重终点格子（多点落同一格，去重后 ~100-300 个）
3. 每个唯一格子画 Bresenham 射线：沿途 → `update(grid, gx, gy, false)`，终点 → `update(grid, gx, gy, true)`
4. 对比旧值，变化才记 delta

### Robot 内部 task 分工

```
state_notifier (100ms tick):
    → 读 robot_state → build Pose → pose_tx.send()
    （只负责位姿通告，不碰地图）

SLAM task (独立频率，如 200ms):
    → 读 lidar_state.scan + robot_state
    → update(grid, pose, scan)
    → 有变化格子 → map_tx.send(deltas)
    → 没变化 → 什么都不做
    （自己算，自己发，不依赖 state_notifier）

main_loop:
    → select! 收 Command → dispatch → stm32/lidar
```

### 架构边界

```
Src/
├── Robot/              ← 机器人大脑
│   ├── core/           ←   Robot::launch() + state_notifier + main_loop
│   ├── slam/           ←   建图（OccupancyGrid + update + map_tx）
│   └── control/        ←   设备驱动
│
├── WebSocket/          ← 遥控通信层（平级）
│   ├── mod.rs          ←   订阅 + 转发
│   ├── server.rs       ←   accept + handle_connection
│   └── protocol.rs     ←   encode/decode: hello, pose, cmd, map
│
├── API/
├── Network/
└── ...
```

## 文件结构

```
Src/Robot/
├── core/
│   └── robot.rs        ←   Robot::launch() + state_notifier + SLAM task
├── slam/               ←
│   ├── mod.rs          ←     模块入口
│   ├── grid.rs         ←     OccupancyGrid + Chunk（256×256, log-odds）
│   └── lidar_mapper.rs ←     update(grid, pose, scan) → Vec<Delta> (Bresenham)
└── control/
    └── ...

Src/WebSocket/
├── mod.rs
├── server.rs
└── protocol.rs
```

## 实现步骤

1. ✅ 实现 `slam/grid.rs` — `OccupancyGrid`, `Chunk`（256×256, 0.5m/cell, 二值）
2. ✅ 实现 `slam/lidar_mapper.rs` — `update(grid, pose, scan) → Vec<Delta>` (Bresenham 射线)
3. ✅ 修改 `core/robot.rs` — `launch()` 创建 grid + broadcast channel
4. ✅ 重构 `Src/Robot/websocket.rs` → `Src/WebSocket/`
5. ✅ 编译 + 测试
6. ✅ LiDAR 帧组装修复（零位包触发全帧）
7. ❌ 概率占据栅格（log-odds）— 替换二值栅格

## Code Review 修复（2026-07-22）

| 修复 | 文件 |
|------|------|
| TX task 写失败连续错误阈值（5次） | `serial/port.rs` |
| `build_host_frame` LEN 加 `debug_assert` | `stm32/protocol.rs` |
| LiDAR 扫描 info! 日志注释 | `lidar/mod.rs` |
| LiDAR 距离过滤 `0.1~12.0m` | `slam/lidar_mapper.rs` |
| `/dev/null` fallback 改为硬错误 | `main.rs`, `main_robot.rs` |
| `launch()` 自动启动 LiDAR 扫描 | `core/robot.rs` |
| `build_map_full()` 内打印地图状态 | `slam/grid.rs` |
| WebSocket 新增 binary frame map_full 转发 | `server.rs` |

## 已知限制

- 位姿 `(x,y)` 有里程计但精度有限（待 LiDAR 扫描匹配）
- 仅单 Chunk，后续需扩展多 Chunk 支持

## 人类评审

<!-- 在此区域写下评审意见 -->

