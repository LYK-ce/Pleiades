# Task 15: Multi-Robot Pathfind 多车路径规划

> 创建日期：2026-08-13
> 状态：方案已定稿，待实施
> 范围：pathfinder 注入他车障碍（Q1）+ ClusterInfoTable 表维护机制（超时清理）

---

## 一、背景

任务来自 `Task/robot_review_problem.md` 的 Q1/Q2 讨论（见 `wb_14_group_goto.md` 末尾待讨论问题）。

- **Q1**：D* 寻路 `cost()` 只读静态地图，不感知其他车位姿——其他车会撞上（P3 动态障碍注入未实施）。
- **Q2**：手动/停车状态下 subtarget 广播残留脏值。

**结论（2026-08-13 定稿）**：采用最简单方案——其他车当障碍注入即可，不做 intent 交通管制；**Q2 整体撤掉**（subtarget 当前无消费端、是死数据，修它无意义，`valid` 字段不动，留待 P4 时一并设计）。

---

## 二、方案设计

### A. pathfinder 注入他车障碍

| 决策点 | 定稿 |
|---|---|
| footprint | **1 格**（对方所在那一格，不膨胀；小车直径 0.3m < 格宽 0.5m） |
| 注入语义 | **无脑读 `ClusterInfoTable` 全量快照**，表里几辆车就注入几辆，**注入层不做在线/超时判断** |
| 超时剔除归属 | 下沉到独立的表维护机制（见 B），让表语义 = 在线车辆集合 |
| 更新时机 | **每次寻路时注入最新障碍格**（executor 每次问 D* 下一格之前） |

实现方式（即问题池 P1 建议的修法）：

1. `DStarLite`（`Src/Robot/core/planning/pathfinder.rs`）增加字段 `dynamic_obstacles: HashSet<(i32,i32)>`。
2. `cost()` 开头命中集合即返回 `f32::MAX`（先查集合，再查越界与静态 grid）。
3. 新增 `set_dynamic_obstacles(&mut self, cells: &[(i32,i32)], grid: &OccupancyGrid)`：对 old/new 集合做 diff，**变更格先更新邻居、再更新自身**（复用 `mark_obstacle` 的局部修补机制），新增格与移除格都这样修补。
4. 新增纯函数 `cluster_to_obstacle_cells(others: &[ClusterInfo]) -> HashSet<(i32,i32)>`：世界坐标 → 格坐标（`world_to_grid`，同格自动去重）。
5. `main_loop` auto_tick 读 `cluster_table.snapshot()` → 转障碍格 → 传给 `executor.step`。

### B. 表维护机制（周期清理 task）

| 决策点 | 定稿 |
|---|---|
| 方案 | **独立周期 task**（优于 upsert 顺带清理：总开销 O(N)/秒 vs O(10N²)/秒，且不加重 consumer 热路径、不饿死 main_loop 读锁） |
| 周期 | **2s** |
| 超时阈值 | **2s**（POSE 10Hz → 20 拍未收到判失联；常量，后续可配置化） |
| 锁策略 | 先**读锁**扫一遍收集超时 `peer_id`；无超时直接返回（不碰写锁）；有超时才**写锁**删除，删除时 **double-check** `last_seen` 仍超时 |

1. `ClusterInfoTable` 新增 `remove_stale(&self, timeout: Duration) -> usize`（先读后写 + double-check）。
2. 新增 `cluster_table_cleaner(table: Arc<ClusterInfoTable>, cancel: CancellationToken)` 周期 task（2s interval，`MissedTickBehavior::Skip`）。
3. `consumer.rs` 的 `upsert` **保持只 insert 一条，不改**。
4. 更新 `cluster_info.rs` 文件头注释：表语义 = 在线车辆集合，超时由 maintenance task 剔除。

---

## 三、范围外（明确不做）

- ❌ Q2 subtarget 广播修正（死数据，无消费端）
- ❌ `valid` 字段改动（暂不大改协议）
- ❌ 协议/Pictor 端任何改动
- ❌ Lua 绑定（本任务不触及 `Src/VM/`）

