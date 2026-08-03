# Task 2: Robot

> 状态：实现中
> 创建日期：2026-07-07
> 最后更新：2026-07-21

## 目标

实现 `Robot` — 长期运行的 tokio task，作为整个机器人系统的中枢。

## 文件结构（当前）

```
Src/Robot/
├── mod.rs          ← 模块入口 + public export
├── state.rs        ← RobotState + LidarState（各设备独立）
├── websocket.rs    ← WebSocket 遥控服务
├── core/           ← Robot 核心
│   ├── mod.rs
│   ├── command.rs  ← Command 枚举
│   └── robot.rs    ← Robot::launch() + 主 select! 循环 + dispatch
└── control/
    ├── types.rs    ← CarType
    ├── serial/
    │   └── port.rs ← spawn_port() 通用 TX+RX tokio task
    └── device/
        ├── mod.rs
        ├── stm32/
        │   ├── mod.rs       ← STM32Device
        │   ├── constants.rs ← 协议常量 + MotionState
        │   └── protocol.rs  ← 帧构建 + 状态机 + 传感器解析 + 测试
        └── lidar/
            ├── mod.rs       ← LidarDevice
            ├── constants.rs ← tmini 协议常量
            ├── types.rs     ← LaserPoint / LaserScan
            ├── parser.rs    ← feed_byte 状态机 + 点云解析 + 测试
            └── checksum.rs  ← XOR 校验和 + 测试
```

## 命令定义 (`core/command.rs`)

```rust
pub enum Command {
    Forward(i16), Backward(i16),
    SpinLeft(i16), SpinRight(i16),
    Stop, Beep(u16),
    StartLidarScan, StopLidarScan,
}
```

## 结构

```rust
pub struct Robot {
    pub cmd_tx: mpsc::Sender<Command>,            // 对外：统一命令输入
    pub robot_state: Arc<RwLock<RobotState>>,      // STM32 独占写，WS/Lua 读
    pub lidar_state: Arc<RwLock<LidarState>>,      // LiDAR 独占写，WS/Lua 读
    cancel: CancellationToken,                     // 内部：优雅退出
}
```

**设计原则**：每个 Device 持有独立的 `Arc<RwLock<自己的State>>`，互不干扰。
- `robot_state` — STM32 RX 回调写入，包含 vx/vy/vz/battery/attitude 等
- `lidar_state` — LiDAR RX 回调写入，包含 scan: LaserScan
- 避免单一大锁的竞争和\"整对象覆盖\"导致的数据丢失

## 启动流程

```
1. 创建各设备独立状态
   robot_state = Arc::new(RwLock::new(RobotState::default()))
   lidar_state = Arc::new(RwLock::new(LidarState::default()))

2. spawn 各 Device
   stm32 = STM32Device::spawn(port, baud, car_type, robot_state.clone())
   lidar = LidarDevice::spawn(lidar_port, lidar_baud, lidar_state.clone())  // 可选
   // 未来: camera, gps, ...

3. 创建命令通道 + spawn 主循环
   (cmd_tx, cmd_rx) = mpsc::channel(32)
   tokio::spawn(main_loop)

4. 返回 Robot { cmd_tx, robot_state, lidar_state, cancel }
```

## 集成

```
main.rs / main_robot.rs
  │
  ├── Robot::launch(stm32_port, stm32_baud, car_type, lidar_port?, lidar_baud?)
  │     └── 内部: broadcast_loop (100ms) → pose_tx + (未来) map_tx
  │
  ├── WebSocket (Src/WebSocket/)     ← 订阅 Robot 的 broadcast
  │     └── 只管传输，不碰 state
  │
  ├── [Lua 绑定]                     ← 上层命令源 (→ cmd_tx)
  └── [LLM Agent]                    ← 上层命令源 (→ cmd_tx)
```

> ⚠️ 计划重构：`websocket.rs` → `Src/WebSocket/`，与 Robot 平级。见 Task 5。

## 已知问题

| # | 问题 | 状态 |
|---|------|:---:|
| 1 | ~~状态写回每帧 spawn task~~ → `try_write()` + `frame_parsed` | ✅ |
| 2 | ~~状态共享冲突（整对象覆盖）~~ → 各设备独立 State | ✅ |

## 人类评审

<!-- 在此区域写下评审意见 -->

