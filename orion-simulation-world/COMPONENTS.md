# orion-simulation-world 组件总览（COMPONENTS.md）

> Presented by KeJi
> Created Date ： 2026-10-21
> Modified Date ： 2026-10-25
>
> 对应任务：`Task/task_30_sim_world.md`（sim 接入 Godot 模拟世界）；`Task/task_33_camera_simulation.md`（相机仿真 + 传感器 / 底盘建模，2026-10-23 起）；`Task/task_34_camera.md`（camera 传感器全链路，2026-10-24 起）
> 状态：**Step 1 产出；Step 2（世界）/ Step 3（观察者相机）/ Step 4（Vehicle）/ Step 5（差速底盘 `DiffDrive`）/ Step 6（`WebSocketManager`）已按本文落地**。**task_33 Step 1（建模）/ Step 2（相机组件 `CameraSensor`，§4.8）亦已落地**。**task_34 Step 3（`camera` 成为可装配零件 + `type=2` 帧下发，§4.8.3 / §8.2）亦已落地**。**支线 `human_model` Step 4（静态人物道具 `Person`）亦已落地**：§3 增第 9 个 `.tscn`（`src/scenes/props/Person.tscn`），`World.gd` 在 `World/Persons` 下**运行时**静态摆 2 人（纯视觉、**零碰撞**、不进装配线，§4.9）。**支线 `walking_person` Step 2（走路人物道具 `PersonWalking`）亦已落地**：§3 增第 10 个 `.tscn`（`src/scenes/props/PersonWalking.tscn`），`World.gd` 在 `World/Walkers` 下**运行时**放 1 个沿 44 m 闭合路径走动的人（自带骨骼 + 走路动画、`speed_scale` 跟速度、**零碰撞**、不进装配线，§4.10）。**`walking_person` Step 3（走动收口）亦已落地**：走动能力**并入 `Person.gd`**（新增 `walk_speed` / `path` / `loop` 三个字段，**默认静态 = 向后兼容**，`Person.tscn` 一个字节不动）、`PersonWalking.tscn` 与静态人物**共用同一个 `Person.gd`**（`PersonWalking.gd` 已删除）、资产目录改名 `assets/humans/rp_nathan_animated_003_walk/`、自检换成 `check_person_walk.gd`（20 项 headless / 26 项含渲染，§4.10-⑤）。**`walking_person` Step 4（三帧真渲染证明"人在走"）亦已落地**：新增 `src/tests/check_person_walk_shots.gd`（**14 项**：结构 4 + 渲染 10），在环路第一段**侧前方**架固定机位、以 `--fixed-fps 60` **真推进 0/40/80 物理帧**（**帧间绝不瞬移**）拍 `walk_t{0,1,2}.jpg`，把「物理帧数 / 弧长增量 / 每帧步长 == v/60」与「框位移 / 框内外差分 / 64×64 对齐裁剪差」一起断言（§4.10-⑦）。**task_35 Step 3（世界布局：用外部驱动的行人替换自动绕圈的 walkers）亦已落地**：`World.gd` 的 `WALKERS` /

> **2026-10-28：静态装饰人已全部移除。** `World.gd` 不再有 `PERSONS` / `BuildPersons()` / `World/Persons` 容器；
> `Person.tscn` / `PersonWalking.tscn` / `Person.gd` 与四个 `check_person_*` 自检一并删除。
> 世界不再预置任何人，行人一律由外部按需创建（`chassis_hello{kind:"pedestrian"}` → `Pedestrian.tscn`，见 §4.11）。
> 下文 §4.9 / §4.10 及目录树中关于 `Person*` 的描述为历史记录，不再代表当前代码。

`BuildWalkers()` / `GetWalkersRoot()` 整节删除，换成 `PEDESTRIANS` / `BuildPedestrians()` / `GetPedestriansRoot()`
（容器 `Walkers` → `Pedestrians`，与 `WebSocketManager` 动态装配的行人**共用同一容器**）；世界**不再内置任何路径 / 巡逻 / 计时**，
没有外部驱动时人**站着不动**（idle = 预期，§4.11.1 / §4.10-⑧）。`PERSONS`（静态两人）一字不动。**task_35 Step 5（端到端验收）亦已落地**：只**新增**两个自检
（`src/tests/check_pedestrian_physics.gd` **22 项** / `src/tests/check_pedestrian_shots.gd` **14 项**），
生产代码**一字未改**；把「直行 10 s 位移 ≈ v×t（+0.03%）/ `dir` 阶跃 180.0 °/s 弧线转弯（Kasa 拟合 r=0.3199 m vs v/ω=0.3183，+0.5%）
/ 停发 300 ms 后 `v→0` 且 1.1 世界秒位移 0.000000 m / 人推车 300 物理帧车位移 0.000000000 m 且不穿透 / 车头雷达 ±45° 扇区 2.2500 m 命中人 /
Xvfb 下走-转-停三帧 1280×720 JPEG」全部跑成实测（§4.11.2）。既有 19 项自检断言数**逐字不变**。
本文是 Step 2~8 的唯一施工依据；后续 step 若需偏离，必须**先改本文并说明理由**。
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
2. 组件清单固定 **10 个 `.tscn`**（§3）：task_30 / task_33 / task_34 的 8 个 + 支线 `human_model` Step 4 的 `src/scenes/props/Person.tscn` + 支线 `walking_person` Step 2/3 的 `src/scenes/props/PersonWalking.tscn`（**与 `Person.tscn` 共用根脚本 `src/props/Person.gd`**，只把 `Body` 换成带骨骼 + 动画的 glb）。Step 1 只落 `src/scenes/World.tscn` 的**占位**版本，其余 7 个由 Step 3~8 与 task_33 产出。**`Person` 不进装配线**（没有 `chassis_hello` / `sensor_hello` 语义、不被 `ChassisCatalog` / `SensorCatalog` 登记、不由 manager 实例化）。
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
│   ├── scenes/                   # 全部 .tscn（组件场景，§3 的 10 个）
│   │   ├── World.tscn            # 主场景（Step 2 完成态；Step 3/6 加挂点实例）
│   │   ├── ObserverCamera.tscn   # [Step 3] ✅ 已落地
│   │   ├── Vehicle.tscn          # [Step 4] ✅ 已落地
│   │   ├── WebSocketManager.tscn # [Step 6] ✅ 已落地
│   │   ├── chassis/
│   │   │   ├── DiffDrive.tscn    # [Step 5] ✅ 已落地
│   │   │   └── Quadrotor.tscn    # [Step 7] ✅ 已落地
│   │   ├── sensors/
│   │       ├── Lidar2D.tscn      # [Step 8] ✅ 已落地
│   │       └── CameraSensor.tscn # [task_33 Step 2] ✅ 已落地（离屏渲染 → JPEG）
│   │   └── props/
│   │       └── Person.tscn       # [已移除 2026-10-28] 静态人物道具（历史）
│   ├── common/                   # WorldTransform.gd / WorldClock.gd            [Step 2]
│   ├── world/                    # World.gd                                      [Step 2]
│   ├── camera/                   # ObserverCamera.gd                             [Step 3] ✅
│   ├── vehicle/                  # Vehicle.gd                                    [Step 4] ✅
│   ├── chassis/                  # ChassisModel.gd / DiffDrive.gd / ChassisCatalog.gd  [Step 5] ✅
│   │                             # Quadrotor.gd                                  [Step 7] ✅
│   ├── sensors/                  # Lidar2D.gd / SensorCatalog.gd                 [Step 8] ✅
│   │                             # CameraSensor.gd                               [task_33 Step 2] ✅
│   ├── props/                    # Person.gd                                   [human_model Step 4] ✅
│   ├── net/                      # WebSocketManager.gd / ComponentLink.gd / WireProtocol.gd  [Step 6] ✅
│   └── tests/
│       ├── check_project.gd      # 工程骨架自检（Step 1，非组件）
│       ├── check_world.gd        # 世界自检（Step 2，非组件）
│       ├── check_camera.gd       # 相机自检（Step 3，非组件）
│       ├── check_vehicle.gd      # Vehicle 自检（Step 4，非组件；40 项断言）
│       ├── check_diff_drive.gd   # DiffDrive 自检（Step 5，非组件；41 项断言）
│       ├── check_ws_manager.gd   # WebSocketManager 自检（Step 6，非组件；76 / 77 项断言）
│       ├── check_quadrotor.gd    # Quadrotor 自检（Step 7，非组件；48 项断言）
│       ├── check_lidar.gd        # Lidar2D 自检（Step 8，非组件；48 项断言）
│       ├── check_pose.gd         # 位姿下行自检（task_31 Step 1，非组件；23 项断言）
│       ├── check_bounds.gd       # 空气墙 / 顶盖自检（task_32 Step 2，非组件；12 项断言）
│       ├── check_models.gd       # 低模外观 + 传感器模型接入自检（Step 2 / task_33 Step 3；24 项断言）
│       ├── check_sensor_models.gd # 传感器 / 底盘模型资产自检（task_33 Step 1，非组件；29 项断言）
│       ├── check_camera_sensor.gd # 相机组件自检（task_33 Step 2，非组件；A 组 12 + B 组 9 = 21 项断言）
│       ├── check_camera_shots.gd # 模拟世界拍照自检（task_33 Step 4，非组件；S 组 18 + R 组 17 = 35 项断言）
│       ├── check_person_assets.gd # [已移除 2026-10-28] 静态人物自检（历史）
│       └── doubles/              # 测试替身（**不是组件**、不落 .tscn）
│           ├── TestChassis.gd    #   底盘鸭子契约：Configure / GetDefaultParams / SetCommand / Step
│           └── TestSensor.gd     #   传感器鸭子契约：Configure / Sample / ScanOnce
└── assets/                       # 只放真实资源（png / glb / obj / material / wav …）
    ├── .gitkeep                  #   空目录占位
    ├── README.md                 #   目录边界说明：代码与场景一律 src/
    ├── uav_quadrotor_lowpoly.glb #   四旋翼整机低模（0.64 × 0.14 × 0.64；原点 = 底面中心）
    ├── ugv_x3plus_lowpoly.glb    #   旧 UGV 外观（保留不删，回退用；task_33 Step 3 起 `ModelUGV`
    │                             #     改用 `chassis/ugv_chassis.glb`）
    ├── sensors/                  #   [task_33 Step 1] 传感器模型（纯视觉，无碰撞体）；
    │                             #     Step 3 起由 `Lidar2D.tscn` / `CameraSensor.tscn` 的 `Body` 静态挂载
    │   ├── lidar_tmini.glb       #     2D 激光雷达：⌀0.080 × 高 0.054；原点 = 扫描面中心
    │   └── camera_module.glb     #     相机模组：0.055 × 0.036 × 0.030；原点 = 镜片前表面中心
    ├── chassis/                  #   [task_33 Step 1] 底盘模型
    │   └── ugv_chassis.glb       #     UGV 外观：0.30 × 0.25 × 0.25；原点 = 底面中心、车头 +x
    └── humans/                   #   [human_model Step 4] 人形资产（**只放最终选用 + 1 个备选**）
        ├── README.md             #     来源 URL / 许可 / 处理方式（单位·缩放·朝向·贴图·三角数）/ 摆位 / 换备选方法
        ├── LICENSE_renderpeople.txt  # Renderpeople 免费页条款原文（883 B；与 tmp 侧原件逐字节相同）
        ├── rp_carla_rigged_001/  #     **首选**：model.glb（1.7307 m / 19 988 tri）+ 2 张贴图 + 4 个 .import
        └── rp_eric_rigged_001/   #     **备选**：model.glb（1.8400 m / 20 542 tri）+ 2 张贴图 + 4 个 .import
```

> 模型源文件（`.blend`）与构建 / 导出脚本**不在工程内**：`Tool/blender/models/`
> （`build_models.py` + `export_all.sh`，工程根下的同级目录）；`assets/` 只放导出后的 `.glb`。
> 建模口径与验收证据见 `tmp/task33/impl.md`。

`src/tests/doubles/` 只被 `src/tests/check_vehicle.gd` 用 `PackedScene.pack()` 现场打包，不进 §3 的 10 个 `.tscn` 清单。

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

# 相机组件（task_33 Step 2 / task_34 Step 3）：① 纯 headless 只跑 A 组（结构 / 参数 / 出帧）18 项，
#    其中出帧部分（A13~A18）只验流绑定 / 帧头 / 频率 / 无渲染器闸门 / 目录登记，不碰像素；B 组（真渲染）10 项自动 [skip]
timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_camera_sensor.gd
# ② **真验收（出图）**：本机 `--headless` 是 dummy 渲染器，物理上出不了图（§9.1 E26-③）；
#    用 tmp/task33/xvfb 里解包的 Xvfb（**:99**）+ llvmpipe 软件 GL 跑 A+B 共 28 项，**必须**带 --audio-driver Dummy
#    （无声卡时 ALSA 会打一行 ERROR，把"0 ERROR"验收搞假红）。
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot --display-driver x11 --rendering-driver opengl3 \
  --audio-driver Dummy --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_camera_sensor.gd

# camera 装配线（task_34 Step 3）：`check_ws_manager` 现在覆盖 `sensor="camera"` 的整套装配
# （J1~J6：sensor_ready -> 几何 -> SetStream -> zero-collision -> assembly_ready -> 出帧闸门；
#  帧内容 J7 在 headless 下 [skip]）+ 行人装配线（task_35 Step 2：A11 / K1~K16），
# 76 项断言（headless 档；Xvfb 档 77 项 —— J7 有渲染器才计。
# 原 51 项 + A9/A10 + J1~J6 + A11 + K1~K13，第 2 轮再加 K14~K16）。
# **两档都要 `SELF-CHECK OK`**（第 2 轮起 Xvfb 档是硬门槛，不是"可选"）：
timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_ws_manager.gd
DISPLAY=:99 timeout 400 godot --path orion-simulation-world --fixed-fps 60 --rendering-driver opengl3 \
  --audio-driver Dummy --script res://src/tests/check_ws_manager.gd

# 模拟世界拍照（task_33 Step 4，§4.8.2）：① 纯 headless 只跑 S 组（结构）18 项，R 组 17 项自动 [skip]
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_camera_shots.gd
# ② **真验收（出图）**：同一脚本换真渲染档，拍 3 张 1280×720 JPEG 到 tmp/task33_shots/，S+R 共 35 项。
#    **必须**带 --audio-driver Dummy（理由同上：无声卡时 ALSA 会打一行 ERROR）；
#    跑前先起虚拟显示：bash tmp/task33/xvfb/start_xvfb.sh（已在跑则直接返回）。
#    脚本自己会关掉 World.tscn 自带的 WebSocketManager.autolisten（**不监听 9999、不连 WS**）。
bash tmp/task33/xvfb/start_xvfb.sh
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot --display-driver x11 --rendering-driver opengl3 \
  --audio-driver Dummy --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_camera_shots.gd

# 世界观感（task_38 Step 1b，§4.1.1）：① headless 结构档 **30 项**（R 组 13 项 `[skip]` 且不计入 total）
#    跑前先 `--import`（E28 同款：缺 `.godot` 缓存时贴图 / HDRI 的 `load()` 会静默 null → 材质降级）
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
  --script res://src/tests/check_world_look.gd
#    ② **真验收（出图）**：Xvfb 档 **43 项**（30 + R 组 13），拍 3 张 1600×900 JPEG 到 tmp/task38/shots/
#    + 2 张 160×90 太阳方向探针；**绝不加 `--headless`**、**必须 `--audio-driver Dummy`**
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 400 godot --display-driver x11 \
  --rendering-driver opengl3 --audio-driver Dummy --path orion-simulation-world \
  --fixed-fps 60 --script res://src/tests/check_world_look.gd

# 街道 + 立面（task_38 Step 2，§4.1.2）：① headless 结构档 **28 项**（R 组 13 项 `[skip]` 且不计入 total）
#    跑前先 `--import`（同 E28：缺 `.godot` 缓存时沥青/路缘贴图 load() 静默 null → 材质降级成纯色）
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
  --script res://src/tests/check_street.gd
#    ② **真验收（出图）**：Xvfb 档 **41 项**（28 + R 组 13），拍 3 张 1600×900 街景到 tmp/task38/shots/
#    （street_00_junction / street_01_building_side / street_02_uav_top）；**必须 `--audio-driver Dummy`**
xvfb-run -a -s "-screen 0 1600x900x24" env LIBGL_ALWAYS_SOFTWARE=1 timeout 400 godot \
  --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
  --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_street.gd

# 静态人物（human_model Step 4，§4.9）：新增自检 32 项；跑前**必须先** `--import`
# （E28：`.godot/` 缺失时 glb / Person.tscn 的 load() 会静默返回 null → 世界 0 人）
timeout 300 godot --headless --path orion-simulation-world --import
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_person_assets.gd

# 走路人物（walking_person Step 3，§4.10）：① 结构档 20 项（headless）；② 渲染档 26 项（三帧 JPEG 落
# tmp/walking_person/shots/，**绝不能加 --headless**）。走满一圈要 2096 个物理帧，渲染档因此只走半圈（见 §4.10-⑥）。
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_person_walk.gd
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 300 godot --display-driver x11 --rendering-driver opengl3 \
  --audio-driver Dummy --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_person_walk.gd

# 走路人物三帧「真推进」（walking_person Step 4，§4.10-⑦）：① 渲染档 14 项（**绝不能加 --headless**，
# 帧间不瞬移：用 await physics_frame 真推进 0/40/80 帧，Δs/Δframes 必须 == v/60）；② headless 冒烟只跑 A 组 4 项。
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot --display-driver x11 --rendering-driver opengl3 \
  --audio-driver Dummy --path orion-simulation-world --fixed-fps 60 \
  --script res://src/tests/check_person_walk_shots.gd
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
  --script res://src/tests/check_person_walk_shots.gd

# 行人端到端验收（task_35 Step 5，§4.11.2）——两张档位、两种参数，别混：
# ① 第 4 + 5 项（人撞车 / 雷达可见）：**自检脚本**一律 `--fixed-fps 60`（靠 await physics_frame 自己数帧）
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
  --script res://src/tests/check_pedestrian_physics.gd
# ② 第 6 项（走 / 转 / 停三帧）：
#    headless 冒烟必须"假绿为零"（10 条 [skip] + total=4，R 组不计入 total）
timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 \
  --script res://src/tests/check_pedestrian_shots.gd
#    真验收（出图到 tmp/task35/shots/）：**绝不加 --headless**、**必须 --audio-driver Dummy**
DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot \
  --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy \
  --path orion-simulation-world --fixed-fps 60 \
  --script res://src/tests/check_pedestrian_shots.gd
# ③ 第 1~3 项（Python 墙钟驱动直行 / 转弯 / 超时）：一把梭起**实时档**世界再跑客户端。
#    ★ 世界**绝对不能**带 `--fixed-fps`（实测 21~37 倍速，10 s 墙钟 = 360 s 世界时间，人会撞空气墙）；
#      实时档只有 `--max-fps 60`（实测 ratio 1.00、pose 10.2 Hz）。脚本自带端口独占检查与日志摘要。
bash tmp/task35/s5_e2e.sh
# ④ 全量回归（21 项 headless + 5 项 Xvfb + --quit 冒烟，逐条与硬编码基线比对）
bash tmp/task35/s5_checks.sh && cat tmp/task35/logs/s5_checks_summary.txt

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
│                                            ※ task_38 Step 1b：`Environment` / `Sky` 由
│                                              `WorldLook.Apply_All()` **运行时复制一份**再替换
│                                              （`PanoramaSkyMaterial` + `assets/hdri/sky_midday_2k.hdr`），
│                                              `background_mode=BG_SKY` / `ambient_light_source=SKY` 原样带走；
│                                              地面材质 = `Ground/MeshInstance3D` 的 surface override、
│                                              太阳 = `Sun` 的 transform / energy，**`.tscn` 零改动**（§4.1.1）
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
├── Persons (Node3D)                         **运行时**静态人物容器（无碰撞、无物理）      [human_model Step 4]
│   ├── person_01 (Person.tscn 实例)        世界 (60, 76, 0)、yaw 0；`Body` = rp_carla 的 glb   [human_model Step 4]
│   └── person_02 (Person.tscn 实例)        世界 (68, 70, 0)、yaw 0；同上                          [human_model Step 4]
│                                            ※ 由 `World.gd::BuildPersons()` **运行时**建并摆放，
│                                              **不在 World.tscn 里**（与 `Bounds` 同策略，§9.1 E16 / §4.9）
├── Pedestrians (Node3D)                     **运行时**外部驱动的行人容器（**有胶囊碰撞**、无 AI）  [task_35 Step 3]
│   ├── pedestrian_01 (Pedestrian.tscn 实例) 世界 (44, 96, 0)、yaw 0（朝 +X 东）；**idle**：谁也没驱动时站住不动
│   └── pedestrian_02 (Pedestrian.tscn 实例) 世界 (52, 102, 0)、yaw 180°（朝 −X 西）；同上
│                                            根脚本 = `src/chassis/Pedestrian.gd`（`extends ChassisModel`）：
│                                            `CharacterBody3D` + `CapsuleShape3D` r0.25 / h1.75 / 中心 y0.875、
│                                            `collision_layer=2` / `collision_mask=5 (1|4)`；见 §4.11
│                                            ※ **同一个容器**既装上面这几个**世界预置（idle）**行人，也装
│                                              `WebSocketManager` 动态装配的 `chassis_hello{kind:"pedestrian"}`
│                                              行人（`Pedestrians/<peer_id>`）；容器名逐字 = `PEDESTRIANS_NODE`
│                                            ※ 由 `World.gd::BuildPedestrians()` 在 `BuildPersons()` **之后**
│                                              运行时建，**不在 World.tscn 里**（与 `Bounds`/`Persons` 同策略，§4.11）
├── Street (Node3D)                          **运行时**街道容器（纯渲染、**零碰撞**）        [task_38 Step 2]
│   ├── Road (MeshInstance3D)                沥青路面：主路 (120×8) + 支路 (8×24)，顶面 z=0.01
│   ├── Markings (MeshInstance3D)            标线 28 个四边形：主虚线 18 + 边线 2 + 支虚线 4 + 斑马线 4，z=0.03
│   └── Curbs (MeshInstance3D)               路缘 7 段 × 5 面（高 0.15 / 宽 0.2，斑马线与路口处断开）
│                                            ※ 3 个 mesh 全部 `cast_shadow=OFF`、`material_override=null`、
│                                              `transform=identity`（顶点直接烘成 Godot 坐标）；
│                                              由 `World.gd::_ready()` 末尾调 `StreetLayout.Build_All(self)` 建，
│                                              **不在 World.tscn 里**（§4.1.2）
├── Facade (Node3D)                          **运行时**建筑立面容器（纯渲染、**零碰撞**）    [task_38 Step 2]
│   ├── Windows (MeshInstance3D)             窗 60 个四边形（bay 2.4 / 窗 1.8×1.6 / 首层窗台 0.9 / 层高 3.2）
│   ├── Plinths (MeshInstance3D)             基座 44 个四边形（11 栋 × 4 面，z∈[0,0.6]，混凝土 × 0.7）
│   └── Caps (MeshInstance3D)                屋顶压顶 15 个四边形（外扩 0.05，路缘贴图 × 0.6）
│                                            ※ 按 `World.OBSTACLES` **只读**生成（确定性、无 RNG）；
│                                              由 `Facade.Build_All(self)` 建，**不在 World.tscn 里**（§4.1.2）
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
| `WorldEnvironment` | `WorldEnvironment` | — | 静态（**环境资源运行时复制替换**） | `Environment` 内联（`ProceduralSkyMaterial` + `ambient_light_source=SKY`）；task_38 Step 1b 起 `WorldLook.Apply_All()` 把 `Environment` + `Sky` **各复制一份**后换成 `PanoramaSkyMaterial`（§4.1.1），`background_mode` / `ambient_light_source` 不变 |
| `Sun` | `DirectionalLight3D` | — | 静态（**位姿/能量运行时覆盖**） | `.tscn` 里 `rotation_degrees=(-55,-35,0)`、`shadow_enabled=true`；运行时由 `WorldLook.Apply_All()` 改成与 HDRI 太阳方向一致（`SUN_DIR`）+ `light_energy=0.7`，`shadow_enabled` 不动 |
| `Ground` | `StaticBody3D` | — | 静态 | **一整块**地面，非体素地面（D10；避免 6.5 万碰撞体） |
| `Ground/CollisionShape3D` | `CollisionShape3D` | — | 静态 | `BoxShape3D(128,1,128)`；`position=(64,-0.5,64)` → 顶面 `y=0` |
| `Ground/MeshInstance3D` | `MeshInstance3D` | — | 静态（**材质运行时换**） | `BoxMesh` 同尺寸；`.tscn` 里是深灰单色 `StandardMaterial3D`，运行时被 `set_surface_override_material(0, WorldLook.Make_Ground_Material())` 换成混凝土铺装（三贴图 PBR + 世界空间 triplanar 6 m）；缺资产时保留原单色 |
| `VoxelWorld` | `GridMap` | — | 静态节点 / 运行时填格 | `cell_center_*=true`；`mesh_library` 运行时构建，其立方体材质由 `WorldLook.Make_Voxel_Material()` 给（混凝土 + 世界空间 triplanar 2 m；缺资产回落到 `VOXEL_COLOR` 单色，§4.1.1） |
| `Bounds` | `StaticBody3D` | —（`World.gd` 内建） | **运行时** | 不可见硬边界；`collision_layer=1`、`collision_mask=0`；5 个 `CollisionShape3D`（4 面 32 m 空气墙 + 32 m 顶盖）；**内侧面贴世界边界平面**（`x=0/128`、`y=0/128`、`z=32`），厚度一律向外（§9.1 E24）；无任何 mesh 子节点 |
| `Vehicles` | `Node3D` | — | 静态容器 | 所有 avatar 的父节点；子节点名 = `peer_id`（净化后） |
| `Persons` | `Node3D` | —（`World.gd` 内建） | **运行时** | 静态人物容器（human_model Step 4）；纯视觉、**无碰撞体**、无物理；幂等重建（`BuildPersons()`）；子节点名 = `PERSONS` 常量表的 `name` |
| `Persons/person_0x` | `Node3D` | `src/props/Person.gd` | **运行时**实例 | 1 个 `Person.tscn` 实例；子节点 `Body` = `assets/humans/rp_carla_rigged_001/model.glb` 的静态实例（glb 根 `Node3D` → 1 个 `MeshInstance3D`）；世界坐标见 §4.9 |
| `Pedestrians` | `Node3D` | —（`World.gd` 内建） | **运行时** | 外部驱动的行人容器（task_35 Step 3）；**世界预置 2 人 + manager 动态装配的人共用一个容器**（名字逐字 = `WebSocketManager.PEDESTRIANS_NODE`）；幂等重建（`BuildPedestrians()`；运行期重建会连带移除已装配的网络行人，见 §4.11）；子节点名 = `PEDESTRIANS` 常量表的 `name`（预置）/ `<peer_id>`（动态） |
| `Pedestrians/pedestrian_0x` | `CharacterBody3D` | `src/chassis/Pedestrian.gd` | **运行时**实例 | 1 个 `Pedestrian.tscn` 实例：**有胶囊碰撞**（r0.25 / h1.75 / 中心 y0.875）、`layer 2` / `mask 1\|4`、`motion_mode=GROUNDED`；子节点 `CollisionShape3D` + `Body` = `assets/humans/rp_nathan_animated_003_walk/model.glb`（glb 根 `Node3D` → `Armature` → `Skeleton3D`(88 骨) + `MeshInstance3D` + `AnimationPlayer`）；出生点 / 朝向 / idle 语义 / 与协议行人的分工见 §4.11 |
| `Street` | `Node3D` | —（`src/world/StreetLayout.gd` 内建） | **运行时** | 街道容器（task_38 Step 2）；纯渲染、**无任何 `CollisionObject3D` / `CollisionShape3D`**；幂等重建（`StreetLayout.Build_All()` 先摘同名旧容器）；3 个子节点见下三行 |
| `Street/{Road,Markings,Curbs}` | `MeshInstance3D` | — | **运行时** | 各 1 个 `ArrayMesh`（SurfaceTool 合并，每四边形独立 4 顶点 + 6 索引）：`Road` 2 面 / `Markings` 28 面 / `Curbs` 35 面；`cast_shadow=OFF`、`material_override=null`（材质挂在 surface 上）、`transform=identity`；材质 = `asphalt` 4 m / 纯参数白 / `curb` 1.5 m 世界空间 triplanar（§4.1.2） |
| `Facade` | `Node3D` | —（`src/world/Facade.gd` 内建） | **运行时** | 建筑立面容器（task_38 Step 2）；纯渲染、**零碰撞**；按 `OBSTACLES` 只读生成，幂等重建；3 个子节点见下 |
| `Facade/{Windows,Plinths,Caps}` | `MeshInstance3D` | — | **运行时** | 60 / 44 / 15 个四边形合并成 3 个 mesh；`Windows` = 深色玻璃（`cull_mode=CULL_DISABLED`）、`Plinths` = `concrete` 2 m × 0.7、`Caps` = `curb` 1.5 m × 0.6（§4.1.2） |
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
| human_model-4 | **不改 `World.tscn`**：`World.gd::BuildPersons()` 在运行时把 `Persons`（`Node3D` + 2 个 `Person.tscn` 实例）挂到 `World` 下；`PERSONS` 常量表给固定世界坐标 |
| walking_person-2 | **不改 `World.tscn`**：`World.gd::BuildWalkers()` 在 `_ready()` 里 `BuildPersons()` **之后**把 `Walkers`（`Node3D` + 1 个 `PersonWalking.tscn` 实例）挂到 `World` 下；`WALKERS` 常量表给闭合路径 / 速度 / 相位（`PERSONS` 一字不动） |
| walking_person-3 | **不改 `World.tscn`**：走动能力并入 `Person.gd`（`PersonWalking.gd` 删除），`PersonWalking.tscn` 的根脚本改指 `Person.gd`、`Body` 改指 `_walk` 资产；`WALKERS` 的键名统一为 `walk_speed` / `loop`（`speed_mps` 仅作 `Configure()` 别名）；`_ready()` 顺序（`BuildPersons()` → `BuildWalkers()`）不变 |
| walking_person-4 | `src/tests/check_person_walk_shots.gd`（**新增**，14 项：A 组结构 4 + R 组渲染 10）+ 同名 `.gd.uid`（`--import` 生成）、`COMPONENTS.md`（L8 状态 / §3 脚本表 / §4.10-⑤ 与 **⑦** / §11 / 附） | 三帧**真推进**证明「人在走」（支线，**不进装配线**）：在环路第一段侧前方（世界 `(48,101)`+1.5 m 瞄 `(44.8,96,0.92)`）架**固定**机位，用 `await physics_frame` **逐帧**推进 0/40/80 帧拍 `tmp/walking_person/shots/walk_t{0,1,2}.jpg`（`27 862 / 28 368 / 28 541 B`，**帧间不瞬移**）。`DISPLAY=:99` **`total=14 failed=0`** + `SELF-CHECK OK` + `EXIT=0` + 0 ERROR；`s = 0.000/0.840/1.680 m`、`frames = 0/40/80`、`Δs/Δframes = 0.021000 = v/60`（偏差 0.00%）；投影框 `Δx = 57.0 / 68.1 / 125.1 px`（判据 30）；框内/框外差分 `1020× / 422× / 566×`（判据 3×）；64×64 对齐裁剪差 `0.06871 / 0.06780 / 0.08183`（判据 0.010）；机位三次变换逐位相同（偏差 0.000000）；headless 冒烟 `[skip]` ×10 + **`total=4 failed=0`**；**16 个既有自检断言数与基线逐字一致**（`project` 无 total / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `ws_manager 59` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 18` / `person_assets 32` / `person_walk 20`（headless）/ `person_shots 7`（headless），全部 `failed=0`、0 ERROR；渲染档 `person_shots 22` / `person_walk 26` 同样逐字不变）；`--import` EXIT=0 且 0 ERROR（生成 `.gd.uid`）；三张旧图 `walker_t{0,1,2}.jpg` 的 `md5` 未被改变。**只做「走动」**：不加碰撞、不改雷达 / 建图 / 决策链、**不做 YOLO**；**不改** `Person.gd` / `World.gd` / `Person*.tscn` / `project.godot` / `assets/**` / 既有自检；**不碰** `pleiades-*` / `Task/` / `Workbook/` / `programs/` / 根 Cargo；**不执行** `git add` / `git commit` |
| task_33-3 | **不改 `World.tscn`**：三件 `.glb` 各自在**组件场景**里 `instance` —— `Vehicle.tscn` 的 `ModelUGV` 换 `chassis/ugv_chassis.glb`，`sensors/Lidar2D.tscn` / `sensors/CameraSensor.tscn` 各加一个 `Body`（§4.3 / §4.7 / §4.8） |
| task_35-3 | **不改 `World.tscn`**：`World.gd` 的 `WALKERS` / `BuildWalkers()` / `GetWalkersRoot()` **整节删除**，换成 `PEDESTRIANS` / `BuildPedestrians()` / `GetPedestriansRoot()` —— 容器改名 `Walkers` → `Pedestrians`（`pedestrian_01` / `pedestrian_02`），**世界不再实例化 `PersonWalking.tscn`、不再内置任何路径 / 巡逻 / 计时**；人在无外部驱动时**站着不动**（idle = 预期，§4.11） |

| task_38-2 | **不改 `World.tscn`**：`World.gd::_ready()` 在 `BuildPedestrians()` 之后追加 `StreetLayout.Build_All(self)` / `Facade.Build_All(self)`，把 `Street`（3 个 `MeshInstance3D`）与 `Facade`（3 个）挂到 `World` 下；新增 `Get_Street_Root()` / `Get_Facade_Root()` 与常量 `STREET_NODE` / `FACADE_NODE`；`OBSTACLES` / `PEDESTRIANS` / 世界尺度 / 四个 layer **一字不动**（§4.1.2） |

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

固定 **11 个**（前 7 个与 task_30 §八 完全一致；第 8 个由 task_33 Step 2 增补、task_34 Step 3 起进入
`sensor_hello` 装配线，见 §4.8 / §4.8.3；第 9 个由支线 `human_model` Step 4 增补，**不进装配线**，见 §4.9；
第 10 个由支线 `walking_person` Step 2 增补，**不进装配线、零碰撞、自带走路动画**，见 §4.10；
第 11 个由 task_35 Step 1 增补：**行人底盘**——登记在 `ChassisCatalog` 里、有胶囊碰撞、由外部脚本经
`motion_cmd{v,dir}` 驱动，但它**不是车**（不进 `Vehicles`、不占 avatar 排位），见 §4.11）：

