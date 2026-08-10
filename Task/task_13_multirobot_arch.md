# Task 13: Multi-Robot Control（多车协同控制架构落地）

> 状态：待启动——设计已定稿（`docs/design_doc/multi_robot_control.md`），实施范围/优先级待人类确认
> 创建日期：2026-08-10
> 最后更新：2026-08-10
> 设计文档：`docs/design_doc/multi_robot_control.md`（2026-08-10 三轮调研收敛，本任务唯一设计依据）
> 问题池：`Task/robot_review_problem.md`（相关前置问题：P1 动态障碍、N1/N2/P2#4 性能组）

## 目标

按 `multi_robot_control.md` 实施多车协同控制：任务分配（确定性分配 + 棋盘/环形散布）、协同行动（多车避碰 + 让行）、环境一致性（车与静态障碍区分）。承接 Task 10（入站 Robot 信息处理）与 Task 11（Robot Update 中与多车相关的完善方向）。

## 背景

- 单车阶段已闭环：SLAM / D* Lite / 走停控制 / gossipsub 广播（Task 12 完成）
- 2026-08-10 三轮子 agent 调研收敛出设计文档（MAPF/move-wait 路线，非连续速度避碰）
- **设计文档与代码现状存在 4 个断层**（2026-08-10 梳理确认），为本任务的前置条件：
  1. DStarLite 无动态障碍增删接口（= 问题池 P1 症状：`mark_obstacle` 不置 ∞，无 `obstacles: HashSet`）
  2. 入站其他车位姿无存储/消费路径（main_loop 仅 `info!` 打印，Task 10 未启动）
  3. gossip map topic 只有 MAP_DELTA 增量，SnapshotCache 快照=最近一次 delta，新车无法重建完整地图（map_full 仅走 WS 单车链路）
  4. 入站消息丢失 author（`Bus_Event::StreamRaw` 仅透传 payload，peer_id 被丢弃；sysid 末字节 1/256 碰撞）

## 已确认决策（源自设计文档 §3~§6）

| 维度 | 决策 |
|---|---|
| 架构 | 三层模型：意图层（终端/Pictor 下发）→ 分配层（每车自算，零协商）→ 执行层（D* Lite + 走/停 + 让行） |
| 分配 | members 按 peer_id 排序取序号 i → 形态模板位置列表[i]；members 快照保证一致性 |
| 形态 | Goto=棋盘同色格 `(gx+gy)%2==0`；围堵=环形均匀分布 θ=i/N·2π（第一版到达即停） |
| 冲突检测 | 连续几何（圆-圆/圆-线段扫掠 + 5s 时间窗），不定义"车在哪个格" |
| 优先级 | `(ETA, robot_id)` 字典序，小者先，**永不双向让**（根治互让死锁） |
| 规避状态机 | NORMAL → DECIDING(≤200ms) → DETOUR(代价比≤1.3) / WAIT(5s) → ESCALATE(15s, 等待图+aging)；EMERGENCY_STOP（LiDAR 3 帧去抖）兜底 |
| 绕行 | D* Lite 重算，动态障碍=额外障碍集合传入（**需新增接口**，非"零改动"） |
| 车 vs 静态障碍 | 点云过滤（车 footprint 内点丢弃，不进地图）+ 通信超时保护（0.5s 未收到→保守停车）；过滤参数待实车实测 |
| 非目标 | 编队行进、连续速度避碰（ORCA）、中心化调度、跨车时间同步 |

## 实施路线（对应设计文档 §7 P1~P6）

| 阶段 | 内容 | 前置依赖 |
|---|---|---|
| **P0** | DStarLite 动态障碍接口（`add_dynamic_obstacle`/`remove`）+ 单元测试（问题池 N10 的 17 场景一并落地） | 问题池 P1 修复 |
| **P1** | 任务消息协议（命令/目标/形态/members）+ `pleiades/robot/task` topic + 确定性分配（序号+形态取位） | gossip task topic（新增） |
| **P2** | Goto 棋盘散布 + 围堵环形散布（到达即停） | P1 |
| **P3** | 位姿→动态障碍格（1-4 格保守标记）→ 重规划 + 1Hz 重规划节流 | P0 |
| **P4** | 入站位姿存储/消费（RemoteRobotInfo 按 peer_id）+ 冲突检测（连续几何）+ 让行闸门 + 规避状态机 | P3 + 断层②（Task 10 方向） |
| **P5** | 点云过滤（车不进地图）+ 通信超时保护 | 实车实测雷达数据 |
| **P6** | 僵持协商（waiting_for 等待图 + aging）+ 超时上报 | P4 |

