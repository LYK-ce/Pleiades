# task_22_5_lua_decision_layer — Lua 决策层定位澄清与决策层 Rust 化（方案二）

> 状态：**讨论中**（2026-08-25 起，边讨论边填充，未实施）
> Created Date ： 2026-08-25
> Modified Date ： 2026-08-25
> 依赖：`Task/task_22_universal_robot.md`（通用机器人）、`Task/task_22_3_single_writer.md`（单写者收敛）、`Task/task_22_4_mavlink_device.md`（飞控设备）
> 分支：`Pleiades-Orion`

---

## 一、目标

把 **Lua 决策层（`car.lua` + `register_robot_caps` + `DecisionResult`）的定位**彻底搞清、讲明白，并据此确定改造方向：**决策层回退 Rust（方案二）**。

**本 task 不做**（范围外，明确排除）：
- ❌ 代码合理性重构 / 性能优化（锁粒度、双写者等遗留问题另议）
- ❌ 动作机加高度 / 起降 / 垂直维度（无人机定高后只做 XY 平面移动，见 D1）

**本 task 要做**：
- ✅ 澄清 Lua 的定位：Lua = 纯决策（给「意图」），速度由设备层绑定
- ✅ 决策层回退 Rust（方案二）：以 `executor.rs` 三状态机为蓝本替换 Lua 决策链路
- ✅ 动作参数化去除（`MotionAction` 去掉 `i16`，纯意图），连带 STM32 速度常量化改造
- ✅ 明确无人机的避障策略（不做避障）

---

## 二、背景：当前 Lua 的实际定位

当前 Lua 决策层是一个**「2D 地面车的路径跟踪微执行器」**，不是「通用设备的决策层」：

| 层 | 现状 | 代码位置 |
|---|---|---|
| 决策输入 | `DecisionReq = (generation, 下一格 (gx,gy))`，**纯 2D 网格，无 z** | `Src/Robot/core/robot.rs` L56 |
| 动作集 | `MotionAction { MoveForward(i16), MoveBackward(i16), TurnLeft(i16), TurnRight(i16), Stop }`，**无高度/起降** | `Src/Robot/core/state.rs` L113-119 |
| 状态机 | `DecisionState { Idle, Turning, Moving }`，其中 `Turning`=原地转向对齐（车语义） | `Src/Robot/core/state.rs` L104-109 |
| 脚本加载 | **硬编码** `read_to_string("programs/robot/car.lua")`，车机共用同一脚本 | `Src/Robot/core/robot.rs` L699 |
| caps 接口 | `self`（get_state/get_position/get_attitude/get_velocity）+ `world`（get_cell/get_agents），**不暴露设备类型/能力** | `Src/VM/capability_binding.rs` L767-881 |
| 急停 | LiDAR 前方 ±45° < 0.3m 触发（车避障语义） | `Src/Robot/core/robot.rs` L627-665 |

无人机（`MavlinkDevice`）接入后暴露的错位：
1. `action_to_velocity` **忽略 Lua 传入的 `i16` 速度参数**，写死 `VEL_FWD=0.3`、`YAW_RATE_DEG=15°/s` → Lua 的 `TURN_SPEED/MOVE_SPEED/arg` 对机无效（抽象错位）。
2. `Takeoff/Land` 只能经手动命令触发，**不在 Lua 决策循环**（`dispatch` 的 `ManualCmd::Takeoff/Land` 分支）。
3. 急停逻辑是车避障（LiDAR），无人机没有对应物。

---

## 三、已确定决策（讨论共识，2026-08-25）

> 以下为与人类讨论后**已确认**的结论，实施时严格遵循。

### D1. 垂直维度不在范围
无人机起飞到 **2 米定高**后，**只做 XY 平面移动**。因此：
- `MotionAction` / `MotionDevice` **不增加**高度、爬升/下降、悬停、起降动作。
- `Takeoff/Land` 维持现状（手动命令触发），不进 Lua 决策循环。
- 决策输入维持 2D（`next_cell = (gx,gy)`）。

### D2. 速度由设备层绑定（决策器只给意图）
- 决策器只给**动作意图**（前进/后退/左转/右转/停），**不传速度值**。
- 具体速度由 **Rust 设备层**决定（安全优先，车机速度都不设快）。
- → **`MotionAction` 的 `i16` 参数去掉**，动作变纯意图枚举。

