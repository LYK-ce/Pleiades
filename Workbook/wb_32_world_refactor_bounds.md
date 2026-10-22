# wb_32_world_refactor_bounds — task_32 工作记录

> Presented by KeJi
> Created Date ： 2026-10-22
> Modified Date ： 2026-10-22
> 任务：Task/task_32_world_refactor_bounds.md（世界工程目录重构 + 32 m 空气墙 + UAV 巡航 2.5 m）

---

## 任务启动

- 开始时间：2026-10-22
- 决策来源（人类）：
  1. 目录：代码与 `.tscn` 全部进 `src/`，`assets/` 只放 png/建模/material 等真资源；`project.godot` 与 `COMPONENTS.md` 留根。
  2. 边界：**可见 2 m 体素墙保留** + **顶盖加** + **空气墙放 layer 1**。
  3. UAV 巡航高度 2.0 → 2.5。
- 非目标：**不动 `pleiades-sim/` 的任何行为逻辑**（决策链 / pathfinder / goal / executor / mapper / 急停）；行为逻辑整改属后续独立任务。
- 已撤回的伪问题（不再进任何清单）：
  - 「世界不告诉 sim 传感器是否健康」——世界是 Godot 仿真，雷达每圈都发帧（无回波也是 1455 B），帧节奏即活性；`world_time` 每帧携带；`assembly_ready` 已是就绪握手。
  - 「`avatar_id` 进程内计数」——进程内单调计数是正常设计，task_31 的 C-B 是判据错、非代码错。
  - 「spawn 可能重叠」——前 8 台都在半径 2 m 内，出生点净空 3 m、最近障碍 19.8 m。
  - 「雷达扫不到别的车」——真车也时而扫到时而扫不到，属现实一致性；建图的 `masked + decay(3)` 已覆盖该情形。

## 现状基线（2026-10-22 实测）

- `orion-simulation-world/`：24 个 `.gd` + 7 个 `.tscn` + `project.godot` + `COMPONENTS.md`；**没有任何资源文件**（几何全程序化，`MeshLibrary` 运行时生成）。
- `run/main_scene = "res://World.tscn"`；`godot 4.7.2`。
- 边界：4 面体素墙 0.5 m 厚 × 2 m 高（`World.gd` `OBSTACLES` 前 4 项）；地面 `BoxShape3D(128,1,128)` 顶面 y=0。
- UAV 巡航高度 `UAV_ALTITUDE_M = 2.0`（与墙高重合）。
- 自检：9 个，全部 `SELF-CHECK OK`。
- 真 sim 二进制：`RobotSim_TestEnv/sim1/pleiades-sim`（2026-10-22 01:33 构建，本任务不需要重编）。

## Step 记录（每 step 结束后追加）

## 执行引擎启动
- 引擎：Tool/execution_engine_task32.ts + Tool/execution_engine_task32.config.json
- 计划：Step 1~3 串行，每 step 最多 3 轮（规划 → 实现 → 评审）