| # | 文件（`res://`，相对工程根） | 根节点类型 | 脚本 | 职责 | Step | 静态挂载位置 | 运行时实例化者 |
|---|---|---|---|---|---|---|---|
| 1 | `src/scenes/World.tscn` | `Node3D` | `src/world/World.gd` | 根场景：地面 / 体素 / 环境 / 容器 | 2 | 主场景（`run/main_scene`） | 引擎 |
| 2 | `src/scenes/ObserverCamera.tscn` | `Camera3D` | `src/camera/ObserverCamera.gd` | 观察者相机（自由 / 环绕 / 跟随） | 3 | `World/ObserverCamera` | 静态实例 |
| 3 | `src/scenes/Vehicle.tscn` | `CharacterBody3D` | `src/vehicle/Vehicle.gd` | avatar 身体：物理 + 装配接口；`ModelUGV = assets/chassis/ugv_chassis.glb`（模型 = 碰撞盒，D11） | 4 ✅ | — | `WebSocketManager` → `World/Vehicles` |
| 4 | `src/scenes/chassis/DiffDrive.tscn` | `Node3D` | `src/chassis/DiffDrive.gd` | 差速底盘（`GROUNDED`） | 5 ✅ | — | `Vehicle.ApplyChassis()` → `Vehicle/Chassis` |
| 5 | `src/scenes/WebSocketManager.tscn` | `Node` | `src/net/WebSocketManager.gd` | WS 服务端：装配 + 收发 | 6 ✅ | `World/Systems/WebSocketManager` | 静态实例 |
| 6 | `src/scenes/chassis/Quadrotor.tscn` | `Node3D` | `src/chassis/Quadrotor.gd` | 四旋翼底盘（`FLOATING`） | 7 ✅ | — | `Vehicle.ApplyChassis()` → `Vehicle/Chassis` |
| 7 | `src/scenes/sensors/Lidar2D.tscn` | `Node3D` | `src/sensors/Lidar2D.gd` | 2D 单线激光（真射线）+ `Body` 外观实例（`lidar_tmini.glb`，纯视觉、无碰撞） | 8 ✅ | — | `Vehicle.AttachSensor()` → `Vehicle/Sensors/<slot>` |
| 8 | `src/scenes/sensors/CameraSensor.tscn` | `Node3D` | `src/sensors/CameraSensor.gd` | 相机：`SubViewport` 离屏渲染 → JPEG + 按 `rate_hz` 组 `type=2` 帧下发（task_34 Step 3）+ `Body` 外观实例（`camera_module.glb`，纯视觉、无碰撞） | task_33-2 ✅ / task_34-3 ✅ | — | `sensor_hello` 装配线（`SensorCatalog` 登记 `camera`）→ `Vehicle/Sensors/<slot>`（task_34-3 起）；自检仍直接实例化 |
| 9 | `src/scenes/props/Person.tscn` | `Node3D` | `src/props/Person.gd` | 静态人物道具：纯视觉、**零碰撞**、无行为、不进装配线 | human_model-4 ✅ | `World/Persons/person_0x`（**运行时**） | — （不由 manager 实例化） |
| 10 | `src/scenes/props/PersonWalking.tscn` | `Node3D` | `src/props/Person.gd` | 走路人物道具：纯视觉、**零碰撞**、**带骨骼 + 走路动画**，自带闭合路径走动能力（`walk_speed` / `path` / `loop`）、不进装配线（**与第 9 个共用同一个脚本**，只换 `Body`） | walking_person-3 ✅ | **无**（task_35 Step 3 起世界**不再**实例化它：自动绕圈已移除；资产与走动能力保留给 §4.10 的人形自检，`Body` 与第 11 个共用同一 glb） | — （不由 manager 实例化） |
| 11 | `src/scenes/props/Pedestrian.tscn` | `CharacterBody3D` | `src/chassis/Pedestrian.gd` | 行人底盘（**特殊底盘**，`GROUNDED`）：`CapsuleShape3D` r0.25 / h1.75 / 中心 y0.875，`layer 2` / `mask 1\|4`，`Body` = 走路 glb（含 `AnimationPlayer`）；**自宿主**推进（`_physics_process` → `move_and_slide()`），`speed_scale = v / 1.26`；外部脚本经 `motion_cmd{v,dir}` 驱动 | task_35-1 / task_35-3 | `World.BuildPedestrians()` → `World/Pedestrians/pedestrian_0x`（**世界预置、idle**，§4.11）；`WebSocketManager` → `World/Pedestrians/<peer_id>`（动态、同一容器） | `WebSocketManager`（Step 2 起，`kind="pedestrian"` 直接实例化，**不经** `Vehicle.ApplyChassis`） |

非组件脚本（无 `.tscn`）：

| 文件 | 类型 | 职责 |
|---|---|---|
| `src/world/World.gd` | 挂 `World` | 世界时钟启动、体素/地面程序化生成、尺度日志、静态人物摆位（`BuildPersons()`，§4.9） |
| `src/world/WorldLook.gd` | 静态工具（`RefCounted`，全 `static`） | **观感唯一入口**（task_38 Step 1b，§4.1.1）：三贴图 PBR 材质工厂（`Make_Ground_Material` / `Make_Voxel_Material` / `Make_Fallback_Voxel_Material`）+ `PanoramaSkyMaterial`（`Make_Sky_Material`）+ 太阳对齐与能量（`Apply_All` / `Get_Missing_Assets` / `Reset_Cache`）；世界空间 triplanar 参数集中在此；**缺资产只 `push_warning` 并降级，绝不 `push_error`** |
| `src/world/StreetLayout.gd` / `Facade.gd` | 静态工厂（`RefCounted`，全 `static`） | 街道 / 立面唯一入口（task_38 Step 2，§4.1.2）：`Build_All(world)` / `Get_Root(world)`；`SurfaceTool` 合并成 3 + 3 个 `MeshInstance3D`，**零碰撞、不写 shader**；`Facade` 只经 `get_script_constant_map()` 读 `World.OBSTACLES`（不 preload `World.gd`，避免循环依赖） |
| `src/common/WorldTransform.gd` | 静态工具 | **坐标转换唯一入口**（§7.3） |
| `src/common/WorldClock.gd` | 静态工具 | 统一 `world_time`（单调墙钟） |
| `src/vehicle/Vehicle.gd` | 挂 `Vehicle` | 装配、物理步进、位姿查询（§4.3） |
| `src/chassis/ChassisModel.gd` | 底盘基类 | 底盘接口契约 + 速度环 / 指令解析公共工具（§7.5） |
| `src/chassis/ChassisCatalog.gd` | 静态目录 | `chassis` 名 → 场景路径 / 默认参数（运行时 `load()`；Step 5 ✅） |
| `src/chassis/DiffDrive.gd` / `Quadrotor.gd` | 底盘实现 | 运动学 + 速度环（`DiffDrive` ✅ Step 5 / `Quadrotor` ✅ Step 7） |
| `src/chassis/Pedestrian.gd` | 挂 `Pedestrian` | **行人底盘实现 + 自宿主推进**（task_35-1）：`v`/`dir` 解析、180 °/s 限速转向、`move_and_slide()` 唯一位置更新点、`GetPose()` / `Play_Walk()`（§4.11） |
| `src/sensors/SensorCatalog.gd` | 静态目录 | `sensor` 名 → 场景路径 / 默认参数（运行时 `load()`；Step 8 ✅，task_34-3 加 `camera`） |
| `src/sensors/Lidar2D.gd` | 挂 `Lidar2D` | 射线扫描 + 二进制下发（Step 8 ✅） |
| `src/sensors/CameraSensor.gd` | 挂 `CameraSensor` | 离屏渲染 + `Configure` + 按 `rate_hz` 出 `type=2` 帧（`SetStream` / `Emit_Frame`，task_34-3 ✅） |
| `src/net/WebSocketManager.gd` | 挂 manager | 监听 / accept / 两张表 / 装配 / manifest / 清理（Step 6 ✅） |
| `src/net/ComponentLink.gd` | `RefCounted` | 单条连接封装与状态（Step 6 ✅） |
| `src/net/WireProtocol.gd` | 静态工具 | 消息名、JSON 编解码、二进制帧构造 / 解析（Step 6 ✅） |
| `src/tests/check_project.gd` | `SceneTree` 脚本 | 工程骨架自检（Step 1） |
| `src/tests/check_vehicle.gd` | `SceneTree` 脚本 | Vehicle 自检（Step 4，40 项断言） |
| `src/tests/check_models.gd` | `SceneTree` 脚本 | 低模外观 + 传感器模型接入自检（Step 2 / task_33 Step 3，24 项断言） |
| `src/tests/check_world.gd` | `SceneTree` 脚本 | 世界自检（Step 2） |
| `src/tests/check_camera.gd` | `SceneTree` 脚本 | 相机自检（Step 3） |
| `src/tests/check_diff_drive.gd` | `SceneTree` 脚本 | 差速底盘自检（Step 5，41 项断言） |
| `src/tests/check_quadrotor.gd` | `SceneTree` 脚本 | 四旋翼底盘自检（Step 7，48 项断言） |
| `src/tests/check_ws_manager.gd` | `SceneTree` 脚本 | WebSocketManager 自检（Step 6，**76 项断言**（headless）/ 77（Xvfb）：51 + task_34-3 的 `A9/A10` + `J1~J6` + task_35-2 的 `A11` + `K1~K16`；真 World + 真 WS 客户端） |
| `src/tests/check_lidar.gd` | `SceneTree` 脚本 | Lidar2D 自检（Step 8，48 项断言；真 World + 自建 box 障碍） |
| `src/tests/check_bounds.gd` | `SceneTree` 脚本 | 空气墙 / 顶盖自检（task_32 Step 2，12 项断言） |
| `src/props/Person.gd` | 挂 `Person` / `PersonWalking` | **唯一**人物脚本（walking_person Step 3 起）：静态常量（`MODEL_GLB` / `HEIGHT_MIN_M` / `HEIGHT_MAX_M`）+ 走动扩展（`WALK_GLB` / `CLIP_MPS` / 导出字段 `walk_speed` / `path` / `loop`）+ 闭合路径采样 / 摆位 + 动画播放（`find_child("AnimationPlayer", true, false)`、播第 0 条、`speed_scale = walk_speed / CLIP_MPS`）；**默认静态**（不给参数 = 站着不动）、**无碰撞、不进装配线**（§4.9 / §4.10） |
| `src/tests/check_person_assets.gd` | `SceneTree` 脚本 | 静态人物自检（human_model Step 4，32 项断言：资产 16 + 场景 5 + 常量 2 + 摆位 9） |
| `src/tests/check_person_walk.gd` | `SceneTree` 脚本 | 走路人物自检（walking_person Step 3，**26 项断言**：资产 8 + 场景 4 + 世界 3 + 运行 5 + 渲染 6；headless 下渲染组 6 项 `[skip]` 且不计入 total → `total=20`） |
| `src/tests/check_person_walk_shots.gd` | `SceneTree` 脚本 | 走路人物三帧**真渲染**自检（walking_person Step 4，**14 项断言**：A 组结构 4 + R 组渲染 10；`--fixed-fps 60` 下真推进 0/40/80 物理帧拍 `walk_t{0,1,2}.jpg`，**帧间不瞬移**；headless 下 R 组 10 项 `[skip]` 且不计入 total → `total=4`） |
| `src/tests/check_pedestrian_physics.gd` | `SceneTree` 脚本 | 行人**端到端物理**自检（task_35-5，**22 项断言**：S 组结构 7 + P 组人推车 8 + L 组雷达可见 7；全 headless、无 `[skip]`）。`--fixed-fps 60` 下推 300 个物理帧：逐帧 `gap = 胶囊轴到车身盒水平距离 − 0.25` 取 min（不穿透）、人**被挡停**（最后 60 帧前进 < 5 mm）、**车位移 0.000000000 m**（人推不动车）；雷达装**车头**（`x=0.15`、`z=0.6`）扫 `Lidar2D.ScanOnce()`，±45° 扇区最近 `2.2500 m` = 2.50 − 0.25，把人挪到 10 m 外变 `9.7500 m`（证明命中的是这个人） |
| `src/tests/check_pedestrian_shots.gd` | `SceneTree` 脚本 | 行人**走 / 转 / 停三帧真渲染**自检（task_35-5，**14 项断言**：A 组结构 4 + R 组渲染 10；headless 下 R 组 10 项 `[skip]` 且不计入 total → `total=4`）。`--fixed-fps 60` 自己数帧推进 40 / 45 / 20 / 12 / 15 帧（**帧间不瞬移**），驱动只发 **3 条** `Pedestrian.SetCommand`；断言 = 文件体积 / JPEG 魔数 / SOF0 1280×720 / 非全黑 / 三张互不相同 / 框水平中心相距 ≥ 20 px（实测 74.2 / 141.6 / 67.5）/ 转向段速率 180.0 °/s 且此帧 `yaw=90.00°` / 停帧冻结 15 帧 `Δpose=0` 且 `speed_scale=0` / 抓拍簿记 / 源码 lint（0 连网、0 写位姿、`SetCommand` 恰 3 发） |
| `src/tests/check_pedestrian.gd` | `SceneTree` 脚本 | 行人底盘自检（task_35-1，**40 项断言**：场景形态 7 + 目录装配 6 + `SetCommand`→`Step` 控制量 11 + 老协议 `action/speed` 回退 5 + 真 World 自宿主推进与动画 8 + 白盒 3；前 4 组全在树外调 `Step`，无渲染依赖） |

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
func BuildPersons() -> void                  # [human_model Step 4] 按 `PERSONS` 常量表实例化 `Person.tscn` 并摆位；幂等
func GetPersonsRoot() -> Node3D              # [human_model Step 4] `World/Persons`（见 §4.9）
func BuildPedestrians() -> void              # [task_35 Step 3] 按 `PEDESTRIANS` 常量表实例化 `Pedestrian.tscn`，入树摆位后 `Configure(self, {})`；**不内置任何行为**；幂等
func GetPedestriansRoot() -> Node3D          # [task_35 Step 3] `World/Pedestrians`（见 §4.11）
```

**三个容器（`Bounds` / `Persons` / `Pedestrians`）与 `Bounds` 同策略**：**由代码建、不写进 `World.tscn`**
（避开 §9.1 E16 的 `node_paths` 坑，也让 `check_bounds.gd` 的 `B1d`（读 `.tscn` 文本数节点）保持 0 命中）；
三者都**幂等**（重复调用先摘旧容器再重建）。`Pedestrians` 额外有一条约束：它与 `WebSocketManager` 的
`kind="pedestrian"` 动态装配**共用同一个容器名**，运行期重建会连带移除已装配的网络行人（§4.11）。
缺资产 / 缺场景时只 `push_warning`、**绝不 `push_error`**（否则 `--quit` 的 0 ERROR 验收假红）。

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

#### 4.1.1 运行时观感（task_38 Step 1b ✅）

世界观感（地面材质 / 体素材质 / 天空 / 太阳）**全部运行时建**，唯一入口是 `src/world/WorldLook.gd`；
`src/scenes/World.tscn` **零改动**（`git diff` 为空）。改的只是 3 个 `Material` + 1 个 `Sky` + `Sun` 的
`transform` / `light_energy`：**不建节点、不动几何、不动 `OBSTACLES`、不动碰撞层**。

`World.gd::_ready()` 的新顺序（`Apply_World_Look()` 必须在 `BuildVoxelWorld()` **之后**、
`_Build_Bounds()` **之前**）：

```gdscript
func _ready() -> void:
	WorldClock.EnsureStarted()
	_read_scale_settings()
	_log_scale()
	BuildVoxelWorld()          # 内部 _make_mesh_library() 已通过 WorldLook 拿到体素材质
	if LOOK_ENABLED:           # const LOOK_ENABLED := true（观感总开关）
		Apply_World_Look()     # 地面材质 + 天空 + 太阳；返回 false = 至少一样降级
	_Build_Bounds()
	BuildPedestrians()
	StreetLayout.Build_All(self)   # task_38 Step 2：路面 + 标线 + 路缘（3 个 MeshInstance3D，零碰撞）
	Facade.Build_All(self)         # task_38 Step 2：窗 + 基座 + 压顶（3 个 MeshInstance3D，零碰撞）
```

> 上面两行在 **`BuildPedestrians()` 之后**：街道 / 立面只读 `OBSTACLES`、不依赖任何容器，
> 顺序排在最后的好处是"任何更早的步骤失败也不会留下脏几何"（§4.1.2）。

**贴图 → 槽位（全部 1a 已入库，本步一个字节都没动）**

| 用途 | 文件（`res://assets/…`） | 尺寸 | 槽位 |
|---|---|---|---|
| 地面 albedo / normal / ORM | `textures/ground_paving_{color.jpg,nor.jpg,orm.png}` | 2048² / 2048² / 512² | `albedo_texture` / `normal_texture` / `orm_texture`（+ 三通道显式） |
| 体素 albedo / normal / ORM | `textures/concrete_{color.jpg,nor.jpg,orm.png}` | 1024² / 1024² / 512² | 同上 |
| 天空 | `hdri/sky_midday_2k.hdr` | 2048×1024 | `PanoramaSkyMaterial.panorama` |

`textures/asphalt_*` / `curb_*` **Step 2 起已引用**：沥青路面 4 m 一循环（`ASPHALT_TILE_M`）、
路缘石 1.5 m 一循环（`CURB_TILE_M`），另作压顶/基座的 `tint` 变体；接线细节与材质表见 §4.1.2。

**唯一来源常量（`WorldLook`，别在别处再写一遍）**

| 常量 | 值 | 说明 |
|---|---|---|
| `GROUND_TILE_M` | `6.0` | 地面 6 m 一循环 ⇒ `uv1_scale = (1/6, 1/6, 1/6)`（实测 `0.166667`） |
| `VOXEL_TILE_M` | `2.0` | 体素 2 m 一循环 ⇒ `uv1_scale = (0.5, 0.5, 0.5)` |
| `SUN_ENERGY` | `0.7` | `Sun.light_energy`；`shadow_enabled` 不动 |
| `SKY_ENERGY` | `0.5` | `PanoramaSkyMaterial.energy_multiplier`（亮度标定见下） |
| `SUN_DIR` | `Vector3(-0.4313, 0.6817, 0.5910)` | **Godot 空间**、从场景指向太阳（`\|v\| = 1.000008`） |

**`SUN_DIR` 是怎么来的（实测，不是估的）**：`sky_midday_2k.hdr` 最亮像素 `(1229, 267)`、峰值亮度
`110947`；按等距柱面反解得 `elev = 42.98°`、`|azim| = 36.12°`。水平方位**取镜像那一侧**：
按 `+azim` 落成 `(0.4313, 0.6817, 0.5910)` 时，10° 探针对准它的 `mean = 0.3083`、对准反方向
`mean = 0.8642`（太阳在反方向）⇒ 翻 `x`。翻后实测：对准 `SUN_DIR` 的探针 `mean = 0.9499`、
对准「同仰角反方位」的对照探针 `mean = 0.0471`（差 `0.9029`）——「天上只有一个太阳、且就是我们
对齐的那个方向」由此可判。

`Sun` 的对齐写法：`basis.z == +SUN_DIR`（`DirectionalLight3D` 沿局部 `-Z` 发光），
即 `Transform3D(Basis.looking_at(-SUN_DIR, up), sun.global_position)`；实测
`basis.z = (-0.431297, 0.681695, 0.590995)`、与 `SUN_DIR` 的 `dot = 1.000000`。

**亮度标定（为什么有 `SKY_ENERGY`）**：HDRI 直出（`energy_multiplier = 1.0`）会过曝 —— 实测
`check_camera_shots` 的 `R3-shot-01/02-not-blank` 近白像素占比 **0.523 / 0.531 > `WHITE_MAX(0.50)`**
（该脚本阈值**不许改**，task_38 §二.5-6）。压到 `SKY_ENERGY = 0.5` 后：`0.000 / 0.197 / 0.139`，
且地面纹理细节重新可见（`R4-shot-*-grid` 的 `contrast` 由基线 `42~56` 升到 `115~138`）。
这一个旋钮同时压「背景天空」与「来自天空的环境光」（ambient 由 sky 的 radiance 立方图产生），
`background_mode` / `ambient_light_source` 都不动。逐次实测值见 `tmp/task38/impl.md`「亮度调参」。

**降级契约（缺资产只 warn，绝不 `push_error`）**

- `_Load_Tex()` 先 `ResourceLoader.exists()` 再 `load()`（未 import 时 `load()` 会打 ERROR，E28）：
  缺任一张 ⇒ `_Make_Pbr()` 返回 `null` ⇒ 地面保留 `.tscn` 的原单色材质、体素回落到
  `Make_Fallback_Voxel_Material(VOXEL_COLOR)`（`albedo_color = 0.55/0.58/0.62`、`roughness = 0.85`，
  与 task_37 逐值相同）、天空保留 `ProceduralSkyMaterial`。
- 缺失清单进 `WorldLook.Get_Missing_Assets()`；`_Warn_Missing_Once()` 只 `push_warning` **一次**。
- `World.Apply_World_Look()` 返回 `false` 时打一行 `[world] look degraded ...`，**不中断世界构建**。

**自检（`src/tests/check_world_look.gd`）**：**S 组 30 项**（headless 也全跑）+ **R 组 13 项**
（仅真渲染器；`Can_Capture() == false` 时打印 13 条 `[skip]` 且**不计入 total**）。
`headless → total=30 failed=0`，`Xvfb → total=43 failed=0`。R 组落盘 3 张 **1600×900** JPEG 到
`tmp/task38/shots/`（`look_00_ground_closeup` / `look_01_building_side` / `look_02_uav_top`，
实测 `175 615` / `187 989` / `269 011 B`）+ 2 张 160×90 探针（`probe_sun` / `probe_anti`）。

**G6 人眼复核的三条实测口径（供后续改观感时对照）**

1. 地面：铺装缝**每 1.5 m** 一条（`ground_paving_color.jpg` 是 `4×4` 块 / 图 ⇒ `4 × 1.5 = 6 m` 正好一个
   triplanar 循环）；拿同一张 UAV 图里的 `building_a`（10 m 宽）当尺子复核，缝距 / 房宽 ≈ `1.5 / 10`。
2. 体素：墙面主周期 **1.0 m**（= 2 m 图幅里的模板缝，列灰度自相关 `lag 161 px → 0.585`），
   0.5 m 处只有 `0.208`（纹理自身的半格浅痕）⇒ **没有**「每格 0.5 m 一张图」的碎块感
   （若 `uv1_world_triplanar` 失效，主周期会塌到 0.5 m 且强度拉满）。
3. 单太阳：天空亮斑方向 `dot(SUN_DIR) → 探针 0.9499`，反方位 `0.0471`；物体阴影一律背向该方向。

---

#### 4.1.2 街道与立面（task_38 Step 2 ✅）

**为什么是代码建**：`World.tscn` 必须 `git diff` 为空（§三.5），所以街道与立面全部在 `_ready()` 末尾由
`World.gd` 调两个静态工厂建出来，与 `Bounds` / `Persons` / `Pedestrians` 同策略。**全部纯渲染、零碰撞**：
`Street` / `Facade` 子树里 `CollisionObject3D` = `CollisionShape3D` = 0，且 `MeshInstance3D` 根本没有
`collision_layer` 属性（`get("collision_layer") == null`，§9.1 E31-①）。

```
Street (Node3D，运行时建，identity)                                     [task_38 Step 2]
├── Road     (MeshInstance3D)  1 surface /  2 个四边形（主路 → 支路，顺序固定）
├── Markings (MeshInstance3D)  1 surface / 28 个四边形
└── Curbs    (MeshInstance3D)  1 surface / 35 个四边形（7 段 × 5 面，**无底面**）

Facade (Node3D，运行时建，identity)                                     [task_38 Step 2]
├── Windows (MeshInstance3D)  1 surface / 60 个四边形
├── Plinths (MeshInstance3D)  1 surface / 44 个四边形（11 栋非边界障碍 × 4 面）
└── Caps    (MeshInstance3D)  1 surface / 15 个四边形（15 个障碍各 1 个）
```

三个 mesh 一律 `cast_shadow = OFF`、`material_override == null`（材质挂在 surface 上）、`transform == IDENTITY`
（顶点直接烘成 Godot 坐标）。每四边形 **4 顶点 + 6 索引**、不焊接、不与别的四边形共享（§9.1 E31-②）——
自检因此能用 `verts.size() == 4 × quads` 从 mesh 反推函数，不读任何自报数字。

**路网常量（`StreetLayout`，世界坐标 Z-up / 米）**

| 元素 | 范围 | z | 判据（§三.6） |
|---|---|---|---|
| 主路（东西） | `x∈[4,124]`、`y∈[60,68]` | 顶面 **0.01** | AABB `(120, 8)`、`z = 0.01 ± 0.005` ✅ |
| 支路（南北） | `x∈[44,52]`、`y∈[68,92]` | 顶面 **0.01** | AABB `(8, 24)` ✅（北端止于 `wall_c`，T 型路口） |
| 主路中心虚线 | `y = 64±0.075`、`x∈[4,124]`，3 m 线 + 3 m 空 | 0.03 | **18 段**（∈ 20±2）✅；跳过 `x∈[44,52]`（路口）与 `x∈[56,60]`（斑马线） |
| 主路边线 | `y = 60.30` / `67.70`，`120 × 0.15` 实线 | 0.03 | **2 条** ✅ |
| 支路中心虚线 | `x = 48±0.075`、`y∈[68,92]`，3 m 线 + 3 m 空 | 0.03 | **4 段**（∈ 4±1）✅ |
| 斑马线 | `x∈[56,60]` 跨 `y∈[60,68]`，白条 0.5 + 间隔 0.5 | 0.03 | **4 条** `0.5 × 8.0`，起点 `56/57/58/59` ✅ |
| 标线合计 | — | — | **28 个四边形**，全部落在两条路面矩形内（无一落到地面）✅ |

主路虚线实测起点：`4,10,16,22,28,34,40,52,64,70,76,82,88,94,100,106,112,118`（`46/58` 两段被跳过）。

**"路面 2 + 标线 1 = 3" 怎么解的**：§三.6-3 的"路面 2 + 标线 1"指**2 块路面几何**（主路 + 支路）与
**1 块标线**。本实现把两块路面合进**同一个** `Road` mesh（2 个四边形、顺序固定：主路 → 支路），
于是 `Street` 下 `MeshInstance3D` 总数 = **3**，同时满足"路面 2 + 标线 1 = 3"与"总数 ≤ 3"两条；
路缘单列第 3 个节点，仍不超上限。

**路缘常量（7 段 × 5 面 = 35 个四边形，高 0.15 / 宽 0.2）**

| 段 | x 范围 | y 范围 | 断开依据 |
|---|---|---|---|
| south_west / south_east | `4~56` / `60~124` | `60~60.2` | 斑马线 `x∈[56,60]`（降坡口，**只断开、不做坡道**） |
| north_west / north_mid / north_east | `4~44` / `52~56` / `60~124` | `67.8~68` | 支路口 `x∈[44,52]` 与斑马线 `x∈[56,60]` |
| branch_west / branch_east | `44~44.2` / `51.8~52` | `68~92` | 与主路北缘在 `(44,68)` / `(52,68)` 成 L 角 |

面 = 顶面（`z = 0.15`）+ 4 个侧面（`z∈[0,0.15]`），**不出底面**：底面与地面 `z=0` 共面会闪
（`curb-profile` 断言"朝下的四边形 = 0"）。**没有坡道几何**——§三.5 只要求"断开"。

**立面常量（`Facade`）**

| 元素 | 几何 | 数量 | 判据 |
|---|---|---|---|
| 窗 | `bay 2.4`、窗 `1.8 × 1.6`、首层窗台 `0.9`、层高 `3.2`，墙外偏移 **0.01** | **60** | `window-total` + 逐栋 `window-per-obstacle` + 逐面 `window-four-walls` ✅ |
| 基座 | 每栋 4 面竖片，`z∈[0,0.6]`，外扩 `0.01`、沿面两端各多伸 `0.01`（包角） | **44**（11 栋） | `plinth`（距墙面正好 0.01 且朝外）✅ |
| 压顶 | 每栋顶面水平片，外扩 `0.05`，`z = h + 0.01 + 0.001 × 障碍下标` | **15**（含 4 面边界矮墙） | `cap`（15 个 z 两两不同 ⇒ 相邻压顶不共面）✅ |

逐栋窗数（实现与自检各自独立复算，结果必须一致）：

| 障碍 | size (x,y,h) | 层数 | ±x 面窗/面 | ±y 面窗/面 | 窗数 |
|---|---|---|---|---|---|
| wall_south / north / west / east | — / 2 | —（跳过） | — | — | **0**（只做压顶） |
| building_a | 10 × 8 × 4 | 1 | 3 | 4 | **14** |
| building_b | 8 × 10 × 5 | 1 | 4 | 3 | **14** |
| wall_c | 30 × 1 × 2 | 0 | 0 | 12（不开） | **0** |
| wall_d | 1 × 24 × 3 | 1 | 10 | 0（1 m 端面放不下） | **20** |
| pillar_e | 2 × 2 × 8 | 2 | 1 | 1 | **8** |
| step_f1 / step_f2 | 8×8×1 / 6×6×2 | 0 / 0 | — | — | **0 / 0** |
| step_f3 | 4 × 4 × 3 | 1 | 1 | 1 | **4** |
| block_1 / 2 / 3 | 1 × 1 × 1 | 0 | — | — | **0** |
| **合计** | — | — | — | — | **60** |

**层数为什么加"一档下限"**：§三.3 的字面公式 `floor((h − 0.9) / 3.2)` 会让 `h = 4` 的 `building_a` 算出
**0 层** ⇒ 全表只剩 `building_b` / `pillar_e` 有窗，直接违反 §三.6-4「窗数量 > 0、每栋四壁都有窗」。
`Floors_Of()` 因此在字面公式后加一条下限：`h ≥ SILL + WIN_H = 2.5 m` 却不足一整层时按 1 层算。
实测层数 `4→1 / 5→1 / 8→2 / 3→1 / 2→0 / 1→0`；**该下限只影响 4 m 与 3 m 两档，不影响 5 m / 8 m**。
`check_street` 的 `window-four-walls` 用同一公式**独立复算**，不做自证。

**深度分层（防 z-fighting）**：地面 `z=0` / 路面 `z=0.01` / 标线 `z=0.03`。`near = 0.05` 的 D24 相机在
64 m 处深度精度 ≈ 4.9 mm ⇒ 路面对地面有 2 倍余量，标线对路面 4 倍余量（1.7 m 眼高下 0.17° 视差不可见）。
路面 `z` 被 §三.6-1 钉死 `0.01`，**要再狠只能抬标线**。

**三处已知的"叠在一起但不闪"**：

1. **边线 × 斑马线**：边线在 `x∈[56,60]` 与白条共面重叠（`markings-skip` 实测 8 处）。同材质、同色、
   同 `z` ⇒ 像素输出完全相同，**不是 z-fighting**（z-fighting 需要"两个不同的结果在抢同一深度"）。
2. **嵌套退台 `step_f1~f3`**：三者实心互相嵌套，`step_f3` 的 4 扇窗在 `z∈[0.9,2.0]` 段被 `step_f2` 的实体
   包住（只有 `2.0~2.5` 露出，视觉上像小高窗），`step_f1/f2` 的压顶环在对方体积内被遮（正是要的退台阶梯感）。
   **不为嵌套做特例**（否则要引入遮挡计算）；计数仍按全量算（60 / 15）。
3. **窗/基座薄片 vs 墙体**：薄片只沿法线外扩 `0.01 m`，不与墙面共面；体素墙是 0.5 m 一格的 `GridMap`，
   墙面格心在 `x = 24.25` 这类位置，薄片平面在 `x = 23.99`，**相距 1 cm 以上**。

**贴图接线（0 字节增量：`assets/` 仍是 `24 306 445 B`）**

| 用途 | 材质工厂 | 循环 / 变体 |
|---|---|---|
| 路面 | `WorldLook.Make_Pbr_Material("asphalt", ASPHALT_TILE_M)` | 4 m（1K 图 ⇒ 256 px/m） |
| 路缘 | `WorldLook.Make_Pbr_Material("curb", CURB_TILE_M)` | 1.5 m |
| 基座 | `WorldLook.Make_Pbr_Material("concrete", VOXEL_TILE_M, 0.7)` | 2 m（与体素墙同源，接缝连续） |
| 压顶 | `WorldLook.Make_Pbr_Material("curb", CURB_TILE_M, 0.6)` | 1.5 m |
| 窗 | 纯参数 `StandardMaterial3D`（albedo 0.07 / roughness 0.15 / metallic 0.3 / `CULL_DISABLED`） | — |
| 标线 | 纯参数白（`Color(0.85,0.85,0.83)`、roughness 0.55、无 metallic） | — |

> 薄片**没有 UV** ⇒ 材质只能经 `WorldLook.Make_Pbr_Material`（内部 `uv1_triplanar + uv1_world_triplanar`），
> 否则整面只采一个 texel（看起来像纯色，自检也抓不到；§9.1 E31）。

**自检：`src/tests/check_street.gd`（S 组 28 + R 组 13）**

`headless → total=28 failed=0`（R 组 13 条 `[skip]` 且不计入 total），`Xvfb → total=41 failed=0`。
R 组落盘 3 张 **1600×900** JPEG 到 `tmp/task38/shots/`：`street_00_junction`（站在支路上看 T 型路口，
实测 `261 789 B`）/ `street_01_building_side`（`building_a` 西南两面 3/4 视角，`172 797 B`）/
`street_02_uav_top`（20 m 俯视路口，`285 920 B`）。R 组三条实测口径：

- `windows-dark`：屏内且 ≥ 12 px 的窗 **11** 个，其中平均亮度 < 0.25 的占 **0.91**、聚合均值 **0.1055**
  （唯一偏亮的那扇 `0.277` 是镜面打到太阳方向的高光，物理正确）。
- `window-clean`：99 个采样点里「偏离本窗均值 > 0.12」的占 **0.0000** ⇒ 窗/墙交界没有互穿斑驳。
- `crosswalk-contrast`：白条均值 **0.9379** − 间隔均值 **0.4727** = **0.4652**（判据 ≥ 0.10，4.7 倍余量）
  ⇒ 斑马线真的画出来且与沥青分得开。

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
                                                                  "chassis_root", "sensors_root",
                                                                  "model_ugv", "model_uav")
