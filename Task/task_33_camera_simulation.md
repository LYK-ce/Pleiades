# task_33_camera_simulation — 摄像头仿真 + 传感器/底盘建模

> Presented by KeJi
> Created Date ： 2026-10-23
> Modified Date ： 2026-10-23
> 状态：**方向已定，待执行**（5 个 step，严格串行）
> 关联文档：`orion-simulation-world/COMPONENTS.md`、`Task/task_30_sim_world.md`、`Task/task_31_sim_world_link.md`、`Task/task_32_world_refactor_bounds.md`、`tmp/camera_sim/DESIGN.md`（调研稿）

---

## 一、目标

在 `orion-simulation-world`（Godot 工程）里做出**一台真的摄像头**——对着哪就拍到哪（真实渲染，不是占位图），并用它拍出几张图存进 `tmp/`。

同时把**传感器与底盘做成真实模型**（Blender 建模 → `.glb`），使"雷达装在车顶、相机装在车鼻、UAV 挂在机头下方"在三维视图里一眼可见，而不是用代码拼的方块。

**本 task 不涉及 sim 侧**（不碰 `pleiades-sim`、不跑决策链、不接网络协议）。

三条支撑目标：

1. **相机是真的**：世界渲染 → 取像素 → JPEG，画面就是相机对着的场景。
2. **模型可维护**：模型是资产文件（`.glb` + `.blend` 源），不是运行时拼几何；换外观不动代码。
3. **不破坏既有 11 个自检**：雷达 / 位姿 / 边界 / 装配全部保持绿灯。

---

## 二、已定决策（2026-10-23 讨论结论）

| # | 决策 | 说明 |
|---|---|---|
| D1 | **真实渲染** | `SubViewport` + `Camera3D` + `get_texture().get_image()`，拍到的就是相机对着的场景（真实几何/遮挡/透视/光照） |
| D2 | **单张按需拍摄** | `Capture_Jpeg()` 调一次拍一张；**不做**定时推流、**不做**请求/应答（推流与 `camera.capture()` 属 sim 接入那条线，本 task 不碰） |
| D3 | **1280×720** | 与真机 `[camera]` 默认值一致（`pleiades-uav/src/config.rs:119-124`）；分辨率做成配置项 |
| D4 | **JPEG** | `Image.save_jpg_to_buffer(quality)`，quality 默认 0.8；与真机输出同格式 |
| D5 | **模型用 Blender 建 `.glb`** | **不采用**运行时 `BoxMesh`/`CSG` 拼装；产出物是资产文件 |
| D6 | **雷达、相机各自独立建模** | 两个独立 `.glb`，可分别挂到 UGV / UAV |
| D7 | **底盘也要模型** | UGV 外观按 **长 30 × 宽 25 × 高 25 cm**；UAV 已有 `uav_quadrotor_lowpoly.glb`，先复核尺寸，不够再重建 |
| D8 | **传感器模型纯视觉** | **禁止** `CollisionShape3D`；光学中心 / 扫描面必须落在 holder 原点 |
| D9 | **一份模型两端通用** | 同一个 `camera_module.glb` 挂 UGV 车鼻与 UAV 机头，差别只在 `mount` |
| D10 | **`mount` 增加可选 `pitch`** | 缺省 0（度），向后兼容；雷达不传则行为不变 |
| D11 | **碰撞盒与外观严格一致** | `[chassis] wheel_base=0.30 / height=0.25 / track=0.25`；`ApplyBodySize` 后的 BoxShape3D 与模型 AABB **逐轴相等**，不允许"模型比碰撞体大一圈" |
| D12 | **UAV 相机俯仰是配置项** | `pitch` 不写死；由 sim 的 `[[sim_world.sensor]]` 配置选择，世界侧只透传，改角度不改代码 |

### 2.1 尺寸与坐标约定

世界坐标系：**Z-up**，`x`=东（车头 +x）、`y`=南、`z`=高；Godot 为 **Y-up**，换轴只在 `WorldTransform` 一处做。

