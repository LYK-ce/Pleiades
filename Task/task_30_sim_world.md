# task_30_sim_world — sim 接入 Godot 模拟世界

> Created Date ： 2026-10-19
> Modified Date ： 2026-10-19
> 状态：**设计已定型，实现步骤已拆分**（8 个 step，见 §八）
> 关联文档：`Architecture/robot_arch.md`、`docs/design_doc/universal_robot_design.md`、`docs/design_doc/orion_protocol.md`

---

## 一、目标

给 `pleiades-sim` 接上一个**真正的模拟世界**（用 Godot 实现），使仿真具备物理、传感器与可视化：

- **物理**：位置/朝向由世界更新（世界权威），碰撞与接触在世界侧
- **传感器**：世界按物理模型渲染读数（LiDAR 真射线投射、相机渲染、IMU、GNSS）
- **可视化**：Godot 自带，调试直观

最终目标：**同一份决策链代码，在仿真里和真机上跑同一套逻辑**——从而在没有硬件的情况下验证多机协同、避障、感知算法（含 task_29 的 YOLO 闭环）。

### 为什么用 Godot

自研世界要写几何、物理、渲染、射线投射、可视化，是数月量级；Godot 全部现成。**但要清楚省下的是什么**：

| 自研要做 | Godot 提供 |
|---|---|
| 场景图 / 几何管理 | ✅ |
| 物理步进、碰撞 | ✅ GodotPhysics / Jolt |
| 渲染（相机传感器） | ✅ |
| 射线投射（LiDAR 底座） | ✅ `intersect_ray` / space state |
| 可视化调试视图 | ✅ |

**Godot 不提供传感器模型**：LiDAR 的扫描模式/线束/噪声、IMU 噪声、GNSS fix 质量仍要自己写，只是站在 raycast/render 之上。

---

## 二、现状（2026-10-19）

`pleiades-sim` 已有 **1756 行 / 11 文件**（决策链完整复制 + `SimDeviceHandler`），但：

- **车永远不动**：全项目 `RobotState` 写入点只有——base 的 origin 注入（`Robot/core/robot.rs:102`）、ugv 的 STM32 反馈 + `odometry::accumulate`（`ugv/stm32/mod.rs:83,96,277,289`）、ugv 的 RTK 覆盖（`device/lg290p/mod.rs:85`）。**sim 一处都没有。**
- `SimDeviceHandler::on_tick` 拿到的是 `rs: &RobotState`（**不可变引用**），只把动作 `info!` 打印出来
- 没有传感器、没有物理、没有障碍

**结论**：现在只能证明"决策链不崩"，证明不了"决策链能走通"。

---

## 三、核心模型（已定）

**世界里的 avatar = 机器人的"身体"；sim = 机器人的"大脑"。**

| | 位置 | 职责 |
|---|---|---|
| **avatar** | Godot 世界内 | 几何外形、物理（位置由世界更新）、挂载的传感器、执行器 |
| **sim** | 独立进程 | 决策链（与真机同一套） |
| **传感器组件** | sim 内 | 读 avatar 的传感器读数（感官） |
| **控制器组件** | sim 内 | 驱动 avatar 的执行器（肌肉） |

关键定性（这几条是讨论中反复澄清的，必须固定）：

1. **世界是"物理世界"的替身，不在机器人网络里，不参与任何 P2P。**
2. **libp2p 只属于 sim 之间**（车际通信，与真机同构）；**世界根本不碰 libp2p**，世界永远不是 swarm 节点。
3. **世界权威**：位置更新在世界侧；sim 的 `robot_state` 是同步过来的镜像；sim **不做运动学积分**（`odometry::accumulate` 在 sim 里不需要）。
4. **一切认知来自传感器**：连自身位姿也是 GPS/RTK/里程计**读出来的**，不是"世界告诉的"。
5. **世界的传感器模型是"真"的**：LiDAR 是从安装位姿出发对世界几何做**射线投射**得到的点云，不是查表返回语义。
6. **机器人身份 = `peer_id`**：世界以 `peer_id` 为键建 avatar，与车际网络里的节点是同一身份，不引入第二套身份体系。

---

## 四、架构

