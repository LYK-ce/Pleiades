# Vehicle Control 设计文档

Presented by KeJi
Created Date ： 2026-07-09
Modified Date ： 2026-07-09

---

## 目录

- [1. 系统概述](#1-系统概述)
- [2. 分层架构](#2-分层架构)
- [3. 各层职责](#3-各层职责)
  - [上层 — 命令源](#上层--命令源)
  - [核心层 — Robot](#核心层--robot)
  - [设备层 — Device](#设备层--device)
  - [串口层 — Serial](#串口层--serial)
- [4. 数据流](#4-数据流)
- [5. 文件结构](#5-文件结构)
- [6. 增加新 Device](#6-增加新-device)

---

## 1. 系统概述

Vehicle Control 是 Orion Robot 的子模块，负责机器人底盘运动控制、传感器数据采集与聚合。设计目标：

- **多设备支持**：底盘控制（STM32）、LiDAR、Camera 等通过统一模型接入
- **设备隔离**：每设备独立 tokio task，互不干扰
- **统一命令接口**：所有命令源（WebSocket、Lua、LLM Agent）通过同一个 `mpsc` 通道发送，由 Robot 主循环分派
- **共享状态**：各 Device 的 rx_loop 直接写入全局 `RobotState`，外部零延迟读取

---

## 2. 分层架构

```
┌──────────────────────────────────────────────────────────────────┐
│                        上　层                                     │
│                                                                   │
│   WebSocket        Lua             LLM Agent                      │
│   (9090)         (脚本)          (路径规划)                        │
│      │               │                │                           │
│      │   JSON → Command              │                            │
│      └───────────────┼────────────────┘                           │
│                      │ cmd_tx.send(Command)                       │
│                      ▼                                            │
├──────────────────────────────────────────────────────────────────┤
│                     核　心　层                                     │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │                      Robot                                   │ │
│  │                                                              │ │
│  │  select! {                                                   │ │
│  │    cmd_rx.recv() → dispatch → Device.*()                     │ │
│  │    tick(200ms)   → state.read() → 状态监控                   │ │
│  │    cancel        → 退出                                      │ │
│  │  }                                                           │ │
│  │                                                              │ │
│  │  RobotState (Arc<RwLock>)                                     │ │
│  └────────────────────────┬────────────────────────────────────┘ │
│                           │                                       │
├───────────────────────────┼───────────────────────────────────────┤
│                     设　备　层                                     │
│                           │                                       │
│   ┌───────────────────┐   │   ┌──────────┐   ┌──────────┐       │
│   │    STM32Device    │       │  LiDAR    │   │  Camera  │       │
│   │                   │       │  Device   │   │  Device  │       │
│   │  forward(speed)   │       │  (未来)   │   │  (未来)  │       │
│   │  stop()           │       └──────────┘   └──────────┘       │
│   │  beep(ms)         │                                          │
│   │  get_state()      │                                          │
│   └────────┬──────────┘                                          │
│            │                                                     │
├────────────┼─────────────────────────────────────────────────────┤
│            │              串　口　层                               │
│            │                                                     │
│   ┌────────┴──────────┐                                          │
│   │    spawn_port()    │  通用函数，不关心设备类型                  │
│   │                    │                                          │
│   │  tokio::spawn ──── TX loop → 写串口                          │
│   │  tokio::spawn ──── RX loop → 读串口 → 回调                   │
│   └───────────────────┘                                          │
│                                                                   │
├───────────────────────────────────────────────────────────────────┤
│                        硬　件　层                                   │
│                                                                   │
│   /dev/ttySTM32      /dev/ttyLidar      /dev/video0              │
│   (STM32F103RC)      (LiDAR传感器)       (摄像头)                  │
└───────────────────────────────────────────────────────────────────┘
```

---

## 3. 各层职责

### 上层 — 命令源

| 命令源 | 入口 | 格式 | 说明 |
|--------|------|------|------|
| **WebSocket** | `:9090` | `{"cmd":"forward","speed":50}` | 浏览器/手机遥控 |
| **Lua 脚本** | VM 绑定 | `robot.forward(50)` | 自动化任务（待重写） |
| **LLM Agent** | 未来 | `Command::GoTo(10, 20)` | 自然语言→导航 |

所有命令源通过 `robot.cmd_tx.clone()` 获得 `mpsc::Sender<Command>`，统一将 `Command` 枚举发送给 Robot。

### 核心层 — Robot

`Robot::launch()` 是系统的**唯一启动入口**，负责：

1. 创建全局 `RobotState`（`Arc<RwLock>`）
2. 调用各 Device 的 `spawn()`，传入 `state.clone()`
3. 创建 `mpsc::channel<Command>` 命令通道
4. `tokio::spawn` 主 `select!` 循环
5. 返回 `Robot` 句柄（含 `cmd_tx` 和 `state`）

```rust
pub struct Robot {
    pub cmd_tx: mpsc::Sender<Command>,    // 上层命令入口
    pub state: Arc<RwLock<RobotState>>,   // 全局状态
    cancel: CancellationToken,            // 退出信号
}
```

### 设备层 — Device

每个 Device 的职责：

| 方向 | 操作 |
|------|------|
| **下行** | 接收业务命令（`forward`/`stop`），pack 为协议帧，通过 `cmd_tx` 发给串口 |
| **上行** | rx_loop 回调中解析串口字节流（状态机），直接写入共享 `RobotState` |
| **协议** | 自包含：帧构建 + 状态机 + 数据解析，不暴露给上层 |

### 串口层 — Serial

`spawn_port()` 是**通用函数**，与设备无关：

```rust
pub fn spawn_port(
    port: &str,
    baudrate: u32,
    read_buf_size: usize,
    on_bytes: impl FnMut(&[u8]) + Send + 'static,
    cancel: CancellationToken,
) -> Result<mpsc::Sender<Vec<u8>>, String>
```

| 返回 | 说明 |
|------|------|
| `mpsc::Sender<Vec<u8>>` | 发送通道：Device pack 后的帧通过此通道发送 |
| `on_bytes` 回调 | 接收处理：rx_loop 每读到字节即调用，Device 在此做状态机解析 |

内部自动创建 TX + RX 两个 tokio task，共享同一个 `Arc<SerialPort>`（serial2 的 `&self` 读写无需 Mutex）。

---

## 4. 数据流

### 下行：命令流

```
上层 (WS / Lua / Agent)
  │  Command::Forward(50)
  ▼
cmd_tx.send()
  │
  ▼
Robot select! → dispatch()
  │  stm32.forward(50)
  ▼
STM32Device
  │  pack_car_run() → [FF,FC,07,11,...]
  ▼
cmd_tx.send(Vec<u8>)
  │
  ▼
TX loop → serial.write()
  ▼
/dev/ttySTM32 → STM32 → 电机
```

### 上行：传感器流

```
STM32 每 10ms 上报
  │  UART RX
  ▼
RX loop → serial.read(&mut buf)
  │  逐字节回调
  ▼
on_bytes → RxStateMachine::feed()
  │  完整帧
  ▼
update_state(&mut local_state)
  │  写回共享缓存
  ▼
state.write() ←  RobotState (全局)
  │
  ├── WS telemetry (10Hz)
  ├── Robot select! tick (200ms)
  └── 外部 get_state()
```

---

## 5. 文件结构

```
Src/Robot/
├── mod.rs               ← 模块入口 + re-export
├── state.rs             ← RobotState 等全局状态类型
├── websocket.rs         ← WebSocket 遥控服务 (上层)
├── core/
│   ├── mod.rs
│   ├── command.rs       ← Command 枚举定义
│   └── robot.rs         ← Robot::launch() + select! 主循环
└── control/
    ├── types.rs          ← CarType（设备层共用）
    ├── serial/
    │   ├── mod.rs
    │   └── port.rs       ← spawn_port() 通用函数
    └── device/
        ├── mod.rs
        └── stm32.rs      ← STM32 协议 + STM32Device
```

---

## 6. 增加新 Device

以增加 LiDAR 为例，只需 **4 步**：

### Step 1: 创建 `control/device/lidar.rs`

```rust
// ① 定义协议帧格式和解析逻辑
fn pack_lidar_cmd(...) -> Vec<u8> { ... }
fn LidarStateMachine { ... }
fn update_lidar_state(state: &mut LidarData, bytes: &[u8]) { ... }

// ② 定义 Device 句柄
pub struct LidarDevice {
    cmd_tx: mpsc::Sender<Vec<u8>>,
    cancel: CancellationToken,
}

impl LidarDevice {
    pub fn spawn(
        port: &str,
        baudrate: u32,
        state: Arc<RwLock<RobotState>>,  // 全局状态
    ) -> Result<Self, String> {
        // ③ 调 spawn_port，在回调里解析数据写 RobotState
        let cmd_tx = port::spawn_port(port, baudrate, 4096, move |bytes| {
            // 状态机解析 → 更新 state 的 LiDAR 相关字段
        }, cancel.clone())?;
        Ok(Self { cmd_tx, cancel })
    }

    pub fn start_scan(&self) { ... }
    pub fn stop_scan(&self)  { ... }
}
```

### Step 2: 在 `RobotState` 里加 LiDAR 字段（`state.rs`）

```rust
pub struct RobotState {
    // ... 已有字段 ...
    pub lidar: LidarData,   // ← 新增
}
```

### Step 3: 在 `Robot::launch()` 里实例化（`core/robot.rs`）

```rust
let lidar = LidarDevice::spawn("/dev/ttyLidar", 115200, state.clone())?;
```

### Step 4: 在 `Command` 和 `dispatch` 里加命令（`core/command.rs` + `core/robot.rs`）

```rust
enum Command {
    // ... 已有 ...
    LidarStart,
    LidarStop,
}

async fn dispatch(devices: &Devices, cmd: Command) {
    match cmd {
        // ...
        Command::LidarStart => devices.lidar.start_scan().await,
        Command::LidarStop  => devices.lidar.stop_scan().await,
    }
}
```

**不改串口层、不改 Robot 主循环结构。**

---

## 7. 技术决策总结

| 决策 | 说明 |
|------|------|
| 串口库 | `serial2` + `serial2-tokio`（`&self` 读写，无需 Mutex） |
| 并发模型 | 每设备 2 个 tokio task（TX + RX），共享主 runtime |
| RX 模式 | 回调闭包直接写 `RobotState`，无中间事件通道 |
| 命令模型 | `mpsc::Sender<Command>` 统一入口 |
| 退出机制 | `CancellationToken` + `select!`（主）+ Port Drop（兜底） |
| 参考 | PX4 飞控 task 模型、ROS2 `transport_drivers` `io_context` |
