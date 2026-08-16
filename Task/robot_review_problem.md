# Review Problems — 遗留问题清单

> 用途：集中记录 Robot 模块（Pleiades-Orion 分支）全部遗留问题，作为后续任务（task_11 完善 / 新任务）的统一问题池
> 创建日期：2026-08-08
> 来源：task_8 / task_10 / task_11 / wb_8 / wb_10 核查记录 + 2026-08-08 代码全面梳理（子 agent）
> 状态说明：✅ = 已解决（保留追溯）；🔴 = 严重；🟠 = 中等；🟡 = 轻微

---

## 一、导航可靠性（task_11 完善方向 A，源自 task_8 遗留 / wb_8）

| # | 位置 | 问题 | 建议 | 来源 |
|---|---|---|---|---|
| 🔴 P1 | `Src/Robot/core/planning/pathfinder.rs` `mark_obstacle` | 未强制置 ∞，仅重读概率栅格——需 4 次 LiDAR 命中才 Occupied，动态障碍 D* 不知情，急停后仍可能反复撞 | mark_obstacle 直接置 cost=∞（DStarLite 内部 `obstacles: HashSet`，cost() 先查集合）或同步写 grid | task_8 / wb_8 待办 #2 |
| 🟠 P3 | `Src/Robot/core/planning/pathfinder.rs` `compute_shortest_path` | 无迭代上限/watchdog，极端地图可拖垮 50ms auto_tick | 加 MAX_ITERS 迭代上限/时间预算，超限降级 | task_8 / wb_8 待办 #3 |
| 🟠 P6 | `Src/Robot/core/executor.rs` | goal 格为 Occupied 时任务永不完成也不失败 | 目标格不可达时明确失败并报错 | task_8 / wb_8 待办 #5 |
| 🟠 P7 | `Src/Robot/core/executor.rs` | D* 规划失败仅 warn!（task_8 Q6 要求 error!） | 提升为 error! | task_8 / wb_8 待办 #5 |
| 🟠 N10 | `Src/Robot/core/planning/pathfinder.rs` | 零单元测试（wb_8 已给出 17 场景清单，未落地） | 补全单元测试 | wb_10 新发现 / task_11 A 组 |
| 🟡 N8 | 实车 | 路径偏差（待实车数据分析定位） | 收集数据后分析 | wb_10 新发现 |
| 🟠 N13 | `Src/Robot/core/planning/pathfinder.rs`（本车点机器人假设） | D* Lite 将本车当点、未处理本车 15cm footprint；他车障碍膨胀半径 < 30cm（对方 15 + 本车 15）时存在残余重叠碰撞（task_17 定稿 20cm 膨胀 → 最坏 10cm 重叠） | 可选升级：本车 footprint 感知规划（连续坐标检查 15cm 圆盘碰撞），或膨胀半径加到 30cm；先实车联调验证 10cm 残余是否真的碰撞 | task_17 讨论 / 2026-08-16 |

## 二、性能（task_11 完善方向 B，源自 task_10 承接 + wb_10 新发现）

| # | 位置 | 问题 | 建议 | 来源 |
|---|---|---|---|---|
| 🟠 N2 | `Src/Robot/core/robot.rs` auto_tick | 每 50ms 全量 clone 三态（grid 65KB+） | 按需 clone / 增量 | wb_10 新发现 |
| 🟠 P2#4 | `Src/Robot/core/robot.rs` slam_task | grid 写锁内做 JSON 序列化 + broadcast（锁持有时间非最短） | 锁内只取 deltas，锁外组 JSON | task_10 承接 |
| 🟠 N1 | `Src/Robot/core/robot.rs` state_notifier | 读锁贯穿组包（P2#4 同类问题） | 同 P2#4：锁内只取数据，锁外组包 | wb_10 新发现 |
| 🟡 P3#9 | `Src/Robot/core/robot.rs` notifier/slam_task | `Get_Local_Peer_Id().to_string()` 每 100/200ms 各分配一次（peer id 不变） | launch 时算一次传闭包 | task_10 承接 |
| 🟡 P3#10 | `Src/Robot/core/robot.rs` | `typed.clone()` 给 map_tx 后又 iter 组 JSON——两次分配 | 复用一次 | task_10 承接 |

## 三、健壮性（task_11 完善方向 C）

