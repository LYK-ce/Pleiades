# 异构设备分析文档

Presented by KeJi
Date ： 2026-08-29

> 本文档梳理系统从「车」扩展到「车 + 无人机」过程中暴露的**六类异构来源**，作为插件化与设备抽象演进的前置背景。配套文档：`docs/plugin_design.md`。

---

## 1. 概述

Pleiades-Orion 分支最初只面向轮式小车（STM32 底盘），后扩展出无人机（MAVLink 飞控）。在「用统一 `MotionDevice` 抽象两类设备」的过程中，暴露出六类异构：

| 编号 | 异构来源 | 一句话 | 决定什么 |
|---|---|---|---|
| ① | 智能闭环开放层次 | 控制杆 vs 自动驾驶 | 驱动封装到哪一层 |
| ② | 运动自由度维度 | SE(2) vs SE(3) | 能力声明哪些维度 |
| ③ | 传感器搭载差异 | 外设组合各不相同 | 装配如何数据驱动 |
| ④ | 指令时序语义 | 锁存 vs 持续喂指令 | 动作接口语义约定 |
| ⑤ | 连接/协议 | 私有二进制帧 vs 标准协议 | 连接与协议抽象 |
| ⑥ | 状态/反馈 + 坐标约定 | 状态模型与坐标单位各异 | 状态 schema 与坐标契约 |

六类共同指向同一结论：**设备 = 能力声明 + 驱动插件 + 装配清单 + 接口契约**，而非一刀切的 `MotionDevice`，也非硬编码的类型枚举。

---

## 2. 异构一：智能闭环开放层次

### 2.1 现象

- **小车 STM32**：把「前进/后退」等**动作原语**直接开放给系统，相当于把**控制杆**交给系统。
- **无人机飞控**（ArduPilot/PX4）：把姿态/速度/位置闭环 + 状态估计 + 导航**全部包进飞控**，系统只下发**目标指令**。

一句话：**小车给的是「控制杆 + 仪表盘」，你得自己当司机；无人机给的是「自动驾驶」，你说目的地它自己导航。**

### 2.2 代码佐证：STM32 的三层接口

STM32 协议（`ros_stm32_protocol.md`）实际提供了三层控制接口，差速/速度环其实都已在 STM32 内部：

| 功能码 | 含义 | 谁做闭环 |
|---|---|---|
| `MOTOR 0x10` | 四轮独立 PWM | 开环，系统自己算四个轮子 |
| `CAR_RUN 0x11` | 预设方向 + 速度档位 | 差速在 STM32 内部 |
| `MOTION 0x12` | 速度矢量，「通过 PID 闭环控制」 | 速度环在 STM32 内部 |

系统当前实际用 **`CAR_RUN`**（`Src/Robot/control/device/stm32/mod.rs` L150-169）：

```rust
impl MotionDevice for STM32Device {
    fn move_forward(&self) { self.forward(self.forward_speed) }  // → send_car_run(Forward, 30)
    fn turn_left(&self)   { self.spin_left(self.turn_speed) }    // → send_car_run(SpinLeft, 10)
    fn stop(&self)        { STM32Device::stop(self) }            // → send_car_run(Stop, 0)
}
```

`CAR_RUN` 帧只带**预设方向**（`Forward/Backward/Left/Right/SpinLeft/SpinRight/Stop`）+ 一个**速度档位**（`[-100,100]`）。四个电机怎么协调（差速）由 STM32 完成。

> ⚠️ 澄清：差速、速度环（PID）**不是**系统要写的——它们都在 STM32 里。系统要写的是**更高层的一整套智能闭环**（见下）。

### 2.3 系统侧自建的闭环

小车开放的是「开环动作 + 原始传感器」，从「传感器 + 动作」到「我知道自己在哪、该怎么走」之间的智能，全部由系统侧代码承担：

| 组件 | 文件 | 职责 |
|---|---|---|
| 里程计 | `Robot/slam/odometry.rs` | 编码器/IMU 上报 → 位置积分（STM32 只报速度，不报「我在哪」） |
| 建图 | `Robot/slam/` | 激光点云 → 占据栅格地图 |
| 寻路 | `Robot/core/planning/pathfinder.rs` | D* Lite |
| 决策 | `Robot/core/executor.rs` | 三状态机 |
| 急停 | `Robot/core/robot.rs::check_emergency_stop` | 前方 ±45° &lt; 0.3m |

无人机飞控内置了上述大部分能力，系统只需给目标。因此两者的**接口暴露层次不同**。

