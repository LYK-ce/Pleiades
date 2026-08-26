# UAV Device 设计文档（MAVLink 飞控设备接入）

Presented by KeJi
Created Date ： 2026-08-25
Modified Date ： 2026-08-25

> 状态：**设计讨论稿**（部分决策待定，见 §6）
> 关联文档：`universal_robot_design.md`（通用机器人定稿方案）、`UAV.md`（DRF450 硬件规格）、`robot_arch.md`（Robot 现状架构）

---

## 1. 背景与目标

把 UAV 仓库（`rust-mavlink-test`）中**已验证**的「Rust 操纵 MAVLink 飞控」能力，迁入 Orion，成为
`Src/Robot/control/device/` 下的**第三个设备**（车 `stm32` / 雷达 `lidar` / 飞控 `mavlink`），
实现 `MotionDevice` trait。对应 task_22 遗留的「机脚本 / MavlinkDevice」与
`universal_robot_design.md` 已定稿的 `FlightCtrlDevice`。

**目标形态**：设备自闭环 —— 上层只发离散动作（`move_forward/stop`），
「MAVLink 速度指令需 ~10Hz 持续下发」这个时序细节由设备内部消化，对上层完全透明。

---

## 2. 现状资产

### 2.1 UAV 侧（rust-mavlink-test，已验证）

UAV Rust 代码分两层：

| 层 | 文件 | 性质 | 内容 |
|---|---|---|---|
| 控制原语 | `mavlink_device.rs` | 同步、发命令**不确认** | `set_mode_send` / `arm_disarm_send` / `takeoff_send` / `land_send` / `set_velocity_target_send` + RX 解析 → `Telemetry` |
| 机动编排 | `uav_common.rs` | **async、带确认回读** | `set_mode`（轮询 custom_mode）/ `takeoff`（轮询高度）/ `land`（轮询 armed）/ `forward` / `turn_degrees` / `hover` |

### 2.2 Orion 侧可复用积木

| 资产 | 位置 | 用途 |
|---|---|---|
| `MotionDevice` trait | `Src/Robot/device.rs` | 统一运行期动作接口 |
| `MotionAction` 枚举 | `Src/Robot/core/state.rs:113` | 期望动作的理想类型（`MoveForward(i16)/MoveBackward/TurnLeft/TurnRight/Stop`） |
| `spawn_port` | `Src/Robot/control/serial/port.rs` | 串口 TX/RX 后台 task（`cancel` 退出），零改动复用 |
| `spawn_slam` 骨架 | `Src/Robot/slam/task.rs:56-63` | `interval + select!{tick/cancel}` 定时 loop 范例 |
| `LidarDevice::spawn` | `Src/Robot/control/device/lidar/mod.rs` | 「设备内部打包 tokio 组」范例 |
| config 预留 | `Src/bootstrap.rs` | `flight_ctrl.enabled`（缺省 false，仅 warn） |

---

## 3. 设备设计

### 3.1 定位

`FlightCtrlDevice`（代码 struct 名 `MavlinkDevice`）= 自包含能力单元，四块内聚：

- **config**：`FlightCtrlConfig { enabled, connection }`（`connection` 待补字段）
- **构造**：`MavlinkDevice::start(&FlightCtrlConfig) -> Box<dyn MotionDevice>`（`start` 不进 trait）
- **行为**：`impl MotionDevice`（离散动作 → 更新期望动作）
- **状态写入**：飞控遥测 → `RobotState`（z = 飞控 EKF 高度）

### 3.2 自闭环结构（核心）

```rust
struct MavlinkDevice {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    desired: Arc<std::sync::Mutex<MotionAction>>,  // 期望动作（std 锁，见 §4 决策 3）
    telemetry: Arc<RwLock<Telemetry>>,
    cancel: CancellationToken,
}

// trait 方法只更新「期望动作」：纳秒级、零 IO、零 await
impl MotionDevice for MavlinkDevice {
    fn move_forward(&self, speed: i16) -> Result<(), String> {
        *self.desired.lock().unwrap() = MotionAction::MoveForward(speed);
        Ok(())
    }
    fn stop(&self) -> Result<(), String> {
        *self.desired.lock().unwrap() = MotionAction::Stop;
        Ok(())
    }
    // move_backward / turn_left / turn_right 同理
    fn shutdown(&self) { self.cancel.cancel(); }
}

// spawn 时起 10Hz 保持 loop（照抄 spawn_slam 骨架）
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        select! {
            _ = interval.tick() => {
                let action = desired.lock().unwrap().clone();  // 锁内只 clone，锁外翻译+发送
                let msg = action_to_mavlink(action);
                if let Err(e) = send(msg) { warn!("[Mavlink] 指令下发失败: {e}"); }  // 失败容忍
            }
            _ = cancel.cancelled() => break,
        }
    }
});
```

