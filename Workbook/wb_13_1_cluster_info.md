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

**已拍板（2026-08-10）**：① 超时处理**不做**——超时就超时，不删除/标记/淘汰，last_seen 仅记录供未来消费端；② WS 下行**带** subtarget（Pictor 可显示意图）。

## 依赖关系

- 前置：Task 13 阶段一（peer_id 身份，commit b7fa822）
- 后续：P0（pathfinder 障碍注入接口，task_13 阶段）依赖本任务的表

---

## 2026-08-10 实施完成 ✅

**改动 9 文件**：
- `protocol/messages.rs`：PoseData +valid/sub_gx/sub_gy，encode/decode 24B→33B，测试更新
- `core/state.rs`：ExecuteState（sub_target）
- `core/executor.rs`：step 拆 wrapper+step_impl，签名 +&mut ExecuteState，结尾统一同步 sub_target
- `core/cluster/`（新）：mod.rs + cluster_info.rs（ClusterInfo/ClusterInfoTable 单表，无超时逻辑）+ consumer.rs（cluster_consumer：robot_bus→decode→本车过滤→写表；5 单测）
- `core/robot.rs`：launch 建 execute_state+cluster_table、spawn cluster_consumer（步骤7）；state_notifier 读 execute_state 组帧（Pose 带 sub_target + PoseData valid/sub）；main_loop 移除 robot_bus 订阅打印（双订阅问题解决）+ auto_tick 传 execute_state；Pose 结构 +sub_target；Robot 结构 +cluster_table
- `WebSocket/server.rs`：pose 下行帧带 valid/sub（Pictor 可显示意图）

**验证**：cargo check ✅ / cargo test --lib robot 55/55 ✅（+8 新测试）/ orion-robot release ✅

**文档**：orion_protocol.md §3.1（POSE 33B 布局 + 意图语义）、multi_robot_control.md §8-1 标记已定、task_13_1 状态✅

**遗留**：双车联调（对方位姿+意图入库验证）；P0 寻路障碍注入（消费 cluster_table）
