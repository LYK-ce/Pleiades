# orion-simulation-world 组件总览（COMPONENTS.md）

> Presented by KeJi
> Created Date ： 2026-10-21
> Modified Date ： 2026-10-22
>
> 对应任务：`Task/task_30_sim_world.md`（sim 接入 Godot 模拟世界）
> 状态：**Step 1 产出；Step 2（世界）/ Step 3（观察者相机）/ Step 4（Vehicle）/ Step 5（差速底盘 `DiffDrive`）/ Step 6（`WebSocketManager`）已按本文落地**。本文是 Step 2~8 的唯一施工依据；后续 step 若需偏离，必须**先改本文并说明理由**。
> 引擎：Godot **4.7.2**（`4.7.2.stable.custom_build.ed1daf0bf`），渲染后端 `gl_compatibility`。

---

## 0 文档定位

### 0.1 与 task_30 的对应关系

| task_30 章节 | 本文对应 | 说明 |
|---|---|---|
| §三 核心模型（avatar / sim / 世界权威 / 认知来自传感器） | §6 数据流 | 世界只负责"身体 + 物理 + 传感"，决策链在 sim |
| §四 架构 + §4.1 组件分解 | §2 / §3 / §4 | 世界组件、Vehicle、相机、manager 逐个落成 `.tscn` |
| §4.2 观察者相机 | §4.2 | 三种视角、输入映射、与仿真解耦 |
| §五 装配流程 + §5.1 manager | §4.5 / §5 / §6.1 | 两张表、pending 传感器、manifest 就绪 |
| §六 通道设计 / 消息 schema | §8 消息契约 | JSON 文本帧 + 二进制帧头（小端） |
| §七 已定决策 D1~D11 / §7.1 尺度 / §7.2 零件目录 / §7.3 坐标 / §7.4 LiDAR | §1.2 / §6.3 / §7.3 / §8 | 尺度与端口集中到 `project.godot` 的 `[sim_world]` |
| §八 实现步骤（Step 2~8） | §11 step ↔ 文件映射 | 每个 step 只动自己那一行 |
| §九 记录（3D 世界 ≠ 3D 决策） | §10 v1 边界 | UAV 高度控制 / 三维寻路不在本任务 |

### 0.2 组件化铁律

1. **一个组件 = 一个 `.tscn` + 一个同名脚本**；场景不内嵌脚本代码（不写 `[sub_resource type="GDScript"]`）。
2. 组件清单固定 **7 个 `.tscn`**（§3），Step 1 只落 `src/scenes/World.tscn` 的**占位**版本，其余 6 个由 Step 3~8 产出。
3. **全项目不使用全局 `class_name`**（见 §9.1 E5）。跨脚本引用一律 `const X := preload("res://…gd")`，类型继承一律 `extends "res://…gd"`。
4. 坐标、时间、消息格式只有**唯一入口**：`WorldTransform` / `WorldClock` / `WireProtocol`。
5. 世界尺度、端口、物理频率只有**唯一来源**：`project.godot` 的 `[sim_world]`（禁止在脚本里散落字面量）。

---

## 1 工程骨架

### 1.1 目录树

```
orion-simulation-world/
├── project.godot                 # 工程配置（唯一来源：尺度 / 端口 / 物理层 / 输入映射）
├── COMPONENTS.md                 # 本文
├── .gitignore                    # 已有：.godot/ .import/ export*.cfg …
├── src/                          # 代码与场景的唯一根（task_32 Step 1 起）
│   ├── scenes/                   # 全部 .tscn（组件场景，§3 的 7 个）
│   │   ├── World.tscn            # 主场景（Step 2 完成态；Step 3/6 加挂点实例）
│   │   ├── ObserverCamera.tscn   # [Step 3] ✅ 已落地
│   │   ├── Vehicle.tscn          # [Step 4] ✅ 已落地
│   │   ├── WebSocketManager.tscn # [Step 6] ✅ 已落地
│   │   ├── chassis/
│   │   │   ├── DiffDrive.tscn    # [Step 5] ✅ 已落地
│   │   │   └── Quadrotor.tscn    # [Step 7] ✅ 已落地
│   │   └── sensors/
│   │       └── Lidar2D.tscn      # [Step 8] ✅ 已落地
│   ├── common/                   # WorldTransform.gd / WorldClock.gd            [Step 2]
│   ├── world/                    # World.gd                                      [Step 2]
│   ├── camera/                   # ObserverCamera.gd                             [Step 3] ✅
│   ├── vehicle/                  # Vehicle.gd                                    [Step 4] ✅
│   ├── chassis/                  # ChassisModel.gd / DiffDrive.gd / ChassisCatalog.gd  [Step 5] ✅
│   │                             # Quadrotor.gd                                  [Step 7] ✅
│   ├── sensors/                  # Lidar2D.gd / SensorCatalog.gd                 [Step 8] ✅
│   ├── net/                      # WebSocketManager.gd / ComponentLink.gd / WireProtocol.gd  [Step 6] ✅
│   └── tests/
│       ├── check_project.gd      # 工程骨架自检（Step 1，非组件）
│       ├── check_world.gd        # 世界自检（Step 2，非组件）
│       ├── check_camera.gd       # 相机自检（Step 3，非组件）
│       ├── check_vehicle.gd      # Vehicle 自检（Step 4，非组件；36 项断言）
│       ├── check_diff_drive.gd   # DiffDrive 自检（Step 5，非组件；41 项断言）
│       ├── check_ws_manager.gd   # WebSocketManager 自检（Step 6，非组件；51 项断言）
│       ├── check_quadrotor.gd    # Quadrotor 自检（Step 7，非组件；48 项断言）
│       ├── check_lidar.gd        # Lidar2D 自检（Step 8，非组件；48 项断言）
│       ├── check_pose.gd         # 位姿下行自检（task_31 Step 1，非组件；23 项断言）
│       ├── check_bounds.gd       # 空气墙 / 顶盖自检（task_32 Step 2，非组件；12 项断言）
│       └── doubles/              # 测试替身（**不是组件**、不落 .tscn）
│           ├── TestChassis.gd    #   底盘鸭子契约：Configure / GetDefaultParams / SetCommand / Step
│           └── TestSensor.gd     #   传感器鸭子契约：Configure / Sample / ScanOnce
└── assets/                       # 只放真实资源（png / glb / obj / material / wav …）
    ├── .gitkeep                  #   空目录占位（当前无资源文件）
    └── README.md                 #   目录边界说明：代码与场景一律 src/
```

`src/tests/doubles/` 只被 `src/tests/check_vehicle.gd` 用 `PackedScene.pack()` 现场打包，不进 §3 的 7 个 `.tscn` 清单。

**目录铁律（task_32 Step 1）**：代码（`.gd`）与场景（`.tscn`）一律在 `src/` —— `src/scenes/` 放 `.tscn`、`src/<模块>/` 放 `.gd`、`src/tests/` 放自检；`assets/` 只放媒体与建模资源。`assets/` 用 `.gitkeep` 占位；`src/` 各目录均有真实文件，不需占位（Godot 忽略未知扩展名，不影响加载）。

### 1.2 `project.godot` 关键段（已落地，Step 2~8 只读不改）

```ini
config_version=5

[application]
config/name="Orion Simulation World"
run/main_scene="res://src/scenes/World.tscn"
config/features=PackedStringArray("4.7", "GL Compatibility")

[display]
window/size/viewport_width=1600
window/size/viewport_height=900

[input]                     ; 11 个相机动作（W/S/A/D/E/Q/Shift/F1/F2/F3/Tab）

[layer_names]
3d_physics/layer_1="world_ground"
3d_physics/layer_2="world_obstacle"
3d_physics/layer_3="vehicle"

[physics]
common/physics_ticks_per_second=60
3d/default_gravity=9.8

[rendering]
renderer/rendering_method="gl_compatibility"
anti_aliasing/quality/msaa_3d=2
environment/defaults/default_clear_color=Color(0.09, 0.11, 0.14, 1)

[sim_world]
network/ws_port=9999
world/voxel_size=0.5
world/cells_x=256
world/cells_y=256
world/layers=64
```

设定理由：

| 项 | 值 | 理由 |
|---|---|---|
| `config_version` | `5` | Godot 4.7 工程格式 |
| `rendering_method` | `gl_compatibility` | 远程 / 无独显环境也能开窗；本工程无 Vulkan 特性需求 |
| `physics_ticks_per_second` | 60 | 决策 tick 50 ms、LiDAR 10 Hz；60 能被 10/20/30 整除，传感器用"时间累加器"而非固定帧数比 |
| `default_gravity` | 9.8 | 与真实一致；UAV 底盘不施加重力（§4.6），故不受影响 |
| `world/voxel_size` | 0.5 m | = `Robot/core/grid.rs` 的 `CELL_RESOLUTION`（D5） |
| `world/cells_x/cells_y` | 256 / 256 | 0.5 m × 256 = 128 m（= `pleiades-sim` `WORLD_MIN/MAX`） |
| `world/layers` | 64 | 0.5 m × 64 = 32 m 垂直（D9） |
| `network/ws_port` | 9999 | 避开仓库已用的 9010（webui）/ 8080（HTTP API），以及本机被 `mihomo` 占用的 9090（原端口） |

> **尺度换算的唯一公式**：米 → 格 = `value / voxel_size`；格 → 米 = `cell * voxel_size`。脚本一律 `ProjectSettings.get_setting("sim_world/…")` 读取。

### 1.3 输入映射（相机专用，11 条，Step 3 直接使用）

| 动作名 | 键 | physical_keycode | 用途 |
|---|---|---|---|
| `cam_forward` | W | 87 | 自由飞行前移 |
| `cam_back` | S | 83 | 自由飞行后移 |
| `cam_left` | A | 65 | 自由飞行左移 |
| `cam_right` | D | 68 | 自由飞行右移 |
| `cam_up` | E | 69 | 自由飞行上升 |
| `cam_down` | Q | 81 | 自由飞行下降 |
| `cam_boost` | Shift | 4194325 | 加速（速度倍率） |
| `cam_mode_free` | F1 | 4194332 | 切自由飞行 |
| `cam_mode_orbit` | F2 | 4194333 | 切环绕 |
| `cam_mode_follow` | F3 | 4194334 | 切跟随 |
| `cam_target_next` | Tab | 4194306 | 切换观察目标 |

鼠标手势（右键拖拽转向 / 中键拖拽环绕 / 滚轮缩放）**不进 input map**，在 `ObserverCamera._unhandled_input` 里直接读 `InputEventMouseButton` / `InputEventMouseMotion`。

### 1.4 运行与校验命令

```bash
cd <repo-root>

# 加载校验（Step 1 硬验收；期望 EXIT=0、stderr 为空）
godot --headless --path orion-simulation-world --quit

# 日志无脚本/解析错误（--quit 对 parse error 仍返回 0，必须 grep）
godot --headless --path orion-simulation-world --quit 2>&1 | grep -nE "SCRIPT ERROR|Parse Error|ERROR"

# 工程骨架内容自检（期望 SELF-CHECK OK + EXIT=0）
godot --headless --path orion-simulation-world --script res://src/tests/check_project.gd

# 组件自检（Step 4/5/6；--fixed-fps 60 让"帧数 -> 位移"的断言可复现；命令一律带 timeout）
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_vehicle.gd
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_diff_drive.gd
# WebSocketManager：同一进程内既当世界又当 sim（真 World + 真 WebSocketPeer 客户端）；
# 用 Listen(0) 取 OS 临时端口，不抢默认端口 9999（被占只 warning 不 ERROR）
timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_ws_manager.gd
# 位姿下行（task_31 Step 1）：10 Hz pose 帧 / 字段 / 与 spawn 一致 / 只发底盘连接
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_pose.gd
# 空气墙 / 顶盖（task_32 Step 2）：5 个 box、layer 1、四面 20 m/s 冲撞与 32 m 顶盖
timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_bounds.gd

# 单文件脚本语法检查（Step 2+ 推荐）
# 注意：裸 `--check-only`（不带 `--script`）不检查任何脚本，而是把主场景跑起来且**永不退出**；
#       必须写成 `--check-only --script res://src/<模块>/<文件>.gd`（逐文件循环即覆盖全部脚本）
godot --headless --path orion-simulation-world --check-only --script res://src/world/World.gd