| 对象 | 尺寸（米） | 备注 |
|---|---|---|
| UGV 车体 | 长 **0.30** × 宽 **0.25** × 高 **0.25** | 人类给定 |
| UAV 机体 | 现有 `uav_quadrotor_lowpoly.glb`（约 0.4 m） | 待复核 |
| 雷达件 | 罐体 ⌀0.07 × 高 0.05 + 底座 ⌀0.08 × 0.005 + 扫描窗 | 参考 YDLIDAR Tmini |
| 相机件 | 机身 0.04 × 0.03 × 0.03 + 镜头筒 ⌀0.02 × 0.015 | 参考 USB 模组 |

**模型原点约定（关键，D8）**：

| 模型 | 原点位置 | 朝向 |
|---|---|---|
| `lidar_tmini.glb` | **扫描面中心**（罐体底面中心） | 扫描窗朝 +x |
| `camera_module.glb` | **镜头光心**（镜头前表面中心） | 镜头看 +x |
| `ugv_chassis.glb` | **底面中心**（x=车长中心、z=0 贴地） | 车头朝 +x |
| `uav_quadrotor_lowpoly.glb` | 现有约定不变 | 机头朝 +x |

### 2.2 挂载点建议值（可改，域名为配置项）

| 平台 | 传感器 | `mount` | 依据 |
|---|---|---|---|
| UGV | 雷达 | `x=-0.05, y=0, z=0.25, yaw=0` | 车顶略靠后（侧视图） |
| UGV | 相机 | `x=+0.15, y=0, z=0.12, yaw=+π/2, pitch=0` | 车鼻前脸、腰线高度；**`yaw` 必须 π/2 才朝车头**（见下） |
| UAV | 相机 | `x=+0.20, y=0, z=0.045, yaw=+π/2, pitch=0`（pitch 由配置改） | 机头正面、**略低于机身中线**（2026-10-24 人类裁定）；实测 UAV 机腹平面 `z=0`、中线 `z=0.06`，故 `0.045` = 中线下方 1.5 cm |

> UAV 的 `pitch` 由配置决定（D12）：`0` = 纯前视，`-30` = 斜下。上表是**建议默认值**，不改代码即可换。

**`mount` 的基准点（易错，2026-10-24 实测校正）**：`mount` 是相对**机体底面中心**（= 车辆节点原点）的局部坐标，不是几何中心。因为 `ApplyBodySize` 把碰撞盒原点放在底面、`Body` 网格再抬 body/2。实测：

| 平台 | 机腹平面 | 机身中线 | 机头前缘 | 总尺寸（含旋翼/轮） |
|---|---|---|---|---|
| UGV（`ugv_chassis.glb`） | `z=0` | `z=0.125` | `x=+0.15` | 0.30 × 0.25 × 0.25 |
| UAV（`uav_quadrotor_lowpoly.glb`） | `z=0` | `z=0.06` | `x=+0.20` | 机身 0.40×0.12×0.40，旋翼展 0.64，总高 0.14 |

> 由此：写 `z=-0.02` 表示"机腹以下 2 cm"（视觉上悬空），不是"中线以下"。要"贴机头正面、略低于中线"就取 `z=0.045`。

> **朝向规则（2026-10-24 实测）**：相机光轴 = 传感器局部 `−Z`，而车头/机头 = 局部 `+X`，所以"朝前看"必须 `yaw = +π/2`（**弧度**）。实测：`yaw=0` → 光轴 `(0,0,-1)`（朝北，不是车头）；`yaw=π/2` → 光轴 `(1,0,0)`（车头）。`yaw` 是弧度、`pitch` 是度，两者单位不同，别混。

UAV 相机的配置示例（与上面表一致，`pitch` 按需改）：

```toml
[[sim_world.sensor]]
slot = "camera_nose"
sensor = "camera"
[sim_world.sensor.mount]
x = 0.20
y = 0.0
z = 0.045        # 机腹 z=0、中线 z=0.06 → 中线下方 1.5 cm
yaw = 1.5708     # 弧度！前视 = +π/2（光轴是局部 -Z，不转就朝左）
pitch = 0.0      # 度：0=前视，-30=斜下，-90=正下方
[sim_world.sensor.params]
width = 1280
height = 720
fov_deg = 70.0
quality = 0.8
```

---

## 三、现状（代码事实，逐条可核）