| # | 位置 | 问题 | 建议 | 来源 |
|---|---|---|---|---|
| 🟠 P1#3 | `Src/Robot/core/robot.rs` state_notifier/slam_task 广播失败 | 广播失败 warn! 无退避——弱网下 cmd_tx 持续满 → 10Hz warn 风暴 | 连续失败 N 次降频 debug! / 每 5s 一次，成功复位 | task_10 承接 |
| 🟠 P2#5 | `Src/Robot/core/robot.rs` recv_robot_event | `Closed => None` 若 sender 全 drop 会忙循环（当前 main_loop 自持 Arc 保活，实际不可达——隐性陷阱） | 注释注明依赖，或 Closed 返回哨兵让 main_loop break | task_10 承接 |
| 🟠 N5 | `Src/WebSocket/server.rs` | WS send 多处 `let _ =` 静默忽略失败；断开才发 Stop，弱网降级无处理 | 失败时降级/断开处理 | wb_10 新发现 |
| 🟡 N6 | `Src/Robot/core/robot.rs` dispatch | `StopLidarScan` 错误用 info! 而非 warn!（日志级别不一致） | 统一规范 | wb_10 新发现 |
| 🟡 P3#13 | `Src/bootstrap.rs` robot_bootstrap | info! 只打 lidar_port 不打 lidar_baudrate | 补字段 | task_10 承接 |

## 四、API 完整性（task_11 完善方向 D）

| # | 位置 | 问题 | 建议 | 来源 |
|---|---|---|---|---|
| 🔴 N3 | `Src/VM/capability_binding.rs:755` | `register_robot_caps` 空 stub 且无调用点（死代码）——`programs/user/robot_test.lua` 期望的 `robot.open/forward/stop/beep/get_state` 均不存在 | 实现基于 STM32Device 的 Lua 绑定（遵循 Lua 绑定规范：业务逻辑独立 fn，闭包薄胶水） | wb_10 新发现 / task_11 D 组 |
| 🟠 N9 | `Src/lib.rs:60-61` | `pub mod robot;` 有声明但无 `pub use` re-export（其他模块均有） | 补齐 re-export | wb_10 新发现 |

## 五、规范/卫生（task_11 完善方向 E）

| # | 位置 | 问题 | 建议 | 来源 |
|---|---|---|---|---|
| 🟡 N4 | `Src/Robot/core/executor.rs` | 双重转向日志（235/239 行） | 删冗余 | wb_10 新发现 |
| 🟡 核查 P3-1 | `Src/Robot/core/executor.rs:110-111` | 重复注释 | 删冗余 | task_10 承接 |
| 🟡 核查 P3-2 | `Src/Robot/core/robot.rs` launch | `// 4. spawn STM32` 与 `// 4. spawn LiDAR` 编号重复 | 改编号 | task_10 承接 |
| 🟡 核查 P3-3 | `Src/Robot/core/state.rs` / `control/device/stm32/mod.rs` / `core/executor.rs` | 文件头 Modified Date 未 bump 至 2026-08-06 | 更新 | task_10 承接 |
| 🟡 核查 P3-4 | `Src/main_robot.rs:65` | 注释仍写 "TUI/CLI + Core"，CLI 已移除 | 更新注释 | task_10 承接 |
| 🟡 P3#11 | `Src/lib.rs` | 文件头无 Modified Date | 补齐 | task_10 承接 |
| 🟡 N7 | `Src/Robot/slam/grid.rs` | build_map_full info! 刷屏 | 降级/删（注：build_map_full 已被 `Chunk::state_bytes()` 取代，问题可能已消失，需复核） | wb_10 新发现 |

## 六、前端 / 文档断链（2026-08-08 代码梳理新发现）

