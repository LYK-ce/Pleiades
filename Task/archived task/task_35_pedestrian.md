# task_35_pedestrian — 行人底盘：Python 经 WS 直接驱动

> Presented by KeJi
> Created Date ： 2026-10-25
> Modified Date ： 2026-10-25
> 状态：**方向已定，待人类审阅**（5 个 step，严格串行）
> 关联文档：`orion-simulation-world/COMPONENTS.md`、`Task/task_34_camera.md`、`Workbook/wb_34_camera.md`

---

## 一、目标

给仿真世界加一个**可被外部脚本直接驱动的行人**：世界侧把它做成一种特殊底盘（有碰撞、会走路、会转身），由 **Python 通过 WebSocket 直接控制**，**不经过 `pleiades-sim`、不经过 libp2p、不经过任何决策链**。

1. **世界侧**：`ChassisCatalog` 新增 `pedestrian`；新场景 `Pedestrian.tscn`（`CharacterBody3D` + 胶囊碰撞 + 走路 glb + `AnimationPlayer`）；一个 `ChassisModel` 子类把「速度 + 方向」翻译成位移/转向，并用 `speed / 1.26` 驱动步频（脚不滑）。
2. **协议侧**：`chassis_hello` 增加 `kind`（`robot` | `pedestrian`），pedestrian 允许可选 `spawn` 指定出生点；`motion_cmd` 增加**可选** `v`（m/s）与 `dir`（世界系朝向角，弧度），老的 `action/speed` 保持兼容。
3. **控制侧**：Python 客户端（放在 `orion-simulation-world/tool/pedestrian/`）连 `ws://127.0.0.1:9999` → 发 hello → 收 `chassis_ready` → **持续**（20~50 Hz）发 `motion_cmd{v, dir}` → 可选读 `pose`；超过 300 ms 没收到指令，世界侧自动停车。
4. **验证**：Python 能让它走、转弯、停下；车撞上去会被挡住；雷达能扫到（顺带现象，不作为建图验收）。

**非目标（硬边界）**：
- **不涉及 SLAM / 建图 / 地图一致性 / 动态障碍 / 避让策略**（人写进地图、离开后被后续观测抹掉，都是正常行为，不做任何掩蔽或衰减）；
- 不做 YOLO、不做 tensor/图传、不做多行人编队；
- 不改 `pleiades-base` / `pleiades-ugv` / `pleiades-uav` / `pleiades-terminal` / `pleiades-sim` 的任何代码；
- 不改 `rust_uav_camera_test`、不改既有 `programs/` 脚本；
- 不 git add / commit。

**现状起点（已核实）**：
- 世界已有装配协议：`chassis_hello{peer_id, chassis, params, manifest}` → `chassis_ready{spawn}`；`motion_cmd{seq, action, speed}` 只取最新一条；`pose` 10 Hz 只发底盘连接（`src/net/WebSocketManager.gd`、`src/net/WireProtocol.gd`）。
- `ChassisCatalog` 只有 `diff_drive_ugv` / `quadrotor_uav`；底盘基类 `ChassisModel.gd` 提供 `GetDefaultParams / Configure / SetCommand / Step`。
- 已有两套**纯视觉**人物：`World.gd` 的 `PERSONS`（静态、无碰撞）与 `WALKERS`（自动绕 44 m 矩形环路、无碰撞、无网络）。
- 走路资产已就绪：`assets/humans/rp_nathan_animated_003_walk/model.glb`（88 骨骼 / 69 帧 / 2.2667 s / 1.26 m/s / 人高 1.84 m）。
- 车辆 `collision_layer = 4`、`mask = 7`；雷达 `scan_mask = 1|2|4`；地面/空气墙在 layer 1。

**硬约束**：
- 行人纯属"世界里的一个可动物体"，**不是** Pleiades 节点：`peer_id` 由 Python 自定义（如 `human-01`），不参与任何 P2P 广播。
- 世界内部按 `kind` 区分车/人：凡"车"语义的地方（`Vehicles` 容器、`_PickSpawn` 的 avatar 排位、日志文案）不得把人算进去。
- 归属注释、命名规范（函数 Pascal_Snake、变量 snake_case、常量 UPPER_SNAKE）沿用 `COMPONENTS.md`。

---

## 二、实施步骤

### Step 1 — 世界侧行人底盘（`orion-simulation-world`）

**改动文件**

