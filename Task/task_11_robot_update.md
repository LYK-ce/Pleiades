# Task 11: Robot Update

> 状态：进行中——通信协议统一方向已明确（2026-08-07 讨论），A~E 待确认
> 创建日期：2026-08-07
> 最后更新：2026-08-07

## 目标

对当前 Robot 模块的设计进行完善。范围涵盖 Task 8 / Task 10 遗留问题 + wb_10 核查新发现的设计缺陷（导航可靠性、性能、健壮性、API 完整性、规范卫生）。

**当前优先方向：通信协议统一**（MAVLink 风格帧 + 自定义消息，面向多车集群场景）——见下文"协议统一"章节。

（具体范围与优先级以人类确认为准）

## 背景

Robot 模块已完成基础闭环（STM32 驱动 / LiDAR / SLAM / D* Lite 导航 / WS 遥控），2026-08-05 实车验证通过。但历次核查累计了一批设计层面待完善项，分布于：

- Task 8（D* Lite）遗留待办（wb_8）
- wb_10 核查新发现 N1~N10
- Task 10 承接问题清单（P1#3 / P2#4~6 / P3#9~13）

## 协议统一（当前优先方向，2026-08-07 讨论明确）

### 背景

- 单车时代：车↔控制终端 WebSocket，JSON/bin 混用可接受（收发两端均自研）
- 多车时代：车↔车（libp2p `DataType::Robot` 散装 JSON）+ 车↔终端（WS）**两套协议并存**，字段风格不一、map_full 无集群二进制通道（`from_utf8_lossy` 会损坏）
- 比赛背景（大规模无人集群联合多域防控，9/15 提交）：统一接入、统一编组、统一调度是赛题核心要求
- 完整设计见 `docs/design_doc/orion_protocol.md`

### 已明确决策（2026-08-07）

| 议题 | 决策 |
|---|---|
| 传输层 | libp2p 统一（车↔车、车↔控制终端）；WS 过渡期保留（含 hello），Pictor 迁移后退役 |
| 帧格式 | MAVLink 风格：`magic 0x4F + len(u32，放宽 255B 上限) + seq + sysid + compid + msgid(u16) + payload + CRC16` |
| 消息集 | 5 条全自定义：`ORION_POSE(1)` / `ORION_MAP_FULL(2)` / `ORION_MAP_DELTA(3)` / `ORION_MANUAL_CONTROL(4)` / `ORION_TASK_SET(5)` |
| sysid | peer_id multihash 末字节（确定性、零配置、不参与路由）；compid 1=主控 / 200=终端 |
| 坐标系 | 全局世界坐标（x 东 y 南，原点 (0,0)，初值 origin）；yaw 弧度**顺时针为正**（⚠️ 踩坑记录 `890e2fc`）；POSE 直接映射 RobotState 无变换 |
| 地图状态 | 内部三态统一改 **0/100/255**（MAVLink 惯例，`[repr(i8)]` 枚举），full/delta 编码一致、零映射 |
| 任务 | `ORION_TASK_SET` 整体替换语义：替换队列、立即中断当前任务、count=0=取消；替代现有 `AutoCmd::Push` 追加语义 |
| 时间戳 | `time_boot_ms`（开机起算毫秒，替代 `Pose.ts` unix 秒） |
| 不做 | 心跳（libp2p 存活检测已覆盖）、命令应答、MISSION/PARAM 协议、标准 MAVLink 设备互操作 |

### 实施前置（已定）

- ✅ `Bus_Event` 新增 `StreamRaw { payload: Vec<u8> }` 变体——robot_bus 二进制承载（前置依赖，4 处改动：event.rs / swarm_events.rs / robot.rs / TUI match）
- ✅ 内部状态改 0/100/255——`CellState` 枚举（`[repr(i8)]`）+ `pathfinder.rs:170` 硬编码 `Some(1)` 同步改
- ✅ ORION_MAP_FULL 编码重写（Pictor 专有布局不复用，独立编码器）
- ✅ 新模块结构：`Src/Robot/core/protocol/`（frame.rs / messages.rs / sysid.rs）
- ✅ seq / CRC16：字段保留、第一版恒填 0（libp2p/TCP 可靠传输，不实现）
- ✅ 时间戳改 boot ms（MAVLink 惯例）：新增 `now_boot_ms()` 辅助函数（开机基准 Instant），`Pose.ts` f64 → u32；Pictor 显示端同步（不再按 unix 秒格式化）；语义：本机单调时间，不做跨车比较
- ✅ 任务替换语义落地（方案 B）：`AutoCmd::Push` → **`AutoCmd::Set(Vec<Mission>)`**（整体替换），`Cancel` 删除（`Set([])` 等价取消）；`MissionQueue` 加 `replace()`；main_loop 分支 = `stm32.stop() + executor.reset() + queue.replace(list)`；**executor 本身不改**（只 pop 队列，替换透明）；循环执行（Patrol）留待 Mission 类型扩展


### 暂缓/后续改进（非本次实施，多车集群阶段处理）

