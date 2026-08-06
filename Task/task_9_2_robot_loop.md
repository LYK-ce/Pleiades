# Task 9_2: Robot Loop Network 接入

> 状态：进行中——已确认部署形态（双二进制）+ 完成 main.rs Robot 移除
> 创建日期：2026-08-06
> 最后更新：2026-08-06

## 目标

（待讨论）

## 背景

### 部署形态（已确认 2026-08-06）

| 二进制 | 入口 | 用途 |
|---|---|---|
| `orion-robot` | main_robot.rs | **装在车上**：Robot 控制 + 网络数据面（本任务注入点） |
| `Pleiades` | main.rs | **跑在 PC 机**：分布式推理系统（纯推理，无 Robot） |

✅ 2026-08-06：`main.rs` 已移除 Robot 相关（Phase 5.6 块/导入/shutdown），`Pleiades` 回归纯推理入口；`robot_bus`（Task 9_1）与 `Robot_Config`（未来配置 robot 用）保留。

- Task 9_1 已为主枝 Network 增加：`DataType::Robot`、`Broadcast`、`robot_bus`（独立事件总线）
- 本任务：Robot 侧（本分支）接入——把 Network 注入 Robot，打通无人集群数据面

## 待决策问题

（待讨论）

## 已决策

| # | 问题 | 决策 |
|---|------|------|
|   |      |      |

---

## 人类评审

<!-- 在此区域写下评审意见 -->