### D3. 无人机不做避障
- 定高 2 米、环境可控、周围无物，无人机**不接 LiDAR、不做急停避障**。
- 现状天然满足：`check_emergency_stop` 首行 `let Some(scan) = lidar_state.scan else { return false }`，无 LiDAR 即无 scan、恒返回 false。**不改代码、不加开关**（Q1 已定）。

### D6. Gossipsub 订阅按 topic 配置化
- 每个 topic 在 `[Network]` 段提供 enable 开关（true 订阅 / false 不订阅），默认全 true（不破坏现状）。
- 5 个 topic：`peer_info` / `models` / `sessions` / `robot_pose` / `robot_map`。
- 无人机设 `subscribe_robot_pose=false` + `subscribe_robot_map=false` → `cluster_consumer` 收不到 → 不合并地图 / 他车不转障碍 → D* 在全空地图走曼哈顿最短。
- 实现落点：`config.rs` `Network_Config` + `network_service.rs` `NetworkConfig` 各加 5 字段，`Start()` 条件订阅。

---

## 四、待讨论 / 待定问题

- **Q1（避障）**：✅ 已定 —— **暂不显式化、不改代码**，维持「有 LiDAR 才避障」隐式约定。飞机不接 LiDAR → `lidar_state.scan` 恒 `None` → `check_emergency_stop` 直接返回 false，天然不避障。无需新增开关或分支。
- **Q2（速度常量值）**：✅ 已定 —— 速度值**全部不动**，仅从决策层挪到设备层作为常量：车保持档位制 `FORWARD=30` / `TURN=10`，机保持 `VEL_FWD=0.3` / `YAW_RATE_DEG=15`。数值照搬，仅归属变更（决策层 → device）。
- **Q3**：（待补充）

---

## 五、开放讨论（未定论）

> 以下为讨论中提出的方向性议题，**尚未拍板**，供后续继续讨论。

### T1. Lua 到底留不留 / 引进方式

- **背景判断（人类）**：未来车/机行为模式**肯定会变**，最终仍会引入 Lua；但「引进方式」尚未想清。
- **历史经验**：曾试过「用 Lua 代替 Rust 做具体操作/行为操作」，发现不行（→ task_22_3 单写者收敛，Lua 退化为纯决策）。
- **当前疑问（人类）**：① 是否仍沿用 Rust 做操作为佳？② 当前 Rust 够用，是否直接就用 Rust？
- **倾向共识（待确认）**：
  - 「操作/副作用」永远留在 Rust（main_loop + MotionDevice），Lua 不再碰命令/状态——此边界 task_22_3 已划死，不回退。
  - Lua 的价值只在「决策策略」层面，不在「操作」层面。
  - 建议采用**策略模式**：将「决策」抽象为 trait（如 `DecisionPolicy`），当前给 Rust 实现（把 car.lua 逻辑写成 Rust），未来需要时加 Lua 实现（读脚本 + caps + on_tick），config 加 `decision_policy = "rust" | "lua"` 切换。这样「当前用 Rust」与「未来引 Lua」不冲突。


### T2. Godot-Library 的 Rust 决策实现（发现 + 恢复方案对比）

- **查证**：`Godot-Library` 分支决策为纯 Rust——`Src/Robot/core/executor.rs` 三状态机（Idle→Turning→Moving），与当前 `car.lua` **逻辑完全等价**（参数一致：turn_speed=10 / move_speed=30 / turn_align=5° / straight_align=10° / sub_target=0.2m / obstacle=0.3m）。该分支**无** `MotionDevice` trait、`world.rs`、`mavlink` 飞控，且 `executor` 硬绑 `&STM32Device`。
- **两分支差异分类**：
  - **设备抽象层**（与决策语言无关，task_22 成果）：`device.rs`(MotionDevice) / `world.rs` / `slam/task.rs` / `goal.rs` / `mavlink/`(task_22_4，**未提交**、仅在工作区) / z 维度数据层。
  - **决策层**：删 `executor.rs`(397 行 Rust) ↔ 增 `car.lua`(48 行) + `register_robot_caps`(+172 行) + 决策线程/代际/看门狗。
