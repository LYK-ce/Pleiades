# Workbook — Task 12: Robot 广播接入 GossipSub

> 对应任务：`Task/task_12_broadcast.md`
> 创建日期：2026-08-08

---

## 2026-08-08 实施完成

**范围**：Robot 广播从 request-response 模拟切换到 gossipsub topic（pose/map 两 topic）。

**改动文件**（8 个，+28/-47）：

| 文件 | 改动 |
|---|---|
| `Src/Network/Gossipsub/mod.rs` | +2 常量：`TOPIC_ROBOT_POSE` / `TOPIC_ROBOT_MAP` |
| `Src/Network/mod.rs` | re-export +2 |
| `Src/Network/network_service.rs` | Start() 订阅数组 +2 |
| `Src/Network/swarm_events.rs` | `Handle_Gossipsub_Event` +TOPIC_ROBOT_POSE/MAP 分支（ORION 二进制透传 robot_bus）；RR `DataType::Robot` 分支删除 → 忽略分支（warn 已废弃） |
| `Src/Robot/core/robot.rs` | state_notifier/slam_task 2 处 `Broadcast` → `Gossipsub_Publish(...).await`；import 更新 |
| `Src/Network/command_handler.rs` | 删 `NodeCommand::Broadcast` 分支；GossipsubPublish 失败 warn→debug（防 10Hz 刷屏） |
| `Src/Network/node_handle.rs` | 删 `NodeCommand::Broadcast` 变体 + `NodeHandle::Broadcast` 方法；新增 `Gossipsub_Publish_Try`（同步 try_send 版） |
| `codec.rs` | **不动**（`DataType::Robot` 保留，wire compat） |

**实施中踩坑**：
- 删除 `Broadcast` 变体/方法时残留孤立 `}`/doc 注释 → 语法错误 `unexpected closing delimiter`，两次修复（枚举处 + impl 处）
- 删 RR Robot 分支后 match 非穷尽 → 加 `DataType::Robot` 忽略分支（warn）

**编译验证**：`cargo check` ✅ + `cargo build --release --bin orion-robot` ✅（26 个存量警告不变）

**遗留**：
- Strict validation 短板（3+ 节点 gossip 不 fan-out）→ 记入 robot_review_problem.md 待处理
- 双车联调验证（验证计划见 task_12 文档）待实车/测试环境执行

## 依赖关系

- 依赖 ML_review 基础设施：`bd0ec43`（Task 18: identify+gossipsub）、`387ad72`（Task 20: SnapshotCache）
- 无反向依赖；`DataType::Robot` 枚举保留供 wire compat
