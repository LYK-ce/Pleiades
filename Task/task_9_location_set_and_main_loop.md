# Task 9: Location Set and Main Loop

> 状态：Location Set 已实施完成（待实车验证）；Main Loop 部分设计已定稿（待实施）
> 创建日期：2026-08-06
> 最后更新：2026-08-06
>
> ✅ 2026-08-06：Location Set 全部实施完成（S1~S8），36+3 测试全绿，orion-robot / Pleiades 两个二进制编译通过。详见 `Workbook/wb_9_location_set_and_main_loop.md`。

## 目标

本任务包含两个子目标：

1. **Location Set（已确认）**：`RobotState` 直接记录全局唯一世界坐标，消除"相对位移 + 消费方各自 +64.0"的旧写法；支持 `./orion-robot 66.5 63.25` 启动参数设定小车初始位置（origin），缺省 (64.0, 64.0)。
2. **Main Loop（已定稿）**：Robot ↔ Network 数据通道打通——位姿/地图经 P2P 广播到集群其他节点，并接收其他节点的位姿/地图落地展示。**不做融合/协同决策**（将来再做）。命令不走 P2P。


## Main Loop 部分设计（2026-08-06 定稿）

### 架构决策

1. **两个主循环保持独立**：推理主循环（Core）与 Robot 主循环各跑各的，互不知道对方存在（实时性隔离：Robot 50ms tick 不被推理任务拖累；职责边界：Core 属 ML_review 分支）。
2. **命令不走 P2P**：Robot 命令来源只有本地（WS 遥控，未来 Lua/TUI）。Network 与 Robot 只交换数据。
3. **数据全部走 B2（request-response 通道）**：帧上限 2GB（Request_Response/codec.rs:25），65KB 全量地图一帧承载，不落盘不进 Storage；send_file 文件流留给 GB 级模型分发。
4. **EventBus 只作入站公告板**：不是两个循环的连接线（两循环间无直接连接）；出向走 Robot 自己的 broadcast（pose_tx/map_tx）。

### 数据流

```
出向（Robot → Network）：组装层 relay task（main.rs 建）
  pose_tx(100ms) ──► RobotPose JSON ──► network.broadcast
  map_tx(200ms)  ──► RobotMapDelta JSON ──► network.broadcast
  grid(低频)     ──► RobotMapFull 65KB ──► network.broadcast

入向（Network → 本地）：
  B2 inbound_rx → route_inbound（branch_command.rs:15）加 3 个 match 臂
    ├─ RobotPose / RobotMapDelta / RobotMapFull
    └─ 解析 → EventBus Stream{type:"robot_pose" / "robot_map_delta" / "robot_map_full"}
        → 前端/TUI 订阅展示（融合/避障逻辑本次不做）
```

### 文件计划

| 侧 | 文件 | 改动 |
|---|---|---|
| Robot 侧（本分支） | 新增 relay task（组装层，main.rs 或新模块） | 订阅 pose_tx/map_tx/grid → Network 能力 |
| Network 侧（ML_review） | command.rs：`NetworkProtocol` 加 `RobotPose`/`RobotMapDelta`/`RobotMapFull` 变体 + Parse/Serialize | 必改 |
| Network 侧 | branch_command.rs：route_inbound 加 3 个 match 臂 → EventBus | 必改 |
| Network 侧 | Network/mod.rs：`broadcast(dt, payload)`（照抄 broadcast_local_info 遍历模式） | 必改 |
| Network 侧 | node_handle.rs：fire-and-forget 发送 API（`send_data_no_wait`），避免 10Hz 广播积压 | 必改 |
| Network 侧 | capability_binding.rs：`caps.network.broadcast` Lua 绑定（可选） | 建议 |

### ⚠️ 入站坑（必须注意）

- `DataType::Data` 入站被 Network 内部自动回 "OK"、**不转发 Core**（swarm_events.rs:150-160）→ 位姿/地图必须走 **Command 通道新协议变体**，否则永远收不到。
- send_data 强制等对端 Response（默认 300s 超时）→ 高频广播必须用 fire-and-forget。
- 无原生广播：broadcast API 需要 Network 侧新增（遍历 peers 模板已存在）。
## 背景

### 历史遗留（Location Set 为什么存在）

