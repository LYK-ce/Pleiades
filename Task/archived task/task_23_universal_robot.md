# task_23_universal_robot — 机器人子系统重构（底座共享 + 设备端独有）

> 状态：✅ 全部完成（阶段 A/B/C 核心 + C1-C5 建 crate + 附加工作）
> Created Date ： 2026-08-29
> Modified Date ： 2026-08-31
> 关联文档：`docs/heterogeneity_analysis.md`（异构分析）、`docs/plugin_design.md`（插件化方案）
> 取代：`task_22_universal_robot`（已废弃，见归档目录）

---

## 当前进度（2026-08-31）

### 已完成

| 阶段 | 内容 | 验证 |
|---|---|---|
| 阶段 A | 目录重组：`control/`→`ugv/`+`uav/`+`util/`、`world.rs`→`core/world.rs`（寻路下沉 GoalService）、`slam/` 拆分（grid→core）、`executor/goal`→`ugv/`、急停拆出、`planning`→`ugv/planning`、删 `MotionDevice`/`device.rs`，命令/动作解析下沉设备 | `cargo check` + robot 133 测试 |
| 阶段 B | state 收敛：`encoders[4]`→STM32 内部、`LidarState`→LidarDevice 内部（`get_scan()`） | `cargo check` |
| 阶段 C 核心 | 反向依赖拆分：`DeviceHandler` trait + 统一 `start()` 契约 + `robot.rs` 不再依赖设备端具体类型；`bootstrap` 按 `node_type` 分发；`MapDelta` 回 base；config 加 `NodeType`/`node_type`/`Clone` | `cargo check` 0 error + robot 134 测试 + 子 agent 审查 10 项全过 |
| C0 | config 拆分：`BaseConfig`/`UgvConfig`/`UavConfig` 经 `#[serde(flatten)]` 复用共享段；`robot_bootstrap` 拆出到车/机各自装配；`mavlink` 依赖移 uav；`robot_cmd_frame_rx` 保留 base（非反向依赖） | `cargo check` |
| C1 | 抽 `pleiades-base` lib（共享底座 + 基础设施 + robot/core + robot/util）；`Pleiades` 纯推理 bin 并入 base | `cargo build -p pleiades-base` |
| C2 | 抽 `pleiades-ugv` bin（车设备端：stm32/lidar/slam/planning/executor/goal/急停 + `CarDeviceHandler`） | `cargo build -p pleiades-ugv` |
| C3 | 抽 `pleiades-uav` bin（机设备端：mavlink + 复制车 2D 决策 + `UavDeviceHandler`） | `cargo build -p pleiades-uav` |
| C4 | 抽 `pleiades-terminal` cdylib（地面站，原 `SrcPictorKernel`） | `cargo build -p pleiades-terminal` |
| C5 | 收尾：workspace 4 crate + `deploy_robot.sh` bin 名改 `pleiades-ugv` + 删死脚本/空目录 + 清理过时 doc | `cargo build --workspace` |
| 附加 | ① 新增第 5 crate `pleiades-sim`（无硬件模拟节点）；② UAV 2D 决策接线（「天上无人小车」）；③ 统一 `enabled=false → 跳过` 语义；④ 两轮 code review 修复（转向符号/D* Lite pop_valid/停车吞错/底盘泄漏等） | `cargo build --workspace` 0 error + robot 224 测试全绿（base 53/ugv 81/uav 47/sim 43） |

### 遗留/待后续（均非本任务范围或明确留后续）

- **UAV 3D 化**（真飞行逻辑、3D 寻路、高度控制、3D 避障）—— 未来 task
- **`ClusterInfo.node_type` 传播**（§3.5 明确「实施留后续」）
- **集成测试 stale**：`pleiades-base/tests/` 的 t01/t04/t07/t09/t13 编译失败（`StorageManager`/`PeerManager`/`DataType`/`lua`→`vm`/`GGUF_Analyze` 改名）—— **ML_review 范围**
- **VM 测试 2 失败**：`test_sandbox_os_blocked`（沙箱）+ `test_load_and_execute_hello_lua`（脚本相对路径）—— **ML_review 范围 / 既有**

---

## 一、目标

把 `Src/Robot/` 从「**一份代码适配所有设备**」重构为「**底座共享 + 设备端独有**」两层架构，让车、机、未来的船/四足各自实现、靠一份同构的共享世界状态协同。最终形态为一个底座 lib + 三个终端 crate（车/机/地面站）。

## 二、背景：为什么废弃 task_22

`task_22_universal_robot` 的「三层统一 + `MotionDevice` 统一 + Lua 决策」思路不对，本质是「一份代码适配所有设备」的固化思维。问题：

1. 统一 `MotionDevice` 只收敛到最小语义交集（前进/后退/转向/停），车机差异（Beep vs Takeoff、差速 vs 四旋翼、编码器 vs GPS）被硬塞或降维；
2. 「能力声明」「车机二选一装配」等复杂度，全部源于「单一核心适配多设备」这个错误前提；
3. 无人机被降维成「飞在天上的小车」（`vz` 恒 0，见 `mavlink/mod.rs` 的 `action_to_velocity`）。

正确认知（详见 `docs/heterogeneity_analysis.md`）：**异构设备在「共享世界状态」层是同构的，在「设备实现」层是异构的。** 底座只统一前者，后者各写各的。