├── Body (MeshInstance3D)            position=(0, 0.125, 0)；占位 BoxMesh(0.30, 0.25, 0.25) + 橙色材质
│   └── Nose (MeshInstance3D)        position=(0.15, 0, 0)；BoxMesh(0.06, 0.10, 0.14) 红 → 朝向标记（车头 = 局部 +X）
├── CollisionShape3D                 position=(0, 0.125, 0)；占位 BoxShape3D(0.30, 0.25, 0.25)（**唯一碰撞体**）
├── Chassis (Node3D)                 ← 底盘挂载点（运行时**至多一个**子节点）
├── Sensors (Node3D)                 ← 传感器挂载点（运行时子节点名 = slot）
├── ModelUGV (Node3D)                instance res://assets/chassis/ugv_chassis.glb；visible=false（task_33 Step 3 起）
│                                    ↑ 子树 9 个节点：Hull / NosePlate / Cabin / Roof + WheelFL/FR/RL/RR
└── ModelUAV (Node3D)                instance res://assets/uav_quadrotor_lowpoly.glb；visible=false
```

**原点约定（写死）**：`Vehicle` 原点 = 车体**底面中心**；`Body` 与 `CollisionShape3D` 都上抬 `height/2`
（占位 `height=0.25` → `0.125`，task_33 Step 3 起；抬升只由 `SetBodySize()` 做，底盘绝不自己写 `body.position.y`）。世界 `z=0` ↔ Godot `y=0`，所以 UGV 出生 `y=0` 就站在地面上。
`BoxMesh` 默认**居中**，因此 Body 必须自己上抬，**不要**把 Body 留在原点（否则半个车身埋进地面，
射线也会从地下发出）。`Chassis` / `Sensors` 初始**没有子节点**——空车 = 只有壳与两个挂载点。

**模型 = 碰撞盒（D11，task_33 Step 3）**：

| 项 | 值 | 依据 |
|---|---|---|
| `ModelUGV` 场景 | `res://assets/chassis/ugv_chassis.glb`（旧 `ugv_x3plus_lowpoly.glb` **保留不删**，回退用） | task_33 Step 1 建模 |
| 模型 AABB（Vehicle 局部系） | `P(-0.15, 0.00, -0.125)` `S(0.30, 0.25, 0.25)` | `check_models` 的 `M3-ugv-aabb` / `M3b-ugv-model-equals-hull` |
| 碰撞盒 | `BoxShape3D(0.30, 0.25, 0.25)`，`CollisionShape3D.position = (0, 0.125, 0)` | `DiffDrive` 默认 `wheel_base 0.30 / height 0.25 / track_width 0.25` → `ApplyBodySize()` |
| 世界 `y` 区间 | 模型与碰撞盒**都是** `y ∈ [0, 0.25]`（原点 = 底面中心） | 同上 |

> 逐轴相等是**硬要求**：外观层与物理层不允许两套尺寸。有模型时占位 `Body`/`Nose` 与底盘占位轮子
> 一起 `visible=false`（`Vehicle._Mount_Model()`），但 `CollisionShape3D` **始终有效**（显隐不影响物理）。
> 车体截面从 `0.16/0.24` 变成 `0.25/0.25` 是**有意的行为变更**：贴墙余量 / 绕行余量 / 急停余量都会变
> （task_33 §四-6）——本 step 只保证自检绿，goto 冒烟属另一条线。

**手写 `.tscn` 的两条硬约束**（§9.1 E8 / E16）：

- **6 个** `@export` 节点引用必须在根节点行写
  `node_paths=PackedStringArray("body", "collision_shape", "chassis_root", "sensors_root", "model_ugv", "model_uav")`，
  并逐个 `body = NodePath("Body")` ……；**漏写 `node_paths` 会静默为 null**（不报错、不告警）。
  `Vehicle._ensure_refs()` 只按子节点名兜底一次，自检 `check_vehicle` 的 A5 / `check_models` 的 `M1-exports-bound` 直接断言"绑定成功"。
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
@export var model_ugv: Node3D = null      # ModelUGV：assets/chassis/ugv_chassis.glb 实例（task_33 Step 3 起）
@export var model_uav: Node3D = null      # ModelUAV：uav_quadrotor_lowpoly.glb 实例

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
        params: Dictionary = {}) -> Node           # mount = {x, y, z, yaw, pitch?}（pitch 度，缺省 0）
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
| `mount` 外参 | `{x, y, z, yaw, pitch?}` 是**车体局部坐标**、轴向沿用**世界语义**（`x` 前 / `y` 右 / `z` 高 / `yaw` 绕 z / `pitch` 绕车体横轴），换算只经 `WorldTransform.PosToGodot()` / `YawToGodot()`。例：`{x:0.1, z:0.2}` = 车头前方 0.1 m、上方 0.2 m |
| `mount.pitch`（task_33 Step 3，**可选**） | **单位 = 度**（与 `yaw` 同口径，内部 `deg_to_rad`）；缺省 `0`，**不预置任何角度**；语义 **正 = 抬头、负 = 低头**。锁死依据：光轴 = 传感器局部 `−Z`，`rotation.x = pitch`（欧拉序 YXZ，`Ry·Rx`），实测 `pitch=-30°` → 光轴 `-basis.z = (-0.866, -0.5, 0)`、`pitch=+15°` → `y=+0.2588`。`pitch` **不会**成为挂载失败原因（非法值 `float()` 回退 0.0） |
| `slot` | `strip_edges().validate_node_name()` 归一化，**键名 = 节点名**（`GetSensor` / `GetStreamSlots` / `DetachSensor` 与 `get_node("Sensors/<slot>")` 完全一致）；同 slot 重挂 = 先卸后挂，同帧完成 |
| 采样 | `Sample()` 优先、`ScanOnce()` 次之、都没有给 `null`（鸭子类型；Step 8 的 `Lidar2D.ScanOnce()` 天然可用，本接口无需为它改一行） |

> **`mount` 的基准点 = 机体底面中心**（不是几何中心，2026-10-24 实测校正）：`ApplyBodySize` 把 BoxShape3D 的原点放在底面，`Body` 网格再抬 `body/2`（UGV 0.125、UAV 0.06）。实测几何（`assets/` 里的 glb）：UGV 机腹 `z=0` / 中线 `0.125` / 车头前缘 `x=+0.15` / 总尺寸 0.30×0.25×0.25；UAV 机腹 `z=0` / 中线 `0.06` / 机头前缘 `x=+0.20` / 机身 0.40×0.12×0.40、旋翼展 0.64、总高 0.14。**写 `z=-0.02` 是"机腹以下 2 cm"（会悬空），不是"中线以下"**；UAV 相机取"贴机头、略低于中线" = `x=0.20, z=0.045`。

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
- 默认参数：`{max_speed: 1.5, max_turn_rate: 90.0, wheel_base: 0.30, track_width: 0.25, height: 0.25}`；碰撞形状 `BoxShape3D(wheel_base, height, track_width)`（Godot **X=前**、Z=横；与 `src/scenes/Vehicle.tscn` 占位形状 `(0.30, 0.25, 0.25)` 一致）。**task_33 Step 3 起 5 个几何默认值 = `ugv_chassis.glb` 的 AABB**（D11：模型 = 碰撞盒）。
- `Configure()` 设 `motion_mode = CharacterBody3D.MOTION_MODE_GROUNDED`，`floor_snap_length` 用默认。

由 `Vehicle.ApplyChassis("diff_drive_ugv", params)` 实例化并挂在 `Vehicle/Chassis`；换底盘先 `queue_free()` 旧节点。

**落地要点（Step 5 实现，已由 `src/tests/check_diff_drive.gd` 41 项断言覆盖）**：

- `Configure(vehicle, params)` 的顺序固定：`super.Configure` 记宿主/参数 → 参数夹到合法域 → **速度环状态清零**（换底盘不继承旧速度）→ `ApplyMotionMode(MOTION_MODE_GROUNDED)` → `ApplyBodySize(Vector3(wheel_base, height, track_width))`（**只传尺寸**，`mesh/shape` 与"原点=底面中心"的抬升由 `Vehicle.SetBodySize()` 负责，底盘绝不自己写 `body.position.y`）→ `_layout_wheels()`（`x = ±wheel_base/2` 前 + 后 −、`z = ±track_width/2` 右 + 左 −（默认 `±0.125`）、`y = 0.06` 让轮底贴 `y=0`；子节点缺失只 `push_warning` 一次）。场景占位轮位与默认参数保持一致（`x=±0.15 / z=±0.125`），运行期 `Configure()` 再按生效参数重摆。
- `SetCommand()` 只改**目标值**（`_speed_target` / `_yaw_target`），实际速度由 `Step()` 的速度环斜坡逼近；`up` / `down` / 未知 `action` / 缺 `action` 一律 = `stop`。`left` / `right` 线速度目标为 0（原地转）是设计，不是漏写。
- `Step()` 的返回值 `velocity` 是 **Godot 全局速度**（`global_basis * Vector3(speed, 0, 0)`，`vel.y` 强制 0），`yaw_rate` 是 `rotation.y` 的角速度（正 = 左转）；树外调用（自检）走 `is_inside_tree()` 兜底，不访问 `global_*`。
- 轮子 `CylinderMesh` 的属性是 `top_radius` / `bottom_radius` / `height` / `radial_segments`（**没有** `radius`）；`rotation_degrees=(90,0,0)` 把圆柱轴从 Y 转到 Z（横轴）。轮子纯视觉、不参与物理：碰撞形状只有 `Vehicle/CollisionShape3D` 一个。
- 撞墙判据是**位置**：`move_and_slide()` 撞墙后自己把 `velocity` 归零（不是底盘逻辑），所以"会停"= 车头不穿透 + 位置停止推进 + 能倒车脱离。实测 world x 停在 `127.3375`（车头 `127.4875`，墙面 `127.5`）。
- 该停位**与车体截面无关**：task_33 Step 3 用 4 组尺寸（`0.30×0.16×0.24` / `0.30×0.25×0.24` / `0.30×0.16×0.25` / `0.30×0.25×0.25`）实测**同为** `127.337524`——撞墙是 **x 向面接触**，x 向尺寸没变（仍 `0.30`）。所以 `check_diff_drive` 的 `WANT_WALL_X = 127.35 ± 0.05` **不动**。
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
signal pedestrian_spawned(peer_id: String, pedestrian: Node3D)   # task_35 Step 2
signal pedestrian_removed(peer_id: String)                       # task_35 Step 2
signal link_closed(peer_id: String, role: StringName, slot: String)

@export var vehicle_scene: PackedScene      # World.tscn 里指向 res://src/scenes/Vehicle.tscn
@export var vehicles_root: Node3D           # World.tscn 里指向 ../../Vehicles（相对自身！§9.1 E20）
@export var pedestrians_root: Node3D        # task_35 Step 2：**World.tscn 不绑**，留空则惰性解析/建
@export var listen_port: int = 0            # 0 → 读 sim_world/network/ws_port（9999）
@export var autolisten: bool = true         # false = 不自动监听（自检用）
@export var handshake_timeout_ms: int = 5000   # 连上后多久没 hello 就关
@export var pending_timeout_ms: int = 10000    # 传感器先到、底盘一直不来 -> 关
@export var stats_log: bool = false         # true 时 Heartbeat 每秒打一行 GetStats()

# 两张表（§5.1）+ 装配簿记
var _peer_vehicles: Dictionary = {}         # 表 1：peer_id -> Vehicle
var _pedestrians: Dictionary = {}           # 表 1b：peer_id -> Pedestrian（task_35 Step 2，与表 1 互不相交）
var _ped_last_motion_ms: Dictionary = {}    # 表 1b 簿记：peer_id -> 最近一条 motion_cmd 的墙钟毫秒
var _ped_idle: Dictionary = {}              # 表 1b 簿记：peer_id -> bool（已经下过 v=0）
var _links: Array = []                      # 表 2 的可枚举侧：Array[ComponentLink]
var _pending_sensors: Dictionary = {}       # peer_id -> Array[{link, hello, since_ms}]
var _manifests: Dictionary = {}             # peer_id -> {slot: bool}
var _manifest_parts: Dictionary = {}        # peer_id -> {slot: sensor}
var _assembly_sent: Dictionary = {}         # peer_id -> bool（每 peer 只发一次）
var _next_avatar_id: int = 1                # u16 语义（**只给车**）
var _next_pedestrian_id: int = 1            # u16 语义（**只给人**；与 _next_avatar_id 完全独立）
var _next_sensor_id: int = 1                # u16 语义，>65535 回绕到 1
var _next_conn_id: int = 1                  # 连接号（日志 / 自检）
var _tcp_server: TCPServer
var _listen_port: int = 0                   # 实际监听端口（0 = 未监听）
var _pose_acc: float = 0.0                  # 位姿下行累加器（秒），与物理帧解耦（task_31 Step 1）

# 位姿下行周期（task_31 Step 1）：对齐真机 state_notifier 的 100 ms，硬编码、不做配置项
const POSE_BROADCAST_INTERVAL_S := 0.1

# 行人（task_35 Step 2）
const PEDESTRIAN_CHASSIS := "pedestrian"                 # kind=pedestrian 的 chassis 名
const PEDESTRIANS_NODE := "Pedestrians"                  # 容器名（与 Vehicles 平级、同父）
const PEDESTRIAN_SPAWN_OFFSET_M := Vector3(0.0, -8.0, 0.0)   # 默认点 = SPAWN_WORLD + 本偏移 = (64, 56)
const PEDESTRIAN_SPAWN_MIN_GAP_M := 1.0                  # 与已占 avatar 的最小水平中心距
const PEDESTRIAN_SPAWN_MARGIN_M := 0.3                   # 静态障碍外扩余量
const PEDESTRIAN_SPAWN_MAX_Z_M := 2.0                    # 高度上限（不做"空中出生"）
const PEDESTRIAN_CMD_TIMEOUT_MS := 300                   # 指令超时 -> 自动 v=0（只停、不断连）

# 生命周期 / 监听
func _ready() -> void                        # _EnsureRefs() + 连 Heartbeat.timeout + autolisten 则 Listen()
func _exit_tree() -> void                    # Stop()
func _process(delta: float) -> void          # _AcceptConnections() → _PollLinks() → _SweepClosed()
                                             #   → _Tick_Pedestrian_Timeouts() → _BroadcastPoses(delta)
func Listen(port: int) -> Error              # port<=0 = OS 临时端口；幂等（先 Stop()）
func Stop() -> void                          # 关全部连接 + 清全部表 + 移除全部 avatar（车 + 人）+ tcp.stop()
func GetListenPort() -> int
func IsListening() -> bool
func GetVehicle(peer_id: String) -> Node3D
func GetVehicleCount() -> int
func GetPedestrian(peer_id: String) -> Node3D    # task_35 Step 2（不存在 / 已释放 = null）
func GetPedestrianCount() -> int
func IsPedestrian(peer_id: String) -> bool
func GetAvatar(peer_id: String) -> Node3D         # = GetVehicle() 或 GetPedestrian()
func GetPedestriansRoot() -> Node3D               # 惰性解析 / 建；没有 Vehicles 容器时 null
func GetLinkInfos() -> Array                 # 表 2 只读快照 [{conn_id, peer_id, role, slot, sensor_id, open, handshaken}]
func GetStats() -> Dictionary                # {listening, port, links, vehicles, pedestrians, pending, assembled}
# 私有
func _AcceptConnections() -> void
func _PollLinks() -> void
func _HandlePacket(link, bytes: PackedByteArray, was_string: bool) -> void
func _ApplyMotionCommand(link, msg: Dictionary) -> void
func _Apply_Pedestrian_Command(link, ped: Node3D, msg: Dictionary) -> void
func _Tick_Pedestrian_Timeouts() -> void                  # 300 ms 无新指令 -> v=0（只对行人生效）
func _BroadcastPoses(delta: float) -> void               # 位姿下行（task_31 Step 1；唯一的发送者）
func _RegisterChassis(link, msg: Dictionary) -> void      # 按 kind 分流：robot -> 车，pedestrian -> 人
func _Register_Pedestrian(link, msg: Dictionary, peer_id: String) -> void
func _RegisterSensor(link, msg: Dictionary) -> void
func _MountSensor(link, hello: Dictionary) -> bool
func _ReplayPendingSensors(peer_id: String) -> void
func _TryAssembly(peer_id: String) -> void
func _RemoveAvatar(peer_id: String, except_link = null) -> bool   # 车 / 人共用（两张表一起摘；返回是否真摘到）
func _RemoveVehicle(peer_id: String, except_link = null) -> void  # 兼容壳 -> _RemoveAvatar()
func _CleanupLink(link) -> void                           # 所有权守卫：只有 link.vehicle != null 才摘 avatar
func _CloseLink(link, code: int, reason: String) -> void
func _SweepClosed() -> void
func _OnHeartbeatTimeout() -> void
func _PickSpawn(chassis_name: String) -> Dictionary      # {x, y, z, yaw}（**只给车**）
func _Pedestrians_Root() -> Node3D                        # 惰性解析 / 建 Pedestrians（每次复检有效性）
func _Pick_Pedestrian_Spawn(msg: Dictionary, peer_id: String) -> Dictionary
func _Pedestrian_Default_Spawn() -> Dictionary
func _Spawn_In_Obstacle(p: Vector3) -> bool
func _Spawn_Occupied(p: Vector3) -> bool
func _World_Span_M() -> float
func _All_Avatars() -> Array
func _HasAvatar(peer_id: String) -> bool
func _Merge_Avatar_Params(avatar: Node, params: Dictionary) -> Dictionary
func _To_Float(value: Variant, fallback: float) -> float
func _Kind_Of(msg: Dictionary, peer_id: String = "") -> String
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

**⑤ 连接的角色是单值**：已握手的**传感器连接**发来 `chassis_hello` 属于协议错误 → `push_warning` + **忽略**（不改角色、不建车、不关别人的链路）。"重复 `chassis_hello` 重建 avatar"（§8.3-7）只在**该连接本身是底盘连接**时生效，且注册前无条件调用 `_RemoveAvatar(peer_id, except_link=本连接)`，绝不把新连接关掉。**跨 kind 一样**：机器人换行人、行人换机器人都在**两张表 + 两个容器**里一起清（`_peer_vehicles`+`Vehicles` 与 `_pedestrians`+`Pedestrians`），不会留下幽灵节点（第 2 轮，`prev_issues` #2）。

**⑥ 监听与端口**：`TCPServer.listen(port)` 绑定全地址；失败（如 9999 被占）**只 `push_warning`**，世界继续跑（不可接入）。`listen_port = 0` 时读 `[sim_world] network/ws_port`；自检用 `Listen(0)` 取 OS 临时端口，**不抢 9999**。

**⑦ 日志指纹（供 grep 验收）**：`[ws] listening on port …` / `accept conn=…` / `chassis_hello peer=…` / `vehicle spawned peer=…` / `sensor pending …` / `sensor_ready peer=…` / `assembly_ready peer=…` / `link closed conn=…` / `vehicle removed peer=…` / `stopped (was port …)`。只在事件处打印，逐帧无输出。

**⑧ 自检证据**：`src/tests/check_ws_manager.gd`（**76 项断言** / `total=76 failed=0`（headless）、`total=77 failed=0`（Xvfb，J7 有渲染器才计）/ `SELF-CHECK OK`；51 → 59 是 task_34-3 的 camera，59 → 73 是 task_35-2 的 A11 + K1~K13，73 → 76 是第 2 轮的 K14~K16）在同一进程里既当世界又当 sim（真 World.tscn + 真 `WebSocketPeer` 客户端），覆盖：装配链路、运动下行、传感器先到、空 manifest、断开清理、6 条错误路径、camera 装配线、**行人装配线**、**跨 kind 重建**、规范 lint。**两档都要 `SELF-CHECK OK`**：headless 与 `DISPLAY=:99 … --rendering-driver opengl3 --audio-driver Dummy`（不加 `--headless`）都是硬门槛（第 2 轮把 Xvfb 从"可选"改成权重 1）。

**⑨ 位姿下行（task_31 Step 1，`pose` 文本帧）**：

- **周期**：`const POSE_BROADCAST_INTERVAL_S := 0.1`（= 10 Hz，对齐真机 `state_notifier` 的 100 ms，`base robot.rs:213`）；**硬编码、不做配置项**（真机也是硬编码）。
- **只发四种条件全满足的连接**：`role == ROLE_CHASSIS && handshaken && !closed && vehicle 有效（is_instance_valid）`。传感器连接**一条都不发**；读取 `link.vehicle` 之前先判 `role`/`handshaken`/`closed`，`call("GetPose")` 之前必须有 `is_instance_valid` 守卫（失效 avatar 上调用会打 `Invalid call` = `SCRIPT ERROR` 且进程不退出）。
- **取值来源（E23）**：位置/朝向 = `link.vehicle.call("GetPose")`（= `WorldTransform.PoseFromTransform(global_transform)`，已是世界坐标）；速度 = `WorldTransform.VelToWorld(vehicle.velocity)`（`vz` 丢弃，不进帧）。
- **累加器与解耦**：`_pose_acc += delta`，`if _pose_acc + 1e-6 < POSE_BROADCAST_INTERVAL_S: return`（`+1e-6` 容差与 `Lidar2D` 同款：60 fps 下 `1/60` 累加 6 帧才恰好到 0.1，无容差会漏帧）；落后超过 2 个周期只补发一帧并清零（不积压）。`--fixed-fps 60` 下一整个 1 s 窗口稳定 **10 条**。
- **`_process` 内顺序**：`_AcceptConnections → _PollLinks → _SweepClosed → _Tick_Pedestrian_Timeouts → _BroadcastPoses`（本帧已判死的连接先清出 `_links`，再按行人超时停车，最后下发位姿；`_Tick_Pedestrian_Timeouts()` 来自 task_35 Step 2，只对 `_pedestrians` 生效，车没有超时）。
- **发送失败不处理**：`SendJson` 在非 `STATE_OPEN` 返回 `ERR_UNCONFIGURED` 属正常路径 —— 不检查返回值、不打日志、不断连（与 `ComponentLink` 既有语义一致）。
- **无新增日志**：逐帧零 `print` / `push_warning` / `push_error`；`stats_log` 保持 `false`。
- **自检证据**：`src/tests/check_pose.gd`（23 项断言 / `total=23 failed=0` / `SELF-CHECK OK`），同一进程真 `src/scenes/World.tscn` + 真 `WebSocketPeer` 客户端 + `Listen(0)` 临时端口，覆盖 task_31 §三 6.3 的 P1~P9（A1~A7 树外协议形态 / B1~B2 接线 / C1~C14 端到端与白盒）。

**⑩ 行人支路（task_35 Step 2 落地，**不改变上文车/传感器的任何语义**）**：

- **分流键**：`chassis_hello.kind`（`"robot"` 缺省 | `"pedestrian"`）。`kind == "pedestrian"` **且** `chassis == "pedestrian"` 时才走行人支路；`kind` 非法（任何其它值）只 `push_warning` 一次并**按 robot 处理**（连接不关）；`kind == "pedestrian"` 但 `chassis != "pedestrian"` 同样 warn + 按 robot。
- **两个表、两套编号**：车 → `_peer_vehicles` + `Vehicles` 容器 + `_next_avatar_id`；人 → `_pedestrians` + `Pedestrians` 容器 + `_next_pedestrian_id`。人**不进** `_peer_vehicles`、**不占** `_PickSpawn` 的 avatar 排位、**不消耗** `_next_avatar_id`，所以 `GetVehicleCount()` / `GetVehicle()` / `Vehicles` 容器三条既有语义**看不到人**。`link.vehicle` 是两者唯一共用字段（pose 下行 / 清理都靠它）。
- **容器是惰性建的**：`World.tscn` **不**加 `Pedestrians` 节点（无人连入时世界节点树**零变化**）；首个行人到达时由 `_Pedestrians_Root()` 在 `vehicles_root` 的父节点（World 根）下建一个同名 `Node3D`，与 `Bounds` / `Persons` / `Pedestrians` 的"代码建、幂等"策略一致（task_35 Step 3 起世界侧由 `BuildPedestrians()` **在 `_ready()` 里就建好**这个同名容器，manager 直接复用、不再自己建）。`_Pedestrians_Root()` 每次调用都 `is_instance_valid` 复检：**Step 3 的 `World.gd::BuildPedestrians()` 若重建这个容器，契约是"复用同名节点"或"重建后 manager 下次调用自动重新解析"**，否则已装的人会一起消失。
- **装配顺序（硬约束）**：校验场景 → 取容器 → 摘旧 avatar → 实例化 `Pedestrian.tscn`（**根节点就是 avatar**，不套 `Vehicle.tscn`）→ `add_child` → 写 `global_position` / `rotation.y` → `Configure(self, params)`（`Pedestrian._yaw_target` 取 `rotation.y`，所以**必须在摆位之后**）→ 记账 → 回 `chassis_ready{kind, pedestrian_id, spawn}`。
- **`chassis_ready` 只增不改**：robot 多一个 `"kind": "robot"` 键；pedestrian 用 `pedestrian_id`（不是 `avatar_id`）并回**校验后**的真实 `spawn`。`pose` 一个字节都没改（行人也是 `CharacterBody3D`，有 `velocity` 与 `GetPose()`）。
- **spawn 校验**（只有 pedestrian 读 `spawn`）：缺省 = 合法（默认点 `(64, 56, 0)`、不 warn）；有值但任一条件不满足 = `push_warning` + 回退默认点：`x/y` 非有限数 / 不在 `[0, span)`（`span` = `cells_x × voxel_size`，走 `ProjectSettings`，**不写 128 字面量**）/ `z ∉ [0, 2.0]` / 落在 `World.OBSTACLES` 内（外扩 0.3 m）/ 与任何已占 avatar 水平中心距 < 1.0 m。`yaw` 归一化到 `[-PI, PI)`。
- **指令面**：`motion_cmd{v, dir}`（`dir` 可省略 = 只改速度、保持朝向），老的 `action/speed` 照样接受（折算是 `Pedestrian.SetCommand` 的事，manager 只转发）。每收到一条刷新 `_ped_last_motion_ms` 并把 `_ped_idle` 置 false（**恢复时不打日志**）。
- **300 ms 超时停车**：`_process` 里 `_Tick_Pedestrian_Timeouts()` 只遍历 `_pedestrians`；`now - last_motion_at >= 300 ms` 且当前不是 idle → 下发 `SetCommand({v: 0})` + **一行**日志。**只停、不断连、不刷屏**（边沿触发）；刚注册的行人 `_ped_idle = true`，不会立刻被"停"一次。**车没有超时**（老 sim 不发指令时不会被强制停车）。
- **传感器**：行人没有 `AttachSensor`（没有 Chassis/Sensors 挂载点），`_MountSensor()` 用 `has_method` 守卫后走"几何未落地"的同一分支（回 `sensor_ready`、manifest 可置位、**绝不** `Invalid call`）。`_RegisterSensor` / `_TryAssembly` / `_CleanupLink` 的 avatar 查询从 `GetVehicle` 放宽到 `GetAvatar` / `_HasAvatar`（车的结果不变）。
- **日志指纹（新增，供 grep）**：`pedestrian spawned peer='…' id=… node=… spawn=(x, y) yaw=…` / `pedestrian idle peer='…'（300 ms 无新指令）-> v=0` / `pedestrian removed peer='…'` / `pedestrians root created: …`。无人连入时**一行都没有**。
- **跨 kind 重建（第 2 轮，`prev_issues` #2；两种 avatar 共用一套规则）**：
  - 注册前**不分类型**先 `_RemoveAvatar(peer_id, except_link=本连接)`：`_RegisterChassis` / `_Register_Pedestrian` 都无条件调用，两张表 + 两个容器一起清，`Vehicles` / `Pedestrians` 里都不留幽灵节点。两张表都空时**立即早退**（不 print、不 emit、不牵连其它连接）—— 这也是"传感器先于底盘"（E 组 pending）不被误关的原因。
  - 被判死刑的旧连接一律**交出所有权**（`link.vehicle = null`），于是它们下一帧的 `_CleanupLink()` 绝不会再摘掉同一个 `peer_id` **刚注册**的新 avatar。`_CleanupLink()` 只在 `link.vehicle != null` 时才摘。
  - `_ApplyMotionCommand()` **只按 `link.vehicle` 的身份分流**（"这条连接的 avatar 是不是 `_pedestrians` 里那个节点"），不看"表里有没有这个 peer"—— 换 kind 的过渡帧里旧连接可能还活着，用表判定会把车指令打到残留行人上。
  - `_RemoveAvatar(peer_id, except_link) -> bool`：返回值 = 是否真的摘到（`false` = 两张表都没有 → **静默**，不打 `vehicle removed` / `pedestrian removed`、不发 `vehicle_removed` / `pedestrian_removed`）。`_RemoveVehicle()` 作为兼容壳保留（内部转调，返回值丢弃）。
  - 自检：`K14`（robot → pedestrian，新连接）/ `K15`（pedestrian → robot，新连接）/ `K16`（**同一连接**重建）。三者都用 `vehicle_removed` / `pedestrian_removed` 的**信号流水** `slice(n) == ["v:…"] / ["p:…"]` 精确锁"只 emit 一次"，并把断言放宽到 `>= 1` 就会漏掉幽灵移除。

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
│                                  node_paths=PackedStringArray("debug_points")（E16）
├── Body (Node3D)                  instance res://assets/sensors/lidar_tmini.glb（task_33 Step 3；**纯视觉、无碰撞**，E27）
│                                  transform 恒 identity：模型原点 = 罐体底面中心 = 扫描面中心（D8），
│                                  模型 +x = 0° 射线方向；子树 4 个 MeshInstance3D：Base / Can / Window / FrontBump
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
而 UGV 车体高 `0.16 m`（task_33 Step 3 起 `0.25 m`）→ 旧车高下 0.2 m 以上的雷达**看不见**别的 UGV；
要看车体需把 `mount.z` 设到车体高度以下（自检 `D8-other-vehicle-visible` 用 `z=0.1` 证明"其它 avatar 可见"，
`0.1 < 0.25` 仍成立 → 该断言测量值 `0.7500 m` 在接入底盘模型前后**逐字不变**）。

**⑨ 自检**：`src/tests/check_lidar.gd`（**48 项断言** / `total=48 failed=0` / `SELF-CHECK OK`），命令同 §1.4 模板：
`timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_lidar.gd`。
证据链（"一圈能正确反映周围障碍的距离"）：`D1-open-space` 空旷 `max=0.0000`（360 × 0.0）、
`D2-forward-distance` `3.4000 m`、`D3-angle-span` 命中 `rel=[-8..8]`（17 条）、`D4-side-distance` `3.0000 m`、
`D5-no-echo-behind` 背向 `0.0`、`D6-follows-position` 前进 1 m → `2.4000 m`、`D7-mount-yaw-rotates` `3.0000 m`、
`D8-other-vehicle-visible` `0.7500 m`、`D11-range-clipping` 超量程 → `0.0`、`E1-rate-10hz` 60 帧 10 圈、
`E4-frame-format` 1455 B / `type=1` / `slot_id=7` / `seq` 1…10、`F7-real-wall-distance` 真实 `wall_c` `2.0000 m`、
`F8-real-wall-hits` 一圈 `152` 条（plan P2 实测 152）。

---

### 4.8 `src/scenes/sensors/CameraSensor.tscn`（task_33 Step 2）

> task 文档 §五-5 要求"新增 §4.7"，但 §4.7 已被 `Lidar2D` 占用（含 §4.7.1 的 9 条实现补充）；
> 为保证全文 20+ 处交叉引用不失效，本节编号取 **§4.8**，不重排既有编号。
> task 文档 §七 Step 4 的验收 6 写的"§4.7 补 `Capture_To_File` / 拍照脚本"同样指本节
> （`Capture_To_File` 与 Step 4 的拍照脚本说明都在 §4.8 / §4.8.2，§4.7 是 `Lidar2D`）。

```
CameraSensor (Node3D)                 script: res://src/sensors/CameraSensor.gd
│                                     职责：光心 = 根节点原点；Configure / Capture_Jpeg / Capture_To_File
├── Body (Node3D)                     instance res://assets/sensors/camera_module.glb（task_33 Step 3）
│                                     rotation_degrees=(0, 90, 0)；**纯视觉、无碰撞**（E27）；position 不写（= 光心）
│                                     子树 4 个 MeshInstance3D：Body / Barrel / Lens / Bracket
└── Viewport (SubViewport)            size=1280×720、own_world_3d=false、render_target_update_mode=0
    └── Lens (Camera3D)               current=true、fov=70、near=0.05、far=200、position=(0,0,0)
```

```gdscript
extends Node3D

const DEFAULT_WIDTH := 1280          # 另有 DEFAULT_HEIGHT=720 / DEFAULT_FOV_DEG=70.0 / DEFAULT_QUALITY=0.8
const DEFAULT_NEAR := 0.05           # near 0.05 = "贴车身也不穿模"（Step 4 的车鼻占比断言依赖它）
const DEFAULT_FAR := 200.0
const WARMUP_FRAMES := 2             # 前 2 帧是黑的（step 明令）
const DEFAULT_RATE_HZ := 1.0         # 出帧频率默认值（task_34 Step 3；UAV 相机口径）
const MIN_RATE := 1e-3               # 频率下限，防 1/0
const ACCUM_EPS := 1e-6              # 累加器容差（与 Lidar2D 同款）

@export var viewport: SubViewport    # node_paths=PackedStringArray("viewport", "lens")
@export var lens: Camera3D           # NodePath("Viewport/Lens")

var width: int = DEFAULT_WIDTH       # 以下 5 个 = Configure() 的生效参数
var height: int = DEFAULT_HEIGHT
var fov_deg: float = DEFAULT_FOV_DEG
var quality: float = DEFAULT_QUALITY
var rate_hz: float = DEFAULT_RATE_HZ

signal frame_produced(seq: int, world_time: float, size_bytes: int)   # task_34 Step 3

func Configure(params = null) -> void             # width / height / fov_deg / quality / **rate_hz**
func Capture_Jpeg() -> PackedByteArray            # await；渲一帧 → JPEG 字节
func Capture_To_File(abs_path: String) -> bool    # await；上面 + 落盘（自动建目录，只收绝对路径）
func Get_Image_Size() -> Vector2i                 # viewport.size（refs 缺失时回退 width/height）
func Can_Capture() -> bool                        # 环境探测（**非 task 文档列的交付能力**，见 ⑦）
func Get_Params() -> Dictionary                   # 生效参数副本（5 键，rate_hz 追加在末尾）
func Get_Last_Error() -> String                   # 最近一次失败原因（空串 = 没失败过）
func Get_Capture_Count() -> int                   # 成功出图次数

# ---- task_34 Step 3：下行流（与 Lidar2D 同签名同时序，manager 鸭子调用）
func SetStream(link, sensor_id: int) -> void      # sensor_ready **之后**绑流；累加器清零
func ClearStream() -> void                        # 解绑（清 link / sensor_id / 累加器）
func HasStream() -> bool
func Tick_Stream(delta: float) -> void            # 累加器唯一实现（_process 只转发它）
func Emit_Frame(jpeg: PackedByteArray) -> bool    # 出帧唯一出口：true = 已 SendBinary
func Get_Tick_Count() -> int                      # 到点次数（>= 下者）
func Get_Frame_Count() -> int                     # 真正下发的帧数
```

