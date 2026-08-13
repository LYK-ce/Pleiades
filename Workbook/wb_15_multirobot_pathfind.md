# Workbook — Task 15: Multi-Robot Pathfind 多车路径规划

> 对应任务：`Task/task_15_multirobot_pathfind.md`
> 创建日期：2026-08-13

---

## 2026-08-13 任务创建 + 方案定稿

- 方案讨论（人类逐项确认）：
  - Q1 障碍注入：footprint=1 格；无脑读 ClusterInfoTable 快照（注入层不做在线判断）；每次寻路时注入；DStarLite 加 dynamic_obstacles + set_dynamic_obstacles（diff + 复用 mark_obstacle 修补）
  - 表维护：独立周期 task（2s 周期 / 2s 超时）；先读锁扫、无超时跳过、有超时写锁删 + double-check；upsert 保持只 insert
  - Q2 撤掉（subtarget 死数据，valid 不动）；不碰协议/Pictor/Lua 绑定
  - 清理周期：人类定 2s（先 1s 后改 2s）
- 子 agent 梳理：6 改 2 增共 8 文件；pathfinder/executor 零测试属实；consumer 不受影响
- 开始实施：2026-08-13

## 实施记录（2026-08-13 完成）

- Step 1：`cluster_obstacles.rs`（cluster_to_obstacle_cells 纯函数 + 7 单测）；planning/mod.rs 注册 re-export
- Step 2：`pathfinder.rs` 加 dynamic_obstacles 字段 + set_dynamic_obstacles（diff + 先邻居后自身 update_vertex）+ cost 命中集合 + 3 单测
- Step 3：`cluster_info.rs` remove_stale（先读锁扫、无超时跳过、写锁删 + double-check）+ 注释更新 + 4 单测
- Step 4：`maintenance.rs` cluster_table_cleaner（2s 周期 / 2s 超时）；cluster/mod.rs 注册
- Step 5：`executor.rs` 增参 dynamic_obstacles 三层贯穿 + query_next_sub_target 内注入；`robot.rs` main_loop 加 cluster_table + auto_tick 快照转障碍 + launch spawn cleaner
- Step 6 验证：cargo check 通过；planning 20 全绿 / cluster 13 全绿 / 全量 214 passed + 1 failed（vm::engine::test_sandbox_os_blocked 既有环境失败，与 Task 15 无关）

## Code Review 修复（2026-08-13）

- 子 agent review 发现并修复 3 个真 bug：
  - 🔴-1 `set_dynamic_obstacles` `mem::take` 清空集合致障碍注入失效 → `mem::replace`（修补期间 cost 可见 new 集合）+ diff 收集到 owned Vec（避免借用冲突）
  - 🔴-2 `remove_stale` `duration_since` 竞态 panic（last_seen 晚于 now）→ `saturating_duration_since`
  - 🟠-3 `move_to`/`set_dynamic_obstacles` 顺序颠倒致入队 key 失效 → 对调（move_to 在前）
- 补 2 个 `next_step` 回归测试锁定修复；pathfinder 5 全绿 / 全量 216 passed + 1 既有失败

## 遗留

- 实车双车联调（障碍注入实车验证）
- 问题池 robot_review_problem.md P1 **部分解决**（他车动态障碍已通过 dynamic_obstacles 注入解决；但本车 LiDAR 急停格的 mark_obstacle 仍未置∞，P1 急停路径部分仍存，需后续处理）
- 架构文档 §3.10 文件结构补 cluster_obstacles / maintenance（可选）

- 结束时间：2026-08-13