## 三、架构分层

### 3.1 底座（共享，车机同一份代码）

| 模块 | 内容 |
|---|---|
| `core/protocol/` | ORION 帧 + `PoseData`/`MapDelta`/`TaskSet` 消息 |
| `core/cluster/` | `ClusterInfoTable` + 入站消费 + 失联清理 |
| `core/state.rs` 位姿部分 | `x/y/z/vx/vy/vz/yaw` + `ExecuteState`（意图） |
| `core/grid.rs`（`OccupancyGrid`，从 `slam` 挪来） | 地图表示（世界状态，CRDT 合并前提） |
| `core/robot.rs` 的 loop 骨架 | `select!` 三分支 + 命令分发框架 + 50ms tick 节拍 |
| `core/command.rs`/`mission.rs`/`mode.rs` | 命令/任务/模式语义（`ManualCmd` 超集） |

### 3.2 设备端（独有，车机各一份）

| | 车 | 机 |
|---|---|---|
| 驱动 | `stm32/` | `mavlink/` |
| 运动 | 差速 | 四旋翼 |
| 建图流程 | `lidar_mapper.rs` + `task.rs`（2D 雷达） | 3D 视觉（未来） |
| 里程计 | `odometry.rs`（编码器/IMU 积分） | `mavlink` 坐标对齐 `aligned_world_pose`（飞控 EKF，已实现） |
| 寻路 | `pathfinder.rs`（2D D*） | 3D 寻路（未来） |
| 决策 | `executor.rs`（三状态机） | 飞行逻辑（未来） |
| 独有状态 | `encoders[4]`、`LidarState`、急停 | 未来 |

### 3.3 两条关键约定

1. **独有状态跟设备走**：`encoders` → STM32 device，`LidarState` → Lidar device，急停 → 车端。
2. **命令超集共享 + 设备选择性响应**：`ManualCmd` 超集统一定义，设备只响应自己支持的子集（车不响应 `Takeoff`，机不响应 `Beep`），能力差异靠「响应」而非「声明」表达。

### 3.4 `robot.rs` 拆分边界 + 目录重组蓝图

**`robot.rs` 是「共享骨架 + 独有逻辑」的混合体**，拆分的精确边界：

| loop 里的部分 | 归属 |
|---|---|
| `select!` 三分支结构 | ✅ 共享（骨架） |
| 命令分发框架（`dispatch`） | ✅ 共享（骨架） |
| 50ms tick 节拍 | ✅ 共享（骨架） |
| `executor`（决策） | ❌ 独有（车三状态机 vs 机飞行逻辑） |
| `check_emergency_stop`（急停） | ❌ 独有（车 2D 扇形 vs 机 3D 避障） |

> ⚠️ **补充（代码调查发现）**：`robot.rs` 除 executor/急停外，还有三处设备耦合需一并拆：
>
> | 位置 | 耦合 | 处理 |
> |---|---|---|
> | `Robot::launch`（19 参） | `CarType`/chassis/lidar/flight_ctrl 装配 | 装配下沉到 ugv/uav |
> | `Robot` 结构体字段 | `lidar_state`（`Arc<RwLock<LidarState>>`）/ `map_tx: broadcast::Sender<Vec<MapDelta>>` | 摘出或改 trait 对象 |
> | `Robot::launch` 局部变量 | `motion: Arc<dyn MotionDevice>`（**非结构体字段**，随 dispatch 下沉） | 下沉到设备端 |
> | `dispatch` 设备分支 | `Beep→stm32` / `Takeoff/Land→mavlink` | 下沉到设备端 |
>
> 另有 `world.rs` 内嵌 `DStarLite` 寻路（`pathfinder` 字段 + `set_goal`/`clear_goal`/`has_goal`/`get_path`/`mark_obstacle`），移到 `core` 后仍反向依赖设备端 `planning`。**方案已定（2026-08-30）**：寻路下沉到设备端 `planning`，`world` 只保留纯地图只读接口 `get_cell`/`get_agents`（详见 A2）。

**目录重组结论**（对齐「底座 vs 设备端」分层）：

| 动作 | 方向 | 语义 |
|---|---|---|
| `control/device/{stm32,lidar}`、`control/types.rs` → `ugv/` | 设备端归位（车） | 车底盘 + 雷达驱动 + CarType |
| `control/device/mavlink` → `uav/mavlink` | 设备端归位（机） | 机飞控驱动 |
| `control/serial` → `util/serial` | 通用工具抽出 | IO 基础设施（车机共用） |
| `world.rs` → `core/world.rs` | 底座归位 | 共享世界状态（grid + cluster） |
| `slam/grid.rs`（`OccupancyGrid`）→ `core/grid.rs` | 底座归位 | 地图表示（共享） |
| `core/{executor,goal}` → `ugv/{executor,goal}` | 设备端归位（车） | 车决策 |
| `core/planning` → `ugv/planning` | 设备端归位（车） | 车寻路 |
| `slam/{lidar_mapper,task,odometry}` → `ugv/slam` | 设备端归位（车） | 车感知建图 |
| `robot.rs` 急停 → `ugv/emergency_stop.rs` | 设备端归位（车） | 车急停 |

### 3.5 节点类型（type）字段设计

**节点类型**（地面站 / 车 / 机）是必要的身份信息——当前 `ClusterInfo` 未存设备类型，一个设备收到另一个设备的 POSE 时无法判断对方是车还是机。