**光轴 = 传感器局部 `−Z`（task_33 Step 3 的朝向契约）**：

| 事实 | 值 | 后果 |
|---|---|---|
| `Camera3D` 看哪边 | **局部 `−Z`**（引擎语义），`Lens` 的 `global_transform` 由 `_Sync_View_Transform()` 同步为 holder 的变换 | 光轴 = `-holder.global_basis.z`，不是 `+X` |
| `camera_module.glb` 的镜头 | 朝模型 **+X**，且模型全部落在 `z ∈ [0, 0.055]`（光心之后） | 要转 `+90°` 才朝光轴，并保证相机**拍不到自己的壳**（`near = 0.05` 在光心之前） |
| `Body.rotation_degrees = (0, 90, 0)` | `basis * (1,0,0) = (0,0,-1)`（实测） | 写 `-90` 会让镜头朝车尾、Step 4 拍到"反方向"的车 |
| 模型原点 | 镜片前表面中心 = **光心**（`A4-lens-node` 锁 `Lens.position == 0`） | 不写 `Body.position` |
| 前视挂载 | 光轴 = 局部 `−Z`，而车头 = 局部 `+X` → 想"看车头方向"必须 `mount.yaw = +PI/2`（**弧度**，见下方单位说明） | Step 4 的拍照机位按此设（`check_camera_shots.gd` 的 `CAM_MOUNT`） |

> **`mount` 的单位（易错，Step 4 实测）**：`yaw` 是**弧度**、`pitch` 是**度**（`Vehicle.gd:183` 对 `pitch`
> 做 `deg_to_rad`、对 `yaw` 不做）。所以前视挂载写 `yaw = +PI/2`（≈1.5708）→ `rotation.y = -1.570796`；
> 若照旧版本节误写的 `+90` 当度用，会得到 `rotation.y = -90 rad ≡ -116.57°`、光轴斜 26.6°，
> 两张"前视"图全歪（`check_camera_shots.gd` 的 `S12-camera-yaw-unit` 专门锁这一点）。
> 旧版本节该行写的 `mount.yaw = +90` 是**单位笔误**，已按实测改正。

> `pitch` 的符号与落点见 §4.3 的 `mount.pitch` 行：`rotation.x = pitch`（度），**正 = 抬头、负 = 低头**。

接口名沿用 task 文档的 **Pascal_snake**（`Capture_Jpeg` / `Get_Image_Size` / `Can_Capture`），
与 §9.2 的 `PascalCase` 示例风格不同但**文件内自洽**（接口名由 task 文档锁定，不自作主张改名）。

`Capture_Jpeg()` 的**固定顺序**（task 文档 §五-4 明令，不许调换）：

```
① warmup 未过（前 2 帧）→ await get_tree().process_frame ×2
② viewport.render_target_update_mode = SubViewport.UPDATE_ONCE   # 只渲这一帧
③ await RenderingServer.frame_post_draw                          # 必须：否则读到上一帧 / 空缓冲
④ img = viewport.get_texture().get_image()                       # GPU → CPU 回读
⑤ return img.save_jpg_to_buffer(quality)                         # 1280×720 实测 ~10 ms
```

失败语义（四类，全部 **`push_error` + 返回空 `PackedByteArray()`**，原因写进 `Get_Last_Error()`）：

| # | 判定 | 说明 |
|---|---|---|
| 1 | `not _Has_Renderer()` | headless / 无渲染器 —— **必须先判，绝不触碰 `get_texture()`**（E26-③） |
| 2 | `viewport == null or lens == null` | 场景绑定丢失（漏写 `node_paths`，E16） |
| 3 | `not is_inside_tree()` | 树外调用（与 `Lidar2D.ScanOnce()` 的 R5 同款） |
| 4 | `get_image()` 为 null / 空 | 渲染未就绪 |

`Capture_To_File()` 的路径类失败（空路径 / 非绝对路径 / 建不了目录 / 写不进）一律 **`push_warning` + 返回 `false`**
——这是**可预期的调用方错误**，而本 step 的验收要求"日志 0 ERROR"，只有抓拍失败才 `push_error`。

#### 4.8.1 实现补充（Step 2 落地时确定，**不改变上文设计**）

**① 组件是"独立零件"**：Step 2 落地时**不挂模型**（Step 3 已补 `Body` + `camera_module.glb`，见上文节点树）、
**不接网络**（无推流 / 无 1 Hz 定时器 / 无 `camera.capture()`，task_33 §九 已明列非目标）、
**不登记 `SensorCatalog`**（目录注册属 `sensor_hello` 装配线，已于 task_34 Step 3 补上，见 §4.8.3）、
**不实例化进 `World.tscn`**（挂世界仍是 Step 4 的拍照脚本）。
Step 2 落地时既有 11 个自检的断言数与基线**逐字一致**；Step 3 补 `Body` 后 `check_camera_sensor` 的
A 组 12 项测量值与基线**逐字不变**（模型在光心后方、零碰撞，`A5-no-collision` 计数仍 0）。

**② 无碰撞体（D8）**：子树里 `CollisionShape3D` / `CollisionPolygon3D` / `PhysicsBody3D` / `Area3D` 计数 **0**；
源码白盒再关一道（`A5-no-collision` + `A12-no-physics-lint`：去掉整行注释后不得出现
`UPDATE_ALWAYS` / `intersect_ray` / `move_and_slide` / `CollisionShape3D` / `CollisionPolygon3D` / `PhysicsBody3D`）。

**③ E26 四条实测约束的落点**：`own_world_3d=false`（共享父视口 `World3D`）、
`update_mode` 初始 `0=DISABLED` 只在抓拍时置 `UPDATE_ONCE`、渲染信号**只 `await` 不 `connect`**、
`Lens` 必须在 `SubViewport` 下（不抢根视口 `current`）、`Lens.position == Vector3.ZERO`（根节点 = 光心）。

**④ `SubViewport` 会"断掉" Node3D 的变换继承链（本 step 最隐蔽的坑）**：
`SubViewport` 继承 `Viewport → Node`，**不是 `Node3D`**。因此
`CameraSensor(Node3D) → Viewport(SubViewport) → Lens(Camera3D)` 里，`Lens.global_transform`
**不会**继承根节点的 `position / rotation`——实测根节点在 `(64, 6, 64)`、`rotation=(-20°,0,0)` 时，
`Lens.global_transform` 仍是 **IDENTITY**（拍到的就是世界原点的画面）。
所以 `Capture_Jpeg()` 在 ①② 之间固定调用 `_Sync_View_Transform()`：`lens.global_transform = global_transform`。
`A4-lens-node` 锁死"`Lens` 在 `SubViewport` 下 + 局部 `position == 0`"，
`B3-not-black` 从数据面兜底（没同步时渲的是"空世界"，灰度方差会掉到 0 附近）。

**⑤ `Configure` 的形参写 `Variant`（不是 `Dictionary`）**：`Configure(null)` 在形参为 `Dictionary` 时是
**编译期错误**（`Cannot pass a value of type "null" as "Dictionary"`，实测），而 task 文档要求空字典 / 缺键容错、
自检 A10 直接传 `null`。鸭子契约（`Vehicle.AttachSensor()` 传 `Dictionary`）不受影响。

**⑥ 抓拍后 `render_target_update_mode` 停在 `UPDATE_ONCE(=1)`**：4.7.2 **不会**自动回落成 `DISABLED`
（`A3` 断言的初始 `0` 只对 `.tscn` 初始态成立）。自检 `B9-update-mode-restored` 断言"抓拍后仍是 `ONCE`、
**不是 `ALWAYS(=4)`**"，即"没有偷偷每帧渲"。

**⑦ 落盘失败"绝不打印 ERROR"**：`DirAccess.make_dir_recursive_absolute()` 失败时走 `ERR_FAIL_V_MSG`，
会打 `ERROR: Could not create directory: '...'`（实测，B7 的坏路径用例直接命中），把"0 ERROR"验收搞假红。
改用自写的 `_Make_Dir_Chain()`（`DirAccess.dir_exists_absolute` 探测 + `DirAccess.make_dir_absolute` 逐级建），
失败**只返回 `Error` 码、不打印**。

**⑧ 性能（实测，llvmpipe 软件 GL）**：1280×720 单张 ≈ **650 ms**（含 warmup 2 帧 ≈ 380 ms 与
`frame_post_draw` 等待 ≈ 190 ms）；稳态单张 ≈ 200 ms（`get_image` 4.7 ms + `save_jpg` 10 ms）。
`--fixed-fps` 只固定逻辑步长、**不**加速渲染，所以自检 B 组 9 项走 6 次抓拍 ≈ 3 s，整条自检 < 15 s。

**⑨ 实测数据（Xvfb :99 + llvmpipe，`tmp/task33/check_camera_sensor_x11.log`）**：

| 项 | 实测值 |
|---|---|
| 1280×720 / quality 0.8 | **29 147 B**、首三字节 `ffd8ff`、SOF0 = 1280×720 |
| 灰度统计（对**交付的 JPEG 解码后**统计） | `var=0.018553`、`mean=0.5668`、`spread=0.875`（阈值 `>0.002` / `0.05~0.95` / `>0.15`） |
| 同机位改 `fov_deg` 70 → 40 | 29 147 B vs **25 005 B**，**字节不同** ✅ |
| 同 fov 连拍 | 两次 `29 147 B` / 逐字节相同（仅记录，不作判据） |
| `Configure({640,360})` 后抓拍 | SOF0 = **640×360**、**8 904 B** |
| `Capture_To_File`（`user://check_camera_sensor/cap_check.jpg`） | `true`、25 005 B、首三字节 `ffd8ff` |
| `Capture_To_File("/proc/orion_no_such_dir/never.jpg")` | `false`，只 1 行 `WARNING`，**无 ERROR** |
| 抓拍后 `render_target_update_mode` | **1（UPDATE_ONCE）** |

**⑩ 自检**：`src/tests/check_camera_sensor.gd`（`SceneTree` 形态，照 `check_sensor_models.gd`）。
A 组 18 项（结构 / 参数 / 出帧）**恒跑**；B 组 10 项（真渲染）只在 `Can_Capture()==true` 时跑，
headless 下逐条 `[skip]` 且**不计入 `total`**（否则断言数会和基线错位）。
其中 A13~A18 与 B10 是 task_34 Step 3 追加的；A13~A17 用**假 JPEG**（`FF D8 FF …`）走 `Emit_Frame()` 真路径 +
**手工 `Tick_Stream()`** 推步，所以 headless 下也**不依赖像素、不依赖墙钟**（§4.8.3-③⑥）。

| 跑法 | 命令 | 期望 |
|---|---|---|
| 纯 headless | `timeout 240 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_camera_sensor.gd` | `total=18 failed=0` + `SELF-CHECK OK` + EXIT=0 + `[skip]` ×10 |
| **真验收（出图）** | `DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_camera_sensor.gd` | **`total=28 failed=0`** + `SELF-CHECK OK` + EXIT=0 + 0 ERROR |

断言清单：`A1` 场景可加载 / `A2` 根节点名+类型+脚本路径 / `A3` `SubViewport` 共享世界+尺寸+初始不渲 /
`A4` 镜头参数+父节点是 `SubViewport`+光心在原点 / `A5` 零碰撞体 / `A6~A10` `Configure` 的
空字典 / 尺寸 / 缺键 / 非法值回退 / `null` 五态 / `A11` `Can_Capture()` 与显示环境一致 /
`A12` 源码白盒 lint；`B1` 非空+JPEG 魔数 / `B2` SOF0 尺寸 / `B3` 非全黑 / `B4` 改 fov 成图不同 /
`B5` 连拍稳定 / `B6` 落盘 / `B7` 坏路径返回 false / `B8` 分辨率真的生效 / `B9` 没有偷偷变 `ALWAYS`；
**task_34 Step 3 追加**：`A13` 流绑 / 解 / 重绑 / `A14` 帧头字段+payload 原样+魔数 / `A15` `seq` 递增+空图不出帧 /
`A16` `rate_hz` 节奏（1 Hz 每 60 步 1 次、2 Hz 每 60 步 2 次） / `A17` 无渲染器闸门（树外实例：只 warn 一次、零帧、
`Get_Last_Error()` 仍空） / `A18` `SensorCatalog` 的 `camera` 登记（场景路径 + 5 键默认值逐键比对） /
`B10` 真抓拍 → 真帧（`15 + N` 字节、payload 首三字节 `ffd8ff`、SOF0 = 当前成图尺寸）。

#### 4.8.2 拍照脚本（task_33 Step 4 落地，**不改变 §4.8 设计**）

`src/tests/check_camera_shots.gd`（`SceneTree`，本 step 唯一代码交付物）把 §4.8 的组件**挂进真世界**拍照：

```
① load("res://src/scenes/World.tscn") → **add_child 之前** 关掉 Systems/WebSocketManager.autolisten
   → add_child（真世界：12104 体素格 / 地面 / 5 面空气墙，日志 3 行）
② load("res://src/scenes/Vehicle.tscn") → World/Vehicles.add_child → 摆到世界 (60, 84, 0)、朝向 A
   → ApplyChassis("diff_drive_ugv")（显示建模底盘 ModelUGV，关掉橙色占位 Box）
③ AttachSensor("lidar_top",  Lidar2D.tscn,      {x:-0.05, y:0, z:0.25, yaw:0})
   AttachSensor("camera_nose", CameraSensor.tscn, {x: 0.15, y:0, z:0.12, yaw:PI/2, pitch:0})  ← 前视
④ 三个机位各拍一张 → Capture_To_File(<绝对路径>) → tmp/task33_shots/
⑤ 35 项断言（S1~S18 结构 / R1~R9 渲染）→ SELF-CHECK OK | FAILED
```

**机位表（`SHOTS`，顺序即文件名前缀）**

| # | 文件 | 机位 | 拍什么 | 为什么这么摆 |
|---|---|---|---|---|
| 0 | `shot_00_heading_a.jpg` | onboard，车朝 **A = 0°（东）** | 8 m 高的 `pillar_e` 落在画面正中（世界 x∈[78,80]、y∈[78,80]） | `pillar_e` 在 `World.gd` 的 `OBSTACLES` 里，改布局要同步重测 `R8` |
| 1 | `shot_01_heading_a_plus_90.jpg` | onboard，车朝 **A+90°（南）** | 8.5 m 外 30 m 长的 `wall_c` 铺满画面、柱子完全离场 | 证明"对着哪拍哪"（同一台相机、同一个世界，只转车头） |
| 2 | `shot_02_external_3q.jpg` | external，世界级相机（车头右前上 3/4） | 车体 + 车顶雷达 + 地面 + `wall_c` 同框 | 证明"不是固定背景"；外部 holder 挂 `world` 下（挂车上会跟着车动） |

**落盘路径**：`const OUT_DIR := "/vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/task33_shots"`
—— **绝对路径**，且 `S15-out-dir` 断言"是绝对路径 + 以 `tmp/task33_shots` 结尾 + 不以 `res://` 开头 + 目录建得出来"。
`Capture_To_File()` 只收绝对路径（相对路径直接 `push_warning` + `false`），并自动逐级建目录。

**`Capture_To_File()` 的失败语义**（沿用 §4.8，不改）：抓拍失败 → `Capture_Jpeg()` `push_error` + 空字节 →
返回 `false`；路径类失败 → `push_warning` + `false`。**拍照脚本自己不再 `push_error`**（否则测试自己把
"日志 0 ERROR"验收搞红），失败一律 `_check(label, false, detail)` + `quit(1)`，且把
`Get_Last_Error()` 写进 detail。

**为什么机位取 `yaw = PI/2` 而不是字面的 `yaw = 0`**（计划 R4/E5，本 step 唯一一处与 step 文案的偏差）：
`yaw=0` 是 **sim 侧世界系（相机看 +x）** 的口径；世界侧 Step 3 已锁定"光轴 = 局部 `−Z`"，
世界侧的等价前视挂载 = `+PI/2`。实测 `yaw=0` 时车体投影占屏 **1.3767**（整幅是自己的侧壁，`R6` 必红）；
`+PI/2` 时为 **0.0000**。若评审要求逐字 `yaw:0`，只改 `CAM_MOUNT["yaw"]` 一处常量，但 `R6` 会立刻红。

**断言清单（35 项 = S 组 18 + R 组 17）**

| 组 | label | 判据（实测值） |
|---|---|---|
| S | `S1-world-loads` / `S2-world-built` / `S3-ws-inert` | 世界可载入并进树；12104 格 ≥ 12000 + `Bounds` 5 个形状；`WebSocketManager.autolisten=false` 且 `IsListening()=false`（**不监听 9999**） |
| S | `S4-vehicle-spawn` / `S5-chassis-model-shown` / `S6-vehicle-still` | 父节点 `World/Vehicles`、位置/朝向经 `WorldTransform`；`ModelUGV.visible=true` + `Body.visible=false`；12 个物理帧位移 `0.000000` |
| S | `S7`~`S10` 挂载 | 两件传感器挂在 `Vehicle/Sensors` 下、`position = PosToGodot(mount)`、`rotation.y=0`（雷达）/ `rotation.x=0`（相机 pitch 缺省） |
| S | `S11-camera-looks-nose` / `S12-camera-yaw-unit` | `(-holder.global_basis.z)·车头 = 1.000000 > 0.999`；`rotation.y = -1.570796`（= −π/2，**锁定 yaw 单位 = 弧度**） |
| S | `S13-sensors-no-collision` / `S14-camera-params` | 传感器子树物理节点 **0**；`Get_Image_Size()=1280×720`、`fov=70.0`、`quality=0.80`、`Viewport.own_world_3d=false` |
| S | `S15-out-dir` / `S16-shot-plan` / `S17-renderer-gate` / `S18-whitebox-lint` | 见上文；机位表 onboard×2 + external×1；`Can_Capture() == (display != "headless")`；自身源码剥整行注释后不得出现 `on_tick` / `WebSocketPeer` / `connect_to` / `HTTPClient`（**不跑决策链、不连 WS**） |

R 组（**仅 Xvfb 档**，17 项 = 逐张 12 + 全局 5）：

| 组 | label | 判据（实测值） |
|---|---|---|
| R1 | `R1-shot-0x-file` | 文件存在、> 10 KB、首三字节 `ffd8ff`（实测 **26 315 / 24 462 / 29 576 B**） |
| R2 | `R2-shot-0x-size` | SOF0 = 1280×720 **且**解码出的 `Image` 尺寸同为 1280×720 |
| R3 | `R3-shot-0x-not-blank` | `var>0.002`、`0.05<mean<0.95`、`spread>0.15`、近白 < 0.50（非全黑 + 非全白） |
| R4 | `R4-shot-0x-grid` | 16×9 网格：格内 `max-min>0.05` 的格数 ≥ 20/144、`abs(格均值−全图均值)>0.10` 的格数 ≥ 10（实测 56/28、42/28、48/30） |
| R5 | `R5-shots-differ` | 三对两两：字节差异 > 0.10 **且** 灰度平均绝对差 > 0.02（实测 0.823/0.070、0.963/0.115、0.960/0.113） |
| R6 | `R6-nose-not-occluding` | `shot_00/01` 的车体投影占屏 **< 0.05**（实测 **0.0000**）—— step 硬性"车鼻占比 < 50%"的强化版 |
| R7 | `R7-external-shows-car` | `shot_02` 的车体投影占屏 ∈ (0.02, 0.30)（实测 **0.0689**）—— 外部机位真的框到了车 + 车顶雷达 |
| R8 | `R8-pillar-follows-heading` | `shot_00` 里 `pillar_e` 屏内角点 **≥ 4**（实测 **8**，bbox `(467,133)-(536,363)` 画面正中）；`shot_01` **≤ 2**（实测 **0**，全在镜头后方） |
| R9 | `R9-capture-bookkeeping` | 两台的 `Get_Capture_Count()` 合计 **3**（机身 2 + 外部 1）、`Get_Last_Error()` 为空、三张文件 mtime ≥ 本次运行起始 − 2 s（防拿旧图充数） |

**实测基线（Xvfb :99 + llvmpipe，`tmp/task33/check_camera_shots_x11.log`）**

| 项 | shot_00 朝东 | shot_01 朝南 | shot_02 外部 3/4 |
|---|---|---|---|
| 字节 / SOF0 | **26 315** / 1280×720 | **24 462** / 1280×720 | **29 576** / 1280×720 |
| 灰度 `var` / `mean` / `spread` | 0.009563 / 0.5728 / 0.824 | 0.008259 / 0.5206 / 0.686 | 0.012607 / 0.4947 / 0.957 |
| 网格 `contrast` / `midband`（/144） | 56 / 28 | 42 / 28 | 48 / 30 |
| 车体投影占屏 / `pillar_e` 屏内角点 | **0.0000** / **8** | **0.0000** / **0** | **0.0689** / — |
| 相机世界位置 | `(60.15, 0.12, 84.0)` | `(60.0, 0.12, 84.15)` | `(60.62, 0.48, 83.6)` |

整体耗时 ≈ **6 s**（`--fixed-fps 60`、`timeout 240` 有充足余量）。

**两档跑法**（§1.4 有可直接复制的命令）

| 档 | 期望 | 说明 |
|---|---|---|
| 纯 headless | `total=18 failed=0` + `SELF-CHECK OK` + `[skip]` ×17 + 0 ERROR | R 组**不计入 total**（否则断言数会和基线错位）；S 组 18 项前置条件在无渲染器时全部成立 |
| Xvfb 真渲染 | **`total=35 failed=0`** + `SELF-CHECK OK` + EXIT=0 + 0 ERROR | 3 张图落盘 + S+R 全跑 |

**边界（与 §4.8 一致）**：不启动 `pleiades-sim`、不连 WS 客户端（脚本把 `World.tscn` 自带的
`WebSocketManager.autolisten` 在 `add_child` **之前**关掉，日志无 `[ws] listening`）、不跑决策链 / 不造
`motion_cmd`；**不改** `CameraSensor.gd` / 任何 `.tscn` / `assets`；不登记 `SensorCatalog`。
> 最后一条只对 task_33 Step 4 成立：task_34 Step 3 起 `SensorCatalog` 已登记 `camera`（见 §4.8.3 / §7.6），
> 装配线会实例化本组件；`check_camera_shots.gd` 本身仍直接实例化、不依赖目录。

#### 4.8.3 出帧（task_34 Step 3 落地，**不改变 §4.8 设计**）

相机从"按需单拍"变成"可按频率出帧的可装配零件"。**抓拍内核一行未改**（`Capture_Jpeg()` 的五步、
`_Sync_View_Transform()`、`.tscn` 的 `update_mode=0` 与"只在抓拍时置 `UPDATE_ONCE`"全部原样），
新增的只是"什么时刻拍、拍完发给谁"。

**① 时序铁律**：`SetStream(link, sensor_id)` 必须在 `sensor_ready` **之后**由 manager 鸭子调用
（`WebSocketManager._MountSensor()` 里 `node.has_method("SetStream")` 那一行，与 `Lidar2D` 同款钩子，
manager 一行未改）。先绑流再回 ready 会让 sim 侧出现"收到帧但还不知道 slot_id"的窗口（§5.3）。

**② 累加器**：`_process(delta)` 只转发 `Tick_Stream(delta)`；公式与 `Lidar2D` **逐字同款**：

```
period = 1.0 / max(rate_hz, MIN_RATE)          # MIN_RATE = 1e-3，防 1/0
_accum += max(delta, 0)
if _accum + ACCUM_EPS < period: return          # ACCUM_EPS = 1e-6
_accum = min(_accum - period, period)           # 卡顿后最多补一张，不"追赶爆拍"
_tick_count += 1; _Capture_Tick()               # 协程，**这里绝不 await**
```

`1e-6` 容差不是可选项：60 × 1/60 = `0.9999999999999999` < 1.0，不加容差 1 Hz 一个整周期出 0 帧。
`_link == null` 时 `Tick_Stream()` 首行直接 return —— 未绑流的实例（`check_models` / `check_camera_shots`
里的相机）**零开销、零副作用**。

**③ 无渲染器闸门（E26-③ 的直接后果）**：`Can_Capture() == false`（`--headless` = dummy 渲染器）时
**只 `push_warning` 一次、绝不出帧、绝不 `await`** —— 否则协程会永久挂在 `frame_post_draw` 上。
到点计数刻意放在闸门**之前**：headless 下"1 Hz 到点 1 次"仍可被观测，而"零帧"同时成立，两条要求互不掩盖
（`check_camera_sensor.A17` + `check_ws_manager.J6` 双向钉死）。

**④ 一帧 = 15 B 头 + JPEG 原始字节**（§8.2）：`WireProtocol.BuildCameraFrame()`，不重编码、不加长度前缀；
宽高从 JPEG 的 SOF0 段读（不上线），端序与 lidar 帧逐字节相同（GDScript 的 `encode_*` 一律小端）。

**⑤ `_capturing` 互斥**：上一张还没回来时，到点**只计数不抓拍**（帧率上限 = 抓拍耗时）。因此
`Get_Tick_Count() >= Get_Frame_Count()` 恒成立；不做队列、不积压、不丢帧补偿。

**⑥ 可观察语义**：`signal frame_produced(seq, world_time, size_bytes)` 在"一张图组好之后"发出，**有 link
但未 OPEN 时也发**（与 `Lidar2D.scan_produced` 同口径）；`Get_Tick_Count()` = 到点次数、
`Get_Frame_Count()` = 真正 `SendBinary` 成功的帧数。信号只带字节数、不带数组（避免每帧复制大缓冲）。

**⑦ 失败表（本 step 只新增一种失败面）**：

| 情形 | 行为 |
|---|---|
| `jpeg` 为空（`Capture_Jpeg()` 失败） | `_Capture_Tick()` 直接 return；`push_error` 已由 `Capture_Jpeg()` 打过，`Get_Last_Error()` 有原因 |
| `Can_Capture() == false` | **只 warn 一次**、`_tick_count` 照增、**绝不出帧、绝不 await** |
| 有 link 但未 OPEN | `Emit_Frame()` 返回 `false`，`frame_produced` 照发，`_frame_count` 不增 |
| 无 link（自检直调 `Emit_Frame`） | 内部 `_seq` 自增，返回 `false`，信号照发 |

**⑧ `Emit_Frame()` 是公开的，且不是测试专用缝**：它是**出帧唯一出口**，生产路径 `_Capture_Tick()` 也走它；
与 `Lidar2D._Publish()` 同构，区别只在"payload 可注入"——因为相机的 payload 来自一次耗时的 `await` 抓拍，
headless 下无法经 `Capture_Jpeg()` 出图，帧头 / 魔数只能这样验（`check_camera_sensor.A14/A15`）。

---

### 4.9 `src/scenes/props/Person.tscn`（human_model Step 4 ✅）

**定位**：§3 的**第 9 个** `.tscn`，但它**不是装配零件** —— 没有 `chassis_hello` / `sensor_hello` 语义、
不在 `ChassisCatalog` / `SensorCatalog` 里、不由 `WebSocketManager` 实例化、不参与任何决策链。
它只做一件事：在世界里站一个"能被相机拍到、能被 YOLO 认出"的人。

```
Person (Node3D)                          script: res://src/props/Person.gd
└── Body                                 instance: res://assets/humans/rp_carla_rigged_001/model.glb
                                         静态实例；transform = IDENTITY；**无 CollisionShape3D**
    └── (glb 内部) 1 个 MeshInstance3D     ← glb 根是 Node3D，子节点才是 mesh（与 E27 同款形态）
```

```gdscript
# src/props/Person.gd  （**唯一**人物脚本：静态 + 走动；walking_person Step 3 起）
extends Node3D

const MODEL_GLB := "res://assets/humans/rp_carla_rigged_001/model.glb"   # 静态人形；与 Person.tscn 的 Body 逐字一致（自检 C1 锁）
const WALK_GLB := "res://assets/humans/rp_nathan_animated_003_walk/model.glb"  # 走动人形；与 PersonWalking.tscn 的 Body 逐字一致
const HEIGHT_MIN_M := 1.60                                               # step.md 判据：人高 ∈ [1.60, 1.85] m
const HEIGHT_MAX_M := 1.85
@export var walk_speed: float = 0.0     # m/s；**<=0 表示不走**（默认值 = 静态站立，向后兼容 Person.tscn）
@export var path: Array[Vector3] = []   # 世界坐标（Z-up，米），隐式闭合；空 = 静态
@export var loop: bool = true           # true=循环；false=到终点停在末点

func Get_Body() -> Node3D            # "Body"（glb 实例根）；缺失返回 null（自检据此报红）
# 走动侧接口（仅 PersonWalking.tscn 用，见 §4.10）：Configure / Set_Progress_M / Get_Progress_M /
# Get_Path_Length / Get_Speed_Mps / Get_Position_W / Get_Yaw_W / Get_Anim_Player / Is_Anim_Playing /
# Path_Hits_Rect
```

**① 为什么零碰撞（E27 的延伸，本次的硬理由）**：人一旦带 `CollisionShape3D`，
`check_lidar` 的 `F7-real-wall-distance=2.0000` / `F8-real-wall-hits=152` 这两条**逐字基线**就会多出回波；
UGV 还会把行人当障碍绕行 / 顶开，`goto` 冒烟语义被改。所以 `Person` 子树、`Persons` 子树的
`CollisionShape3D` / `CollisionPolygon3D` / `PhysicsBody3D` / `Area3D` 计数**必须恒为 0**
（`check_person_assets` 的 `asset-no-collision` / `scene-no-collision` / `world-persons-no-collision` 三重锁）。

**② 为什么不进 `World.tscn`**：容器由 `World.gd::BuildPersons()` 在 `_ready()` 末尾建，理由与 `Bounds` 完全相同
（不写 `.tscn` → 不碰 `check_bounds.B1d` 的文本断言、不改 `load_steps`、避开 E16）；且**幂等**——
重复调用先 `remove_child` + `queue_free` 旧容器再重建，`check_person_assets` 的 `world-persons-idempotent`
连调两次后断言"子节点仍 2、同名节点仍 1、节点总数不增长"。

**③ 摆位（世界坐标，Z-up，米；yaw 0 = 朝 +X = 世界东）**：常量表 `World.PERSONS` 是唯一来源。

| 名 | 世界坐标 | 到出生点 (64,64) | 最近障碍 | 说明 |
|---|---|---|---|---|
| `person_01` | `(60, 76, 0)` | 12.65 m ≫ 3 m 净空 | `wall_c`（`y∈[92,93]`）16.0 m | 出生点西南开阔地 |
| `person_02` | `(68, 70, 0)` | 7.21 m ≫ 3 m 净空 | `pillar_e` 12.8 m | 出生点东南开阔地 |

> **与 `step.md` 举例 `(60,84)` 的差异（必须记录）**：`(60,84)` 正是 `check_camera_shots.gd` 的
> `SPOT_WORLD` —— 该自检会在同一格摆一台静止 UGV 连拍 3 张。人在那里会**与车重叠**（人无碰撞，不会被推开），
> 且无谓扰动 R 组的图像统计基线。故沿同一条 `x=60` **南移 8 m 到 `(60,76)`**，保持"出生点西南侧空地、
> 两人相距 10 m、都在开阔地面"的原意；第二个例子 `(68,70)` 不变。`check_person_assets` 的
> `world-persons-spawn-clear` / `world-persons-obstacle-clear` 把净空与"不压障碍"变成机检。

**④ 资产口径**（细节与来源 URL / 许可原文见 `assets/humans/README.md`）：
`rp_carla_rigged_001` = Renderpeople Free Rigged，**米制、人朝 +X、脚底 `y=0`（glTF/Godot Y-up）、
单 mesh / 单 surface / 无骨骼 / 无动画**，`1.7307 m` / `19 988 tri` / glb `2 664 856 B`；
备选 `rp_eric_rigged_001`（`1.8400 m` / `20 542 tri` / `2 438 336 B`）**进库但不进世界**——
换用只改 `Person.tscn` 的 `ext_resource` 与 `Person.MODEL_GLB` 两处（自检 `script-model-const` 会逼两处一致）。

**⑤ 导入副产物（§9.1 E28）**：Godot 4 的 glTF 导入默认把内嵌贴图**抽取**成 `.glb` 旁的独立文件
（`model_rp_*_dif.jpg` + `model_rp_*_norm.png` + 各自 `.import`）。**这 4 个文件必须与 `.glb` 一起提交**，
否则任何人重导都会 `ERROR: Cannot open file`；且运行前必须先 `--import`（`.godot/` 缺失时 glb 的 `load()` 返回 `null` → `Body` 空壳、**看不到人** + 12 行 `ERROR`，自检 14/32 红）。

**⑥ 自检证据**：`src/tests/check_person_assets.gd`（**32 项断言** / `total=32 failed=0` / `SELF-CHECK OK`），
A 组资产 16（每件 8：loads / instantiates / aabb / feet / no-collision / material / tri-budget / sha256）、
B 组场景 5、C 组常量自洽 2、D 组世界摆位 9。

---

### 4.10 `src/scenes/props/PersonWalking.tscn`（walking_person Step 3 ✅）

**定位**：§3 的**第 10 个** `.tscn`，与 §4.9 的 `Person` 平级 —— **不是装配零件**（无 `chassis_hello` /
`sensor_hello` 语义、不在 `ChassisCatalog` / `SensorCatalog` 里、不由 `WebSocketManager` 实例化）。
它只做一件事：**让世界里的这个人沿路径走起来**（走路动画在播 + 位置随时间变），
**不做**碰撞体、**不接**雷达 / 建图 / 决策链、**不做** YOLO 验证。

**与 §4.9 的关系（Step 3 的收口）**：两者**共用同一个根脚本 `src/props/Person.gd`** ——
走动能力并入 `Person.gd`（`walk_speed` / `path` / `loop` 三字段 + 路径采样 + 动画播放），
**不给参数时默认静态 = `Person.tscn` 行为逐字不变**；`PersonWalking.gd` 已删除（不再有两份要同步的人形脚本）。
`Person.tscn` 本身**一个字节都不动**（`check_person_assets` 的 32 项把它的 Body 路径 / 高度 / 三角数 / sha256 逐位钉死）。

```
PersonWalking (Node3D)                   script: res://src/props/Person.gd
└── Body                                 instance: res://assets/humans/rp_nathan_animated_003_walk/model.glb
                                         静态实例；transform = IDENTITY；**无 CollisionShape3D**
    └── (glb 内部) Armature (Node3D) → Skeleton3D(88 骨) → rp_nathan_animated_003_walking_geo (MeshInstance3D)
                   AnimationPlayer（1 条动画 "Armature|Take 001|BaseLayer"，2.2333 s）
```

