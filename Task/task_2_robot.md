# Task 2: Robot

> 状态：设计中
> 创建日期：2026-07-07

## 目标

实现 `Robot` — 长期运行的 tokio task，作为整个机器人系统的中枢。

## 结构

```rust
pub struct Robot {
    cmd_tx: mpsc::Sender<Command>,       // 对外：统一命令输入
    state: Arc<RwLock<RobotState>>,      // 对外：全局状态
    cancel: CancellationToken,           // 内部：优雅退出
}
```

## 启动流程

`Robot::launch()` 依次完成以下初始化：

```
1. 创建共享状态
   state = Arc::new(RwLock::new(RobotState::default()))

2. spawn 各 Device（每个 Device 内部启动 TX + RX tokio task）
   stm32 = STM32Device::spawn(port, baud, car_type, state.clone())
       └── 内部: tokio::spawn( tx_loop ) ← 写串口
                 tokio::spawn( rx_loop ) ← 读串口 → 写 state
   lidar = LidarDevice::spawn(...)   // 未来
   camera = CameraDevice::spawn(...) // 未来

3. 启动 WebSocket 遥控服务
   spawn_robot_ws_server(9090, event_bus)

4. 创建命令通道 + spawn 主循环
   (cmd_tx, cmd_rx) = mpsc::channel(32)
   tokio::spawn( main_loop )

5. 返回 Robot { cmd_tx, state, cancel }
```

Robot 退出时调 `cancel` → 所有 task（主循环、各 Device 的 TX/RX）优雅退出。

## 职责

1. **启动时**：创建全局状态，spawn 各 Device 的 TX/RX tokio task，启动 WS 服务，spawn 主循环
2. **运行时**：select! 接收统一命令 → 调度到对应 Device
3. **状态**：Device 的 rx_loop 直接写 `RobotState`，Robot 和外部都能读

```rust
impl Robot {
    pub fn launch(port: &str, baud: u32, car_type: CarType) -> Self {
        // 创建共享状态（Device 的 rx_loop 会直接写）
        let state = Arc::new(RwLock::new(RobotState::default()));

        // spawn STM32
        let stm32 = STM32Device::spawn(port, baud, car_type, state.clone());
        // ..未来 spawn LiDAR, Camera, ..

        // 命令通道
        let (cmd_tx, cmd_rx) = mpsc::channel(32);

        // 主循环
        tokio::spawn(async move {
            loop {
                select! {
                    cmd = cmd_rx.recv() => dispatch(cmd, &stm32).await;
                    _ = tick => { /* 状态监控 */ }
                    _ = cancel.cancelled() => break;
                }
            }
        });

        Self { cmd_tx, state, cancel }
    }

    // 对外 API：发命令
    pub async fn forward(&self, speed: i16) { self.cmd_tx.send(Command::Forward(speed)).await; }
    pub async fn stop(&self) { self.cmd_tx.send(Command::Stop).await; }
    // 对外 API：读状态
    pub async fn get_state(&self) -> RobotState { self.state.read().await.clone(); }
}

enum Command {
    Forward(i16),
    Backward(i16),
    Stop,
    GoTo(f32, f32),
    // ...
}
```

## 集成

```rust
// main_robot.rs
let robot = Robot::launch("/dev/myserial", 115200, CarType::X3Plus);
robot.forward(50).await;
```

## 人类评审

<!-- 在此区域写下评审意见 -->