**位置**（不在 `RobotState`，因为它是「节点身份」而非「底盘物理状态」）：

| 层 | 放什么 |
|---|---|
| Config/身份层 | 节点类型是部署时确定的配置，启动时读取 |
| 集群表 `ClusterInfo` | 存 `node_type`，其他设备据此识别「对方是车/机」 |
| 协议层 | 扩展 `compid`（现有 `COMPID_ROBOT=1` / `GROUND_STATION=200` 只区分地面站 vs 机器人，需细分车/机） |

**表示**：`u8` 枚举（非 string）：

```rust
pub enum NodeType: u8 { GroundStation = 0, Car = 1, Uav = 2 }
```

**广播方式**：静态信息，**不进 POSE**（POSE 为 100ms 高频动态广播）。走「启动广播一次 / 低频 / request-response 按需查询」。

### 3.6 拆分后的文件结构（单代码库模块分层）

重构完成后 `Src/Robot/` 的目标目录树（图例：🟦 底座共享 · 🟧 设备端独有 · ⬜ 通用工具）：

```
Src/Robot/
├── mod.rs
│
├── core/                         # 🟦 底座（共享，车/机/地面站同一份）
│   ├── mod.rs
│   ├── protocol/                 # ORION 协议
│   │   ├── mod.rs
│   │   ├── frame.rs              # 帧编解码
│   │   ├── messages.rs           # PoseData / MapDelta / TaskSet
│   │   └── command_decode.rs     # 命令解码
│   ├── cluster/                  # 集群（ClusterInfo 含 node_type）
│   │   ├── mod.rs
│   │   ├── cluster_info.rs       # ClusterInfoTable
│   │   ├── consumer.rs           # 入站消费
│   │   └── maintenance.rs        # 失联清理
│   ├── state.rs                  # 位姿 schema（x/y/z/vx/vy/vz/yaw + ExecuteState）
│   ├── world.rs                  # 世界（grid + cluster，纯地图只读接口 get_cell/get_agents）
│   ├── grid.rs                   # OccupancyGrid（地图表示，从 slam 挪来）
│   ├── command.rs                # Command / Mission 定义
│   ├── command_consumer.rs       # 命令入站消费
│   ├── mission.rs                # MissionQueue
│   ├── mode.rs                   # OpMode
│   └── robot.rs                  # loop 骨架（装配 + 主循环，不含 executor/急停）
│
├── ugv/                          # 🟧 车（设备端，独有）
│   ├── mod.rs
│   ├── types.rs                  # CarType（原 control/types.rs）
│   ├── stm32/                    # 车底盘驱动（含 encoders）
│   ├── lidar/                    # 雷达驱动（含 LidarState）
│   ├── executor.rs               # 车三状态机（原 core/executor.rs）
│   ├── goal.rs                   # 目标服务（原 core/goal.rs）
│   ├── emergency_stop.rs         # 急停（原 robot.rs 拆出，车 2D 扇形）
│   ├── planning/                 # 车寻路（原 core/planning）
│   │   ├── mod.rs
│   │   ├── pathfinder.rs         # D* Lite
│   │   ├── assignment.rs         # 群发分配
│   │   └── cluster_obstacles.rs  # 动态障碍膨胀
│   └── slam/                     # 车感知建图
│       ├── mod.rs
│       ├── lidar_mapper.rs       # 点云建图（车 2D）
│       ├── task.rs               # SLAM task
│       └── odometry.rs           # 里程计（编码器/IMU 积分）
│
├── uav/                          # 🟧 机（设备端，独有）
│   ├── mod.rs
│   ├── mavlink/                  # 飞控驱动（含 aligned_world_pose 坐标对齐）
│   ├── executor.rs               # 飞行逻辑（先复制车 2D 三状态机，后 3D 化）
│   ├── goal.rs                   # 目标服务（先复制车版）
│   └── planning/                 # 机寻路（先 2D，后 3D）
│       ├── mod.rs
│       └── pathfinder.rs         # D* Lite（先 2D）
│
├── terminal/                     # 🟧 地面站（设备端，独有；原 SrcPictorKernel 桥，C 阶段迁入）
│   ├── mod.rs
│   └── ...                       # 手柄/摇杆/UI 等（未来）
│
└── util/                         # ⬜ 通用工具
    ├── mod.rs
    └── serial/
        ├── mod.rs
        └── port.rs               # 串口收发抽象（车机共用）
```

### 3.7 多终端 crate 方案（workspace 结构）

**最终形态：一个底座 lib + 三个终端 crate**，分别对应 `NodeType` 的三种节点（§3.5）：

| crate | 节点类型 | 形态 | 来源目录 | 独有内容 |
|---|---|---|---|---|
| `pleiades-base` | —— | lib（共享底座） | `Src/Robot/core/` + `Src/Robot/util/` + 其他共享模块 | 协议/集群/状态/世界/命令 |
| `pleiades-ugv` | `NodeType::Car = 1` | bin | `Src/Robot/ugv/` | stm32、lidar、executor、goal、planning、slam、急停 |
| `pleiades-uav` | `NodeType::Uav = 2` | bin | `Src/Robot/uav/` | mavlink、executor（先复制车 2D 版）、planning（先 2D） |
| `pleiades-terminal` | `NodeType::GroundStation = 0` | cdylib（给 Godot） | `SrcPictorKernel/`（C4 迁入） | 手柄/摇杆/UI 等 |