```gdscript
# src/props/Person.gd（走动部分；静态部分见 §4.9，同一个文件）
extends Node3D

const WALK_GLB := "res://assets/humans/rp_nathan_animated_003_walk/model.glb"   # 与 .tscn 的 Body 逐字一致（自检锁）
const CLIP_MPS := 1.260              # 资产自带步速 = normalized/…/convert.json 的 speed_mps（滑步判据基准）
const SPEED_TOL_RATIO := 0.10        # 允许 ±10%，超了只在日志里提示"滑步风险"，不阻断
const FEET_TOL_M := 2.0e-3

@export var walk_speed: float = 0.0           # m/s；<=0 = 不走（默认静态）
@export var path: Array[Vector3] = []         # 世界坐标（Z-up 米），隐式闭合
@export var loop: bool = true                 # true=回绕；false=停在末点

func Configure(spec: Dictionary) -> void      # {path, walk_speed（别名 speed_mps）, loop, phase_m}
func Set_Progress_M(s: float) -> void         # 瞬移到指定弧长（渲染档取样用；结构档不靠它证明位移）
func Get_Progress_M() -> float
func Get_Path_Length() -> float
func Get_Speed_Mps() -> float                 # = walk_speed
func Get_Position_W() -> Vector3              # 世界坐标（Z-up）—— 自检据此与路径采样点逐位对比
func Get_Yaw_W() -> float
func Get_Body() -> Node3D
func Get_Anim_Player() -> AnimationPlayer
func Is_Anim_Playing() -> bool
func Path_Hits_Rect(origin_xy: Vector2, size_xy: Vector2, margin_m: float) -> bool   # 路径净空（slab 法）
func _ready() -> void                         # 建弧长表 → 找 AnimationPlayer → 播第 0 条 → 有路径则摆到 s=phase
func _physics_process(delta: float) -> void   # s = fposmod(s + walk_speed·delta, L) → 采样 → global_transform
```

**四条动画/推进硬规则**（导入器给的节点名会随版本变，所以一律不按名字硬编码）：

1. `AnimationPlayer` 先按 step.md 的字面调用 `find_child("AnimationPlayer", true, false)`
   （`owned=false` 才看得见 glb 实例**内部**的子节点），找不到再**按类型递归**兜底（`_Find_Anim_Player`）；
2. 播 `get_animation_list()` 的**第 0 条**，不硬编码 `"Animation"` / `"Take 001"`；
3. `loop_mode` 只在 `LOOP_NONE` 时设一次 `LOOP_LINEAR`，`speed_scale = walk_speed / CLIP_MPS`
   （改速度时步频跟着变，脚不滑）；`walk_speed <= 0` 时 `speed_scale = 1.0`（静止但动画不冻在第一帧）；
4. 只在 **`_physics_process`** 推进（`_process` 里不再推一次，否则速度翻倍）；
   找不到 `AnimationPlayer` / 动画列表为空时**只 `push_warning`（每实例一次）**，绝不 `push_error`
   （静态人形本来就没有 `AnimationPlayer`，push_error 会把 `--quit` 的 0 ERROR 验收搞成假红）。

**摆位**：`s → (pos_world, yaw_world)`，`yaw = atan2(d.y, d.x)`（世界 Z-up、逆时针为正），
一律 `global_transform = WorldTransform.MakeTransform(pos, yaw)` —— 本文件**不出现** `(x, z, y)` 换轴字面量。
`_physics_process` 用 `fposmod` 回绕（纯函数、无累积漂移），不用 Tween。

**路径（世界坐标，Z-up，米；隐式闭合）**：`World.WALKERS` 是唯一来源（**历史口径** ——
`WALKERS` / `BuildWalkers()` 已在 task_35 Step 3 整节删除，见本节 ⑧；现在世界只按 `PEDESTRIANS`
摆出生点与朝向，**没有任何路径**）。下表是 walking_person 时期的口径，保留作为资产/自检的说明。

| 名 | 路径 | 周长 | walk_speed | 相位 |
|---|---|---|---|---|
| `walker_01` | `(44,96) → (56,96) → (56,106) → (44,106)`（闭合回 `(44,96)`） | 44.0 m | 1.260 m/s（= `CLIP_MPS`） | 0 m |

> **选址依据（逐条对 `World.OBSTACLES` 实测）**：`wall_c` 在 `y∈[92,93]`、`wall_d` 在 `x∈[34,35]`、
> `block_1/2` 在 `y=100` 但 `x=60/66`、`block_3` 在 `(72,104)` → 环路与 15 个障碍**全部无交**
> （自检 `world-walkers-path-clear` 用 1.0 m 臂展余量直接断言）；距 spawn `(64,64)` 32 m、
> 距 `person_01 (60,76)` 20 m。**刻意放在 `y≥96`**：既有拍照机位在世界 `(60..68, 76)` 朝 −X，
> 扫不到这里 → 不污染 `check_person_shots` / `check_camera_shots` 的既有基线。
> **机位注意**：想拍这个环路**不能**从正南平视 —— `wall_c`（30 m 长、2 m 高）正好挡在中间
> （实测整帧拍空）。可用机位：世界 `(37,101)` 光心 1.6 m 朝东（`check_person_walk.gd` 用的就是这个）。

**① 零碰撞**：与 §4.9 同一条铁律（E27）。`Walkers` 子树里（**历史口径**：task_35 Step 3 起世界不再建
`Walkers`，改成**有**胶囊碰撞的 `Pedestrians`，见 §4.11.1）`CollisionShape3D` / `CollisionPolygon3D` /
`PhysicsBody3D` / `Area3D` 计数恒为 0（`check_person_walk` 的 `scene-no-collision` 锁）；
`check_lidar` 的 `F7/F8` 逐字不变（48 项）。

**② 与既有静态人物的关系：并入，不新开第二份脚本（Step 3）。** 走动能力搬进 `Person.gd` 后，
`PersonWalking.gd` 删除；`Person.tscn` / `Person.gd::MODEL_GLB` / `World.PERSONS` 的摆位与资产
**一个字都不改**（`check_person_assets` 的 32 项把高度 / 三角数 / sha256 逐位钉死，仍然全绿）。
两件资产、两个容器（`Persons` / `Walkers` —— 后者 task_35 Step 3 起换成 `Pedestrians`）、
两条 `.tscn` 各自独立，**互不影响**。

**③ 资产口径**（细节见 `assets/humans/README.md` 与 `tmp/walking_person/downloads/README.md`）：
Renderpeople Free **Animated** People（下载名 `rp_nathan_animated_003_walking`，**仓库目录 `assets/humans/rp_nathan_animated_003_walk/`**）
→ 剥掉根位移、压到 **1.84 m**、
脚底 `y≈0`、人朝 **+X**，转成**带 skin + 1 条动画**的 glb：`3 263 492 B` / `21 506 tri` / `88 骨` /
`duration 2.2333 s`（68 键 @30 fps；源首帧是零位 T-pose，已裁）/ 原地循环接缝 `0.1145 m`（`ratio 0.97`）/
`POSITION` 跨度 `0.3425 × 1.8490 × 1.8722`（x < z = 朝 ±X 的弱几何证据）。
最省事的复现路径：`tmp/walking_person/tools/w_convert.py`（Blender 4.5 `-b`，十步全在文件头注释里）。

**④ 导入副产物（§9.1 E28 同款）**：`model.glb.import` + 抽出的 `model_rp_nathan_animated_003_dif.jpg` +
`model_rp_nathan_animated_003_norm.png` + 各自 `.import` 必须与 `.glb` 一起在盘上（共 **6 个文件**），否则重导会
`ERROR: Cannot open file`、贴图全丢。资产目录在 Step 3 改名为 `rp_nathan_animated_003_walk/`（只改目录名，
`glb` 字节与 `sha256` 不变），旧 `.import` 的 dest hash 按资源路径算 → **必须删掉旧 `.import` 重跑 `--import`**
（`animation/import` 等 params 段与改名前的备份逐行相同）。

**⑤ 自检**：`src/tests/check_person_walk.gd` —— 结构 20 项（资产 8 / 场景 4 / 世界 3 / 运行 5）+
渲染 6 项（三帧 JPEG / 非全黑 / 投影框在动 / 框内像素在变）。headless `total=20 failed=0`；
`DISPLAY=:99` 下 `total=26 failed=0`。**（task_35 Step 3 起改为 15 项、两档同值** ——
D 组运行 5 + R 组渲染 6 + C 组旧 3 共 11 项被移除、C 组换成"世界预置行人"3 项，理由见本节末尾
**⑧**；本条与 ⑥ 记的是 walking_person 时期的历史数字。）运行组（5 项）在 `--fixed-fps 60` 下**手动推进物理帧**：
`run-anim-playing`（在播 + 骨骼姿态逐帧在变）、`run-progress-monotonic`（**逐帧** `Δs>0` 且
步长 ≤ `walk_speed/60×1.5`）、`run-lap-close`（真跑 2096 帧走满一圈回起点 ≤0.05 m，不用 `Set_Progress_M` 掺假）、
`run-path-dev`（节点位置 == 独立实现的路径采样点 ≤1 mm）、`run-hip-drift`（髋骨相对节点水平偏移 <1 m——
root motion 没剥干净会漂 2.9 m）。三帧的**真推进**版见 ⑦（`check_person_walk_shots.gd`）。

**⑥ 渲染档为什么只走半圈**：一整圈 2096 个物理帧在软件 OpenGL 下 >70 s（实测未完成），会把 300 s 验收窗口吃掉；
故 `DisplayServer.get_name() != "headless"` 时跑 **1/2 圈**测单调性，再用 `Set_Progress_M(L)`（等价 `s=0`）
验证闭合；整圈回起点由 **headless 档真跑**证明。两档都会打印实际帧数与圈数（`mode=headless-full-lap` /
`mode=render-half-lap`），不会出现"看起来跑了其实没跑"。
**⑦ 三帧"真推进"证明「人在走」（Step 4，`src/tests/check_person_walk_shots.gd`，14 项）**：
⑤ 的三帧是 `Set_Progress_M()` **瞬移**出来的，只能证明"三个位置不一样"；⑦ 用 `await physics_frame`
**逐帧真推进** 0 / 40 / 80 物理帧再抓拍，**帧间绝不瞬移**（源码 lint 锁死：全文只允许 1 处归零调用）。
- **机位**（世界 Z-up 米，只摆一次）：光心 `(48, 101)` 地面 + `1.50 m`，瞄 `(44.8, 96, 0.92)`。
  人在 `(44,96)` 朝 **+X**（yaw=0），机位在 `(+4, +5)` → 前方 4 m / 左侧 5 m、夹角 `51°` → **左前 3/4 面**
  （既看得到步态也看得到正脸），且 1.68 m 的走位主要**横向**投在画面上（不会被"迎面走近"吃掉）。
  `y=101` 在环路内侧，与 `wall_c`（`y∈[92,93]`）隔 8 m —— 避开 ⑥ 记录的"正南拍空"。
- **判据**：三帧 JPEG（`>10 KiB` / `ffd8ff` / SOF0 1280×720 / 灰度 `var>0.002`、`0.05<mean<0.95`）、
  机位三次逐位相同（≤1e-9）、AABB 8 角点全在画面内且框高占比 ∈[0.15,0.45]、
  **物理帧差 40±2 且 Δs=0.84±0.05 m 且 Δs/Δframes == v/60（±2%）**、
  框**水平**中心位移 `≥30 px`、框内灰度差 `≥0.02` 且 `≥3×` 框外、框裁到同一 64×64 后 `mean_abs ≥0.010`
  （最后一条是**平移不变量**：把"整体平移"消掉，剩下的只能是姿态变化）。
- **实测**（`DISPLAY=:99`，`logs/s4_check_person_walk_shots_render.log`，`total=14 failed=0`）：
  `s = 0.000 / 0.840 / 1.680 m`（`frames = 0 / 40 / 80`，`Δs/Δframes = 0.021000 = v/60`，偏差 **0.00%**）；
  框 `P(523.7,276.2) S(119.2,173.0)` → `P(583.0,273.2) S(114.7,187.9)` → `P(654.8,269.5) S(107.2,206.2)`；
  `Δx = 57.0 / 68.1 / 125.1 px`；`in/out = 0.0931/0.0001 (1020×)`、`0.0659/0.0002 (422×)`、`0.0872/0.0002 (566×)`；
  64×64 裁剪差 `0.06871 / 0.06780 / 0.08183`。
- **跑法**（**绝不能加 `--headless`**；headless 冒烟只跑 A 组 4 项）：
  `DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1 timeout 240 godot --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_person_walk_shots.gd`
  → `total=14 failed=0`；`timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_person_walk_shots.gd`
  → `[skip]` ×10 + `total=4 failed=0`。产物：`tmp/walking_person/shots/walk_t{0,1,2}.jpg` + `walk_report.md`。

**⑧ task_35 Step 3 起：世界不再实例化 `PersonWalking.tscn`（自动绕圈已移除）。**
`World.WALKERS`（44 m 闭合矩形 + `walk_speed` + `loop` + `phase_m`）/ `BuildWalkers()` / `GetWalkersRoot()`
与 `Walkers` 容器**整节删除**，换成 §4.11 的 `PEDESTRIANS` / `BuildPedestrians()` / `GetPedestriansRoot()`
（只给出生点 + 朝向，**没有任何路径 / 巡逻 / 计时**）；没有外部驱动时人**站着不动**（idle = 预期，不是故障）。
本节的走动能力（`Person.gd` 的 `walk_speed` / `path` / `loop` / `Set_Progress_M`）与资产**一个字都没删**：
`PersonWalking.tscn` 仍被本节的 A/B 组自检与 `assets/humans/README.md` 引用，
而且 `Pedestrian.tscn` 的 `Body` 就是同一件 glb。

随之调整的两个自检（**数字见 §11 的 `task_35-3` 行**）：
`check_person_walk.gd` **20 → 15**（删 D 组运行 5 / R 组渲染 6 / C 组旧 3，加 C 组新 3）；
`check_person_walk_shots.gd` **4 / 14 不变**，但主语从 `walker_01` 换成 `World.PEDESTRIANS[0]` 的 `pedestrian_01`、
驱动从 `Set_Progress_M()` 瞬移换成**一次** `Pedestrian.SetCommand({"v": 1.26, "dir": 0.0})`，
R7 的尺子从"弧长 Δs"换成"`GetPose().x` 的 Δx"。机位几何（世界 `(48,101)`+1.5 m 瞄 `(44.8,96,0.92)`）
**逐位不变** —— 因为 `pedestrian_01` 的出生点 `(44,96)` 就是旧环路首点。

---

### 4.11 `src/scenes/props/Pedestrian.tscn`（task_35 Step 1 ✅）

**定位**：行人 = 世界里的一个**可动物体**，不是 Pleiades 节点。它挂在 `ChassisCatalog` 里（Step 1 只登记），
用**与车同一套** `chassis_hello` / `motion_cmd` / `pose` 协议被外部脚本驱动；`peer_id` 由客户端自定义
（如 `human-01`），**不参与任何 P2P 广播**。

```
Pedestrian (CharacterBody3D)          collision_layer = 2 , collision_mask = 5 (1|4)
  ├─ CollisionShape3D                 CapsuleShape3D r=0.25 / h=1.75 / position y=0.875
  └─ Body                             res://assets/humans/rp_nathan_animated_003_walk/model.glb
                                      （内含 Skeleton3D 88 骨 + AnimationPlayer 1 条 clip；**不再另建** AnimationPlayer）

# src/chassis/Pedestrian.gd   (extends "res://src/chassis/ChassisModel.gd")
const CHASSIS_NAME := "pedestrian"    /  const CLIP_MPS := 1.26
func GetDefaultParams() -> Dictionary            # {"max_speed": 1.26, "max_turn_rate": 180.0}（与目录表逐键一致）
func Configure(vehicle, params) -> void          # 读参 -> 状态清零 -> GROUNDED；**不调** ApplyBodySize()
func SetCommand(cmd: Dictionary) -> void         # v / dir 独立解析；缺省回退 action/speed
func Step(velocity, delta) -> Dictionary         # 纯控制量 {velocity(全局), yaw_rate(rotation.y 角速度)}
func Play_Walk() -> void                         # 播第 0 条 clip、loop、speed_scale = v / CLIP_MPS
func _physics_process(delta) -> void             # 自宿主：Step -> rotation.y += yaw_rate*dt -> move_and_slide()
func GetPose() -> Dictionary                     # {x, y, z, yaw}，口径同 Vehicle.GetPose()（§7.3）
func Get_Speed() / Get_Dir_Target_W() / Get_Yaw_Rate_Limit() / Get_Body()   # 只读查询
```

- **指令语义**（`motion_cmd`，Step 2 起由 manager 转发）：`v` = 线速度 m/s（**clamp 0~1.26**，行人倒着走一律当 0）；
  `dir` = **世界系朝向角（弧度，0 = +X 东，逆时针为正）**，内部存 `rotation.y = -dir`。
  `v` 与 `dir` **各自独立**：`{v:1.2}` 只改速度（保持朝向直走）、`{dir:1.57}` 只改朝向（边走边转）。
  缺 `v`/`dir` 时回退老协议：`forward` → `v = 1.26×speed/100`；`left`/`right` → **只改朝向**（`v` 不动）；
  `stop` / `backward` / 未知 → `v = 0`。
- **运动学**：转向是**限速**的（`max_turn_rate` = 180 °/s，`params` 单位是**度/秒**，内部 rad/s），
  沿**当前** yaw 前进（与 `Vehicle` 的 1 帧滞后同款）。**没有加速度斜坡**（与 `DiffDrive` 的 `Approach()`
  不同，是有意为之）：`v=0` 当帧归零、`dir` 阶跃第一帧就饱和到 180 °/s。
- **自宿主**：`Step()` 只产出控制量，**位置只由 `_physics_process()` 里的 `move_and_slide()` 推进**（唯一位置更新点）。
  宿主是别的节点（`Configure` 传的不是自己）时 `_physics_process` **不推进**，避免双推。
- **动画**：`speed_scale = v / 1.26`（与动画自带步速同值 → 脚不滑）；`v = 0` 时 `speed_scale = 0`
  （人物定格，仍 `is_playing()`）。找不到 `AnimationPlayer` / clip 为空**只 `push_warning`（每类一次）**，
  **绝不 `push_error`**（`--quit` 的 0 ERROR 验收靠这条）。
- **碰撞口径**：`layer=2` → 雷达 `scan_mask = 1|2|4` 能扫到它；`mask=1|4` → 只与地面 + 车辆碰撞，
  **不与 VoxelWorld（layer 2）碰撞**（人走不进建筑碰撞体，Step 3 靠出生点选点避开障碍，见 §6 R2 记录）。
  车辆 `mask = 7` 含 layer 2 → **车会被人挡住**。
- **`.tscn` 手写维护**，不要用编辑器另存（会丢头注释，§9.1 E8）；`Body` 变换必须是恒等（模型原点 = 脚底中心）。
- **自检**：`check_pedestrian.gd` **40 项**（`total=40 failed=0`）。跑法：
  `timeout 200 godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_pedestrian.gd`
- **本 step 的过渡 gap（如实记录）**：Step 1 只登记目录，`WebSocketManager` 仍把它当 robot 走 `_RegisterChassis`；
  真正按 `kind="pedestrian"` 分流到 `World/Pedestrians` 是 Step 2/Step 3 的事。

#### 4.11.1 世界预置行人（task_35 Step 3 ✅）

`World.gd` 的 `PEDESTRIANS` 常量表在 `_ready()`（`BuildPersons()` 之后）把 **2 个人**摆进 `World/Pedestrians`：

| 名 | 世界坐标（Z-up，米） | 初始朝向 | 说明 |
|---|---|---|---|
| `pedestrian_01` | `(44, 96, 0)` | `yaw_deg = 0`（朝 **+X 东**） | 最近障碍 = `wall_c` **3.000 m**；`check_person_walk_shots` 的被拍对象（机位世界 `(48,101)`+1.5 m） |
| `pedestrian_02` | `(52, 102, 0)` | `yaw_deg = 180`（朝 **−X 西**） | 最近障碍 = `block_1` **8.062 m**；`yaw≠0` 是 `Configure()` 对齐逻辑的活体证据 |

两人相距 **10.000 m**。选点判据（全部由 `check_person_walk.gd` 的 `world-pedestrians-spawn-clear` 逐条断言）：

- **不压障碍**：与 15 个 `OBSTACLES` 矩形外扩 `0.5 m`（胶囊 0.25 + 0.25 余量）**无交**；
- **离世界边界** ≥ `1.0 m`（实测 32 / 26 m）；
- **离出生点** `SPAWN_WORLD (64,64)` > `SPAWN_CLEAR_RADIUS_M (3 m)`（实测 37.736 / 39.850 m，也远于 manager 的 8 方位 × 2 m/环排位）；
- **离静态两人** `PERSONS(60,76)/(68,70)` ≥ `2.0 m`（实测 25.6 / 27.2 / 35.4 / 35.8 m —— 不污染 person 系列取景）；
- **两人互距** ≥ `1.0 m`；
- **离雷达/拍照热区 > 12 m**（人 `layer=2` 会被 `Lidar2D` 的 `scan_mask=1|2|4` 扫到）：`check_lidar` 真墙热区 `(60,90)` **17.088 / 14.422 m**、空场热区 `(100,60)` **66.573 / 63.781 m**、`check_camera_shots` 停车点 `(60,84)` **20.000 / 19.698 m** → F7/F8 的 `2.0000 m` / `152` 逐字不变；
- **离协议 spawn 点 > 30 m**（默认 `(64,56)` 44.7 / 47.5、K1 `(70,70)` 36.8 / 36.7、K16 `(72,78)` 33.3 / 31.2）→ 动态行人与世界行人**不会叠人**。

**idle 语义（本 step 最重要的一条）**：`BuildPedestrians()` **不调 `SetCommand`、不设速度、不建路径**。
没有外部驱动时人**站住不动**（`_speed_now = 0` → `Play_Walk()` 把 `speed_scale` 置 `0.0`，动画定格但仍 `is_playing`）。
**"世界里的人不动"是预期行为，不是故障。**

**`Configure(self, {})` 必须在写完 `global_transform` 之后调**（`World.gd` 里就是这个顺序）：
`Pedestrian.Configure()` 末尾执行 `_yaw_target = rotation.y`；漏调它，`yaw_deg = 180` 的 `pedestrian_02`
第一帧就会被 `Step()` 拧回 `yaw = 0`（plan §7 R1）。

**世界预置行人 vs 协议行人（职责划分，别混）**：

| | 世界预置行人（本节） | 协议行人（§4.5.1-⑩，task_35 Step 2） |
|---|---|---|
| 来源 | `World.PEDESTRIANS` 常量表，`_ready()` 里建 | `chassis_hello{kind:"pedestrian", spawn?}`，manager 建 |
| 身份 | **无** `peer_id`、无 `chassis_ready`、不进 `_pedestrians` 表、**不参与任何网络** | 有 `peer_id`，10 Hz `pose` 回传，300 ms 无指令自动停车 |
| 运动 | 站住不动，直到有人在进程内 `SetCommand`（自检 / Step 5 的车撞人） | 由外部客户端连续 `motion_cmd{v,dir}` 驱动 |
| 容器 | `World/Pedestrians/pedestrian_0x` | `World/Pedestrians/<peer_id>`（**同一容器、不同名**） |

> **本 step 明确不做**：让外部客户端"认领"世界预置的那两个人需要给 `chassis_hello` 加"按名字认领"语义 ——
> 那是协议面的增量（Step 2/4），不在本 step（世界只负责布局）。**判据**（`check_person_walk.gd` 的 C 组 3 项）：
> `world-pedestrians-count`（容器名 / 父节点 / 孩子数 / 孩子名 / 场景 / 脚本 / 底盘契约面 `SetCommand`+`GetPose`）、
> `world-pedestrians-spawn-clear`（上面 5 条判据）、`world-pedestrians-idempotent`（`BuildPedestrians()` 连调两次
> 节点总数不变、同名容器恰 1 个、孩子数不变）。

**两条已知取舍（照实写）**：① 人**会穿建筑**（`mask` 不含 VoxelWorld 所在层，Step 1 既定），本 step 靠出生点选点避开；
② 运行期重建 `Pedestrians` 容器会**连带移除**已装配的网络行人（manager 的缓存指针因 `is_instance_valid()==false`
自动按名重解析，不会悬空，但那个人就没了）→ `BuildPedestrians()` 只应在 `_ready()` 与自检里调用。

#### 4.11.2 端到端验收（task_35 Step 5 ✅）

**定位**：Step 1~4 的产出物在这里**全部当黑盒**用 —— 本 step **只新增两个自检脚本 + `tmp/task35/` 下的驱动脚本与报告**，
生产代码（`World.gd` / `Pedestrian.gd` / `WebSocketManager.gd` / `WireProtocol.gd` / `Lidar2D.gd` /
`Vehicle.gd` / `ChassisCatalog.gd` / `src/scenes/**` / `project.godot` / `assets/**` / `tool/pedestrian/**`）**一个字节没改**。
六项验收的逐项实测在 `tmp/task35/e2e_report.md`，第 1~3 项指标表在 `tmp/task35/s5_metrics.md`。

| # | 场景 | 判据 | 落地 | 实测（本轮） |
|---|---|---|---|---|
| 1 | Python 驱动**直行** 10 s | 位移 ≈ `v×t`（±10%）、`yaw` 误差 < 2° | `tmp/task35/e2e_drive.py --phases straight` | `dtw=10.100 s` / `\|d\|=10.102 m` / `v_eff=1.0003 m/s`（**+0.03%**）/ `yaw_max=0.000°` / `Δy=0.0000 m` |
| 2 | Python 驱动**转弯**（`dir` 阶跃 0→π/2） | `yaw` 以 ≤180 °/s 收敛、轨迹是弧线 | 同上 `--phases turn` | `rate=180.0 °/s` / `conv=0.600 s` / Kasa `r_fit=0.3199 m` vs `v/ω=0.3183 m`（**+0.5%**）/ 弧长差 −2.5% |
| 3 | **停发 300 ms** 后 `v→0` | 位置不再变化（两次采样 < 1 mm） | 同上 `--phases timeout` | `stop_latency=0.384 s`（墙钟）/ 冻结 1.100 世界秒位移 **0.000000 m** / 末条 `v=(0,0)` / 世界日志 `pedestrian idle` **恰 1 行** |
| 4 | **人撞车**（车静止，推 300 物理帧） | 不穿透 / 被挡停或滑开 / **车位移 < 1 mm** | `check_pedestrian_physics.gd` P 组 8 项 | `gap_min=0.019952 m`（逐帧 min ≥ −1 mm，= 一帧步长）/ **挡停**（最后 60 帧前进 0.000000 m）/ `moved=0.000000000 m`、`Δyaw=0` |
| 5 | **雷达可见**（顺带，不做建图断言） | ±45° 扇区最近 ≈ 车头→胶囊表面（±5 cm）且 < 3 m | 同上 L 组 7 项 | `d=2.2500 m`（= 2.50 − 0.25，差 **0.0000**）/ `ranges[0]=2.2500` / 扇区 11 条命中（≈11.5° 张角）/ 挪到 10 m 后 `9.7500 m` |
| 6 | **走 / 转 / 停三帧**真渲染 | 非全黑、三张互不相同、人物框位置/姿态有变化 | `check_pedestrian_shots.gd`（Xvfb 档 14 项） | 31 123 / 31 317 / 32 122 B、SOF0 均 1280×720、框水平中心 **74.2 / 141.6 / 67.5 px**、框内灰度差 0.068~0.100、`yaw=90.00°`、`speed_scale=0.000` |

**跑法**（三条命令，见 §1.4 的 "行人端到端验收" 段）：自检脚本 `--headless --fixed-fps 60`；
三帧真渲染 `--display-driver x11 --rendering-driver opengl3 --audio-driver Dummy`（**绝不加 `--headless`**）；
Python 侧 `bash tmp/task35/s5_e2e.sh`（世界 `--headless --max-fps 60`，**绝对不带 `--fixed-fps`**，见 E29）。

**这四条实测事实是本节的核心，下一轮别推翻**：

1. **`--fixed-fps` 只给"自己数帧"的自检用**：它在 headless 下会关掉实时同步，实测跑到 **21~37 倍速**
   （10 s 墙钟 = 360 s 世界时间，人一路撞到东侧空气墙，`|d| ≈ v·t` 必红）。实时档只有 `--max-fps 60`。
2. **时间基必须分开**：速率 / 位移一律用 pose 的 `world_time`；**只有** 300 ms 超时那条用墙钟
   （世界侧 `_Tick_Pedestrian_Timeouts` 用的就是 `Time.get_ticks_msec()`），且只在实时档里比较。
3. **接触判据要带离散步长**：1.2 m/s @ 60 fps = 0.02 m/帧，`move_and_slide` 在"整步都会穿透"的那一帧就停住
   ⇒ 静止间隙必然 ∈ [0, 0.02]（实测 `0.019952`，恰好一整步）。所以"碰到了"的阈值取 `PUSH_V/60 + 5 mm`，
   而"不穿透"另用 `GAP_MIN_M = −1 mm`（`safe_margin`）兜底；想量**真实**接触距离就再用 `0.05 m/s` 贴实 60 帧
   （实测 `0.1508 m` vs `CAR_HALF_X = 0.15`）。
4. **雷达必须装车头**（`mount.x = 0.15`、`mount.z = 0.6`）：`Lidar2D` 量的是"雷达到命中点"，
   而判据是"**车头到**胶囊表面"；装原点差 0.15 m > ±5 cm 必红。高度 0.6 m 落在胶囊圆柱段
   （Godot y ∈ [0.25, 1.50]，横截面恰好 r = 0.25），再低会掉进下半球白吃 5 mm 容差。

**第 6 项的机位**（本 step 唯二动过的常量，`CAM_EYE_W` / `CAM_AIM_W`）：`(48.37, 96.79)` 抬 1.50 m、瞄 `(44.9, 96.5, 1.0)`。
旧机位 `(48,101)` 的视线与走位几乎重合（位移落在**深度**上），三帧框水平中心只差 4.1 / 13.6 / 17.7 px；
换到"路径侧面"后投出 74.2 / 141.6 / 67.5 px，且"走"是正脸（夹角 12°）、"转"是侧身（86~92°）。

**回归**：`bash tmp/task35/s5_checks.sh` → `FAILS=0 / ALL CHECKS GREEN`
（`--quit` 冒烟 + **21 项 headless** + **5 项 Xvfb**）。既有 19 项的 `total` 与历史基线逐字一致
（`check_ws_manager` 以本文 §1.4 与 `tmp/task35/logs/review4_ws_manager.log` 的 **76 / 77** 为准，
不是 plan.md §2.4 表里那个过期的 73）。两个新脚本依次是 **22**（无 skip）与 **4 / 14**（headless / Xvfb）。

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

组件场景自身的静态 instance（task_33 Step 3，随组件加载、无运行时代码）：
Vehicle.tscn
  └─ ModelUGV   instance res://assets/chassis/ugv_chassis.glb            → Vehicle/ModelUGV（visible=false，装配后显示）
Lidar2D.tscn
  └─ Body       instance res://assets/sensors/lidar_tmini.glb            → <slot>/Body（identity，随场景可见）
CameraSensor.tscn
  └─ Body       instance res://assets/sensors/camera_module.glb          → <slot>/Body（rotation_degrees=(0,90,0)）
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

卸载（Step 6 实际顺序；第 2 轮改成车/人**共用** `_RemoveAvatar()`）:
  chassis link 断 → _RemoveAvatar（只有 link.vehicle != null 时才摘）：先从**两张表** erase（`_peer_vehicles` 与 `_pedestrians`，再 free，防同帧二次访问）
                    → remove_child + queue_free(avatar)
                    → 关闭该 peer 其余全部 link（1001 "peer removed"），它们一律交出 `link.vehicle`
                    → 清 _manifests / _manifest_parts / _assembly_sent / _pending_sensors
                    → 只在**真摘到**时才 emit `vehicle_removed`/`pedestrian_removed` + 打一行日志
  sensor  link 断 → vehicle.DetachSensor(slot) + manifest[slot] = false + 从 pending 队列摘除
  同一 peer_id 重复 chassis_hello（**只在该连接是底盘连接时**；**含跨 kind**）→ 先 _RemoveAvatar(peer, except_link=本连接)
                    再新建；except_link 保证刚接手的连接不被误关，两张表都空时立即早退（静默）
  未握手连接超时 → 关（1002）；pending 等待底盘超时 → 关（1008）
  assembly_ready 发出后，部件断开再连**不重发**（_assembly_sent 每 peer 只置一次）
```

**时序铁律**：`sensor_ready` 一定先于该传感器的第一条二进制帧（否则 sim 侧会因未知 `slot_id` 丢包）。
> 实现说明：`Close()` 只置状态，真正的移除留到**下一帧** `_SweepClosed()`（遍历中不删数组）；`WebSocketManager._process(delta)` 每帧顺序固定为 accept → poll → sweep → **行人超时停车** → broadcast（`_Tick_Pedestrian_Timeouts()`（task_35 Step 2，只对 `_pedestrians` 生效）与 `_BroadcastPoses(delta)`（task_31 Step 1）；先清掉本帧已判死的连接，再判超时，最后下发位姿）。

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
const FRAME_TYPE_CAMERA := 2      # task_34 Step 3 落地（payload = JPEG 原始字节）
const FRAME_HEADER_SIZE := 15
const LIDAR_SAMPLE_BYTES := 4     # 每个距离 f32

const KIND_ROBOT := "robot"           # task_35 Step 2：chassis_hello.kind 的缺省值
const KIND_PEDESTRIAN := "pedestrian" # task_35 Step 2：行人（不进 Vehicles / 不占 avatar 排位）

static func ParseJson(bytes: PackedByteArray) -> Dictionary           # 失败返回 {}，不抛、**不打印**
static func MakeJson(msg: Dictionary) -> PackedByteArray              # JSON.stringify(...).to_utf8_buffer()
static func MakeJsonText(msg: Dictionary) -> String                   # send_text() 用
static func NormalizeKind(raw: Variant) -> String                     # task_35 Step 2：strip+lower，非 robot/pedestrian -> ""
static func BuildMotionCommand(v: float, dir_rad: float = NAN, seq: int = 0) -> Dictionary
                                                                      # task_35 Step 2：{msg, seq, v[, dir]}；**不含** action/speed
static func BuildPose(world_time: float, pose: Dictionary, velocity: Vector3) -> Dictionary
static func BuildLidar2DFrame(sensor_id: int, seq: int, world_time: float,
        ranges: PackedFloat32Array) -> PackedByteArray
static func BuildCameraFrame(sensor_id: int, seq: int, world_time: float,
        jpeg: PackedByteArray) -> PackedByteArray                     # task_34 Step 3
static func ParseFrameHeader(bytes: PackedByteArray) -> Dictionary     # 短包(<15 B) -> {}；否则 {type, slot_id, seq, world_time}
static func FramePayloadSize(bytes: PackedByteArray) -> int            # max(size - 15, 0)
```

