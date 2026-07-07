# Task 1: Serial Thread

> 状态：设计中
> 创建日期：2026-07-07

## 目标

重构 Robot 底层串口通信，建立独立的多设备串口 Actor 模型。

## 背景

当前 `SerialIo` 的问题：

| 问题 | 说明 |
|------|------|
| Mutex 竞争 | `tokio-serial` 的 read/write 需要 `&mut self`，读写互锁，靠 10ms timeout 缓解而非解决 |
| 独立 runtime 太重 | `receive_loop` 用 `std::thread::spawn` + 独立 tokio runtime，每设备一个 OS 线程 |
| 生命周期失控 | `SerialIo` drop 后 receive_loop 线程仍在无限循环，线程泄露 |
| 无法优雅退出 | 没有 shutdown/cancel 机制 |
| 耦合严重 | 串口 I/O + 协议解析 + 状态管理全在 `SerialIo` + `protocol.rs` 里 |

## 设计

### 总体架构

```
每设备 = 1 个 struct 句柄 + 2 个 tokio task（TX + RX）

┌──────────────────────────────────────┐
│  Device 句柄 (struct)                 │
│  - cmd_tx: Sender<DeviceCommand>     │
│  - cancel: CancellationToken         │
│  - handles: (JoinHandle, JoinHandle) │
│                                      │
│  内部启动：                           │
│  ┌─ tokio::spawn( tx_loop )         │
│  │   while cmd = cmd_rx.recv()      │
│  │     bytes = protocol.pack(cmd)   │
│  │     serial.write(&bytes)         │
│  └──────────────────────────────────│
│  ┌─ tokio::spawn( rx_loop )         │
│  │   loop                           │
│  │     serial.read(&mut buf)        │
│  │     event = protocol.unpack(buf) │
│  │     event_tx.send(event)         │
│  └──────────────────────────────────│
└──────────────────────────────────────┘
```

### 通用协议 trait

```rust
trait SerialProtocol {
    type Command;
    type Event;

    fn pack(&self, cmd: &Self::Command) -> Vec<u8>;
    fn unpack(&self, bytes: &[u8]) -> Option<Self::Event>;
}
```

- `tx_loop` / `rx_loop` 是通用函数，只依赖 `SerialProtocol` trait
- 新增设备 = 实现自己的 `SerialProtocol` + 定义 struct 句柄

### 多设备示例

```rust
// STM32 控制板
let stm32 = STM32Device::spawn("/dev/ttySTM32", 115200, Stm32Protocol);
stm32.forward(50).await;
let state = stm32.get_state().await;

// LiDAR 雷达
let lidar = LidarDevice::spawn("/dev/ttyLidar", 115200, LidarProtocol);
let scan = lidar.scan().await;
```

### 技术决策

| 决策 | 说明 |
|------|------|
| 串口库 | 换用 `serial2` + `serial2-tokio`（`&self` 读写，无需 Mutex） |
| 并发模型 | 每设备 2 个 `tokio::spawn` task（TX + RX），共享进程 runtime |
| 退出控制 | `CancellationToken` + `JoinHandle` 优雅退出 |
| 协议解耦 | `SerialProtocol` trait，设备实现自己的 pack/unpack |
| 参考 | PX4 飞控独立 task 模型、ROS2 `transport_drivers` `io_context` 模型 |

### 依赖变更

```toml
# 替换
- tokio-serial = "5.5"
# 新增
+ serial2 = "0.2"
+ serial2-tokio = "0.2"
```

## 已决策

| 问题 | 决策 | 说明 |
|------|------|------|
| RX 数据粒度 | ✅ **方案 A**：Device 吐结构化事件 | 状态机留在 protocol，rx_loop 喂字节 → 输出 `Event` |
| 上层接口 | ✅ 上层直接读事件，不碰字节流 | `stm32.event_rx.recv() → Event::StateUpdate { vx, vy, battery }` |
| 断线处理 | ✅ 直接报错，task 退出 | `read()/write()` 返回 `Err` → 打 error 日志 → task return |
| 重连策略 | ✅ 不自动重连 | 上层感知 task 退出后自行决定 |
| RX 退出方式 | ✅ `select!`（主）+ Drop SerialPort（兜底） | A 失败后 B 保证一定退出 |
| read buffer | ✅ Device 构造时自定义 | STM32=512, LiDAR=4K, Camera=64K（上不封顶） |

## 设计完成，待实现

### 文件结构

```
Src/Robot/control/
├── mod.rs
├── types.rs                (共用：CarType, MotionState, RobotState)
├── serial/                 (新增：通用串口抽象)
│   ├── mod.rs
│   └── port.rs             (SerialProtocol trait, spawn_port 通用函数)
└── device/                 (新增：各设备实现)
    ├── mod.rs
    └── stm32.rs            (STM32Protocol, STM32Device, forward/stop/get_state)

移除：
✗ protocol.rs    → 合并到 device/stm32.rs
✗ serial_io.rs   → 替换为 serial/port.rs
✗ capability.rs  → 替换为 device/stm32.rs
```

### 实现步骤

1. 添加依赖 `serial2` + `serial2-tokio`，移除 `tokio-serial`
2. 新建 `control/serial/port.rs`：`SerialProtocol` trait + 通用 `spawn_port()` 函数
3. 新建 `control/device/stm32.rs`：实现 `SerialProtocol` + `STM32Device` 句柄
4. 更新 `main.rs` + `main_robot.rs` + `server.rs` + Lua 绑定，适配新接口
5. 删除旧文件（`protocol.rs`, `serial_io.rs`, `capability.rs`）
6. 编译 + 测试

## 人类评审

<!-- 在此区域写下评审意见 -->

