# Task 13_1: Cluster Info（入站数据接入 + POSE 意图扩展）

> 状态：待实施（设计已与人类讨论收敛，2026-08-10）
> 创建日期：2026-08-10
> 最后更新：2026-08-10
> 父任务：`Task/task_13_multirobot_arch.md`（阶段二）
> 问题池：`Task/robot_review_problem.md`

## 目标

1. **入站数据接入**：gossipsub 入站 POSE 帧从"仅打印"升级为 `ClusterInfo` 表（按 peer_id 动态建表 + stale 超时），使"其他车"数据真正可消费
2. **POSE 意图扩展**：广播本车 subtarget（D* 寻路下一格），接收方存入表——为后续"寻路时把车当障碍注入"（P0）铺数据基础

## 背景

- Task 12 后入站数据链路已通：gossipsub → `robot_bus(StreamRaw)` → main_loop **仅打印**（robot.rs:341-354），无业务消费
- Task 13 阶段一（已实施，`b7fa822`）：帧内 sysid = 完整 peer_id——入站 decode 直接拿到身份，无需跨分支改 StreamRaw
- 多车方向设计：`docs/design_doc/multi_robot_control.md`（寻路把车当障碍 = 规划级规避，不做避碰状态机）

## 决策记录（2026-08-10 讨论收敛）

| 议题 | 决策 |
|---|---|
| 模块结构 | 新建 `Src/Robot/core/cluster/`（`mod.rs` + `consumer.rs` + `cluster_info.rs`） |
| 表模型 | **单表** `ClusterInfo`（RemoteRobotInfo），键 = 完整 peer_id；**stale 标记保留不删**；不做"离线车障碍表"（避免过度设计，语义留给消费端） |
| 表语义 | 活跃车 = 动态障碍；stale 车 = 按最后位置当障碍（断电车仍物理存在）——未来 P0 寻路注入时统一处理 |
| 意图状态 | `ExecuteState`（替代初议 IntentState）：`sub_target: Option<(i32,i32)>`，放 `state.rs` 与 RobotState/LidarState 并列 |
| 写者语义 | **executor 写**（sub_target 是 executor 维护的）：`step()` 增加 `&mut ExecuteState` 参数，结尾统一同步 `self.sub_target`；executor 不持有 Arc |
| 意图字段 | subtarget = D* 下一格（格坐标 i32×2）；**第一版只做 1 格**（k 格 + 时间窗留扩展字段，multi_robot_control §8 待决策 1 已定） |
| 无任务编码 | POSE payload 加 **valid 标志（1B）+ sub_gx(4B) + sub_gy(4B)**（+9B → 33B）；None 时 valid=0 |
| 身份/过滤 | peer_id 即身份（阶段一红利）；consumer 用本地 peer_id 比对过滤 gossipsub 自环回流 |
| 消费方 | 阶段二无显式消费方（先建表）；未来 executor/P0 注入 + TUI 显示 |
| 边界 | 全部改动在 `Src/Robot/` + `Src/WebSocket/`；**不碰** Network/EventBus（ML_review 职责） |
| 不做 | MAP_DELTA 入站处理（后续 CRDT 地图重构，人类已指示挂起）；pathfinder 障碍注入接口（P0，后续阶段） |

## 涉及文件与改法

