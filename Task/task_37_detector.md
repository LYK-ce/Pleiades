# task_37_detector — 相机帧槽位 + detector 设备

> Presented by KeJi
> Created Date ： 2026-10-27
> Modified Date ： 2026-10-27
> 状态：**方案已定，待 task 36 完成后开工**
> 前置：Task 36（任务层改造）收尾后再动，避免与设备端改动交叉
> 关联：`Task/task_34_camera.md`（相机设备）、`Architecture/robot_arch.md`

---

## 一、总体目标

把 detector 做成**设备**（与 `stm32` / `lidar` / `mavlink` 同构：自己的目录、线程、状态、配置段，随 `start()` / `shutdown()` 走）：

1. `pleiades-uav` 里一份；
2. `pleiades-sim` 里一份；
3. 在 `orion-simulation-world` 中，让 **UAV 能识别到 human**。

本任务做到“识别到人 → 算出世界位置 → sim 里跟着走”：
检测框（Step 2）→ 世界坐标 `(x, y)` 与误差（Step 3）→ sim 无人机跟随（Step 4）。
**跟踪器（ID / 卡尔曼）、真机跟随、行为树泛化、协议上报都不在本任务。**

**顺序**：Step 1 相机统一 → Step 2 detector → Step 3 位置估计（uav + sim）→ Step 4 sim 跟随。

---

## 二、Step 1 — 相机统一：常驻抓取 + 共享帧槽位

### 1 目标

三处相机（`camera_pc` / `camera_uav` / `camera_sim`）统一为同一形态：

```text
camera_uav / camera_pc：worker 线程按 stream_fps 抓硬件 ──写──┐
camera_sim：             world_link recv task 推帧 ────────写──┼──► FrameSlot ──► latest_frame() 同步读
未来 ugv：               网络收帧 task ──────────────────写──┘
```

统一的是**槽位结构与读接口**，不是“谁来写”。

### 2 已定决策

| # | 决策 | 说明 |
|---|---|---|
| 1 | **槽位** | `FrameSlot`：最新一帧 + 元数据，**覆盖式**（只留最新，读者可能漏帧） |
| 2 | **时间戳** | 统一为**本机单调毫秒**（进程启动为 0）。sim 的世界时间只作附加字段，转换在 sim 适配层做 |
| 3 | **帧率** | 新增 `stream_fps`；**不复用**现有 `fps`（后者保持“格式协商请求帧率”原义） |
| 4 | **空闲开关** | 新增开关（如 `stream = true/false`）：关闭时不抓帧、不占 CPU |
| 5 | **读接口** | `latest_frame() -> Option<Frame>`：**同步、非阻塞** |
| 6 | **兼容** | `capture()` 保留：读槽位，槽位空则按需抓一次；`info()` 四字段不变；Lua 脚本不改 |

槽位草案：

```rust
struct Frame {
    jpeg: Arc<[u8]>,     // Arc 避免每次读都拷几百 KB
    width: u32,
    height: u32,
    seq: u64,
    captured_at_ms: u64, // 本机单调毫秒
}
```

### 3 不做

- 不改检测、不改 `on_tick`、不改协议、不动 ugv；
- 不做录像/全帧队列（只留最新）；
- 不改 `capture()` / `info()` 的对外语义（Lua 契约不变）。

### 4 完成判据

1. 三处相机都能同步 `latest_frame()` 拿到最新帧，且带 `captured_at_ms`；
2. `camera_photo.lua` 与 `info()` 契约不回归；
3. `stream = false` 时相机不抓帧、空闲无 CPU 占用；
4. 单测覆盖：槽位覆盖、时间戳单调、无帧返回 `None`、开关关闭不启动抓取。

---

## 三、Step 2 — detector 设备（uav / sim）

### 1 目标

- `pleiades-uav/src/device/detector/` 新增设备（`mod.rs`）；
- `pleiades-sim` 新增同构设备，接 `camera_sim`；
- 仿真世界里 UAV 相机对着行人时，能输出 human 检测。

### 2 形态