| 阶段 | 内容 | 证据 |
|---|---|---|
| Task 5（SLAM 建图） | 无定位，位姿硬编码 (64,64)，64.0 是地图 Chunk 中心锚点 | Task 6 文档："当前状态：建图 ✅，定位 ❌（位姿硬编码）" |
| Task 6（里程计） | 为"替代硬编码"引入 odom_x/y（从 0,0 累积相对位移），消费方各自 `64.0 + odom`。约束"不新增 task、不新增锁"，累积塞进 STM32 RX 回调 | commit ff89c4f："位姿改用 64.0+odom，不再硬编码" |
| Task 7/8 | Executor、D* Lite 沿用同一模式 → 魔法数漂移到 4 处 | executor.rs:111、robot.rs:174/207-208 |

**结论**：odom 保持纯相对量本无错（运动学与地图锚点解耦），错在**锚点加法未集中成单一出口**，散落在每个消费方——后续任务继承放大。本次 Task 9 收口。

## 方案设计（Location Set）

### 设计原则（已与人类确认）

1. `RobotState.x/y` = **全局唯一世界坐标**，启动时 = origin（默认 64.0, 64.0）
2. **消费方只读，零换算**：任何地方不再出现 `64.0 + odom_x` 类计算
3. **更新只在 RobotState 上进行**：`odometry::accumulate` 直接在世界坐标上积分（公式不变，字段名变）
4. **单一写入者不变式**：唯一写入路径 = STM32 RX 回调的 `local_state` 全量覆盖写；origin 注入点在 `STM32Device::spawn` 的 local_state 初始化

### 改动清单（7 个必改文件）

| # | 文件 | 位置 | 改动内容 |
|---|---|---|---|
| 1 | `Src/Robot/core/state.rs` | :74-77 字段 + 模块 doc | `odom_x/odom_y` 改名 `x/y`；注释改为"全局世界坐标 (m)，启动时 = origin"；模块 doc 补充单一写入者不变式 |
| 2 | `Src/Robot/slam/odometry.rs` | :19-20 积分体 + doc | 积分目标改为 `x/y`（公式不动）；doc 声明 x/y 初值 = origin、本函数只允许作用于 local_state |
| 3 | `Src/Robot/control/device/stm32/mod.rs` | :44 spawn 签名、:48 初始化、:71 覆盖写 | spawn 加 `origin: (f32, f32)` 参数；local_state 初始化后注入 origin；:71 加单一写入者注释 |
| 4 | `Src/Robot/core/robot.rs` | :67-73 launch、:77 共享态、:90 spawn 调用、:174/:207-208 消费点 | launch 加 origin 参数；创建共享态后**立即注入 origin**（关闭初始化窗口，须在 spawn notifier 之前）；spawn 调用传 origin；两处消费点删 `64.0 +` 直读 `s.x/rs.x`；:205 注释同步更新 |
| 5 | `Src/Robot/core/executor.rs` | :111 感知 | `(wx, wy) = (robot_state.x, robot_state.y)`，删 64.0 换算（executor 不需要 origin，只读） |
| 6 | `Src/main_robot.rs` | :18 launch 调用 | `std::env::args()` 解析两个可选位置参数（`./orion-robot 66.5 63.25`），缺省 (64.0, 64.0)，非法输入 warn 回退默认；launch 调用传 origin；启动日志打印 origin |
| 7 | `Src/main.rs` | :125 Phase 5.6 launch 调用 | 补默认 origin (64.0, 64.0) 实参（完整系统入口无 CLI 需求） |

### 可选/建议改动

| 文件 | 位置 | 说明 |
|---|---|---|
| `stm32/mod.rs` | :84-148 `spawn_mock` | mock 签名同步加 origin（否则 mock 测试态位置恒 (0,0)，当前无测试断言位置、非阻塞） |
| `state.rs` 模块 doc | :1-8 | 强化"x/y 为世界坐标、唯一写入路径 = STM32 RX 回调全量覆盖" |

### 不改清单（已核实无引用）