# 可视化运行（本地 / 远程桌面开窗）
godot --path orion-simulation-world
```

`src/tests/check_project.gd` 校验：引擎 4.7.x、`run/main_scene` 存在且可加载、`[sim_world]` 五个键值、`physics_ticks_per_second`。失败逐行打印 `[fail]` 并 `exit 1`，通过打印 `SELF-CHECK OK`。

> Step 6 起 `src/scenes/World.tscn` 会**自动监听 9999**（`autolisten=true`）。本机 9999 若已被别的进程占用，日志只有一条 `WARNING [ws] listen(port=9999) 失败：Already in use`，世界照常运行；`--quit` 与 `grep ERROR` 验收不受影响（§4.5.1-⑥）。
>
> 端口变更记录：初版定为 9090，但本机 9090 被 `mihomo` 占用，故改为 **9999**。

---

## 2 全场景节点树（World.tscn 为根）

`src/scenes/World.tscn` 是唯一主场景（`run/main_scene`）。下面是**目标态**（Step 6 完成后的完整树），并标注每一步在此树上新增什么。

```
World (Node3D)                              script: res://src/world/World.gd            [Step 2]
│                                           职责：启动 WorldClock、建体素/地面、持有 Vehicles 容器
├── WorldEnvironment (WorldEnvironment)      Environment 内联资源：ProceduralSky + ambient    [Step 2]
├── Sun (DirectionalLight3D)                 shadow_enabled=true，俯角照明                    [Step 2]
├── Ground (StaticBody3D)                    collision_layer=1, collision_mask=0               [Step 2]
│   ├── CollisionShape3D                     BoxShape3D size=(128,1,128)，中心 (64,-0.5,64)      [Step 2]
│   └── MeshInstance3D                       BoxMesh 同尺寸（可视地面）                          [Step 2]
├── VoxelWorld (GridMap)                     cell_size=(0.5,0.5,0.5)，collision_layer=2,
│                                            collision_mask=0，mesh_library 由 World.gd 运行时构建  [Step 2]
├── Bounds (StaticBody3D)                    collision_layer=1, collision_mask=0；5 个 CollisionShape3D  [task_32 Step 2]
│   └── （无 mesh 子节点）                    南/北/西/东 4 面 **32 m 空气墙** + **32 m 顶盖**；
│                                            由 `_Build_Bounds()` **运行时**建，**不在本 .tscn 里**；
│                                            不可见（无 MeshInstance3D，§9.1 E24）
├── Vehicles (Node3D)                        运行时 avatar 容器（由 WebSocketManager 填充）       [Step 6]
│   └── <peer_id> (Vehicle.tscn 实例)        动态；见 §4.3                                   [Step 6]
├── Systems (Node3D)                                                                          [Step 6]
│   └── WebSocketManager (instance: res://src/scenes/WebSocketManager.tscn)             [Step 6] ✅
│                                            @export vehicle_scene → res://src/scenes/Vehicle.tscn
│                                            @export vehicles_root → ../../Vehicles
│                                            （node_paths=PackedStringArray("vehicles_root")）
│                                            ※ **相对该节点自身**：.. = Systems，../.. = World（§9.1 E20）
└── ObserverCamera (instance: res://src/scenes/ObserverCamera.tscn)                     [Step 3] ✅
                                             current=true
                                             ※ Step 2 期间临时用 DebugCamera (Camera3D，写死 transform) 顶替；
                                               Step 3 已删除 DebugCamera，本行即当前态
```

> **`DebugCamera` 命名偏差（实测，Step 3 已删除该节点）**：`ClassDB.class_exists("DebugCamera3D") == false`
> —— 该名字在 4.7.2 里是编辑器内部类，运行时不注册。Step 2 因此用普通 `Camera3D` 命名 `DebugCamera`
> （`position=(64,40,124)`、`rotation_degrees=(-33.69,0,0)`、`current=true`），行为等价、无脚本依赖；
> Step 3 删除该节点并换成 `src/scenes/ObserverCamera.tscn` 实例（`src/tests/check_world.gd` 的 `debug-camera-removed`
> 与 `src/tests/check_camera.gd` 的同名断言会拦住它复辟）。

### 2.1 各节点说明

| 节点 | 类型 | 脚本 | 静态/运行时 | 说明 |
|---|---|---|---|---|
| `World` | `Node3D` | `src/world/World.gd` | 静态 | 场景根；`_ready()` 里 `WorldClock.EnsureStarted()` + `BuildVoxelWorld()` |
| `WorldEnvironment` | `WorldEnvironment` | — | 静态 | `Environment` 内联（`ProceduralSkyMaterial` + `ambient_light_source=SKY`） |
| `Sun` | `DirectionalLight3D` | — | 静态 | 俯角约 -55°，`shadow_enabled=true` |
| `Ground` | `StaticBody3D` | — | 静态 | **一整块**地面，非体素地面（D10；避免 6.5 万碰撞体） |
| `Ground/CollisionShape3D` | `CollisionShape3D` | — | 静态 | `BoxShape3D(128,1,128)`；`position=(64,-0.5,64)` → 顶面 `y=0` |
| `Ground/MeshInstance3D` | `MeshInstance3D` | — | 静态 | `BoxMesh` 同尺寸；材质深灰 |
| `VoxelWorld` | `GridMap` | — | 静态节点 / 运行时填格 | `cell_center_*=true`；`mesh_library` 运行时构建 |
| `Bounds` | `StaticBody3D` | —（`World.gd` 内建） | **运行时** | 不可见硬边界；`collision_layer=1`、`collision_mask=0`；5 个 `CollisionShape3D`（4 面 32 m 空气墙 + 32 m 顶盖）；**内侧面贴世界边界平面**（`x=0/128`、`y=0/128`、`z=32`），厚度一律向外（§9.1 E24）；无任何 mesh 子节点 |
| `Vehicles` | `Node3D` | — | 静态容器 | 所有 avatar 的父节点；子节点名 = `peer_id`（净化后） |
| `Systems` | `Node3D` | — | 静态容器 | 非物理系统（网络等）的父节点 |
| `Systems/WebSocketManager` | `Node` | `src/net/WebSocketManager.gd` | 静态实例 | **全进程唯一** manager，管全部连接；子节点 `Heartbeat(Timer, 1 s)`；`autolisten=true`、`listen_port=0`（→ `[sim_world]` 的 9999） |
| `ObserverCamera` | `Camera3D` | `src/camera/ObserverCamera.gd` | 静态实例 | 观察者相机；不参与任何物理/传感计算 |

### 2.2 各步在此树上的增量

| Step | 在 `src/scenes/World.tscn` 上的动作 |
|---|---|
| 1 | 只落根节点 `World (Node3D)`（占位，无脚本） |
| 2 | 挂 `src/world/World.gd`；加 `WorldEnvironment` / `Sun` / `Ground`(+2 子) / `VoxelWorld` / `Vehicles` / `Systems`；临时 `DebugCamera`（`Camera3D`）供观察 |
| 3 | 加 `ObserverCamera` 实例；移除 `DebugCamera` |
| 4 | 不改（`src/scenes/Vehicle.tscn` 独立产出） |
| 5 | 不改（`src/scenes/chassis/DiffDrive.tscn` 独立产出） |
| 6 | **已执行**：在 `Systems` 下加 `WebSocketManager` 实例（`node_paths=PackedStringArray("vehicles_root")`），绑定 `vehicle_scene = res://src/scenes/Vehicle.tscn`、`vehicles_root = ../../Vehicles`；`load_steps` 9 → 11 |
| 7 / 8 | 不改（`src/scenes/chassis/Quadrotor.tscn` / `src/scenes/sensors/Lidar2D.tscn` 独立产出） |
| task_32-2 | **不改 .tscn**：`World.gd` 的 `_Build_Bounds()` 在运行时把 `Bounds`（5 个 box，layer 1）挂到 `World` 下；可见的 4 面 2 m 体素矮墙保留不动 |

### 2.3 世界坐标 ↔ Godot 坐标（写死，勿改）

世界：**Z-up**，`x`=东、`y`=南、`z`=高，yaw 绕 `z`（从 +x 起，逆时针为正）。
Godot：**Y-up**，前进轴固定 **局部 +X**。

```
Godot.position  = Vector3(world.x,  world.z,  world.y)
Godot.rotation.y = -world.yaw
```

- `Ground` 中心 `global_position=(64,-0.5,64)`，尺寸 `128×128×1` → 顶面在 Godot `y=0`，与世界 `z=0` 地面重合。
- `VoxelWorld` 原点在 Godot `(0,0,0)`，`cell_center_*=true` → 第 `i` 格覆盖 `[i·0.5, (i+1)·0.5)`。

体素索引（含 **y↔z 交换**）：

```gdscript
cell = Vector3i(floor(wx / v), floor(wz / v), floor(wy / v))   # v = voxel_size
```

---

## 3 组件 `.tscn` 清单

固定 **7 个**（路径与 task_30 §八 完全一致，不增不减）：

| # | 文件（`res://`，相对工程根） | 根节点类型 | 脚本 | 职责 | Step | 静态挂载位置 | 运行时实例化者 |
|---|---|---|---|---|---|---|---|
| 1 | `src/scenes/World.tscn` | `Node3D` | `src/world/World.gd` | 根场景：地面 / 体素 / 环境 / 容器 | 2 | 主场景（`run/main_scene`） | 引擎 |
| 2 | `src/scenes/ObserverCamera.tscn` | `Camera3D` | `src/camera/ObserverCamera.gd` | 观察者相机（自由 / 环绕 / 跟随） | 3 | `World/ObserverCamera` | 静态实例 |
| 3 | `src/scenes/Vehicle.tscn` | `CharacterBody3D` | `src/vehicle/Vehicle.gd` | avatar 身体：物理 + 装配接口 | 4 ✅ | — | `WebSocketManager` → `World/Vehicles` |
| 4 | `src/scenes/chassis/DiffDrive.tscn` | `Node3D` | `src/chassis/DiffDrive.gd` | 差速底盘（`GROUNDED`） | 5 ✅ | — | `Vehicle.ApplyChassis()` → `Vehicle/Chassis` |
| 5 | `src/scenes/WebSocketManager.tscn` | `Node` | `src/net/WebSocketManager.gd` | WS 服务端：装配 + 收发 | 6 ✅ | `World/Systems/WebSocketManager` | 静态实例 |
| 6 | `src/scenes/chassis/Quadrotor.tscn` | `Node3D` | `src/chassis/Quadrotor.gd` | 四旋翼底盘（`FLOATING`） | 7 ✅ | — | `Vehicle.ApplyChassis()` → `Vehicle/Chassis` |
| 7 | `src/scenes/sensors/Lidar2D.tscn` | `Node3D` | `src/sensors/Lidar2D.gd` | 2D 单线激光（真射线） | 8 ✅ | — | `Vehicle.AttachSensor()` → `Vehicle/Sensors/<slot>` |

非组件脚本（无 `.tscn`）：

| 文件 | 类型 | 职责 |
|---|---|---|
| `src/world/World.gd` | 挂 `World` | 世界时钟启动、体素/地面程序化生成、尺度日志 |
| `src/common/WorldTransform.gd` | 静态工具 | **坐标转换唯一入口**（§7.3） |
| `src/common/WorldClock.gd` | 静态工具 | 统一 `world_time`（单调墙钟） |
| `src/vehicle/Vehicle.gd` | 挂 `Vehicle` | 装配、物理步进、位姿查询（§4.3） |
| `src/chassis/ChassisModel.gd` | 底盘基类 | 底盘接口契约 + 速度环 / 指令解析公共工具（§7.5） |
| `src/chassis/ChassisCatalog.gd` | 静态目录 | `chassis` 名 → 场景路径 / 默认参数（运行时 `load()`；Step 5 ✅） |
| `src/chassis/DiffDrive.gd` / `Quadrotor.gd` | 底盘实现 | 运动学 + 速度环（`DiffDrive` ✅ Step 5 / `Quadrotor` ✅ Step 7） |
| `src/sensors/SensorCatalog.gd` | 静态目录 | `sensor` 名 → 场景路径 / 默认参数（运行时 `load()`；Step 8 ✅） |
| `src/sensors/Lidar2D.gd` | 挂 `Lidar2D` | 射线扫描 + 二进制下发（Step 8 ✅） |
| `src/net/WebSocketManager.gd` | 挂 manager | 监听 / accept / 两张表 / 装配 / manifest / 清理（Step 6 ✅） |
| `src/net/ComponentLink.gd` | `RefCounted` | 单条连接封装与状态（Step 6 ✅） |
| `src/net/WireProtocol.gd` | 静态工具 | 消息名、JSON 编解码、二进制帧构造 / 解析（Step 6 ✅） |
| `src/tests/check_project.gd` | `SceneTree` 脚本 | 工程骨架自检（Step 1） |
| `src/tests/check_world.gd` | `SceneTree` 脚本 | 世界自检（Step 2） |
| `src/tests/check_camera.gd` | `SceneTree` 脚本 | 相机自检（Step 3） |
| `src/tests/check_diff_drive.gd` | `SceneTree` 脚本 | 差速底盘自检（Step 5，41 项断言） |
| `src/tests/check_quadrotor.gd` | `SceneTree` 脚本 | 四旋翼底盘自检（Step 7，48 项断言） |
| `src/tests/check_ws_manager.gd` | `SceneTree` 脚本 | WebSocketManager 自检（Step 6，51 项断言；真 World + 真 WS 客户端） |
| `src/tests/check_lidar.gd` | `SceneTree` 脚本 | Lidar2D 自检（Step 8，48 项断言；真 World + 自建 box 障碍） |
| `src/tests/check_bounds.gd` | `SceneTree` 脚本 | 空气墙 / 顶盖自检（task_32 Step 2，12 项断言） |

---

## 4 每个组件的节点树 + 接口

> 全项目**不声明、不引用全局 `class_name`**。跨脚本引用：`const WorldTransform := preload("res://src/common/WorldTransform.gd")`；继承：`extends "res://src/chassis/ChassisModel.gd"`。

### 4.1 `src/scenes/World.tscn`（Step 2）

节点树见 §2。脚本职责：启动世界时钟、程序化搭世界、提供坐标工具与容器访问。

```gdscript
# src/world/World.gd
extends Node3D

const WorldClock := preload("res://src/common/WorldClock.gd")

# task_32 Step 2：不可见硬边界（空气墙 / 顶盖）
const AIR_WALL_HEIGHT_M := 32.0
const AIR_WALL_THICKNESS_M := 1.0
const AIR_WALL_LAYER := 1

func _ready() -> void                        # WorldClock.EnsureStarted()；读 [sim_world] 打印尺度；BuildVoxelWorld() → _Build_Bounds()
func BuildVoxelWorld() -> void               # 建 MeshLibrary(0.5³ 立方 + BoxShape3D) → $VoxelWorld；稀疏填障碍
func _Build_Bounds() -> void                 # 建 `Bounds`(StaticBody3D) + 5 个 CollisionShape3D；layer 1、mask 0；幂等；**不建 mesh**
func WorldToCell(world_pos: Vector3) -> Vector3i   # (floor(x/v), floor(z/v), floor(y/v))
func GetVehiclesRoot() -> Node3D             # $Vehicles
```

体素构建要点（D4 程序化生成，实测可行）：

```gdscript
var lib := MeshLibrary.new()
lib.create_item(0)
lib.set_item_mesh(0, BoxMesh.new())                       # size = Vector3.ONE * v
lib.set_item_shapes(0, [BoxShape3D.new(), Transform3D.IDENTITY])
$VoxelWorld.mesh_library = lib
$VoxelWorld.cell_size = Vector3(v, v, v)
$VoxelWorld.set_cell_item(cell, 0)                        # 只填墙/障碍，绝不 256×256×64 全填
```

由引擎创建 / 销毁。`VoxelWorld` 的 `collision_layer=2`、`collision_mask=0`（静态几何，不主动检测）。

**空气墙 / 顶盖几何（task_32 Step 2，写死，不得自行发挥）**

5 个 `BoxShape3D` 由 `World.gd` 的 `AIR_WALL_BOXES` 常量在运行时建进 `World/Bounds`（`StaticBody3D`，
`collision_layer=1`、`collision_mask=0`）；`.tscn` 里**一个节点都不加**（绕开 §9.1 E16 的 `node_paths` 坑）。

| 件 | 世界坐标范围（Z-up，米） | Godot `position` | `size` | 内侧面（贴边界平面） |
|---|---|---|---|---|
| 南墙 | `x∈[0,128], y∈[-1,0], z∈[0,32]` | `(64, 16, -0.5)` | `(128, 32, 1)` | `z=0`（+z 面） |
| 北墙 | `x∈[0,128], y∈[128,129], z∈[0,32]` | `(64, 16, 128.5)` | `(128, 32, 1)` | `z=128`（−z 面） |
| 西墙 | `x∈[-1,0], y∈[0,128], z∈[0,32]` | `(-0.5, 16, 64)` | `(1, 32, 128)` | `x=0`（+x 面） |
| 东墙 | `x∈[128,129], y∈[0,128], z∈[0,32]` | `(128.5, 16, 64)` | `(1, 32, 128)` | `x=128`（−x 面） |
| 顶盖 | `z∈[32,33]` | `(64, 32.5, 64)` | `(128, 1, 128)` | `y=32`（−y 面，即下表面） |

可见的 2 m 体素矮墙（`OBSTACLES` 前 4 项，layer 2）**保留不动**；空气墙与地面同层 layer 1
（`Lidar2D` 的 `scan_mask = 1|2|4` 因此也能读到它）。碰撞面一律**贴世界边界平面、厚度向外**——
车中心永远 `≥` 边界平面，`WorldToCell` 不会出负格坐标（§9.1 E24）。

### 4.2 `src/scenes/ObserverCamera.tscn`（Step 3 ✅）

```
ObserverCamera (Camera3D)          script: res://src/camera/ObserverCamera.gd
                                   position=(64, 26, 108)、rotation_degrees=(-30, 0, 0)
                                   current=true, fov=70, near=0.05, far=500
```

默认位姿已实测：该位姿下 `is_position_in_frustum(Vector3(64, 0, 64)) == true`（初始视角就能看见世界中心地面）。
场景里**只写这 6 个引擎原生属性**，脚本 `@export` 一律用默认值、不写进 `.tscn`（属性改名不会静默失效）。

```gdscript
extends Camera3D

enum Mode { FREE = 0, ORBIT = 1, FOLLOW = 2 }

# —— 公开接口（自检与 Step 6 的唯一契约面）
func SetMode(p_mode: int) -> bool                   # 切视角；非法值 push_warning + false（模式不变）
func GetMode() -> int
func GetModeName() -> String                        # "FREE" / "ORBIT" / "FOLLOW"
func SetTarget(p_target: Node3D) -> void            # null 合法；不在 vehicles_group 只告警
func GetTarget() -> Node3D                          # 目标被释放后自动返回 null（防悬空引用）
func GetTargetName() -> String                      # 无目标 "<none>"
func SelectNextTarget() -> Node3D                   # Tab：按名字稳定排序循环；空组返回 null
func GetFocusPoint() -> Vector3                     # 注视点：有目标 → 目标位 + UP*follow_height
func GetViewState() -> Dictionary                   # {mode, mode_name, target, distance, focus}
func _unhandled_input(event: InputEvent) -> void    # 唯一输入入口（不抢 UI 输入）
func _physics_process(delta: float) -> void         # 视角更新；不写物理世界
```

`@export`（默认值即最终值）：`control_enabled`(=true；false 时相机完全冻结，自检用它做对照组)、
`vehicles_group`(= `&"vehicle"`)、`free_speed`(=12)、`free_speed_min/max`(1/60)、`free_speed_step`(=1.15)、
`look_sensitivity`(=0.0035 rad/px)、`pitch_limit_deg`(=88)、`orbit_distance`(=18)、`orbit_distance_min/max`(2/80)、
`orbit_smoothing`(=12)、`follow_distance`(=8)、`follow_height`(=3)、`follow_smoothing`(=8)、
`orbit_fallback_focus`(= `(64, 1, 64)`，无目标时的环绕中心)。

三种视角（算法要点，均有自检覆盖）：

| 模式 | 位置更新 | 输入 |
|---|---|---|
| `FREE` | `pos += (Basis(pitch,yaw,0)*(ix,0,iz) + UP*iy) * speed * dt`；`wish` 超长先归一化；前向 = 相机视线 `-basis.z`（"往哪看往哪飞"），Q/E 走绝对世界上下 | WASD + Q/E + Shift(×3) + 右键拖拽转向 + 滚轮调速 |
| `ORBIT` | `pos = focus + Basis(orbit_pitch, orbit_yaw, 0) * (0,0,dist)`，指数平滑靠拢后 `look_at(focus)` | 中键拖拽改 yaw/pitch（±80°）+ 滚轮改半径 |
| `FOLLOW` | `pos = tgt.pos + UP*follow_height + (车后水平方向绕 Y 转 follow_yaw) * follow_dist`，平滑后看向 `tgt.pos + UP*follow_height/2` | 滚轮改 `follow_dist` |

- **车辆前向是局部 `+X`**（§6.5），故"车后方向"取 `-tgt.basis.x` 并水平化；若误用 `-basis.z` 会变成正面跟车。
- 平滑权重 `1 - exp(-k*dt)`，帧率无关；插值不引入物理副作用。
- `look_at` 前有退化守卫（与注视点距离 < 1 mm、或视线与 `UP` 共线时跳过），避免抖动。
- **切模式不瞬移**：`_sync_state_from_transform()` 从当前 `transform` 反推新模式全部状态
  （ORBIT 反推 yaw/pitch/dist；FOLLOW 反推水平距离；FREE 反推 yaw/pitch），实测切模式首帧位移 = 0。
- 模式键 `F1/F2/F3`，目标 `Tab`（v1 用 group 循环；`SetTarget()` 已留好 Step 6 按 `peer_id` 指定的入口）。
- **输入链路**（task_30 §4.2）：离散事件（`F1/F2/F3`、`Tab`、鼠标按键/滚轮/位移）全部经 `_unhandled_input`
  处理，命中后 `set_input_as_handled()`；连续按键（WASD/QE/Shift）用 `Input.get_axis` /
  `Input.is_action_pressed` 在 `_physics_process` 里轮询。**全项目不出现 `_input` / `_gui_input`**
  （`decoupling-static` 白盒断言），所以永远不抢 UI 输入。
- 打印纪律：只在 `_ready` 与模式/目标切换时各打 1 行，不逐帧打印。
- **与仿真完全解耦**（可验证形式）：脚本零 `preload`、无任何物理 API、不进 `vehicle` 组、不发不收信号、
  位置与朝向只被自己写且无人读；`src/tests/check_camera.gd` 的 `decoupling-*` 与 `physics-*-invariant` 是量化证据。
- 静态实例，挂在 `World/ObserverCamera`；`World.gd` 完全不感知它。

运行自检（期望 `total=32 failed=0` + `SELF-CHECK OK` + `EXIT=0`）：

```bash
godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_camera.gd
```

### 4.3 `src/scenes/Vehicle.tscn`（Step 4 ✅）

```
Vehicle (CharacterBody3D)            script: res://src/vehicle/Vehicle.gd
                                     groups=["vehicle"]
                                     collision_layer=4 (vehicle)、collision_mask=7 (1|2|4)
                                     motion_mode 由底盘 Configure() 设定（UGV GROUNDED / UAV FLOATING）
                                     node_paths=PackedStringArray("body", "collision_shape",
                                                                  "chassis_root", "sensors_root")
├── Body (MeshInstance3D)            position=(0, 0.08, 0)；占位 BoxMesh(0.30, 0.16, 0.24) + 橙色材质
│   └── Nose (MeshInstance3D)        position=(0.15, 0, 0)；BoxMesh(0.06, 0.10, 0.14) 红 → 朝向标记（车头 = 局部 +X）
├── CollisionShape3D                 position=(0, 0.08, 0)；占位 BoxShape3D(0.30, 0.16, 0.24)
├── Chassis (Node3D)                 ← 底盘挂载点（运行时**至多一个**子节点）
└── Sensors (Node3D)                 ← 传感器挂载点（运行时子节点名 = slot）
```

**原点约定（写死）**：`Vehicle` 原点 = 车体**底面中心**；`Body` 与 `CollisionShape3D` 都上抬 `height/2`
（占位 `height=0.16` → `0.08`）。世界 `z=0` ↔ Godot `y=0`，所以 UGV 出生 `y=0` 就站在地面上。
`BoxMesh` 默认**居中**，因此 Body 必须自己上抬，**不要**把 Body 留在原点（否则半个车身埋进地面，
射线也会从地下发出）。`Chassis` / `Sensors` 初始**没有子节点**——空车 = 只有壳与两个挂载点。

**手写 `.tscn` 的两条硬约束**（§9.1 E8 / E16）：

- 4 个 `@export` 节点引用必须在根节点行写
  `node_paths=PackedStringArray("body", "collision_shape", "chassis_root", "sensors_root")`，
  并逐个 `body = NodePath("Body")` ……；**漏写 `node_paths` 会静默为 null**（不报错、不告警）。
  `Vehicle._ensure_refs()` 只按子节点名兜底一次，自检 A5 直接断言"绑定成功"。
- 材质挂 `surface_material_override/0`（`MeshInstance3D` 的属性），**不要**写进 `BoxMesh.material`——
  底盘 `SetBodySize()` 换 `mesh` 时材质才不会丢。

```gdscript
extends CharacterBody3D

signal chassis_installed(chassis_name: String)      # 底盘装配完成
signal sensor_attached(slot: String, sensor: Node)  # 传感器挂载完成

@export var body: MeshInstance3D = null
@export var collision_shape: CollisionShape3D = null
@export var chassis_root: Node3D = null
@export var sensors_root: Node3D = null

var _chassis: Node3D = null          # 底盘策略节点（Chassis 的唯一子节点）
var _chassis_name: String = ""       # 零件名，如 "diff_drive_ugv"
var _chassis_params: Dictionary = {} # 生效参数 = 底盘默认值 <- 传入值
var _cmd: Dictionary = {}            # 最新一条 motion_cmd（只保留最新，不排队）
var _sensors: Dictionary = {}        # slot(String) -> Node3D
var _nose: MeshInstance3D = null     # Body/Nose（SetBodySize 时跟着摆到车头）

# ---- 装配
func ApplyChassis(chassis_name: String, params: Dictionary = {},
        chassis_scene: PackedScene = null) -> bool
func AttachSensor(slot: String, sensor_scene: PackedScene, mount: Dictionary = {},
        params: Dictionary = {}) -> Node
func DetachSensor(slot: String) -> void                 # 不存在则空操作
func SetBodySize(size: Vector3) -> void                 # 车身尺寸唯一入口（mesh + shape + 底面中心）

# ---- 查询
func GetChassisName() -> String                         # 未挂 = ""
func HasChassis() -> bool
func GetChassisParams() -> Dictionary                   # 生效参数副本（调试 / 自检）
func GetSensor(slot: String) -> Node3D                  # 不存在 = null；键名 = Sensors/<slot> 节点名
func GetStreamSlots() -> Dictionary                     # slot -> Node3D（副本，供 manager 建表 / 清理）
func GetPose() -> Dictionary                            # {x, y, z, yaw}，世界坐标（Z-up），yaw∈[-PI, PI)

# ---- 统一指令入口（canonical）
func SetMotionCommand(cmd: Dictionary) -> void           # 只存最新一条 + 转发给底盘 SetCommand()
func SampleSensors() -> Dictionary                       # slot -> 传感器自报样本

# ---- 统一接口别名（task_30 §八 Step 4 里写的名字，一行转发，便于按名调用）
func apply_motion(cmd: Dictionary) -> void               # == SetMotionCommand
func sample_sensors() -> Dictionary                      # == SampleSensors
func get_pose() -> Dictionary                            # == GetPose
```

**命名仲裁**：task_30 Step 4 要求"对外统一接口 `apply_motion / sample_sensors / get_pose`"，
而 §9.2 规定方法一律 `PascalCase`。本组件的处理是**同时满足、不二选一**：`PascalCase` 为实现与
Step 6 调用面，三个 snake_case 名字是**一行转发别名**（没有第三套语义、不存在两处逻辑）。

`_physics_process`（顺序固定，避免车体与底盘互相等待）：

```gdscript
if not _is_chassis_alive():                      # 空车 / 底盘被外部释放
    if not velocity.is_zero_approx():
        velocity = Vector3.ZERO
    return                                       # 不调用 move_and_slide()：空车不因重力/浮点噪声漂移
var ctl: Dictionary = _chassis.call("Step", velocity, delta)   # {velocity: Vector3(Godot), yaw_rate: float}
rotation.y += float(ctl.get("yaw_rate", 0.0)) * delta
velocity = ctl.get("velocity", Vector3.ZERO)
move_and_slide()                                 # 唯一的位置更新点（D8）
```

**`yaw_rate` 的符号**：它是 **Godot `rotation.y` 的角速度**，正 = 左转（`rotation.y` 增大）；
而 `GetPose().yaw` 是**世界 yaw** = `-rotation.y`（§6.5）。两者符号相反，勿混用。

**装配接口语义**：

| 项 | 约定 |
|---|---|
| 名字 → 场景 | ① 显式传 `chassis_scene`；② 否则走 `ChassisCatalog`（`ResourceLoader.exists()` + `load()`，Step 5 落地后自动接上，**不 preload**）；③ 都拿不到 → `push_warning` + `false`，**状态不变、不崩** |
| 参数合并 | 底盘 `GetDefaultParams()`（左）<- 传入 `params`（右，优先），结果既存 `_chassis_params` 也交给 `Configure(self, params)` |
| 换底盘 | 先 `remove_child()` 再 `queue_free()`（同帧重挂不会被改名成 `Xxx2`），`chassis_root` 恒 ≤ 1 个子节点 |
| 挂载失败 | 空名 / 场景缺失 / 根节点不是 `Node3D` / 缺 `Step()` → `false`（或 `null`），**不改任何状态** |
| `mount` 外参 | `{x, y, z, yaw}` 是**车体局部坐标**、轴向沿用**世界语义**（`x` 前 / `y` 右 / `z` 高 / `yaw` 绕 z），换算只经 `WorldTransform.PosToGodot()` / `YawToGodot()`。例：`{x:0.1, z:0.2}` = 车头前方 0.1 m、上方 0.2 m |
| `slot` | `strip_edges().validate_node_name()` 归一化，**键名 = 节点名**（`GetSensor` / `GetStreamSlots` / `DetachSensor` 与 `get_node("Sensors/<slot>")` 完全一致）；同 slot 重挂 = 先卸后挂，同帧完成 |
| 采样 | `Sample()` 优先、`ScanOnce()` 次之、都没有给 `null`（鸭子类型；Step 8 的 `Lidar2D.ScanOnce()` 天然可用，本接口无需为它改一行） |

**装配契约（Vehicle 视角）**：

| 角色 | 必需 | 可选 | 缺必需项时 |
|---|---|---|---|
| 底盘 | `Step(velocity: Vector3, delta: float) -> Dictionary{velocity, yaw_rate}` | `Configure(vehicle, params)` / `SetCommand(cmd)` / `GetDefaultParams()` | 直接拒挂（`false`） |
| 传感器 | 根节点是 `Node3D` | `Configure(params)` / `Sample()` / `ScanOnce()` | 直接拒挂（`null`）；缺采样方法不报错，`SampleSensors()` 给 `null` |

**数据流（本步打通的半条链，Step 6 补齐首尾）**：

```
[Step 6] manager ──ApplyChassis(name, params[, scene])──► Vehicle ──Configure(self, merged)──► Chassis
         manager ──AttachSensor(slot, scene, mount, params)──► Vehicle/Sensors/<slot>
         manager ──SetMotionCommand(cmd)──► Vehicle._cmd ──SetCommand──► Chassis
60 Hz： Vehicle._physics_process ──Step(velocity, delta)──► {velocity, yaw_rate}
                    └─ rotation.y += yaw_rate*dt ; velocity = … ; move_and_slide()   ← 世界权威 D8
读取：   GetPose() ─► {x,y,z,yaw}（世界，Z-up，yaw = -rotation.y）；SampleSensors() ─► {slot: 样本}
```

由 `WebSocketManager._RegisterChassis()` 实例化并 `add_child(World/Vehicles)`（子节点名 = 净化后的 `peer_id`）；
chassis 连接断开时由 manager `queue_free()`。根节点在 group `vehicle` 里，供相机选择目标（§4.2 `SelectNextTarget`）。

### 4.4 `src/scenes/chassis/DiffDrive.tscn`（Step 5 ✅）

```
DiffDrive (Node3D)                 script: res://src/chassis/DiffDrive.gd   (extends ChassisModel)
└── Wheels (Node3D)
    ├── WheelFL (MeshInstance3D)   CylinderMesh r=0.06 h=0.04（纯视觉，Configure 摆位）
    ├── WheelFR (MeshInstance3D)
    ├── WheelRL (MeshInstance3D)
    └── WheelRR (MeshInstance3D)
```

底盘基类 `src/chassis/ChassisModel.gd`：

```gdscript
extends Node3D

# 基类只定义契约，子类覆写；方法一律 PascalCase。
func Configure(vehicle: Node3D, params: Dictionary) -> void     # 基类只记宿主/参数；设 motion_mode / 尺寸是子类语义
func GetDefaultParams() -> Dictionary
func SetCommand(cmd: Dictionary) -> void                        # 接收 motion_cmd（由 Vehicle 转发）
func Step(current_velocity: Vector3, delta: float) -> Dictionary # {velocity: Vector3, yaw_rate: float}
```

`DiffDrive` 语义（与 §8.1 `motion_cmd` 澄清一致）：

| `action` | 线速度目标 | 偏航角速度目标 |
|---|---|---|
| `forward` | `+max_speed · pct` | 0 |
| `backward` | `-max_speed · pct` | 0 |
| `left` | 0 | `+max_turn_rate · pct` |
| `right` | 0 | `-max_turn_rate · pct` |
| `up` / `down` | 忽略（UGV 无竖直自由度） | 0 |
| `stop` / 未知 | 0 | 0 |

- `pct = clamp(speed, 0, 100) / 100.0`（`speed` 是百分比，对应 §六 的 `"speed": 50`）。
- **速度环在世界侧**（§6.3-1，与真机 STM32 内部跑速度环同构）：线速度与角速度按加速度上限斜坡逼近目标（`max_accel ≈ 3 m/s²`、`max_yaw_accel ≈ 180 °/s²`），再 `clamp` 到 `max_speed` / `max_turn_rate`。
- 车体速度：`velocity = global_basis * Vector3(v, 0, 0)`（**前进轴 = 局部 +X**，与 `rotation.y = -yaw` 自洽）。
- 默认参数：`{max_speed: 1.5, max_turn_rate: 90.0, wheel_base: 0.30, track_width: 0.24, height: 0.16}`；碰撞形状 `BoxShape3D(wheel_base, height, track_width)`（Godot **X=前**、Z=横；与 `src/scenes/Vehicle.tscn` 占位形状 `(0.30, 0.16, 0.24)` 一致）。
- `Configure()` 设 `motion_mode = CharacterBody3D.MOTION_MODE_GROUNDED`，`floor_snap_length` 用默认。

由 `Vehicle.ApplyChassis("diff_drive_ugv", params)` 实例化并挂在 `Vehicle/Chassis`；换底盘先 `queue_free()` 旧节点。

**落地要点（Step 5 实现，已由 `src/tests/check_diff_drive.gd` 41 项断言覆盖）**：

- `Configure(vehicle, params)` 的顺序固定：`super.Configure` 记宿主/参数 → 参数夹到合法域 → **速度环状态清零**（换底盘不继承旧速度）→ `ApplyMotionMode(MOTION_MODE_GROUNDED)` → `ApplyBodySize(Vector3(wheel_base, height, track_width))`（**只传尺寸**，`mesh/shape` 与"原点=底面中心"的抬升由 `Vehicle.SetBodySize()` 负责，底盘绝不自己写 `body.position.y`）→ `_layout_wheels()`（`x = ±wheel_base/2` 前 + 后 −、`z = ±track_width/2` 右 + 左 −、`y = 0.06` 让轮底贴 `y=0`；子节点缺失只 `push_warning` 一次）。
- `SetCommand()` 只改**目标值**（`_speed_target` / `_yaw_target`），实际速度由 `Step()` 的速度环斜坡逼近；`up` / `down` / 未知 `action` / 缺 `action` 一律 = `stop`。`left` / `right` 线速度目标为 0（原地转）是设计，不是漏写。
- `Step()` 的返回值 `velocity` 是 **Godot 全局速度**（`global_basis * Vector3(speed, 0, 0)`，`vel.y` 强制 0），`yaw_rate` 是 `rotation.y` 的角速度（正 = 左转）；树外调用（自检）走 `is_inside_tree()` 兜底，不访问 `global_*`。
- 轮子 `CylinderMesh` 的属性是 `top_radius` / `bottom_radius` / `height` / `radial_segments`（**没有** `radius`）；`rotation_degrees=(90,0,0)` 把圆柱轴从 Y 转到 Z（横轴）。轮子纯视觉、不参与物理：碰撞形状只有 `Vehicle/CollisionShape3D` 一个。
- 撞墙判据是**位置**：`move_and_slide()` 撞墙后自己把 `velocity` 归零（不是底盘逻辑），所以"会停"= 车头不穿透 + 位置停止推进 + 能倒车脱离。实测 world x 停在 `127.3375`（车头 `127.4875`，墙面 `127.5`）。
- 自检：`timeout 180 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_diff_drive.gd` → `total=41 failed=0` + `SELF-CHECK OK` + `EXIT=0`，stderr 为空。

### 4.5 `src/scenes/WebSocketManager.tscn`（Step 6 ✅）

```
WebSocketManager (Node)              script: res://src/net/WebSocketManager.gd
├── Heartbeat (Timer)                wait_time=1.0, autostart=true（握手/pending 超时巡检 + 可选统计日志）
└── （无其它子节点；每条连接是 RefCounted 的 ComponentLink，不进场景树）
```

```gdscript
extends Node

signal vehicle_spawned(peer_id: String, vehicle: Node3D)
signal assembly_completed(peer_id: String)
signal vehicle_removed(peer_id: String)
signal link_closed(peer_id: String, role: StringName, slot: String)

@export var vehicle_scene: PackedScene      # World.tscn 里指向 res://src/scenes/Vehicle.tscn
@export var vehicles_root: Node3D           # World.tscn 里指向 ../../Vehicles（相对自身！§9.1 E20）
@export var listen_port: int = 0            # 0 → 读 sim_world/network/ws_port（9999）
@export var autolisten: bool = true         # false = 不自动监听（自检用）
@export var handshake_timeout_ms: int = 5000   # 连上后多久没 hello 就关
@export var pending_timeout_ms: int = 10000    # 传感器先到、底盘一直不来 -> 关
@export var stats_log: bool = false         # true 时 Heartbeat 每秒打一行 GetStats()

# 两张表（§5.1）+ 装配簿记
var _peer_vehicles: Dictionary = {}         # 表 1：peer_id -> Vehicle
var _links: Array = []                      # 表 2 的可枚举侧：Array[ComponentLink]
var _pending_sensors: Dictionary = {}       # peer_id -> Array[{link, hello, since_ms}]
var _manifests: Dictionary = {}             # peer_id -> {slot: bool}
var _manifest_parts: Dictionary = {}        # peer_id -> {slot: sensor}
var _assembly_sent: Dictionary = {}         # peer_id -> bool（每 peer 只发一次）
var _next_avatar_id: int = 1                # u16 语义
var _next_sensor_id: int = 1                # u16 语义，>65535 回绕到 1
var _next_conn_id: int = 1                  # 连接号（日志 / 自检）
var _tcp_server: TCPServer
var _listen_port: int = 0                   # 实际监听端口（0 = 未监听）
var _pose_acc: float = 0.0                  # 位姿下行累加器（秒），与物理帧解耦（task_31 Step 1）

# 位姿下行周期（task_31 Step 1）：对齐真机 state_notifier 的 100 ms，硬编码、不做配置项
const POSE_BROADCAST_INTERVAL_S := 0.1

# 生命周期 / 监听
func _ready() -> void                        # _EnsureRefs() + 连 Heartbeat.timeout + autolisten 则 Listen()
func _exit_tree() -> void                    # Stop()
func _process(delta: float) -> void          # _AcceptConnections() → _PollLinks() → _SweepClosed() → _BroadcastPoses(delta)
func Listen(port: int) -> Error              # port<=0 = OS 临时端口；幂等（先 Stop()）
func Stop() -> void                          # 关全部连接 + 清 5 张表 + 移除全部 avatar + tcp.stop()
func GetListenPort() -> int
func IsListening() -> bool
func GetVehicle(peer_id: String) -> Node3D
func GetVehicleCount() -> int
func GetLinkInfos() -> Array                 # 表 2 只读快照 [{conn_id, peer_id, role, slot, sensor_id, open, handshaken}]
func GetStats() -> Dictionary                # {listening, port, links, vehicles, pending, assembled}
# 私有
func _AcceptConnections() -> void
func _PollLinks() -> void
func _HandlePacket(link, bytes: PackedByteArray, was_string: bool) -> void
func _ApplyMotionCommand(link, msg: Dictionary) -> void
func _BroadcastPoses(delta: float) -> void               # 位姿下行（task_31 Step 1；唯一的发送者）
func _RegisterChassis(link, msg: Dictionary) -> void
func _RegisterSensor(link, msg: Dictionary) -> void
func _MountSensor(link, hello: Dictionary) -> bool
func _ReplayPendingSensors(peer_id: String) -> void
func _TryAssembly(peer_id: String) -> void
func _RemoveVehicle(peer_id: String, except_link = null) -> void
func _CleanupLink(link) -> void
func _CloseLink(link, code: int, reason: String) -> void
func _SweepClosed() -> void
func _OnHeartbeatTimeout() -> void
func _PickSpawn(chassis_name: String) -> Dictionary      # {x, y, z, yaw}
func _ReadManifest(msg: Dictionary) -> Dictionary        # {slot: sensor}
func _ResolveCatalogScene(catalog_path: String, name: String) -> PackedScene
func _CatalogStatus(catalog_path: String, name: String) -> int   # KNOWN / UNKNOWN / MISSING
func _EnsureRefs() -> void                               # E16/E20 兜底
```

`ComponentLink`（`RefCounted`，`src/net/ComponentLink.gd`）：

```gdscript
extends RefCounted

var tcp: StreamPeerTCP
var peer: WebSocketPeer          # WebSocketPeer.new(); accept_stream(tcp)
var conn_id: int = 0
var peer_id: String = ""
var role: StringName = &""       # &"chassis" / &"sensor"
var slot: StringName = &""
var sensor_id: int = 0           # u16，sensor_ready 里回给 sim
var vehicle: Node3D = null
var out_seq: int = 0
var handshaken: bool = false
var closed: bool = false
var created_ms: int = 0          # 建链墙钟（握手超时）
var last_error: String = ""      # 关闭原因 / 对端关闭码（manager 一行日志）

func Init(p_tcp: StreamPeerTCP, p_conn_id: int) -> void   # set_no_delay(true)；set_heartbeat_interval(10.0)
func Poll() -> void                                      # peer.poll()；**先抽包**（OPEN/CLOSING）→ 再看 CLOSED
func TakePackets() -> Array                              # [{bytes, was_string}]，取出即清空
func SendJson(msg: Dictionary) -> Error                   # 未 OPEN -> ERR_UNCONFIGURED
func SendBinary(bytes: PackedByteArray) -> Error          # peer.send(bytes, WebSocketPeer.WRITE_MODE_BINARY)
func NextSeq() -> int                                    # 先自增再返回（首条 = 1）
func IsOpen() -> bool
func IsConnecting() -> bool
func Close(code: int = 1000, reason: String = "") -> void  # **void**（§9.1 E19）
func Describe() -> String
```

> Godot 4.7 的 `WebSocketPeer` **没有** `send_binary`：文本用 `send_text()`，二进制用 `send(bytes, WRITE_MODE_BINARY)`（§9.1 E15）。

静态实例，挂在 `World/Systems/WebSocketManager`；**全进程唯一**（"单例"语义 = 一个 Node 管全部连接，不用 autoload，避免与场景加载顺序耦合）。销毁由场景卸载负责。

#### 4.5.1 实现补充（Step 6 落地时确定，**不改变上文设计**）

**① 两张表怎么落地**：表 1 = `_peer_vehicles`；表 2 = **每个 `ComponentLink` 自己带的 `(peer_id, role, slot)`** + `_links` 保证可枚举（不另建 Dictionary，避免两份真相）。`_manifests` / `_manifest_parts` / `_assembly_sent` / `_pending_sensors` 只是**就绪簿记**，不构成第三套身份。

**② manifest 就绪语义（关键澄清）**：`manifest` 就绪 = **manifest 声明的槽位都在对应连接上就位**（task_30 §五"这辆车应该有几个零件、都连上了没"）。
"几何是否实例化成功"属世界侧能力（`src/scenes/sensors/Lidar2D.tscn` 到 Step 8 才有），**不得**阻塞 `assembly_ready`；两者在日志（`sensor_ready ... geometry=false`）与 `_manifest_parts` 里区分清楚。

**③ 传感器名的三种情况（必须分清）**：

| 情况 | 判据 | 行为 |
|---|---|---|
| 目录里**没有**这个名字 | `SensorCatalog.Has()` 为假（目录已落地） | 忽略该 slot：不回 `sensor_ready`、不置位、**连接保持**（§8.3-4） |
| 目录脚本**还没落地** | `res://src/sensors/SensorCatalog.gd` 不存在（Step 8 前） | 没有可校验的名字表 → **按已知登记**（回 `sensor_ready`，几何为 null，manifest 该 slot 可置位）；只 `push_warning` 一次 |
| 目录有、**场景文件**还没落地 | `ResourceLoader.exists(SCENES[name])` 为假 | 同上：只登记连接、不实例化几何，不影响就绪 |

> 第三行与第二行是**同一个** `_ResolveCatalogScene() == null` 分支；第一行在它之前就用 `_CatalogStatus()` 拦掉。
> Step 8 落地 `SensorCatalog` 后，第二行自动消失（目录存在），行为回到 §8.3-4 的严格校验。

**④ spawn 规则（确定性、无 RNG）**：第 1 台 = `World.SPAWN_WORLD`（`(64, 64, 0)`）；之后按 8 方位环错开，`ring = floor((idx-1)/8)+1`、`slot = (idx-1) mod 8`、`offset = (cos, sin, 0) * 2.0 * ring` 米；`quadrotor_uav` 的 `z = 2.5`（§7.1 巡航高度；task_32 Step 3 由 2.0 提高，`UAV_ALTITUDE_M`），UGV `z = 0`。`idx = _next_avatar_id - 1`，**必须在自增之前取**。

**⑤ 连接的角色是单值**：已握手的**传感器连接**发来 `chassis_hello` 属于协议错误 → `push_warning` + **忽略**（不改角色、不建车、不关别人的链路）。"重复 `chassis_hello` 重建 avatar"（§8.3-7）只在**该连接本身是底盘连接**时生效，且 `_RemoveVehicle(peer_id, except_link=本连接)` 绝不把新连接关掉。

**⑥ 监听与端口**：`TCPServer.listen(port)` 绑定全地址；失败（如 9999 被占）**只 `push_warning`**，世界继续跑（不可接入）。`listen_port = 0` 时读 `[sim_world] network/ws_port`；自检用 `Listen(0)` 取 OS 临时端口，**不抢 9999**。

**⑦ 日志指纹（供 grep 验收）**：`[ws] listening on port …` / `accept conn=…` / `chassis_hello peer=…` / `vehicle spawned peer=…` / `sensor pending …` / `sensor_ready peer=…` / `assembly_ready peer=…` / `link closed conn=…` / `vehicle removed peer=…` / `stopped (was port …)`。只在事件处打印，逐帧无输出。

**⑧ 自检证据**：`src/tests/check_ws_manager.gd`（51 项断言 / `total=51 failed=0` / `SELF-CHECK OK`）在同一进程里既当世界又当 sim（真 World.tscn + 真 `WebSocketPeer` 客户端），覆盖：装配链路、运动下行、传感器先到、空 manifest、断开清理、6 条错误路径、规范 lint。

**⑨ 位姿下行（task_31 Step 1，`pose` 文本帧）**：

- **周期**：`const POSE_BROADCAST_INTERVAL_S := 0.1`（= 10 Hz，对齐真机 `state_notifier` 的 100 ms，`base robot.rs:213`）；**硬编码、不做配置项**（真机也是硬编码）。
- **只发四种条件全满足的连接**：`role == ROLE_CHASSIS && handshaken && !closed && vehicle 有效（is_instance_valid）`。传感器连接**一条都不发**；读取 `link.vehicle` 之前先判 `role`/`handshaken`/`closed`，`call("GetPose")` 之前必须有 `is_instance_valid` 守卫（失效 avatar 上调用会打 `Invalid call` = `SCRIPT ERROR` 且进程不退出）。
- **取值来源（E23）**：位置/朝向 = `link.vehicle.call("GetPose")`（= `WorldTransform.PoseFromTransform(global_transform)`，已是世界坐标）；速度 = `WorldTransform.VelToWorld(vehicle.velocity)`（`vz` 丢弃，不进帧）。
- **累加器与解耦**：`_pose_acc += delta`，`if _pose_acc + 1e-6 < POSE_BROADCAST_INTERVAL_S: return`（`+1e-6` 容差与 `Lidar2D` 同款：60 fps 下 `1/60` 累加 6 帧才恰好到 0.1，无容差会漏帧）；落后超过 2 个周期只补发一帧并清零（不积压）。`--fixed-fps 60` 下一整个 1 s 窗口稳定 **10 条**。
- **`_process` 内顺序**：`_AcceptConnections → _PollLinks → _SweepClosed → _BroadcastPoses`（本帧已判死的连接先清出 `_links`，再下发）。
- **发送失败不处理**：`SendJson` 在非 `STATE_OPEN` 返回 `ERR_UNCONFIGURED` 属正常路径 —— 不检查返回值、不打日志、不断连（与 `ComponentLink` 既有语义一致）。
- **无新增日志**：逐帧零 `print` / `push_warning` / `push_error`；`stats_log` 保持 `false`。
- **自检证据**：`src/tests/check_pose.gd`（23 项断言 / `total=23 failed=0` / `SELF-CHECK OK`），同一进程真 `src/scenes/World.tscn` + 真 `WebSocketPeer` 客户端 + `Listen(0)` 临时端口，覆盖 task_31 §三 6.3 的 P1~P9（A1~A7 树外协议形态 / B1~B2 接线 / C1~C14 端到端与白盒）。

### 4.6 `src/scenes/chassis/Quadrotor.tscn`（Step 7 ✅）

```
Quadrotor (Node3D)                 script: res://src/chassis/Quadrotor.gd   (extends ChassisModel)
└── Rotors (Node3D)
    ├── RotorFL (MeshInstance3D)   CylinderMesh r=0.12 h=0.02（纯视觉，可自转）
    ├── RotorFR (MeshInstance3D)
    ├── RotorRL (MeshInstance3D)
    └── RotorRR (MeshInstance3D)
```

- `Configure()`：`motion_mode = MOTION_MODE_FLOATING`；碰撞/车身 `BoxShape3D(0.4, 0.12, 0.4)`；**不施加重力**（D3 自写运动模型，悬停由"无竖向指令即 `v_y=0`"实现）。
- `Step()`：水平速度按 `forward/backward/left/right` + `speed%`；`up`/`down` 决定 `±max_climb_rate`，无竖向指令时 `v_y = 0`（= 高度保持）；`yaw_rate` 规则同 UGV（`left`/`right`）。
- 默认参数：`{max_speed: 2.0, max_climb_rate: 1.0, max_yaw_rate: 90.0, size: 0.40}`。
- 由 `Vehicle.ApplyChassis("quadrotor_uav", params)` 实例化并挂在 `Vehicle/Chassis`；出生高度 `z=2.5 m`（§7.1 巡航高度；task_32 Step 3）。
- `ClampAltitude()`（task_32 Step 3）：每物理帧把**宿主车身**世界 z 夹到 `[0.0, World.AIR_WALL_HEIGHT_M - CLAMP_MARGIN_M] = [0.0, 31.8]`，越界时 `velocity.y = 0`；详见下文「高度软件限幅」bullet。

**落地要点（Step 7 实现，`src/chassis/Quadrotor.gd`）**

- **action 语义（唯一自洽解释，本 step 裁决）**：`motion_cmd` 是**单 action**（§8.1-2），一个动作不组合，故

| action | 水平 `v_x` | 竖直 `v_y`（= 爬升率） | `yaw_rate` |
|---|---|---|---|
| `forward` | `+max_speed·pct` | 0 | 0 |
| `backward` | `-max_speed·pct` | 0 | 0 |
| `left` | 0 | 0 | `+max_yaw_rate·pct`（原地左转，同 UGV） |
| `right` | 0 | 0 | `-max_yaw_rate·pct` |
| `up` | 0 | `+max_climb_rate·pct` | 0 |
| `down` | 0 | `-max_climb_rate·pct` | 0 |
| `stop` / 未知 / 缺 `action` | 0 | 0 | 0（三轴目标 0 = 悬停） |

- **`left`/`right` = 原地偏航（不是侧移）**：UAV 参数表（§7.2 / §7.6）里只有 `max_speed` / `max_climb_rate` / `max_yaw_rate` / `size`，**没有侧移速度参数**；若把 `left`/`right` 当侧移，`max_yaw_rate` 将永远无人使用。加上 §4.6 自身写明"`yaw_rate` 规则同 UGV（`left`/`right`）"、真实 UAV 决策链与 UGV 同构、`motion_cmd` 单 action 四点，落地取"偏航"。**该裁决不改设计**（参数表、消息 schema、§3 零件目录均未动）。
- **单 action 的直接后果**：从 `forward` 切到 `up`，水平目标被清零，速度按 `MAX_ACCEL_MPS2 = 3.0 m/s²` 斜坡降到 0（实测漂移 **0.650 m**，解析 `(2.0+0)/2·40/60 = 0.667`）。
- **速度环**：水平与竖直共用基类常量 `MAX_ACCEL_MPS2 = 3.0 m/s²`（不新增参数），偏航用 `MAX_YAW_ACCEL_DPS2 = 180 °/s²`；到顶时间 = **水平 40 帧 / 竖直 20 帧 / 偏航 30 帧**。
- **无重力悬停的实现与证据**：脚本**从不**施加任何重力（无 `get_gravity` / `apply_gravity` / 手写 `y -= g·dt`），`CharacterBody3D` 也不自动加重力 → 无竖向指令时 `v_y = 0`。实测：`y=2.5` 悬停 300 帧 `dy = 0.00000000`；`y=10.0` 同样 `dy = 0.00000000`（与 `default_gravity = 9.8` 无关，排除"贴着地面所以不掉"）。
- **60 帧实测位移（`--fixed-fps 60`）**：水平 **1.3500 m**（15 帧 0.1000 m，120 帧 3.3500 m）、竖直 **0.8417 m**（`up` / `down` 对称）、偏航 **68.25° = 1.1912 rad**；从 2.5 m 全速下降 240 帧落在地面 `y ≈ 0.0001`（下降 2.5 m 约 160 帧 < 240，结论不变），再 60 帧 `dz = 0`（不打穿、不抖）。
- **高度与边界的已知事实（task_32 Step 3 已变更，覆盖 task_30 的 2.0 m 记录）**：巡航高度 **2.5 m**（`WebSocketManager.UAV_ALTITUDE_M`）**高于 2 m 可见矮墙**，UAV 朝墙飞会从矮墙上方越过（task_30 记录的「巡航 2.0 m = 墙高 2.0 m → 朝墙飞撞停、实测 `x = 127.2999`」是 **Step 3 之前**的历史结论）；真正的硬边界是 32 m 空气墙 + 32 m 顶盖（§4.1 / §10）。世界侧空域策略仍另议（决策链不控高度）。
- **高度软件限幅（task_32 Step 3，§9.1 E25）**：`Quadrotor._physics_process()` 每帧调 `ClampAltitude()`（父节点 `Vehicle` 的 `Step` + `move_and_slide()` 先跑 → 天然夹在推进之后）。夹的是**宿主车身**（`CharacterBody3D` 在 `Vehicle` 根节点上；本节点 local 恒 0，夹自己没有物理意义）；读写走 `global_transform.origin.y`（D8 白盒 lint 的 `BANNED_TOKENS` 禁 `global_position`，见 E25），只改 `origin.y`、不动 `basis`。上限不写死字面量：`ceiling = World.AIR_WALL_HEIGHT_M - CLAMP_MARGIN_M = 31.8`。越界时 `velocity.y = 0`、`_climb = 0`、`push_warning` **1 s 限流**（`CLAMP_WARN_INTERVAL_MS = 1000`，绝不 `push_error`）。**不改运动学**：参数表仍 4 键、位置推进仍只由 `Vehicle.move_and_slide()` 完成；限幅只是物理失效时的兜底。
- **自检**：`src/tests/check_quadrotor.gd`（48 项断言 / `total=48 failed=0` / `SELF-CHECK OK`），命令见 §1.4 同款模板：
  `timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_quadrotor.gd`。
  覆盖：场景形态 / 旋翼外观与摆位 / 目录解析（Step 6 时这里返回 null）/ 装配与 `FLOATING` 真改（先置 `GROUNDED` 再断言，E18 反向坑）/ 车身与旋翼重摆位 / 无重力悬停（2.5 m 与 10 m）/ 平移 / 升降 / 偏航 / 落地 / 指令容错 / 高度软件限幅（3 项：`clamp-ceiling-pins` / `clamp-ceiling-holds-under-climb` / `clamp-ceiling-derivation`）/ 白盒越界 lint。

### 4.7 `src/scenes/sensors/Lidar2D.tscn`（Step 8）

```
Lidar2D (Node3D)                   script: res://src/sensors/Lidar2D.gd
└── DebugPoints (MeshInstance3D)   ImmediateMesh，扫描点可视化；visible=false（默认关）
```

```gdscript
extends Node3D

signal scan_produced(ranges: PackedFloat32Array, world_time: float, seq: int)

var range_limit: float = 12.0      # 与真机一致（§7.4）
var ray_count: int = 360
var rate_hz: float = 10.0
var _accum: float = 0.0
var _seq: int = 0
var _link = null                   # ComponentLink（RefCounted）
var _sensor_id: int = 0
var _rng := RandomNumberGenerator.new()   # 可选噪声，v1 默认关闭

func Configure(params: Dictionary) -> void        # range / rays / rate_hz
func SetStream(link, sensor_id: int) -> void      # 必须在 sensor_ready 之后调用
func ScanOnce() -> PackedFloat32Array             # 供自检 / 调试直接调用（不依赖时钟）
func _physics_process(delta: float) -> void       # 累加器 → ScanOnce → send binary
```

扫描实现（§7.4，真射线，不查表）：

- 在**水平面**循环 `ray_count` 条射线，角度 `a = TAU * i / ray_count`（起点为挂载点局部 +X），终点 `to_global(Vector3(cos(a), 0, sin(a)) * range_limit)`。
- `PhysicsRayQueryParameters3D.create(from, to, mask, exclude)`：`mask = 1|2|4`（地面 / 体素 / 车辆；其它车可见）；`exclude = [get_parent_vehicle().get_rid()]`（排除自身，避免起点落在自身碰撞体内）。
- 命中 → `from.distance_to(hit.position)`；未命中 → **`0.0`**（wire 约定：`0.0` = 无回波，见 §8.2）。
- 每圈：`seq = NextSeq()`、`world_time = WorldClock.Now()`；`WireProtocol.BuildLidar2DFrame(sensor_id, seq, world_time, ranges)` → `link.SendBinary()`；同时 `emit_signal("scan_produced", …)` 便于自检。

由 `Vehicle.AttachSensor(slot, scene, mount, params)` 实例化并挂在 `Vehicle/Sensors/<slot>`；`mount` 为局部外参 `{x,y,z,yaw}`（先在 `Vehicle` 局部系里设 `position` / `rotation.y`）。`DetachSensor(slot)` 时 `queue_free()`。

#### 4.7.1 实现补充（Step 8 落地时确定，**不改变上文设计**）

**① 角度约定（S2）**：`index i -> 局部方向 (cos a, 0, sin a)`，`a = TAU · i / rays`，局部 `+X` = 车头；
`LaserPoint.angle = a`（弧度，从 `+X` 起、**世界 yaw 增大方向**）→ sim 侧 `ob_angle = yaw + p.angle` 可直接代入。
角度全在**雷达局部系**，换轴由车辆的 `rotation.y = -yaw` 负责，所以 `Lidar2D.gd` 不引 `WorldTransform`。

**② 射线与掩码（S3）**：`PhysicsRayQueryParameters3D.create(origin, origin + dir·range, mask, exclude)`，
`mask = 1|2|4`（`world_ground` / `world_obstacle` / `vehicle`：其它 avatar 可见），
`exclude = [宿主 CollisionObject3D.get_rid()]`（`_SelfRidArray()`：向上找第一个碰撞体 = `Vehicle`；自车不可见）。
`mask` 的 `7` 由 `[layer_names]` 的语义决定；**未命中返回空字典 `{}`（不是 `null`），记 `0.0`**（§8.2 无回波约定，S4）。

**③ `LaserScan`（task_30 §7.4，S8）**：`GetLaserScan()` 返回
`{points: [{angle, range, intensity}], stamp, scan_freq, scan_time}`；`intensity` 命中 = `intensity_hit`（255.0）、未命中 = 0；
`stamp = WorldClock.NowUsec() * 1000`（**纳秒**，基 = 世界时钟，**不是 Unix 时间**，只用于相对对齐）；
`scan_freq = rate_hz`、`scan_time = 1 / rate_hz`。wire 上仍只发 `f32[]`（**角度不上线**，§8.2），两者并存不冲突。

**④ 接口（S7：不提供 `Sample()`）**：装配 `Configure(params)` / `GetParams()`；数据流 `SetStream(link, sensor_id)` /
`ClearStream()` / `HasStream()`（`link` 写 `Variant` + 鸭子调用，**不 preload `ComponentLink`**，R6）；
采样 `ScanOnce()`（不发包 / 不递增 seq / 不发 signal，`Vehicle.SampleSensors()` 走它）/ `GetLastRanges()` /
`GetLaserScan()` / `GetScanCount()` / `SetDebugVisible()`。`scan_produced(ranges, world_time, seq)` 每圈都发（有无 link 都发）。

**⑤ 累加器（S6）**：`ACCUM_EPS = 1e-6`。60 fps 下 `6 × 1/60 = 0.09999999999999999 < 0.1`，朴素写法 1 s 只扫 **9** 次；
加容差后帧 `6/12/…/60` 恰好 **10** 次（实测 `E1-rate-10hz scans=10`）。卡顿后最多补一圈
（`_accum = minf(_accum - period, period)`），不"追赶爆扫"。

**⑥ 参数与边界**：`range / rays / rate_hz / noise_sigma / intensity_hit / scan_mask`；键缺失 = 保留当前值，
值非法 = 回退**默认常量**并 `push_warning` **一次**（`rays = 0/-1/"x" → 360`、`range = 0/-1 → 12`、`rate_hz = 0 → 10`）。
树外 `ScanOnce()` 返回全 0（树外访问 `global_*` 会 ERROR）；`< 0.01 m` 的命中会被读成无回波（R14）。

**⑦ 性能（实测）**：360 线 × 100 圈 = 107.6 ms → **1.076 ms/圈**（≈3 µs/条，headless）；10 Hz/车 ≈ 1% 单核，
故不上多线程 / 不批量 `intersect_shape`。

**⑧ 水平面扫描的盲区（R4，已知事实不是缺陷）**：射线固定在挂载点高度（默认 `mount.z = 0.2 m`），
而 UGV 车体高 `0.16 m` → 0.2 m 以上的雷达**看不见**别的 UGV；要看车体需把 `mount.z` 设到 `0.16` 以下
（自检 `D8-other-vehicle-visible` 用 `z=0.1` 证明"其它 avatar 可见"）。

**⑨ 自检**：`src/tests/check_lidar.gd`（**48 项断言** / `total=48 failed=0` / `SELF-CHECK OK`），命令同 §1.4 模板：
`timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_lidar.gd`。
证据链（"一圈能正确反映周围障碍的距离"）：`D1-open-space` 空旷 `max=0.0000`（360 × 0.0）、
`D2-forward-distance` `3.4000 m`、`D3-angle-span` 命中 `rel=[-8..8]`（17 条）、`D4-side-distance` `3.0000 m`、
`D5-no-echo-behind` 背向 `0.0`、`D6-follows-position` 前进 1 m → `2.4000 m`、`D7-mount-yaw-rotates` `3.0000 m`、
`D8-other-vehicle-visible` `0.7500 m`、`D11-range-clipping` 超量程 → `0.0`、`E1-rate-10hz` 60 帧 10 圈、
`E4-frame-format` 1455 B / `type=1` / `slot_id=7` / `seq` 1…10、`F7-real-wall-distance` 真实 `wall_c` `2.0000 m`、
`F8-real-wall-hits` 一圈 `152` 条（plan P2 实测 152）。

---

## 5 挂载关系

### 5.1 静态挂载（场景内 `instance`，随场景加载）

```
World.tscn
  ├─ instance res://src/scenes/ObserverCamera.tscn              → World/ObserverCamera            [Step 3]
  └─ Systems/ instance res://src/scenes/WebSocketManager.tscn   → World/Systems/WebSocketManager  [Step 6] ✅
                                                                @export vehicle_scene → res://src/scenes/Vehicle.tscn
                                                                @export vehicles_root → ../../Vehicles
                                                                （node_paths=PackedStringArray("vehicles_root")；
                                                                  相对该节点自身：.. = Systems，../.. = World）
```

静态挂载的特点：随场景树生命周期存在；`@export` 在 `src/scenes/World.tscn` 里以 `NodePath` / `ExtResource` 绑定，**位置在 `src/scenes/World.tscn` 中显式指定**，不在脚本里硬编码路径。

### 5.2 动态挂载（运行时 `add_child`）

```
WebSocketManager._RegisterChassis()
  └─ Vehicle.tscn instance               → World/Vehicles/<peer_id 净化名>
       ├─ Vehicle.ApplyChassis()         → Vehicle/Chassis/<DiffDrive|Quadrotor>
       └─ Vehicle.AttachSensor()         → Vehicle/Sensors/<slot>/<Lidar2D|…>
```

四个挂载点（Step 2 建树时一次性留好）：

| 挂载点 | 谁往里挂 | 约束 |
|---|---|---|
| `World/Vehicles` | `WebSocketManager` | 子节点名 = `peer_id` 经 `String.validate_node_name()` 净化；一个 peer 一个 avatar |
| `Vehicle/Chassis` | `Vehicle.ApplyChassis()` | **同时只有一个**底盘子节点；换底盘 = 先 `remove_child()` 再 `queue_free()` 旧的，然后挂新的（顺序写死，防同名重挂被改名） |
| `Vehicle/Sensors` | `Vehicle.AttachSensor()` | 子节点名 = `slot`（如 `lidar_front`），便于按 slot 卸载与自检 |
| `World/Systems` | 静态（编辑器内实例化） | 非物理系统容器；v1 只有 `WebSocketManager` |

### 5.3 挂载 / 卸载时序

```
装配（chassis 线）:
  WS open → 首包 chassis_hello
    → Vehicle 实例 + name + add_child(World/Vehicles)
    → global_position = WorldTransform.PosToGodot(spawn) ; rotation.y = -yaw
    → ApplyChassis(chassis, params)      # 挂 Vehicle/Chassis/<底盘>
    → 回 chassis_ready{peer_id, avatar_id, spawn, world_time}
    → 回放 _pending_sensors[peer_id]

装配（sensor 线）:
  WS open → 首包 sensor_hello
    → 该 peer 无 vehicle ? 进 _pending_sensors，等底盘
                          : AttachSensor(slot, scene, mount, params)
                            → sensor_id = _next_sensor_id++      # u16
                            → 回 sensor_ready{slot, sensor_id}
                            → sensor.SetStream(link, sensor_id)  # 先 ready 再发数据
    → _TryAssembly(peer_id)：manifest 全就绪 → assembly_ready（每 peer 仅一次）

卸载（Step 6 实际顺序）:
  chassis link 断 → _RemoveVehicle：先 _peer_vehicles.erase（再 free，防同帧二次访问）
                    → remove_child + queue_free(vehicle)
                    → 关闭该 peer 其余全部 link（1001 "peer removed"）
                    → 清 _manifests / _manifest_parts / _assembly_sent / _pending_sensors
  sensor  link 断 → vehicle.DetachSensor(slot) + manifest[slot] = false + 从 pending 队列摘除
  同一 peer_id 重复 chassis_hello（**只在该连接是底盘连接时**）→ 先 _RemoveVehicle(peer, except_link=本连接)
                    再新建；except_link 保证刚接手的连接不被误关
  未握手连接超时 → 关（1002）；pending 等待底盘超时 → 关（1008）
  assembly_ready 发出后，部件断开再连**不重发**（_assembly_sent 每 peer 只置一次）
```

**时序铁律**：`sensor_ready` 一定先于该传感器的第一条二进制帧（否则 sim 侧会因未知 `slot_id` 丢包）。
> 实现说明：`Close()` 只置状态，真正的移除留到**下一帧** `_SweepClosed()`（遍历中不删数组）；`WebSocketManager._process(delta)` 每帧顺序固定为 accept → poll → sweep → **broadcast**（`_BroadcastPoses(delta)`，task_31 Step 1；先清掉本帧已判死的连接再下发位姿）。

---

## 6 数据流

### 6.1 装配流

```
sim（chassis 连接）                                 Godot 世界
  chassis_hello  ──────────────────────────────►  _RegisterChassis
                                                     ├─ Vehicle 实例 + ApplyChassis
                                                     └─ 回放 pending 传感器
                 ◄──────────────────────────────  chassis_ready{avatar_id, spawn{x,y,z,yaw}, world_time}
  sensor_hello × K ─────────────────────────────►  _RegisterSensor
                                                     └─ Vehicle.AttachSensor(slot)
                 ◄──────────────────────────────  sensor_ready{slot, sensor_id}
  (manifest 全部就绪) ◄─────────────────────────  assembly_ready{peer_id, world_time}
  sim 收到 assembly_ready 后才启动决策链
```

### 6.2 控制流

```
motion_cmd（20~50 Hz，JSON text）
  ──► ComponentLink.Poll() ──► WebSocketManager._HandlePacket
        ──► Vehicle.SetMotionCommand(cmd)          # 只保留最新一条
              ──► Chassis.SetCommand(cmd)
Vehicle._physics_process(60 Hz):
  ctl = Chassis.Step(velocity, delta)               # 世界侧速度环 + 运动学
  rotation.y += ctl.yaw_rate * delta
  velocity = ctl.velocity
  move_and_slide()                                  # 唯一的位置更新点（世界权威 D8）
```

### 6.3 传感流

```
Lidar2D._physics_process(delta)                     # 累加器：>= 1/rate_hz 才扫
  ──► PhysicsDirectSpaceState3D.intersect_ray × ray_count   # mask=7，排除自身 RID
  ──► PackedFloat32Array 距离（0.0 = 无回波）
  ──► WireProtocol.BuildLidar2DFrame(sensor_id, seq, world_time, ranges)
  ──► ComponentLink.SendBinary(bytes)  ──► sim（tokio-tungstenite，按 §8.2 解析）
  ──► emit scan_produced(...)                       # 供自检 / 可视化
```

### 6.4 时间基与相机

```
[时间]  World._ready() ─► WorldClock.EnsureStarted()
        所有下发帧的 world_time 均取 WorldClock.Now()（= Time.get_ticks_usec() 差值的秒数）
        60 Hz 物理 ≠ 10 Hz LiDAR ≠ 50 ms 决策 tick；对齐靠 world_time 时间戳，不靠帧数比例

[相机]  ObserverCamera 只读场景树（group "vehicle" 选目标）
        → 无数据流进 sim、无数据流进物理世界；移动相机不改变任何位姿 / 传感读数
        _unhandled_input 只改相机自己的状态量（yaw/pitch/radius），
        _physics_process 只写自己的 global_position / rotation；
        两者都不写任何其它节点的属性，也不调用任何物理查询（固定 60 Hz 物理帧更新，不做渲染插值）
```

### 6.5 坐标、单位与朝向约定（汇总）

| 项 | 约定 |
|---|---|
| 世界坐标系 | Z-up：`x`=东、`y`=南、`z`=高；yaw 绕 `z`，从 +x 起逆时针为正（右手系） |
| Godot 坐标系 | Y-up；`position = (wx, wz, wy)`；`rotation.y = -yaw` |
| 车辆前进轴 | **局部 +X**（Godot）；世界侧 +x 方向即 yaw=0。`Chassis.Step().yaw_rate` 是 **`rotation.y` 的角速度**（正 = 左转），而 `GetPose().yaw` 是**世界 yaw** = `-rotation.y`——两者符号相反，勿混用 |
| 距离 / 速度 | 米、米/秒；内部角度一律弧度，`*_rate` 入参可为度/秒，转换只在 `ChassisModel` 一处 |
| 原点 | `Vehicle` 原点 = 车体底面中心；世界 `z=0` ↔ Godot `y=0` |
| 时间 | `world_time` = 单调墙钟秒（`Time.get_ticks_usec()`），跨连接唯一对齐源 |
| 无回波 | LiDAR 距离 `0.0`（不是 `range`、不是 `inf`） |

---

## 7 接口签名汇总

### 7.1 `WorldClock`（`src/common/WorldClock.gd`，静态工具）

```gdscript
extends RefCounted
static var _start_usec: int = 0

static func EnsureStarted() -> void      # 首次调用记录起点；重复调用无副作用
static func Now() -> float               # (now_usec - _start_usec) / 1e6
static func NowUsec() -> int
static func Reset() -> void              # 仅测试用
```

### 7.2 `WorldTransform`（`src/common/WorldTransform.gd`，静态工具）

```gdscript
extends RefCounted

static func PosToGodot(p: Vector3) -> Vector3        # (wx, wy, wz) -> Vector3(wx, wz, wy)
static func PosToWorld(g: Vector3) -> Vector3        # Vector3(gx, gz, gy)
static func VelToGodot(v: Vector3) -> Vector3        # 同 PosToGodot
static func VelToWorld(v: Vector3) -> Vector3        # 同 PosToWorld
static func YawToGodot(world_yaw: float) -> float    # -world_yaw
static func YawToWorld(godot_yaw: float) -> float    # -godot_yaw
static func MakeTransform(world_pos: Vector3, world_yaw: float) -> Transform3D
static func PoseFromTransform(t: Transform3D) -> Dictionary   # {x, y, z, yaw}，yaw 归一化到 [-PI, PI)
```

> **禁止**其它文件出现 `Vector3(p.x, p.z, p.y)` 这类字面量转换（唯一入口原则）。

### 7.3 `WireProtocol`（`src/net/WireProtocol.gd`，静态工具）

```gdscript
extends RefCounted

const MSG_CHASSIS_HELLO := "chassis_hello"
const MSG_CHASSIS_READY := "chassis_ready"
const MSG_SENSOR_HELLO := "sensor_hello"
const MSG_SENSOR_READY := "sensor_ready"
const MSG_MOTION_CMD := "motion_cmd"
const MSG_ASSEMBLY_READY := "assembly_ready"
const MSG_POSE := "pose"          # 位姿下行（task_31 Step 1）；imu / gnss 在 v1 不做（§10）

const FRAME_TYPE_LIDAR := 1
const FRAME_TYPE_CAMERA := 2      # 预留
const FRAME_HEADER_SIZE := 15
const LIDAR_SAMPLE_BYTES := 4     # 每个距离 f32

static func ParseJson(bytes: PackedByteArray) -> Dictionary           # 失败返回 {}，不抛、**不打印**
static func MakeJson(msg: Dictionary) -> PackedByteArray              # JSON.stringify(...).to_utf8_buffer()
static func MakeJsonText(msg: Dictionary) -> String                   # send_text() 用
static func BuildPose(world_time: float, pose: Dictionary, velocity: Vector3) -> Dictionary
static func BuildLidar2DFrame(sensor_id: int, seq: int, world_time: float,
        ranges: PackedFloat32Array) -> PackedByteArray
static func ParseFrameHeader(bytes: PackedByteArray) -> Dictionary     # 短包(<15 B) -> {}；否则 {type, slot_id, seq, world_time}
static func FramePayloadSize(bytes: PackedByteArray) -> int            # max(size - 15, 0)
```

实现约定（Step 6 实测）：

- `ParseJson` 用 `JSON.new().parse()`（返回 `Error`）而不是 `JSON.parse_string()`：后者遇非法输入会向 **stderr 打印 `ERROR: Parse JSON failed`**，污染「grep ERROR 必须为空」的验收（§9.1 **E21**）。解析失败 / 非 `Dictionary`（数组、标量、null）一律 `{}`。
- `BuildLidar2DFrame`：先 `resize(15)` 编头（`encode_u8(0,…)` / `encode_u16(1, sensor_id & 0xFFFF)` / `encode_u32(3, seq & 0xFFFFFFFF)` / `encode_double(7, world_time)`），再 `append_array(ranges.to_byte_array())`；长度 = `15 + 4·rays`（360 线 = **1455 B**）。
- `ParseFrameHeader` 的短包语义 = `{}`（调用方据此丢弃整帧，不部分解析）。
- `BuildPose`（task_31 Step 1）：固定 **8 键** `{msg, world_time, x, y, z, yaw, vx, vy}`，`z` **必带**（UGV 0 / UAV 2.5）；`velocity.z` 丢弃（`vz` 不上线），不发 `valid` / `sub_gx` / `sub_gy` / `rtk_fixed`；`velocity` 由**调用方**传 `WorldTransform.VelToWorld(vehicle.velocity)` 的结果（本文件**不** preload `WorldTransform`，保持"只做协议事实"的分层）；`pose` 缺 key 时四字段回 `0.0`（不崩）。
- **只做协议事实**：不碰 socket、不碰场景树。

### 7.4 `ComponentLink`（`src/net/ComponentLink.gd`，`RefCounted`）

见 §4.5（`Init` / `Poll` / `TakePackets` / `SendJson` / `SendBinary` / `NextSeq` / `IsOpen` / `IsConnecting` / `Close` / `Describe`）。

补充（Step 6 实测）：

- `Close(code, reason)` **返回 `void`**（§9.1 **E19**）：写成 `var e := peer.close(...)` 会直接 Parse Error。
- `Poll()` 顺序写死：`peer.poll()` → **先抽包**（`STATE_OPEN`/`STATE_CLOSING` 都抽）→ 再看 `STATE_CLOSED` 置 `closed`；先关后抽会丢最后几帧。
- `last_error` 记关闭原因 / 对端关闭码（`get_close_code()` / `get_close_reason()`），供 manager 一行日志。
- `SendJson` / `SendBinary` 未 OPEN 时返回 `ERR_UNCONFIGURED`（不崩、不打印）。

### 7.5 `ChassisModel` 及实现（`src/chassis/`）

见 §4.4（`Configure` / `GetDefaultParams` / `SetCommand` / `Step`）。

基类 = **契约 + 公共工具**（不含任何运动学：`Step()` 返回零控制量 = 不动的底盘），
速度环 / 指令解析的公共部分只在本文件实现一次，Step 7 的 `Quadrotor` 复用：

```gdscript
# src/chassis/ChassisModel.gd —— extends Node3D；不声明 class_name
const MIN_SIZE := 0.01                 # 尺寸下限（防零尺寸 shape / 除零）
const MAX_ACCEL_MPS2 := 3.0            # 线加速度上限（世界侧速度环）
const MAX_YAW_ACCEL_DPS2 := 180.0      # 偏航角加速度上限（度/秒²）

var vehicle: Node3D                    # Configure 记下的宿主（Vehicle 本体）
var params: Dictionary                 # 生效参数（= 默认值 <- 传入值）
var last_cmd: Dictionary               # 最新一条 motion_cmd（只保留最新，不排队）

func GetParam(key: String, fallback: float) -> float   # 缺失 / 字符串 / 非法类型都回退
func SpeedPct(cmd: Dictionary) -> float                # clamp(speed,0,100)/100；非法值 -> 0.0
func ActionOf(cmd: Dictionary) -> String               # strip_edges().to_lower()；缺省 "stop"
func RateToRad(dps: float) -> float                    # 度/秒 -> rad/s
func Approach(current: float, target: float, max_delta: float) -> float   # 一阶斜坡
func ApplyMotionMode(mode: int) -> bool                # is CharacterBody3D 守卫后写 motion_mode
func ApplyBodySize(size: Vector3) -> bool              # has_method("SetBodySize") -> vehicle.call(...)
```

`DiffDrive` 在此之上只用 `GetDefaultParams` / `Configure` / `SetCommand` / `Step` + 上表工具：
`Step()` = `Approach` 斜坡（`3 m/s²` / `180 °/s²`）+ `clamp` 到 `max_speed` / `max_turn_rate`
+ `global_basis * Vector3(v, 0, 0)`（`vel.y = 0`）。`GetDefaultParams()` 返回常量字典
（可能在 `add_child` **之前**被 `Vehicle._merge_params()` 调用，不得访问场景树）。

### 7.6 `ChassisCatalog` / `SensorCatalog`（静态目录，Step 5/6/8）

`ChassisCatalog`：Step 5 ✅ 已落地（`src/chassis/ChassisCatalog.gd`，`extends RefCounted`，全静态）；
`SensorCatalog`：Step 8 ✅ 已落地（`src/sensors/SensorCatalog.gd`，签名同构）。

```gdscript
# src/chassis/ChassisCatalog.gd —— 不声明 class_name；Step 5 ✅
const SCENES := {
    "diff_drive_ugv": "res://src/scenes/chassis/DiffDrive.tscn",
    "quadrotor_uav":  "res://src/scenes/chassis/Quadrotor.tscn",
}
const DEFAULTS := {
    "diff_drive_ugv": {"max_speed": 1.5, "max_turn_rate": 90.0,
                       "wheel_base": 0.30, "track_width": 0.24, "height": 0.16},
    "quadrotor_uav":  {"max_speed": 2.0, "max_climb_rate": 1.0,
                       "max_yaw_rate": 90.0, "size": 0.40},
}
static func Has(p_name: String) -> bool                # 目录外名字 = false（不 load、不报错）
static func GetScene(p_name: String) -> PackedScene    # load() + 缓存；不存在返回 null 并 push_error
static func GetDefaults(p_name: String) -> Dictionary  # 副本（duplicate(true)）；未登记 -> {}
```

`SensorCatalog` 同构：

```gdscript
const SCENES := {"lidar_2d": "res://src/scenes/sensors/Lidar2D.tscn"}
const DEFAULTS := {"lidar_2d": {"range": 12.0, "rays": 360, "rate_hz": 10.0}}
```

> **必须用 `load()` 而非 `preload()`**：Step 6（manager）先于 Step 7/8（场景文件）完成，`preload` 会因资源不存在导致整脚本解析失败。

### 7.7 `Vehicle` / `WebSocketManager` / `Lidar2D`

见 §4.3 / §4.5 / §4.7。

---

## 8 消息契约

### 8.1 文本帧（JSON，task_30 §六 +「实现约定」澄清）

```jsonc
// sim → world（底盘连接首包）
{ "msg": "chassis_hello", "peer_id": "12D3Koo...", "chassis": "diff_drive_ugv",
  "params": { "max_speed": 1.5, "max_turn_rate": 90 },
  "manifest": [ { "slot": "lidar_front", "sensor": "lidar_2d" } ] }

// world → sim
{ "msg": "chassis_ready", "peer_id": "12D3Koo...", "avatar_id": 7,
  "spawn": { "x": 64.0, "y": 64.0, "z": 0.0, "yaw": 0.0 }, "world_time": 12345.678 }

// sim → world（20~50 Hz）
{ "msg": "motion_cmd", "seq": 1234, "action": "forward", "speed": 50 }

// world → sim（manifest 到齐，每 peer 仅一次）
{ "msg": "assembly_ready", "peer_id": "12D3Koo...", "world_time": 12345.678 }

// world → sim（底盘连接；10 Hz 位姿下行，task_31 Step 1）
{ "msg": "pose", "world_time": 12345.678,
  "x": 64.0, "y": 64.0, "z": 0.0, "yaw": 0.0,
  "vx": 0.0, "vy": 0.0 }

// 传感器连接首包
{ "msg": "sensor_hello", "peer_id": "12D3Koo...", "slot": "lidar_front",
  "sensor": "lidar_2d",
  "mount": { "x": 0.1, "y": 0.0, "z": 0.2, "yaw": 0.0 },
  "params": { "range": 12.0, "rays": 360, "rate_hz": 10 } }

// world → sim
{ "msg": "sensor_ready", "slot": "lidar_front", "sensor_id": 3 }

// §六 已定义、v1 不做（见 §10）：imu / gnss 传感器零件与消息**均不实现**，
// 位姿由上面的 `pose` 文本消息提供（§8.1-8 / §9.1 E23）。
```

**实现约定（只补语义，不改 §六 设计）**：

1. `spawn` 增补 `z`：§六 示例只有 `x`/`y`，3D 世界需要高度（UGV=0，UAV=2.5）。**字段只增不改**，不破坏既有解析。
2. `action ∈ {forward, backward, left, right, up, down, stop}`；`speed` = **0~100 的百分比**（`"speed": 50` 即半速）。世界侧 `clamp(speed, 0, 100)`；未知 `action` 视为 `stop`（不报错、不崩）。
3. `seq`：发送端单调递增；世界侧**只用最新一条**（`SetMotionCommand` 覆盖），TCP 保序，不做重放 / 插值。
4. `motion_cmd` 的 20~50 Hz 与 60 Hz 物理解耦：世界每物理帧按"当前目标 + 速度环"推进。
5. 连接保活：`WebSocketPeer.set_heartbeat_interval(10.0)` + `get_ready_state()` 判死；**不做应用层 ping/pong**（§6.3-5 允许二者择一）。
6. 未知 `msg`：记 warning 后忽略，不断连接、不崩。
7. `sensor_id` / `avatar_id` 均由 manager 单调分配（`u16` 语义，v1 用 `int`）。
8. **`pose`（world → sim，task_31 Step 1）**：频率 **10 Hz**（`POSE_BROADCAST_INTERVAL_S = 0.1`，对齐真机 `state_notifier` 的 100 ms）；**只走底盘连接**（与 `motion_cmd` 同一条，传感器连接一条都不发）；`world_time` 单位=**秒**，与 `chassis_ready` / `assembly_ready` 同时间基（`WorldClock.Now()`）；`x`/`y`/`z`/`yaw` 取自 `Vehicle.GetPose()`（世界坐标 Z-up、米、yaw 弧度绕 +z、归一化 `[-PI, PI)`，**与 `chassis_ready.spawn` 同构**，静止时逐字段相等）；`vx`/`vy` 取自 `WorldTransform.VelToWorld(vehicle.velocity)`（世界 x=东、y=南）；`z` **必带**（UGV `0.0` / UAV `2.5`，消费方不得假设 `z==0`）；**不发** `vz` / `valid` / `sub_gx` / `sub_gy` / `rtk_fixed`（决策链产物归 sim、世界无 RTK）。

### 8.2 二进制帧（LiDAR，**小端**）

| offset | size | field | 说明 |
|---|---|---|---|
| 0 | 1 | `type` u8 | `1` = `lidar_2d`；`2` = camera（预留） |
| 1 | 2 | `slot_id` u16 | = `sensor_id`（`sensor_ready` 里下发的值） |
| 3 | 4 | `seq` u32 | 每连接独立递增 |
| 7 | 8 | `world_time` f64 | 秒，`WorldClock.Now()` |
| 15 | N | `payload` | lidar：`f32[rays]` 距离（米，`0.0` = 无回波） |

总长 = `15 + 4·rays`（360 线 → 1455 B）。

- GDScript 侧 `PackedByteArray.encode_u16/encode_u32/encode_double` 与 `PackedFloat32Array.to_byte_array()` 在 x86/ARM 上均为小端，与 Rust 侧 `from_le_bytes` 对齐。
- **Rust 侧按本表小端解析，不要依赖主机序**。
- 角度不上线：`slot_id` + `seq` + `world_time` 已足够；`rays` 与 `range` 由 `sensor_hello.params` 静态约定。

### 8.3 状态机与错误处理

| 情况 | 行为（Step 6 实际实现） |
|---|---|
| 首包不是 `chassis_hello` / `sensor_hello` | `push_warning` + 断开（**1002**，reason `first message must be *_hello`） |
| 首包非法 JSON / 非对象 / 缺 `msg` | `push_warning` + 断开（**1002**，`bad json` / `missing msg`）；**不打印 ERROR**（§9.1 E21） |
| 5 s（`handshake_timeout_ms`）内未握手 | Heartbeat 巡检发现超时 + 断开（1002，`hello timeout`） |
| `chassis_hello` 缺 `peer_id` / `chassis` | `push_warning` + 断开（1002） |
| `chassis` 名**不在目录** | `push_warning` + 断开（**1002**，`unknown chassis`） |
| `chassis` 名在目录、**场景文件缺失**（Step 7 前的 `quadrotor_uav`） | `push_warning` + 断开（**1011**，`chassis scene missing`）；不重建 avatar |
| `ApplyChassis()` 返回 false（装配失败） | `push_warning` + 丢弃刚建的空车 + 断开（1011） |
| `Vehicles` 容器缺失（E20 写错 NodePath） | `push_warning` + 丢弃空车 + 断开（1011） |
| `sensor_hello` 未知 `sensor`（目录已落地） | `push_warning`，**忽略该 slot**：不回 `sensor_ready`、不置位、**连接保持**（manifest 该 slot 永远不就绪，不误发 `assembly_ready`） |
| `SensorCatalog` 未落地（Step 8 前） | 没有名字表可校验 → 按"已知"登记（回 `sensor_ready`，几何为 null），只 warning 一次；就绪语义见 §4.5.1-③ |
| `sensor_hello` 的 slot 不在 manifest | 照常挂载 + 回 `sensor_ready`，但 `push_warning` 且**不置位**（不算就绪） |
| `sensor` 与 manifest 声明不一致 | 以 `slot` 为准照常挂载 + `push_warning` |
| 传感器先于底盘到达 | 进 `_pending_sensors`；pending 期间**不发** `sensor_ready`；底盘到位后回放 |
| pending 等待底盘超过 `pending_timeout_ms`（10 s） | 断开该连接（**1008**，`pending timeout`） |
| 同一 `peer_id` 重复 `chassis_hello`（该连接本身是底盘连接） | 先移除旧 avatar（`except_link=本连接`，不误关新链路），再新建；`assembly_ready` 不重发 |
| 已握手的**传感器**连接发 `chassis_hello` | `push_warning` + **忽略**（角色是单值，不改角色、不建车） |
| `motion_cmd` 出现在非底盘连接 | `push_warning` + 忽略（数据面只在底盘连接上） |
| 已握手连接发未知 `msg` | `push_warning` + 忽略，**不断连、不崩**（§8.1-6） |
| sim 发二进制帧 | `push_warning` + 丢弃该帧（二进制只下行，§8.2），不断连 |
| 连接断开 | 按 §5.3 清理；`_manifests` / `_manifest_parts` / `_assembly_sent` / `_pending_sensors` 同步清除 |

---

## 9 约定

### 9.1 硬约束（实测得来，违反会静默出错）

| # | 约束 | 原因 |
|---|---|---|
| E5 | **禁止全局 `class_name`** | 无 `.godot` 缓存的干净检出上，全局类名无法解析（`Parse Error: Identifier "X" not declared`）；一律 `preload()` 常量 / `extends "res://…gd"` |
| E4 | `--quit` 在脚本报错时**仍返回 0** | 验收必须同时 grep 日志（`SCRIPT ERROR` / `Parse Error` / `ERROR`） |
| E7 | `.tscn` 可省略 `uid` | 手写场景 `[gd_scene format=3]` + `[ext_resource type="Script" path=...]` 即可加载 |
| E8 | `.tscn` / `project.godot` 顶部可用 `;` 注释 | 头注释写在场景文件里；**编辑器另存会丢 `.tscn` 头注释**，以脚本为准 |
| E15 | `WebSocketPeer` 无 `send_binary` | 文本 `send_text()`，二进制 `send(bytes, WRITE_MODE_BINARY)` |
| E16 | 手写 `.tscn` 里 `@export` 的**节点引用**必须 `node_paths=PackedStringArray("a","b")` + `a = NodePath("A")`；漏写会**静默为 null**（4.7.2 实测：不报错、不告警） | 装配接口会整体失效且极难查；脚本侧按名回退（`_ensure_refs()`）只是兜底，自检 A5 直接断言绑定成功 |
| E17 | `CylinderMesh` 只有 `top_radius` / `bottom_radius` / `height` / `radial_segments`，**没有** `radius`；写错不报错、只是用默认值（r=0.5 / h=2.0 的巨柱） | 轮子静默变成"大圆柱"；`src/tests/check_diff_drive.gd` 直接断言半径/高度/轴向，把静默错误变红灯 |
| E18 | `CharacterBody3D.MOTION_MODE_GROUNDED == 0` **就是** `motion_mode` 的默认值（4.7.2 实测） | "底盘设了 GROUNDED"的断言会**假通过**；自检必须先把 `motion_mode` 置为 `FLOATING` 再断言被改回 |
| E19 | **`WebSocketPeer.close(code, reason)` 返回 `void`**（4.7.2 实测）：写成 `var e := peer.close(...)` 直接 **Parse Error** | `ComponentLink.Close()` 也返回 `void`；manager 侧 `_CloseLink()` 不取值。握手未完成（`STATE_CONNECTING`）时 `close()` **有效**：下一帧 `get_ready_state() == STATE_CLOSED`（握手超时用例依赖它） |
| E20 | `@export` **节点引用**在 `node_paths=PackedStringArray("x")` 里给出的 `NodePath` 是**相对该节点自身**解析的：`World/Systems/WebSocketManager` 的 `vehicles_root` 必须写 `../../Vehicles`（`..`=Systems、`../..`=World）；写 `../Vehicles` **静默 null**、不报错 | 装配整体失效且无任何日志；`_EnsureRefs()` 只是兜底，`src/tests/check_ws_manager.gd` 的 B2 直接断言 `manager.vehicles_root == World/Vehicles` |
| E21 | **`JSON.parse_string()` 遇非法输入会向 stderr 打印 `ERROR: Parse JSON failed`**（`core/io/json.cpp`），即使返回值只是 `null`；改用 `JSON.new().parse()` 只返回 `Error`，不打印 | 任何"必须把不可信输入挡在门外"的地方（`WireProtocol.ParseJson`、自检的 sim 客户端）都用 `JSON.new().parse()`；否则 `grep ERROR` 验收永远不干净 |
| E22 | **`ImmediateMesh` 没有 `surface_get_array_len()` / `surface_get_primitive_type()`**（4.7.2 实测 → `Invalid call`）；点云只能用 `surface_begin(Mesh.PRIMITIVE_POINTS, mat)` / `surface_add_vertex()` / `surface_end()` / `clear_surfaces()` / `get_surface_count()` / `get_aabb()` | `Lidar2D._RefreshDebug()` 与自检 `F4-white-box-lint` 都直接禁止这两个符号；误用会 **`SCRIPT ERROR` 且进程不退出**（`quit()` 不执行），所以所有 Godot 命令必须带 `timeout` |
| E23 | **位姿只能取 `Vehicle.GetPose()`**（= `WorldTransform.PoseFromTransform`，已换轴的世界坐标）。❌ 不得发 `global_position`（Godot Y-up：y=高度、z=南向）；❌ 不得发 `rotation.y`（= `-world_yaw`，符号相反）；❌ 不得自己用 `basis` 算 yaw。**前提**：`PoseFromTransform` 用 `basis.get_euler().y`，仅在 avatar **只绕 Y 旋转**时成立（当前 `Vehicle` 只写 `rotation.y`，无 roll/pitch；以后引入倾斜/翻车即失效，需重做） | 违反任一条 → 方位/高度**静默错**，而 `pose` 是 sim 的唯一位置来源，错了整车定位就错；`src/tests/check_pose.gd` 的 `A7 broadcast-lint` 把这三条禁止变成可执行红灯（`_BroadcastPoses` 函数体里禁止出现这三个符号），`C4 pose-matches-spawn` 再从数据面卡一遍（静止位姿 == `chassis_ready.spawn`） |
| E24 | **空气墙碰撞面必须"内侧面正好贴世界边界平面"**（`x=0/128`、`y=0/128`、`z=32`），厚度一律**向外**；且**不得用 `MeshInstance3D` / 任何 mesh 渲染**（必须不可见） | 贴反（厚度朝内）→ 车中心可到 `-0.5`，`WorldToCell` 出**负格坐标**，SLAM 地图 / 边界判据错乱，且车能"嵌进"边界里 0.5 m；加了 mesh → "不可见硬边界"语义破坏，`Bounds` 会被误当可视几何。执行者：`src/tests/check_bounds.gd` 的 `B1-bounds-structure`（递归 `MeshInstance3D` 计数 == 0）/ `B1b-walls-geometry`（四面墙 AABB 与内侧面数值断言，容差 `1e-3`）/ `B1c-lid-geometry`（顶盖下表面 `y==32`）；副闸：`src/tests/check_world.gd` 的 `ground-no-hole`（`mask=1` 向下射线，若墙放进界内会先打到墙） |
| E25 | **UAV 高度软件限幅的口径**（task_32 Step 3）：`Quadrotor.ClampAltitude()` 是**安全限幅**、不是运动学——位置推进仍只由 `Vehicle.move_and_slide()` 完成（D8）。写法硬约束：①夹的必须是**宿主车身**（`CharacterBody3D` 在 `Vehicle` 根节点上），夹 `Quadrotor` 自身（local 恒 0）毫无作用；②必须用 `global_transform`（只改 `origin.y`，保留 `basis`），**不得**出现 `global_position`——`src/tests/check_quadrotor.gd` 的 `BANNED_TOKENS` 会直接 FAIL（唯一合法例外是**整行** `#` 注释，该 lint 只剥离整行注释）；③上限取 `World.AIR_WALL_HEIGHT_M - CLAMP_MARGIN_M`（= 31.8），**不得**再写一个 `31.8` 字面量（`clamp-ceiling-derivation` 断言两份常量自洽）；④越界只 `push_warning` 且 **1 s 限流**，**不得** `push_error`（验收要求 0 ERROR）；⑤不引入参数、不进 `params`（参数表固定 4 键，§7.2 / §7.6） | 无 clamp → 物理失效时 UAV 能飞出 32 m 顶盖；用 `global_position` → 白盒 lint 红（若改用别的写法绕开 lint，就把"位置只由 Vehicle 推进"这条硬约束悄悄废掉了）；把 `31.8` 写死第二遍 → 改世界高度时两处不一致，限幅变成隐性 bug |

### 9.2 命名规范

| 对象 | 规范 | 例 |
|---|---|---|
| 函数 / 方法 | `PascalCase` | `ApplyChassis()`、`ScanOnce()` |
| 变量 / 成员 | `snake_case`（私有加 `_`） | `_peer_vehicles`、`ray_count` |
| 常量 | `UPPER_SNAKE` | `MAX_SPEED`、`FRAME_HEADER_SIZE` |
| 信号 | `snake_case` | `chassis_installed`、`scan_produced` |
| 引擎回调 | 保持原名 | `_ready`、`_physics_process`、`_unhandled_input` |
| 场景 / 节点 | `PascalCase`；动态子节点名 = `slot` / `peer_id`（净化） | `VoxelWorld`、`lidar_front` |

### 9.3 文件头注释

所有 `.gd` / `.md` / `project.godot` / `.tscn` 顶部：

```
Presented by KeJi
Created Date ： YYYY-MM-DD
Modified Date ： YYYY-MM-DD
```

`.tscn` / `project.godot` 用 `;` 前缀，`.gd` 用 `#` 前缀，`.md` 用 `>` 引用块。

### 9.4 其它

- **一个组件 = 一个 `.tscn` + 同名脚本**；场景不内嵌脚本（不写 `[sub_resource type="GDScript"]`）。
- 场景里不硬编码 `res://` 路径，跨场景引用走 `@export`。
- **代码与场景一律在 `src/`**：`src/scenes/` 放 `.tscn`，`src/<模块>/` 放 `.gd`，`src/tests/` 放自检；`assets/` 只放媒体 / 建模资源（png / jpg / webp / glb / gltf / obj / fbx / material / tres / env / wav / ogg），**不得**把 `.gd` / `.tscn` 放进 `assets/`。
- `.gd.uid` 是 Godot 4.4+ 的编辑器旁路文件：`--headless` 运行不产生；若编辑器导入后出现可保留（不影响按路径加载），不视为新增设计文件。
- 不新增未列入 §3 的 `.tscn`；新增必须先进本文 §3 表格。

---

## 10 v1 边界

| 项 | v1 状态 | 说明 |
|---|---|---|
| 传感器 | **只有 `lidar_2d`**（D6） | `imu` / `gnss` **不做**（不是零件、也不发消息）：位姿由 `pose` 文本消息提供，见 §8.1-8 / §9.1 E23 |
| 位姿下行 | `pose`（10 Hz，走底盘连接） | task_31 Step 1；**理想真值**，无噪声 / 无延迟 / 无丢包；`z` 由世界给（UGV `0.0` / UAV `2.5`），消费方不得假设 `z==0`；`vz` 不上线 |
| 底盘 | `diff_drive_ugv` / `quadrotor_uav` 两种（D6） | 目录外名字直接拒绝 |
| 世界边界 | **可见 2 m 体素矮墙 + 不可见 32 m 空气墙 + 32 m 顶盖**（空气墙在 layer 1，与地面同层） | task_32 Step 2；矮墙只作视觉 / 近距障碍，真正的硬边界是空气墙：20 m/s 四面冲撞 10 s 与 5 m/s 持续爬升 10 s 都出不去（`src/tests/check_bounds.gd` 的 B2~B6）；雷达贴边时会把空气墙扫成静态障碍 → SLAM 地图边缘出现"墙"，**预期行为**（task_32 §三.5-3）。另：UAV 还有 `[0, 31.8]` **软件 clamp 兜底**（`Quadrotor.ClampAltitude()`，task_32 Step 3），即使物理失效也飞不出 32 m 顶盖 |
| LiDAR 噪声 | 默认关闭 | `_rng` 保留，`params.noise` 未启用 |
| 相机传感器 | 不实现 | 二进制 `type=2` 预留 |
| UAV 高度控制 / 三维寻路 / 三维避障 | **不在本任务**（§九） | UAV 决策链仍是 2D（**不控高度**）；世界侧巡航高度 **2.5 m**（`WebSocketManager.UAV_ALTITUDE_M`，task_32 Step 3）由世界给，另有 `[0, 31.8]` 软件 clamp 兜底；本任务只给它"3D 身体" |
| sim 侧 `WorldLink` | **不在本任务**（§九） | sim 侧接入是另一条线；本任务只做 Godot 世界侧 |
| 仓库 / 地图编辑 | 不实现 | 世界障碍由 `World.gd` 程序化稀疏生成（D4） |
| 多世界 / 分场景 | 不实现 | 一个 Godot 进程 = 一个世界 |

---

## 11 step ↔ 文件映射

| Step | 新增 / 修改文件 | 验收要点 |
|---|---|---|
| 1 | `COMPONENTS.md`、`project.godot`、`src/scenes/World.tscn`（占位）、`src/tests/check_project.gd`、目录骨架（`src/` + `assets/`） | 本文覆盖四项；`godot --headless --path . --quit` → `EXIT=0` 且无 `ERROR`；`--script res://src/tests/check_project.gd` → `SELF-CHECK OK` |
| 2 | `src/scenes/World.tscn`（扩展）、`src/world/World.gd`、`src/common/{WorldTransform,WorldClock}.gd`、`src/tests/check_world.gd` | 可见 128×128×32 m 体素世界；物体被地面挡住不下坠 |
| 3 | `src/scenes/ObserverCamera.tscn`、`src/camera/ObserverCamera.gd`、`src/tests/check_camera.gd`、`src/scenes/World.tscn`（+实例，−DebugCamera）、`src/tests/check_world.gd`（相机断言改指 `ObserverCamera`） | 三种视角可切换；转镜头不影响物理 |
| 4 | `src/scenes/Vehicle.tscn`、`src/vehicle/Vehicle.gd`、`src/tests/check_vehicle.gd`、`src/tests/doubles/{TestChassis,TestSensor}.gd` | 能实例化"空车"；底盘与传感器可动态挂载（`check_vehicle.gd`：36 项断言 / `total=36 failed=0` / `SELF-CHECK OK`；底盘与传感器替身用 `PackedScene.pack()` 现场打包，不落 `.tscn`） |
| 5 | `src/scenes/chassis/DiffDrive.tscn`、`src/chassis/{ChassisModel,DiffDrive,ChassisCatalog}.gd`、`src/tests/check_diff_drive.gd` | 前进 / 后退 / 转向；撞墙停（`check_diff_drive.gd`：41 项断言 / `total=41 failed=0` / `SELF-CHECK OK`；全部走真实 `src/scenes/World.tscn` + `src/scenes/Vehicle.tscn`，不新增 `.tscn`） |
| 6 | `src/scenes/WebSocketManager.tscn`、`src/net/{WebSocketManager,ComponentLink,WireProtocol}.gd`、`src/scenes/World.tscn`（+Systems 实例 + `node_paths` 绑定）、`src/tests/check_ws_manager.gd` | 一个 sim 连上并完成装配，收到 `assembly_ready`（`check_ws_manager.gd`：**51 项断言** / `total=51 failed=0` / `SELF-CHECK OK`；同一进程内真 `src/scenes/World.tscn` + 真 `WebSocketPeer` 客户端，日志链路 `accept → chassis_hello → vehicle spawned → sensor_ready → assembly_ready → link closed → vehicle removed`） |
| 7 | `src/scenes/chassis/Quadrotor.tscn`、`src/chassis/Quadrotor.gd`、`src/tests/check_quadrotor.gd` | 无重力悬停；按指令升降 / 平移（`check_quadrotor.gd`：**48 项断言** / `total=48 failed=0` / `SELF-CHECK OK`；全部走真实 `src/scenes/World.tscn` + `src/scenes/Vehicle.tscn`，不新增 `.tscn`。证据链：`hover-idle-300f`（2.5 m 处 `dy=0.00000000`）+ `hover-high-altitude`（10 m 处 `dy=0.00000000`）+ `white-box-boundaries`（无任何重力调用）证明"无重力悬停"；`climb-60f` / `descend-60f` / `land-on-ground` / `land-still` × `forward-60f` / `backward-60f` / `yaw-left-60f` / `yaw-right-60f` 证明"能按指令升降与平移"；task_32 Step 3 追加 `clamp-ceiling-pins` / `clamp-ceiling-holds-under-climb` / `clamp-ceiling-derivation` 证明"高度软件限幅"真在工作） |
| 8 | `src/scenes/sensors/Lidar2D.tscn`、`src/sensors/{Lidar2D,SensorCatalog}.gd`、`src/tests/check_lidar.gd` | 一圈扫描正确反映周围障碍距离（`check_lidar.gd`：**48 项断言** / `total=48 failed=0` / `SELF-CHECK OK`；全部走真实 `src/scenes/World.tscn` + `src/scenes/Vehicle.tscn`，不新增 `.tscn`。证据链：`D2-forward-distance` **3.4000 m** + `D4-side-distance` **3.0000 m** + `D6-follows-position` **2.4000 m** + `D7-mount-yaw-rotates` **3.0000 m** + `F7-real-wall-distance` **2.0000 m**（真实 `wall_c`）+ `F8-real-wall-hits` **152** 条证明"距离正确"；`E1-rate-10hz` 60 帧 10 圈 + `E4-frame-format` 1455 B / `type=1` / `slot_id=7` / `seq` 递增证明"10 Hz + §8.2 帧格式"） |

| task_32-2 | `src/world/World.gd`（+`_Build_Bounds()` / `AIR_WALL_HEIGHT_M`·`AIR_WALL_THICKNESS_M`·`AIR_WALL_LAYER` / `AIR_WALL_BOXES` 几何表）、`src/tests/check_bounds.gd`（**新建**）、`COMPONENTS.md`（§2 / §2.1 / §2.2 / §4.1 / §9.1 E24 / §10 + §1.1 / §1.4 / §3 / §11 / 附） | 不可见硬边界落地：`World/Bounds`（`StaticBody3D`，layer 1）+ 5 个 `CollisionShape3D`（4 面 32 m 空气墙 + 32 m 顶盖），`_Build_Bounds()` 运行时建、`.tscn` **零改动**。`check_bounds.gd`：**12 项断言** / `total=12 failed=0` / `SELF-CHECK OK`；9 个旧自检断言数与基线完全一致（`project` / `world 26` / `camera 32` / `vehicle 36` / `diff_drive 41` / `quadrotor 45` / `ws_manager 51` / `lidar 48` / `pose 23`，`failed=0`；`quadrotor` 在 task_32 Step 3 后为 **48**）；`--quit` EXIT=0 且 0 ERROR |

| task_32-3 | `src/net/WebSocketManager.gd`（`UAV_ALTITUDE_M` 2.0 -> **2.5**）、`src/chassis/Quadrotor.gd`（+`ClampAltitude()` / `_physics_process()` / `CLAMP_MARGIN_M` / `CLAMP_WARN_INTERVAL_MS`）、`src/tests/check_quadrotor.gd`（**45 -> 48**：`SPAWN_WORLD.z` 2.5 + 3 项 clamp 断言）、`src/tests/check_ws_manager.gd`（B5 UAV spawn `z` 2.5）、`COMPONENTS.md`（§4.5.1-④ / §4.6 / §7.3 / §8.1 / §9.1 E25 / §10 / §11 / 附） | UAV 巡航高度 **2.5 m**（越过 2 m 可见矮墙）+ `[0, 31.8]` 软件 clamp 兜底。**10 个自检全 PASS**（`quadrotor 48`，其余与 task_32-2 基线完全一致）+ 冒烟 ugv/uav 双通（两侧装配日志 + `status.assembled=true` + UAV `z=2.5`），`--quit` EXIT=0 且 0 ERROR |

> **task_31 Step 1 追加**（不属于 task_30 的 Step 1~8）：`pose` 下行（10 Hz，走底盘连接）+ `src/tests/check_pose.gd`（**23 项断言** / `total=23 failed=0` / `SELF-CHECK OK`）。改动文件仅 `src/net/{WireProtocol,WebSocketManager}.gd`、新建 `src/tests/check_pose.gd`、本文；`pleiades-sim/**` 一行未动（消费在 task_31 后续 step）。

### 11.1 施工顺序的依赖关系

```
Step 1（本文 + 骨架）
  └─► Step 2（世界 + common 工具）        ← 后续所有 step 依赖 WorldTransform / WorldClock
        ├─► Step 3（相机，只依赖输入映射）
        ├─► Step 4（Vehicle）             ← 依赖 World/Vehicles 容器
        │     ├─► Step 5（DiffDrive）     ← 依赖 ChassisModel 契约 + Vehicle/Chassis 挂载点
        │     ├─► Step 7（Quadrotor）     ← 依赖 ChassisModel 契约
        │     └─► Step 8（Lidar2D）       ← 依赖 Vehicle/Sensors 挂载点 + WireProtocol
        └─► Step 6（WebSocketManager）    ← 依赖 Vehicle + ChassisCatalog + SensorCatalog（均用 load()）
```

> 注意 Step 6 早于 Step 7/8 落地：`ChassisCatalog` / `SensorCatalog` 必须运行时 `load()`，不得 `preload()`。
> Step 6 落地时 `SensorCatalog` 尚不存在，因此 manager 用 **三态查询**（`_CatalogStatus`：KNOWN / UNKNOWN / MISSING）区分
> 「名字非法」与「目录还没落地」——后者按已知放行，否则 Step 6 无法产出 `assembly_ready`（§4.5.1-③）。

---

## 附：落地文件清单

| 文件 | 状态 |
|---|---|
| `project.godot` | ✅ 完整（含 11 个相机输入动作、物理层名、`[sim_world]` 尺度与端口） |
| `src/scenes/World.tscn` | ✅ Step 1 占位 → Step 2 扩展为完整世界 → **Step 3 用 `ObserverCamera` 实例替换临时 `DebugCamera`**（`World` + `WorldEnvironment` + `Sun` + `Ground`(+2 子) + `VoxelWorld` + `Vehicles` + `Systems` + `ObserverCamera`） |
| `src/scenes/ObserverCamera.tscn` | ✅ Step 3 观察者相机（`Camera3D` + 6 个引擎原生属性，`current=true`） |
| `src/camera/ObserverCamera.gd` | ✅ Step 3 三视角脚本（零依赖、无物理 API、只用 `_unhandled_input`） |
| `src/scenes/Vehicle.tscn` | ✅ Step 4 `CharacterBody3D` + `Body`(`Nose`) + `CollisionShape3D` + `Chassis`/`Sensors` 两个空挂载点；`collision_layer=4`、`collision_mask=7`；4 个 `@export` 节点引用靠 `node_paths` 绑定（E16） |
| `src/vehicle/Vehicle.gd` | ✅ Step 4 装配（挂底盘 / 挂传感器 / 卸传感器）+ 物理步进（`move_and_slide()` 是唯一位置更新点）+ 位姿 / 采样查询；含 `apply_motion` / `sample_sensors` / `get_pose` 三个契约别名 |
| `src/tests/check_vehicle.gd` | ✅ Vehicle 自检（Step 4；`SELF-CHECK OK` + `exit 0`，36 项断言） |
| `src/scenes/chassis/DiffDrive.tscn` | ✅ Step 5 差速底盘组件（`Node3D` + `Wheels` + `WheelFL/FR/RL/RR`；`CylinderMesh` r=0.06 h=0.04 纯视觉；无内嵌脚本） |
| `src/chassis/ChassisModel.gd` | ✅ Step 5 底盘基类（4 个契约方法 + `GetParam`/`SpeedPct`/`ActionOf`/`RateToRad`/`Approach`/`ApplyMotionMode`/`ApplyBodySize`；不含运动学） |
| `src/chassis/DiffDrive.gd` | ✅ Step 5 差速底盘（`MOTION_MODE_GROUNDED` + 世界侧速度环 + 差速运动学；只给控制量，不写 position） |
| `src/chassis/ChassisCatalog.gd` | ✅ Step 5 底盘目录（`SCENES` / `DEFAULTS` + `Has` / `GetScene`（`load()`+缓存）/ `GetDefaults`；`quadrotor_uav` 已于 Step 7 落地） |
| `src/scenes/chassis/Quadrotor.tscn` | ✅ Step 7 四旋翼底盘组件（`Node3D` + `Rotors` + `RotorFL/FR/RL/RR`；`CylinderMesh` 平盘 r=0.12 h=0.02 `radial_segments=16` 纯视觉；无内嵌脚本、无 `uid`） |
| `src/chassis/Quadrotor.gd` | ✅ Step 7 四旋翼底盘（`MOTION_MODE_FLOATING` + 世界侧速度环 + 三轴运动学：平移 / 爬升 / 偏航；不施加重力、不写 position）+ **task_32 Step 3 高度软件限幅**（`ClampAltitude()`：宿主车身世界 z 夹到 `[0, 31.8]`，越界 `velocity.y = 0` + 1 s 限流 warn；用 `global_transform`，不动运动学与参数表） |
| `src/tests/check_diff_drive.gd` | ✅ DiffDrive 自检（Step 5；`SELF-CHECK OK` + `exit 0`，41 项断言：形态 / 目录装配 / 运动语义 / 撞墙会停 / 白盒边界） |
| `src/tests/check_quadrotor.gd` | ✅ Quadrotor 自检（Step 7；`SELF-CHECK OK` + `exit 0`，**48 项断言**：形态 / 目录装配 / 无重力悬停 / 升降 / 平移 / 偏航 / 落地 / 边界与白盒 + task_32 Step 3 的 3 项高度软件限幅） |
| `src/tests/doubles/{TestChassis,TestSensor}.gd` | ✅ Step 4 自检替身（鸭子契约；`PackedScene.pack()` 现场打包，不落 `.tscn`、不进 §3 组件清单） |
| `src/tests/check_project.gd` | ✅ 工程骨架自检（`SELF-CHECK OK` + `exit 0`） |
| `src/tests/check_world.gd` | ✅ 世界自检（Step 2；Step 3 相机断言改指 `ObserverCamera`，`SELF-CHECK OK` + `exit 0`，26 项断言） |
| `src/tests/check_camera.gd` | ✅ 相机自检（Step 3；`SELF-CHECK OK` + `exit 0`，32 项断言，含 4 项物理不变量） |
| `src/scenes/WebSocketManager.tscn` | ✅ Step 6 组件（`Node` + `Heartbeat(Timer, 1 s, autostart)`；无内嵌脚本、无节点引用导出 → 不需要 `node_paths`） |
| `src/net/WireProtocol.gd` | ✅ Step 6 协议唯一入口（7 个消息名：`chassis_hello` / `chassis_ready` / `sensor_hello` / `sensor_ready` / `motion_cmd` / `assembly_ready` / **`pose`（task_31 Step 1）**；`imu` / `gnss` 已删除不再预留；`ParseJson`/`MakeJson`/`MakeJsonText`/**`BuildPose`**/`BuildLidar2DFrame`/`ParseFrameHeader`/`FramePayloadSize`） |
| `src/net/ComponentLink.gd` | ✅ Step 6 单连接封装（`RefCounted`；`Init`/`Poll`/`TakePackets`/`SendJson`/`SendBinary`/`NextSeq`/`IsOpen`/`IsConnecting`/`Close`（void）/`Describe`；`last_error`） |
| `src/net/WebSocketManager.gd` | ✅ Step 6 服务端（监听 / accept / 两张表 / hello 装配 / manifest 就绪 / `assembly_ready` / 断开清理 / `Heartbeat` 巡检；全静态实例、无 `push_error`；`UAV_ALTITUDE_M` 于 task_32 Step 3 改 **2.5 m**）+ ✅ task_31 Step 1 位姿下行（`POSE_BROADCAST_INTERVAL_S = 0.1` / `_pose_acc` / `_BroadcastPoses()`，只发已握手且活着的底盘连接） |
| `src/tests/check_ws_manager.gd` | ✅ WebSocketManager 自检（Step 6；`SELF-CHECK OK` + `exit 0`，**51 项断言**：场景形态 / World 接线 / 按序装配 / 运动下行 / 传感器先到 / 空 manifest / 断开清理 / 错误路径 / 规范 lint；Step 8 后 `C14 sensor-geometry-consistency` 由"双双为假"变为**双双为真**） |
| `src/scenes/sensors/Lidar2D.tscn` | ✅ Step 8 传感器组件（`Node3D` + `DebugPoints(MeshInstance3D, visible=false)`；`node_paths=PackedStringArray("debug_points")`；无内嵌脚本、无 `uid`） |
| `src/sensors/Lidar2D.gd` | ✅ Step 8 2D 单线激光（360 条水平射线 / 10 Hz / `BuildLidar2DFrame` 二进制下发 + `LaserScan`；不写 position、不发 JSON、不 preload `ComponentLink`） |
| `src/sensors/SensorCatalog.gd` | ✅ Step 8 传感器目录（`SCENES` / `DEFAULTS` + `Has` / `GetScene`（`load()`+缓存）/ `GetDefaults`；与 `ChassisCatalog` 同构） |
| `src/tests/check_lidar.gd` | ✅ Lidar2D 自检（Step 8；`SELF-CHECK OK` + `exit 0`，**48 项断言**：场景形态 / 目录 / 装配 / 一圈几何 / 10 Hz 时序 / 帧格式 / 边界与白盒） |
| `src/tests/check_pose.gd` | ✅ 位姿下行自检（task_31 Step 1；`SELF-CHECK OK` + `exit 0`，**23 项断言**：`A1~A7` 协议形态与白盒 lint（E23 三条禁止）/ `B1~B2` World 接线 / `C1~C14` 端到端（10 Hz = 60 帧 10 条、8 键字段、与 `spawn` 一致、yaw 范围、`world_time` 单调、传感器连接 0 条、forward 的 `x`/`vx`、left 的 `yaw`/`vy`、断开后不再发、`vehicle==null` 守卫、每周期每连接恰好一次）） |
| `src/tests/check_bounds.gd` | ✅ 空气墙 / 顶盖自检（task_32 Step 2；`SELF-CHECK OK` + `exit 0`，**12 项断言**：`B1` 结构 / layer / 无 mesh + `B1b` 四面墙内侧面贴边界 + `B1c` 顶盖下表面 32 m + `B1d` 由代码构建（`.tscn` 无 `Bounds`）+ `B1e` 幂等重建 + `B2~B5` 四面 20 m/s 冲撞 10 s 出不去 + `B6` 5 m/s 爬升被顶盖挡住 + `B7` 地面自由落体静止 + `B8` 可见 2 m 矮墙各 1024 格仍在） |
| `src/world/World.gd` | ✅ Step 2 世界根脚本 + **task_32 Step 2 空气墙 / 顶盖**（`_Build_Bounds()`、`World/Bounds` layer 1、5 个 box、`AIR_WALL_*` 常量与 `AIR_WALL_BOXES` 几何表；`.tscn` 零改动） |
| `COMPONENTS.md` | ✅ 本文 |
| 目录骨架 | ✅ `src/scenes/`（含 `chassis/` `sensors/`）+ `src/{common,world,camera,vehicle,chassis,sensors,net}/` + `src/tests/`（含 `src/tests/doubles/`）+ `assets/`（`.gitkeep` + `README.md`，当前无资源文件） |

**已全部落地**（§11 的 Step 1~8 均已按本文完成）。
> `quadrotor_uav` 已于 Step 7 落地（`src/scenes/chassis/Quadrotor.tscn` + `src/chassis/Quadrotor.gd`），
> `ChassisCatalog.GetScene("quadrotor_uav")` 不再返回 `null`——§4.5.1-③ / §8.3 里"Step 7 前场景缺失 → 1011"是 **Step 6 的历史实测记录**，保留不改。
> `src/scenes/sensors/Lidar2D.tscn` / `src/sensors/SensorCatalog.gd` 已于 Step 8 落地：`_CatalogStatus()` 由 `CATALOG_MISSING` 转为
> `CATALOG_KNOWN`/`CATALOG_UNKNOWN` 两态，`_MountSensor()` 走"目录有 + 场景存在 → 真正实例化几何"分支，
> `sensor_ready` 之后 `SetStream()` 绑定下行流（§5.3 时序铁律），§4.5.1-③ / §8.3 里"Step 8 前只登记连接"同为历史记录。
> **task_31 Step 1（位姿下行）**：`pose` 文本帧已落地（`MSG_POSE` + `WireProtocol.BuildPose` +
> `WebSocketManager._BroadcastPoses`，10 Hz 只发底盘连接），自检 `src/tests/check_pose.gd`（**23 项断言**）。
> `MSG_IMU` / `MSG_GNSS` 两个预留常量已删除，§10 中 `imu` / `gnss` 由"预留"改为"**不做**"——位姿走 `pose`（§8.1-8 / E23）。
