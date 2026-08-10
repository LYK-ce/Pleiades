# Task 11: Robot Update

> 状态：进行中——通信协议统一方向已明确（2026-08-07 讨论），A~E 待确认
> 创建日期：2026-08-07
> 最后更新：2026-08-08

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

> 问题清单已统一迁移至 `Task/robot_review_problem.md`（唯一问题池，2026-08-08 建立），本文件不再重复维护清单。
>
> A~E 分组对应：
> - A. 导航可靠性 → robot_review_problem.md 第一节（P1/P3/P6/P7/N10/N8）
> - B. 性能完善 → 第二节（N2/P2#4/N1/P3#9/P3#10）
> - C. 健壮性完善 → 第三节（P1#3/P2#5/N5/N6/P3#13）
> - D. API 完整性 → 第四节（N3/N9）
> - E. 规范/卫生 → 第五节（N4/核查 P3-1~4/P3#11/N7）
> - 前端/文档断链（2026-08-08 梳理新增）→ 第六节
> - 已解决追溯 → 第七节
>
> 后续问题管理、状态更新一律在 `robot_review_problem.md` 中进行。
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