### 3.3 翻译表（唯一「业务」逻辑）

```rust
const VEL_FWD: f32 = 0.3;                        // 前进/后退速度 m/s（机体系 vx）
const YAW_RATE: f32 = (15.0f32).to_radians();    // 转向角速度 rad/s（= 15°/s）
fn action_to_mavlink(a: MotionAction) -> MavMessage {
    match a {
        MoveForward(_) => velocity(vx=+VEL_FWD, vy=0, vz=0, yaw_rate=0),
        MoveBackward(_)=> velocity(vx=-VEL_FWD, vy=0, vz=0, yaw_rate=0),
        TurnLeft(_)    => velocity(vx=0, vy=0, vz=0, yaw_rate=-YAW_RATE),  // 左转=负（NED 正=右转/顺时针）
        TurnRight(_)   => velocity(vx=0, vy=0, vz=0, yaw_rate=+YAW_RATE),  // 右转=正
        Stop           => velocity(0, 0, 0, 0),   // 停 = 悬停 = 持续发零速度
    }
}
```

> **yaw 符号**：`turn_left` ↔ `yaw_rate` 正负号需按机体系 NED 核对一次（UAV `turn_degrees` 注释「正=顺时针/右转」），避免左右反转。

### 3.4 一次性命令（供 ManualCmd，不进 trait）

`takeoff_send / land_send / set_mode_send / arm_disarm_send` 等，同步 `try_send`，
从 UAV `mavlink_device.rs` 直接迁移。设计文档已定：**takeoff/land 归 ManualCmd**（手动遥控阶段），不进 action（Lua 决策动作集）。

**已敲定的固定命令语义**：

| 命令 | 参数 | 语义 |
|---|---|---|
| `takeoff` | 无参 | 固定起飞到 **1.8 m**（`TARGET_ALT = 1.8`）；到位判定 `alt >= 1.8 × 0.9 = 1.62 m`；起飞前需 GPS 3D fix |
| `land` | 无参 | 原地降落（`NAV_LAND`），等 armed 位清除（触地自动上锁） |

> 解锁（arm）不提供程序命令：**由遥控器手动解锁**，程序只读 HEARTBEAT 的 armed 位。

### 3.5 RX：遥测 → 状态

- `parse_one_frame` + `handle_message` 从 UAV 迁移（MAVLink v1/v2 帧提取 + 解析 → `Telemetry`）。
- 解锁检测：HEARTBEAT `base_mode` 含 `MAV_MODE_FLAG_SAFETY_ARMED` → `Telemetry.armed`；armed `false→true` 跳变 = 解锁事件（复用 UAV 已有 `was_armed != t.armed` 检测）。
- `Telemetry` → `RobotState` 映射：`z = relative_alt`、`vz = 垂直速度`、`yaw/roll/pitch = ATTITUDE`；`x/y` 经 §3.6 坐标对齐后写入。

### 3.6 坐标系对齐（已定案）

- **平移**：`origin = (64, 64)`（复用车的 origin 注入；飞机在 origin 点解锁起飞）。
- **旋转**：`yaw_offset` = 解锁时刻（armed `false→true` 跳变）读一次飞控 `ATTITUDE.yaw`，之后固定。
- 换算公式（世界 x 轴 = 解锁时机头朝向）：

```rust
let wx = origin.0 + local_x * offset.cos() + local_y * offset.sin();
let wy = origin.1 - local_x * offset.sin() + local_y * offset.cos();
let yaw_world = yaw_fc - offset;
```

- 兜底：若改为「所有设备朝北」，`yaw_offset = 0` 即可，无需改代码。

---

## 4. 关键设计决策