实现约定（Step 6 实测）：

- `ParseJson` 用 `JSON.new().parse()`（返回 `Error`）而不是 `JSON.parse_string()`：后者遇非法输入会向 **stderr 打印 `ERROR: Parse JSON failed`**，污染「grep ERROR 必须为空」的验收（§9.1 **E21**）。解析失败 / 非 `Dictionary`（数组、标量、null）一律 `{}`。
- `BuildLidar2DFrame`：先 `resize(15)` 编头（`encode_u8(0,…)` / `encode_u16(1, sensor_id & 0xFFFF)` / `encode_u32(3, seq & 0xFFFFFFFF)` / `encode_double(7, world_time)`），再 `append_array(ranges.to_byte_array())`；长度 = `15 + 4·rays`（360 线 = **1455 B**）。
- `ParseFrameHeader` 的短包语义 = `{}`（调用方据此丢弃整帧，不部分解析）。
- `BuildCameraFrame`（task_34 Step 3）：**同一套 15 B 帧头**（`type=2` / 同样的字段偏移 / 同样的小端），
  payload = **JPEG 原始字节**——不重编码、不加长度前缀、不加分隔符；长度 `15 + N`（N = `jpeg.size()`），
  宽高从 JPEG 的 `SOF0` 段读（不上线，与 lidar 的"角度不上线"同理）；空 payload 时长度恰为 15，
  **调用方应自行丢弃**（sim 侧按短包忽略）。解析侧复用 `ParseFrameHeader()` / `FramePayloadSize()`，无需新增函数。
- `BuildPose`（task_31 Step 1）：固定 **8 键** `{msg, world_time, x, y, z, yaw, vx, vy}`，`z` **必带**（UGV 0 / UAV 2.5）；`velocity.z` 丢弃（`vz` 不上线），不发 `valid` / `sub_gx` / `sub_gy` / `rtk_fixed`；`velocity` 由**调用方**传 `WorldTransform.VelToWorld(vehicle.velocity)` 的结果（本文件**不** preload `WorldTransform`，保持"只做协议事实"的分层）；`pose` 缺 key 时四字段回 `0.0`（不崩）。
- `NormalizeKind` / `BuildMotionCommand`（task_35 Step 2）：**纯协议事实**，不打日志、不查场景。
  `NormalizeKind()` 只认 `robot` / `pedestrian`（strip + lower），缺省 / 空 / 非字符串 / 其它值一律返回 `""`，
  由 **manager** warn 一次后按 robot 处理（分层：判断在 manager、字面量在协议层）。
  `BuildMotionCommand(v, dir=NAN, seq=0)` 返回 `{msg:"motion_cmd", seq, v}`，`dir` **只有当它是有限数时**才带上
  —— 超时停车就靠这个语义发"只改速度、不动朝向"的 `v=0`；**不写** `action` / `speed`（两套字段并存、互不覆盖，§8.1-9）。
- **只做协议事实**：不碰 socket、不碰场景树。

### 7.4 `ComponentLink`（`src/net/ComponentLink.gd`，`RefCounted`）

见 §4.5（`Init` / `Poll` / `TakePackets` / `SendJson` / `SendBinary` / `NextSeq` / `IsOpen` / `IsConnecting` / `Close` / `Describe`）。

补充（Step 6 实测）：