**workspace 声明**：

```toml
[workspace]
members = ["pleiades-base", "pleiades-ugv", "pleiades-uav", "pleiades-terminal"]
```

**依赖关系**（三个终端互不依赖，都只依赖 base）：

```
              pleiades-base（lib，共享底座）
            ▲            ▲            ▲
       依赖 │       依赖 │       依赖 │
   ┌────────┴───┐  ┌────┴────┐  ┌────┴─────────┐
   │ pleiades-  │  │ pleiades│  │ pleiades-    │
   │   ugv(bin) │  │ uav(bin)│  │ terminal(cdylib)
   └────────────┘  └─────────┘  └──────────────┘
       车             机            地面站
```

**关键性质**：

- **底座单一来源**：`pleiades-base` 一个 lib，改协议/集群三个终端同时生效；
- **设备端彻底分离**：车的二进制没有 mavlink，机没有 stm32，地面站没有车机驱动；
- **各自编译**：`cargo build -p pleiades-ugv` / `-p pleiades-uav` / `-p pleiades-terminal` 互不拖累；
- **不用分支同步**：同仓库同 workspace，靠 crate 依赖分层。

### 3.8 config 拆分设计

config 与代码同构——也分「共享段」和「设备段」。当前 `Robot_Config` 把 chassis/lidar/flight_ctrl 三种设备开关混在一个结构里，是「一份代码适配所有设备」在 config 层的体现。

| config 段 | 归属 | 内容 |
|---|---|---|
| `[Log]` / `[Network]` / `[Storage]` | base（共享） | 日志 / 网络 / 存储 |
| `[Identity]` | base（共享） | `peer_name` + **`node_type`**（新增，§3.5） |
| `[chassis]` / `[lidar]` | ugv（车） | 车底盘 + 雷达 |
| `[flight_ctrl]` | uav（机） | 飞控 |

**关键变更**：

1. **`Robot_Config` 拆掉**：chassis/lidar 归车、flight_ctrl 归机，底座 `Pleiades_Config` 移除 `Robot` 段；
2. **`node_type` 进 `[Identity]`**：与 `peer_name` 并列（节点身份），`u8` 枚举（§3.5）；
3. **`robot_bootstrap` 拆到各终端**：当前 `bootstrap.rs::robot_bootstrap` 读 `Robot_Config` 硬编码展开 18 参数 + 车机二选一；拆分后车/机各自维护自己的装配逻辑（读自己的设备段、spawn 自己的 device）；
4. **`identity.rs` 留 base**：PeerId 密钥对所有终端共用。

**文件层面拆分**（每个终端有自己的 config.rs）：

| 文件 | 结构 | 内容 |
|---|---|---|
| `pleiades-base/src/config.rs` | `BaseConfig` | 共享段 `Log`/`Network`/`Storage`/`Identity`（含 `node_type`），结构定义一次 |
| `pleiades-ugv/src/config.rs` | `UgvConfig { #[serde(flatten)] base, chassis, lidar, obstacle_inflation_radius }` | 车独有：`ChassisConfig` + `LidarConfig` |
| `pleiades-uav/src/config.rs` | `UavConfig { #[serde(flatten)] base, flight_ctrl }` | 机独有：`FlightCtrlConfig` |
| `pleiades-terminal/src/config.rs` | `TerminalConfig` | 地面站独有（未来） |

> 共享段结构在 base 定义一次，ugv/uav 用 `#[serde(flatten)]` 组合复用、**不重复定义**；设备段字段只出现在自己终端的 config.rs，互不污染。

**拆分后 config.toml 形态**（各终端一份，共享段 + 自己的设备段）：

```toml
# 共享段（所有终端相同）
[Log]
[Network]
[Storage]
[Identity]
peer_name = "robot-pi"
node_type = "car"        # 车=car / 机=uav / 地面站=ground_station

# ugv 独有设备段（顶层段，Robot 段已删）
[chassis]
port = "/dev/ttyUSB0"
...
[lidar]
...
# uav 独有设备段
[flight_ctrl]
connection = "/dev/ttyS0"
...
```

### 3.9 已确认设计决策（2026-08-30）

讨论定稿三条决策，§五（涉及文件）与 §六（实施步骤）据此展开：

| # | 决策 | 内容 |
|---|---|---|
| D1 | **world = 纯世界地图信息** | `world` 只保留 `grid`（`get_cell`）+ `cluster`（`get_agents`）两个只读接口；寻路 `DStarLite` 下沉设备端，车/机各自拿地图信息、各自寻路（不抽象 base trait，直接下沉） |
| D2 | **删除 `MotionDevice`** | 车/机独立后每个二进制只有一种运动设备，无「二选一」多态需求；`dispatch`/`apply_action`/`invalidate`/急停改走具体类型（`STM32Device`/`MavlinkDevice`），删 `device.rs` |
| D3 | **Lua 决策层不恢复** | 决策已回退 Rust（task_22_3 单写者收敛），不实现 robot 的 Lua 绑定；死脚本 `robot_test.lua`/`car_lua_decision.lua` 移除 |

---

## 四、范围

重构分**三个阶段**，按序执行。每阶段结束都可编译、可验证、**车行为不变**（安全底线）。