| # | 决策 | 结论 |
|---|---|---|
| 1 | 持续性指令 | **设备自闭环**（期望动作 + 10Hz 保持 loop），不依赖上层调用频率 |
| 2 | 期望动作类型 | 复用 `MotionAction`（`core/state.rs`），1:1 映射 trait 方法 |
| 3 | 期望动作的锁 | **`std::sync::Mutex`**（trait 方法同步、不可 await；临界区纳秒级），不用 tokio RwLock |
| 4 | stop 语义 | 持续发零速度（悬停），不是「什么都不发」 |
| 5 | 发送失败 | `try_send` 失败 `warn` 并跳过本帧，下一 tick（100ms）自然补发，不 panic |
| 6 | takeoff/land/切模式/解锁 | 归 `ManualCmd`（Rust 直接下发），不进 `MotionDevice`/action；**takeoff 固定 1.8m、land 无参**；**解锁用遥控器**（程序只读 armed 位） |
| 7 | 命令确认 | **第一版不做 COMMAND_ACK**，沿用 UAV「回读遥测字段」确认（切模式看 custom_mode / 起飞看高度 / 降落看 armed）；后续联调排障困难时再加 |
| 8 | 设备命名 | 模块/文件 `mavlink`，struct `MavlinkDevice`；概念层称 `FlightCtrlDevice` |
| 9 | 飞行模式 | **全程 GUIDED**（`custom_mode=4`）；不用 loiter（loiter 不接受速度指令，程序无法控制飞行） |
| 10 | 速度缩放 | **固定速度**（忽略 i16 参数）：`VEL_FWD=0.3 m/s`、`YAW_RATE=15°/s`；需要变速时再改翻译表 |
| 11 | 坐标系对齐 | origin(64,64) 平移 + 解锁时刻记 `yaw_offset`（armed false→true 跳变读一次 yaw）+ 位置旋转/朝向减法；兜底 `yaw_offset=0` 即「全朝北」 |
| 12 | 运行方式 | **SSH 前台运行**（禁止后台守护）；断连即进程终止、靠飞控 failsafe 保护。后台运行会「一直向前」，需先补手动命令看门狗 |

---

## 5. 待定项（§ 待对齐）

| # | 待定项 | 说明 |
|---|---|---|
| 1 | `FlightCtrlConfig.connection` | config 结构体字段是否已定义，还是需补（bootstrap.rs 当前仅 `enabled`） |
| 2 | mavlink crate 依赖 | Orion `Cargo.toml` 需加 `mavlink 0.18`（`dialect-ardupilotmega` + `dialect-common`），确认可接受 |

---

## 6. 安全与运维注意事项（重要）

### 运行方式：SSH 前台运行（强制）

**必须用 SSH 前台运行 Orion，禁止 nohup / tmux / systemd 后台守护。** 原因：

- **SSH 前台**：地面站 SSH 断连 → 进程收到 SIGHUP 终止 → Orion 停止发 MAVLink 消息 → 飞控两道保险先后生效（速度指令 2~3s 超时回落悬停 + GCS 失联保护 ~5s）→ 飞机自动停住。
- **后台运行（危险）**：地面站断连 → Orion 仍在后台运行 → 继续发速度指令 + 心跳 → 飞控认为一切正常，两道保险**均不触发** → 手动模式下飞机会**一直向前**（期望动作锁存，无人来发 stop）。

> 根本原因：飞控的自动保护都以「Orion 停止发消息」为前提，后台运行时该前提不成立。

### 兜底

- 遥控器**直连飞控**（不经地面站/Orion），随时拨模式开关（回 Loiter/Stabilize）即可人工接管。
- 未来若确需后台运行，**必须先实现「手动命令看门狗」**（手动模式下 N 秒未收到地面站消息则自动置 `Stop` 悬停）。

---

## 7. 附：实现步骤

1. `Cargo.toml` 加 mavlink 依赖。
2. 建 `Src/Robot/control/device/mavlink/`：`mod.rs`（`MavlinkDevice` + `MotionDevice` impl + 保持 loop）、`protocol.rs`（`parse_one_frame` / `handle_message` / 命令打包）、`types.rs`（`Telemetry`）。
3. `control/device/mod.rs` 加 `pub mod mavlink;`。
4. `bootstrap.rs` 补 `FlightCtrlConfig` + 遍历 spawn 接入。
5. `MotionDevice` 自闭环 + 翻译表 + 一次性命令。
6. 遥测 → `RobotState` 映射。
7. 单测（复用 `spawn_mock` 模式：channel 模拟 TX/RX）。

---

## 附：术语

- **期望动作（desired action）**：设备内部维护的「当前应执行动作」，由 `MotionDevice` 方法写入、保持 loop 读取。
- **保持 loop（keepalive loop）**：10Hz 定时循环，把期望动作持续翻译为 MAVLink 速度指令下发。
- **一次性命令**：takeoff/land/切模式/解锁等「发一条即完成」的命令，不带持续性。
