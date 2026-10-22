# wb_30_sim_world — task_30 工作记录

> Presented by KeJi
> Created Date ： 2026-10-21
> Modified Date ： 2026-10-21
> 任务：Task/task_30_sim_world.md（sim 接入 Godot 模拟世界）

---

## Step 1 — 整体规划
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/plan.md](tmp/plan.md)（886 行）。 - **交付物**：`COMPONENTS.md`（§2 全场景节点树 / §3 七个 `.tscn` 清单 / §4 各组件节点树+接口 / §5–§6 挂载关系与数据流 / §7–§10 接口签名、wire 契约、坐标与规范）+ 可加载骨架（`project.godot`、占位 `World.tscn`、`tests/check_project.gd`、10 个目录）。 - **关键实测结论**：`--quit` 必须有主场景（否则 exit 1）→ 占位 `World.tscn` 是必要且唯一的越界边界；**全局 `class_name` 在无 `.godot` 缓存时解析失败**，因此全项目禁用、一律 `preload`/路径继承；`--quit` 对 parse error 仍返回 0，故验收固定为「exit 0 + grep 无 ERROR」两条。 - **设计全部对齐 task_30**：7 个场景路径与 §八 一致、尺度/端口（9090，避开 9010/8080）/物理层位值/坐标单入口 `WorldTransform`/二进制帧小端（已实测编码）逐项写死，并已用等价临时工程跑通 G1/G2/G3。 - **本 step 不写任何组件代码**：World/Camera/Vehicle/底盘/Manager/Lidar 留待 Step 2–8，plan §15 给了逐 step 的文件映射与验收要点。

## Step 2 — 世界组件
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/plan.md](tmp/plan.md)（403 行）。要点： 1. **改动范围**：新建 `scripts/common/WorldTransform.gd`、`WorldClock.gd`、`scripts/world/World.gd`、`tests/check_world.gd`，改写 `World.tscn`；`project.godot` 只读不改，不新增任何 `.tscn`。 2. **世界构成**：`GridMap` 采用稀疏程序化布局（边界墙、3 栋建筑、2 道长墙、1 根高柱、1 处三级退台、3 个散块，≈12.1k 格、构建 5~6 ms），另有一整块 `StaticBody3D` 加 `BoxShape3D(128,1,128)` 地面（顶面恰为 y=0），再加 `WorldEnvironment`（ProceduralSky，ambient=SKY）和带阴影的 `DirectionalLight3D`。 3. **唯一入口**：Z-up 到 Y-up 的换轴只出现在 `WorldTransform` 中，`WorldToCell` 通过它实现；尺度全部读取 `[sim_world]`；`world_time` 只来自 `WorldClock`。 4. **验证方式**：临时工程已端到端跑通——`--quit` 输出构建日志且退出码为 0，`--check-only` 全部为 0，`check_world.gd` 的 24 项断言（含从 5 m 落体静止于 y≈0.14）得到 `SELF-CHECK OK`；本机没有 DISPLAY，也没有 Xvfb，因此无法截图，改用 `get_meshes()` 非空、Sky 与阴影生效、`is_position_in_frustum(世界中心)` 三项共同替代，另外给出带显示机器上的人

## Step 3 — 观察者相机
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/plan.md)（490 行）。 1. **新建** `ObserverCamera.tscn`（`Camera3D`，默认位姿 `(64,26,108)/-30°`，`current/fov 70/near .05/far 500`）+ `scripts/camera/ObserverCamera.gd`（零依赖、`enum Mode{FREE,ORBIT,FOLLOW}`、只用 `_unhandled_input`、三模式算法与 `SetMode` 保连续切换全部定案）+ `tests/check_camera.gd`（A1~A27 断言）。 2. **修改** `World.tscn`（加实例、删 `DebugCamera`、`load_steps` 8→9）、`tests/check_world.gd` 的相机断言（否则删 `DebugCamera` 会让 Step 2 自检直接回归失败）、`COMPONENTS.md` 的 §1.1/§2/§4.2/§11/附录。 3. 关键引擎事实已用 `/tmp` 探针实测：`Input.parse_input_event` 真能进 `_unhandled_input`、`Input.action_press` 可驱动 `get_axis`、headless 无 `FEATURE_MOUSE`（需守卫）、环绕公式 `dot==1.0`；并发现**轨迹重放对比不可靠**（错开 1 帧即差 0.158 m），故解耦证明改用对齐无关的"静止不被唤醒 + 自由落体二阶差分 vs `-g·dt²` + 同帧双体对照"。 4. 验收给到 8 条可复制命令（`--quit`/grep/`--check-only`/三个自检脚本/解耦 grep）与窗口内手工点验单；风险表 20 条覆盖类型推断、Floa