### 2.4 本质

**「智能闭环」的开放层次不同**：小车把整个闭环暴露给系统，无人机把整个闭环收进飞控。差速/PID 这种电机级逻辑两边都在下位机，真正的分界线在「智能闭环」这一层。

### 2.5 方向

「运动设备」抽象不能再假设设备的智能闭环层次一致。应区分：

- **指令级运动设备**（command-level，飞控）：发高层指令，设备自己闭环，系统侧直通。
- **执行器级运动设备**（actuator-level，STM32）：系统侧补里程计/定位/闭环控制。

---

## 3. 异构二：运动自由度维度（SE(2) vs SE(3)）

### 3.1 现象

- **小车**：SE(2) 平面运动，3 自由度（x, y, yaw）。
- **无人机**：SE(3) 空间运动，6 自由度（x, y, z, roll, pitch, yaw）。

当前做法是**把无人机降维成「飞在天上的小车」**——垂直速度冻结，只保留水平运动 + 航向旋转。

### 3.2 代码佐证：vz 恒 0

`Src/Robot/control/device/mavlink/mod.rs` L50-60，动作 → 速度四元组 `(vx, vy, vz, yaw_rate)`：

```rust
fn action_to_velocity(a: MotionAction, vel_fwd: f32, yaw_rate_deg: f32) -> (f32, f32, f32, f32) {
    match a {
        MotionAction::MoveForward  => (vel_fwd, 0.0, 0.0, 0.0),   // vz 恒 0
        MotionAction::MoveBackward => (-vel_fwd, 0.0, 0.0, 0.0),  // vz 恒 0
        MotionAction::TurnLeft     => (0.0, 0.0, 0.0, -yaw_rate), // vz 恒 0
        MotionAction::TurnRight    => (0.0, 0.0, 0.0, yaw_rate),  // vz 恒 0
        MotionAction::Stop         => (0.0, 0.0, 0.0, 0.0),       // 停 = 悬停
    }
}
```

无人机的垂直速度 `vz` 被硬编码为 0。这个降维贯穿整个智能闭环：

| 层 | 现状 | 丢失 |
|---|---|---|
| 动作层 | `MotionAction` 仅 5 个 2D 动作 | 垂直动作（上升/下降/悬停） |
| 决策层 | `decide()` 输入 `x/y/yaw` | `z`、`vz`、姿态 |
| 任务层 | `Goto{x,y}` / `Circle{x,y}` | 高度（`Goto{x,y,z}`、巡航高度） |
| 地图层 | `OccupancyGrid` 2D 栅格 | 高度层 / 体素 |
| 寻路层 | D* Lite 2D 4 连通 | 3D/2.5D 寻路 |
| 避障层 | 前方 ±45° 扇形 | 上/下/四周 3D 避障 |

### 3.3 为什么「够用但非终极」

比赛里无人机大概率在**固定高度平面**做水平机动（俯视即 2D 投影），高度冻结后问题暂不暴露。但代价是丢掉无人机的核心能力：3D 机动、越障爬升、立体避障、多机「不同高度分层」的战术。

### 3.4 方向：能力维度声明 + 2.5D 规划

**不追求统一运动模型，而是把运动能力拆成正交维度，设备声明自己有哪些维度**：

```rust
pub struct MotionCapabilities {
    pub planar: bool,      // 水平平移 (vx, vy)      —— 车✓ 机✓
    pub yaw: bool,         // 航向旋转 (yaw_rate)     —— 车✓ 机✓
    pub vertical: bool,    // 垂直运动 (vz, 悬停/爬升) —— 车✗ 机✓
    // 每维带范围，如 vertical 的 [vz_min, vz_max]
}
```

决策/任务层按能力声明分支：

- **2D 任务**（`Goto{x,y}`、围圈）：跑在 `{planar + yaw}`，车机都能执行，机的高度冻结（维持现状）。
- **3D 任务**（起飞/降落/巡航高度/越障爬升）：只在 `vertical=true` 的设备上执行，车直接声明「不支持」。

规划层用 **2.5D** 而非全 3D：2D 建图/寻路为主，高度作为独立一维（分层高度图或 2D 地图 + 高度通道），避免 3D 体素寻路的复杂度爆炸。

---

## 4. 异构三：传感器搭载差异

### 4.1 现象

不同设备搭载的传感器各不相同（车挂编码器/雷达，机挂 IMU/GPS/测距），当前用 config 配置启动哪些设备。

