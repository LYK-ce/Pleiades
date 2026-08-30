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

## C1-C5 建 crate —— 完成（2026-08-30）

### C0（前置，随 C1-C4 同步）

- **config 拆分**：base `Pleiades_Config`→`BaseConfig`（只留 Log/Network/Storage/Identity + node_type，删 `Robot` 段）；`ChassisConfig`/`LidarConfig` → `pleiades-ugv/src/config.rs`（`UgvConfig` = flatten base + chassis + lidar + obstacle_inflation_radius）；`FlightCtrlConfig` → `pleiades-uav/src/config.rs`（`UavConfig` = flatten base + flight_ctrl）；base 新增 `config_file_path()` 供设备端读同一 config.toml
- **robot_bootstrap 拆出**：base `bootstrap.rs` 删 `robot_bootstrap` + `CarDeviceHandler`/`UavDeviceHandler`/`DeviceHandler`/`Robot` import（反向依赖消除）；ugv/uav 各自 `bootstrap.rs` 写 `ugv_bootstrap`/`uav_bootstrap`（`CarDeviceHandler::new`/`UavDeviceHandler::new`）
- **依赖拆分**：`mavlink` → pleiades-uav；`serial2/serial2-tokio` 留 base（util/serial 在 base）
- **robot_cmd_frame_rx 未摘出（决策）**：`command_consumer`（消费方）与 `Robot::new` 均在 base，通道本就是 base 内部事务，非反向依赖；保留 `CoreBootstrap.robot_cmd_frame_rx`（Option，设备端 take），零改动
- **node_type 传播暂缓**：`ClusterInfo.node_type` 实施留后续（task §3.5「实施留后续」）；terminal GroundStation 身份同暂缓（terminal 不跑 robot_bootstrap，无消费方）

### C1 pleiades-base

- 迁入：config/network/ml_engine/peer_management/orchestrator/storage/session/bootstrap/event_bus/tui/cli/vm/api + robot/core + robot/util；`lib.rs` 改名 `pleiades_base`，`Robot/mod.rs` 只留 `core`+`util`；集成测试 `tests/` 迁入 `pleiades-base/tests/`（`pleiades::`→`pleiades_base::`，但 t09/t13 有既存 stale 引用 lua/GGUF_Analyze，与本次无关）
- `Pleiades` 纯推理 bin 并入 base（`src/main.rs`，`[[bin]] name="Pleiades"`）

### C2 pleiades-ugv

- `main_robot.rs`→`src/main.rs`（`parse_origin` + 测试原样迁移）；`robot/ugv/` 整目录迁入 `src/ugv/`；路径重写 `crate::robot::ugv::`→`crate::ugv::`、`crate::robot::core/util`→`pleiades_base::robot::...`、`crate::config/network/event_bus`→`pleiades_base::...`
- `robot_handler.rs` 改用 `UgvConfig`（`self.config.chassis/lidar/obstacle_inflation_radius`，替代 `self.config.Robot.*`）

### C3 pleiades-uav

- `robot/uav/` 迁入 `src/uav/`；`mavlink` 路径重写；`robot_handler.rs` 改用 `UavConfig`（`self.config.flight_ctrl`）；`on_tick` 仍 stub
- **复制 2D 决策**：`executor.rs`/`goal.rs`/`planning/` 从 ugv 复制到 uav（`crate::ugv::`→`crate::uav::`），uav/mod.rs 声明三模块（当前未接线，产生 dead_code 警告，预期）

### C4 pleiades-terminal

- `SrcPictorKernel/lib.rs`→`pleiades-terminal/src/lib.rs`（`pleiades::`→`pleiades_base::`），Cargo.toml crate-type=cdylib、依赖 pleiades-base

### C5 收尾

- workspace `Cargo.toml` members = 4 crate；`deploy_robot.sh` bin 名 `orion-robot`→`pleiades-ugv`；删死脚本 `programs/archived/car_lua_decision.lua` + 空目录 `programs/robot/`；清理过时 doc（core/mod.rs、Robot/mod.rs、core/state.rs「LidarState」、uav/mavlink/mod.rs「MotionDevice」）

### 验证

- `cargo build --workspace` ✅（Pleiades / pleiades-ugv / pleiades-uav / libpleiades_terminal.so + libpleiades_base.rlib 全产出）
- `cargo check --workspace` ✅ 0 error（base 24 警告 + ugv 42 + uav 34，主要为设备端死常量 + uav 复制的 2D 决策死代码，预期）
- Robot 测试：base robot 53 + ugv 81 + uav 45 = 179 全绿（uav 增量来自复制 2D 决策的测试副本）

### 遗留/待后续

- uav 2D 决策接线 + 3D 化（未来 task）
- `ClusterInfo.node_type` 传播（§3.5 实施留后续）
- 集成测试 t09/t13 stale 引用（lua→vm、GGUF_Analyze 改名，ML_review 范围）
- 既有 `vm::engine::test_sandbox_os_blocked` 沙箱测试失败（与本次无关）