- **两个恢复方案对比**：
  - **方案一（回退 Godot-Library 再改）**：回退会丢掉设备抽象层全部成果（MotionDevice/World/mavlink/z），需重做 task_22_universal_robot + task_22_4 的大部分。优点：executor.rs 是经过测试的基线、无 Lua 债。缺点：巨大重复劳动 + 回归风险。
  - **方案二（从当前改回 Rust 决策）**：保留设备抽象层，仅把决策层 Lua→Rust（参考 executor.rs，`&STM32Device`→`&dyn MotionDevice`）。优点：改动小、聚焦，同样清掉 Lua 债。缺点：需自证 Rust 决策正确（有 executor.rs 现成参考、逻辑等价，风险低）。
- **倾向结论（待确认）**：**方案二明显更优**。「回退保正确性」不成立——决策逻辑两分支本就等价，真正要保的设备抽象层成果反而会被回退丢掉。


### T3. 方案二细化改造清单（草案，待确认）

> 前提：采纳「从当前分支改回 Rust 决策」，保留设备抽象层。

1. **新增 Rust 决策器**：以 Godot-Library `executor.rs` 三状态机为蓝本，改造为基于 `&dyn MotionDevice`：
   - `&STM32Device` → `&dyn MotionDevice`；`stm32.forward/spin_left/...` → `motion.move_forward/turn_left/...`
   - 速度绑设备层（D2）：决策器不再传速度，动作纯意图
2. **替换决策链路**（`robot.rs`）：删 `spawn_decision_thread`、`decision_tick_tx/result_rx`、generation 代际、看门狗；main_loop 直接调 Rust 决策器；`invalidate` 去掉换代/drain
3. **清理 Lua 绑定**（`capability_binding.rs`）：删 `register_robot_caps` / `RobotCapsContext` / `DecisionResult::from_lua` 等（约 172 行）
4. **删脚本**：`programs/robot/car.lua`（归档）
5. **数据结构**（`state.rs`）：`MotionAction` 去 `i16`（D2）；`DecisionResult`/决策线程相关类型可简化
6. **保留不动**：`MotionDevice` / `world.rs` / `goal.rs` / `slam/task.rs` / `mavlink` / z 维度 / cluster / planning(D*)
7. **测试**：编译 + 现有 robot 测试调整 + 补 Rust 决策器单测

### T4. 无人机「无需地图/寻路」观察（待讨论）

- **观察（人类）**：无人机飞空中默认无障碍，**无需订阅 `TOPIC_ROBOT_MAP`**、无需地图合并、无需 D* 寻路。
- **现状**：`network_service.rs` `Start()` 无条件订阅 5 topic（含 `TOPIC_ROBOT_POSE`/`TOPIC_ROBOT_MAP`）；`cluster_consumer` 消费 POSE(他车表)/MAP_DELTA(合并地图)。
- **牵连三层**：① 网络订阅层（topic 列表写死，需设备类型）② Robot 装配层（无人机不应 spawn grid/cluster_consumer/slam_task/D*）③ 决策层（车走格点绕障、机走直线航点——属决策层行为差异的实例）。
- **待定**：是否将「导航方式（格点寻路 vs 直线航点）」纳入设备能力/决策策略差异。

- **复用方案（人类，待确认）**：无人机**不订阅 `TOPIC_ROBOT_MAP`** → 自地图全 Unknown → D* 把 Unknown 当 Free（`pathfinder.rs` cost：Free/Unknown=1，Occupied=∞）→ 无障 → 走最短路径，D*/goal/world/决策逻辑全部复用，只改订阅。
- **两处精确化**：① D* `neighbors` 仅 4 连通（无对角）→ 最短路径为**曼哈顿阶梯**，非对角线直线；② 仅不订阅 MAP 不够——`cluster_consumer` 仍会把 POSE 写 `ClusterInfoTable` → `cluster_to_obstacle_cells` 转成 D* 动态障碍（他车会被当障碍绕开）。
- **完整最小改动**（三者同开）：① 不订阅 `TOPIC_ROBOT_MAP`；② 不订阅 `TOPIC_ROBOT_POSE`（或不转障碍）；③ 不接 LiDAR。三者均需「设备类型/能力」开关区分车/机（现 `network_service.rs` topic 列表写死）。

---

## 六、记录

- 2026-08-25：与人类讨论，确定 D1~D6 + Q1~Q3，拍板方案二（改回 Rust 决策、保留设备抽象）。D4/D5（决策脚本配置化/脚本目录）因改回 Rust 决策已移除，未来引 Lua 时再议。