### 4.2 代码佐证：硬编码的类型枚举

`Src/Config/config.rs` L145-150：

```rust
pub struct Robot_Config {
    pub chassis: Option<ChassisConfig>,        // STM32 轮式底盘
    pub lidar: Option<LidarConfig>,            // YDLIDAR 雷达
    pub flight_ctrl: Option<FlightCtrlConfig>, // Pixhawk 飞控
    pub obstacle_inflation_radius: Option<f32>,
}
```

三个设备类型是**硬编码字段**，各有专属 Config（`ChassisConfig` 有 `car_type/forward_speed/turn_speed`，`LidarConfig` 有 `port/baudrate`，`FlightCtrlConfig` 有 `connection/vel_fwd/yaw_rate_deg`）。

`Src/bootstrap.rs` 的 `robot_bootstrap`（L209-300）把这些开关**硬编码展开成 18 个参数**传给 `Robot::launch`，并写死「车机互斥」：

```rust
// 车机互斥（bootstrap.rs L269-277）
let chassis_enabled = if flight_ctrl_port.is_some() {
    if chassis_enabled { warn!("flight_ctrl 已启用，强制禁用 chassis（车机互斥）"); }
    false
} else { chassis_enabled };
```

`config.toml` 的 `[Robot]` 段目前仅一行 `obstacle_inflation_radius`（设备字段全是 `Option`，缺省即可运行）。

### 4.3 为什么「config 开关」非终极

| 局限 | 具体表现 | 后果 |
|---|---|---|
| 类型枚举式，非数据驱动 | 加新传感器（GPS/摄像头/超声波/深度相机）要改 4 处：`config.rs` 加字段 → `bootstrap.rs` 加解析 → `Robot::launch` 加参数 → `robot.rs` 加 spawn | 每换组合都动主程序 |
| 组合编译期固定 | config 只能开关「已知三种设备」 | 无法表达任意传感器组合 |
| 设备与能力脱钩 | config 配「设备类型」，不是「能力」 | 换同能力不同品牌传感器也要改代码 |
| 数据流硬编码 | `lidar→SLAM`、`encoder/IMU→里程计` 写死绑定 | 无法表达「有 IMU 无雷达」的降级组合 |

### 4.4 方向：设备清单 + 能力声明 + Registry

从「类型枚举 + 开关」转向「数据驱动装配」：

```toml
# 终极形态：设备是「清单项」，type 是数据（对应一个可插拔驱动）
[[robot.devices]]
type = "stm32_chassis"
port = "/dev/myserial"
baudrate = 115200

[[robot.devices]]
type = "ydlidar"
port = "/dev/rplidar"

[[robot.devices]]
type = "pixhawk_flightctrl"
connection = "/dev/ttyS0"
```

- **`type` 字符串 → `DeviceRegistry` 工厂**：加新传感器 = 写驱动 + 注册进表 + config 加一行，主程序装配逻辑零改动。
- **设备声明能力，系统按能力路由数据流**：按「我是测距/定位/IMU」路由，而非硬编码「lidar→SLAM」。
- **依赖/互斥声明化**：`flight_ctrl 需要 vertical 能力`、`chassis 与 flight_ctrl 互斥` 写成声明，而非写死在 bootstrap。

这与此前 `plugin_design.md` 的 `.so` 插件化**合流**：设备类型是数据（config 字符串）、驱动是插件（`.so` 或编译期注册）、能力声明是接口契约。

---

## 5. 异构四：指令时序语义（锁存 vs 持续喂指令）

### 5.1 现象

- **车（CAR_RUN）是锁存型**：发一次 `Forward`，STM32 持续前进，直到收到 `Stop` / `Reset`。
- **机（MAVLink GUIDED）是持续型**：飞控要求持续收到速度指令，约 2~3s 无新指令即自动回落悬停。

### 5.2 代码佐证

`Src/Robot/control/device/mavlink/mod.rs` 头部注释：

```rust
//! 自闭环：上层只发离散动作（move_forward / stop），设备内部用
//! 「期望动作 + 10Hz 保持 loop」持续下发速度指令
//! （GUIDED 速度指令约 2~3s 无新指令即回落悬停）。
```

`MavlinkDevice` 用 `desired: Arc<Mutex<MotionAction>>` + 10Hz 保持 loop 弥合差异；而 `STM32Device::forward` 只是一次 `try_send`（发一帧即完成，STM32 侧锁存）。

### 5.3 本质