```
┌─ 世界模拟器（Godot，独立进程）──────────────┐
│  几何 / 物理 / 传感器模型 / 渲染 / 可视化      │
│  零件库：底盘、LiDAR、相机、IMU、GNSS …        │
└───┬──────────────────────────┬─────────────┘
    │ 传感器读数（下行）           │ 运动指令（上行）
    │ WebSocket（一组件一连接）    │ WebSocket
┌───┴──────────────────────────┴─────────────┐
│  sim 进程 A                                 │
│    传感器组件 / 控制器组件                     │
│    决策链（goal / executor / planning）       │
│    libp2p  ←──────────────→  libp2p  sim B  │ ← 车际通信，与世界无关
└────────────────────────────────────────────┘
```

### 4.1 组件分解

**世界组件**
- `GridMap` 体素填充（墙 / 障碍，稀疏）
- **地面 = 一整块**：`StaticBody3D` + `BoxShape3D`（或 `WorldBoundaryShape3D`），**不用 256×256 体素地面**（那是 6.5 万个碰撞体）
- 物理用 **Godot 内置**（`CharacterBody3D` / `move_and_slide()` / `intersect_ray()` / `Area3D`）

**Vehicle 组件**
- 本身是 **`CharacterBody3D`**（不是 `Node3D` 容器）
- **虚的**：没有固定形态，必须通过 WS 拿到"用什么底盘、用什么传感器"才拼装出来
- 拼装后：行为 / 行动模式 / 行动机制**由底盘决定**；感知能力**由传感器决定**

```
Vehicle (CharacterBody3D)              ← 物理体本身就是 vehicle
├── MeshInstance3D                     ← 车身外观（含朝向标记）
├── CollisionShape3D                   ← 形状/尺寸由底盘决定
├── ChassisModel                       ← 底盘策略：设 motion_mode + 算 velocity
└── Sensors[]                          ← 子节点（v1：Lidar2D）
```

### 4.2 观察者相机（Godot 内控制）

**相机在 Godot 里用输入控制**（不是 WS 下发）——人要在模拟世界里观看小车的行为。

> 注意区分：**小车**不在 Godot 里控制（在 sim 的决策链里）；**相机**在 Godot 里控制。

- 一个 `Camera3D` + 观察者相机脚本
- 三种视角，可切换：
  | 视角 | 输入映射 | 用途 |
  |---|---|---|
  | 自由飞行 | WASD 平移、右键拖拽转向、滚轮调速、Q/E 升降 | 看全场 |
  | 环绕 | 中键拖拽绕目标转、滚轮缩放 | 围着某台车看 |
  | 跟随 | 点选某台车 | 第三人称跟车 |
- 输入用 `_unhandled_input`（避免抢 UI 输入）
- **与仿真完全解耦**：相机位置/朝向不参与任何物理或传感器计算，转镜头不影响小车行为

---

## 五、装配流程（sim 侧声明装配）

```
sim 启动
 ├─ 1) 开「底盘连接」→ chassis_hello { peer_id, chassis, params, manifest }
 │      Godot：以 peer_id 为键创建 avatar（身体），这条连接此后承载运动指令
 ├─ 2) 开「传感器连接」× K → sensor_hello { peer_id, slot, sensor, mount, params }
 │      Godot：把传感器挂到该 peer_id 的 avatar 上
 ├─ 3) Godot 收齐 manifest 里所有零件 → 发 assembly_ready
 └─ 4) sim 收到 assembly_ready 后才启动决策链
```

**`manifest` 是关键**：世界据此知道"这辆车应该有几个零件、都连上了没"，才能判断组装完成。

### 5.1 Godot 侧 WebSocketManager

```
Godot 启动
 └─ WebSocketManager（单例 Node，一个管所有连接）
      └─ listen(port)
           └─ accept N 条连接 → 每条建一个连接对象
```

内部维护两张表：

- `peer_id → vehicle`
- `连接 → (peer_id, 组件角色)`

每条连接到达后：

```
收到第一条消息（声明）
 ├─ chassis_hello → 以 peer_id 为键建 vehicle，放进世界
 └─ sensor_hello  → 找到该 peer_id 的 vehicle，把传感器挂上去
```

- 底盘连接：决定"有一台车出现了"
- 传感器连接：给已有的车加零件
- `manifest` 全部到齐 → 给底盘连接回 `assembly_ready`

三个实现要点：

1. **manager 是单例**：一个 Node 管 N 条连接
2. **连接有状态**：记住"我是谁的、我是什么组件"；断开时按 `peer_id` 清理，最后一个零件断开即移除 vehicle
3. **传感器可能先于底盘到达**：需要暂存挂起的传感器连接，等底盘建立后再挂载