| 文件 | 动作 |
|---|---|
| `src/scenes/props/Pedestrian.tscn` | **新建**：`CharacterBody3D` + `CollisionShape3D`（`CapsuleShape3D`，r=0.25、h=1.75、中心 y=0.875）+ 走路 glb 实例 + `Body`/`Animation` 节点；`collision_layer = 2`、`collision_mask = 1 | 4`（地面 + 车辆） |
| `src/chassis/Pedestrian.gd` | **新建**：`extends ChassisModel`；`SetCommand(dict)` 解析 `v`/`dir`（缺省回退 `action/speed`）；`Step(delta)` 把 yaw 朝 `dir` 限速旋转（默认 180°/s）、沿当前 yaw 前进 `v`（clamp 0~1.26），返回控制量；`Play_Walk()` 找 `AnimationPlayer` 播第一条动画、`loop`、`speed_scale = v / 1.26` |
| `src/chassis/ChassisCatalog.gd` | `SCENES` 加 `"pedestrian"`；`DEFAULTS` 加 `{max_speed: 1.26, max_turn_rate: 180.0, turn_rate_deg: 180.0}` |
| `src/tests/check_pedestrian.gd` | **新建**：场景形态（节点/胶囊/层掩码）、目录装配、`SetCommand` 后 `Step` 产出正确速度与角速度、clamp 生效、`v=0` 时速度归零 |

**要点**
- 行人**不用** `Vehicle.tscn`（那是车：有 Chassis/Sensors 挂载点、轮式外观）；但运动仍由 `move_and_slide()` 单一推进，沿用"代码只给控制量、不写 position"的既有约定。
- 动画找不到时只 `push_warning`，不得 `push_error`（否则 `--quit` 的 0 ERROR 验收假红）。
- 停住（`v=0`）时 `speed_scale=0`，人物定格，不闪不跳。

**验收**
- `timeout 200 godot --headless --path orion-simulation-world --quit` → EXIT=0、`grep -E "SCRIPT ERROR|Parse Error|ERROR"` 为空；
- `check_pedestrian` → `SELF-CHECK OK`；
- 既有 14 个自检 total 与基线逐字一致（`check_world 26` / `check_vehicle 40` / … / `check_person_walk 20`）。

### Step 2 — 协议扩展：`kind` + `v/dir`（`orion-simulation-world`）

**改动文件**

| 文件 | 动作 |
|---|---|
| `src/net/WireProtocol.gd` | `chassis_hello` 文档与常量补 `kind`、可选 `spawn`；`BuildMotionCommand` 支持 `v`/`dir`（保留 `action`/`speed`） |
| `src/net/WebSocketManager.gd` | `_RegisterChassis` 读 `kind`（缺省 `robot`）、pedestrian 允许 `spawn`（校验：在 `[0,128)` 内、不与已占 avatar 重叠、不在静态障碍内，非法则回退默认点并 warn）；`_ApplyMotionCommand` 对 pedestrian 只认 `v/dir`（也接受 `action/speed` 折算）；`pose` 照常 10 Hz 下发 |
| `src/tests/check_ws_manager.gd` | 追加：`kind="pedestrian"` 能装配、`motion_cmd{v,dir}` 生效、超时 300 ms 自动停、`kind` 缺省仍按 robot 处理 |
| `COMPONENTS.md` | §4.5 / §8.1（`chassis_hello.kind`、`motion_cmd.v/dir`、pedestrian 的 spawn 规则）/ §10 / §11 |

**要点**
- `v/dir` 为**可选**字段：老的 `action/speed` 行为完全不变（`check_ws_manager` 的 59 项断言不得回归）。
- 世界内部记录 `kind`；`Vehicles` 容器与 `_PickSpawn` 的 avatar 排位只对 `robot` 生效，pedestrian 单独进 `Pedestrians` 容器。
- 超时停车：每个 pedestrian 记录 `last_motion_at`，`_process` 里超过 300 ms 未见新指令 → `v=0`（只停，不断连，不刷日志）。

**验收**
- `check_ws_manager` 在 Xvfb / headless 两档都 `SELF-CHECK OK`（新增断言 + 既有 59 项不变）；
- `--quit` EXIT=0、0 ERROR。

### Step 3 — 世界布局：用外部驱动的行人替换自动绕圈的 `WALKERS`

**改动文件**

| 文件 | 动作 |
|---|---|
| `src/world/World.gd` | `WALKERS`（自动路径）改为 `PEDESTRIANS`：只定义出生点（世界坐标 + 初始朝向）与数量，**不再内置巡逻逻辑**；`BuildWalkers()` → `BuildPedestrians()`，实例化 `Pedestrian.tscn`，等待 WS 驱动；`PERSONS`（静态两人）保持不动；幂等 |
| `src/tests/check_person_walk.gd` | 保留资产/动画断言，去掉"自动走动"相关断言（改由 Step 1 的 `check_pedestrian` 与 Step 2 的 WS 测试覆盖） |
| `COMPONENTS.md` | §2 节点树 / §4.x / §10 / §11 同步 |

