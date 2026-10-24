# task_34_camera — UAV 相机集成 + 仿真相机对齐

> Presented by KeJi
> Created Date ： 2026-10-24
> Modified Date ： 2026-10-24
> 状态：**方向已定，待执行**（5 个 step，严格串行）
> 关联文档：`rust_uav_camera_test/README.md`、`orion-simulation-world/COMPONENTS.md`、`Task/task_31_sim_world_link.md`、`Task/task_33_camera_simulation.md`

---

## 一、目标

把**相机**接进两条链路，并让两边对 Lua 暴露同一套方法；**本任务不涉及 YOLO / 检测 / 图传**。

1. **真机 UAV —— 两个彼此独立的相机 sensor**（不是"一个设备两种模式"，也不是同一个目录下的两个文件）：
   - **两个独立子目录**：`pleiades-uav/src/device/camera_pc/` 与 `pleiades-uav/src/device/camera_uav/`，互不引用、互不共享抽象；
   - `[camera_pc]`：**开发机（电脑）摄像头**，沿用现有 NV12 实现，**默认 `false`**；
   - `[camera_uav]`：**UAV 机载相机**（SY011HD / UVC），照搬 `rust_uav_camera_test` 已验证的采集路径（显式 `open_stream()`、MJPG 直通、格式协商、超时、不 panic），**默认 `true`**；
   - 两者各有自己的配置段、各自的设备实现、各自的 Lua 表（`camera_pc` / `camera_uav`），**可以同时启用**（不同 `/dev/videoN`；同一设备被开后开者 `warn` 失败，不影响前者）。
2. **仿真**：世界侧的 `CameraSensor` 从"组件"变成**可装配的 `camera` 零件**（进 `SensorCatalog`、按配置频率下发 `type=2` 二进制帧）；sim 侧新增 `camera` 传感器（只保留最新帧），以 **`camera_sim`** 这个名字注入 Lua；最终在 `orion-simulation-world` + `pleiades-sim` 真跑一张图出来。
3. **统一方法契约**：三个 sensor 都是 `capture() → JPEG 字节` + `info() → {width, height, format, age_ms}`，脚本写一次、三处可跑。
4. **验证方式**：用最简单的新增拍照脚本（`programs/user/camera_photo.lua`，**新建文件，不改任何现有脚本**）出图落盘；**真机与仿真的验证都由人类自己在 TUI 里输入命令完成**，agent 只交付可验证的代码/配置/说明；**不跑 YOLO、不做吞吐、不做图传**。

**非目标（硬边界）**：**不改 `Orchestrator/Core` / `TUI` / `CLI` / `pleiades-sim` 控制口 / `CoreBootstrap` 的既有字段与命令通路，不新增任何命令注入接口（控制口命令、CLI 参数、自动脚本都不加）**；YOLO/检测器接入、tensor stream 图传、视频推流/浏览器预览、统一 `camera` 别名（避免"哪个生效"的隐式歧义）、两个相机目录共享的基类/公共模块（重复代码可以接受）、IMU/GNSS、决策链与建图、`pleiades-ugv`/`pleiades-terminal` 任何改动；**不改变现有 PC 相机实现的抓帧逻辑**（迁移时仅允许补 `open_stream()` 这一处必要修正）。

**已核实的现状（本任务的起点）**：
- 真机：现有实现（PC 摄像头）硬请求 `NV12`（`pleiades-uav/src/device/camera/mod.rs:91-98`）→ 对只出 MJPG/YUYV 的 SY011HD 不成立；且没有 `open_stream()`（`:182` 直接 `frame()`）。打开失败在 `bootstrap.rs:44-53` 只 `warn`，所以"程序能启动"不代表相机可用。`rust_uav_camera_test` 已在树莓派上实测通过：14 种格式（9 MJPG + 5 YUYV，无 NV12）、选 `1920x1080@30 MJPG`、单帧 4,147,200 B、稳态抓帧 47 ms。
- 仿真：世界 `CameraSensor` 只被 `check_camera_shots.gd` 用过，**不进 `SensorCatalog`、不挂 `World.tscn`、不推流**；`FRAME_TYPE_CAMERA=2` 只有常量；sim 的 `SensorSet::Build` 与 `world_link/catalog.rs` 只认 `lidar_2d`；`sim_bootstrap` 没有 `capabilities` 参数（`pleiades-sim/src/main.rs:88-94`），所以 Lua 里没有任何相机表。