### 阶段 A：目录重组（纯移动 + 改名，行为不变）

1. `control/` 拆散归位：`device/{stm32,lidar}` + `types.rs` → `ugv/`，`device/mavlink` → `uav/mavlink`，`serial/` → `util/serial/`
2. `world.rs` → `core/world.rs`
3. `slam/grid.rs` → `core/grid.rs`；`slam/{lidar_mapper,task,odometry}` → `ugv/slam/`
4. `core/executor.rs` → `ugv/executor.rs`、`core/goal.rs` → `ugv/goal.rs`
5. `core/robot.rs` 急停拆出 → `ugv/emergency_stop.rs`
6. `core/planning/` → `ugv/planning/`（出 core）
7. 删除 `device.rs`（`MotionDevice` 统一接口）

### 阶段 B：state 收敛（独有状态摘出）

1. `RobotState` 摘出 `encoders[4]` → STM32 device
2. `LidarState` 摘出 → Lidar device

### 阶段 C：crate 拆分（多终端 workspace）

1. 抽 `pleiades-base` lib（底座 + 基础设施）
2. 抽 `pleiades-ugv` bin（车设备端）
3. 抽 `pleiades-uav` bin（机设备端）
4. 抽 `pleiades-terminal` cdylib（地面站，原 `SrcPictorKernel`）

### 本次不做（后续 task）

- 机端扩展（MavlinkDevice 3D 能力、机决策/飞行逻辑；位姿维护已由飞控 EKF + 坐标对齐实现，不重复建里程计）
- 真 3D 寻路（先 2D，机的高度独立通道后续）
- 运行时 `.so` 动态插件（先编译期 + config）
- 世界模型扩展字段（设备类型标签等，设计见 §3.5，实施留后续）

---

## 五、涉及文件

按改动类型分五类：**移动**（`git mv` 纯搬）、**拆分**（从文件拆出部分逻辑）、**重写**（逻辑改动）、**移除**（删除）、**依赖/清单同步**（伴随以上动作的机械改动）。

### 5.1 移动（git mv，内容不变）

| 原路径 | 新路径 | 阶段 |
|---|---|---|
| `Src/Robot/control/device/stm32/` | `Src/Robot/ugv/stm32/` | A1 |
| `Src/Robot/control/device/lidar/` | `Src/Robot/ugv/lidar/` | A1 |
| `Src/Robot/control/device/mavlink/` | `Src/Robot/uav/mavlink/` | A1 |
| `Src/Robot/control/types.rs` | `Src/Robot/ugv/types.rs` | A1 |
| `Src/Robot/control/serial/` | `Src/Robot/util/serial/` | A1 |
| `Src/Robot/world.rs` | `Src/Robot/core/world.rs` | A2 |
| `Src/Robot/slam/grid.rs` | `Src/Robot/core/grid.rs` | A3 |
| `Src/Robot/slam/{lidar_mapper,task,odometry}.rs` | `Src/Robot/ugv/slam/` | A1 |
| `Src/Robot/core/executor.rs` | `Src/Robot/ugv/executor.rs` | A4 |
| `Src/Robot/core/goal.rs` | `Src/Robot/ugv/goal.rs` | A4 |
| `Src/Robot/core/planning/`（整目录） | `Src/Robot/ugv/planning/` | A5 |

### 5.2 拆分（从文件拆出部分逻辑到别处）

| 文件 | 拆出内容 | 去向 | 阶段 |
|---|---|---|---|
| `world.rs` | `pathfinder` 字段 + `set_goal`/`clear_goal`/`has_goal`/`get_path`/`mark_obstacle` 5 方法 | 设备端：`GoalService`/急停直接持有 `DStarLite`；world 只留 `get_cell`/`get_agents` | A2 |
| `core/robot.rs` | `check_emergency_stop`（`robot.rs:580-618`） | `ugv/emergency_stop.rs`（新建） | A4 |
| `core/robot.rs` | `dispatch` 设备分支（`Beep→stm32`、`Takeoff/Land→mavlink`、LiDAR 开关） | 车/机各自 dispatch | A6 |
| `core/state.rs` | `encoders: [i32; 4]` 字段（`state.rs:77`） | STM32 device 内部存储 | B1 |
| `core/state.rs` | `LidarState` 结构（`state.rs:93-96`）+ `use ...lidar::LaserScan` | Lidar device 内部维护 | B2 |

### 5.3 重写（逻辑改动）

| 文件 | 重写内容 | 阶段 |
|---|---|---|
| `core/robot.rs` | `Robot::launch`（19 参，`robot.rs:88-108`）装配下沉；`Robot` 结构体去设备字段（`lidar_state`/`map_tx` 等）；`invalidate`/`apply_action`/`dispatch`/`check_emergency_stop` 的 `&dyn MotionDevice` 改具体类型 | A6 |
| `core/robot.rs` | 主循环 `main_loop` 车/机分支分离（车三状态机 + 急停；机飞行逻辑） | C2/C3 |
| `Src/bootstrap.rs` | `robot_bootstrap`（`bootstrap.rs:202-306`）拆到车/机终端装配；`core_bootstrap` 摘出 `robot_cmd_frame_rx`（`bootstrap.rs:38,130,174,194`） | C0 |
| `Src/Config/config.rs`（base）+ 新建 `pleiades-ugv/src/config.rs`、`pleiades-uav/src/config.rs` | base 删 `Robot` 段、`Identity_Config` 加 `node_type`；`ChassisConfig`/`LidarConfig` → ugv、`FlightCtrlConfig` → uav（共享段 `#[serde(flatten)]` 复用） | C0 |
| `Src/main_robot.rs` | 改为 `pleiades-ugv` 入口，调 ugv 装配 | C2 |
| `Src/Robot/mod.rs` | 移除 `pub use control::types::CarType`（types 迁 ugv），更新模块声明 | A1/A6 |

