# Task 6: SLAM — 里程计定位

> 状态：设计完成
> 创建日期：2026-07-22

## 目标

为 SLAM 增加定位能力：编码器 + IMU 里程计，替代当前硬编码 `(x=64, y=64)`。

当前状态：建图 ✅（OccupancyGrid + Bresenham），定位 ❌（位姿硬编码）。

## 背景

### 可用传感器

| 数据 | 来源（STM32） | 频率 | 含义 |
|------|:---:|:---:|------|
| `vx` | `RPT_SPEED` | ~10Hz | 小车前进方向线速度 (m/s) |
| `attitude.yaw` | `RPT_IMU_ATT` | ~10Hz | IMU AHRS 解算的绝对航向角 (rad) |
| LiDAR scan | `LidarState` | 7-10Hz | 360° 点云 |

## 设计

### 核心思路

里程计位移累积放在 `STM32Device` 的 **RX 回调**里做——与 `update_state()` 同一处，不新增 task、不新增锁。

```
RX 回调（当前唯一的 RobotState 写入者）:
  feed_state_machine → 帧完整了
    update_state(local_state, func, data)   ← 写 vx, yaw 等
    如果是 RPT_SPEED:
      dt = now - last_speed_instant
      local_state.odom_x += vx × dt × cos(yaw)   ← 🆕
      local_state.odom_y += vx × dt × sin(yaw)   ← 🆕
      last_speed_instant = now
  try_write → 发布到共享 RobotState（含 odom_x/y）
```

用 IMU yaw 直接做航向（不由 vz 积分，磁力计可修正）。零标定——STM32 的 `vx` 已经是 m/s。

### 开销

每帧：一次 `dt` 减法 + 一次乘法 + `cos`/`sin`（~30ns 各）→ 总计 < 100ns。10Hz 频率，每秒 <1μs。无新增锁。

## 实现步骤

### 文件改动

| 文件 | 改动 |
|------|------|
| `Src/Robot/state.rs` | `RobotState` 新增 `odom_x`, `odom_y` 字段 + `accumulate_odom(dt)` 方法 |
| `Src/Robot/control/device/stm32/mod.rs` | RX 回调中：RPT_SPEED 后调 `accumulate_odom()`；新增 `last_speed_ts`；mock 同步 |
| `Src/Robot/core/robot.rs` | `state_notifier`: `x/y = 64.0 + odom_x/y`；`slam_task`: `RobotPose` 读 odom |

### RobotState 新增

```rust
pub struct RobotState {
    // ... 现有字段 ...
    /// 里程计累积位移 (m)，初始 (0,0)
    pub odom_x: f32,
    pub odom_y: f32,
}

impl RobotState {
    /// 基于当前 vx + yaw 累积里程计位移
    pub fn accumulate_odom(&mut self, dt: f32) {
        let yaw = self.attitude.yaw;
        self.odom_x += self.vx * dt * yaw.cos();
        self.odom_y += self.vx * dt * yaw.sin();
    }
}
```

### state_notifier / slam_task 改动

```rust
// state_notifier: 64.0 + odom → Pose
x: 64.0 + s.odom_x, y: 64.0 + s.odom_y

// slam_task: 读取 odom → RobotPose
let pose = RobotPose {
    x: 64.0 + rs.odom_x,
    y: 64.0 + rs.odom_y,
    yaw: rs.attitude.yaw,
};
```

## 已知限制

- `vx` 带噪声，累积位移有小幅漂移
- 轮子打滑时 `vx` ≠ 实际位移（后续 LiDAR 扫描匹配可解决）
- 不维护协方差（不需要 EKF）
- 第一次 SPEED 帧 dt 极短，odom 近零，等价于从 (0,0) 起步

## 人类评审

<!-- 在此区域写下评审意见 -->

