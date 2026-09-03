# task_27_uav_pathfind — UAV 寻路（忽略地面障碍）

> Created Date ： 2026-09-03
> Modified Date ： 2026-09-03
> 状态：已实施（代码改动 + 编译验证完成，待实机验证）
> 关联文档：`Architecture/robot_arch.md`（§1.1 uav「车 2D 寻路，已接线；后 3D」）

---

## 一、目标

无人机 = 「飞在天上的小车」：保留小车的 2D 格子寻路（D* Lite），但地面障碍对空中的无人机无效，寻路时**忽略障碍**——喂空 grid，让 D* 在「全可走」网格上走 Manhattan 阶梯路径（一格一格、4 连通），不绕任何地面/他车障碍。

## 二、设计决策

| # | 决策 |
|---|------|
| D1 | 保留 2D 格子寻路（D* Lite），**不改算法、不抽公共模块、不大动骨架** |
| D2 | 忽略障碍 = 每次寻路喂空 `OccupancyGrid`（全 Unknown = 全可走）+ 不注入他车动态障碍 |
| D3 | **z 轴不管**（飞控固定 2m），寻路只动 x/y |
| D4 | **单机**（一架无人机），不考虑无人机间避让 |
| D5 | 任务分配（`group_goto` 棋盘散布 / `circle_mission` 环形散布）**照旧复用，不改** |
| D6 | 地图合并（`cluster_consumer` 写 grid）**保留**，只是寻路不再读该 grid |
| D7 | 实现方式用「**注释掉**」而非删除原代码（便于追溯/回退） |
| D8 | 空 grid 每次 `OccupancyGrid::new()`（64KB 清零 @20Hz，开销可忽略，单机无压力） |

## 三、数据流（改动后）

```
get_path()（uav/goal.rs）
  ① 到达判定（世界距离 0.3m）            —— 不变
  ② pop mission + assignment 群发/围圈分配 —— 不变（D5）
  ③ 网格级到达检查                       —— 不变
  ④ 寻路：move_to(当前格) → next_step(空 grid)  —— ✏️ 唯一改动（D2）
        └─ 空 grid 全 Unknown → D* 走 Manhattan 阶梯最短路径，不绕障碍
  → executor 三状态机（转向对齐/前进）    —— 不变
  → mavlink 运动命令                      —— 不变
```

## 四、涉及文件

| 文件 | 改动 |
|---|---|
| `pleiades-uav/src/uav/goal.rs` | ✏️ 唯一改动（`get_path()` 第④步：注释掉避障逻辑 + 喂空 grid） |
| `pleiades-uav/src/uav/planning/pathfinder.rs` | ⬜ 不动 |
| `pleiades-uav/src/uav/planning/cluster_obstacles.rs` | ⬜ 不动（文件保留） |
| `pleiades-uav/src/uav/executor.rs` | ⬜ 不动 |
| `pleiades-base/**` | ⬜ 不动 |
| `pleiades-ugv/**` / `pleiades-sim/**` | ⬜ 不动 |

## 五、具体改动点（`uav/goal.rs` `get_path()` 第④步）

原代码：

```rust
// ④ 寻路：move_to 当前位置 → 注入动态障碍 → next_step
let dynamic_obstacles: Vec<(i32, i32)> = {
    let others = self.cluster_table.snapshot().await;
    cluster_to_obstacle_cells(&others, self.obstacle_inflation_radius).into_iter().collect()
};
let current_gx = (wx / CELL_RESOLUTION).floor() as i32;
let current_gy = (wy / CELL_RESOLUTION).floor() as i32;
let grid = self.grid.read().await;
let next = match self.pathfinder.as_mut() {
    Some(pf) => {
        pf.move_to((current_gx, current_gy));
        pf.set_dynamic_obstacles(&dynamic_obstacles, &grid);
        pf.next_step(&grid)
    }
    None => None,
};
```

改动后（注释保留 + 喂空 grid）：

```rust
// ④ 寻路（task_27：uav 忽略地面障碍，喂空 grid）
// ── 原小车避障逻辑注释保留，便于回退 ──
// let dynamic_obstacles: Vec<(i32, i32)> = {
//     let others = self.cluster_table.snapshot().await;
//     cluster_to_obstacle_cells(&others, self.obstacle_inflation_radius).into_iter().collect()
// };
let current_gx = (wx / CELL_RESOLUTION).floor() as i32;
let current_gy = (wy / CELL_RESOLUTION).floor() as i32;
// let grid = self.grid.read().await;
let empty_grid = OccupancyGrid::new();   // 全 Unknown = 全可走，忽略地面障碍
let next = match self.pathfinder.as_mut() {
    Some(pf) => {
        pf.move_to((current_gx, current_gy));
        // pf.set_dynamic_obstacles(&dynamic_obstacles, &grid);
        pf.next_step(&empty_grid)
    }
    None => None,
};
```

## 六、实现细节 / 连带清理

1. **import 同步注释**：`use crate::uav::planning::{assignment, cluster_to_obstacle_cells};` 中的 `cluster_to_obstacle_cells` 注释掉（避免 unused import 警告）：
   ```rust
   use crate::uav::planning::{assignment /*, cluster_to_obstacle_cells */};
   ```
2. **`obstacle_inflation_radius` 字段**：注释掉后可能变「从未读取」（仅 `dead_code` warning，非 error）。骨架保留不删；若 CI `deny(warnings)` 再处理（如 `#[allow(dead_code)]`）。
3. **`self.grid` / `self.cluster_table` 字段**：`grid` 仍被 `mark_obstacle()` 引用、`cluster_table` 仍被任务分配引用，均**保留**，无 unused 问题。

## 七、验证

- [x] `cargo check -p pleiades-uav` → **0 error**（2026-09-03 已通过）
- [ ] 实机/`pleiades-sim`：uav 下发 Goto，应沿 Manhattan 阶梯路径直走目标格，不因地面静态障碍或他车动态障碍绕行（待验证）

> 备注：注释掉避障逻辑后产生若干 `dead_code` warning（`cluster_table` / `obstacle_inflation_radius` 字段、`cluster_to_obstacle_cells`、`set_dynamic_obstacles` / `has_rhs`），为「注释保留」的固有现象，已确认接受，不影响编译与运行。

## 八、讨论记录

- 2026-09-03：与李永康讨论定稿——无人机定位为「飞在天上的小车」，沿用小车 2D D* 格子寻路（D1）；地面障碍/他车对空中无人机无效，寻路喂空 grid 忽略障碍（D2）；z 固定 2m 由飞控管、寻路只动 x/y（D3）；单机不考虑机间避让（D4）；任务分配照旧复用（D5）；地图合并保留、寻路不读（D6）；用「注释掉」而非删除（D7）；空 grid 每次 new、开销可忽略（D8）。