## Step 4 — Vehicle 组件
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/plan.md](tmp/plan.md)（710 行），核心要点： 1. **只交付一个组件**：新建 `Vehicle.tscn`（`CharacterBody3D` + 上抬半高的 `Body/Nose` 朝向标记 + `CollisionShape3D` + `Chassis`/`Sensors` 两个挂载点）与 `scripts/vehicle/Vehicle.gd`，外加 `tests/check_vehicle.gd` + 两个测试替身；`World.tscn`/`project.godot` 一律不动。 2. **接口**：`ApplyChassis(name, params[, scene])` / `AttachSensor(slot, scene, mount, params)` / `DetachSensor` / `SetBodySize` 装配面 + `SetMotionCommand`/`GetPose`/`SampleSensors`，并给 step 要求的 `apply_motion`/`sample_sensors`/`get_pose` 三行转发别名（命名冲突已在计划中仲裁并要回写 COMPONENTS.md）。 3. **实测结论已固化**：手写 `.tscn` 的 `@export` 节点引用必须 `node_paths=PackedStringArray(...)` 否则**静默 null**（新增 E16）；`PackedScene.pack()` 可造替身，Step 5/8 未落地也能验证"动态挂载"；`Step().yaw_rate` 是 Godot `rotation.y` 角速度、`GetPose().yaw` 符号相反。 4. **验收**：G1~G6 六条命令（`--quit` + grep、`

## Step 5 — 差速底盘 diff_drive_ugv
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/plan.md](tmp/plan.md)（472 行）。核心要点： 1. **范围固定**：只新建 `chassis/DiffDrive.tscn` 加上 `scripts/chassis/{ChassisModel,DiffDrive,ChassisCatalog}.gd` 和 `tests/check_diff_drive.gd`，回写 `COMPONENTS.md`；`Vehicle.gd`、`World.tscn`、`project.godot` 全都不动，因为 `Vehicle` 已经预留好 `ChassisCatalog` 这条解析路径。实测表明 `load(gd).call("静态函数")` 可用，静态目录通过 `load()` 加缓存即可接通，不用改调用方。 2. **实现形态**：基类只提供 4 个契约方法，再加速度环工具（`SpeedPct/ActionOf/Approach/RateToRad/ApplyMotionMode/ApplyBodySize`，供 Step 7 复用）；`DiffDrive` 按 §4.4 把 `action` 映射到 `velocity/yaw_rate`，用 3 m/s²、180°/s² 的斜坡收拢，`Step` 只返回控制量，位置仍由 `Vehicle.move_and_slide()` 唯一推进（D8）。自检对“前进 1.1375 m、后退 −1.1375 m、左转 +1.19 rad、右转 −1.19 rad”都有解析期望值。 3. **实测已定下的关键判定**：撞墙后 `move_and_slide()` 会自己把 `velocity` 归零，所以“撞墙会停”必须用位置来判定——真实 `World.tscn` 探针停在 **x=127.3499（车头 127.4999 贴 wall_eas

## Step 6 — WebSocketManager
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/plan.md](tmp/plan.md)（726 行）。临时生成的测试脚本已全部删除，工程 `--quit` 仍正常。 **要点（3–5 行）** 1. **改动范围**：新建 `WebSocketManager.tscn` + `scripts/net/{WebSocketManager,ComponentLink,WireProtocol}.gd` + `tests/check_ws_manager.gd`；只改 `World.tscn`（在 `Systems` 下挂实例、绑定 `vehicle_scene` + `vehicles_root=../../Vehicles`、把 `load_steps` 从 9 改成 11）和 `COMPONENTS.md`；`project.godot`/`Vehicle.gd`/底盘/相机全不动。 2. **实现形态**：manager 是单例 `Node`（不用 autoload），`TCPServer` + `WebSocketPeer.accept_stream()` 服务端；两张表分别由 `_peer_vehicles` 和每个 `ComponentLink` 的 `(peer_id, role, slot)` 承载；`manifest` 只作就绪记录，`assembly_ready` 每个 peer 仅发一次；传感器先到则挂起并回放，握手/pending 双超时 + 断开分级清理。 3. **关键实测已固化**（本轮探针）：headless 下 WS server 端到端可用；`WebSocketPeer.close()` **返回 void**（新增 E19）；`listen(0)` 可拿临时端口；实例节点上的节点引用需 `node_paths` + **相对该节点自身**的路径（`../../V