```rust
DetectorDevice::spawn(cfg: DetectorConfig, source: Arc<dyn FrameSource>, rs: Arc<RwLock<RobotState>>)
  -> Result<Arc<Self>, String>

DetectorDevice::latest_dets() -> Vec<Detection>   // 只读快照
DetectorDevice::stats() -> DetectorStats
DetectorDevice::shutdown()
```

- 内部跑成 tokio task：读 `latest_frame()` → `spawn_blocking` 跑推理 → 覆盖结果；
- 结果带 `frame_ts`，供后续与位姿对齐（本步只打印/可视化，不消费）；
- 相机没开 / 无帧 → 只 `warn`，不空转崩溃。

### 3 接入方式（uav）

```text
uav_bootstrap：保留一份 Arc<Uav_Camera>，push 进 device_caps 之前 clone 给 handler
UavDeviceHandler::new(..., camera: Option<Arc<Uav_Camera>>)
start()：DetectorDevice::spawn(cfg.detector, camera, robot_state)
UavInner：detector: Option<Arc<DetectorDevice>>
shutdown()：detector.shutdown()
```

### 4 配置

```toml
[detector]
enabled = false
model = "yolov8n.safetensors"
device = "cpu"
conf_thres = 0.35
nms_thres = 0.45
```

### 5 模型与测试环境

- 权重放 **`Pleiades_Workspace/`**，`[detector].model` 只写文件名；
- 现有 `Yolo_Detector::new` 的解析规则：`<workspace_dir>/<model_file>`，文件名必须是 `yolov8<size>.safetensors`（型号从文件名推断，如 `yolov8n.safetensors`），**加载一次、常驻**；
- person 过滤（COCO `class_index == 0`）**Step 2 不必做**——日志里能认出 `person` 即可；过滤放后续（配置 `classes = [0]`）；
- **测试环境**：把权重拷到 `RobotSim_TestEnv/<实例>/Pleiades_Workspace/`；同一实例的 `.config/config.toml` 里加 `[detector]` 与 `[[sim_world.sensor]] sensor="camera"`；`programs` 软链沿用 task_34 的说明；
- ⚠️ `Yolo_Detector::new` 内部调用 `Ensure_Config()` 取 workspace 路径——算法若以后搬出 base，这层耦合要改成“由调用方传 workspace 路径”；若 Step 2 的算法归属选独立 crate，必须一起处理。

### 6 完成判据

**本步只验证“能拿到框”，按两条依次判：**

1. **帧到**：detector 打出 `frame_ts`（相机帧槽位有帧，时间戳单调）；
2. **框出**：同一帧结果里出现 `class_name == "person"`，带 `conf` 与像素 `bbox`（形如 `person conf=0.87 bbox=(412,208)-(455,390)`）。

配套（非功能判据）：

3. uav / sim 各一份 detector 设备，`enabled=false` 时零线程；
4. 模型缺失、相机无帧只 `warn`，不拖垮进程；
5. 不改动现有 `on_tick` 决策链与执行器。

**明确不判**：持续帧率/稳定性、检测精度、person 过滤、定位/跟踪/`TargetTable`/树、`stats()`、webui。

---

## 四、Step 3 — 人在世界中的位置（**不含跟随**）

### 1 目标

把检测框换算成世界坐标 `(x, y)`，并证明它算得对。
**只做“标记位置”**：不跟走、不下发速度、不做跟踪器。
本步在 **uav 与 sim 两侧**都做（uav 有相机）；跟随行为只在 Step 4 的 sim 里出现。

### 2 方法：地面平面投影（IPM / 单应）

```text
框底边中点 (u,v)（≈脚）
  → 去畸变（Step 3 只信画面中部，可先略）
  → 针孔反投影：相机系射线
  → R_mount（yaw/pitch）→ 机体系射线
  → R_yaw（本机朝向）→ 世界系射线
  → 与 z = ground_z 求交 → 世界 (x, y)
```

关键简化：相机相对机体固定 ⇒ **机体系单应是常量**，启动时由参数解析生成，**不手工标表**。
无 roll 时可退化成「行号 → 前方距离」一维表 + `(u - cx)` 横向偏移；表由 `h / tan(θ(v))` 生成。