- `Close(code, reason)` **返回 `void`**（§9.1 **E19**）：写成 `var e := peer.close(...)` 会直接 Parse Error。
- `Poll()` 顺序写死：`peer.poll()` → **先抽包**（`STATE_OPEN`/`STATE_CLOSING` 都抽）→ 再看 `STATE_CLOSED` 置 `closed`；先关后抽会丢最后几帧。
- `last_error` 记关闭原因 / 对端关闭码（`get_close_code()` / `get_close_reason()`），供 manager 一行日志。
- `SendJson` / `SendBinary` 未 OPEN 时返回 `ERR_UNCONFIGURED`（不崩、不打印）。
- **收发缓冲 4 MiB（task_34 Step 3）**：`Init()` 里 `peer` 建好之后、`accept_stream()` **之前**，把
  `peer.inbound_buffer_size` / `peer.outbound_buffer_size` 都设为 `MAX_FRAME_BUFFER_BYTES = 4 MiB`
  （`WebSocketPeer` 默认只有 **64 KiB**，一张 720p JPEG 就能撑爆）。顺序是硬约束：连上后再改对已分配的
  wslay 缓冲无效。代价 = 每连接常驻 2 × 4 MiB（世界侧连接数 = sim 数，个位数，可接受）。
  自检 `check_ws_manager.A10` 用 `Init(null, 99)`（`null tcp` 分支在 `accept_stream` 之前 return）直接断言属性值。

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
# src/chassis/ChassisCatalog.gd —— 不声明 class_name；Step 5 ✅ / task_35-1 加 pedestrian
const SCENES := {
    "diff_drive_ugv": "res://src/scenes/chassis/DiffDrive.tscn",
    "quadrotor_uav":  "res://src/scenes/chassis/Quadrotor.tscn",
    "pedestrian":     "res://src/scenes/props/Pedestrian.tscn",     # task_35 Step 1
}
const DEFAULTS := {
    "diff_drive_ugv": {"max_speed": 1.5, "max_turn_rate": 90.0,
                       "wheel_base": 0.30, "track_width": 0.25, "height": 0.25},
    "quadrotor_uav":  {"max_speed": 2.0, "max_climb_rate": 1.0,
                       "max_yaw_rate": 90.0, "size": 0.40},
    "pedestrian":     {"max_speed": 1.26, "max_turn_rate": 180.0},  # task_35 Step 1
}
static func Has(p_name: String) -> bool                # 目录外名字 = false（不 load、不报错）
static func GetScene(p_name: String) -> PackedScene    # load() + 缓存；不存在返回 null 并 push_error
static func GetDefaults(p_name: String) -> Dictionary  # 副本（duplicate(true)）；未登记 -> {}
```

`SensorCatalog` 同构：

```gdscript
const SCENES := {
    "lidar_2d": "res://src/scenes/sensors/Lidar2D.tscn",
    "camera":   "res://src/scenes/sensors/CameraSensor.tscn",       # task_34 Step 3
}
const DEFAULTS := {
    "lidar_2d": {"range": 12.0, "rays": 360, "rate_hz": 10.0},
    "camera":   {"width": 640, "height": 480, "fov_deg": 70.0, "quality": 0.8, "rate_hz": 1.0},
}
```

> **两个口径，不要混（task_34 Step 3 的关键取舍）**：`DEFAULTS["camera"]` 是 **640×480 @ 1 Hz**
> （装配线口径，对齐 UAV 相机默认、也对齐 task 文档硬要点 4），而组件常量
> `CameraSensor.DEFAULT_WIDTH/HEIGHT = 1280×720` 与 `CameraSensor.tscn` 的 `size` 是 **task_33 的既定基线**
> （组件口径，`check_camera_shots.gd` 的 `S14/R2` 9 项按它硬断言）。两者**不冲突也不互相同步**：
> 目录表是"新零件进装配线时的默认参数"，组件常量是"不经装配线直接实例化时的默认值"。
>
> **当前调用链的已知 gap（如实记录，不假装闭环）**：`WebSocketManager._MountSensor()` 至今**不读**
> `GetDefaults()`，只把 `sensor_hello.params` 原样交给 `Vehicle.AttachSensor()`。所以
> `DEFAULTS["camera"]` 是**契约 / 样例级**默认值（由 `check_camera_sensor.A18` 锁定其内容），
> 与 lidar 的 `DEFAULTS["lidar_2d"]` 处境相同；真正生效的参数由 sim 侧 `sensor_hello.params` 给
> （`check_ws_manager.J2` 用 640×480 的 params 证明"从配置可调"）。
>
> `"imu"` 仍然 `Has() == false`（`check_lidar.B1` 依赖），`camera` 的加入不改任何既有语义。

> **必须用 `load()` 而非 `preload()`**：Step 6（manager）先于 Step 7/8（场景文件）完成，`preload` 会因资源不存在导致整脚本解析失败。

> `DEFAULTS["diff_drive_ugv"]` 的 5 个值必须与 `DiffDrive.GetDefaultParams()` **逐键一致**（`check_diff_drive` 的
> `default-params` 断言直接比对两份字典）。task_33 Step 3 起其中的几何三项（`wheel_base 0.30` / `track_width 0.25` /
> `height 0.25`）= `assets/chassis/ugv_chassis.glb` 的 AABB（D11：模型 = 碰撞盒）。

> **`pedestrian` 是"特殊底盘"（task_35 Step 1）**：`DEFAULTS["pedestrian"]` = `{max_speed 1.26, max_turn_rate 180.0}`
> 必须与 `Pedestrian.GetDefaultParams()` **逐键一致**（`check_pedestrian` 的 `default-params` 断言三处一起比对：
> 对象 / 目录 / 期望字面量）。**只有两个键**——task 文档正文提到的 `turn_rate_deg` 别名**没有采纳**：
> 它与 `max_turn_rate` 同义，再加一个无消费者的键会让"逐键一致"这条不变量变成三处同步负担
> （理由记在 `tmp/task35/plan.md` §2.3 / §6 R7）。`pedestrian` 的 `SCENES` 指向 `src/scenes/props/`（不是
> `src/scenes/chassis/`），因为它带胶囊碰撞、自宿主推进，和"车体里的底盘零件"不是一回事（§4.11）。

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

// sim → world（行人首包，task_35 Step 2；kind / spawn **都是可选键**）
{ "msg": "chassis_hello", "peer_id": "human-01", "chassis": "pedestrian",
  "kind": "pedestrian",
  "spawn": { "x": 70.0, "y": 70.0, "z": 0.0, "yaw": 0.0 },
  "params": {}, "manifest": [] }

// world → sim
{ "msg": "chassis_ready", "peer_id": "12D3Koo...", "avatar_id": 7, "kind": "robot",
  "spawn": { "x": 64.0, "y": 64.0, "z": 0.0, "yaw": 0.0 }, "world_time": 12345.678 }

// world → sim（行人回执：kind + pedestrian_id，**没有** avatar_id；spawn 是校验后的真实值）
{ "msg": "chassis_ready", "peer_id": "human-01", "kind": "pedestrian", "pedestrian_id": 1,
  "spawn": { "x": 70.0, "y": 70.0, "z": 0.0, "yaw": 0.0 }, "world_time": 12345.678 }

// sim → world（20~50 Hz；老形态）
{ "msg": "motion_cmd", "seq": 1234, "action": "forward", "speed": 50 }

// sim → world（20~50 Hz；行人形态，task_35 Step 2；dir 可省略 = 只改速度、保持朝向）
{ "msg": "motion_cmd", "seq": 1235, "v": 1.0, "dir": 1.5707963 }

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
9. **`kind` / `spawn` / `v` / `dir`（task_35 Step 2，全部"只增不改"）**：
   - `chassis_hello.kind` ∈ `{"robot", "pedestrian"}`，**缺省 `"robot"`**（老 sim 一字不改照旧走车）。非法值（大小写 / 空白不敏感，其它任何字符串、数字、null）→ manager `push_warning` 一次后**按 robot 处理，连接不关**。`kind="pedestrian"` 要求 `chassis="pedestrian"`，否则 warn + 按 robot。
   - `chassis_hello.spawn` `{x, y, z, yaw}`：**只有 `kind="pedestrian"` 读**；缺省 = 合法（默认点 `World.SPAWN_WORLD + (0, -8, 0) = (64, 56, 0)`，yaw 0，不 warn）。有值但非法 → warn + 回退默认点：`x/y` 非有限数 / `x,y ∉ [0, 128)`（`cells_x × voxel_size`，走 `ProjectSettings`）/ `z ∉ [0, 2.0]` / 落在 15 个 `World.OBSTACLES` 内（外扩 0.3 m）/ 与已占 avatar（车 + 人）水平中心距 < 1.0 m。`yaw` 归一化 `[-PI, PI)`。robot 的 `spawn` 仍由世界按 §4.5.1-④ 决定（hello 里的 spawn 对车**无意义**、被忽略）。
   - `chassis_ready` 增 `kind`（两种 avatar 都带）与 `pedestrian_id`（仅 pedestrian，u16 语义，与 `avatar_id` **两套编号**）；pedestrian 回执里的 `spawn` 是**校验后**的真实出生点，静止时与 `pose` 逐字段相等。
   - `motion_cmd.v`（m/s，行人侧 clamp `0~1.26`）/ `motion_cmd.dir`（世界系朝向角，弧度，0 = +X 东，逆时针为正，可省略 = 保持原朝向）。**与 `action/speed` 并存、互不覆盖**：带 `v` 就按 m/s，带 `action` 才走老的百分比折算；`dir` 与 `v` 各自独立解析（`{v:1.2}` 只改速度，`{dir:1.57}` 只改朝向）。
   - **超时停车**：行人**超过 300 ms**（`PEDESTRIAN_CMD_TIMEOUT_MS`）没收到新 `motion_cmd` → 世界侧自动下发 `v=0`（**只停、不断连、不刷屏**，日志每段"动→停"一行）。**车没有超时**。
   - **同一 `peer_id` 换 `kind` = 重建 avatar**（第 2 轮）：注册前**不分类型** `_RemoveAvatar(peer_id, except_link=本连接)`（两张表 + 两个容器一起清，旧连接被关且置 `link.vehicle = null`）；`chassis_ready` **重发**、`assembly_ready` **不重发**；`_RemoveAvatar` 只在真摘到节点时才 emit `*_removed` / 打日志。细节见 §4.5.1-⑩。

### 8.2 二进制帧（LiDAR / camera，**小端**）

| offset | size | field | 说明 |
|---|---|---|---|
| 0 | 1 | `type` u8 | `1` = `lidar_2d`；`2` = camera（task_34-3 落地） |
| 1 | 2 | `slot_id` u16 | = `sensor_id`（`sensor_ready` 里下发的值） |
| 3 | 4 | `seq` u32 | 每连接独立递增 |
| 7 | 8 | `world_time` f64 | 秒，`WorldClock.Now()` |
| 15 | N | `payload`（lidar） | `f32[rays]` 距离（米，`0.0` = 无回波） |
| 15 | N | `payload`（camera） | **JPEG 原始字节**（不重编码、不加长度前缀） |

总长 = `15 + 4·rays`（lidar，360 线 → 1455 B）/ `15 + N`（camera，N = JPEG 字节数）。

- GDScript 侧 `PackedByteArray.encode_u16/encode_u32/encode_double` 与 `PackedFloat32Array.to_byte_array()` 在 x86/ARM 上均为小端，与 Rust 侧 `from_le_bytes` 对齐。
- **Rust 侧按本表小端解析，不要依赖主机序**。
- 角度不上线：`slot_id` + `seq` + `world_time` 已足够；`rays` 与 `range` 由 `sensor_hello.params` 静态约定。
- **camera 帧（`type=2`，task_34 Step 3）**：payload 就是 `CameraSensor.Capture_Jpeg()` 交出的那段字节，
  **不重编码、不裁剪、不加长度前缀**（长度由 WS 帧本身给出）；宽高不上线，sim 侧从 JPEG 的 `SOF0` 段读
  （与 lidar"角度不上线"同一取舍）；`type` / `slot_id` / `seq` / `world_time` 四个头字段与 lidar **逐字节同构**，
  所以 `ParseFrameHeader()` / `FramePayloadSize()` 一套代码通吃。
- 空 payload（长度恰 15）**不是合法图像帧**：sim 侧应直接丢弃（世界侧正常不会发，`Emit_Frame()` 对空图返回 `false`）。

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
| 同一 `peer_id` 重复 `chassis_hello`（该连接本身是底盘连接；**含跨 kind**：同一 `peer_id` 先后 robot / pedestrian） | 注册前**不分类型** `_RemoveAvatar(peer_id, except_link=本连接)`（两张表 + 两个容器一起清、旧连接被关且交出 `link.vehicle`），再新建；`chassis_ready` 重发、`assembly_ready` 不重发 |
| 已握手的**传感器**连接发 `chassis_hello` | `push_warning` + **忽略**（角色是单值，不改角色、不建车） |
| `motion_cmd` 出现在非底盘连接 | `push_warning` + 忽略（数据面只在底盘连接上） |
| 已握手连接发未知 `msg` | `push_warning` + 忽略，**不断连、不崩**（§8.1-6） |
| sim 发二进制帧 | `push_warning` + 丢弃该帧（二进制只下行，§8.2），不断连 |
| 连接断开 | 按 §5.3 清理；`_manifests` / `_manifest_parts` / `_assembly_sent` / `_pending_sensors` 同步清除 |
| `chassis_hello.kind` **非法**（非 `robot`/`pedestrian`） | `push_warning` 一次 + **按 robot 处理**，连接**不关**（task_35 Step 2） |
| `kind="pedestrian"` 但 `chassis != "pedestrian"` | `push_warning` + **按 robot 处理**（不建行人、连接不关） |
| `chassis_hello.spawn` 非法（越界 / 非有限数 / `z` 越界 / 落在障碍内 / 与已占 avatar 重叠） | `push_warning` + **回退默认点** `(64, 56, 0)`，照常装配并在 `chassis_ready.spawn` 回**真实**出生点（只有 pedestrian 读 `spawn`） |
| 行人**超过 300 ms** 没有新 `motion_cmd` | `_Tick_Pedestrian_Timeouts()` 下发 `v=0`（**只停、不断连、不刷屏**；日志 `pedestrian idle …` 每段一行）；**车没有超时** |
| 行人的 `sensor_hello`（manifest 声明的槽位） | 行人没有 `AttachSensor` → `has_method` 守卫后走"几何未落地"同一分支：回 `sensor_ready`（`geometry=false`）、manifest 可置位、**绝不** `Invalid call` |
| 行人底盘连接断开 / 重复 `chassis_hello` | `_RemoveAvatar()`：先从 `_pedestrians` 摘表再 free（R9）+ 清 `_ped_last_motion_ms` / `_ped_idle` + 关该 peer 其余连接；日志 `pedestrian removed peer=…`。**只在真摘到节点时才 emit / 打日志**（两张表都空 = 静默，第 2 轮） |

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
| E26 | **相机组件（`SubViewport` 离屏渲染）的四条硬约束**（task_33 Step 2）：① `SubViewport.own_world_3d` **必须 `false`**（共享父视口的 `World3D`；`true` 会渲空世界）；② **必须 `await RenderingServer.frame_post_draw`** 之后才 `get_image()`，否则读到上一帧 / 空缓冲 —— 而渲染信号**只能 `await`、绝不能 `connect()`**（连 Callable 会让进程退出时 SIGABRT / EXIT=134）；③ 本机 **`godot --headless` = `--display-driver headless`，只支持 dummy 渲染驱动**：`frame_post_draw` 永不触发、`get_texture().get_image()` 返回 null 并打 `ERROR: Parameter "t" is null`、`await` 会**永久挂住**（实测被 240 s timeout 杀掉）→ 组件与自检都必须**先过 `Can_Capture()` 闸、再 await**，出图验收只能在虚拟显示下跑（`tmp/task33/xvfb` 的 `Xvfb :99` + `--rendering-driver opengl3` + **`--audio-driver Dummy`**，否则无声卡会多一行 ALSA `ERROR`）；④ **`SubViewport` 不是 `Node3D`**：`Node3D → SubViewport → Camera3D` 会**断掉变换继承链**（实测根节点在 `(64, 6, 64)` 时 `Lens.global_transform` 仍是 IDENTITY），抓拍前必须手工 `lens.global_transform = global_transform` | ①渲空世界；②③静默读到空 / 上一帧，或永久挂住（看起来像死循环）；④拍到"世界原点的画面"而不是挂载点的画面，且**不报任何错**。执行者：`src/tests/check_camera_sensor.gd` 的 `A3-viewport-node`（`own_world_3d=false` + 初始 `update_mode==0`）/ `A4-lens-node`（`Lens` 在 `SubViewport` 下 + 光心在原点）/ `A12-no-physics-lint`（禁 `UPDATE_ALWAYS`）/ `B1~B9`（真渲染九项）。另有三个同级坑写在 §4.8.1-⑤⑥⑦：`Configure(null)` 形参不能写 `Dictionary`、`make_dir_recursive_absolute()` 失败会打 ERROR |
| E27 | **传感器模型不得带碰撞体**（task_33 Step 3）：`Lidar2D.tscn` / `CameraSensor.tscn` 里的 `Body` 只是 `assets/sensors/*.glb` 的实例（glb 本身 0 碰撞），**整个传感器子树里 `CollisionShape3D` / `CollisionPolygon3D` / `PhysicsBody3D` / `Area3D` 计数必须为 0**——不允许为了"能放置/能站住"给传感器加形状。配套口径：① `Body` 是 glb 实例根（`Node3D`），子节点才是 `MeshInstance3D`（多网格模型无法直接做成单个 `MeshInstance3D`）；② 相机光轴 = 传感器局部 `−Z`，模型镜头朝 `+X` 故 `Body.rotation_degrees=(0,90,0)`（写 `-90` 会朝车尾）；③ `Body.position` 不写：模型原点已是光心 / 扫描面中心 | 加了碰撞体 → 雷达**扫到自己**（`0.0x m` 的假回波堆在车顶，SLAM 直接崩）、相机被自己的壳挡（`near=0.05` 也救不了）、且 `SelfRidArray()` 只排除宿主 `Vehicle` 一个 RID，**不会**排除传感器自己的形状。执行者：`src/tests/check_models.gd` 的 `S5-sensor-no-collision`（两个已挂传感器子树计数 = 0）+ `check_sensor_models.gd` 的 `*-no-collision`（资产层 9 项/件）+ `check_camera_sensor.gd` 的 `A5-no-collision`（组件层）三重守卫 |
| E28 | **glTF 内嵌贴图会被导入器抽取成 `.glb` 旁的独立贴图文件**（human_model Step 4 实测）：Godot 4.7.2 的场景导入参数默认 `gltf/embedded_image_handling=1`（Extract）→ `assets/humans/<name>/model.glb` 导入后**额外生成** `model_<glTF 内图名>.jpg/.png`（本资产是 `model_rp_*_dif.jpg` + `model_rp_*_norm.png`）以及各自的 `.import`。① 这 4 个文件**必须与 `.glb`、`.glb.import` 一起提交** —— 只提交 `.glb` + `.import` 时，任何人在干净检出上重导都会 `ERROR: Cannot open file …_dif.jpg`；② 运行 / 自检**之前必须先** `godot --headless --path orion-simulation-world --import`，`.godot/` 缓存缺失时 `load(model.glb)` 返回 `null`（.tscn 仍能加载但 `Body` 为 `null`）——实测 `Person` 节点仍被建出却是**空壳**（0 mesh，画面里看不到人），引擎打 **12 行** `ERROR: Cannot open file 'res://.godot/imported/…*.scn'`，`check_person_assets` 报 **`total=32 failed=14`**；③ 反过来把参数改 `2`（Basis Universal 内嵌）能换回“只有 `.glb`”，但那是**有损压缩**、对下游 YOLO 判据是负收益，本支线不采用 | 漏提交 sidecar → 别人重导即 ERROR 或贴图全丢；忘了先 `--import` → `Person.tscn` 的 `Body` 拿不到 glb、世界里 0 个人，而 `--quit` 仍可能 EXIT=0（“0 ERROR”假红）。执行者：`src/tests/check_person_assets.gd` 的 `asset-loads-*`（缓存缺失即红）/ `asset-material-*` 与 `scene-material`（贴图真在）/ `asset-sha256-*`（字节未被悄悄替换）+ `World.gd::BuildPersons()` 的 `ResourceLoader.exists()` 守护（缺场景只 warn） || E28 | **glTF 内嵌贴图会被导入器抽取成 `.glb` 旁的独立贴图文件**（human_model Step 4 实测）：Godot 4.7.2 的场景导入参数默认 `gltf/embedded_image_handling=1`（Extract）→ `assets/humans/<name>/model.glb` 导入后**额外生成** `model_<glTF 内图名>.jpg/.png`（本资产是 `model_rp_*_dif.jpg` + `model_rp_*_norm.png`）以及各自的 `.import`。① 这 4 个文件**必须与 `.glb`、`.glb.import` 一起提交** —— 只提交 `.glb` + `.import` 时，任何人在干净检出上重导都会 `ERROR: Cannot open file …_dif.jpg`；② 运行 / 自检**之前必须先** `godot --headless --path orion-simulation-world --import`，`.godot/` 缓存缺失时 `load(model.glb)` 返回 `null`（.tscn 仍能加载但 `Body` 为 `null`）——实测 `Person` 节点仍被建出却是**空壳**（0 mesh，画面里看不到人），引擎打 **12 行** `ERROR: Cannot open file 'res://.godot/imported/…*.scn'`，`check_person_assets` 报 **`total=32 failed=14`**；③ 反过来把参数改 `2`（Basis Universal 内嵌）能换回“只有 `.glb`”，但那是**有损压缩**、对下游 YOLO 判据是负收益，本支线不采用 | 漏提交 sidecar → 别人重导即 ERROR 或贴图全丢；忘了先 `--import` → `Person.tscn` 的 `Body` 拿不到 glb、世界里 0 个人，而 `--quit` 仍可能 EXIT=0（“0 ERROR”假红）。执行者：`src/tests/check_person_assets.gd` 的 `asset-loads-*`（缓存缺失即红）/ `asset-material-*` 与 `scene-material`（贴图真在）/ `asset-sha256-*`（字节未被悄悄替换）+ `World.gd::BuildPersons()` 的 `ResourceLoader.exists()` 守护（缺场景只 warn） |
| E29 | **`--fixed-fps 60` 只给"自己数帧"的自检用，墙钟驱动的端到端**绝对不能**带**（task_35 Step 5 实测）：它在 headless 下会**关掉实时同步**，实测跑到 **21~37 倍速**（`--max-fps 60` 也救不了）。判据"`\|d\| ≈ v×t`"会因此必红（10 s 墙钟 = 360 s 世界时间，人撞到东侧空气墙）。**正确姿势**：① 自检脚本（`await physics_frame` 自己数帧、需要 `delta` 恒 = 1/60）→ `--headless --fixed-fps 60`；② Python / 墙钟驱动的端到端 → `--headless --max-fps 60`（实测 ratio 1.00、pose 10.2 Hz），并且**速率类断言一律用 pose 的 `world_time`** 当时间基（只有"300 ms 超时"那条必须用墙钟，因为世界侧 `_Tick_Pedestrian_Timeouts` 用的就是 `Time.get_ticks_msec()`）。反例与本轮实测见 §4.11.2 |

| E30 | **世界观的四条硬约束**（task_38 Step 1b 实测）：① `uv1_scale` 是**乘子**不是"米"——世界空间 triplanar 的采样坐标是 `(MODEL_MATRIX * vec4(VERTEX,1)).xyz * uv1_scale + uv1_offset`，所以 **6 m 一循环 = `1/6`**、2 m = `1/2`（写 `6.0` 会让纹理每 1/6 m 重复一次）；② `Environment` / `Sky` 是 `.tscn` 里的**共享 SubResource**，运行时必须先 `duplicate()` 再改并回写 `WorldEnvironment.environment`——直接改原对象会让**同一进程里第二次 `instantiate()` 拿到已被改过的环境**，而 `World.tscn` 仍显示 `ProceduralSkyMaterial`，极难查；③ 天空换成 HDR panorama 后 `background_mode` 必须仍是 **`BG_SKY(2)`**、`ambient_light_source` 必须仍是 **`AMBIENT_SOURCE_SKY(3)`**（`check_world` 的 `environment-sky` 与 `check_world_look` 的 `env-modes` 双重守卫），**不要**改成 `AMBIENT_SOURCE_COLOR`；④ `--headless`（dummy 渲染器）下 `Can_Capture()==false`，出图断言必须**整体 skip 且不计入 total**，并且照旧**先闸后 await**（E26-③） | ①纹理尺度静默错 6/2 倍且画面只是"看起来密一点"；②同进程复用世界时环境被"串味"，`check_*` 之间互相污染；③环境光来源一改，`check_world` 立刻红且天空不再照亮场景；④headless 下 await `frame_post_draw` 会永久挂住（看起来像死循环），或把 `total` 冲成与基线不一致。执行者：`src/tests/check_world_look.gd` 的 `ground-scale` / `voxel-scale`（`1e-6` 容差、且必须 `≠1`）/ `env-modes` / `sky-material` / `fallback-and-idempotent`（二次 `Apply_World_Look()` 必须换回**同一个**材质实例）+ `src/tests/check_world.gd` 的 `environment-sky` |
| E31 | **街道/立面这五条坑**（task_38 Step 2 实测）：① `MeshInstance3D`（`GeometryInstance3D`）**没有 `collision_layer` 属性**——`get("collision_layer")` 返回 `null`，零碰撞断言必须把 `null` 也算 0，不能写 `int(mi.collision_layer) == 0`；② `SurfaceTool` 合并薄片时必须 **4 顶点 + 6 索引**（`add_index(base+0/1/2, base+0/2/3)`，`base` 自己累加）：若按 6 顶点/四边形提交，`verts.size() / 4` 会算出 **1.5 倍**四边形数（实测 28 标线被数成 42、60 窗被数成 90、35 路缘面被数成 52）；③ **Godot 的正面是顺时针绕序**，`generate_normals()` 给出的法线 = `(c−a)×(b−a)`，与"右手系 `(b−a)×(c−a)`"**反号**——按右手系排绕序会让路面/路缘顶面的法线朝**地下**（实测顶面法线 `-Y`、路缘顶面数 0、朝下"底面"数 11），面被背面剔除；本工程统一在 `_Add_Quad()` 里用 `(c−a)×(b−a)`，且 `_Quad_Up` 的四角按"从上方看逆时针"给；④ `CameraSensor` **只在 `Capture_Jpeg()` 里** `_Sync_View_Transform()`：挪 holder 不会同步镜头，任何 `unproject_position` 断言必须**紧跟在同一张抓拍之后**（否则用上一张的机位投影，实测斑马线对比度假红成 `0.0014`，修好后 `0.4652`）；⑤ `World.OBSTACLES` 是 `const`，不在属性表里 ⇒ `Facade` 只能经 `world.get_script().get_script_constant_map()` 读（**不能** `preload("World.gd")`，`World.gd` 已 preload `Facade.gd`，会形成循环依赖） | ①零碰撞断言永远红或永远绿；②"四边形数"自证法失效，条数断言全体错位；③几何看不见/受光反了，而自检若只数条数则全绿；④出图类断言在"对的位置"上采到"错的机位"，红绿都不可信；⑤循环 preload 让整个 World 解析失败。执行者：`src/tests/check_street.gd` 的 `street-zero-collision` / `facade-zero-collision`（容 `null`）、`road-top-z`（附带断言法线朝上）、`curb-profile`（朝下四边形数 = 0）、`window-total` / `crosswalk-contrast`（`verts.size() == 4 × quads` 反推）、`windows-dark`（紧跟 shot-01 抓拍） |

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
| 传感器 | **只有 `lidar_2d`**（D6），**外观模型已接入**（task_33 Step 3） | `imu` / `gnss` **不做**（不是零件、也不发消息）：位姿由 `pose` 文本消息提供，见 §8.1-8 / §9.1 E23。`Lidar2D` / `CameraSensor` 两个场景已挂 `Body`（`lidar_tmini.glb` / `camera_module.glb`，**纯视觉、无碰撞**，E27）；`SensorCatalog` 仍只登记 `lidar_2d`（相机不进目录） |
| 位姿下行 | `pose`（10 Hz，走底盘连接） | task_31 Step 1；**理想真值**，无噪声 / 无延迟 / 无丢包；`z` 由世界给（UGV `0.0` / UAV `2.5`），消费方不得假设 `z==0`；`vz` 不上线 |
| 底盘 | `diff_drive_ugv` / `quadrotor_uav` 两种（D6） | 目录外名字直接拒绝。`diff_drive_ugv` 的几何默认值 = `ugv_chassis.glb` 的 AABB **`0.30 × 0.25 × 0.25`**（D11：模型 = 碰撞盒，task_33 Step 3）；车体截面由 `0.16/0.24` 变 `0.25/0.25` **会改变贴墙 / 绕行 / 急停余量**（同样的墙面，车头更早接触），goto 冒烟需按新尺寸复核 |
| 世界边界 | **可见 2 m 体素矮墙 + 不可见 32 m 空气墙 + 32 m 顶盖**（空气墙在 layer 1，与地面同层） | task_32 Step 2；矮墙只作视觉 / 近距障碍，真正的硬边界是空气墙：20 m/s 四面冲撞 10 s 与 5 m/s 持续爬升 10 s 都出不去（`src/tests/check_bounds.gd` 的 B2~B6）；雷达贴边时会把空气墙扫成静态障碍 → SLAM 地图边缘出现"墙"，**预期行为**（task_32 §三.5-3）。另：UAV 还有 `[0, 31.8]` **软件 clamp 兜底**（`Quadrotor.ClampAltitude()`，task_32 Step 3），即使物理失效也飞不出 32 m 顶盖 |
| 静态人物 | **2 个实例**（`rp_carla_rigged_001` ×2；备选 `rp_eric` **在库但未摆**） | human_model Step 4：纯视觉 / **零碰撞** / 无动画 / 无 AI / 不进装配线 / 不参与决策链；不随 `motion_cmd` 移动、不被射线扫到（`check_lidar` F7/F8 逐字不变）；只在 Step 5 用于"相机拍得到 + YOLOv8n 能认出人"的视觉验证 |
| 行走人物 | **0 个自动走动实例**（task_35 Step 3 起自动绕圈已移除） | walking_person Step 2/3 曾用 1 个 `rp_nathan_animated_003_walk` 沿 44 m 闭合矩形路径以 `1.260 m/s` 走动；**task_35 Step 3 把 `World.WALKERS` / `BuildWalkers()` 整节删除**，世界**不再**自己让人走（走动能力与 `PersonWalking.tscn` 资产保留给 §4.10 的人形自检，`Person.gd` / `Person.tscn` 一字未改）。世界里现在的人见下一行 |
| 外部驱动的行人 | **`kind="pedestrian"` 的 avatar 已可装配**（task_35 Step 2）+ **世界预置 2 个 idle 行人**（task_35 Step 3） | 世界侧把它当"特殊底盘"：`Pedestrian.tscn`（`CharacterBody3D` + 胶囊 + 走路 glb，`layer 2` / `mask 1\|4`）**根节点就是 avatar**，进 `Pedestrians` 容器（**不进** `Vehicles`、**不占** `_PickSpawn` 排位），由 `chassis_hello{kind, spawn?}` 装配、`motion_cmd{v, dir}` 驱动、`pose` 10 Hz 回传、**300 ms 无指令自动停车**。**非目标**：不经 `pleiades-sim` / 不连 libp2p / 不参与任何决策链；**不涉及 SLAM / 建图 / 地图一致性 / 动态障碍 / 避让**（人写进地图、离开后被后续观测抹掉都是正常现象，**不做任何掩蔽或衰减**）；不做 YOLO、不做 tensor 图传、不做多行人编队。"人会穿建筑"是 Step 1 的既定取舍（`mask` 不含 VoxelWorld 所在层），选点由 spawn 校验兜住。**同一 `peer_id` 换 `kind`**（robot ↔ pedestrian）= 重建 avatar：两张表 + 两个容器一起清、旧连接交出所有权、`_RemoveAvatar` 只在真摘到节点时才 emit / 打日志（§4.5.1-⑩、§8.1-9）。**task_35 Step 3 起**：世界里还预置 **2 个 idle 行人**（`World.PEDESTRIANS` → `World/Pedestrians/pedestrian_01`(44,96) / `pedestrian_02`(52,102)），它们**没有任何 `peer_id`、不参与网络、没有外部驱动时站着不动**（不是故障），与动态行人**共用同一个 `Pedestrians` 容器**；动态行人的 300 ms 超时停车、`pose` 10 Hz、`_pedestrians` 表计数**都不受它们影响**（`GetPedestrianCount()` 只数注册的 peer）。世界预置的人**不会**被 `_Pick_Spawn` / `_Spawn_Occupied` 看见（那套只查已注册 avatar），所以选点靠常量表硬保证离协议 spawn 点 > 30 m（§4.11.1） |
| LiDAR 噪声 | 默认关闭 | `_rng` 保留，`params.noise` 未启用 |
| 相机传感器 | **世界侧零件已落地 + 已登记为可装配零件**（task_33 Step 2/3、task_34 Step 3）+ **已由 Step 4 的 `check_camera_shots.gd` 挂进世界拍照**（§4.8.2），**可按 `rate_hz` 出帧** | `SensorCatalog` 已登记 `camera`（§7.6），`sensor_hello` 会经 `Vehicle.AttachSensor()` 把它实例化到 `Vehicle/Sensors/<slot>`，`sensor_ready` 之后 `SetStream()`，随后按 `rate_hz`（默认 **1 Hz**）下发 `type=2` 帧（15 B 头 + JPEG 原始字节，§8.2）；默认 640×480（装配线口径，组件常量仍是 1280×720，见 §7.6）。**仍不推流 / 无请求应答**（无 `camera.capture()`）、**不进 `World.tscn`**（`World.tscn` 不引用相机）、**无碰撞体**（E27）。帧**内容**验收仍需真渲染环境：headless（dummy 渲染器）下只验头部、频率与"零帧"闸门（`A13~A18` / `J1~J7`） |
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

| task_33-1 | `Tool/blender/models/{build_models.py,export_all.sh,lidar_tmini.blend,camera_module.blend,ugv_chassis.blend}`（**新建**，工程根下，不在 `orion-simulation-world/` 内）、`orion-simulation-world/assets/sensors/{lidar_tmini,camera_module}.glb`（**新建**）、`orion-simulation-world/assets/chassis/ugv_chassis.glb`（**新建**）、`src/tests/check_sensor_models.gd`（**新建**）、`COMPONENTS.md`（§1.1 / §11 / 附） | Step 1 建模：三件 `.glb`（雷达 `0.080 × 0.054 × 0.080` / 相机 `0.055 × 0.036 × 0.030` / 底盘 `0.30 × 0.25 × 0.25`）+ 三个 `.blend` 源 + bpy 构建脚本 + 导出脚本；**无碰撞体、无 `.tscn` 改动、无运行时改动**。`check_sensor_models.gd`：**29 项断言** / `total=29 failed=0` / `SELF-CHECK OK`；**11 个旧自检断言数与基线逐字一致**（`project` / `world 26` / `camera 32` / `vehicle 36` / `diff_drive 41` / `quadrotor 48` / `ws_manager 51` / `lidar 48` / `pose 23` / `bounds 12` / `models 16`，`failed=0`）；`--import` 与 `--quit` 均 EXIT=0 且 0 ERROR。UAV 复核结论：**沿用现有模型**（`0.64 × 0.14 × 0.64`、机头 +x、原点 = 底面中心、**无机头下方水平挂载面**） |

| task_33-2 | `src/sensors/CameraSensor.gd`（**新建**）、`src/scenes/sensors/CameraSensor.tscn`（**新建**，§3 第 8 个组件）、`src/tests/check_camera_sensor.gd`（**新建**）、`COMPONENTS.md`（§0.2 / §1.1 / §1.4 / §3 / §4.8 / §9.1 E26 / §10 / §11 / 附） | Step 2 相机组件（离屏渲染 → JPEG）：`Configure` / `Capture_Jpeg` / `Capture_To_File` / `Get_Image_Size`（+ 环境探测 `Can_Capture()`）。**不挂模型、不接网络、不实例化进 `World.tscn`、不改 `SensorCatalog`**。`check_camera_sensor.gd`：**A 组 12 项**（结构 / 参数，纯 headless 也跑）+ **B 组 9 项**（真渲染，`Can_Capture()==false` 时整组 `[skip]` 且不计入 total）→ headless `total=12 failed=0`、**Xvfb 下 `total=21 failed=0`**，两条均 `SELF-CHECK OK` + EXIT=0 + 0 ERROR；`--quit` EXIT=0 且 0 ERROR；**12 个既有自检断言数与基线逐字一致**（`project` / `world 26` / `camera 32` / `vehicle 36` / `diff_drive 41` / `quadrotor 48` / `ws_manager 51` / `lidar 48` / `pose 23` / `bounds 12` / `models 16` / `sensor_models 29`，`failed=0`）。实测：1280×720 → **29 147 B**（SOF0 1280×720，首三字节 `ffd8ff`，灰度 `var=0.018553 mean=0.5668 spread=0.875`）；`fov 70 → 40` 成图 **29 147 B vs 25 005 B 不同**；`640×360` → 8 904 B；`Capture_To_File` 25 005 B；坏路径返回 `false` 且无 ERROR |

| task_33-3 | `src/scenes/sensors/Lidar2D.tscn`（+`Body`）、`src/scenes/sensors/CameraSensor.tscn`（+`Body`）、`src/scenes/Vehicle.tscn`（`ModelUGV` 换 `assets/chassis/ugv_chassis.glb` + 占位 `0.30×0.25×0.25` / `y=0.125`）、`src/scenes/chassis/DiffDrive.tscn`（占位轮位 `z=±0.125`）、`src/vehicle/Vehicle.gd`（`mount.pitch`）、`src/chassis/DiffDrive.gd` + `src/chassis/ChassisCatalog.gd`（默认 `track_width/height 0.24/0.16 -> 0.25/0.25`）、`src/tests/check_models.gd`（**16 -> 24**）、`src/tests/check_vehicle.gd`（**36 -> 40**）、`src/tests/check_diff_drive.gd`（值，断言数仍 41）、`COMPONENTS.md`（§1.1 / §2.2 / §3 / §4.3 / §4.4 / §4.7 / §4.8 / §5.1 / §7.6 / §9.1 E27 / §10 / §11 / 附） | Step 3 传感器接入场景（模型 + 挂点 + pitch + 碰撞盒）：三件 `.glb` 各在**组件场景**里静态 `instance`（**不改 `World.tscn`**）；`Body` 是 glb 实例根（`Node3D`），雷达 identity、相机 `rotation_degrees=(0,90,0)`（光轴 = 局部 `−Z`）；`mount.pitch` 单位度、正抬头 / 负低头、缺省 0（不预置任何角度）；底盘碰撞盒 = 模型 AABB `0.30×0.25×0.25`（D11）。自检：`check_models` **24**（+`M3b-ugv-model-equals-hull` + `S1~S7`）/ `check_vehicle` **40**（+`placeholder-size` + `mount-pitch-default/down/up`）/ `check_diff_drive` **41**（值更新，`wall-stops x=127.3375` 与旧尺寸逐字相同）/ `check_sensor_models` **29**；**传感器零影响**：`check_lidar` **48 条断言数字逐字不变**（`F7-real-wall-distance 2.0000` / `F8-real-wall-hits 152` / `D8-other-vehicle-visible 0.7500`，改前改后日志 diff 仅剩 `.tscn` 字节数与扫描时间戳两项非测量量）、`check_camera_sensor` headless **12**（Xvfb 下 **21**）A 组数字不变；13 条自检全 `SELF-CHECK OK` + `EXIT=0`；`--import` / `--quit` 均 EXIT=0 且 0 ERROR。**不执行** `git add` / `git commit` |

| task_33-4 | `src/tests/check_camera_shots.gd`（**新建**）、`COMPONENTS.md`（§1.1 / §1.4 / §4.8 / **§4.8.2** / §10 / §11 / 附） | Step 4 模拟世界拍照（出图到 `tmp/task33_shots/`，**不在仓库**）：真 `World.tscn` + 静止 UGV（`ApplyChassis("diff_drive_ugv")`）+ 建模雷达/相机，三机位拍 **3 张 1280×720 JPEG**（朝东 / A+90° 朝南 / 外部 3/4）到**绝对路径**。`check_camera_shots.gd`：**S 组 18 项**（结构，headless 也跑）+ **R 组 17 项**（真渲染，`Can_Capture()==false` 时整组 `[skip]` 且不计入 total）→ headless `total=18 failed=0`（`[skip]` ×17）、**Xvfb 下 `total=35 failed=0`**，两条均 `SELF-CHECK OK` + EXIT=0 + 0 ERROR；**13 个既有自检断言数与基线逐字一致**（`project` / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `ws_manager 51` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 12`，`failed=0`）；`--quit` EXIT=0 且 0 ERROR；日志无 `[ws] listening`（脚本在 `add_child` 前关掉 `autolisten`）。实测三张 **26 315 / 24 462 / 29 576 B**（SOF0 均 1280×720、首三字节 `ffd8ff`），灰度 `var 0.009563 / 0.008259 / 0.012607`、网格 `contrast 56/42/48` 与 `midband 28/28/30`，车体投影占屏 **0.0000 / 0.0000 / 0.0689**，`pillar_e` 屏内角点 **8 / 0**（"对着哪拍哪"），两两字节差异 **0.823 / 0.963 / 0.960**。**不改** `CameraSensor.gd` / 任何 `.tscn` / `assets`；**不启动 sim、不连 WS、不跑决策链**；**不执行** `git add` / `git commit` |
| task_34-3 | `src/sensors/SensorCatalog.gd`（`SCENES` + `"camera"`、`DEFAULTS` + 640×480@1 Hz）、`src/sensors/CameraSensor.gd`（+`rate_hz` / `DEFAULT_RATE_HZ` / `MIN_RATE` / `ACCUM_EPS` / `frame_produced` / `SetStream` / `ClearStream` / `HasStream` / `Tick_Stream` / `Emit_Frame` / `Get_Tick_Count` / `Get_Frame_Count` / `_Capture_Tick` / `_Warn_No_Renderer` / `_Rate_Or` / `_exit_tree` / `_process`）、`src/net/WireProtocol.gd`（+`BuildCameraFrame`）、`src/net/ComponentLink.gd`（+`MAX_FRAME_BUFFER_BYTES`，`Init()` 在 `accept_stream` 前设 4 MiB 双缓冲）、`src/tests/check_camera_sensor.gd`（**12 -> 18**：`A13~A18` + `B10` + 内联 `StubLink` + `_Step_Ticks`）、`src/tests/check_ws_manager.gd`（**51 -> 59**：`A9` / `A10` + J 段 `J1~J6` + `[skip] J7`）、`COMPONENTS.md`（§1.4 / §3 / §4.8 / **§4.8.3** / §7.3 / §7.4 / §7.6 / §8.2 / §10 / §11 / 附） | 世界侧 camera 成为**可装配零件**并按频率出 `type=2` 帧：`sensor_hello(sensor="camera")` → `sensor_ready` → `SetStream` → 按 `rate_hz`（默认 1 Hz）组帧下发（15 B 头 + JPEG 原始字节，§8.2）；`Can_Capture()==false`（headless）时**只 warn 一次、绝不出帧、绝不 await**（E26-③）。`check_camera_sensor`：headless **`total=18 failed=0`**（`[skip]` ×10）、Xvfb 下 **`total=28`**；`check_ws_manager`：**`total=59 failed=0`**（`[skip]` ×1）；**其余 12 个自检断言数与基线逐字一致**（`project` / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_shots 18`，全部 `failed=0`）；`--quit` EXIT=0 且 0 ERROR。实测关键行：`A14 size=25 type=2 slot=7 seq=1` / `A16 1hz=1/1 1hz=1/1 2hz=2/2` / `A17 can=false ticks=1 frames=0 warned=true` / `A9 size=20 type=2 slot=4 seq=11 empty=15` / `A10 in=4194304 out=4194304 closed=true` / `J2 size=(640, 480) fov=70.0 rate=1.00 can_capture=false` / `J4 physics_nodes=0` / `J6 ticks=+1 frames=0`。**不改** `World.tscn` / `Vehicle.gd` / `Lidar2D.gd` / `ChassisModel.gd` / `WebSocketManager.gd` / 任何 `.tscn` / `assets` / 任何 `pleiades-*` crate；Xvfb 档（`B10` / `J7`）本机无 `DISPLAY` **未实测**；**不执行** `git add` / `git commit` |
| human_model-4 | `assets/humans/`（**新建**：`README.md` + `LICENSE_renderpeople.txt` + `rp_carla_rigged_001/` + `rp_eric_rigged_001/`，每件 = `model.glb` + `.glb.import` + 2 张贴图 + 2 个贴图 `.import`）、`src/scenes/props/Person.tscn`（**新建**，§3 第 9 个）、`src/props/Person.gd`（**新建**）、`src/tests/check_person_assets.gd`（**新建**）、`src/world/World.gd`（+`PERSON_SCENE` / `PERSONS_NODE` / `PERSONS` / `_persons_root` / `BuildPersons()` / `GetPersonsRoot()`）、`COMPONENTS.md`（§0.2 / §1.1 / §1.4 / §2 / §2.1 / §2.2 / §3 / §4.1 / **§4.9** / §9.1 E28 / §10 / §11 / 附） | 静态人物接入 Godot（支线，**不进装配线**）：2 件人形 `model.glb` 落 `assets/humans/`（sha256 与 Step 3 证据链逐位一致），`Person.tscn` = `Node3D` + glb 静态实例（**零碰撞**），`World.gd` 以常量表 `PERSONS` **程序化、幂等**摆 2 人（世界 `(60,76)` / `(68,70)`，yaw 0）。`check_person_assets.gd`：**32 项断言** / `total=32 failed=0` / `SELF-CHECK OK`（资产 16 + 场景 5 + 常量 2 + 摆位 9）；**14 个既有自检断言数与基线逐字一致**（`project` / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `ws_manager 59` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 18` / `camera_shots 18`，全部 `failed=0`）；`--import` / `--quit` 均 EXIT=0 且 0 ERROR（日志有 `[world] persons built: 2 (2 specs)`）；真渲染出图 3 张 1600×900（`DISPLAY=:99` + `opengl3` + `Dummy`，非 headless）。**不改** `World.tscn` / `project.godot` / 既有自检的断言与节点树；**不碰** `pleiades-*` / `Task/` / `Workbook/`；**不执行** `git add` / `git commit` |
| walking_person-2 | `assets/humans/rp_nathan_animated_003/`（**新建**：`model.glb` + `.glb.import` + 抽出的 `model_*_dif.jpg` / `model_*_norm.png` + 两个 `.import`）、`src/scenes/props/PersonWalking.tscn`（**新建**，§3 第 10 个）、`src/props/PersonWalking.gd`（**新建**）、`src/tests/check_walking_person.gd`（**新建**）、`src/world/World.gd`（+`WALKER_SCENE` / `WALKERS_NODE` / `WALKERS` / `_walkers_root` / `BuildWalkers()` / `GetWalkersRoot()`；`_ready()` 末尾在 `BuildPersons()` 之后调用）、`assets/humans/README.md`（只追加 1 行 + 1 段）、`COMPONENTS.md`（§1.4 / §2 / §2.1 / §2.2 / §3 / §4.1 / **§4.10** / §10 / §11） | 走路人物接入 Godot（支线，**不进装配线**）：Renderpeople Free **Animated** People 的 `rp_nathan_animated_003_walking` → 剥根位移 / 压到 **1.84 m** / 脚底 `y≈0` / 朝 **+X** → 带 skin + 1 条动画的 glb（`3 263 492 B` / `21 506 tri` / `88 骨` / `duration 2.2333 s`，源首帧零位 T-pose 已裁、原地循环接缝 `0.1145 m`）。`PersonWalking.tscn` = `Node3D` + glb 实例（**零碰撞**），`World.gd::BuildWalkers()` 幂等建 `Walkers` 并 `Configure` 一条 44 m 闭合路径（1.260 m/s）。`check_walking_person.gd`：headless **`total=16 failed=0`**、`DISPLAY=:99` **`total=22 failed=0`**（三帧 1280×720 JPEG 落 `tmp/walking_person/shots/`，28 900 / 27 183 / 27 750 B，投影框中心两两相距 300.5 / 582.7 / 283.6 px，框内灰度 `mean_abs` 0.201 / 0.253 / 0.172）；**13 个既有自检断言数与基线逐字一致**（`project` / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `ws_manager 59` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 18`，全部 `failed=0`）+ `check_person_assets 32` / `check_person_shots 7`（headless）；`--import` EXIT=0 且 0 ERROR（日志有 `[world] walkers built: 1 (1 specs)`）。**不做**碰撞体 / 雷达 / 建图 / 决策链 / YOLO；**不改** `World.tscn` / `project.godot` / `Person.*` / 既有自检断言；**不碰** `pleiades-*` / `Task/` / `Workbook/`；**不执行** `git add` / `git commit` |
| walking_person-3 | `assets/humans/rp_nathan_animated_003_walk/`（**目录改名**：6 文件、`glb` `sha256` 不变、3 个 `.import` 重建且 params 段与备份逐行相同）、`src/props/Person.gd`（**改写**：走动并入 `walk_speed` / `path` / `loop` + 弧长采样 + `_physics_process` 推进 + 动画播放，**默认静态**）、`src/props/PersonWalking.gd`（**删除**）、`src/scenes/props/PersonWalking.tscn`（改 2 行：根脚本 → `Person.gd`、`Body` → `_walk` 资产）、`src/world/World.gd`（`WALKERS` 键名统一 `walk_speed` + `loop`，`Configure` 传三字段；`PERSONS` / `BuildPersons()` 一字不动）、`src/tests/check_person_walk.gd`（**新增**，headless 20 项）、`src/tests/check_walking_person.gd`（**删除**）、`assets/humans/README.md`、`COMPONENTS.md`（§1 状态 / §1.4 / §2 / §2.1 / §2.2 / §3 / §4 / §4.9 / §4.10 / §10 / §11） | 走动收口（支线，**不进装配线**）：世界里有 1 个人**真的在走**（动画在播 + 位置随时间变）。`check_person_walk.gd`：headless **`total=20 failed=0`**、`DISPLAY=:99` **`total=26 failed=0`**；运行组实测 `L=44.000 m` / `v=1.260 m/s` / `frames=2096`（真跑一整圈）/ `Δs` 逐帧 `0.02100 m`（上限 `0.03150`）/ `close=0.0160 m`（≤0.05）/ `path_dev=0.000000 m` / `hip_drift=0.0239 m`；三帧 1280×720 JPEG 落 `tmp/walking_person/shots/`，`28 877 / 27 165 / 27 775 B`，投影框中心两两相距 `301.6 / 583.8 / 283.6 px`，框内灰度 `mean_abs` `0.20178 / 0.25179 / 0.17722`；**14 个既有自检断言数与基线逐字一致**（`project` 无 total / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `ws_manager 59` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 18` / `person_assets 32` / `person_shots 7`（headless）或 `22`（DISPLAY），全部 `failed=0`、0 ERROR）；`--quit` EXIT=0 且 0 ERROR（日志有 `[world] walkers built: 1 (1 specs)` + `[person] walker_01 configured: ... walk_speed=1.260 m/s loop=true`）。**不做**碰撞体 / 雷达 / 建图 / 决策链 / YOLO；**不改** `World.tscn` / `project.godot` / `Person.tscn` / `PERSONS` / 既有自检断言；**不碰** `pleiades-*` / `Task/` / `Workbook/`；**不执行** `git add` / `git commit` |

| task_35-2 | `src/net/WireProtocol.gd`（+`KIND_ROBOT` / `KIND_PEDESTRIAN` / `NormalizeKind()` / `BuildMotionCommand()`）、`src/net/WebSocketManager.gd`（+`PEDESTRIAN_CHASSIS`·`PEDESTRIANS_NODE`·`PEDESTRIAN_SPAWN_*`·`PEDESTRIAN_CMD_TIMEOUT_MS` / `pedestrians_root` / `_pedestrians`·`_ped_last_motion_ms`·`_ped_idle`·`_next_pedestrian_id` / `pedestrian_spawned`·`pedestrian_removed` / `GetPedestrian`·`GetPedestrianCount`·`IsPedestrian`·`GetAvatar`·`GetPedestriansRoot` / `_Register_Pedestrian`·`_Apply_Pedestrian_Command`·`_Tick_Pedestrian_Timeouts`·`_Pedestrians_Root`·`_Pick_Pedestrian_Spawn`·`_Pedestrian_Default_Spawn`·`_Spawn_In_Obstacle`·`_Spawn_Occupied`·`_World_Span_M`·`_All_Avatars`·`_HasAvatar`·`_Merge_Avatar_Params`·`_To_Float`·`_Kind_Of`·`_RemoveAvatar`·`_UniqueAvatarName`；`_process` 加超时停车、`Stop()` 清行人表、`GetStats()` 加 `pedestrians`、`chassis_ready` 加 `kind`。**第 2 轮**：`_RemoveAvatar(peer_id, except_link) -> bool`（两张表 + 两个容器一起摘、只摘到时才 emit/print、被判死刑的连接交出 `link.vehicle`）、`_CleanupLink()` 所有权守卫（`link.vehicle != null` 才摘）、`_ApplyMotionCommand()` 按 `link.vehicle` 身份分流、`_RegisterChassis` / `_Register_Pedestrian` 注册前**不分类型**先摘）、`src/tests/check_ws_manager.gd`（**59 -> 76**：`A11` + `K1~K13`，第 2 轮再加 `K14~K16`（跨 kind 重建）+ `_drive_physics` / `_wait_closed_frames` 装置）、`COMPONENTS.md`（§1.4 / §4.5 / **§4.5.1-⑩** / §7.3 / §8.1 / §8.3 / §10 / §11） | Step 2 协议扩展：世界**认得"人"**。`chassis_hello.kind`（缺省 `robot`）+ 行人可选 `spawn`；`motion_cmd{v, dir}` 连续驱动；**300 ms 无新指令自动 `v=0`**（只停、不断连、不刷屏）；pedestrian **不进** `Vehicles`、**不占** `_PickSpawn` 排位、用独立的 `_next_pedestrian_id`。`check_ws_manager`：**headless `total=76 failed=0` / Xvfb `total=77 failed=0`**（两档都 `SELF-CHECK OK`、EXIT=0、0 ERROR；差 1 是 J7 需要渲染器）。第 2 轮 Xvfb 整轮墙钟 **65 s**（规划期基线 `total=74 failed=2`、50.95 s）。`A11` 协议形态白盒 + `K1` 装配回执 / `K2` 路由与容器 / `K3` pose 10 Hz / `K4` v+dir 弧线 / `K5` 老 `action/speed` 折算 / `K6` 超时阈值白盒 / `K7` 超时停车且位姿不动 / `K8` 非法 spawn 回退 / `K9` 缺省 spawn / `K10` 非法 kind 按 robot / `K11` 缺省 kind 按 robot / `K12` 行人传感器不崩（回执**无** `geometry` 键）/ `K13` 断连移除 / `K14` robot→pedestrian 换 kind / `K15` pedestrian→robot 换 kind / `K16` 同一连接重建。实测关键行（Xvfb 档）：`K1 spawn=(70.00, 70.00) yaw=0.00` / `K2 parent=Pedestrians vehicles=3 (was 3)` / `K3 pose=+10 in 60 frames` / `K4 yaw_err=0.0000 pose=(70.3266, 70.7934)` / `K5 speed=0.6300` / `K7 speed=0.0000 idle=true open=true dx=0 dy=0` / `K8 spawn=(64.00, 56.00)` / `K13 count=3 (was 4)` / `K14 vehicles=5(was 5) peds=4(was 3) events=["v:human-05"]` / `K15 pose=(64.0, 62.0)->(64.158, 62.0) events=["p:human-06"]` / `K16 open=true events=["v:human-07"]` / `H5 closed=true code=1002`。**断言"一项不能少"是机器化的**：标签 diff（`comm -23`）缺 0 条，既有 59 项 + A11 的 grep 计数 = 60。**16 个既有自检断言数与基线逐字一致**（`project` 无 total / `world 26` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 18` / `camera_shots 18` / `person_assets 32` / `person_walk 20` / `person_shots 7` / `pedestrian 40`，全部 `failed=0`、0 ERROR）；`--quit` EXIT=0 且 0 ERROR、日志**零** pedestrian 行（无人连入 = 节点树零变化）。**不改** `World.tscn` / `World.gd` / `Vehicle.tscn` / `Vehicle.gd` / `Pedestrian.gd` / `Pedestrian.tscn` / `project.godot` / 任何既有自检断言；**不做** SLAM / 建图 / 动态障碍 / 避让 / YOLO / 图传；**不碰** `pleiades-*` / `Task/`；**不执行** `git add` / `git commit` |

| task_35-1 | `src/scenes/props/Pedestrian.tscn`（**新建**，§3 第 11 个）、`src/chassis/Pedestrian.gd`（**新建**，`extends ChassisModel`）、`src/chassis/ChassisCatalog.gd`（`SCENES` + `"pedestrian"`、`DEFAULTS` + `{max_speed 1.26, max_turn_rate 180.0}`）、`src/tests/check_pedestrian.gd`（**新建**，40 项）、`COMPONENTS.md`（§3 首句 / §3 表 / §3 非组件表 / **§4.11** / §7.6 / §11） | 世界侧**行人底盘**（task_35 Step 1，只登记、不改任何既有语义）：`CharacterBody3D` + `CapsuleShape3D` r0.25 / h1.75 / 中心 y0.875 + 走路 glb（内含 `AnimationPlayer`，**不另建**）；`layer 2` / `mask 1\|4`（雷达扫得到、车撞得到、不撞 VoxelWorld）；`Step()` 是**纯控制量**（树外可调）、`move_and_slide()` 只在 `_physics_process()`（白盒锁 1 处）；`v`/`dir` 独立解析 + `action/speed` 回退；转向限速 180 °/s、**无加速度斜坡**（`v=0` 当帧归零）；`speed_scale = v/1.26`（`v=0` 定格）；缺动画只 `push_warning`。`check_pedestrian`：**`total=40 failed=0`** / `SELF-CHECK OK` / EXIT=0 / 0 ERROR（场景形态 7 + 目录 6 + `Step` 控制量 11 + 老协议 5 + 真 World 推进与动画 8 + 白盒 3）。实测关键行：`world-walk-60f dx=1.2602 dy=0 dz=0` / `world-turn-limited-1f d_world_yaw=0.0523599`（= 180 °/s × 1/60）/ `world-turn-converge-30f world_yaw=1.570796` / `world-stop-still moved=0.000000` / `pose-getter yaw=1.57079637`（与 `WorldTransform.PoseFromTransform` 逐键一致）/ `anim-play-walk clip=Armature\|Take 001\|BaseLayer loop=true scale=1.000/0.500/0.000 playing=true` / `white-box-single-mover move_and_slide x1`。**16 个既有自检断言数与基线逐字一致**（`project` 无 total / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `ws_manager 59` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 18` / `person_assets 32` / `person_walk 20` / `person_shots 7`，全部 `failed=0`、0 ERROR）；`--import` / `--quit` 均 EXIT=0 且 0 ERROR。**不改** `Vehicle.tscn` / `Vehicle.gd` / `Lidar2D.gd` / `World.tscn` / `World.gd` / `Person.gd` / `WebSocketManager.gd` / `assets`；**不做** SLAM / 建图 / 动态障碍 / 避让 / YOLO / 图传；**不碰** `pleiades-*` / `Task/`；**不执行** `git add` / `git commit` |

| task_35-3 | `src/world/World.gd`（**删**：`WALKER_SCENE` / `WALKERS_NODE` / `WALKERS` / `_walkers_root` / `BuildWalkers()` / `GetWalkersRoot()`；**加**：`PEDESTRIAN_SCENE` / `PEDESTRIANS_NODE` / `PEDESTRIANS` / `_pedestrians_root` / `BuildPedestrians()` / `GetPedestriansRoot()`；`_ready()` 里 `BuildWalkers()` → `BuildPedestrians()`，位置不变）、`src/tests/check_person_walk.gd`（**20 -> 15**：删 C 旧 3 / D 5 / R 6，加 C 新 3）、`src/tests/check_person_walk_shots.gd`（**总数 4 / 14 不变**，主语 `walker_01` -> `pedestrian_01`、驱动 `Set_Progress_M()` -> 一次 `SetCommand({"v": 1.26, "dir": 0.0})`、R7 尺子 `Δs` -> `Δx`；**step.md 名单之外但验收必需** —— 旧文件强依赖 `GetWalkersRoot()`，不改必红）、`COMPONENTS.md`（§2 / §2.1 / §2.2 / §3 / §4.1 / §4.10-⑤⑧ / **§4.11.1** / §10 / §11 / 附） | Step 3 世界布局：**世界不再自己让人走**，交给人（外部）驱动。`World.WALKERS`（1 条 44 m 闭合矩形路径）/ `BuildWalkers()` / `GetWalkersRoot()` 与 `Walkers` 容器**整节删除**，换成 `PEDESTRIANS`（**只有**出生点 + 初始朝向 + 数量，**无路径 / 无巡逻 / 无计时 / 无速度**）+ `BuildPedestrians()`（先摘同名旧容器再建，幂等）+ `GetPedestriansRoot()`；容器名 `Pedestrians` **逐字 = `WebSocketManager.PEDESTRIANS_NODE`**（与动态装配的人共用）。`PERSONS` / `BuildPersons()` **一字不动**；`Person.gd` / `Person.tscn` / `PersonWalking.tscn` / `World.tscn` / `project.godot` / `Pedestrian.*` / `WebSocketManager.gd` / 其余 13 个自检**一行未改**。**2 个世界预置 idle 行人**：`pedestrian_01` 世界 `(44,96)` yaw 0 / `pedestrian_02` `(52,102)` yaw 180`（互距 10.000 m）；选点判据逐条实测（离最近障碍 3.000 / 8.062 m；离边界 32 / 26 m；离 `SPAWN_WORLD` 37.736 / 39.850 m；离 `PERSONS` ≥ 25.6 m；离 `check_lidar` 热区 `(60,90)` **17.088 / 14.422 m** —— 全部 > 12 m range；离 `check_camera_shots` 停车点 `(60,84)` 20.000 / 19.698 m；离协议 spawn 点 `(64,56)` / K1 `(70,70)` / K16 `(72,78)` 均 > 30 m）。**无外部驱动时人站着不动 = 预期**（`speed_scale = 0.0`、仍 `is_playing`）。`check_person_walk`：**`total=15 failed=0`**，**headless 与 Xvfb 两档同值**（A 8 / B 4 逐字保留 + C 新 3：`world-pedestrians-count`（容器名 / 父 / 孩子数 2 / 名字逐字 / `Pedestrian.tscn` / `Pedestrian.gd` / `SetCommand`+`GetPose` 契约面）、`world-pedestrians-spawn-clear`（五条判据全过，`bad=[]`）、`world-pedestrians-idempotent`（`nodes 40->40`、同名容器 1 个、孩子数 2））。`check_person_walk_shots`：**headless `[skip]` ×10 + `total=4 failed=0`**、**Xvfb `total=14 failed=0`**（三张 `walk_t{0,1,2}.jpg` = `27 857 / 28 321 / 28 614 B`、SOF0 均 1280×720；`x = 44.000 / 44.840 / 45.680 m`、`frames = 0 / 40 / 80`、`Δx = 0.8400 m`、`Δx/Δframes = 0.021000 = v/60`（偏差 **0.00%**）；框 `P(523.7,276.2) S(119.2,173.0)` → `P(583.0,273.2) S(114.7,187.9)` → `P(654.8,269.5) S(107.2,206.2)`；`h_frac 0.240 / 0.261 / 0.286`；框水平位移 `57.0 / 68.1 / 125.1 px`；框内/框外 `0.0962/0.0001 (996.5×)`、`0.0666/0.0002 (406.4×)`、`0.0881/0.0002 (545.4×)`；64×64 裁剪差 `0.06694 / 0.07616 / 0.08532`；源码 lint `Set_Progress_M=0 / _ped.global_position=0 / SetCommand=1`）。**16 个既有自检断言数与基线逐字一致**（`project` 无 total / `world 26` / `camera 32` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `camera_sensor 18` / `camera_shots 18` / `lidar 48` / `person_assets 32` / `person_shots 7` / `pedestrian 40`，全部 `failed=0`、0 ERROR、EXIT=0）+ `ws_manager` **headless 76 / Xvfb 77** 逐字不变（K1/K2/K13/K15/K16 全绿；`[ws] pedestrians root created` **0 行** —— manager 直接复用世界已建的同名容器，`K2 parent=Pedestrians` 即证据）。`--quit` / `--quit --fixed-fps 60` 均 EXIT=0 且 **0 ERROR**，日志：`[world] persons built: 2 (2 specs)` + `[world] pedestrians built: 2 (2 specs, idle until driven)`、`walkers built` **0 行**、`[pedestrian]` 告警 **0 行**；`--import` EXIT=0 且 0 ERROR。**不做** SLAM / 建图 / 动态障碍 / 避让 / YOLO / 图传；**不改** `World.tscn` / `project.godot` / `PERSONS` / `pleiades-*` / `Task/` / `programs/` / 根 Cargo；**不执行** `git add` / `git commit` |
| task_35-5 | `src/tests/check_pedestrian_physics.gd`（**新建**，**22 项**：S 7 / P 8 / L 7）、`src/tests/check_pedestrian_shots.gd`（**新建**，**14 项**：A 4 / R 10；headless 档 4 + 10 `[skip]`）、两个自动生成的 `.gd.uid`、`COMPONENTS.md`（顶部状态段 / §1.4 / §3 脚本表两行 / **§4.11.2** / §9.1 **E29** / §11）；工程外 `tmp/task35/{e2e_drive.py,s5_e2e.sh,s5_checks.sh,s5_metrics.md,e2e_report.md,impl.md,pose_*.csv,shots/ped_*.jpg,logs/*}` | Step 5 **端到端验收**（**生产代码一字未改**，Step 1~4 的产出物全部当黑盒）。**① 直行**：Python → WS → 世界，10.100 世界秒位移 10.102 m（`v_eff=1.0003 m/s`，+0.03%）、`yaw_max=0.000°`、`Δy=0.0000 m`、pose 10.10 Hz。**② 转弯**：`dir` 阶跃 0→π/2，只用**严格内部点**拟合得 `180.0 °/s`（被 clamp 的端点会把 180 压成 160.7 —— 第一版实测）、`conv=0.600 s`（中间经过 15/33/51/69/87 五个 yaw ⇒ 不是瞬转）、Kasa（**零 numpy**，手写 3×3 高斯消元）`r_fit=0.3199 m` vs `v/ω=0.3183 m`（+0.5%）、弧长差 −2.5%。**③ 超时**：停发后 `stop_latency=0.384 s`（= 世界 300 ms 阈值 + 一条 pose 量化）、冻结 1.100 世界秒位移 **0.000000 m**、末条 `v=(0,0)`、世界日志 `pedestrian idle` **恰 1 行**。**④ 人撞车**：车静在世界 `(60,84)` 朝东、人自 `(62.5,84)` 以 1.2 m/s 正碰后**再推 300 物理帧** —— 逐帧 `gap_min=0.019952 m`（= 一帧步长 0.02，≥ −1 mm 不穿透）、**挡停**（最后 60 帧前进 0.000000 m、横向 0.000000 m）、**车位移 0.000000000 m**、`Δyaw=0`、`min(ped.x−car.x)=0.4200 m`（没被挤穿）；再用 0.05 m/s 贴实 60 帧量得真实车头→胶囊表面 `0.1508 m`（`\|0.150−v\|=0.0008`）。**⑤ 雷达可见**：雷达装**车头**（`x=0.15`、`z=0.6`）扫 `ScanOnce()`，±45° 扇区最近 `2.2500 m` = 2.50 − 0.25（差 **0.0000**）、`ranges[0]` 与该最小值一致、扇区 11 条命中（≈11.5° 张角 = 一个人的宽度）、后向扇区只有 11.31 m 外的 `wall_c`、把人挪到 10 m 外变 `9.7500 m`。**⑥ 三帧真渲染**（Xvfb `:99` + llvmpipe，**绝不 `--headless`**、必带 `--audio-driver Dummy`）：`tmp/task35/shots/ped_{walk,turn,stop}.jpg` = 31 123 / 31 317 / 32 122 B、SOF0 均 1280×720、`var 0.0235~0.0242`、逐字节差 0.669/0.681/0.682、框内灰度差 0.068~0.100、框水平中心 **74.2 / 141.6 / 67.5 px**、`h_frac 0.375/0.525/0.530`；`R7` 此帧 `yaw=90.00°` 且转向段 60 Hz 逐帧拟合 **180.0 °/s**；`R8` 冻结 15 帧 `Δpose=0.000000 m` + `speed_scale=0.000` + 停帧与前两帧各差 ≥ 67 px。**回归**：`bash tmp/task35/s5_checks.sh` → `FAILS=0 / ALL CHECKS GREEN`（`--quit` EXIT=0/0 ERROR + **21 项 headless** + **5 项 Xvfb**），既有 19 项 `total` 逐字不变（`check_ws_manager` 按本文 §1.4 的 **76 / 77**，非 plan.md §2.4 过期的 73）。**不做** SLAM / 建图 / 动态障碍 / 避让 / YOLO / 图传；**不碰** `pleiades-*` / `Task/` / `programs/` / 根 Cargo；**未起停 Xvfb**（用现成 `:99`）；**不执行** `git add` / `git commit` |
| task_38-2 | `src/world/StreetLayout.gd`（**新建**）、`src/world/Facade.gd`（**新建**）、`src/tests/check_street.gd`（**新建**）+ 两个自动生成的 `.gd.uid`、`src/world/World.gd`（**只加不删**：2 个 preload / `STREET_NODE`·`FACADE_NODE` / `_ready()` 末尾两行 / `Get_Street_Root`·`Get_Facade_Root`）、`src/world/WorldLook.gd`（+`ASPHALT_TILE_M`·`CURB_TILE_M`·`FALLBACK_COLORS`·`Make_Pbr_Material()`，既有函数一字不改）、`COMPONENTS.md` | Step 2 街道几何 + 建筑立面（**纯渲染、零碰撞、`.tscn` 零改动、0 字节新资产**）：主路 `(120×8)` / 支路 `(8×24)` 顶面 `z=0.01`；标线 **28** 个四边形（主虚线 **18** ∈ 20±2 / 边线 **2** / 支虚线 **4** ∈ 4±1 / 斑马线 **4**）；路缘 **7 段 × 5 面 = 35**（高 0.15，斑马线与 `x∈[44,52]` 路口断开，**无底面**）；立面 **窗 60 / 基座 44 / 压顶 15**，`Street` 与 `Facade` 各 **3 个 `MeshInstance3D`**。`check_street.gd`：**S 组 28 项**（headless 也全跑）+ **R 组 13 项**（仅真渲染器，headless 打印 13 条 `[skip]` 且不计入 total）→ `headless total=28 failed=0` / **`Xvfb total=41 failed=0`**，两条均 `SELF-CHECK OK` + `EXIT=0` + 0 ERROR；落盘 3 张 1600×900 街景（`261 789 / 172 797 / 285 920 B`），斑马线白条均值 `0.9379` vs 间隔 `0.4727`（差 `0.4652` ≥ 0.10）、屏内窗 11 个中暗窗占 `0.91` 聚合均值 `0.1055`、窗内偏窗均值 > 0.12 的采样占 `0.0000`。**18 个既有 headless 自检断言数与基线逐字一致**（`project` 无 total / `world 26` / `camera 32` / `camera_sensor 18` / `camera_shots 18` / `vehicle 40` / `diff_drive 41` / `quadrotor 48` / `ws_manager 76` / `lidar 48` / `pose 23` / `bounds 12` / `models 24` / `sensor_models 29` / `pedestrian 40` / `pedestrian_physics 22` / `pedestrian_shots 4` / `world_look 30`，全 `failed=0`）；Xvfb 档 `camera_shots 35` / `camera_sensor 28` / `pedestrian_shots 14` / `world_look 43` 同样 `failed=0`（观感像素随沥青/立面自然漂移，阈值未放宽）。`--import` / `--quit` 均 EXIT=0 且 0 ERROR；`World.tscn` md5 不变、`assets/` 仍 `24 306 445 B`。**不碰** `OBSTACLES` / `PEDESTRIANS` / 世界尺度 / 碰撞层；**不写** shader；**不做**点缀物（红绿灯/路牌/车/树）、不做坡道、不做夜间发光窗；**不执行** `git add` / `git commit` |

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
| `src/scenes/Vehicle.tscn` | ✅ Step 4 `CharacterBody3D` + `Body`(`Nose`) + `CollisionShape3D` + `Chassis`/`Sensors` 两个空挂载点；`collision_layer=4`、`collision_mask=7`；6 个 `@export` 节点引用靠 `node_paths` 绑定（E16）。**task_33 Step 3**：`ModelUGV` 换 `res://assets/chassis/ugv_chassis.glb`，占位 `BoxMesh`/`BoxShape3D` 改 `0.30 × 0.25 × 0.25`、`Body`/`CollisionShape3D` 抬到 `y=0.125`（模型 = 碰撞盒，D11） |
| `src/vehicle/Vehicle.gd` | ✅ Step 4 装配（挂底盘 / 挂传感器 / 卸传感器）+ 物理步进（`move_and_slide()` 是唯一位置更新点）+ 位姿 / 采样查询；含 `apply_motion` / `sample_sensors` / `get_pose` 三个契约别名。**task_33 Step 3**：`AttachSensor()` 支持可选 `mount.pitch`（度，缺省 0 = 不预置任何角度；`rotation = (deg_to_rad(pitch), -yaw, 0)`，正 = 抬头 / 负 = 低头），失败语义完全不变 |
| `src/tests/check_vehicle.gd` | ✅ Vehicle 自检（Step 4；`SELF-CHECK OK` + `exit 0`，**40 项断言**：task_33 Step 3 增 `placeholder-size`（占位 = `0.30 × 0.25 × 0.25`）与 `mount-pitch-default` / `mount-pitch-down` / `mount-pitch-up`） |
| `src/scenes/chassis/DiffDrive.tscn` | ✅ Step 5 差速底盘组件（`Node3D` + `Wheels` + `WheelFL/FR/RL/RR`；`CylinderMesh` r=0.06 h=0.04 纯视觉；无内嵌脚本）。task_33 Step 3：占位轮位随默认轮距改 `z=±0.125` |
| `src/chassis/ChassisModel.gd` | ✅ Step 5 底盘基类（4 个契约方法 + `GetParam`/`SpeedPct`/`ActionOf`/`RateToRad`/`Approach`/`ApplyMotionMode`/`ApplyBodySize`；不含运动学） |
| `src/chassis/DiffDrive.gd` | ✅ Step 5 差速底盘（`MOTION_MODE_GROUNDED` + 世界侧速度环 + 差速运动学；只给控制量，不写 position）。task_33 Step 3：默认 `track_width 0.24 -> 0.25`、`height 0.16 -> 0.25`（几何 = `ugv_chassis.glb` 的 AABB，D11） |
| `src/chassis/ChassisCatalog.gd` | ✅ Step 5 底盘目录（`SCENES` / `DEFAULTS` + `Has` / `GetScene`（`load()`+缓存）/ `GetDefaults`；`quadrotor_uav` 已于 Step 7 落地）。task_33 Step 3：`DEFAULTS["diff_drive_ugv"]` 的 `track_width/height` 同步 0.25/0.25 |
| `src/scenes/chassis/Quadrotor.tscn` | ✅ Step 7 四旋翼底盘组件（`Node3D` + `Rotors` + `RotorFL/FR/RL/RR`；`CylinderMesh` 平盘 r=0.12 h=0.02 `radial_segments=16` 纯视觉；无内嵌脚本、无 `uid`） |
| `src/chassis/Quadrotor.gd` | ✅ Step 7 四旋翼底盘（`MOTION_MODE_FLOATING` + 世界侧速度环 + 三轴运动学：平移 / 爬升 / 偏航；不施加重力、不写 position）+ **task_32 Step 3 高度软件限幅**（`ClampAltitude()`：宿主车身世界 z 夹到 `[0, 31.8]`，越界 `velocity.y = 0` + 1 s 限流 warn；用 `global_transform`，不动运动学与参数表） |
| `src/tests/check_diff_drive.gd` | ✅ DiffDrive 自检（Step 5；`SELF-CHECK OK` + `exit 0`，41 项断言：形态 / 目录装配 / 运动语义 / 撞墙会停 / 白盒边界） |
| `src/tests/check_quadrotor.gd` | ✅ Quadrotor 自检（Step 7；`SELF-CHECK OK` + `exit 0`，**48 项断言**：形态 / 目录装配 / 无重力悬停 / 升降 / 平移 / 偏航 / 落地 / 边界与白盒 + task_32 Step 3 的 3 项高度软件限幅） |
| `src/tests/doubles/{TestChassis,TestSensor}.gd` | ✅ Step 4 自检替身（鸭子契约；`PackedScene.pack()` 现场打包，不落 `.tscn`、不进 §3 组件清单） |
| `src/tests/check_project.gd` | ✅ 工程骨架自检（`SELF-CHECK OK` + `exit 0`） |
| `src/tests/check_world.gd` | ✅ 世界自检（Step 2；Step 3 相机断言改指 `ObserverCamera`，`SELF-CHECK OK` + `exit 0`，26 项断言） |
| `src/tests/check_camera.gd` | ✅ 相机自检（Step 3；`SELF-CHECK OK` + `exit 0`，32 项断言，含 4 项物理不变量） |
| `src/scenes/WebSocketManager.tscn` | ✅ Step 6 组件（`Node` + `Heartbeat(Timer, 1 s, autostart)`；无内嵌脚本、无节点引用导出 → 不需要 `node_paths`） |
| `src/net/WireProtocol.gd` | ✅ Step 6 协议唯一入口（7 个消息名：`chassis_hello` / `chassis_ready` / `sensor_hello` / `sensor_ready` / `motion_cmd` / `assembly_ready` / **`pose`（task_31 Step 1）**；`imu` / `gnss` 已删除不再预留；`ParseJson`/`MakeJson`/`MakeJsonText`/**`BuildPose`**/`BuildLidar2DFrame`/**`BuildCameraFrame`（task_34-3）**/`ParseFrameHeader`/`FramePayloadSize`） |
| `src/net/ComponentLink.gd` | ✅ Step 6 单连接封装（`RefCounted`；`Init`/`Poll`/`TakePackets`/`SendJson`/`SendBinary`/`NextSeq`/`IsOpen`/`IsConnecting`/`Close`（void）/`Describe`；`last_error`）。**task_34 Step 3**：`MAX_FRAME_BUFFER_BYTES = 4 MiB`，`Init()` 在 `accept_stream()` **之前**设 `inbound_buffer_size` / `outbound_buffer_size`（默认 64 KiB 撑不住一张 720p JPEG） |
| `src/net/WebSocketManager.gd` | ✅ Step 6 服务端（监听 / accept / 两张表 / hello 装配 / manifest 就绪 / `assembly_ready` / 断开清理 / `Heartbeat` 巡检；全静态实例、无 `push_error`；`UAV_ALTITUDE_M` 于 task_32 Step 3 改 **2.5 m**）+ ✅ task_31 Step 1 位姿下行（`POSE_BROADCAST_INTERVAL_S = 0.1` / `_pose_acc` / `_BroadcastPoses()`，只发已握手且活着的底盘连接） |
| `src/tests/check_ws_manager.gd` | ✅ WebSocketManager 自检（Step 6；`SELF-CHECK OK` + `exit 0`，**76 项断言**（headless）/ **77 项**（Xvfb）：场景形态 / World 接线 / 按序装配 / 运动下行 / 传感器先到 / 空 manifest / 断开清理 / 错误路径 / 规范 lint；Step 8 后 `C14 sensor-geometry-consistency` 由"双双为假"变为**双双为真**；**task_34 Step 3**：+`A9 camera-frame`（`type=2` 帧格式）+`A10 link-buffer-size`（4 MiB 双缓冲）+ **J 段** `J1 camera-sensor-ready` / `J2 camera-geometry` / `J3 camera-stream-bound` / `J4 camera-no-collision` / `J5 camera-assembly` / `J6 camera-stream-gate` + `[skip] J7 camera-payload-jpeg`（无渲染器不计入 total），`Sim` 额外按序记录 `binaries`） |
| `src/scenes/sensors/Lidar2D.tscn` | ✅ Step 8 传感器组件（`Node3D` + `DebugPoints(MeshInstance3D, visible=false)`；`node_paths=PackedStringArray("debug_points")`；无内嵌脚本、无 `uid`）。task_33 Step 3：加 `Body`（`lidar_tmini.glb` 实例根，identity，**无碰撞**，E27） |
| `src/sensors/Lidar2D.gd` | ✅ Step 8 2D 单线激光（360 条水平射线 / 10 Hz / `BuildLidar2DFrame` 二进制下发 + `LaserScan`；不写 position、不发 JSON、不 preload `ComponentLink`） |
| `src/sensors/SensorCatalog.gd` | ✅ Step 8 传感器目录（`SCENES` / `DEFAULTS` + `Has` / `GetScene`（`load()`+缓存）/ `GetDefaults`；与 `ChassisCatalog` 同构）。**task_34 Step 3**：`SCENES` 加 `camera`（→ `CameraSensor.tscn`）、`DEFAULTS` 加 `camera = {640×480, fov 70.0, quality 0.8, 1 Hz}`（装配线口径，与组件常量 1280×720 是两个口径，见 §7.6） |
| `src/tests/check_lidar.gd` | ✅ Lidar2D 自检（Step 8；`SELF-CHECK OK` + `exit 0`，**48 项断言**：场景形态 / 目录 / 装配 / 一圈几何 / 10 Hz 时序 / 帧格式 / 边界与白盒） |
| `src/tests/check_pose.gd` | ✅ 位姿下行自检（task_31 Step 1；`SELF-CHECK OK` + `exit 0`，**23 项断言**：`A1~A7` 协议形态与白盒 lint（E23 三条禁止）/ `B1~B2` World 接线 / `C1~C14` 端到端（10 Hz = 60 帧 10 条、8 键字段、与 `spawn` 一致、yaw 范围、`world_time` 单调、传感器连接 0 条、forward 的 `x`/`vx`、left 的 `yaw`/`vy`、断开后不再发、`vehicle==null` 守卫、每周期每连接恰好一次）） |
| `src/tests/check_bounds.gd` | ✅ 空气墙 / 顶盖自检（task_32 Step 2；`SELF-CHECK OK` + `exit 0`，**12 项断言**：`B1` 结构 / layer / 无 mesh + `B1b` 四面墙内侧面贴边界 + `B1c` 顶盖下表面 32 m + `B1d` 由代码构建（`.tscn` 无 `Bounds`）+ `B1e` 幂等重建 + `B2~B5` 四面 20 m/s 冲撞 10 s 出不去 + `B6` 5 m/s 爬升被顶盖挡住 + `B7` 地面自由落体静止 + `B8` 可见 2 m 矮墙各 1024 格仍在） |
| `src/world/WorldLook.gd` | ✅ task_38 Step 1b 观感唯一入口（`RefCounted` 全 `static`，§4.1.1）：三贴图 PBR 材质工厂（`Make_Ground_Material` / `Make_Voxel_Material` / `Make_Fallback_Voxel_Material`）、`PanoramaSkyMaterial`（`Make_Sky_Material`，`SKY_ENERGY = 0.5`）、太阳对齐 + 能量（`Apply_All`，`SUN_DIR = (-0.4313, 0.6817, 0.5910)`、`SUN_ENERGY = 0.7`）、幂等缓存与缺失清单（`Get_Missing_Assets` / `Reset_Cache`）；**缺资产只 warn、降级为原单色** |
| `src/tests/check_world_look.gd` | ✅ task_38 Step 1b 世界观感自检（**S 组 30 项** headless 也跑 + **R 组 13 项**仅真渲染器）：三贴图非 null / `uv1_triplanar && uv1_world_triplanar` / `uv1_scale = 1/6` 与 `1/2` / 天空是 `PanoramaSkyMaterial` 且 `panorama` 2048×1024 / `bg=2 & ambient=3` / `Ground` 几何与 AABB 未变 / `Sun` 与 `SUN_DIR` 的 `dot > 0.9999` / 体素格数不变 / 降级材质 + `Apply_World_Look()` 幂等；R 组落盘 3 张 **1600×900** JPEG（`tmp/task38/shots/`）+ 2 张 160×90 太阳探针。`headless total=30` / `Xvfb total=43`，均 `failed=0` |
| `src/world/StreetLayout.gd` | ✅ task_38 Step 2 街道唯一入口（`RefCounted` 全 `static`，§4.1.2）：常量表 `ROADS`(2) / `EDGE_LINES`(2) / `CROSSWALK` / `CURBS`(7) / `MAIN_SKIP_X`，递推生成主虚线 18 / 边线 2 / 支虚线 4 / 斑马线 4 = **28** 个四边形，`_Add_Box_Quads()` 出 7 段 × 5 面 = **35**；`SurfaceTool` 合并成 `Road`/`Markings`/`Curbs` **3 个 `MeshInstance3D`**，`Build_All(world)` 幂等（先摘同名旧容器）、`Get_Root(world)`；**零碰撞、无 RNG、不写 shader** |
| `src/world/Facade.gd` | ✅ task_38 Step 2 立面唯一入口（同风格，§4.1.2）：按 `World.OBSTACLES`（经 `get_script_constant_map()` **只读**）生成窗 **60** / 基座 **44** / 压顶 **15** 个四边形，合并成 `Windows`/`Plinths`/`Caps` **3 个 `MeshInstance3D`**；纯函数 `Floors_Of(h)`（字面公式 + 2.5 m 一档下限）/ `Bays_Of(w)`（`max(1, floor(w/2.4))`，面宽 < 2 m 不开窗）供自检独立复算；`Build_All(world)` 幂等、`Get_Root(world)`；**零碰撞、不写 shader** |
| `src/tests/check_street.gd` | ✅ task_38 Step 2 街道 + 立面自检（**S 组 28 项** headless 也跑 + **R 组 13 项**仅真渲染器）：从 mesh 顶点反推四边形（`verts.size() == 4 × quads`）→ 路网 AABB `(120,8)`/`(8,24)` 与顶面 `z=0.01` / `Street`+`Facade` 子树零 `CollisionObject3D`·`CollisionShape3D` / 标线 28（主虚线 18∈[18,22]、边线 2、支虚线 4∈[3,5]、斑马线 4）/ 路缘 7×5=35 与断口覆盖率探针（1200+1200 点）/ 窗 60 逐栋逐面 / 基座 44（距墙 0.01 且朝外）/ 压顶 15（15 个 z 两两不同）/ 连调两次节点数与 mesh 数不变 / `OBSTACLES` 15 条逐位不变 / 尺度与四个 layer 不变；R 组落盘 3 张 **1600×900** JPEG（`tmp/task38/shots/street_0{0,1,2}_*.jpg`）+ 窗暗度 / 窗无斑驳 / 斑马线对比度。`headless total=28` / `Xvfb total=41`，均 `failed=0` |
| `src/tests/check_models.gd` | ✅ 低模外观 + 传感器模型接入自检（Step 2 / task_33 Step 3；`SELF-CHECK OK` + `exit 0`，**24 项断言**：`M1~M8` 模型显隐 / AABB / 车头朝向 / 碰撞盒 = 模型 / 幂等 / 换底盘循环 + `M3b` 模型 = 碰撞盒 + `S1~S7` 两个传感器场景的 `Body` 挂载 / AABB / 光心 / 零碰撞 / `pitch`） |
| `assets/humans/README.md` | ✅ human_model Step 4 资产说明（来源 URL `https://renderpeople.com/sample/free/renderpeople_free_rigged_people_FBX.zip` + 许可原文 + 处理方式（单位修正 / 缩放 / 朝向 / 贴图 / 三角数）+ 导入副产物 + 换备选方法 + 使用边界 + 摆位差异） |
| `assets/humans/LICENSE_renderpeople.txt` | ✅ Renderpeople 免费页条款原文（883 B，`sha256 aa4f1583…`，与 `tmp/human_model/normalized/` 原件逐字节相同） |
| `assets/humans/rp_carla_rigged_001/{model.glb,model.glb.import,model_rp_carla_rigged_001_dif.jpg(+.import),model_rp_carla_rigged_001_norm.png(+.import)}` | ✅ **首选**人形（`2 664 856 B` / `sha256 bc9677b8…` / 1.7307 m / 19 988 tri / 1 mesh / 1 surface / 无骨骼无动画 / 脚底 y=0 / 朝 +X）；贴图与 `.import` 是 Godot 抽取的 sidecar（E28），**必须一起提交** |
| `assets/humans/rp_eric_rigged_001/{同上 6 个文件}` | ✅ **备选**人形（`2 438 336 B` / `sha256 12e959ba…` / 1.8400 m / 20 542 tri）；**在库但未摆进世界**，换用只改 `Person.tscn` + `Person.MODEL_GLB` 两处 |
| `src/scenes/props/Person.tscn` | ✅ human_model Step 4 人物道具（§3 第 9 个 `.tscn`）：`Node3D` + `Body`（`rp_carla_rigged_001/model.glb` 静态实例，identity）+ 根脚本 `Person.gd`；**无 CollisionShape3D**、无 `node_paths`、无内嵌脚本；`load_steps=3` |
| `src/props/Person.gd` | ✅ **唯一**人物脚本（human_model Step 4 静态 + walking_person Step 3 走动**并入同一文件**）：静态常量 `MODEL_GLB` / `HEIGHT_MIN_M`(1.60) / `HEIGHT_MAX_M`(1.85)、走动常量 `WALK_GLB` / `CLIP_MPS`(1.260)，导出字段 `walk_speed`(默认 0.0=静态) / `path` / `loop`，`Configure` / `Set_Progress_M` / `Get_*` / `Path_Hits_Rect`，闭合路径弧长采样 + `_physics_process` 推进 + 动画播放（`find_child("AnimationPlayer", true, false)` → 播第 0 条 → `speed_scale = walk_speed/CLIP_MPS`）；**默认静态、零碰撞、不进装配线**（§4.9 / §4.10） |
| `src/tests/check_person_assets.gd` | ✅ human_model Step 4 自检（`SELF-CHECK OK` + `exit 0`，**32 项断言**：A 组资产 16（每件 loads / instantiates / aabb / feet / no-collision / material / tri-budget / sha256）+ B 组 `Person.tscn` 5 + C 组常量自洽 2 + D 组世界摆位 9（root / count / placement / feet / no-collision / idempotent / spawn-clear / obstacle-clear / not-vehicles）） |
| `assets/humans/rp_nathan_animated_003_walk/{model.glb,model.glb.import,model_rp_nathan_animated_003_dif.jpg(+.import),model_rp_nathan_animated_003_norm.png(+.import)}`（**共 6 个文件**；Step 3 只把目录改名，`glb` 字节与 `sha256` 不变） | ✅ **走路人形**（walking_person Step 2/3）：`3 263 492 B` / `sha256 0e907fc9…` / **1.8490 m**（rest 姿态；动画峰值 1.8400 m）/ 21 506 tri / 88 骨蒙皮 / 1 条动画 `2.2333 s`（68 键，已裁源首帧）/ 脚底 `y=0.000510` / 朝 +X；**只有这一件带骨骼与动画** |
| `src/scenes/props/PersonWalking.tscn` | ✅ walking_person Step 2/3 走路人物道具（§3 第 10 个 `.tscn`）：`Node3D` + `Body`（`rp_nathan_animated_003_walk/model.glb` 实例，identity）+ 根脚本 **`Person.gd`**（与 `Person.tscn` 同一个脚本）；**无 CollisionShape3D**、无 `node_paths`、无内嵌脚本 |
| `src/tests/check_person_walk.gd` | ✅ walking_person Step 3 / **task_35 Step 3 改写**自检（`SELF-CHECK OK` + `exit 0`，**15 项断言**：A 组资产 8 + B 组场景 4 + C 组世界 3（世界预置行人：容器形态 / 出生点净空 / 幂等重建）；D 组运行 5 + R 组渲染 6 已随自动绕圈一并移除 → headless 与 `DISPLAY=:99` **两档都是 `total=15`**） |
| `src/tests/check_person_walk_shots.gd` | ✅ walking_person Step 4 / **task_35 Step 3 换主语**自检（`SELF-CHECK OK` + `exit 0`，**14 项断言**：A 组结构 4 + R 组渲染 10；`--fixed-fps 60` 真推进 0/40/80 物理帧拍 `walk_t{0,1,2}.jpg`，帧间不瞬移；主语 = `World.PEDESTRIANS[0]` 的 `pedestrian_01`，驱动 = **一次** `SetCommand({"v": 1.26, "dir": 0.0})`，尺子 = `GetPose().x` 的 Δx；headless 下 R 组 10 项 `[skip]` 不计入 total → `total=4`，`DISPLAY=:99` 下 `total=14`） |
| `src/world/World.gd` | ✅ Step 2 世界根脚本 + **task_32 Step 2 空气墙 / 顶盖**（`_Build_Bounds()`、`World/Bounds` layer 1、5 个 box、`AIR_WALL_*` 常量与 `AIR_WALL_BOXES` 几何表；`.tscn` 零改动）+ **task_35 Step 3 世界预置行人**（`PEDESTRIANS` / `BuildPedestrians()` / `GetPedestriansRoot()`，`WALKERS` / `BuildWalkers()` 已删；`.tscn` 仍然零改动） |
| `COMPONENTS.md` | ✅ 本文 |
| `assets/sensors/lidar_tmini.glb` | ✅ task_33 Step 1 雷达模型（17 444 B / 324 tri / 4 网格 / 材质 `ORION_MetalGray` + `ORION_GlassBlue`(BLEND) + `ORION_MatteBlack`；原点 = 罐体底面中心，AABB `0.080 × 0.054 × 0.080`） |
| `assets/sensors/camera_module.glb` | ✅ task_33 Step 1 相机模组（11 944 B / 208 tri / 4 网格；原点 = 镜片前表面中心，AABB `0.055 × 0.036 × 0.030`） |
| `assets/chassis/ugv_chassis.glb` | ✅ task_33 Step 1 UGV 外观（21 888 B / 416 tri / 8 网格；原点 = 车体底面中心、车头 +x，AABB **`0.30 × 0.25 × 0.25`**，与 D11 的碰撞盒逐轴相等） |
| `src/tests/check_sensor_models.gd` | ✅ task_33 Step 1 资产自检（`SELF-CHECK OK` + `exit 0`，**29 项断言**：每件 9 项（loads / instantiates / root-identity / aabb-size / origin / no-collision / materials / tri-budget / file-size）+ `uav-loads` + `uav-aabb`（复核锁定 `0.64 × 0.14 × 0.64`）） |
| `Tool/blender/models/build_models.py` | ✅ task_33 Step 1 bpy 构建脚本（**工程根下，不在 `orion-simulation-world/` 内**；`Build_Lidar` / `Build_Camera` / `Build_Ugv` 各一函数、参数集中在顶部常量区、建完自断言 AABB 与三角数，不符即 `RuntimeError` → 进程 EXIT≠0） |
| `Tool/blender/models/export_all.sh` | ✅ task_33 Step 1 一键导出（`blender -b -P build_models.py` → `blend2glb.sh` ×3 → `file` + 体积校验，`set -euo pipefail`） |
| `Tool/blender/models/{lidar_tmini,camera_module,ugv_chassis}.blend` | ✅ task_33 Step 1 模型源文件（各 ~90 KB，可在任意 Blender 里继续手工编辑） |
| `src/scenes/sensors/CameraSensor.tscn` | ✅ task_33 Step 2 相机组件（`Node3D` + `Viewport(SubViewport 1280×720, own_world_3d=false, update_mode=0)` + `Lens(Camera3D fov=70 near=0.05 far=200 current=true)`；`node_paths=PackedStringArray("viewport","lens")`；无内嵌脚本、无 `uid`）。task_33 Step 3：加 `Body`（`camera_module.glb` 实例根，`rotation_degrees=(0,90,0)`，不写 position，**无碰撞**，E27） |
| `src/sensors/CameraSensor.gd` | ✅ task_33 Step 2 相机（`Configure` / `Capture_Jpeg` / `Capture_To_File` / `Get_Image_Size` + `Can_Capture`；warmup 2 帧 + `UPDATE_ONCE` + `await frame_post_draw` + `get_image` + `save_jpg_to_buffer`；抓拍前手工同步 `lens.global_transform`；四类失败 `push_error` + 空数组，路径类失败只 `push_warning`；无碰撞体、无物理 API、不碰 socket）。**task_34 Step 3**：+`rate_hz`（`DEFAULT_RATE_HZ=1.0` / `MIN_RATE` / `ACCUM_EPS`）+ `frame_produced` 信号 + `SetStream`/`ClearStream`/`HasStream`/`Tick_Stream`/`Emit_Frame`/`Get_Tick_Count`/`Get_Frame_Count` + `_Capture_Tick()`/`_Warn_No_Renderer()`/`_exit_tree()`；`_process` 只转发累加器；`Can_Capture()==false` 时**只 warn 一次、绝不出帧、绝不 await**（§4.8.3） |
| `src/tests/check_camera_sensor.gd` | ✅ task_33 Step 2 相机自检 + **task_34 Step 3 出帧断言**（`SELF-CHECK OK` + `exit 0`，**A 组 18 项 + B 组 10 项**；headless 下 B 组 `[skip]` 不计入 total → `total=18`（`[skip]` ×10），Xvfb 下 `total=28`。新增 `A13` 流绑定 / `A14` 帧头 / `A15` seq+空图 / `A16` `rate_hz` 节奏 / `A17` 无渲染器闸门 / `A18` 目录登记 / `B10` 真抓拍→真帧，用内联 `StubLink` + 手工 `Tick_Stream()` 推步，不依赖墙钟与像素） |
| `src/tests/check_camera_shots.gd` | ✅ task_33 Step 4 模拟世界拍照自检（`SELF-CHECK OK` + `exit 0`，**S 组 18 项 + R 组 17 项**；headless 下 R 组 `[skip]` 不计入 total → `total=18`，Xvfb 下 **`total=35`** 并落 3 张 1280×720 JPEG 到 `tmp/task33_shots/`；§4.8.2） |
| 目录骨架 | ✅ `src/scenes/`（含 `chassis/` `sensors/`）+ `src/{common,world,camera,vehicle,chassis,sensors,net}/` + `src/tests/`（含 `src/tests/doubles/`）+ `assets/`（`.gitkeep` + `README.md` + 2 个旧 glb + `sensors/` + `chassis/`，`sensors/` 与 `chassis/` 为 task_33 Step 1 新增） |