---

## 二、实施步骤

### Step 1 — 两个独立相机 sensor（`pleiades-uav`）

**依据**：`rust_uav_camera_test/src/camera.rs`（树莓派已验证）。现有实现是开发机摄像头，保留。

**改动文件**

| 文件 | 动作 |
|---|---|
| `src/device/mod.rs` | 改为 `pub mod camera_pc; pub mod camera_uav;`（删除旧 `pub mod camera;`） |
| `src/device/camera_pc/mod.rs` | **新建（迁移）**：把现有 `src/device/camera/mod.rs` 的 NV12 实现整体搬入（`Pc_Camera`，worker 线程 + oneshot），**逻辑不改**；仅实测确有必要时补 `open_stream()`；`DeviceCapability` 注册表名 `camera_pc` |
| `src/device/camera_uav/mod.rs` | **新建**：`Uav_Camera`，移植 `rust_uav_camera_test` 的枚举/选格式/`open_stream()`/MJPG 直通/超时/关闭；注册表名 `camera_uav` |
| `src/device/camera_uav/format.rs` | **新建**：UAV 用的纯函数 `Choose_Format` / `Best_At` / `Best_Raw_At` / `Closest_Score` / `Format_Rank` + 单测 |
| `src/device/camera_uav/frame.rs` | **新建**：`Frame { format, width, height, jpeg, rgb }` + MJPG 直通 / 原始格式解码→JPEG 重编码 |
| `src/device/camera/` | **删除**（内容全部迁入 `camera_pc/`，不留兼容层） |
| `src/config.rs` | 拆成 `CameraPcConfig` / `CameraUavConfig` 两段；`fill_camera_pc` / `fill_camera_uav` 缺才补、幂等；旧 `[camera]` 段保留解析但**不读取**，启动时提示迁移 |
| `src/bootstrap.rs` | 分别尝试打开：`camera_pc`（默认关）/ `camera_uav`（默认开）；各自独立 `warn`；日志标明 sensor 名、选定格式、分辨率/帧率、耗时 |

**配置形态（两段独立，互不继承）**

```toml
[camera_pc]
enabled = false            # 默认 false
path = "0"                 # "0"/"1" 或 "/dev/video0"
width = 1280
height = 720
timeout_ms = 3000

[camera_uav]
enabled = true             # 默认 true
path = "0"
prefer = "mjpg"            # mjpg | raw
resolutions = [[640,480],[1280,720]]   # 默认 640×480，取不到才退 720p
fps = 0                    # 0 = 取该组合枚举到的最高帧率
open_timeout_ms = 8000
capture_timeout_ms = 5000
```

**硬要点**

1. 两个 sensor **完全独立**：独立目录、独立 struct、独立打开流程、独立 `DeviceCapability`、独立 Lua 表名；两个目录之间**不得 `use` 对方**，允许少量重复（帧类型、线程骨架），不做共享抽象；一个失败不影响另一个。
2. 允许同时 enable；若指向同一个 `/dev/videoN`，后打开者必须给出可读的 `Device or resource busy` 并只 `warn`。
3. `path` 映射：纯数字 → `CameraIndex::Index(n)`；`/dev/videoN` → 取尾部数字再走 `Index`（nokhwa 0.10 的 V4L2 后端**不接受字符串路径**，传 `"/dev/video0"` 会在内部 `parse::<u32>()` 失败）。
4. `camera_uav` 实现：抓帧超时后**不 `join`** worker；重开前必须 `drop` 旧设备（nokhwa 全局设备表对同 index 并存会 `assert!`）；MJPG 绝不 `decode_image`；失败一律 `Err(String)` + 排查提示。
5. `camera_pc` 迁移保持行为等价（同样的 NV12→RGB→JPEG、同样的线程模型、同样的 `timeout_ms` 语义）；**除 `open_stream()` 外不得改语义**。

