# task_22_4_mavlink_device — 飞控设备接入：MAVLink 自闭环驱动

> 状态：**已实施**（2026-08-25），编译通过 + robot 125 测试全过（含 3 个 mavlink 单测）
> Created Date ： 2026-08-25
> Modified Date ： 2026-08-25
> 依赖：`Task/task_22_universal_robot.md`（通用机器人）、`Task/task_22_3_single_writer.md`（单写者收敛）
> 设计文档：`docs/design_doc/uav_device.md`（已定稿）
> 参考实现：`/vepfs-mlp2/c20250205/240804016/Workspace/UAV/rust-mavlink-test/`（已验证的飞控操纵代码）
> 分支：`Universal-Robot`（稳定分支 `Pleiades-Orion` / `Godot-Library` 不动）

---

## 一、目标

把 UAV 仓库 `rust-mavlink-test` 中**已验证**的「Rust 操纵 MAVLink 飞控」能力，迁入 Orion，成为 `Src/Robot/control/device/` 下的**第三个设备**（车 `stm32` / 雷达 `lidar` / 飞控 `mavlink`），实现 `MotionDevice` trait。落地 task_22 遗留的「机脚本 / MavlinkDevice」，使无人机能以与车**完全一致的动作语义**（前进/后退/左转/右转/停）被上层驱动。

**核心诉求：设备自闭环。** 上层只发离散动作（`move_forward` / `stop`），「MAVLink GUIDED 速度指令需 ~10Hz 持续下发（约 2~3s 无新指令即回落悬停）」这个时序细节由设备内部消化，对上层完全透明。

---

## 二、背景

1. **task_22 已定稿**「车/机/船通用插件化三层架构」，`MotionDevice` trait 已建立（`Src/Robot/device.rs`），STM32 / Lidar 已改造为实现。
2. **飞控是最后一个待接入设备**：`FlightCtrlConfig { enabled, connection }` 已在 `config.rs` 预留（L166-170），`bootstrap.rs` 已读 `enabled` 但仅 `warn`「尚未实现」（L218-220）。
3. **UAV 仓库已有可复用两层**：
   - `mavlink_device.rs`：控制原语（同步发命令，不确认）+ RX 解析 → `Telemetry`；
   - `uav_common.rs`：确认回读 + 机动编排（async）。
4. **车机本质差异（本 task 的动因）**：车发一条 `FUNC_CAR_RUN` 持续生效直到 `stop`；机发一条 GUIDED 速度指令 2~3s 后自动回落。因此机设备需要额外的「保持 loop」。

---

## 三、文件架构

### 3.1 新增目录 `Src/Robot/control/device/mavlink/`

| 文件 | 职责 |
|---|---|
| `mod.rs` | `MavlinkDevice` 结构 + `spawn` + `MotionDevice` impl + **保持 loop** + 翻译表 + 一次性命令方法 + 坐标对齐 |
| `protocol.rs` | MAVLink 帧提取/解析（`parse_one_frame` / `handle_message`）+ 命令打包（`send_*`） |
| `types.rs` | `Telemetry` 遥测快照结构 |
| `constants.rs` | 常量（速度/高度/模式号/超时等） |

> 划分依据：对齐 stm32 设备（`mod.rs` + `protocol.rs` + `constants.rs`）与 lidar 设备（`mod.rs` + `parser.rs` + `types.rs` + `constants.rs` + `checksum.rs`）的既有风格。

### 3.2 改动文件