### 5.4 移除（删除）

| 文件 | 说明 | 阶段 |
|---|---|---|
| `Src/Robot/device.rs` | `MotionDevice` trait（27 行，统一运动接口，决策 D2） | A6 |
| `programs/user/robot_test.lua` | 死脚本（引用不存在的 `robot.*` caps + `encoders`，决策 D3） | B1 |
| `programs/archived/car_lua_decision.lua` | 已归档死脚本，随 Lua 决策废弃（决策 D3） | C5 |
| `SrcPictorKernel/Cargo.toml` | 并入 `pleiades-terminal` | C4 |

### 5.5 依赖/清单同步（伴随以上动作）

| 文件 | 改动 | 阶段 |
|---|---|---|
| `Src/Robot/{mod,core/mod,ugv/mod,uav/mod,util/mod}.rs` | 模块声明 + use 路径更新 | A1-A5 |
| `Cargo.toml` | `mavlink` 依赖移 `pleiades-uav`；`serial2/serial2-tokio` 随 util/serial 归属 | C0 |
| `build.sh` / `deploy_robot.sh` | bin 名改 `pleiades-ugv`；programs 分发清单去 `robot_test.lua`/archived | C5 |
| `MapDelta`/`DecisionState`/`MotionAction`/`DecisionResult` | 明确归属：`MapDelta` 随 SLAM 留设备端；`DecisionState`/`MotionAction` 随 executor 留设备端 | C1 |

---

## 六、详细实施步骤

> 每个步骤结束都需 `cargo check`（或 `cargo build -p <crate>`）通过；阶段 A/B 还需「车行为不变」。小步提交，便于回滚。每步标注动作类型：🟦 拆分 / 🟧 重写 / 🟥 移除 / ⬜ 移动。

### 阶段 A：目录重组（纯移动 + 改名 + 拆耦合，行为不变）

**A1. `control/` 拆散归位（stm32/lidar → ugv，mavlink → uav，serial → util）** ⬜ 移动

1. 新建 `Src/Robot/ugv/`、`Src/Robot/uav/`；
2. `git mv Src/Robot/control/device/stm32/ Src/Robot/ugv/stm32/`；
3. `git mv Src/Robot/control/device/lidar/ Src/Robot/ugv/lidar/`；
4. `git mv Src/Robot/control/device/mavlink/ Src/Robot/uav/mavlink/`；
5. `git mv Src/Robot/control/types.rs Src/Robot/ugv/types.rs`；
6. `git mv Src/Robot/control/serial/ Src/Robot/util/serial/`；
7. 删除空的 `control/`，新建 `ugv/mod.rs`、`uav/mod.rs`、`util/mod.rs`；
8. 全库替换 use 路径：`control::device::{stm32,lidar}` → `ugv::...`、`control::device::mavlink` → `uav::mavlink`、`control::types` → `ugv::types`、`control::serial` → `util::serial` 等；
9. 验证：`cargo check` 通过。

**A2. `world.rs` → `core/world.rs`（拆出内嵌寻路）** 🟦 拆分

> **决策 D1**：world = 纯世界地图信息。只保留 `grid`（`get_cell`）+ `cluster`（`get_agents`）两个只读接口；寻路下沉设备端。

1. `git mv Src/Robot/world.rs Src/Robot/core/world.rs`；
2. **拆** `World` 的 `pathfinder: Mutex<Option<DStarLite>>` 字段（`world.rs:30`）与 5 个寻路方法 `set_goal`/`clear_goal`/`has_goal`/`get_path`/`mark_obstacle`（`world.rs:55-96`）；`World::new` 只收 `grid` + `cluster`；
3. **改调用点**（寻路下沉后直接持有 `DStarLite`）：
   - `goal.rs`：`set_goal`/`clear_goal`/`get_path`（`goal.rs:76/129/139/144/153/163`）改由 `GoalService` 自己持有 `DStarLite`，从 `world.get_cell` + cluster 快照取地图信息寻路；
   - `robot.rs:614` 急停 `mark_obstacle`：改由 `check_emergency_stop`（A4 拆出后）直接持有 `DStarLite` 引用；
4. **删** `world.rs:20` 的 `use ...planning::pathfinder::DStarLite`（反向依赖消除）；
5. 更新 `core/mod.rs` 声明 + 引用路径（`slam::{OccupancyGrid, CELL_RESOLUTION}` 改 `core::grid`）；
6. 验证：`cargo check`。

**A3. `slam/` 拆分归位（grid → core，其余 → ugv/slam）** ⬜ 移动

1. `git mv Src/Robot/slam/grid.rs Src/Robot/core/grid.rs`；
2. `git mv Src/Robot/slam/ Src/Robot/ugv/slam/`（lidar_mapper/task/odometry 随之移动，grid 已先挪走）；
3. 更新 `core/mod.rs`（加 grid）、`ugv/slam/mod.rs`、`ugv/mod.rs`、引用路径（`slam::OccupancyGrid` → `core::grid::OccupancyGrid`）；
4. 验证：`cargo check`。

