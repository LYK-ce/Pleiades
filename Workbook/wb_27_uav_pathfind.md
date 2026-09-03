# wb_27_uav_pathfind

- 任务：uav 寻路忽略地面障碍（喂空 grid，D* 走格子不避障）
- 状态：代码实施完成 + 编译验证通过（0 error），待实机/sim 验证
- 改动文件：`pleiades-uav/src/uav/goal.rs`
  - `get_path()` 第④步：注释保留 `dynamic_obstacles` 计算 / `grid` 读锁 / `set_dynamic_obstacles` 调用；新增 `let empty_grid = OccupancyGrid::new()` + `next_step(&empty_grid)`
  - import：`cluster_to_obstacle_cells` 以 `/* */` 注释保留
  - 文件头 Modified Date → 2026-09-03
- 验证：`cargo check -p pleiades-uav` → 0 error（3.57s）
- 新增 warning（已确认接受）：`cluster_table` / `obstacle_inflation_radius` 字段、`cluster_to_obstacle_cells`、`set_dynamic_obstacles` / `has_rhs` 变 dead_code（「注释保留」固有现象）
- 未改动：ugv / terminal / base / sim 全部未动；D* 算法、executor、任务分配均未动
- 关键决策（D1~D8 见 task_27）：z 固定 2m 不动、单机不避让、任务分配复用、地图合并保留、注释而非删除、空 grid 每次 new