- ⏳ 传输层广播机制：`Broadcast` 现为 request_response 模拟（逐 peer 请求无回执 → 超时日志噪音）；建议多车阶段改用 **gossipsub**（Network 模块范畴，ML_review 分支）
- ⏳ 位姿广播 10Hz 节流：带宽非瓶颈（10Hz × N 车 ≈ KB/s 级），第一版保持 10Hz；多车联调时若噪音/带宽成问题再处理
- ⏳ 集群 map_full 通道：ConnectionEstablished 触发点，待集群多车阶段实现
## 完善方向候选清单

### A. 导航可靠性（Task 8 遗留）

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| P1 | `pathfinder.rs` mark_obstacle | 未强制置 ∞，只重读概率栅格——需 4 次 LiDAR 命中才 Occupied，动态障碍 D* 不知情，急停后仍可能反复撞 | mark_obstacle 直接置 cost=∞（或同步写 grid） |
| P3 | `pathfinder.rs` compute_shortest_path | 无迭代上限（无 watchdog），极端地图拖垮 50ms auto_tick | 加迭代上限/时间预算，超限降级 |
| P6 | `executor.rs` | goal 格为 Occupied 时任务永不完成也不失败 | 目标格不可达时明确失败并报错 |
| P7 | `executor.rs` | D* 规划失败仅 warn! | 提升为 error! |
| N10 | `pathfinder.rs` | 零单元测试（17 场景清单已在 wb_8 给出，未落地） | 补全单元测试 |

### B. 性能完善

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| N2 | `core/robot.rs` auto_tick | 每 50ms 全量 clone 三态（grid 65KB+） | 按需 clone / 增量 |
| P2#4 | `core/robot.rs` slam_task | grid 写锁内做 JSON 序列化 + broadcast | 锁内只取 deltas，锁外组 JSON |
| P3#9 | `core/robot.rs` notifier/slam_task | `Get_Local_Peer_Id().to_string()` 每 100/200ms 分配一次 | launch 时算一次传闭包 |
| P3#10 | `core/robot.rs` | `typed.clone()` 两次分配 | 复用一次 |

### C. 健壮性完善

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| P1#3 | `core/robot.rs` 广播失败 | warn! 无退避——弱网下 10Hz warn 风暴 | 连续失败降频 debug!，成功复位 |
| P2#5 | `core/robot.rs` recv_robot_event | `Closed => None` 隐性忙循环 | 注释注明依赖 / Closed 返回哨兵 break |
| P2#6 | `Network/swarm_events.rs:199` | 入站 Robot payload 走 `from_utf8_lossy`——未来二进制 map 会被损坏 | 文档注明 robot_bus 仅 UTF-8 JSON；二进制另走通道 |
| N5 | `WebSocket/` | WS send 静默忽略失败 | 失败时降级/断开处理 |
| N6 | Robot 模块 | 日志级别不一致 | 统一规范 |

### D. API 完整性

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| N3 | `Src/VM/capability_binding.rs:755` | `register_robot_caps` 空 stub 且无调用点——`robot_test.lua` 期望的 `robot.open/forward/stop/beep/get_state` 均不存在 | 实现基于 STM32Device 的 Lua 绑定（遵循 Lua 绑定规范：闭包薄胶水） |
| N9 | `lib.rs` | Robot 模块无 re-export | 补齐（若按项目惯例需要） |

### E. 规范/卫生

| # | 位置 | 问题 |
|---|---|---|
| N4 | `executor.rs` | 双重转向日志 |
| N7 | `slam/grid.rs` | build_map_full info! 刷屏 |
| N8 | 实车 | 路径偏差（待分析） |
| P3#11 | `lib.rs` | 文件头无 Modified Date |
| 核查 P3-1~4 | 多处 | 重复注释、编号重复、Modified Date 未 bump、注释过时 |

## 待决策问题

1. 完善范围与优先级：A~E 哪些纳入本次？是否全做？
2. N3（Robot Lua 绑定）是否本次实现？若实现，API 形态以 `robot_test.lua` 为准还是重新设计？
3. N8 路径偏差是否已有实车数据需要分析？
4. 其他人类期望的设计完善方向（补充）？

## 文件计划（草案，待确认）

```
Src/Robot/slam/pathfinder.rs      ← [修改] P1/P3 + N10 测试
Src/Robot/core/executor.rs        ← [修改] P6/P7/N4
Src/Robot/core/robot.rs           ← [修改] N2/P2#4/P3#9/P3#10/P1#3/P2#5
Src/Robot/state.rs                ← [修改] 视 N2 方案而定
Src/VM/capability_binding.rs      ← [修改] N3（若纳入）
Src/WebSocket/                    ← [修改] N5
Network/swarm_events.rs           ← [修改] P2#6（仅文档/注释，若允许）
lib.rs / bootstrap.rs             ← [修改] N9/P3#11/P3#13
```

## 实现步骤（草案）

1. 确认范围与优先级
2. 分项实现（导航 → 性能 → 健壮性 → API → 卫生）
3. 单元测试补全（pathfinder 17 场景等）
4. 编译检查 + 回归
5. （可选）实车验证

---

## 人类评审

<!-- 在此区域写下评审意见 -->