- `Src/WebSocket/server.rs`、`Src/WebSocket/protocol.rs`：只透传 `p.x/p.y`，无 64 换算
- `Tool/robot_control.html`：直显 `d.x/d.y`、直发世界坐标，新旧语义行为一致
- `Src/VM/capability_binding.rs`：`register_robot_caps` 是空 stub，无位置引用
- `Src/TUI/`、`Src/CLI/`、`Src/API/`、`programs/user/*.lua`、`tests/`：无 RobotState 位置引用
- `Src/Robot/slam/pathfinder.rs`、`grid.rs`：纯网格坐标，无 odom/64 字面量
- `Src/Robot/control/serial/`、`types.rs`、`constants.rs`、`protocol.rs`：update_state 不写位置
- 归档文档（task_6/wb_6/robot_controller.md）：历史记录，不改
- lidar_mapper.rs 测试 fixture 的 `RobotPose { x: 64.0 }`：值仍合法，不改
- ⚠️ 注意：checksum/parser 中的 `64.0` 是 LiDAR 角度缩放 SDK_UNIT64，**与位置无关，勿动**

### origin 传递链

```
CLI (main_robot.rs) → Robot::launch(origin) → 共享态注入(robot.rs)
                                           └→ STM32Device::spawn(origin) → local_state 初始化
                                                                          └→ RX 回调 accumulate 积分 x/y
                                                                              └→ 全量覆盖共享态
                                                                                  └→ 消费方直读 (notifier/slam/executor)
```

### 实施步骤

依赖关系：state.rs（地基）→ 写入侧（odometry/stm32）→ 组装（robot.rs）→ 消费侧（executor）→ 入口（main_robot/main）。

| 步骤 | 改动 | 验证 |
|---|---|---|
| S1 | state.rs 字段改名 odom_x/y → x/y + 注释 | 与 S2-S5 同批原子落地（单独编译必然失败，属预期） |
| S2 | odometry.rs 积分目标改 x/y + doc | 新增 accumulate 世界坐标积分测试 |
| S3 | stm32/mod.rs spawn 加 origin、local_state 注入、:71 注释；spawn_mock 同步 | stm32 3 个 mock 测试 + 新增 origin 注入测试 |
| S4 | robot.rs：launch 加 origin、共享态注入（关初始化窗口）、spawn 传参、两处消费点删换算、:205 注释 | 编译通过 + 新增初始化窗口测试 |
| S5 | executor.rs:111 直读 x/y | `./build.sh check` |
| S6 | main_robot.rs CLI 解析 + 传 origin + 日志 | `cargo build --bin orion-robot`；启动验证日志 |
| S7 | main.rs:125 补默认 origin | `cargo build --bin Pleiades` |
| S8 | 全量回归 | `./build.sh test` 全绿；实车验证 WS 遥测显示 origin |

### 风险与注意事项

1. **初始化窗口（必处理）**：launch 创建共享态用 `default()`，首帧 RX 覆盖前（~100ms）notifier/slam_task 可能读到 (0,0)。必须在 spawn notifier 之前向共享态注入 origin（S4 已含）。
2. **单一写入者不变式**：x/y 只在 RX 回调 local_state 维护，共享态只被全量覆盖。未来加"人工设位/地图匹配修正"必须维护此不变式。
3. **f32 精度（无需处理）**：~64 量级 ulp ≈ 7.6e-6 m，每帧增量远大于 ulp；1 小时累积误差 ≈ 7e-4 m，远小于机械里程计漂移。
4. **CLI 范围校验**：grid 限制世界坐标 ∈ [0,128)m，CLI 传入越界值会落在地图外、建图失效，解析时 warn。
5. **外部消费者语义升级**：get_state() 返回的世界坐标对 Lua/API 是透明升级（不再需要自己 +64）。

## 待决策问题（Main Loop 部分）

（待人类补充需求后填充）

## 已决策

| # | 问题 | 决策 |
|---|------|------|
| 1 | CLI 参数格式 | `./orion-robot 66.5 63.25`（两个位置参数，无括号），缺省 (64.0, 64.0) |
| 2 | 位置记录方式 | RobotState 直接记录全局世界坐标 x/y，消费方直读零换算，更新在 RobotState 上进行 |
| 3 | origin 注入点 | STM32Device::spawn 的 local_state 初始化（唯一写入路径） |
| 4 | 初始化窗口 | launch 创建共享态后立即注入 origin，关闭 (0,0) 窗口 |
| 5 | main.rs 入口 | 传默认 origin (64.0, 64.0)，不做 CLI |
| 6 | Config 扩展 | 本次不改（Robot_Config 仅 ws_bind），留待后续 |
| 7 | 前端 Goto 默认值 (1,1) | 本次不改（语义为世界坐标，重构前后一致） |

---

## 人类评审

<!-- 在此区域写下评审意见 -->