**要点**
- 世界不再"自己让人走"：没有 Python 连接时，人就是站着的（这是预期，不是故障）。
- 出生点必须避开 `OBSTACLES` / 空气墙 / 车辆 spawn 净空；在代码注释里写明选点依据。
- 既有自检不得因"人不走了"而变红（`check_person_walk` 的自动路径断言必须同步调整）。

**验收**
- `--quit` EXIT=0、0 ERROR；14 个既有自检 + 两个 person 自检全绿；
- 无 Python 时世界日志出现 pedestrian 装配提示、但**不出现 ERROR**。

### Step 4 — Python 客户端（`orion-simulation-world/tool/pedestrian/`）

**改动文件**

| 文件 | 动作 |
|---|---|
| `orion-simulation-world/tool/pedestrian/pedestrian_client.py` | **新建**：最小 WS 客户端（优先 `websockets`，缺失则用标准库按 RFC6455 手写 JSON 帧）；`hello(peer_id, spawn=None)` → 等 `chassis_ready`；`drive(v, dir)` 以 20~50 Hz 持续发送；`pose()` 读世界回传的位姿；退出时发 `v=0` |
| `orion-simulation-world/tool/pedestrian/patrol.py` | **新建**：示例行为——沿矩形/折线巡逻（路点 + 方位角计算 + `v` 恒定），支持 `--speed` / `--loop` / `--duration` |
| `orion-simulation-world/tool/pedestrian/README.md` | **新建**：用法、协议字段（`v`/`dir`/超时规则）、依赖说明、排障（端口被占 / 未 assembled / 人不动） |
| `orion-simulation-world/tool/.gdignore` | **新建**：让 Godot 完全跳过 `tool/` 目录（否则编辑器会扫描 `.py`/日志等非资源文件，`.godot` 缓存里出现噪音） |

**要点**
- Python 只发控制、不读地图、不连 P2P；`dir` 用世界系弧度（0 = +X 东）。
- 所有失败可读：连不上、handshake 超时、`chassis_ready` 未到、`spawn` 被拒，都要打印原因并退出非零。
- 脚本自带"停止"保证：异常/`Ctrl-C` 时发一次 `v=0`。

**验收**
- 世界（headless 即可，不需要渲染）起来后，`python3 orion-simulation-world/tool/pedestrian/patrol.py --duration 10` 能让 `pose` 的 `x/y` 按 `v` 变化、`yaw` 收敛到 `dir`；
- 断开后 300 ms 内世界日志/位姿显示 `v→0`（人停住）；
- 无 Python 依赖时给出明确安装提示，不静默失败。

### Step 5 — 端到端验证（世界 + Python + 车撞人）

**验证项**

| # | 场景 | 判据 |
|---|---|---|
| 1 | Python 驱动直行 | 10 s 内位移 ≈ `v × t`（±10%），`yaw` 与 `dir` 误差 < 2° |
| 2 | Python 驱动转弯 | 给定 `dir` 阶跃，`yaw` 以 ≤180°/s 收敛，轨迹为弧线（不是原地瞬转） |
| 3 | 断连/超时 | 停止发送 300 ms 后 `v→0`，位置不再变化 |
| 4 | 车撞人（headless 物理） | 车朝人开过去，最终车心到人胶囊表面距离 ≥0（不穿透），车停住 |
| 5 | 雷达可见（顺带） | 人挡在车前方时，sim 日志出现该距离的扫描命中（不做建图断言） |
| 6 | 真渲染出图 | Xvfb 下拍 3 帧（走 / 转 / 停），人物姿态与位置变化可见 |

**产物**：`tmp/task35/`（脚本日志、`pose` 采样 CSV、三帧 JPEG、验收记录）；`Workbook/wb_35_pedestrian.md`（每 step 一节）。

**不可用环境下的降级**：没有 X 显示时，第 6 项判"未验证"并上报；其余 5 项在 headless 下必须真跑。

---

## 三、记录

- 2026-10-25：方向确认——人**不进** `pleiades-sim`，由 Python 直接经 WS 驱动；粒度 = 持续输出「速度 + 方向」；碰撞要做；**不涉及 SLAM/建图**。
- 2026-10-25：Python 客户端位置定案——放 `orion-simulation-world/tool/pedestrian/`（Godot 工程内、`.gdignore` 排除），不放仓库根 `Tool/`。