**A4. `executor`/`goal`/急停 → `ugv/`** 🟦 拆分 + ⬜ 移动

1. `git mv Src/Robot/core/executor.rs Src/Robot/ugv/executor.rs`；
2. `git mv Src/Robot/core/goal.rs Src/Robot/ugv/goal.rs`；
3. **拆** `core/robot.rs` 的 `check_emergency_stop`（`robot.rs:580-618`）→ 新建 `ugv/emergency_stop.rs`；其依赖的 `lidar_state`/`robot_state`/`motion`/`world` 由参数传入（`motion` 此时仍是 `&dyn MotionDevice`，A6 再改具体类型）；
4. 更新 `core/mod.rs`、`ugv/mod.rs`、引用路径；
5. 验证：`cargo check` + 车行为不变（急停仍正常）。

**A5. `core/planning/` → `ugv/planning/`（出 core）** ⬜ 移动

1. `git mv Src/Robot/core/planning/ Src/Robot/ugv/planning/`；
2. 更新 `core/mod.rs`（移除 planning）、`ugv/mod.rs`、`ugv/planning/mod.rs`、引用路径；
3. 验证：`cargo check` + 现有寻路测试通过。

**A6. 删除 `device.rs`（`MotionDevice`），dispatch 改具体类型** 🟥 移除 + 🟧 重写

> **决策 D2**：车/机独立后各二进制只有一种运动设备，删除统一 trait，改走具体类型。

1. **重写 `robot.rs` 运动调用链**（去 `&dyn MotionDevice`）：
   - `launch` 内 `motion: Arc<dyn MotionDevice>` 局部变量（`robot.rs:180-186`）删除；车/机装配下沉后（C0）由各自装配直接传具体类型；
   - `invalidate`（`robot.rs:493`）/ `apply_action`（`robot.rs:505`）/ `dispatch`（`robot.rs:522`）/ `check_emergency_stop`（`robot.rs:583`）的参数 `&dyn MotionDevice` → 具体类型（车 `STM32Device`、机 `MavlinkDevice`）；
2. **拆分 dispatch 设备分支**：
   - 车 dispatch：`Forward/Backward/SpinLeft/SpinRight/Stop` + `Beep`（`stm32.beep`）+ `StartLidarScan/StopLidarScan`；
   - 机 dispatch：`Forward/Backward/SpinLeft/SpinRight/Stop` + `Takeoff/Land`（`mavlink.takeoff_send/land_send`）；
3. **删** `Src/Robot/device.rs`（`MotionDevice` trait，27 行），移除 `mod.rs` 导出与 `robot.rs:33` 的 use；
4. **删** `stm32/mod.rs` 与 `mavlink/mod.rs` 的 `impl MotionDevice` 块；
5. 验证：`cargo check` 通过（无 dead code 引用）。

### 阶段 B：state 收敛（独有状态摘出）

**B1. `encoders` 摘出 → STM32 device** 🟦 拆分（零风险，当前为死数据）

现状：`encoders[4]` 由 `stm32/protocol.rs::update_state`（`protocol.rs:139`）写入，仅被死脚本 `robot_test.lua` 与测试断言（`protocol.rs:384`）读取，核心代码不消费。

1. **拆** `state.rs:77` 的 `encoders: [i32; 4]` 字段；
2. `STM32Device` 内部新增编码器存储，`update_state` 改写到设备内部；
3. **删** 死脚本 `programs/user/robot_test.lua`（决策 D3：Lua 决策层不恢复）；更新 `stm32/protocol.rs:384` 测试断言；
4. 验证：`cargo check` + 测试更新通过 + 车行为不变。

**B2. `LidarState` 摘出 → Lidar device** 🟦 拆分

现状：`LidarState` 消费者 = SLAM（`slam/task.rs:48,74-81`，经 `SlamContext`）+ 急停（`robot.rs:441` → A4 已挪 `ugv/emergency_stop.rs`）。

1. **拆** `state.rs:93-96` 的 `LidarState`，移到 `ugv/lidar/` 模块，`LidarDevice` 内部维护（SLAM 直接消费，不再走共享锁）；
2. **删** `state.rs:15` 的 `use crate::robot::control::device::lidar::LaserScan`（反向依赖消除）；
3. 急停的 LiDAR 读取改从 `LidarDevice` 句柄获取（A4 已就位）；
4. 验证：`cargo check` + 急停/建图行为不变。

### 阶段 C：crate 拆分（多终端 workspace）

**C0. config + 装配通道拆分（前置，随 C1-C4 同步）** 🟧 重写