---

## 四、涉及文件

### 修改（6 个）

| 文件 | 改动摘要 |
|---|---|
| `Src/Robot/core/planning/pathfinder.rs` | 加 `dynamic_obstacles` 字段；`cost()` 命中返回 MAX；新增 `set_dynamic_obstacles()`；补 `#[cfg(test)]` |
| `Src/Robot/core/cluster/cluster_info.rs` | 新增 `remove_stale(timeout: Duration) -> usize`；更新表语义注释；补测试 |
| `Src/Robot/core/executor.rs` | `step/step_impl/step_idle/step_moving/query_next_sub_target` 增参 `dynamic_obstacles: &[(i32,i32)]` 并透传；`query_next_sub_target` 内先 `set_dynamic_obstacles` 再问 D* |
| `Src/Robot/core/robot.rs` | `main_loop` 增 `cluster_table: Arc<ClusterInfoTable>` 参数；auto_tick 读快照 + 转障碍格 + 传入 executor；`launch` 传参 + spawn cleaner task |
| `Src/Robot/core/planning/mod.rs` | 注册并 re-export `cluster_obstacles` |
| `Src/Robot/core/cluster/mod.rs` | 注册并 re-export `maintenance` |

### 新增（2 个）

| 文件 | 内容 |
|---|---|
| `Src/Robot/core/planning/cluster_obstacles.rs` | 纯函数 `cluster_to_obstacle_cells` + 单测 |
| `Src/Robot/core/cluster/maintenance.rs` | 周期清理 task `cluster_table_cleaner` |

### 不改

`consumer.rs`、`grid.rs`（`world_to_grid` 已存在）、`core/mod.rs`、`Robot/mod.rs`、`Src/VM/`。

### 关键签名

```rust
// cluster_obstacles.rs
pub fn cluster_to_obstacle_cells(others: &[ClusterInfo]) -> HashSet<(i32, i32)>;

// cluster_info.rs
pub async fn remove_stale(&self, timeout: Duration) -> usize;

// maintenance.rs
pub async fn cluster_table_cleaner(table: Arc<ClusterInfoTable>, cancel: CancellationToken);

// pathfinder.rs
pub fn set_dynamic_obstacles(&mut self, cells: &[(i32, i32)], grid: &OccupancyGrid);

// executor.rs（step 系列）
pub fn step(..., grid: &OccupancyGrid, dynamic_obstacles: &[(i32,i32)], mission_queue: &mut MissionQueue, execute_state: &mut ExecuteState, own_peer_id: &[u8]);
```

---

## 五、实施步骤（按依赖顺序，每步独立可编译/测试）

> 验证命令：`./build.sh check` = `cargo check`；`./build.sh test -p Pleiades --lib <filter>` = `cargo test -p Pleiades --lib <filter>`。包名 `Pleiades`。

### Step 1：纯函数（独立可编译）

- 新增 `Src/Robot/core/planning/cluster_obstacles.rs`：`cluster_to_obstacle_cells`（`world_to_grid(info.x, info.y)` 收集去重）+ 单测。
- 改 `Src/Robot/core/planning/mod.rs`：`pub mod cluster_obstacles;` + `pub use cluster_obstacles::cluster_to_obstacle_cells;`。
- 文件头 Created/Modified = 2026-08-13。
- 单测 ≥7 场景：空输入 / 单车上取整 / 负坐标 floor / 边界 0.5→1 / 多车同格去重 / 多车不同格 / 不膨胀验证。
- 验证：`./build.sh check` + `./build.sh test -p Pleiades --lib robot::core::planning::cluster_obstacles`

### Step 2：DStarLite 动态障碍（独立可编译）