| 环节 | 现状 | 位置 |
|---|---|---|
| 二进制帧头 | `[0]u8 type \| [1]u16 slot_id \| [3]u32 seq \| [7]f64 world_time \| [15..] payload`；**已预留 `FRAME_TYPE_CAMERA := 2`**，无构造/使用 | `WireProtocol.gd:35-41` |
| 文本消息 | 7 条，**无抓拍请求**（本 task 也不需要，D2） | `WireProtocol.gd:25-32` |
| 传感器目录 | 只有 `lidar_2d` | `SensorCatalog.gd:22-33` |
| 组件装配 | 一组件一连接；`sensor_hello` → 目录校验 → 实例化 → `AttachSensor(slot, scene, mount, params)` → `sensor_ready` → `SetStream` | `WebSocketManager.gd:480-553` |
| 外参 | `mount={x,y,z,yaw}`，**无 pitch** | `Vehicle.gd:148-183` |
| 传感器场景 | `Lidar2D.tscn` 只有 `DebugPoints`（点云调试），**无外观模型** | `src/scenes/sensors/Lidar2D.tscn` |
| sim 传感器层 | `Sensor` trait = `Slot/On_Binary/Shutdown`；`SensorSet::Build` 只认 `lidar_2d` | `pleiades-sim/src/sim/sensors/mod.rs:72-133` |
| sim 零件目录 | `SENSORS=["lidar_2d"]` | `world_link/catalog.rs:17-40` |
| Lua 注入 | `DeviceCapability::register_lua_caps` → `capabilities.device_caps`；**sim 的 bootstrap 签名无 `capabilities`** | `pleiades-uav/src/bootstrap.rs:43-53`、`pleiades-sim/src/bootstrap.rs:21-56` |
| 真机相机 | `[camera]` 默认 1280×720、超时 3000 ms；`capture()` on-demand。**已知缺陷：从不调 `open_stream()`，`frame()` 必失败** | `pleiades-uav/src/device/camera/mod.rs:85,100-107,181` |
| 资产 | `assets/` 仅 2 个 glb（车/机低模）+ README；无传感器模型 | `orion-simulation-world/assets/` |

---

## 四、Step 1 — 建模（雷达 / 相机 / 底盘 / UAV 复核）

### 1 目标

产出可直接被 Godot 导入的 `.glb` 资产：**雷达一件、相机一件、UGV 底盘一件**，并复核/决定 UAV 是否沿用现有模型。**本 step 不写任何运行时逻辑**，只出资产 + 一个资产自检。

### 2 零件分解（逐件写死，避免建模跑偏）

**（a）`lidar_tmini.glb` — 2D 激光雷达**

| 部件 | 尺寸 | 说明 |
|---|---|---|
| 底座 | 圆柱 ⌀0.080 × h0.005 | 贴车顶，带 2 个 φ0.003 安装孔（可省略） |
| 罐体 | 圆柱 ⌀0.070 × h0.045 | 深灰金属，顶部平 |
| 扫描窗 | 环带（罐体外壁 z 方向 0.020~0.035） | 半透明偏蓝，**朝 +x 开口**，用材质区分即可 |
| 顶盖标记 | 小凸起（0.006 × 0.006 × 0.004） | 标出"正前方"，便于肉眼校验 yaw |
| 原点 | 罐体底面中心（= 扫描面中心） | D8 |

**（b）`camera_module.glb` — 车载相机**

| 部件 | 尺寸 | 说明 |
|---|---|---|
| 机身 | 立方 0.040 × 0.030 × 0.030（x×y×z，**x 为前后**） | 哑光黑 |
| 镜头筒 | 圆柱 ⌀0.020 × 长 0.015，沿 +x 伸出 | 前端面中心 = **原点（光心）** |
| 镜片 | 圆片 ⌀0.016，贴在镜头筒前端 | 深蓝高光 |
| 支架 | 薄板 0.006 × 0.030 × 0.040（在机身底部/背面） | 表示固定方式，可省 |
| 原点 | 镜片前表面中心 | D8 |

**（c）`ugv_chassis.glb` — UGV 外观（长30 × 宽25 × 高25 cm）**

