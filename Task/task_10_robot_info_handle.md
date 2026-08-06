# Task 10: Robot Info Handle

> 状态：待启动——问题承接自 Task 9 系列（已归档），具体方案待人类补充需求
> 创建日期：2026-08-06
> 最后更新：2026-08-06

## 目标

（待人类补充——初步理解：Robot 信息处理，承接 Task 9 入站数据"仅打印"的后续演进方向）

Task 9_2 决策 #11：入站机器人数据（位姿/地图）本次不处理，main_loop 订阅 robot_bus 仅 `info!` 打印。
本任务承接该扩展点——处理远端机器人信息（存储/展示/融合等，具体范围待定）。

## 承接问题清单（自 Task 9_2 迁移，2026-08-06）

以下问题在 Task 9 系列核查/评审中发现，未在 Task 9_2 修复，记录于此待本任务处理。

### 功能性/正确性

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| P1#3 | `robot.rs` state_notifier/slam_task 广播失败 warn | 广播失败 `warn!` 无退避——弱网下 cmd_tx 持续满 → 10Hz warn 风暴 | 连续失败 N 次降频 debug! / 每 5s 一次，成功复位 |
| P2#4 | `robot.rs:266-287` | slam_task 在 `grid` 写锁内做 JSON 序列化 + broadcast（锁持有时间非最短） | 锁内只取 deltas，锁外组 JSON |
| P2#5 | `robot.rs` recv_robot_event | `Closed => None` 若 sender 全 drop 会忙循环（当前 main_loop 自持 Arc 保活，实际不可达——隐性陷阱） | 注释注明依赖，或 Closed 返回哨兵让 main_loop break |
| P2#6 | `swarm_events.rs:199` | 入站 Robot payload 走 `from_utf8_lossy`——当前 JSON 安全；未来 map_full 二进制会被损坏 | 文档注明 robot_bus 仅支持 UTF-8 JSON；二进制另走通道 |

### 性能/卫生

| # | 位置 | 问题 | 建议 |
|---|---|---|---|
| P3#9 | `robot.rs` notifier/slam_task | `Get_Local_Peer_Id().to_string()` 每 100ms/200ms 各分配一次（peer id 不变） | launch 时算一次传闭包 |
| P3#10 | `robot.rs:272-283` | `typed.clone()` 给 map_tx 后又 iter 组 JSON——两次分配 | 复用一次 |
| P3#13 | `bootstrap.rs` robot_bootstrap | info! 只打 lidar_port 不打 lidar_baudrate | 补字段 |

### 规范/注释

| # | 位置 | 问题 |
|---|---|---|
| P3#11 | `lib.rs` 文件头 | 无 Modified Date |
| 核查 P3-1 | `executor.rs:110-111` | 重复注释（Task 9 改行时未删净） |
| 核查 P3-2 | `robot.rs` launch | `// 4. spawn STM32` 与 `// 4. spawn LiDAR` 编号重复 |
| 核查 P3-3 | `state.rs` / `stm32/mod.rs` / `executor.rs` | 文件头 Modified Date 未随 Task 9 bump 至 2026-08-06 |
| 核查 P3-4 | `main_robot.rs:65` | 注释仍写 "TUI/CLI + Core"，CLI 已移除 |
| 文档偏差-1 | task_9_2 签名草案 | `robot_bootstrap` 草案含 peer_name 参数，实际内部读取 |
| 文档偏差-2 | task_9 main.rs:125 | launch 条目已被 Robot 移除取代 |

## 外部待办（非代码，联调/部署层）

- 多节点联调：位姿/地图广播互通 + 入站打印验证
- 实车验证：串口 + LiDAR + WS origin 显示
- 车端 config.toml 按需更新（Task 9_2 已补缺省，无需强改）

## 已决策

（待人类补充）

---

## 人类评审

<!-- 在此区域写下评审意见 -->