---

## 六、通道设计（WebSocket）

### 6.1 基本选择

- **Godot 当 server，sim 当 client**：世界是中心，N 个 sim 连进来，正好对应"启动很多 sim 进程"
- **一个组件一条 WS 连接**：WebSocket 没有多路复用（不像 HTTP/2、QUIC），一组件一连接是拿到"队头阻塞隔离"最自然的办法；且丢帧策略、故障隔离、重连都可按组件定制
- **同一端口可连多条**：WS 服务器监听一个端口，任意多个客户端都能连；靠 TCP 四元组区分，**不需要给每个 sim 分配端口**
- **Godot 侧**：`TCPServer` + `WebSocketPeer.accept_stream()`（或 `WebSocketMultiplayerPeer.create_server`）
- **Rust 侧**：`tokio-tungstenite`（项目已有 axum ws 先例：task_29 `API/webui.rs`）

### 6.2 消息 schema

**底盘连接（每 sim 一条）**

```jsonc
// sim → world（连上后第一条）
{
  "msg": "chassis_hello",
  "peer_id": "12D3Koo...",
  "chassis": "diff_drive_2d",
  "params": { "max_speed": 1.5, "max_turn_rate": 90 },
  "manifest": [                                  // 该 sim 会连入的全部零件
    { "slot": "lidar_front", "sensor": "lidar_2d" },
    { "slot": "imu",         "sensor": "imu" },
    { "slot": "gnss",        "sensor": "gnss_rtk" }
  ]
}

// world → sim
{ "msg": "chassis_ready", "peer_id": "...", "avatar_id": 7,
  "spawn": { "x": 64.0, "y": 64.0, "yaw": 0.0 }, "world_time": 12345.678 }

// sim → world（20~50Hz，持续）
{ "msg": "motion_cmd", "seq": 1234, "action": "forward", "speed": 50 }

// world → sim（收齐 manifest 后）
{ "msg": "assembly_ready", "peer_id": "...", "world_time": 12345.678 }
```

**传感器连接（每传感器一条）**

```jsonc
// sim → world
{ "msg": "sensor_hello", "peer_id": "12D3Koo...", "slot": "lidar_front",
  "sensor": "lidar_2d",
  "mount": { "x": 0.1, "y": 0.0, "z": 0.2, "yaw": 0.0 },   // 外参
  "params": { "range": 12.0, "rays": 360, "rate_hz": 10 } }

// world → sim
{ "msg": "sensor_ready", "slot": "lidar_front", "sensor_id": 3 }

// world → sim（持续；小数据 JSON text frame）
{ "msg": "imu",  "t": 12345.678, "gyro": [0.01, -0.02, 0.0], "accel": [0.0, 0.0, 9.81] }
{ "msg": "gnss", "t": 12345.678, "x": 64.02, "y": 63.98, "fix": "rtk_fixed" }

// world → sim（持续；大数据 binary frame）
// 头: [u8 type][u16 slot_id][u32 seq][f64 world_time][payload]
//   lidar  payload = f32 距离数组
//   camera payload = JPEG 字节
```

### 6.3 设计决定

1. **世界的底盘模型接受与真机相同的控制接口**：真车 STM32 收"运动状态 + 速度"（`FUNC_CAR_RUN`），速度环在 STM32 内部。世界底盘也应如此（自己跑速度环）——**这样 sim 的控制器组件与真机一模一样**，不需要为仿真写额外逻辑。
2. **声明/控制用 JSON text frame，大数据用 binary frame**：GDScript 解析 JSON 方便、调试直观；点云/图像走 binary，避免 base64 膨胀。
3. **每帧带 `world_time`**：多条连接异步到达，只有统一世界时间戳才能对齐（IMU 100Hz + 相机 30fps + 雷达 10Hz 混合时尤其重要）。
4. **就绪协调**：`manifest` + `assembly_ready`（见第五节）。
5. **心跳 / 断线重连**：WS ping/pong 或应用层心跳；sim 必须能自动重连并重新注册（世界重启时不必手动重启 N 个 sim）。
6. **启动顺序容错**：sim 很可能先于世界启动 → sim 侧连接必须是**带退避的重试循环**，而不是连一次失败就退出。

---

## 七、已定决策（2026-10-19）