1. `Identity_Config` 加 `node_type`（`u8` 枚举，§3.5），config.toml `[Identity]` 加 `node_type`；
2. **重写** config：base 的 `Pleiades_Config` 删 `Robot` 段、`Identity_Config` 加 `node_type`；**新建** `pleiades-ugv/src/config.rs`（`UgvConfig` = chassis/lidar/obstacle_inflation_radius）与 `pleiades-uav/src/config.rs`（`UavConfig` = flight_ctrl），共享段经 `#[serde(flatten)]` 复用 `BaseConfig`（见 §3.8）；
3. **重写** `bootstrap.rs::robot_bootstrap`（`bootstrap.rs:202-306`）→ 拆成各终端装配函数（车：读 chassis/lidar → spawn stm32/lidar；机：读 flight_ctrl → spawn mavlink）；
4. **拆** `core_bootstrap` 里 `robot_cmd_frame_rx`（`bootstrap.rs:38,130,174,194`）——它仅车/机消费，从 base 移到车/机设备 bootstrap；
5. **Cargo 依赖拆分**：`mavlink` 移 `pleiades-uav`；`serial2/serial2-tokio` 随 util/serial 归属走；
6. 各终端 crate 的 `main.rs` 调自己的装配函数 + 传自己的 `node_type`；
7. 验证：各终端 `cargo build -p <crate>` 通过，config 解析正确。

**C1. 抽 `pleiades-base` lib** 🟧 重写

1. 新建 `pleiades-base/` crate（`Cargo.toml` + `src/lib.rs`）；
2. 迁入共享模块：`config/network/ml_engine/peer_management/orchestrator/storage/session/bootstrap/event_bus/tui/cli/vm/api` + `robot/core/` + `robot/util/`；
3. 迁入 `Src/lib.rs` 的模块声明与 `pub use` 导出（改名 `pleiades_base`）；
4. 验证：`cargo build -p pleiades-base` 通过。

**C2. 抽 `pleiades-ugv` bin** 🟧 重写

1. 新建 `pleiades-ugv/` crate（依赖 `pleiades-base`）；
2. 迁入 `Src/main_robot.rs` → `src/main.rs` + `robot/ugv/`（整目录：stm32/lidar/types/executor/goal/emergency_stop/planning/slam）；
3. 设备端通过 `pleiades_base` 的公共接口（协议/集群/状态/世界）与底座交互；
4. 验证：`cargo build -p pleiades-ugv` 通过，车载节点可启动。

**C3. 抽 `pleiades-uav` bin** 🟧 重写

1. 新建 `pleiades-uav/` crate（依赖 `pleiades-base`）；
2. 迁入 `robot/uav/`（mavlink 驱动）；新建机载 `main.rs`（`NodeType::Uav` 身份）；**复制**车的 `ugv/executor`、`ugv/goal`、`ugv/planning` 到 uav（先 2D 跑通，后 3D 化）；
3. 验证：`cargo build -p pleiades-uav` 通过（当前仅底座 + mavlink 驱动可编译）。

**C4. 抽 `pleiades-terminal` cdylib** 🟧 重写

1. `SrcPictorKernel/` → `pleiades-terminal/`，`Cargo.toml` 设 `crate-type = ["cdylib"]`，依赖 `pleiades-base`；
2. `lib.rs` 改 `NodeType::GroundStation` 身份；预留手柄/摇杆等地面站设备（未来）；
3. 验证：`cargo build -p pleiades-terminal` 通过。

**C5. 收尾** 🟧 重写 + 🟥 移除

1. 更新 workspace `Cargo.toml`：`members = ["pleiades-base", "pleiades-ugv", "pleiades-uav", "pleiades-terminal"]`；
2. 处理 `Src/main.rs`（`Pleiades` 纯推理 bin）——并入 base 作为独立 bin，或保留为推理节点；
3. **删** `programs/archived/car_lua_decision.lua`（决策 D3）与 `programs/user/robot_test.lua`（若 B1 未删）；
4. 全量 `cargo build --workspace` 通过 + 车行为端到端验证。

---

## 七、验证方式

| 阶段 | 验证 |
|---|---|
| 阶段 A（目录重组） | 每步 `cargo check` + 车行为不变（走格子/转向/直行/到达/急停） |
| 阶段 B（state 收敛） | `cargo check` + 测试通过 + 急停/建图行为不变 |
| 阶段 C（crate 拆分） | `cargo build -p pleiades-{base,ugv,uav,terminal}` 各自通过 + 车载节点端到端 |
| 全程 | 不破坏 `Pleiades-Orion` 稳定分支 |

> **行为一致性是安全底线**：重构后车的行为必须与重构前完全一致。

## 八、风险与注意

1. **行为一致性**：阶段 A/B 均为「纯重构 + 死数据迁移」，不引入新逻辑，车行为必须不变。
2. **急停安全**：急停挪位置但**不能降级**——仍是 Rust 最内层、编译期代码、不可被绕过。
3. **单一写入者不变式**：`RobotState` 收敛后，位姿/速度/姿态部分仍由设备 RX 回调单一写入；`encoders`/`LidarState` 摘出后各自设备内部维护，不变式更简单。
4. **crate 依赖方向**：`base` 不能反向依赖 `ugv/uav/terminal`（否则分层失效）；设备端通过 base 暴露的 trait/接口与底座交互，接口边界需在 C1 先定清楚。
5. **`MotionDevice` 删除时序**：必须在 A4（dispatch 解耦）之后删，避免先删导致 dispatch 无接口可用。
6. **小步提交**：每个步骤单独 commit，便于回滚。
7. **反向依赖硬耦合（调查发现的 4 处）**：`world.rs→DStarLite`、`robot.rs` 装配/结构体/dispatch、`state.rs→LaserScan`、`core_bootstrap→robot_cmd_frame_rx` 必须显式拆，否则 `pleiades-base` 无法独立于设备端编译。