| 部件 | 尺寸 | 说明 |
|---|---|---|
| 下车体 | 长方体 0.30 × 0.25 × 0.12（z 0~0.12） | 深灰 |
| 上车体/舱 | 长方体 0.22 × 0.20 × 0.10（z 0.12~0.22，居中略后） | 略浅灰 |
| 前挡斜面 | 楔形，从 (x=+0.11, z=0.22) 斜到 (x=+0.15, z=0.12) | 侧视图的关键特征 |
| 车顶平台 | 0.14 × 0.14 × 0.01（z 0.22~0.23） | 雷达安装面 |
| 车鼻前脸 | 0.02 厚立板（x=+0.14~0.15） | 相机安装面 |
| 轮子 ×4 | ⌀0.06 × 宽 0.02 | 位置沿用 `[chassis]` 的 `wheel_base=0.30`、`track=0.24` |
| 原点 | 车体底面中心（x=0 在车长中点，z=0 贴地） | 车头朝 +x |

> ⚠️ **碰撞盒已按 D11 同步**：`[chassis] wheel_base/height/track = 0.30/0.25/0.25`，`ApplyBodySize` 后 BoxShape3D 与上面这个 30×25×25 的 AABB **逐轴相等**，不存在"模型比碰撞体大一圈"。改动同时会动到 `check_diff_drive` / `check_vehicle` 的既有期望值，见 §Step1-6。

**（d）UAV — 先复核，不默认重建**

- 现有 `assets/uav_quadrotor_lowpoly.glb`（20 KB）：Step 1 里用 Blender/Godot 读出 AABB，确认：
  1. 尺寸是否合理（约 0.4 m 轴距）；
  2. 机头是否朝 +x；
  3. 原点是否在机体中心/底面；
  4. 是否有机头下方的挂载平面（相机要挂"略低于机身中线"）。
- 四条都满足 → **沿用**，只补挂载点；任一不满足 → 按 (b) 的相机规格重建一台 UAV 机体，命名 `uav_quadrotor_v2.glb`，旧的保留不删。

### 3 建模方式与工具链（**已实测可用，2026-10-23 核实**）

| 项 | 值 |
|---|---|
| Blender | **4.5.14 LTS**，位于 `/vepfs-mlp2/c20250205/240804016/tool/blender/blender`（包装脚本，带本地 lib 兜底） |
| 已封装工具 | `tool/blender/blend2glb.sh <in.blend> <out.glb>`（GLB 单文件、含材质、`export_apply=1`、`export_yup=True`） |
| GUI | **不可用**：本机无 `DISPLAY`、无 Xvfb；`blender -b` 可用，直接开窗会报 `Unable to open a display` |

因此 Step 1 的实际做法（**必须这么走**，与 `tool/blender/README.md` 的约定一致）：

1. 写 `Tool/blender/models/build_models.py`（bpy 脚本，`blender -b -P` 跑）：每个模型一个函数
   `Build_Lidar()` / `Build_Camera()` / `Build_Ugv()`，参数集中放在脚本顶部常量区；
2. 脚本**先存 `.blend` 源文件**到 `Tool/blender/models/*.blend`（可编辑、可复现），
3. 再逐件 `blend2glb.sh` 导出到 `orion-simulation-world/assets/…`。

> 说明：本机没有 GUI，所以"建模"这一步只能由 bpy 脚本产出网格；但**交付物是真的 `.blend` + `.glb` 资产**（可在任何 Blender 里继续手工编辑），不是运行时拼几何 —— 与 D5 不冲突。

- 材质：两三个 Principled BSDF（金属灰 / 哑光黑 / 半透明蓝），**不贴图**。
- 三角面预算：单件 ≤ 2000 tri（低模即可，视觉够用）。

### 4 产出文件

```
Tool/blender/models/
├── build_models.py              ← 新建（bpy 构建脚本：三件模型 + 参数常量区）
├── lidar_tmini.blend            ← 新建（可编辑源文件）
├── camera_module.blend          ← 新建
└── ugv_chassis.blend            ← 新建

orion-simulation-world/assets/
├── sensors/
│   ├── lidar_tmini.glb          ← 由 blend 导出
│   └── camera_module.glb        ← 由 blend 导出
├── chassis/
│   └── ugv_chassis.glb          ← 由 blend 导出
├── uav_quadrotor_lowpoly.glb    ← 保留（复核后决定是否替换）
└── ugv_x3plus_lowpoly.glb       ← 保留（Step 3 起由 ugv_chassis.glb 取代外观）
```

### 5 验收（硬性）

