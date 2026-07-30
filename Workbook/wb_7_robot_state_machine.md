# Workbook — Task 7: Robot State Machine

> 对应任务：`Task/task_7_robot_state_machine.md`
> 创建日期：2026-07-28

---

## 设计讨论（2026-07-28）

### 底层同步化

STM32Device 的 `forward()`/`stop()` 等方法标注 `async`，实际只是 `mpsc::Sender::send().await`——串口写入在独立 TX task 中完成，这些方法仅做队列 push。

**决策**：全部改为同步（`try_send`），dispatch 同步化。

理由：去假 async，select! 中不阻塞，executor 发命令走同一路径。

注意：`try_send` 失败意味着通道满（串口物理故障），应打 error。

### auto_tick 集成

- executor.step() 通过 `cmd_tx` 发命令（不直接调 STM32）
- auto_timer 用 `sleep_until` 替代 `interval`，随模式生灭

### 已决策

| 项 | 决策 |
|------|------|
| yaw 归一化 | `(delta + π).rem_euclid(2π) - π` |
| 实时急停 | executor.step() 首步读 LiDAR 前方 < 0.3m → Stop + 通知 D* |
| Idle 重入保护 | executor 内 `stepping` 标志位 |
| Turning→Moving | stop → 下 tick 确认角速度 < 阈值 → forward |

### 已决策（全部完成）

| 项 | 决策 |
|------|------|
| 卡住检测 | 不做 |
| Pause 模式 | 去掉 |
| 不合法命令 | 静默丢弃 + warn 日志 |
| D* 失败 | error 日志 + sub_target 不更新 + executor 停 Idle |
| 角度过冲 | 单一阈值 5°，下 tick 自然修正 |

---

## 实现步骤（规划）

1. `STM32Device` 方法去 async（`send` → `try_send`）
2. `dispatch` 去 async
3. 新建 `core/mode.rs` — `OpMode` 枚举
4. 新建 `core/mission.rs` — `MissionQueue`
5. 重写 `core/command.rs` — 三层 `Command`
6. 重写 `core/robot.rs` — 主循环 + auto_tick + Executor
7. 新建 `core/executor.rs` — `Executor` + Idle/Turning/Moving
8. 新建 `slam/pathfinder.rs` — D* Lite
9. 更新 `WebSocket/protocol.rs` — 新 JSON 协议
10. 更新 `Config` — `RobotConfig`