**验收**
- `cargo check -p pleiades-uav` 绿；`cargo test -p pleiades-uav` 通过（新增格式选择单测；pc 既有测试不回归）。
- 开发机：`[camera_pc] enabled=true` 能打开并抓帧，与迁移前一致；`[camera_uav] enabled=true`（默认）因无 MJPG 设备而 `warn` 跳过，**进程照常启动**。
- 树莓派：`[camera_uav] enabled=true` 日志出现"选定格式 …（MJPG 直通优先…）"，不再出现 `Stream Not Started`。
- 两段都关 → 两个 sensor 都不注册，进程正常。

### Step 2 — Lua 拍照契约（`camera_pc` / `camera_uav`）

**改动文件**

| 文件 | 动作 |
|---|---|
| `src/device/camera_pc/mod.rs` | `register_lua_caps` 生成 `camera_pc.capture()` / `camera_pc.info()`；闭包只做薄胶水 |
| `src/device/camera_uav/mod.rs` | `register_lua_caps` 生成 `camera_uav.capture()` / `camera_uav.info()`；写法与 `camera_pc` 平行、互不调用 |
| `programs/user/camera_photo.lua` | **新建文件**（`test_camera.lua` 等既有脚本一律不动）：依次探测 `camera_uav` → `camera_sim` → `camera_pc`，打印用的是哪个 sensor + `info()` + 字节数，再写 workspace；三个都没有时返回明确错误 |

**硬要点**
- `capture()` 语义 = "取当前流里最新的一帧"（`open_stream()` 后 `frame()` 即最新帧），不额外起采集 task、不加缓冲队列。
- `info()` 返回 Lua table：`{width, height, format, age_ms}`，字段名与 Step 4 的仿真侧逐字一致。
- **不提供统一 `camera` 别名**：脚本自己按名字挑，避免"两个都开时哪个生效"的隐式歧义。

**验收（人类在 TUI 里执行，agent 不代跑）**
- 开发机（只开 `camera_pc`）：`exec camera_photo` → 日志显示 `sensor=camera_pc`，`Pleiades_Workspace/camera_photo.jpg` 存在、`file` 识别为 JPEG、尺寸与配置一致。
- 树莓派（只开 `camera_uav`）：同上，日志显示 `sensor=camera_uav`，尺寸/格式符合 `resolutions` 首选组合。
- 两段都开（不同设备）：两个表都在，`camera_photo.lua` 优先用 `camera_uav`；手动单独调用 `camera_pc.capture()` 也能出图。
- 连拍两次字节数或内容不同（证明不是同一帧缓存）。
- 不修改、不运行任何 `yolo_*` 脚本。

### Step 3 — 世界侧：`camera` 成为可装配零件（`orion-simulation-world`）

**改动文件**

