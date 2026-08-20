# task_22_universal_robot — Robot 子系统通用化改造

> 状态：方案已确认（2026-08-20），**尚未开始实施**
> Created Date ： 2026-08-20
> Modified Date ： 2026-08-20
> 关联设计文档：`docs/design_doc/universal_robot_design.md`
> 分支：`Universal-Robot`（基于 `Godot-Library` 创建，稳定分支 `Pleiades-Orion` / `Godot-Library` 不动）

---

## 一、目标

将 `Src/Robot/` 从「2D 轮式车专用」改造成「车/机通用的插件化架构」，使设备（车、四旋翼、未来的船等）可像插件一样插入，核心逻辑稳定不变。

核心架构（详见设计文档）：

- **三层**：Rust 核心层（世界地图/设备抽象/主循环/集群/协议）· Lua 决策层（每设备一个脚本）· 终端层（底盘/飞控自己闭环）
- **三层统一**：世界地图、导航模型（走格子）、动作语义（前进/后退/左转/右转/停）车机一致；**唯一不同 = 底层协议编解码**（`FUNC_CAR_RUN` vs MAVLink）
- **插件机制**：config 里每个设备一个 `enabled` 开关，启动时遍历 spawn；设备 = 自包含能力单元，`start()` 内部打包 tokio 组
- **Lua 决策**：Rust 50ms 定时器驱动无状态决策（读世界 → 决策 → 发统一动作）

## 二、范围

### 本次要做

1. ✅ **已定稿**：caps 接口集（设计文档 §7，3 模块 11 方法）
2. 设备抽象 trait（`MotionDevice`），STM32/Lidar 改造为实现
3. 世界模块（三子模块：static `get_cell` / dynamic `get_agents` / pathfinding `get_path`）
4. 启动流程改造（config 开关 + 遍历 spawn + SLAM 归入雷达设备）
5. Lua 决策层（`register_robot_caps` 补全 + 决策脚本框架 + main_loop 接 Lua）
6. 车脚本迁移（executor 车决策 → Lua 脚本），**验证车行为不变**

### 本次不做

- 机脚本 / `MavlinkDevice`（等机硬件，后续 task）
- 真 3D 寻路（先固定 2D 格子 + 高度独立通道）
- 运行时 `.so` 动态插件（编译期 + config 足够）
- 设备依赖的代码校验（配置者自行保证）

## 三、涉及文件

### 3.1 主要改动

| 文件 | 改动 |
|---|---|
| `Src/Robot/core/executor.rs` | 拆「通用目标跟踪」与「车动作翻译」；车动作翻译下沉到设备/Lua |
| `Src/Robot/core/robot.rs` | `launch` 遍历开关 spawn；`main_loop` auto_tick 接 Lua；`dispatch` 走设备统一接口 |
| `Src/Robot/control/device/stm32/mod.rs` | `STM32Device` 实现 `MotionDevice` trait |
| `Src/Robot/control/device/lidar/` | `LidarDevice` 实现 trait；**打包 SLAM 建图 tokio**（`start()` 内 spawn 读数据 + 建图） |
| `Src/Robot/slam/grid.rs` | 归入世界模块（数据结构保留，接口按世界模块暴露） |
| `Src/Robot/slam/lidar_mapper.rs` | 归入雷达设备（与 `control/device/lidar` 合并成感知单元） |
| `Src/Robot/slam/odometry.rs` | 归入底盘/车设备侧 |
| `Src/Robot/core/state.rs` | `RobotState` 加 `z: f32`（z 写入者 = 设备自己：车写 0，机写飞控） |
| `Src/Robot/core/protocol/messages.rs` | `PoseData` 加 z（33→37 字节，同步地面站） |
| `Src/Robot/core/cluster/cluster_info.rs` | `ClusterInfo` 加 z |
| `Src/Robot/core/cluster/consumer.rs` | 解析 PoseData 填 z |
| `Src/main_robot.rs` | `parse_origin` 支持 `[x y z]`（z 可选，默认 0） |
| `Src/Robot/core/planning/pathfinder.rs` | 世界模块 `get_path`（4 连通固定，不暴露 `get_neighbors`） |
| `Src/Robot/core/command.rs` | 命令体系保留（`ManualCmd` 动作集即车机统一动作语义） |
| `Src/Config/config.rs` | `[Robot]` 段改为设备开关（`[Robot.devices.*] enabled=true/false`） |
| `Src/bootstrap.rs` | `robot_bootstrap` 遍历 config 开关 spawn 设备 |
| `Src/VM/capability_binding.rs` | `register_robot_caps` 从空 stub 补全 caps 接口 |
| `programs/robot/` | 新增车决策 Lua 脚本（启动时 robot 主循环主动加载，独立 LuaContext） |

### 3.2 新增

| 路径 | 内容 |
|---|---|
| `Src/Robot/device/`（或类似） | `MotionDevice` trait 定义 + 设备注册表 |
| `Src/Robot/world/`（或类似） | 世界模块（static 地图 / dynamic 集群 / pathfinding 寻路，`get_cell`/`get_agents`/`get_path`） |
| `docs/design_doc/universal_robot_design.md` | 设计文档（已创建） |

### 3.3 不动

- `Src/Robot/core/mission.rs` / `mode.rs`（任务队列/运行模式，通用）
- `Src/Robot/core/command_consumer.rs`（命令入站，通用）

> 注：`cluster/` 和 `protocol/` 骨架通用，但本次需小改加 z（`ClusterInfo` / `PoseData`），不列入「完全不动」。

## 四、详细实施步骤

### 步骤 1：定稿 caps 接口集（设计，无代码改动）

caps 接口集**已定稿**（设计文档 §7）：

