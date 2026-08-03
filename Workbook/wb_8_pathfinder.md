# Workbook — Task 8: D* Lite 路径规划器

> 对应任务：`Task/task_8_pathfinder.md`
> 创建日期：2026-08-03

---

## 2026-08-03 代码审查（子 agent）

逐行审查 `pathfinder.rs`（275 行）+ grid/executor/lidar_mapper/odometry。

**结论**：核心算法与标准 D* Lite (Koenig & Likhachev 2002) 一致（Key 最小堆、收敛条件、update_vertex、pop_valid 惰性清理、km 累积、Manhattan 一致性、next_step 沿 min(cost+g)）。Task 8 决策 Q1~Q7 集成时序全部落实。

**问题清单**（按严重度）：
- 🔴 P1 `mark_obstacle` 未置 ∞，仅重读概率栅格（需 4 次 LiDAR 命中才 Occupied）→ 动态障碍 D* 不知情，反复急停无法绕行
- 🔴 P2 executor 用 `.round()` vs grid/mapper 用 `.floor()` 坐标换算不一致 → 障碍/start 格错位
- 🔴 P3 `compute_shortest_path` 无迭代上限 → 无 watchdog，极端地图拖垮 50ms tick
- 🟠 P5 sub_target 取格角非格中心 → 边界反复查询 ✅ 已修（见下）
- 🟠 P6 网格级到达判据过宽 + goal 格为 Occupied 时任务永不完成也不失败
- 🟠 P7 D* 失败日志 warn!（Task 8 Q6 要求 error!）
- 🟡 P4/P8~P14：震荡、f32 饱和、浮点相等比较、日志风暴、隐式不变式、Turning→Moving 角速度校验缺失等
- 🟡 测试缺失：pathfinder.rs 无 tests（Task 8 步骤 6 未完成，已给 17 场景测试清单）

## 2026-08-03 修复 P5：sub_target 取格中心

**文件**：`Src/Robot/core/executor.rs`（Modified Date → 2026-08-03）

**改动**：
- 新增 `Executor::cell_center_world(gx)`：`(gx as f32 + 0.5) * CELL_RESOLUTION`（格中心，与 world_to_grid floor 语义对齐）
- 替换 3 处 `st * CELL_RESOLUTION`（格角）→ `Self::cell_center_world(st)`：
  - `step_idle` 角偏差计算
  - `step_turning` 对准角度计算
  - `step_moving` 到达距离计算

**验证**：`./build.sh check` 通过，无新增警告。

## 2026-08-03 方案 A：直行连续化（消除直行段顿挫）

**背景讨论**：走格子"一顿一顿"归因——D* 单格接口 + 4 连通（40%）决定运动粒度，Executor 每格 stop→Idle 闭环（60%）决定物理停车。方案 A 只消除直行段停顿（锯齿路径转弯处保留 stop→Turning）。

**文件**：`Src/Robot/core/executor.rs`

**改动**：
- `ExecutorConfig` 新增 `straight_align_threshold_deg`（默认 10.0°）
- 抽取 `query_next_sub_target(wx, wy, grid)`：move_to + next_step 复用（step_idle 同步改用）
- `step_moving` 到达分支重写：到达 sub_target 先问 D* 下一格
  - 方向一致（|角偏差| < 10°）→ 更新 sub_target，**不停车**继续 Moving
  - 方向不一致 → 更新目标，停车进 Idle 交给 Turning
  - None（终点/不可达）→ 停车进 Idle 收尾
- `step_moving` 签名 + `yaw, grid` 参数

**安全性**：连续走期间 LiDAR 急停（step() 入口每 tick）不受影响；到达 goal 格时 next_step 返回 None → 停车 → step_idle 网格级到达检查收尾（逻辑闭环）。

**验证**：`./build.sh check` 通过，无新增警告。

**待调参**：`straight_align_threshold_deg` 需实车验证（10° 初值，太小→连续不生效，太大→该转不转）。

## 2026-08-03 修复 P2：round → floor 统一坐标换算

**实车日志暴露的问题**（Goto 测试）：
- 目标点常落在 .5 边界（如 64.25/0.5=128.5），Rust round() 半远离零舍入 → 目标格/当前格系统性偏 1 格
- Goto(64.25,63.75) 被 round 成 (129,128)，与机器人同格 → “秒到达”不移动
- 机器人真实到达目标格中心但 round 算成 (129,128) → D* 永远返回 (128,128) → 方案 A “直行连续化” 死循环刷屏（2 秒 40+ 次）
- 直到 odom 微动 round 跳变 → 触发 -166° 大掉头 + 7 秒慢转（turn_speed=10）

**结论**：死循环直接触发原因是 P2（round/floor 不一致），修 floor 治本；防循环保护（方案 B）留待后续。

**改动**：`executor.rs` 全部 12 处 `.round()` → `.floor()`（query_next_sub_target / 障碍格 ob_ / pop Mission start/goal / 当前格 / 网格级到达 goal），与 grid.rs/lidar_mapper.rs 建图端统一。

**效果**：Goto(66.25,63.75) → goal 格 (132,127)，格中心=目标点本身；格中心 64.25 → floor(128.5)=128 正确。

**验证**：`./build.sh check` 通过。

## 待办（下一步）

1. 修 P1：DStarLite 内部 `obstacles: HashSet`，cost() 先查集合（mark_obstacle 强制 ∞）
2. 修 P2：统一坐标换算（建议 floor，与建图端一致）
3. 修 P3：加 MAX_ITERS 迭代上限
4. 补 pathfinder 单元测试（17 场景清单在审查报告中）
5. 修 P6（goal 占用格死循环）、P7（日志级别）
6. 实车验证（步骤 7）