## Step 1 — 目录重构（src/ + assets/）
- 范围：orion-simulation-world/（Godot 工程）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](<tmp/task32/plan.md>)，规划轮只做了只读勘察，实现轮可直接照做；`orion-simulation-world/` 与 `pleiades-*` 未动一行、无 `git add/commit`。 1. **搬迁清单已落到逐文件逐行**：32 个文件（7 个 `.tscn` → `src/scenes/…`、25 个 `.gd` → `src/{模块}/` 与 `src/tests/`）+ 新建 `assets/{.gitkeep,README.md}` + 删 4 个旧目录与 9 个空 `.gitkeep`；代码/场景侧 **97 行 `res://`**（84 处可执行 + 13 处注释）、`COMPONENTS.md` **33 处 `res://` + 约 90 行裸路径**全部给了 before→after 表。 2. **实测出两处文档级坑**（已写进风险并给出等价命令）：step.md 的验收 grep 缺 `-E`（`\|` 按字面匹配 → 假绿），以及裸 `godot --check-only` 不带 `--script` 根本不检查脚本、会跑主循环挂死（实测 `timeout 60` → `EXIT=124`）；另纠正 task_32 §二.2 写的「WebSocketManager 4 处 preload」实为 **5 个 preload + 3 个路径常量 + 1 处注释**。 3. **基线已测**：`--quit` `EXIT=0`/0 ERROR 且日志 6 行可作「行为未变」对照，`check_world total=26`、`check_camera total=32`，其余 7 个期望总数（36/41/51/45/48/23）与 §7-A2 表列齐。 4. **最易错点单列**：`src/

## Step 2 — 空气墙（32 m）+ 顶盖
- 范围：orion-simulation-world/（Godot 工程）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task32/plan.md`（35 KB，9 章，全部表格通过 markdown 列数校验；仅新增该文件，未碰任何代码）。 要点总结（4 行）： 1. **实施规模**：改 `src/world/World.gd`（新增 3 个常量、`AIR_WALL_BOXES` 5 项几何表以及 `_Build_Bounds()`；`_ready()` 中在 `BuildVoxelWorld()` 之后调用）、新建 `src/tests/check_bounds.gd`（12 项断言，`total=12`）、同步 `COMPONENTS.md` 共 12 处锚点；**`World.tscn`、`project.godot` 零改动**（Bounds 走代码，从而避开 E16）。 2. **几何判据**：内侧面严格贴 Godot `x=0/128`、`z=0/128`、`y=32`，厚度一律向外、layer 1 / mask 0、子树 `MeshInstance3D` 计数必须为 0；`B1b/B1c/B1d/B1e` 把“贴边界、厚度朝外、不在 .tscn、可见、幂等”这些本来只能靠肉眼检查的约定变成会自动报错的检查。 3. **验收基线已实测**：本机 Godot 4.7.2，9 个既有自检开工前全部 PASS（`world 26 / camera 32 / vehicle 36 / diff_drive 41 / quadrotor 45 / ws_manager 51 / lidar 48 / pose 23`），验收固定为「`--quit` EXIT=0 + `grep` 无 ERROR + 10 个自检 + 快照 `sha1sum` 证明旧文件字节未动」。 4. **风险与取舍已显式记录**：step.md 的 `_BuildWorld()` 不存在（

## Step 3 — UAV 巡航高度 2.5 m + 软件 clamp + 真跑冒烟
- 范围：orion-simulation-world/（Godot 工程）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task32/plan.md`（391 行）。规划期间我实际运行了基线，并验证了冒烟夹具，`orion-simulation-world/` 一字未改。 **计划要点（4 行）** 1. **改动面**：涉及 5 个文件——`WebSocketManager.gd` 中 `UAV_ALTITUDE_M` 从 2.0 改为 2.5；`Quadrotor.gd` 新增 `ClampAltitude()`、`_physics_process()` 以及 1 秒限流 warn，把宿主车身的世界 z 限制在 `[0, World.AIR_WALL_HEIGHT_M-0.2] = [0, 31.8]`；`check_quadrotor.gd` 把 `SPAWN_WORLD.z` 从 2.0 改为 2.5，并新增 3 项 clamp 断言（45→48）；`check_ws_manager.gd` 的 B5 期望值从 2.0 改为 2.5；`COMPONENTS.md` 按 17 处逐行清单修改，并新增 E25 和 §11 `task_32-3` 行。 2. **关键裁决**：`check_quadrotor` 的白盒 lint 禁止脚本出现 `global_position`（D8），所以 clamp 通过 `global_transform.origin.y` 修改**Vehicle 车身**（不是 Quadrotor 子节点）；备选方案（Vehicle 在 move 后回调）已登记但未采用；`check_pose.gd` 实测只测 UGV（3 处 2.0 是 `BuildPose` 样本），因此不改并说明；回写 task_30 受范围约束，改为记进 COMPONENTS。 3. **审计口径**：带边界正则的 `2.0` 命中 42 行 = 9 行 UAV 语义
