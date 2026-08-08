# Task 12: Robot 广播接入 GossipSub（request-response → topic 广播）

> 状态：**定稿待实施**——方案已确认（2026-08-08 讨论收敛），ML_review 基础设施已 merge
> 创建日期：2026-08-08
> 最后更新：2026-08-08

## 目标

将小车之间（车↔车集群）的 Robot 消息广播从 **request-response 模拟**（`NodeHandle::Broadcast` 逐 peer 发送）改为 **gossipsub topic 广播**，降低广播性能开销，消除逐 peer 请求无回执导致的超时日志噪音。

## 背景

- 旧路径：`robot.rs` state_notifier（100ms 位姿）/ slam_task（200ms 地图）→ `nh.Broadcast(DataType::Robot, ORION帧)` → `command_handler.rs` 逐 peer `send_request` → 对端回 "OK" 闭合状态机
- 痛点：逐 peer 克隆 payload（O(N)）、RR 往返状态机、"OK" 回执、300s 超时风暴、双端 info 日志刷屏
- **基础设施已就绪**：ML_review 已 merge 两次（`bd0ec43` Task 18：identify + gossipsub 三 topic；`387ad72` Task 20：发送侧解耦 + SnapshotCache 快照机制）——本任务只需"接 robot topic"，不需新建 gossip 基础设施

## 已确认决策（2026-08-08 讨论）

| 议题 | 决策 |
|---|---|
| 协议 | gossipsub 一步到位（libp2p 0.56 内置，已加 `"gossipsub"` feature） |
| 定位 | 消息传播层（topic 广播 + mesh 中继）。**分工不替代**：广播类走 gossip；点对点请求类（命令应答/张量流/带宽测速）仍走 request-response / stream |
| **topic 拆分** | **拆 2 个**：`pleiades/robot/pose`（位姿 10Hz 高频流）+ `pleiades/robot/map`（地图增量/全量，状态快照）。**task topic 暂不拆**（多车任务协同需求出现后再加，如 `pleiades/robot/task`） |
| 拆分理由 | 订阅过滤（消费方按需订阅）+ SnapshotCache 快照语义精准（map 是状态吃快照红利，pose 是纯流不背负担）+ 大小/频率隔离（未来 map 全量可能超 1MiB 需单独调 max_transmit_size） |
| msgid 双保险 | ORION 帧内 msgid（POSE=1/MAP_FULL=2/MAP_DELTA=3）**保留**——topic 是网络层分流，msgid 是协议层分流，两层并存 |
| 不入 gossip | **MANUAL_CONTROL（遥控）不走 gossip**：终端→车点对点命令，走 WebSocket 本地链路（避免任何车能遥控任何车） |
| 消息签名 | `MessageAuthenticity::Signed`（ML_review 已配）——ORION 二进制 payload 零污染透传（签名是独立字段，`message.data` 即原字节） |
| 节点发现 | mDNS（发现）→ identify（识别/协议协商）→ gossipsub（业务广播），三层职责已由 ML_review 落地 |
| 心跳 | gossipsub heartbeat 为协议内在机制，保持默认；libp2p ping（60s）负责连接存活，两者并行 |
| 重复投递 | at-least-once：robot main_loop 只 decode_frame 打印、无状态变更，重复帧无害，暂不加去重 |
| 本地闭环 | gossipsub 不回流本机；本地显示走 pose_tx/map_tx → WebSocket，独立路径不受影响 |
| 过渡策略 | **删除旧 RR 广播路径**（Broadcast 命令/方法/分支/RR Robot 入站分支）；`DataType::Robot` 枚举变体**保留**（wire compat，老版本二进制可能仍发 u8=3） |

## 实施计划（基于 merge 后代码，调研报告确认）

### 必须改（5 文件 ~17 行，纯增量）