| 文件 | 改动 |
|---|---|
| `Cargo.toml` | 加 `mavlink = { version = "0.18", default-features = false, features = ["std", "dialect-ardupilotmega", "dialect-common"] }` |
| `Src/Robot/control/device/mod.rs` | 加 `pub mod mavlink;` |
| `Src/Config/config.rs` | `FlightCtrlConfig` 已有 `enabled` + `connection`，**无需改**（或补 `baudrate: Option<u32>`） |
| `Src/Config/config.toml` | `[Robot.flight_ctrl]` 补实际连接参数（`enabled` / `connection` / `baudrate`） |
| `Src/bootstrap.rs` | `robot_bootstrap` 补 flight_ctrl 分支：解析 `connection`/`baudrate` → 传入 `Robot::launch` |
| `Src/Robot/core/robot.rs` | `Robot::launch` 加 flight_ctrl 参数并 spawn；`Robot` 结构持有 `Option<Arc<MavlinkDevice>>`；`dispatch`/`apply_action` 由 `&STM32Device` 改为 `&dyn MotionDevice`（通用动作统一），机特有命令单独分支 |
| `Src/Robot/core/command.rs` | `ManualCmd` 加 `Takeoff` / `Land` 变体（`SetGuided` 不进命令，由 spawn 自动切） |

### 3.3 复用（零改动）

- `Src/Robot/control/serial/port.rs` 的 `spawn_port`（串口 TX/RX 两 task，`cancel` 退出）
- `Src/Robot/device.rs` 的 `MotionDevice` trait
- `Src/Robot/core/state.rs` 的 `MotionAction` 枚举（作为「期望动作」类型）
- `Src/Robot/slam/task.rs` 的 `spawn_slam`（`interval + select!{tick/cancel}` 定时 loop 骨架，照抄改 100ms）

---

## 四、代码运行架构

### 4.1 运行时 task 组（3 个）

| task | 来源 | 职责 |
|---|---|---|
| **RX loop** | `spawn_port` 内置 | 读串口 → `parse_one_frame` → `handle_message` → 更新 `Telemetry` → `try_write` 全量覆盖 |
| **TX loop** | `spawn_port` 内置 | 从 channel 取字节 → 写串口 |
| **保持 loop** | **新增** | 10Hz 读「期望动作」→ 翻译 → `try_send` 塞 TX channel |

### 4.2 数据流全景

```
                    ┌─────────────────────────────────────────────────────────┐
                    │              MavlinkDevice（自闭环）                      │
                    │                                                         │
  MotionDevice 方法  │  期望动作: Arc<std::sync::Mutex<MotionAction>>          │
  (move_forward等) ─┼─▶ 同步写入（纳秒级，零 IO）                              │
                    │        │                                                │
                    │        ▼ (10Hz 读 + clone)                              │
                    │  保持 loop ──翻译──▶ set_velocity_target_send ──try_send─┼─▶ [TX channel] ─▶ TX loop ─▶ 串口 ─▶ 飞控
                    │                                                         │
  一次性命令         │  takeoff_send / land_send（ManualCmd 触发）            │
  启动自动           │  set_mode_send(GUIDED)（spawn 内自动）                  │
                    ┼─▶ 同步 try_send ────────────────────────────────────────┤
                    │                                                         │
  串口 RX ──▶ RX loop ─▶ parse_one_frame ─▶ handle_message ─▶ Telemetry ──────┼─▶ RobotState（含坐标对齐）
                    └─────────────────────────────────────────────────────────┘
```

### 4.3 自闭环核心（期望动作）

```rust
struct MavlinkDevice {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    desired: Arc<std::sync::Mutex<MotionAction>>,   // 期望动作（std 锁，见设计文档决策 3）
    state: Arc<RwLock<Telemetry>>,
    yaw_offset: std::sync::Mutex<f32>,              // 解锁时刻记录
    cancel: CancellationToken,
}

// trait 方法只更新期望动作（同步、纳秒级、零 await）
impl MotionDevice for MavlinkDevice {
    fn move_forward(&self, _speed: i16) -> Result<(), String> {
        *self.desired.lock().unwrap() = MotionAction::MoveForward(0);  // 固定速度，忽略参数
        Ok(())
    }
    fn stop(&self) -> Result<(), String> {
        *self.desired.lock().unwrap() = MotionAction::Stop;            // 停 = 悬停
        Ok(())
    }
    // move_backward / turn_left / turn_right 同理；shutdown 取消 cancel
}
```

**保持 loop**（`spawn` 时起，10Hz）：