| # | 位置 | 问题 | 建议 | 来源 |
|---|---|---|---|---|
| 🔴 前端断链 | `Tool/robot_control.html` | **已废弃**：仍发旧 JSON（`{cmd:'manual',...}`/`{cmd:'auto',action:'push'}`）并期待 JSON `type:'pose'` 遥测；WS 已迁移到 ORION 二进制帧，文本消息被忽略、二进制帧 `JSON.parse` 失败被吞。`orion_protocol.md` §1.5 明确"已废弃，主力地面站为 Pictor" | 同步升级到 ORION 协议 或 正式废弃并清理 | 2026-08-08 梳理 |
| 🟠 文档过时 | `Architecture/Pleiades_Architecture.md` §3.10 | 最后更新 07-21，Robot 模块描述过时（目录结构 / Robot 8 字段 / Command 三层 / launch 签名 / spawn_robot_ws_server 均与现状不符；实际为 `WebSocket::start` + `bootstrap::robot_bootstrap`，且无 websocket.rs） | 更新架构文档 §3.10 | 2026-08-08 梳理 |
| 🟠 文档过时 | `docs/design_doc/robot_controller.md` | 设计草案过时：`AutoCmd::Push/Cancel`（现 Set）、`ModeCmd::Pause/Resume`（未实现）、WS JSON 协议（已换 ORION）、`DStarLite::set_goal/update_cell`（现 new/move_to/mark_obstacle） | 更新或废弃 | 2026-08-08 梳理 |
| 🟡 文档未完成 | `docs/design_doc/orion_protocol.md` §6 | 待补充章节（Pictor 接入、libp2p 投递方式）未完成 | 后续补充 | 2026-08-08 梳理 |
| 🟡 注释过时 | `Src/Robot/core/executor.rs` step() | `// ① 感知` 重复注释（与 P3-1 同类，确认是否同一处） | 删冗余 | 2026-08-08 梳理 |

## 七、基础设施（libp2p 标准件缺失，2026-08-08 task_12 讨论新增）

| # | 位置 | 问题 | 建议 | 来源 |
|---|---|---|---|---|
| 🔴 N11 | `Src/Network/network_service.rs` behaviour 配置 | **identify 缺失**：libp2p 栈未启用 identify（节点自我介绍/协议能力协商标准件），自研 Info 交换替代（ConnectionEstablished 时逐 peer `send_data(Info)` 等待回执，swarm_events.rs L69-84）——无法自动协商协议能力；gossipsub 上线后新老版本节点混合时，节点需知道对方支持哪些协议；identify 零配置、连接建立即自动交换 | `PleiadesNetworkBehaviour` 增 `identify::Behaviour`；与自定义 Info 交换**共存**：identify 管协议层协商，Info 管业务字段（能力/模型分片） | 2026-08-08 讨论 |
| 🟠 N12 | `Cargo.toml` libp2p features | **gossipsub 缺失**（广播依赖 request-response 模拟，详见 task_12） | features 加 `"gossipsub"`，按 task_12 实施 | 2026-08-08 讨论 |
## 八、已解决 ✅（保留追溯，避免重复处理）

| # | 位置 | 说明 | 解决方式 |
|---|---|---|---|
| ✅ P2#6 | `Network/swarm_events.rs:199` | 入站 Robot payload 走 `from_utf8_lossy`，二进制会被损坏 | task_11 前置改造：`Bus_Event::StreamRaw` 二进制通道（event.rs / swarm_events.rs / robot.rs / TUI 4 处改动） |
| ✅ N7 | `Src/Robot/slam/grid.rs` | build_map_full info! 刷屏 | `build_map_full` 已被 `Chunk::state_bytes()` 取代（待复核确认是否还需处理，见第五节） |
| ✅ 文档偏差-1 | task_9_2 签名草案 | robot_bootstrap 草案含 peer_name 参数，实际内部读取 | 已消失（代码与文档已一致） |
| ✅ 文档偏差-2 | task_9 main.rs:125 | launch 条目已被 Robot 移除取代 | 已消失 |

---

## 附：来源索引

- **task_8 / wb_8**：D* Lite 路径规划器（已归档 2026-08-08）→ 遗留 P1/P3/P6/P7/N10 转入本清单第一节
- **task_10 / wb_10**：Robot Info Handle（待启动，目标待人类补充）→ 承接 Task 9 系列问题清单 + N1~N10 新发现
- **task_11**：Robot Update（进行中，A~E 完善方向待人类确认）→ 本清单按 A~E 分组对应
- **2026-08-08 梳理**：子 agent 深度代码梳理新增（前端断链 / 文档过时）

> 待决策问题（来自 task_11，未在本清单内）：①完善范围与优先级 A~E ②N3 Lua 绑定 API 形态 ③N8 路径偏差数据 ④Task 10 目标范围（入站信息存储/展示/融合）
