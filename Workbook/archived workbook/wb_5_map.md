# Workbook — Task 5: Map

> 对应任务：`Task/task_5_map.md`
> 创建日期：2026-07-21

---

## 背景（2026-07-21）

- Pictor 协议：`map_full`（全量）+ `map_delta`（增量），voxel 格式 `{gx, gy, gz, state, conf}`
- 传感器：STM32 编码器 → 位姿，LiDAR → 点云，IMU → 姿态
- 多传感器协同：各实现 `MapContributor`，共享 `OccupancyGrid`

---