## 文件计划（草案，待确认）

```
Src/Robot/slam/pathfinder.rs            ← [修改] P0：动态障碍接口 + 17 场景单测（承接问题池 P1/P3/N10）
Src/Robot/core/protocol/messages.rs     ← [修改] P1：ORION_TASK_SET(5) 扩展（命令/目标/形态/members）
Src/Robot/core/mission.rs               ← [修改] P1：集群任务/形态模板
Src/Robot/core/command.rs               ← [修改] P1：AutoCmd 集群扩展
Src/Robot/core/robot.rs                 ← [修改] P1/P4：task topic 收发 + RemoteRobotInfo 存储 + 规避状态机
Src/Robot/core/executor.rs              ← [修改] P4：让行闸门
Src/Robot/slam/lidar_mapper.rs          ← [修改] P5：点云过滤
Src/Robot/slam/grid.rs                  ← [修改] P4：动态障碍格标记（如需）
Src/Network/Gossipsub/mod.rs            ← [修改] P1：+TOPIC_ROBOT_TASK 常量（分支边界内增量，参照 Task 12）
Src/Network/swarm_events.rs             ← [修改] P1/P4：task topic 分支；author 透传（需批准，见分支边界）
Src/Network/mod.rs                      ← [修改] P1：re-export
```

## 分支边界说明

- gossip/identify 基础设施归 ML_review：本任务仅**消费**（新增 topic 常量/订阅/分支），不动核心设计——参照 Task 12 既定模式；若发现基础设施缺陷，回写 ML_review 修复后 merge
- `Bus_Event::StreamRaw` 变体扩展（携带 author/peer_id）涉及 EventBus 接口（ML_review 职责），**需人类批准**后方可触碰；备选方案：gossip topic 分支内 peer_id 不入 robot_bus，改用独立 `Bus_Event` 变体或让 robot_bus 消息结构携带 peer_id（同样跨接口）

## 待决策问题

1. 意图广播范围：第一版"下一格" vs "前方 k 格 + 时间窗"（设计文档 §8-1）
2. 围堵半径参数来源（§8-2）；围堵"动态跟随"是否纳入本次（设计 §4.4 留白，第一版到达即停对移动目标基本无效）
3. 点云过滤是否需要（§8-3，取决于实车雷达实测）
4. task topic 是否新增 `pleiades/robot/task`（§8-4，推荐新增）
5. lane_dir 单行化是否预留 config 字段（§8-5）
6. **跨车时间基准**：5s 预测窗口 vs 无跨车时钟——是否接受"本地接收时刻近似"方案
7. **ETA 估算**：优先级 `(ETA, robot_id)` 的 ETA 从何而来（对方速度/路径信息不足）；是否降级为"纯 robot_id + aging"
8. 任务幂等：gossipsub at-least-once 可能重复投递任务，去重/幂等策略（msgid 复用？）
9. 断层③ map 全量同步：gossip 周期 map_full 是否纳入本次（环境一致性目标 vs 范围控制）

## 实施步骤（草案）

1. 确认范围与优先级（P0~P6 哪些纳入本次；待决策 1~9 拍板）
2. P0：DStarLite 动态障碍接口 + 单测（独立提交，问题池 P1/P3/N10 闭环）
3. P1：任务协议 + task topic + 确定性分配
4. P2：散布形态
5. P3：动态障碍注入 + 节流
6. P4：入站消费 + 冲突检测 + 规避状态机
7. P5/P6：点云过滤 + 僵持协商
8. 编译检查 + 回归 + 单元测试
9. 双车联调 / 实车验证

## 验证计划

1. `./build.sh check` + `cargo build --release --bin orion-robot` 编译通过
2. 单测：分配确定性（members 排序/形态取位一致）、D* 动态障碍 17 场景回归、frame/messages 编解码
3. 双车联调（沿用 Task 12 未完成的联调项）：task 广播互通、位姿 10Hz、避碰行为（一车停一车绕）
4. 实车验证：棋盘散布、围堵、让行闸门、LiDAR 急停兜底
5. 日志验证：无刷屏、无 RR 超时 error

---

## 人类评审

<!-- 在此区域写下评审意见 -->
