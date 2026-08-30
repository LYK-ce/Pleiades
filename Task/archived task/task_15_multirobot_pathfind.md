# Task 15: Multi-Robot Pathfind 多车路径规划

> 创建日期：2026-08-13
> 状态：主体（Step 1~6）已实施；C 节 LiDAR 掩蔽方案已定稿，待实施
> 范围：pathfinder 注入他车障碍（Q1）+ ClusterInfoTable 表维护机制（超时清理）+ LiDAR 误标他车掩蔽（2026-08-15 补充）

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

### C. LiDAR 误标他车掩蔽（2026-08-15 补充）

> 来源：2026-08-15 讨论。原假设 LiDAR 只扫静态环境、他车靠 A 节广播注入（`dynamic_obstacles`）；实车发现 LiDAR 360° 扫描会把其他小车当实体障碍扫到并标 Occupied，且扫描**间歇**（时有时无），导致地图抖动、跨车污染、幽灵障碍。

**问题本质**：他车（动态物体）被混进了静态地图。`lidar_mapper.rs` bresenham 射线把他车表面当终点 `+3`，3 次命中（~600ms）判 Occupied；命中点写 own+chunk 双表，Δ 经广播污染其他车。

**定稿方案（讨论收敛）**：

| 决策点 | 定稿 |
|---|---|
| 分层原则 | 静态层（grid）与动态层（dynamic_obstacles）分离：他车格不进静态地图；他车位置权威来源 = POSE 广播 → `ClusterInfoTable`（与 A 节同源） |
| 作用位置 | **接收方本地每帧**：每辆车在 `slam::update` 内对他车格做 `-3` 抵消（而非发送方广播——差分广播只有变化才发、且 1s 节流对抗不了 5Hz +3） |
| 抵消力度 | **`-3`**（=`OCCUPIED_INCREMENT`）：中和 LiDAR 的 `+3`，他车格稳定在 log ≤ 5（Unknown），永不到 Occupied 阈值 6 |
| 为何不用 -16 | -16 强制 Free（伪造“明确空地”）且第一次会广播 -8；-3 净 0 不广播、语义温和（中和而非改写）、且能清历史残留（+8 被 clamp 顶住后 -3 真实降 3 → 5） |
| 时序约束 | `-3` 与 hit 在**同一写锁**内（`slam::update` 内部），否则 auto_tick(50ms) 可能读到 hit 后中间值 +8 |
| 写表约束 | `-3` 走 `update` 双写语义（own+chunk）；**不可**用 `apply_delta`（只写 chunk，own 的 +3 仍会广播） |
| footprint | 1 格（复用 `cluster_to_obstacle_cells`，不膨胀） |

**与 A 节关系**：A 节 = 动态层（D* 运行时注入，他车走了障碍消失）；C 节 = 静态层（阻止 LiDAR 把他车写进 grid）。互补：grid 里他车格可通行（Unknown/Free），寻路时由 `dynamic_obstacles` 挡住，不穿车。

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

### C 节（LiDAR 掩蔽）涉及文件

| 文件 | 改动摘要 |
|---|---|
| `Src/Robot/slam/grid.rs` | `Chunk::update` free 分支减量改为可配置（支持 `-3`），或新增 `decay(gx, gy, amount)`；补单测 |
| `Src/Robot/slam/lidar_mapper.rs` | `update` 加 `masked: &HashSet<(i32,i32)>` 参数；hit 后对 masked 格做 `-3`；补单测 |
| `Src/Robot/core/robot.rs` | `slam_task` 读 `cluster_table.snapshot()` → `cluster_to_obstacle_cells` → 传入 `update`；`launch` 传 `cluster_table` |

复用：`Src/Robot/core/planning/cluster_obstacles.rs::cluster_to_obstacle_cells`（A 节已实现，1 格不膨胀）。

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

### Step 7：grid.rs 减量接口（独立可编译）

- 改 `Src/Robot/slam/grid.rs`：`Chunk::update` 的 free 分支减量从固定 `FREE_DECREMENT` 改为可配置（新增 `decay(gx, gy, amount: i8)`：`saturating_sub(amount).max(FREE_CLAMP)`），供 `-3` 抵消使用。
- 文件头 Modified bump 2026-08-15。
- 补单测：`-3` 从 0→-3→-6→-8 逐帧递减 / 从 +8 一帧到 5（清历史残留）/ 净 0 幂等。
- 验证：`./build.sh check` + `./build.sh test -p Pleiades --lib robot::slam::grid`

### Step 8：lidar_mapper.rs 掩蔽（独立可编译）

- 改 `Src/Robot/slam/lidar_mapper.rs`：
  - `update` 加参数 `masked: &HashSet<(i32,i32)>`。
  - hit 循环写完所有 endpoints 后（同一写锁内），遍历 `masked` 格调 `decay(-3)`。
  - 文件头 Modified bump 2026-08-15。
  - 补单测：masked 格 hit 后仍 ≤5 / 历史 +8 被清到 5 / 非 masked 格不受影响 / 空 masked 幂等。
- 验证：`./build.sh check` + `./build.sh test -p Pleiades --lib robot::slam::lidar_mapper`

### Step 9：robot.rs slam_task 接线（一起改，保证编译）

- 改 `Src/Robot/core/robot.rs`：
  - import `cluster_to_obstacle_cells`。
  - `slam_task` 加参数 `cluster_table: Arc<ClusterInfoTable>`；每帧读 `cluster_table.snapshot()` → `cluster_to_obstacle_cells(&others)` → `masked` → 传入 `slam::update`。
  - `launch`：spawn `slam_task` 时传 `cluster_table.clone()`。
  - 文件头 Modified bump 2026-08-15。
- 验证：`./build.sh check`

### Step 10：全量验证

- `./build.sh check`
- `./build.sh test -p Pleiades --lib robot::slam`
- 全量：`./build.sh test -p Pleiades --lib`（成本允许时）

---

## 六、待办

- [x] Step 1：`cluster_obstacles.rs` 纯函数 + 单测 + planning/mod.rs 注册
- [x] Step 2：`pathfinder.rs` 动态障碍字段/cost/set_dynamic_obstacles + 单测
- [x] Step 3：`cluster_info.rs` `remove_stale` + 注释 + 单测
- [x] Step 4：`maintenance.rs` 清理 task + cluster/mod.rs 注册
- [x] Step 5：`executor.rs` + `robot.rs` 接线（传参/spawn）
- [x] Step 6：全量 `check` + 分区 `test`
- [x] C Step 7：grid.rs 减量接口（-3）+ 单测
- [x] C Step 8：lidar_mapper.rs masked 参数 + hit 后 -3 + 单测
- [x] C Step 9：robot.rs slam_task 接 cluster_table + launch 传参
- [x] C Step 10：全量 check + test
- [ ] 实车双车联调（LiDAR 掩蔽验证：两车相向/交错，确认他车格不被标 Occupied）