1. 4 个 `.glb` 存在，`file` 识别为 glTF binary；单件 ≤ 100 KB、≤ 2000 tri。
2. `godot --headless --path orion-simulation-world --import` 成功，无 ERROR。
3. 新增 `src/tests/check_sensor_models.gd`，逐件断言：
   - 能被 `load()` 成 `PackedScene`；
   - **AABB 尺寸**：雷达 ≤ 0.09×0.09×0.06、相机 ≤ 0.06×0.04×0.04、UGV = 0.30×0.25×0.25（±5%）；
   - **原点位置**：雷达 AABB 的 z 最小值 ≈ 0、x/y 中心 ≈ 0；相机镜片前端 x ≈ 0；UGV z 最小值 ≈ 0；
   - **无 `CollisionShape3D` / `CollisionPolygon3D`**（D8 硬性）；
   - 材质非空（`surface_get_material` 不返回 null）。
4. 既有 11 个自检全部仍 `SELF-CHECK OK`（本 step 只加资产，理论零回归）。
5. `git status`：只增 `assets/sensors/`、`assets/chassis/`、`Tool/blender/`、`src/tests/check_sensor_models.gd`、`COMPONENTS.md`。

### 6 已定 + 连带影响（原两个开放项已裁）

| # | 结论 | 连带影响（Step 1 / Step 3 必须一起做） |
|---|---|---|
| **Q1** | **碰撞与建模一致**：`[chassis] wheel_base=0.30`（不变）、`height 0.16 → 0.25`、`track 0.24 → 0.25` | ① `DiffDrive.Configure` 的 `ApplyBodySize(wheel_base, height, track)` 自动变成 `0.30 × 0.25 × 0.25`，与模型 AABB 相等；② **既有自检的期望值必须同步**：`check_diff_drive.gd` 的"撞墙停"位置（原 x=127.3499）、`check_vehicle.gd` 的 body/collision 尺寸断言；③ 车体变大 → D\* 的贴邻膨胀余量与急停 0.3 m 阈值的实际余量下降，属**行为变更**，需在 Step 3 的验收里重跑一次 goto 冒烟并记录耗时差 |
| **Q2** | **UAV 相机 pitch 走配置**（D12） | 配置写 `[sim_world.sensor.mount] pitch = -30`（或 0）；世界侧 `AttachSensor` 只按 `mount.pitch` 设 `rotation_degrees.x`，**不预置任何角度**；sim 侧 `Mount.pitch` 透传 |

配置示例（写进 `RobotSim_TestEnv/sim1/.config/config.toml`，Step 3/4 验收用）：

```toml
# UGV：车顶雷达 + 车鼻相机
[[sim_world.sensor]]
slot = "lidar_top"
sensor = "lidar_2d"
[sim_world.sensor.mount]
x = -0.05
y = 0.0
z = 0.25
yaw = 0.0
[sim_world.sensor.params]
range = 12.0
rays = 360
rate_hz = 10

[[sim_world.sensor]]
slot = "camera_nose"
sensor = "camera"
[sim_world.sensor.mount]
x = 0.15
y = 0.0
z = 0.12
yaw = 1.5708         # 前视 = +π/2 弧度（相机光轴是局部 -Z）
pitch = 0.0          # 度：0 = 纯前视；要斜下就写 -30
[sim_world.sensor.params]
width = 1280
height = 720
fov_deg = 70.0
quality = 0.8
rate_hz = 1.0
```

### 7 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | 本机无 Blender | 先装/换机器；产出 `.glb` 入库后不再依赖 Blender |
| 2 | 模型原点与"光学中心/扫描面"不一致 | 建模时先把原点钉死（§2.1），导出 `export_apply` |
| 3 | glTF 的 Y-up 与 Godod Y-up 方向搞反 | 统一 `export_yup=True`，导入后 AABB 在 `check_sensor_models.gd` 里断言"车头在 +x、高在 y" |
| 4 | 模型过大导致相机拍到自己的壳 | 相机件尺寸小（4 cm），挂载点前移到 x=+0.15 |
| 5 | 现有 `ugv_x3plus_lowpoly.glb` 与新底盘尺寸冲突 | **已定（D11）**：外观换成 `ugv_chassis.glb`，碰撞盒同步改成 0.30/0.25/0.25；旧 glb 保留不删，便于回退 |