```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        select! {
            _ = interval.tick() => {
                let action = desired.lock().unwrap().clone();   // 锁内只 clone
                let (vx, vy, vz, yaw_rate) = action_to_velocity(action);  // 锁外翻译
                if let Err(e) = set_velocity_target_send(vx, vy, vz, yaw_rate) {
                    warn!("[Mavlink] 指令下发失败: {e}");         // 失败容忍，下一 tick 补发
                }
            }
            _ = cancel.cancelled() => break,
        }
    }
});
```

**翻译表**（唯一业务逻辑；固定速度，忽略 i16 载荷）：

```rust
fn action_to_velocity(a: MotionAction) -> (f32, f32, f32, f32) {  // (vx, vy, vz, yaw_rate)
    match a {
        MoveForward(_)  => ( VEL_FWD, 0.0, 0.0, 0.0),
        MoveBackward(_) => (-VEL_FWD, 0.0, 0.0, 0.0),
        TurnLeft(_)     => (0.0, 0.0, 0.0, -YAW_RATE),  // NED：yaw_rate 正=顺时针/右转，左转为负
        TurnRight(_)    => (0.0, 0.0, 0.0,  YAW_RATE),
        Stop            => (0.0, 0.0, 0.0, 0.0),                  // 悬停 = 零速度
    }
}
```

### 4.4 一次性命令（同步 try_send，纯触发、不带确认回读）

| 方法 | MAVLink 命令 | 说明 |
|---|---|---|
| `set_mode_send(GUIDED)` | `MAV_CMD_DO_SET_MODE` | 切 GUIDED（param1=1, param2=4）；**spawn 启动时自动发**，不进 ManualCmd |
| `takeoff_send()` | `MAV_CMD_NAV_TAKEOFF` | 固定 `param7 = 1.8`（`TARGET_ALT`） |
| `land_send()` | `MAV_CMD_NAV_LAND` | 原地降落（飞控自主触地 + 上锁） |
| `arm_disarm_send(arm)` | `MAV_CMD_COMPONENT_ARM_DISARM` | 保留备用；实飞由遥控器解锁，程序只读 armed 位 |

> 确认回读（等高度到位 / 等 armed 清除）本 task **不做**（决策 #7：第一版不做 COMMAND_ACK，确认靠上层读 `Telemetry` 展示）。

### 4.5 坐标对齐（设计文档 §3.6）

- **平移**：`origin = (64, 64)`（复用车的 origin 注入；飞机在 origin 点解锁起飞）。
- **旋转**：`yaw_offset` = 解锁时刻（armed `false→true` 跳变）读一次 `ATTITUDE.yaw`，之后固定。
- 换算（遥测 → `RobotState` 时）：

```rust
let wx = origin.0 + local_x * offset.cos() + local_y * offset.sin();
let wy = origin.1 - local_x * offset.sin() + local_y * offset.cos();
let yaw_world = yaw_fc - offset;
```

---

## 五、详细实施步骤

### 步骤 1：加 mavlink 依赖

`Cargo.toml` 的 `[dependencies]` 加：

```toml
mavlink = { version = "0.18", default-features = false, features = ["std", "dialect-ardupilotmega", "dialect-common"] }
```

> 注意：0.18 起方言移到 `mavlink::dialects::*`，解析/序列化在 `mavlink-core`（经 `mavlink` re-export）。照 UAV `Cargo.toml` 抄即可。

### 步骤 2：`types.rs` — Telemetry 遥测快照

从 UAV `mavlink_device.rs` 迁移 `Telemetry` 结构，字段保留：`system_id/component_id/armed/custom_mode/roll/pitch/yaw/battery_voltage/relative_alt/lat/lon/vel_*/hdg/local_x/local_y/local_z/local_vx/local_vy/local_vz/ekf_flags/fix_type/satellites_visible` + 各消息计数（`heartbeat_count/attitude_count/global_pos_count/local_pos_count/...`，供状态新鲜度判定用）。

### 步骤 3：`constants.rs`