| # | 文件 | 改动 |
|---|---|---|
| 1 | `Src/Robot/core/protocol/messages.rs` | `PoseData` 加 `valid: bool, sub_gx: i32, sub_gy: i32`；`encode_pose`/`decode_pose` 同步（24B → 33B）；roundtrip 测试更新 |
| 2 | `Src/Robot/core/state.rs` | 新增 `ExecuteState { sub_target: Option<(i32,i32)> }`（Default） |
| 3 | `Src/Robot/core/executor.rs` | `step()` 签名加 `intent_state: &mut ExecuteState`；结尾 `intent_state.sub_target = self.sub_target;` |
| 4 | `Src/Robot/core/cluster/mod.rs` | 模块声明 + re-export |
| 5 | `Src/Robot/core/cluster/cluster_info.rs` | `ClusterInfo` 结构（peer_id/x/y/yaw/vx/vy/time_boot_ms/last_seen/sub_target）+ 表 `ClusterInfoTable`（Arc<RwLock<HashMap<Vec<u8>, ClusterInfo>>>）+ `is_stale()`（0.5s 起步，常量可调）+ 单测 |
| 6 | `Src/Robot/core/cluster/consumer.rs` | `cluster_consumer` task：订阅 robot_bus → `decode_frame` → 过滤本车（本地 peer_id 比对）→ `decode_pose` → 写表 → debug 日志；CancellationToken 生命周期；单测（可测部分） |
| 7 | `Src/Robot/core/robot.rs` | ① `launch`：创建 `ExecuteState` + `ClusterInfoTable`，spawn `cluster_consumer`（同 state_notifier 模式）；② `state_notifier`：读 execute_state → POSE 帧带 valid+sub 字段；③ `main_loop`：**移除 robot_bus 订阅打印分支**（broadcast 双订阅：consumer 接管）；④ auto_tick：`executor.step(..., &mut *execute_state.write().await)`；⑤ `Pose` 广播结构加 sub_target（pose_tx 消费方 WS 同步） |
| 8 | `Src/WebSocket/server.rs` | pose 转发 `PoseData` 组装带 sub_target（WS 下行帧与 gossip 链路一致） |

**不受影响（已确认）**：`slam_task`、`pathfinder.rs`、`WebSocket/protocol.rs`（入站命令不解析 POSE）、`swarm_events.rs`/`command_handler.rs`（字节透传）。

## 协议变更（POSE 扩展，orion_protocol.md §3.1 同步）

```
POSE payload（msgid=1）旧：24B
  time_boot_ms(4) | x(4) | y(4) | vx(4) | vy(4) | yaw(4)

POSE payload（msgid=1）新：33B
  time_boot_ms(4) | x(4) | y(4) | vx(4) | vy(4) | yaw(4) | valid(1) | sub_gx(4) | sub_gy(4)
  └─ 意图广播（Task 13_1）：subtarget = D* 寻路下一格（格坐标）
     valid=0 → 无当前任务/无子目标，接收方忽略 sub 坐标
```

- `orion_protocol.md` §3.1：POSE 消息表加 3 字段（valid/sub_gx/sub_gy），注明意图语义（下一格，k 格留扩展）与 valid=0 约定
- `multi_robot_control.md` §8 待决策 1：标记已定（下一格）
- ⚠️ 协议版本不兼容（24B→33B，decode 长度校验拒绝旧帧）——整车/全端同步升级，双车联调时"旧帧被拒"属预期行为

## 实施步骤（顺序）

1. `messages.rs`：POSE 扩展 + 测试
2. `state.rs`：`ExecuteState`
3. `executor.rs`：step 参数化
4. `cluster/`：cluster_info.rs（表+stale+单测）→ consumer.rs → mod.rs
5. `robot.rs`：launch/spawn + state_notifier 组帧 + main_loop 移除打印 + auto_tick 传参
6. `server.rs`：WS 下行带 sub_target
7. `cargo check` + `cargo test --lib robot`（protocol + cluster）+ `cargo build --release --bin orion-robot`
8. 文档同步：`orion_protocol.md` §3.1 POSE 定义；`multi_robot_control.md` §8 待决策 1 标记已定；task_13 / wb_13 更新
9. git 提交（单 commit）

## 风险清单

1. **协议不兼容**：POSE payload 布局变化（24→33B），旧版二进制 decode 失败——整车升级，联调时注意版本一致
2. **broadcast 双订阅**：main_loop 打印分支必须移除，否则重复处理（tokio broadcast 是广播非竞争消费）
3. **subtarget None 编码**：valid=0 时接收方必须忽略 sub 坐标（约定好，测试覆盖）
4. **锁时序**：consumer 写表 vs 未来读——RwLock 粒度；auto_tick 增加 execute_state 写锁（与现有锁无交叉，无死锁风险）
5. **time_boot_ms 跨车不可比**：表内仅作数据标签，新鲜度一律用本地 `last_seen`（Instant）

## 待决策点

1. stale 超时时长：0.5s 起步（设计文档建议值），实车联调后按丢包情况调整
2. WS 下行带 subtarget：**建议带**（Pictor 可显示意图）——已并入实施计划，如有异议可改

---

## 人类评审

<!-- 在此区域写下评审意见 -->