---

## 五、Step 2 — 世界侧相机组件（离屏渲染 → JPEG）

### 1 目标

在世界里做出一个**能出图**的相机零件：给它一个 `Configure(params)`，它就能渲染出"当前对着的画面"并交出一段 JPEG 字节。**本 step 不挂模型**（模型在 Step 3 接）、**不接网络**（本 task 不涉及 sim）。

### 2 组件设计

```
CameraSensor (Node3D, CameraSensor.gd)        ← 挂载点 = 光心（D8）
└── Viewport (SubViewport, size=1280x720, own_world_3d=false, update_mode=UPDATE_ONCE)
    └── Lens (Camera3D, fov=70, near=0.05, far=200)
```

- **必须 `own_world_3d = false`**（共享父视口的 World3D，否则渲染空世界）。
- **必须 `update_mode = UPDATE_ONCE`**，不能 `ALWAYS`（否则每帧都在渲）。
- `Configure(params)`：`width` / `height` / `fov_deg` / `quality`（默认 1280 / 720 / 70 / 0.8）；支持字典缺键（与 `Lidar2D.Configure` 同款容错）。

### 3 公开接口（本 step 唯一要交付的能力）

| 方法 | 语义 |
|---|---|
| `Configure(params: Dictionary) -> void` | 设分辨率 / FOV / JPEG 质量；重设 `SubViewport.size` |
| `Capture_Jpeg() -> PackedByteArray`（`await`） | 渲染一帧 → 返回 JPEG 字节；失败返回空数组并在 `push_error` 里说明 |
| `Capture_To_File(abs_path: String) -> bool`（`await`） | 上面 + 落盘（供 Step 4 拍照用） |
| `Get_Image_Size() -> Vector2i` | 白盒断言用 |

### 4 `Capture_Jpeg()` 流程（顺序不能变）

```
① warmup 未过（前 2 帧）→ await 两次 process_frame（首帧是黑的）
② sv.render_target_update_mode = SubViewport.UPDATE_ONCE   # 触发本帧渲染
③ await RenderingServer.frame_post_draw                     # 必须：否则读到上一帧/空缓冲
④ img = sv.get_texture().get_image()                        # GPU → CPU 回读
⑤ return img.save_jpg_to_buffer(quality)                    # 1280×720 估 15~50 ms
```

> 不做 1 Hz 定时器：**本 task 只要"能拍"**。定时推流属于 sim 接入那条线，见 §九非目标。

### 5 验收

1. 新增 `src/tests/check_camera_sensor.gd`（`SceneTree` 脚本形态，参照 `check_*.gd`）：
   - 场景可实例化、`Configure` 后 `Get_Image_Size() == (1280,720)`；
   - `Capture_Jpeg()` 返回非空，且前 3 字节 = `FF D8 FF`（JPEG 魔数）；
   - 解析 JPEG 的 `SOF0` 段，宽高 = 1280×720；
   - **图像非全黑**（灰度方差 > 阈值，证明"真的渲染了世界"而不只是空缓冲）；
   - 改 `fov_deg` 后同一机位两次成图**不同**（证明 FOV 生效）；
   - headless 下能出图（否则本 step 直接 FAIL，见风险 1）。
2. `godot --headless --path orion-simulation-world --quit`：EXIT=0、`grep -E "SCRIPT ERROR|Parse Error|ERROR"` 为空。
3. 既有 11 个自检 + Step 1 的 `check_sensor_models` 全 PASS。
4. `COMPONENTS.md`：§1.1 目录、§3 场景清单、§4.7（新）相机组件接口、§9.1 新增 E26（`SubViewport` 必须共享 World3D + 必须 `frame_post_draw`）。

### 6 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | **headless 取不到图** | 先跑 `tmp/camera_sim/DESIGN.md` §8.1 探针；失败改 `xvfb-run` 或 `--rendering-driver opengl3`；仍失败则本 step 阻塞上报 |
| 2 | 1280×720 回读 + 编码 15~50 ms | 本 task 只拍几张，无需优化；配置项可降 640×360 |
| 3 | 首帧黑 | warmup 丢前 2 帧 |
| 4 | `SubViewport` 不共享 World3D → 空世界 | `own_world_3d=false` + 写进 E26 |
| 5 | `get_image()` 返回 null（渲染未就绪） | 判空 + `push_error`，自检里断言失败原因可见 |