## Step 7 — 四旋翼底盘 quadrotor_uav
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [`tmp/plan.md`](tmp/plan.md)（531 行）。 **要点** 1. **只产出 3 个新文件、改 2 个文件**：`chassis/Quadrotor.tscn`、`scripts/chassis/Quadrotor.gd`（`extends ChassisModel`，与 `DiffDrive` 平级）、`tests/check_quadrotor.gd`；`COMPONENTS.md` 只回填落地状态与实测值，`ChassisCatalog.gd` 仅修一行过期注释 —— **不动** `ChassisModel`、`Vehicle`、目录 `DEFAULTS`、`WebSocketManager`、`World.tscn`。 2. **运动学**：`Configure()` 设 `MOTION_MODE_FLOATING` + `SetBodySize(size, 0.12, size)` + 旋翼摆位；`Step()` 复用基类速度环产出 `{velocity = global_basis*(v,0,0) + y=climb, yaw_rate}`，无竖向指令即 `climb=0` → 高度保持（无重力悬停），位置推进仍只由 `Vehicle.move_and_slide()` 负责（D8）。`left/right` 裁决为**原地偏航**，用"参数表无侧移参数 + §4.6 原文 + 单 action schema"三条证据固定，不改设计。 3. **数值已实测锚定**（`--fixed-fps 60` 探针，跑完已删）：水平 15/60 帧 = 0.10 / **1.35 m**，爬升 60 帧 = **0.8417 m**，偏航 60 帧 = **1.1912 rad**，落地 `y≈0.0001` 不打穿；同时实测到 *

## Step 8 — 传感器 lidar_2d
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/plan.md](tmp/plan.md)（472 行）。 **要点总结** 1. **只新增 4 个文件 + 改 1 份文档，不动既有组件**：`scripts/sensors/SensorCatalog.gd`、`sensors/Lidar2D.tscn`、`scripts/sensors/Lidar2D.gd`、`tests/check_lidar.gd`，以及 `COMPONENTS.md` 六处状态/实测补充。`SensorCatalog` 一落地，Step 6 的 manager 就从「目录缺失→只登记连接」自动切到「真正实例化几何」，manager/Vehicle/WireProtocol 一行都不用改。 2. **实现严格照 §4.7**：挂载点 `global_position` 起射、`a = TAU·i/rays`、mask `1|2|4`、`exclude=[宿主 RID]`、未命中 `0.0`、`link.NextSeq()`+`WorldClock.Now()` → `BuildLidar2DFrame`（1455 B）→ `SendBinary`，并额外提供 `GetLaserScan()` 输出 task_30 §7.4 的 `LaserScan`。 3. **本轮先在 4.7.2 上跑了探针，把 10 条实测事实写进计划**（P1–P10），其中三条直接影响实现：朴素累加器在 60 fps 下 1 s 只扫 9 次（需 `1e-6` 容差才恰好 10 次）、`ImmediateMesh` 没有 `surface_get_array_len`/`surface_get_primitive_type`（误用 → `SCRIPT ERROR` 且进程不退出，必须 `timeout`）、本机 9090 被 mihomo 占用（

## 2026-10-21 端口变更（人工介入）
- `sim_world/network/ws_port`：9090 → **9999**（本机 9090 被 mihomo 占用，`listen` 失败）
- 同步修改：`project.godot`、`scripts/net/WebSocketManager.gd`（`DEFAULT_WS_PORT` + 注释）、`tests/check_project.gd` 断言、`tests/check_lidar.gd` / `tests/check_ws_manager.gd` 注释、`COMPONENTS.md`
- 验证：`--quit` → `[ws] listening on port 9999`；`check_project` → SELF-CHECK OK；`check_ws_manager` → 51/51 OK
