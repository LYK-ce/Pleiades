# Workbook — Task 13_1: Cluster Info

> 对应任务：`Task/task_13_1_cluster_info.md`
> 创建日期：2026-08-10

---

## 2026-08-10 任务创建（Task 13 阶段二拆分）

**背景**：Task 13 阶段一（sysid→peer_id）已实施（commit b7fa822）。阶段二讨论收敛为独立任务 task_13_1。

**讨论决策链**（人类逐项确认）：
1. 入站处理位置：**独立 task**（cluster_consumer），不放 main_loop——数据面/控制面解耦（main_loop 是实时控制链，未来避碰重逻辑不能拖累 auto_tick）
2. 目录：`Src/Robot/core/cluster/`（用户定名 cluster，弃 swarm——避免与 libp2p swarm 概念混淆）
3. 发布侧不挪进 cluster（发布是 state_notifier/slam_task 自己的事，gossipsub API 已够薄——YAGNI）
4. MAP_DELTA 挂起：人类指示后续走 **CRDT 地图重构**，阶段二不管
5. 身份澄清：peer_id 唯一身份（阶段一后 sysid=peer_id），peer_name 只是名字；"收到 POSE = 是车"（动态发现）；节点在线 ≠ robot 在线（位姿新鲜度更精确）
6. 离线车处理：用户否决双表/障碍表方案——**单表 + stale 保留**（"越搞越复杂"）；stale 语义留给消费端（断电车仍物理存在 = 按最后位置当障碍）
7. **规划级规避**（人类明确方向）：不做避碰状态机——寻路时把车当障碍注入 D*（= P0 障碍集合接口），改动最少
8. 意图广播：POSE 附带 subtarget（下一格）——multi_robot_control §5.2"意图广播"；**只做 1 格**（k 格不做）
9. ExecuteState 写者：**executor 写**（用户纠正：sub_target 是 executor 维护的，step 参数 &mut，结尾同步）
10. 命名：cluster_info.rs（弃 remote_info.rs）、ExecuteState（弃 IntentState）

**范围**：POSE 扩展(33B) + ExecuteState + cluster 模块（表/consumer）+ state_notifier 组帧 + main_loop 移除打印 + WS 下行同步。

**待人类拍板**：stale 超时（0.5s 起步）；WS 带 subtarget（建议带）。

## 依赖关系

- 前置：Task 13 阶段一（peer_id 身份，commit b7fa822）
- 后续：P0（pathfinder 障碍注入接口，task_13 阶段）依赖本任务的表