```rust
pub const MODE_GUIDED: u32 = 4;
pub const TARGET_ALT: f32 = 1.8;              // takeoff 固定高度
pub const VEL_FWD: f32 = 0.3;                 // 前进/后退速度 m/s
pub const YAW_RATE_DEG: f32 = 15.0;           // 转向角速度 °/s
pub const FORCE_DISARM_MAGIC: f32 = 21196.0;  // 强制上锁魔法数字（保留备用）
pub const TAKEOFF_ARRIVE_RATIO: f32 = 0.9;    // 到位判定比例（1.8×0.9=1.62m）
```

### 步骤 4：`protocol.rs` — 帧解析 + 命令打包

从 UAV `mavlink_device.rs` 迁移两段：

1. **`parse_one_frame(buf) -> Option<(MavHeader, MavMessage)>`**：MAVLink v1（STX=0xFE）/ v2（STX=0xFD）帧提取 + `mavlink::read_v1_msg/read_v2_msg` 解析，坏帧丢弃、噪声字节 `remove(0)` 重同步。
2. **`handle_message(t, header, msg)`**：按消息类型更新 `Telemetry`（HEARTBEAT 的 armed 位/custom_mode、ATTITUDE、SYS_STATUS、GLOBAL_POSITION_INT、LOCAL_POSITION_NED、EKF_STATUS_REPORT、GPS_RAW_INT、STATUSTEXT）。
3. **命令打包**：`send_heartbeat` / `request_telemetry_streams` / `set_mode_send` / `arm_disarm_send` / `takeoff_send` / `land_send` / `set_velocity_target_send`（每条构造 `MavMessage` → `send_message` 序列化 → `try_send`）。

> 本步是「纯迁移」，逻辑与 UAV 一致，仅改 crate 路径引用。

### 步骤 5：`mod.rs` — MavlinkDevice 主体

按 §4.3~4.5 实现：`spawn(connection, baudrate, state, origin)` → 打开串口 + 起 RX 回调（`parse_one_frame` + `handle_message` + `try_write`）+ 起保持 loop；**spawn 内自动发 `send_heartbeat` → `request_telemetry_streams` → `set_mode_send(GUIDED)`**；实现 `MotionDevice`；实现一次性命令方法（takeoff/land）；实现坐标对齐（含 armed 跳变记录 `yaw_offset`）。

`spawn` 签名（供 bootstrap/launch 调用）：

```rust
pub fn spawn(
    port: &str,
    baudrate: u32,
    state: Arc<RwLock<Telemetry>>,
    origin: (f32, f32, f32),
) -> Result<Self, String>
```

### 步骤 6：`control/device/mod.rs` 注册

```rust
pub mod stm32;
pub mod lidar;
pub mod mavlink;   // 新增
```

### 步骤 7：config + bootstrap + Robot::launch 接入

1. `config.toml` 补 `[Robot.flight_ctrl]`（`enabled=false` 缺省 + `connection="/dev/ttyS0"` + `baudrate=921600` 注释）。
2. `bootstrap.rs::robot_bootstrap`：读 `flight_ctrl.enabled` / `connection` / `baudrate`（缺省 false），解析出 `Option<(port, baudrate)>`，作为参数传入 `Robot::launch`。
3. `Robot::launch`：加 `flight_ctrl_port: Option<&str>` / `flight_ctrl_baudrate: Option<u32>` 参数；若启用则 `MavlinkDevice::spawn(...)`，存入 `Robot` 的 `Option<Arc<MavlinkDevice>>` 字段。

### 步骤 8：ManualCmd 扩展 + dispatch 接入

1. `command.rs` 的 `ManualCmd` 加 `Takeoff` / `Land` 两个变体（`SetGuided` 不进命令，由 spawn 自动切）。
2. **设备抽象改造（前置）**：`apply_action` / `dispatch` 的通用动作参数由 `&STM32Device` 改为 `&dyn MotionDevice`（调 `move_forward/turn_left/stop` 等 trait 方法，车机统一）；车特有（`beep`）、雷达特有（`StartLidarScan`）、机特有（`takeoff/land`）各保留 `Option<具体设备>` 单独分支。
3. `dispatch` 加 `mavlink: Option<&MavlinkDevice>` 参数，新增分支：
   - `ManualCmd::Takeoff` → `mavlink.takeoff_send()`（纯触发，发 `NAV_TAKEOFF(1.8)` 即返回）
   - `ManualCmd::Land` → `mavlink.land_send()`（纯触发，发 `NAV_LAND` 即返回）

