# task_32_world_refactor_bounds — 世界工程目录重构 + 32 m 空气墙

> Presented by KeJi
> Created Date ： 2026-10-22
> Modified Date ： 2026-10-22
> 状态：**已获人类批准，待执行**（3 个 step，串行）
> 关联文档：`orion-simulation-world/COMPONENTS.md`、`Task/task_30_sim_world.md`、`Task/task_31_sim_world_link.md`

---

## 一、目标（人类原话 → 拆解）

1. **目录重构**：在 `orion-simulation-world/` 内新建 `src/` 与 `assets/`；
   **代码（`.gd`）与场景（`.tscn`）一律进 `src/`**；**`assets/` 只放真正的资源文件**（png / glb / obj / material / wav 等）。
2. **空气墙 + 顶盖**：边界改为「可见 2 m 体素墙 **保留** + 不可见空气墙（0→32 m）+ 32 m 顶盖」，
   **空气墙放 layer 1**，确保任何东西都出不去。
3. **UAV 巡航高度 2.0 m → 2.5 m**（现 2.0 m 与边界墙高完全重合），并加软件 clamp 兜底。

### 明确不做（非目标）

- **不动 `pleiades-sim/` 的任何行为逻辑**（决策链 / pathfinder / goal / executor / mapper / 急停策略）。
  行为逻辑的整改属于后续独立任务，本任务一行不碰。
- 不动 `pleiades-base` / `pleiades-ugv` / `pleiades-uav` / `pleiades-terminal`。
- 不新增传感器（相机 / IMU / GNSS）、不做多线雷达、不改帧格式。
- **"雷达扫不到别的车"不是缺陷**（真车也时而扫到时而扫不到），本任务不修、不改建图与急停语义。

---

## 二、Step 1 — 目录重构：`src/` + `assets/`

### 1 目标布局（唯一权威）

```text
orion-simulation-world/
├── project.godot                 ← 留在根；只改 run/main_scene
├── COMPONENTS.md                 ← 留在根（唯一施工文档）
├── .gitignore
├── src/
│   ├── scenes/
│   │   ├── World.tscn
│   │   ├── ObserverCamera.tscn
│   │   ├── Vehicle.tscn
│   │   ├── WebSocketManager.tscn
│   │   ├── chassis/
│   │   │   ├── DiffDrive.tscn
│   │   │   └── Quadrotor.tscn
│   │   └── sensors/
│   │       └── Lidar2D.tscn
│   ├── camera/ObserverCamera.gd
│   ├── chassis/{ChassisModel,DiffDrive,Quadrotor,ChassisCatalog}.gd
│   ├── common/{WorldTransform,WorldClock}.gd
│   ├── net/{WireProtocol,ComponentLink,WebSocketManager}.gd
│   ├── sensors/{Lidar2D,SensorCatalog}.gd
│   ├── vehicle/Vehicle.gd
│   ├── world/World.gd
│   └── tests/
│       ├── check_project.gd / check_world.gd / check_camera.gd / check_vehicle.gd
│       ├── check_diff_drive.gd / check_quadrotor.gd / check_ws_manager.gd
│       ├── check_lidar.gd / check_pose.gd
│       └── doubles/{TestChassis.gd,TestSensor.gd}
└── assets/
    ├── .gitkeep
    └── README.md                 ← 写明「只放媒体与建模资源；代码与场景一律 src/」
```

- `.gd` 的**模块分组保持原样**（`world/ vehicle/ chassis/ sensors/ net/ camera/ common/`），只是根从工程根换成 `src/`。
- `tests/doubles/` 跟随 `tests/` 一起进 `src/tests/doubles/`。
- 搬迁后删除根下已空的 `scripts/`、`chassis/`、`sensors/`、`tests/` 及其中的 `.gitkeep`。

### 2 必须同步修改的引用（漏一条就是加载期失败）