- 改 `Src/Robot/core/planning/pathfinder.rs`：
  - `use std::collections::{BinaryHeap, HashMap, HashSet};`
  - 结构体末尾加 `dynamic_obstacles: HashSet<(i32, i32)>`，`new()` 初始化 `HashSet::new()`。
  - `cost()` 开头加 `if self.dynamic_obstacles.contains(&to) { return f32::MAX; }`。
  - 新增 `set_dynamic_obstacles`：diff old/new，变更格「先邻居后自身」`update_vertex`。
  - 补 `#[cfg(test)]`：cost 命中集合 / 移除恢复 / 相同集合幂等 / next_step 避障（可选）。
- 文件头 Modified bump 2026-08-13。
- 验证：`./build.sh check` + `./build.sh test -p Pleiades --lib robot::core::planning::pathfinder`

### Step 3：ClusterInfoTable 超时删除（独立可编译）

- 改 `Src/Robot/core/cluster/cluster_info.rs`：
  - `use std::time::{Duration, Instant};`
  - 新增 `remove_stale(timeout: Duration) -> usize`（先读锁收集超时 key，空则返回 0；非空写锁 + double-check 删除）。
  - 更新文件头/字段注释（表语义 = 在线车辆集合）。
  - 补测试：空表 / 全未超时 / 一条超时 / 混合超时未超时。
- 文件头 Modified bump 2026-08-13。
- 验证：`./build.sh check` + `./build.sh test -p Pleiades --lib robot::core::cluster::cluster_info`

### Step 4：表维护 task（独立可编译，暂无调用方）

- 新增 `Src/Robot/core/cluster/maintenance.rs`：`cluster_table_cleaner`（2s interval + `MissedTickBehavior::Skip`，`remove_stale(2s)`，removed>0 才 `info!`，`cancel` 退出）。
- 改 `Src/Robot/core/cluster/mod.rs`：`pub mod maintenance;` + `pub use maintenance::cluster_table_cleaner;`。
- 文件头 Created/Modified = 2026-08-13。
- 验证：`./build.sh check`

### Step 5：executor + robot 接线（一起改，保证编译）

- 改 `Src/Robot/core/executor.rs`：
  - `step/step_impl/step_idle/step_moving/query_next_sub_target` 增参 `dynamic_obstacles: &[(i32,i32)]` 并透传（插在 `grid` 之后）。
  - `query_next_sub_target` 内：`pf.set_dynamic_obstacles(dynamic_obstacles, grid);` 再 `move_to` + `next_step`。
  - 两处调用点（`step_idle`、`step_moving`）同步传参。
- 改 `Src/Robot/core/robot.rs`：
  - import `cluster_to_obstacle_cells`、`cluster_table_cleaner`。
  - `main_loop` 加参数 `cluster_table: Arc<ClusterInfoTable>`。
  - auto_tick：读 `cluster_table.snapshot()` → `cluster_to_obstacle_cells(&others)` → 传给 `executor.step`。
  - `launch`：加 `loop_cluster_table = cluster_table.clone()` 传入 main_loop；在 cluster_consumer spawn 之后 spawn cleaner（clone `cluster_table` + `cancel`）。
- 文件头 Modified bump 2026-08-13。
- 验证：`./build.sh check`

### Step 6：全量验证

- `./build.sh check`
- `./build.sh test -p Pleiades --lib robot::core::planning`
- `./build.sh test -p Pleiades --lib robot::core::cluster`
- 全量：`./build.sh test -p Pleiades --lib`（成本允许时）

---

## 六、待办

- [x] Step 1：`cluster_obstacles.rs` 纯函数 + 单测 + planning/mod.rs 注册
- [x] Step 2：`pathfinder.rs` 动态障碍字段/cost/set_dynamic_obstacles + 单测
- [x] Step 3：`cluster_info.rs` `remove_stale` + 注释 + 单测
- [x] Step 4：`maintenance.rs` 清理 task + cluster/mod.rs 注册
- [x] Step 5：`executor.rs` + `robot.rs` 接线（传参/spawn）
- [x] Step 6：全量 `check` + 分区 `test`