| 文件 | 动作 |
|---|---|
| `src/sensors/SensorCatalog.gd` | `SCENES` 加 `"camera": "res://src/scenes/sensors/CameraSensor.tscn"`；`DEFAULTS` 加 `{width:640, height:480, fov_deg:70, quality:0.8, rate_hz:1.0}` |
| `src/sensors/CameraSensor.gd` | 新增 `SetStream(link, sensor_id)`（与 `Lidar2D` 同时序：`sensor_ready` 之后再绑流）+ 按 `rate_hz` 的出帧累加器 + `Capture_Jpeg()` 后组帧下发 |
| `src/net/WireProtocol.gd` | 新增 `BuildCameraFrame(sensor_id, seq, world_time, jpeg)`：15 B 头（`type=2`/`slot_id`/`seq`/`world_time`）+ JPEG 原始字节 |
| `src/net/ComponentLink.gd` | 把 `WebSocketPeer.inbound_buffer_size` / `outbound_buffer_size` 提到 4 MiB（默认 64 KiB，720p JPEG 会超） |
| `src/tests/check_camera_sensor.gd` | 追加：帧头字段正确、payload 以 `FF D8 FF` 开头、`rate_hz` 计时（`--fixed-fps 60`） |
| `src/tests/check_ws_manager.gd` | 追加：客户端声明 `sensor="camera"` → 收到 `sensor_ready` → 能解析 `type=2` 帧 |
| `COMPONENTS.md` | §3 组件清单 / §4.8 接口 / §8.2 二进制帧（type=2）/ §10 边界 / §11 映射 |

**硬要点**
1. `CameraSensor` 是**纯视觉零件、无碰撞**（E27），且**不进 `World.tscn`**（由 `SensorCatalog` 动态实例化）。
2. `Can_Capture()==false`（`--headless` = dummy 渲染器）时**只打一次 warning、绝不出帧**，否则 `await` 会永久挂住。
3. 出帧在 `_process` 累加器里做，不用 `UPDATE_ALWAYS`；一张图 = 一次 `UPDATE_ONCE` + `frame_post_draw` + `get_image()`（`CameraSensor` 现有流程不变）。
4. 帧大小与频率必须能从配置调（默认 640×480 @1 Hz，与 UAV 相机默认一致）。

**验收**
- `check_camera_sensor` / `check_ws_manager` 在 Xvfb 下 `SELF-CHECK OK`（headless 下相机项按既有约定 `[skip]`），其余 13 个自检断言数与基线逐字一致。
- `--quit` EXIT=0 且 `grep -E "SCRIPT ERROR|Parse Error|ERROR"` 为空。

### Step 4 — sim 侧：`camera` 传感器 + `camera_sim` 注入（`pleiades-sim`）

**改动文件**

| 文件 | 动作 |
|---|---|
| `src/world_link/catalog.rs` | `SENSORS` 加 `"camera"` |
| `src/sim/sensors/camera/mod.rs` | **新建**：`CameraSensor`（实现 `Sensor`），`On_Binary` 解析 `type=2` 帧，`Mutex<Option<Frame>>` 只存最新一帧；`Latest()` / `Age_Ms()` |
| `src/sim/sensors/camera/frame.rs` | **新建**：15 B 头 + JPEG payload 解析；宽高从 JPEG `SOF0` 读（不新增协议字段），短包/坏 JPEG 限流丢弃 |
| `src/sim/sensors/mod.rs` | `SensorSet::Build` 支持 `camera`，并提供类型化句柄（与 `lidars` 同法） |
| `src/sim/device_cap.rs` | **新建**：实现 `DeviceCapability`，注册 **`camera_sim`** 的 `capture()` / `info()`（无帧时返回 Err("还没有收到相机帧")，不阻塞） |
| `src/bootstrap.rs` / `src/main.rs` / `src/sim/sim_handler.rs` | `sim_bootstrap` 只增加一个 `capabilities` 参数（从 `CoreBootstrap.capabilities` 传入）；装配合格后把相机 cap 推进 `device_caps`。**不传、不加 `user_cmd_tx`，不碰控制口** |
| `src/config.rs` | `[[sim_world.sensor]]` 允许 `sensor="camera"`（现有 `Validate_Sensor` 放行） |

**硬要点**
1. `Sensor::On_Binary` 必须**同步**（收包 task 不 `await`），只覆盖 `Mutex` 里的最新帧。
2. 相机零件不参与建图、不参与急停；`SimDeviceHandler::on_tick` **一行不改**。
3. `grep -rE "pleiades_ugv|pleiades_uav" pleiades-sim/` 必须为空（硬约束沿用 task_31）。
4. `camera_sim.capture()` **不等待**未来帧：最新帧超过 2×周期就给 `age_ms` 警示（日志限流），仍返回该帧。

