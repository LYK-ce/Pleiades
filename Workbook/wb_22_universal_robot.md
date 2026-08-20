# Workbook — Task 22: Robot 子系统通用化改造

> 对应任务：`Task/task_22_universal_robot.md`（+ 已取消的 `task_22_1_rwlock_refactor.md`）
> 关联设计：`docs/design_doc/universal_robot_design.md`
> 分支：`Universal-Robot`（稳定分支 `Pleiades-Orion` / `Godot-Library` 不动）
> 创建日期：2026-08-20

---

## 2026-08-20 文档闭环 + 开工

- 文档闭环（开工前置）：
  - 修 3 处硬冲突：`get_neighbors` 残留（design §9 #5 / task 涉及文件表）→ 统一「4 连通固定，不暴露」；caps 计数「8→11」；设计文档头部「讨论稿→方案已定稿」
  - 可选：§7.1 补命名空间说明（world 子模块标签 vs 扁平 self./world./action.）；robot_arch.md 头部加「task_22 前现状架构」注记
- 终检：子 agent 复核，文档闭环 ✅，代码基线 ✅（register_robot_caps 空 stub、无 MotionDevice/world/、无 z），GO
- 开始实施：2026-08-20

## 实施记录

### 数据层前置（z 维度 + vz 语义）—— 2026-08-20 完成

- **RobotState 加 `z: f32`**（`core/state.rs`）：垂直高度；写入者=设备自己（车恒 0，机写飞控 EKF）；odometry 不积分 z；`vz` 注释改「垂直速度 (m/s)」（原「角速度 rad/s」语义废弃）
- **PoseData 加 z（33→37 字节）**（`core/protocol/messages.rs`）：字段序 time/x/y/z/vx/vy/yaw/valid/sub_gx/sub_gy；encode/decode 同步 + 长度校验 37；测试 `test_pose_roundtrip` 加 z + 37B + 旧版 33B 拒绝；`test_frame_with_message` 补 z
- **ClusterInfo 加 z**（`core/cluster/cluster_info.rs`）+ **consumer 填 z**（`core/cluster/consumer.rs`：upsert z=pose.z + debug 日志补 z）
- **state_notifier 用真实 z**（`core/robot.rs`）：本地 `Pose.z = s.z`（原硬编码 0.0）+ `PoseData.z = s.z`
- **parse_origin 支持 [x y z]**（`main_robot.rs`）：返回 `(f32,f32,f32)`，z 可选默认 0，`DEFAULT_ORIGIN=(64,64,0)`，三参数测试
- **origin 三元组贯穿**：`bootstrap.rs::robot_bootstrap` / `robot.rs::Robot::launch`（g.z=origin.2）/ `stm32/mod.rs::spawn`+`spawn_mock`（local_state.z=origin.2）+ 测试
- **RPT_SPEED 第三槽**（`stm32/protocol.rs`）：加注释「=垂直速度（车恒 0）；原偏航角速度语义废弃」
- **验证**：`./build.sh check` 通过；`./build.sh test -p Pleiades --lib robot` 122 passed / 0 failed；`./build.sh test --bin orion-robot` 3 passed
- 注意：`cargo check` 需用 `./build.sh check`（nvcc 需 gcc-12，build.sh 注入 PATH）；之前 `cargo clean` 后直接 `cargo test` 曾撞 GCC13 报错

### 步骤 2：MotionDevice trait（纯重构）—— 完成

- 新增 `Src/Robot/device.rs`：`MotionDevice` trait（move_forward/backward/turn_left/right/stop/shutdown，start 不进 trait）
- `STM32Device` 实现 trait（move_forward→forward、turn_left→spin_left 等，内部仍 FUNC_CAR_RUN）
- executor 的 step/step_impl/step_idle/step_turning/step_moving 由 &STM32Device → &dyn MotionDevice

### 步骤 3：世界模块 —— 完成

- 新增 `Src/Robot/world.rs`：`World{grid, cluster, pathfinder: Mutex<Option<DStarLite>>}` + get_cell/get_agents/get_path/set_goal/clear_goal/mark_obstacle
- D* 从 executor 移入 World（std Mutex）；executor.query_next_sub_target 委托 world.get_path；step 变 async

### 步骤 4：启动流程改造 —— 完成

- config.rs：`Robot_Config` 改嵌套设备开关（chassis/lidar/flight_ctrl + enabled）+ ChassisConfig/LidarConfig/FlightCtrlConfig；config.toml/DEFAULT_CONFIG 同步
- bootstrap.rs：读嵌套配置 + 设备开关（chassis/lidar 缺省启用、flight_ctrl 缺省禁用），遍历传参
- robot.rs launch：chassis_enabled/lidar_enabled 开关；chassis 未启用报错
- SLAM 归雷达：slam_task + MapDelta + accumulate/drain_pending 迁 `slam/task.rs`，LidarDevice::spawn 打包 SlamContext 内部 spawn

### 步骤 5+6：Lua 决策层 + 车脚本迁移 —— 完成

- 新增 `core/goal.rs`：GoalService（完整目标服务 get_path = 到达检测 + 任务切换 + 寻路）
- capability_binding.rs：register_robot_caps 从空 stub 补全，注册 self/world/action 三表 caps（读方法 create_async_function + tokio 锁，发动作 create_function try_send）；新增 RobotCapsContext
- robot.rs：spawn_decision_thread（专用线程 + 独立 LuaContext + 加载 car.lua + 每 50ms tick 调 on_tick）+ check_emergency_stop（Rust 侧急停）；main_loop auto_tick 改「急停 → 调 Lua on_tick → 同步意图」
- 新增 `programs/robot/car.lua`：车决策脚本（get_path → 角偏差 → 转向/直行，无状态）
- 删除 executor.rs（Idle/Turning/Moving 状态机取消）+ 模块声明 + re-export
- branch_user.rs 移除旧 register_robot_caps 死代码调用

## 遗留

- **地面站（Pictor）同步**：`PoseData` 33→37 字节，Pictor 的 POSE 解析需同步加 z（外部 Godot 仓库）
- **车行为一致性验证**：步骤 6 迁移后需实车验证「走格子/转向/直行/到达/急停」与稳定分支一致（Lua 无状态决策 vs 原状态机，行为等价性需实车联调确认）
- **Lua 决策脚本路径硬编码**：`programs/robot/car.lua` 相对路径，后续可进 config
- 后续 task：机脚本 + MavlinkDevice（步骤 7，等机硬件）
- 结束时间：2026-08-20