| # | 决策 | 说明 |
|---|---|---|
| D1 | **3D**（不做 2.5D） | 需要模拟无人机，世界必须是 3D |
| D2 | **Godot** 做世界 | 省下几何 / 物理 / 渲染 / 射线 / 可视化 |
| D3 | **`CharacterBody3D`** | UGV 用 `MOTION_MODE_GROUNDED`，UAV 用 `MOTION_MODE_FLOATING`；运动模型自己写（= 速度环在世界侧） |
| D4 | **程序化生成**世界 | 不手工在编辑器摆 `GridMap` |
| D5 | **尺度与 Orion 现状一致** | 体素 0.5m、水平 128×128m、原点 (64,64,0)（见 §7.1） |
| D6 | **零件目录 v1** | 底盘 2 种 + 传感器 1 种（`lidar_2d`，见 §7.2） |
| D7 | **一个组件一条 WS 连接** | 见 §六 |
| D8 | **世界权威** | 位置更新在世界侧，sim 不做运动学积分 |
| D9 | **垂直范围 0 ~ 32 m** | 现有 2D 设定无高度维，本任务定为 32m |
| D10 | **世界组件** = `GridMap` 体素 + 一整块地面 + Godot 内置物理 | 见 §4.1 |
| D11 | **Vehicle 组件** = `CharacterBody3D`，按 WS 声明动态装配 | 见 §4.1 |

### 7.1 尺度（已从代码核实）

| 参数 | 值 | 出处 |
|---|---|---|
| 栅格分辨率 | **0.5 m** | `Robot/core/grid.rs:19 CELL_RESOLUTION` |
| 世界范围（水平） | **128 m × 128 m** | `pleiades-sim/src/main.rs:23 WORLD_MIN/MAX` |
| 默认原点 | **(64, 64, 0)** | `pleiades-sim/src/main.rs:26 DEFAULT_ORIGIN` |
| 障碍膨胀半径 | 0.2 m | `pleiades-ugv/src/config.rs:28` |
| 本车 footprint | 0.15 m（半径） | `Task/robot_review_problem.md` N13 |
| 决策 tick | 50 ms | `Robot/core/robot.rs:280 auto_tick_ms` |
| 位姿广播 | 100 ms | `Robot/core/robot.rs:213 state_notifier` |
| UAV 巡航高度 | 2 m（固定，由飞控管） | `Task/task_27_uav_pathfind.md` D3 |

→ 体素边长取 **0.5 m**（= `CELL_RESOLUTION`），水平 **256 × 256** 格，垂直 **0 ~ 32 m**（64 层）。

### 7.2 零件目录 v1

**底盘（运动器）—— 只做两种**

| 名字 | 说明 | 参数 |
|---|---|---|
| `diff_drive_ugv` | 差速车（`MOTION_MODE_GROUNDED`） | `max_speed`, `max_turn_rate`, `wheel_base`, `track_width` |
| `quadrotor_uav` | 四旋翼（`MOTION_MODE_FLOATING`） | `max_speed`, `max_climb_rate`, `max_yaw_rate` |

**传感器 —— v1 只做一个**

| 名字 | 说明 | 参数 |
|---|---|---|
| `lidar_2d` | 2D 单线激光雷达（真射线投射），**与当前 Orion 雷达规格一致**（见 §7.4） | `range`, `rays`, `rate_hz`, `mount` |

### 7.3 坐标转换（**必须一次性定死，只留一个入口**）

项目是 **Z-up**（`x`=东、`y`=南、`z`=高度，yaw 绕 z）；**Godot 是 Y-up**。所以：

```
Godot.position = Vector3(world.x, world.z, world.y)   // 世界 z(高度) → Godot Y
Godot.rotation.y = -world.yaw                          // 符号待实测确认
```

**只在世界侧写一个转换函数**，作为全局唯一入口——否则符号错误会散落到各处，极难排查。

### 7.4 `lidar_2d` 的模拟方式

**不复刻 Tmini 的协议 / 三角测距 / 噪声细节**——那是设备层的事，不影响 sim 的决策逻辑。只需要模拟"周围一圈距离"。

- **实现**：在传感器挂载点，用 Godot 的 `PhysicsDirectSpaceState3D.intersect_ray()` 在**水平面**循环投射射线（如 360 条 / 一圈）
- **性能**：360 条 × 10Hz = **3600 次/秒/车**；10 台车 = 3.6 万次/秒，Godot 射线查询是微秒级，完全够
- **输出格式对齐真机**：`LaserScan { points: Vec<LaserPoint { angle, range, intensity }>, stamp, scan_freq, scan_time }`，这样 sim 侧的读取 / 建图 / 避障代码**一行不用改**