**验收**
- `cargo check -p pleiades-sim` 绿；`cargo test -p pleiades-sim` 通过（新增：帧解析、最新帧覆盖、无帧返回 Err、`SOF0` 宽高）。
- `grep -rE "pleiades_ugv|pleiades_uav" pleiades-sim/` 空。

### Step 5 — 世界 + sim 拍照验证（**由人类执行，agent 不代跑**）

> 本步只交付"可验证的状态"：代码、配置样例、排障清单。**验证动作由人类自己在 TUI 里完成**；本任务不新增任何自动化注入通道（不改控制口、不加 CLI、不改 Core/TUI）。

**人类验证的前置条件（agent 只负责说明，不代为执行/安装）**
- **虚拟显示**：`tmp/task33/xvfb/` 已被清理，本机当前**没有** `Xvfb`（`which Xvfb` 为空）；世界相机必须真渲染才能出图，所以跑世界前需要人类自己准备一个 X 显示（`sudo apt install xvfb` 后 `Xvfb :99 &`，或自备 Xvfb 解包目录）。
- **世界**：`DISPLAY=:99 godot --path orion-simulation-world --rendering-driver opengl3 --audio-driver Dummy`（**不加** `--headless`，headless 是 dummy 渲染器，相机出不了图）。
- **sim 实例目录**（`RobotSim_TestEnv/sim1/`）：`programs -> ../../../programs` 软链（`ProgramRegistry` 扫的是进程 CWD 下的 `programs/`，不软链则 `exec camera_photo` 会报未知命令）；`.config/config.toml` 增加相机零件声明：

```toml
[[sim_world.sensor]]
slot = "camera_nose"
sensor = "camera"
[sim_world.sensor.mount]
x = 0.15
y = 0.0
z = 0.12
yaw = 0.0
pitch = 0.0
[sim_world.sensor.params]
width = 640
height = 480
fov_deg = 70.0
quality = 0.8
rate_hz = 1.0
```

- **sim**：**带 TUI 启动**（`./pleiades-sim`，需要真终端；`--headless` 没有 TUI，也就没有命令输入），等日志出现 `[world] assembled`。

**人类在 sim 的 TUI 里执行（预期结果）**
1. 输入 `exec camera_photo` → 日志显示 `sensor=camera_sim`、`info()`（宽/高/格式/帧龄）与 JPEG 字节数。
2. `RobotSim_TestEnv/sim1/Pleiades_Workspace/camera_photo.jpg` 落盘；`file` 认出 JPEG、尺寸 = 640×480。
3. 图片应能看出模拟世界（地面/墙/柱），不是全黑、不是占位图。
4. 把 sim 换个朝向再 `exec camera_photo`，第二张图应不同（"对着哪拍哪"）。
5. 两侧日志无 `ERROR` / `SCRIPT ERROR` / `Parse Error`；退出后无 godot / pleiades-sim 残留进程。

**agent 在本步的交付物（仅此）**
- 上面这份前置说明 + 配置样例；
- 排障对照表（无帧 / 帧龄过大 / 全黑 / 尺寸不符 / `requested format not supported` / `Device or resource busy`）；
- 人类反馈失败时，按日志定位到 Step 3（世界侧）或 Step 4（sim 侧），**不擅自改核心代码**；
- 回写 `COMPONENTS.md`（相机零件可装配 + `type=2` 帧 + 配置样例）与 `Workbook/wb_34_camera.md`（每 step 一节：轮次/结果/关键数字/耗时）。

**明确不做**：不替人类起 world/sim、不在控制口加命令、不写自动化注入脚本、不把"未验证"记成"通过"。