### 3 输入（三条，缺一不可）

| 输入 | 来源 | 备注 |
|---|---|---|
| 内参 | `[detector.camera].fov_deg` + `fov_axis` + 帧自带 `width/height` | **分辨率不单独配**，用帧的，避免与相机配置不一致 |
| 外参 | `[detector.camera].mount_*` | 单位写进键名（`_deg` / 米），别像 sim 那样 yaw 弧度、pitch 度混用 |
| 位姿 | **帧时刻**的 `pose(x,y,z,yaw)` | 高度用 `pose.z`，**不用常量**（真机悬停有起伏） |

### 4 配置

```toml
[detector.camera]
fov_deg = 70.0
fov_axis = "vertical"

mount_x = 0.20
mount_y = 0.0
mount_z = 0.045
mount_yaw_deg = 90.0      # 光轴 = 局部 -Z，前视要 +90°
mount_pitch_deg = -15.0   # 俯角；pitch=0 时 2.5 m 高最近只能看到 ~3.6 m
ground_z = 0.0
max_range_m = 12.0
```

### 5 保护（不满足就**拒答**，不输出坏点）

- 框底边不在画面下半部 → 拒答；
- 射线接近地平线（`|ray_z|` 过小）、`t < 0`、距离 > `max_range_m` → 拒答；
- 只有地面上的点成立：脚被挡住、台阶、蹲下 → 可能算错；
- Step 3 不补偿 roll、不校畸变（只信画面中部），sim 无风，够用。

### 6 完成判据

1. 同一时刻输出「估计 `(x,y)`」与「真值 `(x,y)`」+ 误差（sim 行人真值）；
2. 3~6 m 内误差 **< 1 m**；6~8 m 放宽（不出现明显离谱点即可）；
3. 出画 / 被挡 / 贴边时**拒答**，不输出坏点；
4. 投影是**纯函数**，可单测：给定像素 + 位姿 + 参数 → 确定结果；改 `pitch` 或高度，结果随之变。

### 7 不做

跟随/控制、跟踪器、动态障碍注入、`TargetTable`、行为树、协议。

---

## 五、Step 4 — sim 无人机跟随（真机不跟）

### 1 目标

sim 里 UAV 跟着一个行人走：保持距离、人走它走、人停它停、丢了就停。
**只在 sim（`uav_sim`）实现**；真机 `pleiades-uav` / `pleiades-ugv` 收到 Follow 只做“平凡终值”，不进跟随逻辑。

> 依赖：Task 36 的 sim 双栈（`uav_sim/` + `mission/`）先落地，本步才接得进去。

### 2 协议：`Mission::Follow`

- 新增 `Mission::Follow`；`TASK_SET` 布局不变：`type=2 | x(f32) | y(f32)`，`x/y` 预留（现在填 0）；
- **type 用 2，不用 1**（`1` 是旧 `Circle`，会被误认成 Follow）；
- 取消仍走 `mission_count == 0`，不新增入口；
- 真机收到 Follow：建**平凡树**（`Action(Success)`），一拍终值 → 队列进下一条；
  **建议返回 `Failure(Unsupported)` 而非 `Success`**——`main_loop` 对 `Terminated(_)` 都会 `pop_next()`，队列照样前进，但不把“跳过”伪装成“成功”；
- ⚠️ **不能用 `build_task` 返回 `Err`**：Task 36 的约定是“建树失败 = 清队列 + 停车”，会打断后续任务。

### 3 数据流

```text
detector：最新 person 框 → Step 3 投影 → latest_target() { x, y, ts, age }
                                                    │ 同步读，不 await
FollowLeaf：dir  = normalize(P - self)
            goal = P - dir * follow_dist
            目标移动 > 0.5 m（或距上次 set_goal 超过 N 拍）→ nav.set_goal(goal)   // 节流
            next_cell = nav.step()
            Frame.out.next_cell = next_cell
                                                    │
on_tick ④：decide(next_cell) → ExecuteState → apply_action   （原样不动）
```

人的坐标**不写** `RobotState` / `ExecuteState`，只有“下一格”进控制状态。

### 4 树形