真机规格（YDLIDAR Tmini，型号 140）仅供参考，不必逐项复刻：

| 参数 | 真机值 | 仿真是否复刻 |
|---|---|---|
| 采样率 | 4K（4000 点/秒） | **不必**，按"射线数 × 频率"决定 |
| 强度 | 8 bit（0-255） | 可给常数，或按 collider 类型映射 |
| 扫描频率 | 10Hz | 建议一致（影响 sim 的时序假设） |
| 协议 / 三角测距 / 噪声 | 真实设备行为 | **不复刻** |
| 量程 / 角度范围 | 设备配置 | 作为传感器参数 `range` / `rays` 传入 |

---

## 八、实现步骤

> 按 `.github/instructions.md` 的执行引擎规则，每个 step 串行执行（规划 → 实现 → 评审，最多 3 轮）；每个 step 结束后更新本 task 的 workbook。
>
> 注：本列表覆盖 **Godot 模拟世界**；sim 侧的 `WorldLink` 与传感器/控制器组件接入是另一条线。

### Step 1 — 整体规划（先行）

**产出**：`orion-simulation-world/COMPONENTS.md`

内容：
- 全场景节点树（`World.tscn` 为根，含各组件挂载点）
- **每个组件一个 `.tscn`** 的清单
- 每个组件的**节点树结构**
- 组件之间的挂载关系与数据流

**后续所有 step 都以这份文档为准实现。**

### Step 2 — 世界组件

- 产出：`World.tscn`（根场景）
- 内容：`GridMap` 体素（程序化生成）+ 一整块地面（`StaticBody3D`）+ 光照 / 环境
- 验收：场景可见 128×128×32m 的体素世界；物体被地面挡住，不会无限下坠

### Step 3 — 观察者相机

- 产出：`ObserverCamera.tscn`
- 内容：`Camera3D` + 自由飞行 / 环绕 / 跟随三种视角（§4.2）
- 验收：能在世界里自由移动视角；转镜头不影响任何物理

### Step 4 — Vehicle 组件

- 产出：`Vehicle.tscn`
- 内容：`CharacterBody3D` + `MeshInstance3D` + `CollisionShape3D` + 装配接口（挂底盘 / 挂传感器）
- 验收：能实例化一台"空车"；底盘与传感器可动态挂载

### Step 5 — 差速底盘 `diff_drive_ugv`

- 产出：`chassis/DiffDrive.tscn`
- 内容：`MOTION_MODE_GROUNDED` + 差速运动学 + 碰撞形状
- 验收：收到运动指令能前进 / 后退 / 转向；撞墙会停

### Step 6 — WebSocketManager

- 产出：`WebSocketManager.tscn`
- 内容：监听端口、accept 连接、`peer_id → vehicle` 与 `连接 → (peer_id, 角色)` 两张表、`chassis_hello` / `sensor_hello` 装配、`manifest` 就绪、`assembly_ready`、断开清理（§5.1）
- 验收：一个 sim 能连上并完成装配，收到 `assembly_ready`

### Step 7 — 四旋翼底盘 `quadrotor_uav`

- 产出：`chassis/Quadrotor.tscn`
- 内容：`MOTION_MODE_FLOATING` + 四旋翼运动学（平移 / 爬升 / 偏航）
- 验收：无重力悬停；能按指令升降与平移

### Step 8 — 传感器 `lidar_2d`

- 产出：`sensors/Lidar2D.tscn`
- 内容：从挂载点出发的水平面射线投射（360 条/圈），输出 `LaserScan` 格式（§7.4）
- 验收：一圈扫描能正确反映周围障碍的距离

---

## 九、记录：决策链驱动点

- base 目前只有 `DeviceHandler::on_tick` 一个驱动点（`Robot/core/robot.rs:77`），决策链由它驱动；sim 侧的 `WorldLink` 是独立 tokio task（不属于 §八 的 Godot 侧 step 列表）。
- **3D 世界 ≠ 3D 决策**：UAV 现在的决策链仍是 2D（固定高度 2m）。3D 世界给 UAV 一个真正的 3D 身体；三维寻路 / 高度控制 / 三维避障不在本任务内。
