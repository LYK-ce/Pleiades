# Workbook Orion — 进展记录

> 分支: Pleiades-Orion
> 日期: 2026-06-22

---

## 已完成

### 1. Robot 模块 (`Src/Robot/`)

STM32 串口协议实现，控制 Rosmaster M1 系列小车。

```
Src/Robot/
├── mod.rs              ← 模块声明 + 全局 Robot 单例
└── control/
    ├── mod.rs
    ├── types.rs        ← CarType, MotionState, RobotState
    ├── protocol.rs     ← 帧构建、校验和、接收状态机、数据解析（24 测试）
    ├── serial_io.rs    ← tokio-serial 封装 + 后台接收（独立线程）
    └── capability.rs   ← RobotControl trait + Robot 实现
```

**关键设计决策：**
- 接收任务用 `std::thread::spawn` + 独立 runtime，不受 Lua 脚本生命周期影响
- 串口读写共享 `Arc<Mutex<SerialStream>>`，10ms 超时避免长期占锁
- 传感器数据以 `RobotState` 缓存，`get_state()` 零阻塞读取

### 2. WebSocket 遥控 (`Src/Robot/server.rs`)

- 独立线程运行，监听 `0.0.0.0:9090`
- JSON 协议：`{"cmd":"forward"}`, `{"cmd":"stop"}` 等
- 连接断开自动停车
- 200ms 定时推送遥测到 EventBus

### 3. Lua 绑定 (`Src/VM/capability_binding.rs`)

`register_robot_caps()` — 注册 `robot` 全局表：

```lua
robot.open(port, baudrate, car_type)
robot.forward(speed)  robot.backward(speed)
robot.left(speed)     robot.right(speed)
robot.spin_left(speed) robot.spin_right(speed)
robot.stop()          robot.beep(ms)
robot.get_state() → {vx, vy, vz, battery, attitude, encoders, ...}
```

### 4. PC 端遥控页面 (`Tool/robot_control.html`)

- WASD 键盘控制 + 触摸按钮
- 可配置 IP/端口
- 实时遥测显示（速度、电池、姿态、编码器）
- 按住移动，松手即停

### 5. 测试脚本 (`programs/user/robot_test.lua`)

前进 2 秒 → 停车 → 蜂鸣，过程中打印传感器数据。

### 6. 系统集成 (`Src/main.rs`)

Phase 5.6：
```rust
let robot = Arc::new(Robot::new());
init_robot(robot.clone());
robot.open("/dev/myserial", 115200, CarType::X3Plus).await?;
spawn_robot_ws_server(9090, event_bus.clone());
```

---

## 修复的 Bug

| Bug | 原因 | 修复 |
|---|---|---|
| HTML 按键无效 | 两个独立的 Robot 实例（Lua 和 WS 各一个） | 统一为全局单例 `Src/Robot/mod.rs` |
| 停止命令无效 | 串口读取无超时，接收无限阻塞占锁 | 设 `timeout(10ms)` |
| 传感器始终为 0 | 接收任务绑在 Lua 临时 runtime，脚本结束即死 | 改用独立线程 + 独立 runtime |
| 重复打开串口报错 | Lua 脚本不检查是否已打开 | 加 `robot.is_open()` 检查 |
| 退出后小车继续跑 | 进程退出时没有发 stop 帧 | 待修复 |

---

## 待修复

- [ ] 进程退出时发送 stop 帧（Core shutdown 路径或 SerialIo Drop）
- [ ] x86_64 → aarch64 交叉编译（GLIBC 版本不匹配）
- [ ] 新增 `Src/Robot/server.rs` 中的 `Arc<Mutex<WriteHalf>>` 改为 Mutex 方案已回退

---

## 新增依赖

```toml
tokio-serial = "5.5"        # 串口通信
tokio-tungstenite = "0.24"  # WebSocket 服务端
```

---

## 测试

- `robot::control::protocol::tests` — 24 项全部通过
- 帧构建（6项）、校验和（3项）、状态机（3项）、解析（5项）、打包（4项）、LEN 计算、往返测试
