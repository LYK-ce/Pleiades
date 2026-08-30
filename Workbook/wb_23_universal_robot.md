# Workbook — Task 23: 机器人子系统重构（底座共享 + 设备端独有）

> 对应任务：`Task/task_23_universal_robot.md`
> 关联设计：`docs/heterogeneity_analysis.md`、`docs/plugin_design.md`
> 分支：`Universal-Robot`
> 创建日期：2026-08-30

---

## 阶段 A：目录重组 —— 完成（2026-08-30）

- **A1** `control/` 拆散：`device/{stm32,lidar}` + `types.rs` → `ugv/`，`device/mavlink` → `uav/mavlink`，`serial/` → `util/serial/`
- **A2** `world.rs` → `core/world.rs`，寻路（D* Lite）下沉到 `GoalService`（决策 D1：world 只留 `get_cell`/`get_agents`）
- **A3** `slam/` 拆分：`grid.rs` → `core/grid.rs`，`lidar_mapper/task/odometry` → `ugv/slam/`
- **A4** `executor.rs`/`goal.rs` → `ugv/`，急停 `check_emergency_stop` 拆出 → `ugv/emergency_stop.rs`
- **A5** `core/planning/` → `ugv/planning/`
- **A6** 删 `device.rs`（`MotionDevice` trait）+ 两个设备的 `impl MotionDevice`；命令/动作解析下沉设备（`STM32Device`/`MavlinkDevice` 各自 `handle_manual_cmd`/`apply_action`），主循环只做 `if stm32 / else mavlink` 路由
- 验证：`cargo check` 通过 + robot 133 测试

## 阶段 B：state 收敛 —— 完成（2026-08-30）

- **B1** `encoders[4]` 摘出：`RobotState` 删字段；`STM32Device` 内部 `Arc<Mutex<[i32;4]>>`；`update_state` 返回 `Option<[i32;4]>`，RX 回调写入设备内部
- **B2** `LidarState` 摘出：`state.rs` 删定义 + `LaserScan` use（反向依赖消除）；`LidarState` 移到 `lidar/mod.rs`，`LidarDevice` 内部维护 + `get_scan()`；急停改从 `LidarDevice.get_scan()` 读
- 删死脚本 `programs/user/robot_test.lua`（决策 D3）
- 验证：`cargo check` 通过

## 阶段 C 核心：反向依赖拆分 —— 完成（2026-08-30）

- **决策**：trait 方案（用户拍板）+ 统一 device `start()` 契约（用户追加）
- `core/robot.rs` 定义 `DeviceHandler` trait（`start`/`handle_manual_cmd`/`reset`/`on_tick`/`stop`/`shutdown`）
- `Robot::launch` 拆成 `Robot::new`（共享状态 + 非设备 task）+ `Robot::run`（main_loop 骨架，接收 `Arc<dyn DeviceHandler>`）
- `main_loop` 纯骨架：启动调 `device.start()`，命令/决策/急停/停车全走 device
- `CarDeviceHandler`（`ugv/robot_handler.rs`）：`start()` 里读 `[Robot.chassis]/[Robot.lidar]` + spawn stm32/lidar + goal_service + 决策器（`OnceLock` 幂等）
- `UavDeviceHandler`（`uav/robot_handler.rs`）：`start()` 里读 `[Robot.flight_ctrl]` + spawn mavlink
- `bootstrap` 拆分：`core_bootstrap`（共享）+ `robot_bootstrap`（按 `node_type` 分发到 Car/Uav handler），删 `assemble_car/uav`
- `MapDelta` 移回 `core/map_delta.rs`（共享协议类型，因 base 的 `map_tx` 通道用）
- config：加 `NodeType` 枚举 + `Identity_Config.node_type` + `Get_Node_Type` + 所有结构 `Clone`
- 验证：`cargo check` 0 error（28 warnings）+ robot 134 测试全绿；子 agent 审查 10 项全过

## 下一步（C1-C5 建 crate）

1. **C1** 抽 `pleiades-base` lib（共享底座 + 基础设施）
2. **C2** 抽 `pleiades-ugv` bin（车设备端）
3. **C3** 抽 `pleiades-uav` bin（机设备端）
4. **C4** 抽 `pleiades-terminal` cdylib（地面站，原 `SrcPictorKernel`）
5. **C5** 收尾（workspace `Cargo.toml` + 全量 build）

### C1 前收尾项

- 清理过时 doc 注释：`core/mod.rs`（「RobotState + LidarState」「goal.rs/executor.rs」「Robot::launch()」「planning/」）、`Robot/mod.rs`（「slam/ 拆到 ugv」）、`uav/mavlink/mod.rs`（「实现 MotionDevice」）
- C0 剩余：`robot_cmd_frame_rx` 从 `core_bootstrap` 摘出（§六 C0 第 4 步，仅车/机消费）
- `DecisionState`/`MotionAction`/`DecisionResult` 归属（C1 定：随 executor 留设备端或留 base）
- 全量 `cargo test --lib` 有 `vm::engine::test_sandbox_os_blocked` 预先存在失败（VM 沙箱，与本次无关）