**已全部落地**（§11 的 Step 1~8 均已按本文完成）。
> `quadrotor_uav` 已于 Step 7 落地（`src/scenes/chassis/Quadrotor.tscn` + `src/chassis/Quadrotor.gd`），
> `ChassisCatalog.GetScene("quadrotor_uav")` 不再返回 `null`——§4.5.1-③ / §8.3 里"Step 7 前场景缺失 → 1011"是 **Step 6 的历史实测记录**，保留不改。
> `src/scenes/sensors/Lidar2D.tscn` / `src/sensors/SensorCatalog.gd` 已于 Step 8 落地：`_CatalogStatus()` 由 `CATALOG_MISSING` 转为
> `CATALOG_KNOWN`/`CATALOG_UNKNOWN` 两态，`_MountSensor()` 走"目录有 + 场景存在 → 真正实例化几何"分支，
> `sensor_ready` 之后 `SetStream()` 绑定下行流（§5.3 时序铁律），§4.5.1-③ / §8.3 里"Step 8 前只登记连接"同为历史记录。
> **task_31 Step 1（位姿下行）**：`pose` 文本帧已落地（`MSG_POSE` + `WireProtocol.BuildPose` +
> `WebSocketManager._BroadcastPoses`，10 Hz 只发底盘连接），自检 `src/tests/check_pose.gd`（**23 项断言**）。
> `MSG_IMU` / `MSG_GNSS` 两个预留常量已删除，§10 中 `imu` / `gnss` 由"预留"改为"**不做**"——位姿走 `pose`（§8.1-8 / E23）。
