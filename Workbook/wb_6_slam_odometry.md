# Workbook — Task 6: SLAM 里程计定位

> 对应任务：`Task/task_6_slam_odometry.md`
> 创建日期：2026-07-22

---

## 实现完成（2026-07-22）

### 设计决策

- **位置累积放在 RX 回调中**（`STM32Device::spawn` 闭包内），不新增 task/锁
- 收到 `RPT_SPEED` 帧后，`dt = now - last_speed_ts` → `local_state.accumulate_odom(dt)`
- 直接用 STM32 的 `vx` (m/s) + IMU `yaw`，零标定
- dt 范围过滤：`0.0 < dt < 1.0`（过滤首帧/异常）

### 改动文件

| 文件 | 改动 |
|------|------|
| `state.rs` | +`odom_x: f32`, +`odom_y: f32`, +`accumulate_odom(dt)` |
| `stm32/mod.rs` | spawn: +`last_speed_ts`, RPT_SPEED 后累积; mock 同步 |
| `robot.rs` | state_notifier: `x/y = 64.0 + odom`; slam_task: `RobotPose` 读 odom |

### 数据流

```
STM32 串口 → RX callback
  RPT_IMU_ATT → local_state.yaw = ...
  RPT_SPEED → local_state.vx = ...
    → dt = now - last_speed_ts
    → odom_x += vx * dt * cos(yaw)
    → odom_y += vx * dt * sin(yaw)
    → try_write → RobotState (含 odom_x/y)

state_notifier (100ms): Pose { x: 64.0 + odom_x, y: 64.0 + odom_y }
slam_task (200ms): RobotPose { x: 64.0 + odom_x, y: 64.0 + odom_y }
```

### 测试

- 30/30 passed（包括 mock 集成测试）
- mock: `test_mock_rx_state_update` 验证 SPEED 帧解析不破坏（odom dt 极短 ≈ 0，不影响）