### 步骤 9：单测

复用 `spawn_mock` 模式（channel 模拟 TX/RX，参照 `stm32/mod.rs` 的 `spawn_mock`）：

- `test_mock_move_forward_updates_desired`：`move_forward` 后读期望动作 = `MoveForward`。
- `test_translation_table`：`action_to_velocity` 各分支返回正确四元组（前进 +0.3、后退 -0.3、转向 ±YAW_RATE、停 0）。
- `test_keepalive_loop_emits`：喂期望动作后，等待若干 tick，断言 TX channel 收到 `SET_POSITION_TARGET_LOCAL_NED` 字节。
- `test_handle_message_armed_flag`：喂 HEARTBEAT 帧，断言 `Telemetry.armed` 与 `yaw_offset` 记录正确。

---

## 六、验收标准

| 项 | 标准 |
|---|---|
| 编译 | `./build.sh` 通过（或 `cargo check -p Pleiades`） |
| 单测 | `cargo test` robot 相关测试通过 |
| 动作语义 | `move_forward/backward/turn_left/turn_right/stop` 经翻译表输出正确速度四元组 |
| 自闭环 | 期望动作置位后，保持 loop 以 10Hz 持续下发；`stop` 持续发零速度（悬停） |
| 一次性命令 | `takeoff_send` 发 `NAV_TAKEOFF(1.8)`、`land_send` 发 `NAV_LAND`、`set_mode_send` 发 `DO_SET_MODE(4)` |
| 坐标对齐 | 遥测写入 `RobotState` 前经 origin 平移 + yaw_offset 旋转；解锁跳变记录一次 offset |
| 无回归 | 车路径（STM32/Lidar/SLAM/寻路）行为不变 |

---

## 七、风险与注意事项

1. **运行方式（安全，强制）**：**必须 SSH 前台运行 Orion**，禁止 nohup/tmux/systemd 后台守护。后台运行时地面站断连 → Orion 仍发指令 → 飞控保护不触发 → 手动模式「一直向前」（详见设计文档 §6）。兜底：遥控器直连飞控可随时拨模式开关接管。
2. **期望动作的锁必须用 `std::sync::Mutex`**（trait 方法同步、不可 await；临界区纳秒级），不可用 tokio RwLock——与 task_22 全局状态统一用 tokio 锁是一个**有意的例外**。
3. **yaw 符号核对**：`turn_left` ↔ `yaw_rate` 正负号须按机体系 NED 实机核对（UAV `turn_degrees` 注释「正=顺时针/右转」），避免左右反转。
4. **保持 loop 发送失败要容忍**：`try_send` 失败仅 `warn` 跳过本帧，下一 tick（100ms）自然补发，不可 panic/break。
5. **起飞前需 GPS 3D fix**：GUIDED 硬依赖 `fix_type>=3`，takeoff 前须 `wait_gps_fix`（上层流程保证，代码不做兜底）。
6. **坐标系对齐的 origin 前提**：飞机须在 origin 点（默认 64,64）解锁，否则 `home` 偏移导致位置全偏。

---

## 八、范围边界

**本 task 做**：MavlinkDevice 设备模块 + config/bootstrap 接入 + `dispatch`/`apply_action` 走 `MotionDevice` 抽象 + ManualCmd 扩展（Takeoff/Land）+ 手动模式飞行。

**本 task 不做（后续 task）**：
- Auto 决策驱动飞机（机脚本 `plane.lua`，让 Lua 寻路决策驱动飞机；本 task 只做手动模式）；
- COMMAND_ACK 确认机制（决策 #7：第一版不做）；
- 手动命令看门狗（决策 #12：依赖 SSH 前台运行，暂不做）。