| # | 文件 | 改动 |
|---|---|---|
| ① | `Src/Network/Gossipsub/mod.rs` | 新增 `TOPIC_ROBOT_POSE: &str = "pleiades/robot/pose"` + `TOPIC_ROBOT_MAP: &str = "pleiades/robot/map"` |
| ② | `Src/Network/mod.rs` | re-export 加 2 个常量 |
| ③ | `Src/Network/network_service.rs` | `Start()` topics 数组加 2 项订阅 |
| ④ | `Src/Network/swarm_events.rs` | `Handle_Gossipsub_Event` 加 2 个分支：`TOPIC_ROBOT_POSE`/`TOPIC_ROBOT_MAP` → `robot_bus.Publish(Bus_Event::StreamRaw { payload: message.data })`（**ORION 二进制透传，不 JSON 解析**——与 RR Robot 分支同风格） |
| ⑤ | `Src/Robot/core/robot.rs` | state_notifier（L226）/ slam_task（L295）：`nh.Broadcast(DataType::Robot, frame)` → `nh.Gossipsub_Publish(TOPIC_ROBOT_POSE/MAP, frame).await`；import 改 `use crate::network::{NodeHandle, TOPIC_ROBOT_POSE, TOPIC_ROBOT_MAP}` |

### 可选改（建议做）

| # | 文件 | 改动 | 理由 |
|---|---|---|---|
| ⑥ | `Src/Network/command_handler.rs` L79 | `GossipsubPublish` 失败 `warn!` → `debug!` | `NoPeersSubscribedToTopic` 属正常，robot 10Hz 下 warn 会刷屏（peer-info/models/sessions 是事件驱动不受影响） |
| ⑦ | `Src/Network/node_handle.rs` | 新增 `Gossipsub_Publish_Try`（同步 try_send 版） | 保留"channel 满即丢弃"旧语义，100ms 节拍零阻塞风险（`.await` 阻塞概率≈0，两者皆可） |

### 删除旧路径（可选清理，建议做）

| # | 文件 | 改动 |
|---|---|---|
| ⑧ | `Src/Network/node_handle.rs` | 删 `NodeHandle::Broadcast`（L147-152）+ `NodeCommand::Broadcast` 变体（L46-50） |
| ⑨ | `Src/Network/command_handler.rs` | 删 `NodeCommand::Broadcast` 分支（L33-50） |
| ⑩ | `Src/Network/swarm_events.rs` | 删 RR `DataType::Robot` 入站分支（L195-211） |
| ⑪ | `codec.rs` | **保留 `DataType::Robot`**（wire compat，0 改动） |

## 风险清单（调研确认）

| 风险 | 结论 |
|---|---|
| async 背压 vs try_send | `.await` 阻塞概率≈0（15 条/秒 vs channel 容量 100）；选⑦可保留满则丢弃语义 |
| 无订阅者日志刷屏 | 选⑥降 debug 解决 |
| 二进制透传 | ✅ `message.data` 原样（Signed 模式下签名独立字段，不污染 payload） |
| Strict validation 短板 | ⚠️ 已知：代码未调用 `report_message_validation_result`，3+ 节点 gossip 只传 1 跳不 fan-out（LAN 全互联无影响；peer-info/models/sessions 同受限）——**记入问题池，不本次处理** |
| 帧大小 | KB 级 << 1MiB 默认上限；未来 map 全量超限时单独调大 map topic 的 max_transmit_size |
| t07 集成测试 | 不引用 Broadcast/DataType::Robot，删旧路径不影响（该测试相对 merge 后代码本就过期，与本任务无关） |
| 快照机制 | Task 20 SnapshotCache 对 map topic 自动生效（新节点订阅即拿最近地图快照）；pose 高频流快照意义小但无害 |

## 验证计划

1. `./build.sh check` + `cargo build --release --bin orion-robot` 编译通过
2. 双车联调：A 车 10Hz 位姿 + 地图增量 → B 车 `[Robot] 收到远端 ORION 帧: msgid=1/3` 打印正常
3. 日志验证：无 `gossipsub 发布失败` 刷屏（⑥生效）、无 RR 超时 error
4. 旧路径确认：`Broadcast` 已删，无调用点残留
5. 迟到节点：B 车后启动订阅 map topic → 收到 A 车最近地图快照（SnapshotCache 生效）

## ⚠️ 分支边界说明（已确认处理方式）

`.github/instructions.md` 声明 Network 归 `ML_review` 职责。**本次处理**：gossipsub/identify 基础设施由 ML_review 实现并已 merge（`bd0ec43`/`387ad72`）；本任务仅在 Orion 侧**消费**基础设施（新增 topic 常量/订阅/分支 + robot.rs 调用点切换），不改动 ML_review 的核心设计。若未来发现基础设施缺陷，回写 ML_review 修复后 merge 回来。

---

## 人类评审

<!-- 在此区域写下评审意见 -->
