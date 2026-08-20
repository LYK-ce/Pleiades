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

## 遗留

- **地面站（Pictor）同步**：`PoseData` 33→37 字节，Pictor 的 POSE 解析需同步加 z（外部 Godot 仓库）
- 后续步骤：2 MotionDevice trait → 3 世界模块 → 4 启动流程 → 5 Lua 决策层 → 6 车脚本迁移