```text
Fallback
├─ ReactiveSequence
│   ├─ Condition  有目标（age < stale）
│   ├─ Condition  未太近（d > d_min）
│   └─ Action     FollowLeaf
└─ Action        Hold（next_cell = None，原地停）
```

- 用 `ReactiveSequence`（条件每拍重判），不用 `Sequence`；
- 丢失目标 → `Hold`，**不追最后一个点**；
- 人同时作为动态障碍注入 D*（复用 `cluster_to_obstacle_cells` 的膨胀），防撞人。

### 5 参数（初值，sim 可调）

| 参数 | 初值 | 说明 |
|---|---|---|
| `follow_dist` | 2.5 m | 停在人身前多远 |
| `d_min` / `d_max` | 1.5 / 3.0 m | 距离带 + 迟滞，太近停、太远追 |
| 目标更新阈值 | 0.5 m | 避免每拍重建 D* |
| `stale` | 0.8 s | 超过即视为丢失 |
| 相机 `pitch` | −15° | 保证 2~3 m 时脚在画面内（pitch=0 时最近约 3.6 m） |

### 6 完成判据

1. 行人直线走 10 s：UAV 保持 1.5~3 m 跟随，不撞人、不穿人；
2. 人停 → UAV 停在距离带内，不来回抖；
3. 人走出视野 / 丢失 > `stale` → UAV 停住，不乱飞；
4. 新 `TASK_SET` 或 `mission_count=0` → 一个 tick 内停下；
5. 真机（uav / ugv）收到 Follow：任务立即终值、队列进下一条，日志有记录。

### 7 不做

多目标 / ID 跟踪、真机跟随、ugv 跟随、把目标写进静态栅格、跟随结果的协议上报。

---

## 六、范围之外

- 多目标跟踪（ID / 卡尔曼）、`TargetTable` 的长期维护；
- 真机（uav / ugv）跟随实现——收到 Follow 只做平凡终值；
- 地面站上报（新 `msgid`）、多车融合；
- ugv 跟随；
- 模型训练/微调。

---

## 七、开工前待确认

1. **算法归属**（Step 2）：detector 的模型加载 + candle 推理，是放到独立 crate，还是 Step 2 先直接调用 base 现有 `ML_Engine/Yolo`（后续再搬）。
2. **观察方式**（已定）：Step 2 先用日志（`frame_ts` + `class/conf/bbox`）；落盘带框图、推 `webui` 留到后续。
3. **sim 场景**（Step 2）：默认测试环境只声明了 `lidar_2d`，需加 `[[sim_world.sensor]] sensor="camera"`；行人位置是否要特意摆在 UAV 视野内。
4. **Follow 的终值语义**（Step 4）：真机收到 Follow 用 `Success`（静默跳过）还是 `Failure(Unsupported)`（日志可辨、队列同样前进）；建议后者。

---

## 八、记录

- 2026-10-27：目标确定——detector 设备化，uav 一份、sim 一份，仿真世界 UAV 能识别 human。
- 2026-10-27：拆步——Step 1 先统一相机（常驻抓取 + 共享帧槽位），Step 2 再做 detector。
- 2026-10-27：Step 1 决策——`stream_fps` 新增不复用 `fps`；时间戳统一本机单调毫秒；空闲抓取给开关；`capture()` 兼容不变。
- 2026-10-27：本任务等 Task 36 收尾后开工，避免与设备端改动交叉。
- 2026-10-27：Step 2 验收收窄为“帧到 + person 框出”，观察方式先用日志。
- 2026-10-27：Step 3 定稿——只做地面平面投影（IPM）求世界 `(x,y)` 与误差，不做跟随；参数放 `[detector.camera]`；分辨率用帧自带、外参键名带单位、高度取 `pose.z`。
- 2026-10-27：Step 4 定稿——`Mission::Follow`（type=2，避开旧 `Circle=1`）；sim 用 `ReactiveSequence` 跟人，真机建平凡树立即终值；目标位置读 detector、不进 `ExecuteState`；初值 `follow_dist=2.5`、距离带 1.5~3.0、`stale=0.8 s`、相机 `pitch≈-15°`。