| # | 位置 | 内容 |
|---|---|---|
| 1 | `project.godot` | `run/main_scene="res://World.tscn"` → `"res://src/scenes/World.tscn"` |
| 2 | 7 个 `.tscn` | 每个 `[ext_resource ... path="res://scripts/..."]`、实例化的 `PackedScene` 路径逐条改 |
| 3 | `WebSocketManager.gd` | 4 个 `preload()` 路径 |
| 4 | `ChassisCatalog.gd` / `SensorCatalog.gd` | 场景 `load()` 路径（`res://chassis/DiffDrive.tscn` 等） |
| 5 | `Vehicle.gd` | 对 Catalog / 底盘场景的 `preload`/`load` 路径 |
| 6 | `World.gd` | 对 `WorldTransform` / `WorldClock` 的 `preload` 路径 |
| 7 | `tests/*.gd` | 对 `doubles/` 与其它脚本的 `preload`/`load` 路径 |
| 8 | `COMPONENTS.md` | 全文 `res://` 与文件路径（§1.1 目录树、§3 清单、§7 接口签名、§11 step↔文件映射），以及自检命令里的 `--script res://tests/...` |
| 9 | 自检调用方式 | 统一改为 `--script res://src/tests/check_*.gd` |

### 3 硬验收

1. `rm -rf orion-simulation-world/.godot`（清旧路径缓存）后：
   `godot --headless --path orion-simulation-world --quit` → **EXIT=0**，且 `grep -E "SCRIPT ERROR|Parse Error|ERROR"` **为空**。
2. **9 个自检全 PASS**（`SELF-CHECK OK` + `failed=0` + EXIT=0）：
   `check_project / check_world / check_camera / check_vehicle / check_diff_drive / check_quadrotor / check_ws_manager / check_lidar / check_pose`
   命令形如 `timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_xxx.gd`。
3. `grep -rn "res://tests/\|res://scripts/\|res://chassis/\|res://sensors/\|res://camera/\|res://world/\|res://vehicle/\|res://common/" orion-simulation-world/ --include=*.gd --include=*.tscn --include=*.godot --include=*.md` → **无输出**。
4. `godot --headless --path orion-simulation-world --check-only`（对所有脚本）→ 0 错误。
5. `git status` 改动只落在 `orion-simulation-world/`。

### 4 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | 只搬一半 → 解析期全红，而 `--quit` 仍返回 0（E4） | 必须**一次性**改完所有引用再验；验收固定为「exit 0 + grep 无 ERROR + 9 自检」两条 |
| 2 | `.godot/` 旧缓存让旧路径继续生效 | 搬迁后先 `rm -rf .godot` 再跑 |
| 3 | `.tscn` 里 `uid="uid://..."` 指向旧文件 | 本项目按 E7/E8 未使用 uid，禁止新加；若遇到则删 uid 属性 |
| 4 | 手写 `.tscn` 的 `@export` 节点引用需 `node_paths`，改路径时容易漏 | 只改 `path=` 字符串，不重构节点结构（E16/E20） |
| 5 | `assets/` 建了但空目录不进 git | 放 `.gitkeep` + `README.md` |

---

## 三、Step 2 — 空气墙（32 m）+ 顶盖

### 1 目标

在现有**可见 2 m 体素边界墙**（`World.gd` 的 `OBSTACLES` 前 4 项，保留不动）之外，增加**不可见**的硬边界，确保任何实体都出不去。

### 2 几何与物理（写死，不得自行发挥）

| 件 | 位置（世界坐标 Z-up，米） | Godot 节点参数 |
|---|---|---|
| 南墙 | x∈[0,128], y∈[-1,0], z∈[0,32] | `position=(64,16,-0.5)`, `size=(128,32,1)` |
| 北墙 | x∈[0,128], y∈[128,129], z∈[0,32] | `position=(64,16,128.5)`, `size=(128,32,1)` |
| 西墙 | x∈[-1,0], y∈[0,128], z∈[0,32] | `position=(-0.5,16,64)`, `size=(1,32,128)` |
| 东墙 | x∈[128,129], y∈[0,128], z∈[0,32] | `position=(128.5,16,64)`, `size=(1,32,128)` |
| 顶盖 | z∈[32,33] | `position=(64,32.5,64)`, `size=(128,1,128)` |