统一的 `move_forward()` 这个「瞬时动作」接口，在两台设备上语义不同：车是「进入前进状态」，机是「这一拍要前进」。插件接口必须显式约定：**动作是「瞬时意图」还是「持续状态」**。

### 5.4 方向

上层只发「意图」，保持逻辑（10Hz loop）由设备驱动内部消化——这也解释了「指令级设备」驱动实现复杂度更高（自带保持 loop），而「执行器级设备」则是一次性下发。

---

## 6. 异构五：连接/协议（私有二进制帧 vs 标准协议）

### 6.1 现象

- 车：STM32 自定义私有二进制帧（`0xFF|DEV_ID|LEN|FUNC|DATA|CHKSUM`，串口 115200）。
- 机：MAVLink 标准协议（串口 921600，且天然支持 UDP）。

两者当前恰好都走串口，掩盖了差异；将来机走 UDP、车走 CAN 时会彻底裂开。

### 6.2 代码佐证

- `stm32/protocol.rs`：`feed_state_machine` 私有帧状态机。
- `mavlink/protocol.rs`：`serialize_message` MAVLink 序列化。
- `serial/port.rs` 只统一了「串口收发」，串口之上的协议适配是异构的。

### 6.3 方向

连接抽象与协议解析解耦：`Connection`（串口/UDP/CAN）与 `Protocol`（私有帧/MAVLink/…）分别抽象，设备驱动 = 一个 `Connection` + 一个 `Protocol` 的组合。

---

## 7. 异构六：状态/反馈模型 + 坐标/单位约定

### 7.1 现象

- 状态结构被迫分离：车的 `RobotState`（`encoders[4]`、`vx/vy/vz`）与机的 `Telemetry`（GPS、气压、姿态）是两个独立结构。
- 坐标/单位转换遍布代码：`aligned_world_pose`（yaw_offset 旋转 + origin 平移）、`yaw_rate_deg.to_radians()`（度→弧度）、`* 1000.0`（速度放大千倍）、NED 注释。

### 7.2 代码佐证

`mavlink/mod.rs` 的坐标对齐函数：

```rust
/// 坐标对齐：飞控 NED local 坐标 + yaw → 世界坐标（origin 平移 + yaw_offset 旋转）
fn aligned_world_pose(local_x, local_y, yaw, offset, origin) -> (f32, f32, f32) { ... }
```

以及 `action_to_velocity` 里 `yaw_rate_deg.to_radians()`（度→弧度）、`pack_motion` 里 `* 1000.0`（速度放大千倍）。

### 7.3 方向

插件接口契约必须提前定死：坐标系（NED/ENU/世界）、单位（m/s vs 档位、度 vs 弧度）、状态 schema。否则每加一个设备就在这些地方翻车一次。

---

## 8. 收束：六类共同指向

```
① 智能闭环开放层次（控制杆 vs 自动驾驶）──→ 驱动封装到哪一层
② 运动自由度维度（SE(2) vs SE(3)）      ──→ 能力声明哪些维度
③ 传感器搭载差异（外设组合各异）        ──→ 装配如何数据驱动
④ 指令时序语义（锁存 vs 持续喂）        ──→ 动作接口语义约定
⑤ 连接/协议（私有帧 vs 标准协议）       ──→ 连接与协议抽象
⑥ 状态/反馈 + 坐标约定（状态坐标各异）  ──→ 状态 schema 与坐标契约
        │
        ▼
   设备 = 能力声明 + 驱动插件 + 装配清单 + 接口契约
   （不再是一刀切 MotionDevice，也不是硬编码类型枚举）
```

六类分别回答了「驱动封装」「能力维度」「数据驱动装配」「动作语义」「连接协议」「状态坐标」六个正交问题，共同收敛到 `plugin_design.md` 的最终形态：**能力声明式 + 可插拔 + 数据驱动装配 + 接口契约**。

---

## 9. 待定决策点

1. 感知类设备数据上行通道选型：**回调 vs 轮询 vs 共享环形缓冲**（影响 `Robot::launch` 重构方式）。
2. `MotionCapabilities` 的维度粒度：是否引入「姿态」维度（roll/pitch）为将来 3D 机保留。
3. 2.5D 规划的具体形态：分层高度图 vs 2D 地图 + 高度通道。
4. Lua 去留：确认「运行时热更新」「低门槛脚本」是否仍被需要（若 executor 已走 `.so` 方案）。
5. 指令时序语义的统一：动作接口按「意图 + 设备侧闭环保持」统一，还是显式区分「锁存型 / 持续型」。