---

## 六、Step 3 — 传感器模型接入场景（外观接线 + 位置校准）

### 1 目标

把 Step 1 的 `.glb` 挂进传感器场景与车辆场景，且**不影响任何物理/感知行为**。

### 2 改动

| 文件 | 动作 | 内容 |
|---|---|---|
| `orion-simulation-world/…/.config/config.toml`（测试环境） | 改 | `[chassis]`：`height 0.16 → 0.25`、`track 0.24 → 0.25`（`wheel_base=0.30` 不变），使 BoxShape3D = 模型 AABB（D11） |
| `src/scenes/sensors/Lidar2D.tscn` | 改 | 加 `Body(MeshInstance3D)` 子节点挂 `lidar_tmini.glb`；保留 `DebugPoints` |
| `src/scenes/sensors/CameraSensor.tscn` | 改 | 加 `Body(MeshInstance3D)` 挂 `camera_module.glb` |
| `src/scenes/Vehicle.tscn` | 改 | `ModelUGV` 的 mesh 换成 `ugv_chassis.glb` |
| `src/vehicle/Vehicle.gd` | 改 | `AttachSensor` 支持可选 `pitch`（缺省 0）：按 `mount.pitch` 设 `rotation_degrees.x`，符号由自检锁定（D12：**不预置任何角度**） |
| `src/tests/check_models.gd` | 改 | 增加"传感器模型已挂、无碰撞、AABB ≤ 规格、车体模型 AABB = 碰撞盒尺寸"断言 |
| `src/tests/check_diff_drive.gd` | 改 | 车体变大 → 更新"撞墙停"期望位置（原 `x=127.3499`）与其他尺寸相关断言 |
| `src/tests/check_vehicle.gd` | 改 | 更新 body/collision 尺寸断言（0.30 × 0.25 × 0.25） |
| `src/tests/check_lidar.gd` / `check_camera_sensor.gd` | 改/加 | 断言模型不影响射线/成像：加模型前后 `ScanOnce()` 结果一致（模型无碰撞） |

### 3 验收

1. 三个自检（models / lidar / camera_sensor）全 PASS。
2. **传感器零影响**：加模型前后，`check_lidar` 的 48 条断言数字不变（尤其墙距 152±）——模型无碰撞，射线结果必须逐字一致。
3. **车体变更是有意的**（D11）：`check_diff_drive` / `check_vehicle` 的期望值按新尺寸更新后 PASS；并**重跑一次 goto 冒烟**（`goto 40,80`），把到达时间与改前对比记进 workbook——车体从 0.16/0.24 变 0.25/0.25，贴墙与绕行会略变，属预期。
4. 视觉核对：`--quit` 截图或人工开窗确认——雷达在车顶、相机在车鼻、UAV 相机在机头下方、车身与碰撞盒不"穿模"。
5. `COMPONENTS.md`：§4.3 节点树补 `Body`、§4.6/§4.7 补模型说明、§9.1 新增 E27（传感器模型不得带碰撞）、§10 记 `[chassis]` 新默认值。

---

## 七、Step 4 — 模拟世界拍照（出图到 `tmp/`）

### 1 目标

在 `orion-simulation-world` 里**不启动 sim、不跑决策链**，只把"一台静止的 UGV（挂建模的雷达 + 相机）"摆进世界，用**建模出来的那台相机**拍 3~5 张图，落到**本目录的 `tmp/task33_shots/`**。

### 2 产出与做法

| 产出 | 路径 |
|---|---|
| 拍照自检脚本 | `orion-simulation-world/src/tests/check_camera_shots.gd` |
| 图片输出 | `tmp/task33_shots/shot_00_*.jpg` …（≥3 张，1280×720） |

脚本形态（参照 `check_*.gd` 的 `SceneTree` 自检写法）：