- 记忆口径：Godot `y` = 世界 `z`（见 `WorldTransform`）。碰撞面**内侧面正好贴世界边界平面**（x=0/128、y=0/128、z=32），厚度一律**向外**，避免车中心越界导致格坐标为负。
- `collision_layer = 1`（与地面同层 → `Lidar2D.scan_mask = 1|2|4` 也能读到），`collision_mask = 0`。
- 一个 `StaticBody3D`（命名 `Bounds`）+ 5 个 `CollisionShape3D`；**由代码构建**（`_Build_Bounds()`），不写进 `.tscn`（避开 E16 的 `node_paths` 坑）。
- 常量：`AIR_WALL_HEIGHT_M := 32.0`、`AIR_WALL_THICKNESS_M := 1.0`、`AIR_WALL_LAYER := 1`。
- 可见的 4 面 2 m 体素墙**保留**。

### 3 新自检 `src/tests/check_bounds.gd`

沿用既有自检形态（同进程真 `World.tscn`、`--fixed-fps 60`、外部 `timeout` 兜底）。断言：

| # | 断言 |
|---|---|
| B1 | `Bounds` 节点存在，恰有 5 个 `CollisionShape3D`，`collision_layer == 1`，且**无任何 `MeshInstance3D` 子节点**（不可见） |
| B2 | 东向：`CharacterBody3D` 从 `(64,64,0)` 以 20 m/s 向 +x 冲 10 s → 最终 `x < 128` |
| B3 | 西向 → `x > 0`；B4 南向 → `y > 0`；B5 北向 → `y < 128` |
| B6 | 顶盖：从 `(64,64,2)` 以 5 m/s 上升 10 s → 最终 `z ≤ 32` |
| B7 | 地面仍有效：从 `(64,64,5)` 自由落体 → 静止后 `z ∈ [0.10, 0.25]` |
| B8 | 可见矮墙仍在：`OBSTACLES` 前 4 项体素格数 > 0（`wall_south/north/west/east`） |

（"雷达能读到空气墙"不作硬断言：量程 12 m，只在贴边时可测，易 flaky。）

### 4 硬验收

1. `--quit` EXIT=0 且无 ERROR；`check_bounds.gd` → `SELF-CHECK OK` / `failed=0`。
2. **既有 9 个自检全部仍 PASS**（尤其 `check_world` 与 `check_lidar`）。
3. `COMPONENTS.md` 同步：§2 节点树加 `Bounds`、§4.1 补空气墙几何表、§10 边界改为「可见 2 m 矮墙 + 32 m 空气墙 + 顶盖」、§9.1 新增 E24（空气墙碰撞面必须贴边界平面、不得用 mesh 渲染）。

### 5 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | 碰撞面放在边界外侧 → 车中心到 -0.5，格坐标负 | 严格按上表（厚度向外） |
| 2 | 顶盖压到 UAV 巡航高度 | 顶盖内表面 32 m，巡航 2.5 m，余量充足 |
| 3 | 空气墙被 SLAM 写成静态障碍 | 预期行为：贴边时地图出现墙，正确 |
| 4 | 高速穿透 | 最快 4 m/s / 60 Hz = 0.067 m/帧 << 1 m 厚 |
| 5 | `Bounds` 放在 `World.tscn` 根下会影响 `_EnsureRefs` 的路径推断 | 作为 `World` 子节点、命名固定 `Bounds`，不改既有节点层级 |

---

## 四、Step 3 — UAV 巡航高度 2.5 m + 软件 clamp

### 1 目标

`UAV_ALTITUDE_M` 2.0 → **2.5**（与 2.0 m 边界墙解耦，越过矮墙）；`Quadrotor` 加软件 clamp 兜底（即使物理失效也不会飞出 32 m）。

