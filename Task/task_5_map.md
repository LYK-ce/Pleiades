# Task 5: Map

> 状态：设计中
> 创建日期：2026-07-21

## 目标

实现单 Chunk 占据栅格建图，通过 Robot 的 broadcast channel 推送到 Pictor 可视化。

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
| 初始状态 | 全部 2（未知） |
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
3. 每个唯一格子画 Bresenham 射线：经过 → 0 (FREE)，终点 → 1 (OCCUPIED)
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

三条 task 各管各的，互不耦合。

### 数据对外广播

```
Robot 通过 broadcast channel 对外输出：
    pose_tx: broadcast::Sender<Pose>         ← state_notifier 写
    map_tx:  broadcast::Sender<MapDelta>     ← SLAM task 写

WebSocket 订阅（独立于 Robot，见架构边界）：
    ├── subscribe pose_tx → JSON → 客户端
    └── subscribe map_tx  → binary(全量)/JSON(增量) → 客户端
```

### 架构边界

```
Src/
├── Robot/              ← 机器人大脑
│   ├── core/           ←   Robot::launch() + state_notifier + main_loop
│   ├── slam/           ←   建图（OccupancyGrid + update + map_tx）
│   └── control/        ←   设备驱动
│
├── WebSocket/          ← [重构] 遥控通信层（平级）
│   ├── mod.rs          ←   订阅 + 转发
│   ├── server.rs       ←   accept + handle_connection
│   └── protocol.rs     ←   encode/decode: hello, pose, cmd, map
│
├── API/
├── Network/
└── ...
```

- Robot 负责生产数据，通过 `broadcast::Sender` 对外广播
- WebSocket 只负责订阅 + 传输，不碰任何 state
- SLAM 是 Robot 内部的独立 task，自己计算自己发

## 文件结构

```
Src/Robot/
├── core/
│   └── robot.rs        ←   [修改] 新增 state_notifier + SLAM task
├── slam/               ←   [新增]
│   ├── mod.rs          ←     模块入口
│   ├── grid.rs         ←     OccupancyGrid + Chunk（256×256）
│   └── lidar_mapper.rs ←     update(grid, pose, scan) → Vec<Delta>
└── control/
    └── ...

Src/WebSocket/          ←   [新增，从 Src/Robot/websocket.rs 重构而来]
├── mod.rs
├── server.rs
└── protocol.rs
```

## 实现步骤

1. 实现 `slam/grid.rs` — `OccupancyGrid`, `Chunk` 数据结构
2. 实现 `slam/lidar_mapper.rs` — `update(grid, pose, scan) → Vec<Delta>`
3. 修改 `core/robot.rs` — `launch()` 创建 grid + broadcast channel
   - 新增 `state_notifier` task: 读 robot_state → pose_tx.send()
   - 新增 SLAM task: 读 lidar_state → update grid → map_tx.send(deltas)
4. 重构 `Src/Robot/websocket.rs` → `Src/WebSocket/`，订阅 broadcast 替代直接读 state
5. 编译 + 测试 + Pictor 联调

## 人类评审

<!-- 在此区域写下评审意见 -->