- **self**：`get_position()` → (x,y,z) / `get_attitude()` → (roll,pitch,yaw) / `get_velocity()` → (vx,vy,vz)
- **world.static**：`get_cell(x,y)` → Free/Occupied/Unknown（不含动态障碍）
- **world.dynamic**：`get_agents()` → 其他设备位置/速度
- **world.pathfinding**：`get_path()` → **完整目标服务**（内部：到达检测 + 任务切换 + 寻路）
- **action**：`move_forward/backward` / `turn_left/right` / `stop`

**签名约定**：读方法用 `create_async_function` + tokio 锁 `.read().await`（保持 tokio 锁，不换 std）；发动作用 `create_function`（同步 `try_send`）。

### 步骤 2：设备抽象 trait（纯重构，行为不变）

1. 定义 `MotionDevice` trait（**`start` 不进 trait**，各自构造）：
   - 统一动作：`move_forward / move_backward / turn_left / turn_right / stop`
   - 生命周期：`shutdown()`
   - `start()` 各自构造：参数是各自的 config 类型（`Stm32Device::start(&ChassisConfig)`），返回 `Box<dyn MotionDevice>`
2. `STM32Device` 实现 `MotionDevice`（内部继续走 `FUNC_CAR_RUN`，行为不变）
3. `LidarDevice` 实现 trait（感知单元）
4. `executor.rs` / `robot.rs` 中 `stm32: STM32Device` 换成 `Box<dyn MotionDevice>`
5. **验证**：`cargo check` 通过 + 车行为不变

### 步骤 3：世界模块

1. 抽出世界模块（大模块 + 三子模块）：`static`（grid）/ `dynamic`（cluster_table 归入）/ `pathfinding`（D*）
2. 暴露 `get_cell(x,y)` / `get_agents()` / `get_path()`
3. `get_path` = **完整目标服务**（内部封装：到达检测 + 任务切换 pop/群发分配/设 goal + D* 寻路），D* 加 `std::sync::Mutex<Option<DStarLite>>`
4. 邻居 4 连通固定（真 3D 的 26 连通留后续，不暴露 get_neighbors）
5. **验证**：`cargo check` + 现有寻路测试通过

### 步骤 4：启动流程改造（config 开关 + 遍历 spawn）

1. `config.rs` 的 `[Robot]` 段改为设备开关列表（`chassis.enabled` / `lidar.enabled` / `flight_ctrl.enabled`）
2. `bootstrap.rs` 的 `robot_bootstrap` 遍历 config，spawn `enabled=true` 的设备
3. SLAM 建图 task 从 `robot.rs` 的独立 `slam_task` 移入雷达设备 `start()` 内部
4. **验证**：`cargo check` + 车启动正常（STM32 + 雷达照常 spawn）

### 步骤 5：Lua 决策层

1. `register_robot_caps` 从空 stub 补全：注册 self/world/action 的 caps 到 Lua（读方法 `create_async_function` + tokio 锁 `.read().await`；发动作 `create_function`）
2. 脚本生命周期：`programs/robot/` 目录 + 独立 `LuaContext`（与推理脚本隔离）+ robot 主循环启动时主动加载
3. Lua 决策脚本框架：`on_tick()` 入口 + 无状态决策（Rust 50ms 定时器驱动，砍掉 Idle/Turning/Moving 状态机）
4. `main_loop` 的 auto_tick 分支：`executor.step(...)` 改为「检查急停 → 调 Lua on_tick → Lua 决策 → 下发动作」
5. **验证**：Lua 能读到状态、能下发动作（先用手动测试脚本验证 caps 通）

### 步骤 6：车脚本迁移 + 验证

1. 写车决策 Lua 脚本：读位置 + `get_path` 拿下一格 → 算角偏差 → `turn_left/turn_right` / `move_forward`
2. 把 `executor.rs` 的转向/直行决策逻辑迁到 Lua 脚本
3. **验证（关键）**：车在 `Universal-Robot` 分支上行为与稳定分支完全一致（走格子、转向、直行、到达、急停）

### 步骤 7：机脚本（后续，不在本次）

等机硬件到位后，写机脚本 + `MavlinkDevice`，走同一套 caps。

## 五、验证方式

| 阶段 | 验证 |
|---|---|
| 步骤 2-4（纯重构） | `cargo check` 通过 + 单元测试通过 + 车行为不变 |
| 步骤 5-6（Lua 迁移） | `cargo check` + Lua 脚本跑通 + 车行为与迁移前一致 |
| 全程 | 不破坏 `Pleiades-Orion` / `Godot-Library` 稳定分支 |

## 六、风险与注意

1. **行为一致性**：步骤 6 迁移车决策到 Lua 时，务必保证「走格子、转向、直行、到达、急停」行为与现状完全一致（这是本次改造的安全底线）
2. **`vz` 语义**：车里 `vz` = 偏航角速度（rad/s，死数据无人消费），机里 `vz` = 垂直速度。改造时**直接删除角速度语义、`vz` 回归垂直速度**（车恒 0，机写飞控垂速）；**不需要 yaw_rate**（yaw 直接来自 IMU，不积分角速度）
3. **`set_motion`/`FUNC_MOTION` 死代码**：车底盘的速度矢量接口当前未被使用，本次改造**不启用**（车维持离散动作 `FUNC_CAR_RUN`）
4. **每次改动小步提交**：每个步骤单独 commit，便于回滚
5. **锁决策**：保持 `tokio::sync::RwLock` + caps 异步函数，**不换 std 锁**（`task_22_1_rwlock_refactor` 已取消——换 std 锁收益纳秒级、代价几十处改造，不对等）