### 2 涉及点

| # | 位置 | 动作 |
|---|---|---|
| 1 | `src/net/WebSocketManager.gd` `UAV_ALTITUDE_M` | 2.0 → 2.5 |
| 2 | `src/chassis/Quadrotor.gd` | 每物理帧把 `global_position.y` clamp 到 `[0.0, AIR_WALL_HEIGHT_M - 0.2]`（世界 z），越界时把 `velocity.y` 归零并限流 warn |
| 3 | `src/tests/check_quadrotor.gd` | 悬停 / 升降用例的期望高度同步为 2.5（若写死 2.0） |
| 4 | `src/tests/check_pose.gd` | `spawn.z` / `pose.z` 的 UAV 期望改 2.5（静止位姿必须等于 spawn 的断言保持） |
| 5 | `src/tests/check_ws_manager.gd` | 若断言 UAV spawn z=2.0 则同步 |
| 6 | `COMPONENTS.md` | §4.6 巡航高度、§8 消息示例的 `spawn.z`、§10 边界、§9.1 相关禁用项 |

### 3 硬验收

1. `--quit` EXIT=0、无 ERROR；`check_quadrotor` / `check_pose` / `check_ws_manager` 全 PASS。
2. `grep -rn "2\.0" orion-simulation-world/src --include=*.gd` 中不再有「UAV 巡航高度」语义的 2.0（允许其它语义的 2.0，逐条说明）。
3. **真跑冒烟**（世界 + 真二进制）：
   - `godot --headless --path orion-simulation-world` 起世界；
   - `RobotSim_TestEnv/sim1/pleiades-sim --headless`（已有 2026-10-22 构建的二进制，无需重编）；
   - 30 s 内日志出现 `[world] assembled`，`curl`/`nc` 连 9098 发 `{"cmd":"status"}` 得到 `assembled=true`；
   - 结束必须 kill 掉两个进程，`ps` 无残留。
4. `git status` 只改 `orion-simulation-world/`（`RobotSim_TestEnv/` 已 gitignore，可写日志）。

### 4 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | 抬高 UAV 后越过 2 m 墙，行为与 task_30「UAV 恒在墙内」的记录冲突 | 属预期变更，回写 `COMPONENTS.md` §4.6 与 task_30 的对应记录 |
| 2 | `check_*` 里写死的 2.0 散落多处，漏改即假失败 | 先 `grep -rn "2\.0"` 全量列出，逐条判定 |
| 3 | 用旧 sim 二进制做冒烟时，世界侧改动会让老断言（若有）失效 | 冒烟只看 `assembled` 与 `status`，不判其它 |

---

## 五、全局约束

1. **只改 `orion-simulation-world/`**（Step 3 的冒烟可写 `RobotSim_TestEnv/` 日志，已 gitignore）。
2. 不动 `pleiades-sim/` 与其它 crate；本任务与行为逻辑无关。
3. 归属注释：`.gd` / `.md` 新建文件都要 `Presented by KeJi / Created Date / Modified Date`（见 `.github/instructions.md`）。
4. 命名：函数 `PascalCase`、变量 `snake_case`、常量 `UPPER_SNAKE`（`COMPONENTS.md` §9.2）。
5. 不执行 `git add` / `git commit`。

## 六、完成判据（整任务）

- 目录：`src/` + `assets/` 就位，根下无遗留 `scripts/ chassis/ sensors/ tests/`；
- 世界：`--quit` EXIT=0、0 ERROR；**10 个自检**（原 9 + `check_bounds`）全 `SELF-CHECK OK`；
- 边界：20 m/s 四面冲撞与 40 m 爬升都留在界内；
- 联调：世界 + sim 真跑装配成功且进程干净退出；
- 文档：`COMPONENTS.md` 与代码一致（目录树、路径、边界、UAV 高度、E24）。