```
1) load("res://src/scenes/World.tscn")，实例化并 add_child（真世界：体素/地面/空气墙）
2) 实例化 Vehicle.tscn，摆到世界某处（如 (64,64,0)），rotate 到指定朝向
3) AttachSensor("lidar_top", Lidar2D 场景, mount{…})      ← 用 Step 1 的模型
   AttachSensor("camera_nose", CameraSensor 场景, mount{…, pitch})
4) 机位循环（≥3 个）：
     ① 车朝向 A，拍一张            → shot_00_*
     ② 车朝向 A+90°，拍一张        → shot_01_*   （证明"对着哪拍哪"）
     ③ 相机装到世界另一处（如墙边）→ shot_02_*   （证明不是固定背景）
5) 每张写真：tmp/task33_shots/<name>.jpg（绝对路径写盘）
```

**关键约束**：
- **不启动 `pleiades-sim`、不连 WS、不跑 `on_tick`**（用户明确"连车都不用启动"）。
- 输出必须是**绝对路径**写入 `tmp/task33_shots/`（`Image.save_jpg` 支持绝对路径），不写 `res://`（会把图塞进工程）。

### 3 验收（硬性）

1. `tmp/task33_shots/` 下 **≥3 张 `.jpg`**，`file` 识别为 JPEG，尺寸 1280×720，单张 > 10 KB。
2. `check_camera_shots.gd` 自检 PASS，断言：
   - 每张图**非全黑**（灰度方差 > 阈值）、非全白；
   - 三个机位的图**两两不同**（字节或直方图差异 > 阈值）——证明"真的对着拍"；
   - 图里能读到世界几何（用"画面中亮暗格数 ≥ N"这类弱断言，不做像素级比对）；
   - 相机自身不被完全遮挡（画面底部车鼻占比 < 50%，防"拍到自己的壳"）。
3. `godot --headless --path orion-simulation-world --quit`：EXIT=0、无 `ERROR|SCRIPT ERROR|Parse Error`。
4. 既有 11 个自检 + `check_sensor_models` + `check_camera_sensor` 全 PASS。
5. **人工确认**：打开 `tmp/task33_shots/*.jpg`，能看到世界（地面/墙/柱）与车鼻/雷达模型。
6. `COMPONENTS.md`：§4.7 补 `Capture_To_File` / 拍照脚本说明；§11 加 `task_33-4` 行。

### 4 风险

| # | 风险 | 处理 |
|---|---|---|
| 1 | headless 下渲染为空 | Step 2 已先行验证；若此处仍空，退 `xvfb-run` 跑同一脚本 |
| 2 | 相机贴着车身 → 画面被挡 | 挂载点前移到 `x=0.15`、`near=0.05`；自检里断"车鼻占比 < 50%" |
| 3 | 世界是程序化方块，照片"看着单调" | 属预期；本 step 验的是"能拍出真实场景"，不是好看 |
| 4 | 并行跑多个 godot 抢渲染/端口 | 逐个串行跑；`--fixed-fps 60` + 外部 `timeout` |

---

## 八、全局约束

1. 只改 `orion-simulation-world/`、`Tool/blender/models/`、`tmp/task33/`、`tmp/task33_shots/`、`Workbook/`；**不动** `pleiades-base` / `pleiades-ugv` / `pleiades-uav` / `pleiades-terminal` / `pleiades-sim`（本 task 不涉及 sim 侧）。
2. 归属注释：新建 `.gd` / `.py` / `.md` 一律带 `Presented by KeJi` + Created/Modified Date。
3. 命名：函数 `Pascal_snake`、变量 `snake_case`、常量 `UPPER_SNAKE`（`COMPONENTS.md §9.2`）。
4. 不执行 `git add` / `git commit`。
5. 每个 step 结束后更新 `Workbook/wb_33_camera_simulation.md`。

## 九、非目标（本 task 明确不做）

- **sim 侧任何改动**：`pleiades-sim` 的 camera 传感器、`camera.capture()` Lua 绑定、`device_caps` 注入 —— 全部不在本 task。
- **网络协议**：`FRAME_TYPE_CAMERA=2` 的帧构造、WS 推流、`sensor_hello` 的 camera 目录登记 —— 等 sim 接入那条线再做。
- 1 Hz 定时推流、请求/应答式抓拍、视频流。
- 相机噪声 / 曝光 / 运动模糊模拟。
- 相机 → YOLO 的精度验证（世界里是低模方块，COCO 类目认不出）。
- 传真机 `open_stream()` 的缺陷（`pleiades-uav`，另行处理）。
- 修 ugv/sim 建图的 free 语义（上一话题，人类明确"先不改"）。
