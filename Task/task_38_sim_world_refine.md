# task_38_sim_world_refine — 仿真世界观感精修（城市街道 + PBR 材质 + HDRI）

> Presented by KeJi
> Created Date ： 2026-10-28
> Modified Date ： 2026-10-28
> 状态：**Step 1/2 已 PASS（世界精修完成）；Step 3（新世界 Follow 复验）FAIL，卡在动作层，待 task_39 后复跑**
> 前置：无（不依赖 task_37；task_37 改的是 Rust 侧 detector，本任务只动 Godot 世界侧）
> 关联：`Task/task_30_sim_world.md`（世界搭建）、`Task/task_33_camera_simulation.md`（相机/传感器）、`Task/task_34_camera.md`（相机 sensor）、`orion-simulation-world/COMPONENTS.md`

---

## 一、总体目标

把 `orion-simulation-world` 从"素体灰模"升级为**城市街道**观感，并在新世界里**复验** task_37 的跟随链路。五项观感要求 + 一项复验（人类 2026-10-28 定）：

1. **地面/路面**：沥青路面 + 混凝土人行区（PBR 三件套，不再是单色）；
2. **路缘**：马路牙子，**纯视觉、不加碰撞**；
3. **标线**：车道中心虚线、两侧实线、斑马线；
4. **天空**：换 **HDRI** 全景（替换现在的 `ProceduralSkyMaterial`），环境光随之改变；
5. 附加（人类同意）：**建筑立面感**——体素块要有窗、有基座、有屋顶压顶，不能是"一块贴了水泥的砖"；
6. **复验（Step 3，人类 2026-10-28 追加）**：在精修后的世界里重跑 task_37 的 **Follow** 端到端——贴图 / HDRI / 标线 / 立面**不得**破坏"相机 → detector → 地面投影 → 跟随"这条链。

### 1.1 现状（为什么现在是素体）

| 位置 | 现状 |
|---|---|
| `src/scenes/World.tscn:39-41` | 地面材质 = 单色 `(0.22,0.24,0.26)`、roughness 0.9，无贴图 |
| `src/scenes/World.tscn:36-44` | 地面 = 一整块 `BoxMesh(128,1,128)`（D10：不做体素地面） |
| `src/scenes/World.tscn:28-34` | 环境只有程序天空 + 环境光 + tonemap；无 AO、无雾 |
| `src/scenes/World.tscn:52-54` | 唯一光源 = `DirectionalLight3D`（-55°/-35°），有阴影 |
| `src/world/World.gd:289-306` | 1.2 万个体素**共用一个材质**、单色 `(0.55,0.58,0.62)`、roughness 0.85 |
| `src/world/World.gd:35-57` | `OBSTACLES` = 4 面边界矮墙 + 11 个轴对齐方块（建筑/长墙/柱/退台/散块） |
| `src/world/World.gd:116` | 世界预置 2 个行人（`PEDESTRIANS`）；其余行人由 WS 按需创建（静态装饰人 `PERSONS` 已于 `d46a339` 移除） |
| `assets/README.md` | 明写"当前无资源文件，世界与全部组件均为程序化生成（D4）"——本任务后该句必须改写 |

**没有街道、没有路缘、没有标线、没有任何贴图。**

### 1.2 已定决策（人类 2026-10-28）

| # | 决策 | 说明 |
|---|---|---|
| 1 | **场景定调 = 城市街道** | 沥青 + 路缘 + 标线（不是野外/工地） |
| 2 | **街道布局由 agent 决定** | 见 §三.2 的路网表；原则是**只在既有空地铺，绝不移动既有障碍/摆位** |
| 3 | **路缘不碰撞** | 纯视觉：`collision_layer = 0`、无 `CollisionShape3D`；不得影响 LiDAR / 急停 / 寻路 |
| 4 | **建筑加立面感** | 窗 + 基座 + 屋顶压顶；**不改体素体量、不改 `OBSTACLES`** |
| 5 | **天空换 HDRI** | CC0 全景 HDRI；`ambient_light_source = SKY` 已是现状（`World.tscn:31`，值 3），换天空材质即自动接管环境光 |
| 6 | **材质资产只用 CC0** | Poly Haven / ambientCG / 3DTextures.me / cgbookcase；`assets/README.md` 记来源 + 许可 |
| 7 | **`World.tscn` 零改动** | 天空/材质/几何全部**运行时**建（与 `BuildPersons()` / `_Build_Bounds()` 同风格）；手写 tscn 不得用编辑器另存 |
| 8 | **分三步** | Step 1 = 材质管线与天空（不碰几何/布局）；Step 2 = 街道几何与建筑立面；Step 3 = 新世界下复验 task_37 的 Follow（只验证 + 最小修正，**不改 follow 逻辑**） |
| 9 | **不改世界尺度与物理语义** | 128×128×32 m、`cell_size=0.5`、碰撞层（地面/体素/车）、出生点 (64,64)、`OBSTACLES`/`PEDESTRIANS` 全部不动 |

### 1.3 全局硬约束（三步共用）

- **Step 1/2 只改** `orion-simulation-world/` + `COMPONENTS.md` + `assets/README.md`；**Step 3 额外允许** `Tool/`（新增复验脚本）与 `RobotSim_TestEnv/<实例>/`（只改实例 `config.toml` 与拷权重）。**任何一步都不碰** `pleiades-*`（follow 逻辑属 task_37）、`Task/`（本文件除外）、协议与 Rust 测试。
- **贴图只放 `assets/`**（`.png/.jpg/.hdr`），代码/场景只放 `src/`；贴图必须连同 `.import` 一起提交。
- **干净检出必须先 `godot --headless --path orion-simulation-world --import`**，否则 `load()` 返回 null / 贴图丢失（E28 的教训）。
- **尺寸 1K~2K，禁止 8K**；`assets/` 总增量 **≤ 10 MB**。
- **不新增插件、不新增第三方脚本**；只用 Godot 内置节点与 `StandardMaterial3D`。
- 既有 **17 个自检脚本全部 `failed=0`**：`check_project` / `check_world` / `check_camera` / `check_camera_sensor` / `check_camera_shots` / `check_vehicle` / `check_diff_drive` / `check_quadrotor` / `check_ws_manager` / `check_lidar` / `check_pose` / `check_bounds` / `check_models` / `check_sensor_models` / `check_pedestrian` / `check_pedestrian_physics` / `check_pedestrian_shots`（断言数以开工时实测为准；`person_*` 系列已随 `d46a339` 移除）。
- 出图验收只能在 **Xvfb**（`Xvfb :99` + `--rendering-driver opengl3` + `--audio-driver Dummy`）下做（headless 是 dummy 渲染，E26-③）。
- 新 `.gd` 必须带 `Presented by KeJi / Created Date / Modified Date` 头注释；函数 `Upper_Snake`、变量 `lower_snake`，与本目录既有风格一致。
- **验收分工：三步全部由 agent 自己跑、自己出证据，不需要人类现场操作，也不需要 Pictor**（详见 §四.3）。人类只做最终 review 与拍板。

---

## 二、Step 1 — 材质管线与天空（PBR + HDRI）

> **执行引擎里拆成两段跑**（2026-10-29 加）：**1a = 资产入库**（只下载 + `.import` + README 记账，不写代码、不跑 Godot 长任务）、**1b = 材质接线与验收**（WorldLook + check_world_look + 出图）。拆开是为了让"资产入库"与"材质接线 + 验收"各自独立判 PASS，互不牵连。

### 1 目标

在不改任何几何/布局的前提下，让世界"有材质、有天光"：

- 地面 → 混凝土铺装（PBR，世界空间 triplanar，6 m 一循环）；
- 体素 → 混凝土（PBR，世界空间 triplanar，2 m 一循环）；
- 天空 → HDRI 全景（`PanoramaSkyMaterial`），环境光自动来自天空；
- `DirectionalLight3D` 与 HDRI 的太阳方向对齐，能量下调，避免"双太阳"假影。

### 2 资产清单（全部 CC0，1K~2K）

| 用途 | 来源 | 文件名（建议） | 需要的通道 |
|---|---|---|---|
| 地面铺装 | ambientCG `PavingStones` / `Concrete` 类目任选一件 2K | `assets/textures/ground_paving_*.jpg` | Color / NormalGL / ORM |
| 体素墙 | ambientCG `Concrete` 或 Poly Haven `concrete_wall_*` 2K | `assets/textures/concrete_*.jpg` | Color / NormalGL / ORM |
| 路面沥青（Step 2 用，本步先入库） | Poly Haven [`asphalt_02`](https://polyhaven.com/a/asphalt_02) 2K | `assets/textures/asphalt_*.jpg` | `diff` / `nor_gl` / `arm` |
| 路缘/屋顶（Step 2 用） | 同混凝土系列换一件（略深的色号） | `assets/textures/curb_*.jpg` | Color / NormalGL / ORM |
| 天空 HDRI | Poly Haven HDRI（晴天正午，2K `.hdr`） | `assets/hdri/sky_*.hdr` | — |

**通道约定（写死，别搞混）**：
- Poly Haven 的 `arm` **就是** Godot 的 `orm_texture`（R=AO / G=Roughness / B=Metallic），直接挂；
- Poly Haven 用 `nor_gl`；**禁止用 `nor_dx`**（Godot 是 OpenGL 约定）；
- ambientCG 给的是分离通道，需自己拼 ORM（R←AmbientOcclusion、G←Roughness、B←Metalness），拼一次存成 `*_orm.png`，**不要**在运行时拼。

**下载通道**：直连失败时走本机 **mihomo** 代理（监听 `127.0.0.1:7897`，`http_proxy` / `https_proxy` 指过去即可；`9090` 是它的控制口，别当代理用）。**代理只用于下载**，不得写进仓库里的任何脚本/配置默认值；实际用了什么通道记进 workbook。

**许可记录**：每件资产在 `assets/README.md` 里记 **来源 URL + 许可（CC0）+ 处理方式（尺寸/通道拼装/是否降采样）**，一件一行。

### 3 落点（文件 → 改什么）

| # | 文件 | 改什么 |
|---|---|---|
| 1 | `src/world/WorldLook.gd`（**新增**） | 材质工厂：`Make_Ground_Material()` / `Make_Voxel_Material()` / `Make_Sky_Material()`；贴图 `load()` + **缺失返回 null 并降级为原单色材质**（绝不 push_error）；triplanar 参数集中在此 |
| 2 | `src/world/World.gd` | `_ready()` 里 `BuildVoxelWorld()` 之后、`_Build_Bounds()` 之前，调 `WorldLook.Apply_All(self)`；`_make_mesh_library()` 不再自建 `StandardMaterial3D`，改为取 `WorldLook.Make_Voxel_Material()`；新增常量 `LOOK_ENABLED`（缺资产时整体关闭） |
| 3 | `src/world/World.gd` | 天空：`$WorldEnvironment.environment` 复制一份 → `sky.sky_material = WorldLook.Make_Sky_Material()` → 回写（不改 `World.tscn`） |
| 4 | `src/tests/check_world_look.gd`（**新增**） | 断言：材质非 null 且三张图都在、`uv1_triplanar && uv1_world_triplanar`、`uv1_scale` 符合表、天空材质类型 = `PanoramaSkyMaterial`、`background_mode`/`ambient_light_source` 未变、地面 AABB 与 `Ground` 节点未变 |
| 5 | `assets/README.md` | 改写"当前无资源文件"那段 → 新增 `textures/` 与 `hdri/` 小节（来源 URL / 许可 / 处理方式 / 尺寸） |
| 6 | `COMPONENTS.md` | §2 节点表补 `WorldLook`、§4.1 补运行时材质/天空、§9.1 新增坑（见 §五 风险）、§10 自检清单加 `check_world_look` |

### 4 不做

- 不铺任何街道几何、不动 `OBSTACLES`、不动任何位置表（那是 Step 2）；
- 不改 `World.tscn`（`git diff -- src/scenes/World.tscn` 必须为空）；
- 不引入 `ShaderMaterial`（Step 1 只用 `StandardMaterial3D`，把 shader 风险全部推到 Step 2 之外）；
- 不做夜间/雨天/发光窗、不做体积雾；
- **不把 SSAO 写进判据**：自检跑的是 Compatibility（`opengl3`）渲染器，SSAO/SSIL 在该渲染器下**不生效**；接触阴影靠 ORM 里的 AO + `DirectionalLight` 阴影。

### 5 完成判据

1. **贴图真在**：`Ground` 材质与体素 `MeshLibrary` 材质的 `albedo_texture` / `normal_texture` / `orm_texture` 全部非 null（`check_world_look` 断言）。
2. **triplanar 生效**：两者 `uv1_triplanar == true` 且 `uv1_world_triplanar == true`；地面 `uv1_scale ≈ (1/6, 1/6, 1/6)`、体素 `≈ (1/2, 1/2, 1/2)`（米制换算的等价写法由实现选定，自检只断言"非 1 且三者相等"）。
3. **天空换成 HDRI**：`env.sky.sky_material is PanoramaSkyMaterial` 且 `panorama != null`；`env.background_mode == 2` 与 `env.ambient_light_source == 3` **保持不变**（`check_world` 原有断言不许红）。
4. **零回归**：既有 17 个自检 `failed=0`；新增 `check_world_look` `total=N failed=0`。
5. **出图**：Xvfb 下 `check_camera_shots` / `check_pedestrian_shots` / `check_camera_sensor` B 组全绿；另存 3 张 1600×900 对比图（地面近景 / 建筑侧视 / UAV 俯视）。
6. **像素判据不飘**：`check_camera_shots` 的 `VAR_MIN=0.002` / `MEAN 0.05~0.95` / `WHITE_MAX=0.50` / `GRID_*` 全部照原值通过；**若某项需要放宽，必须在 workbook 记录"改前实测值 / 改后实测值 / 放宽倍数"**，并确认仍在文档声明的 3~5 倍余量内。
7. **体积**：`du -sh orion-simulation-world/assets` 增量 ≤ 10 MB。
8. **`.tscn` 零改动**：`git diff -- orion-simulation-world/src/scenes/World.tscn` 为空。

---

## 三、Step 2 — 街道几何与建筑立面

### 1 目标

在 Step 1 的材质基础上，把"空地"变成"街道"，并让体素建筑有立面：

- 路面（沥青）+ 路缘（纯视觉）+ 标线（几何细条，不用贴图）；
- 建筑：窗、基座、屋顶压顶（**全部用 `StandardMaterial3D` + 合并 mesh，不写 shader**）；
- 布局确定性、幂等可重建（与 `BuildVoxelWorld()` / `BuildPersons()` 同风格）。

### 2 路网（本任务定稿，坐标为世界坐标 Z-up / 米）

**选址原则**：只在既有空白区铺；**与既有障碍/敏感点冲突时改路段端点，不动障碍**（`OBSTACLES` 是雷达热区、拍照停车点、行人摆位等自检的敏感目标）。

| 元素 | 范围（世界坐标） | 说明 |
|---|---|---|
| 主路（东西向） | `x ∈ [4, 124]`，`y ∈ [60, 68]`，路面顶 `z = 0.01` | 8 m 宽 / 120 m 长；**穿过出生点 (64,64)**，机器人默认站在路中央 |
| 支路（南北向） | `x ∈ [44, 52]`，`y ∈ [68, 92]` | 8 m 宽 / 24 m；北端止于 `wall_c`（y=92）南侧，形成 **T 型路口** |
| 斑马线 | `x ∈ [56, 60]`，跨主路 `y ∈ [60,68]` | 4 条 0.5 m 白条（间隔 0.5 m），长 8 m（横跨路面） |
| 中心虚线（主路） | `y ≈ 64`，`x ∈ [4,124]` | 3 m 线 + 3 m 空，0.15 m 宽；**路口段 `x∈[44,52]` 与斑马线段 `x∈[56,60]` 跳过** |
| 边线（主路） | `y ≈ 60.30` 与 `y ≈ 67.70` | 实线 120 m × 0.15 m |
| 中心虚线（支路） | `x ≈ 48`，`y ∈ [68,92]` | 3 m 线 + 3 m 空 |
| 路缘（主路南） | `y ∈ [60, 60.2]`，`z ∈ [0, 0.15]` | 在斑马线断开（降坡） |
| 路缘（主路北） | `y ∈ [67.8, 68]` | 在支路口 `x∈[44,52]` 与斑马线 `x∈[56,60]` 断开 |
| 路缘（支路东/西） | `x ∈ [44,44.2]` / `x ∈ [51.8,52]`，`y ∈ [68,92]` | 与主路北缘相接 |

```text
            y=92  ┌───────────┐ wall_c（y∈[92,93]）
                  │  支路(8m) │
                  │  x 44..52 │
            y=68 ─┴──┬────┬───┴──────────────────────────── 北缘
      主路(8m)  ══════╪════╪═════════════════════════════════  y∈[60,68]
            y=60 ────┴────┴──────────────────────────────── 南缘
                  x=44 x=48 x=52        斑马线 x 56..60
            x=4                                              x=124
```

**为什么不是完整街区网格**：重排 `OBSTACLES` 会同时改动 `check_lidar` 的热区 (60,90)/(100,60)、`check_camera_shots` 的停车点 (60,84)、行人摆位 (44,96)/(52,102)、`check_ws_manager` 的 spawn 点 (64,56)/(70,70)/(72,74)/(72,78)，波及面远大于"画条街"。**主路 + 支路 + 斑马线足够读出"城市街道"**，且零波及。

### 3 建筑立面（不写 shader）

对 `OBSTACLES` 里**每一栋建筑**（`building_a`/`building_b`/`wall_c`/`wall_d`/`pillar_e`/`step_f1~f3`/`block_1~3`；4 面边界矮墙只做压顶）程序化生成三类**薄片**，合并成极少的 `MeshInstance3D`：

| 元素 | 几何 | 参数 |
|---|---|---|
| 窗 | 每面墙按 `bay = 2.4 m` 排布，窗 `1.8 × 1.6 m`；首层窗台 `0.9 m`，层高 `3.2 m`，层数 = `floor((h - 0.9) / 3.2)` | 略向墙外偏移 `0.01 m`；材质 = 深色玻璃（albedo ≈ 0.07、roughness 0.15、metallic 0.3） |
| 基座 | 每栋四壁 `z ∈ [0, 0.6]` 一圈 | 深一档的混凝土（albedo × 0.7） |
| 屋顶压顶 | 每栋顶面 `z = h`，向外扩 `0.05 m` | 深灰（albedo × 0.6），解决"俯视看到墙皮" |

- 全部薄片用 `SurfaceTool` **合并**：`Windows` 一个 mesh、`Plinths` 一个 mesh、`Caps` 一个 mesh（节点数 +3，不随建筑数增长）。
- 薄片 **`collision_layer = 0`、无 `CollisionShape3D`**；不得改变任何既有碰撞体。
- 窗/基座/压顶在 `_ready()` 里按 `OBSTACLES` 常量表生成（确定性、无 RNG），可幂等重建。

### 4 落点（文件 → 改什么）

| # | 文件 | 改什么 |
|---|---|---|
| 1 | `src/world/StreetLayout.gd`（**新增**） | 常量表（`ROADS` / `CURBS` / `MARKINGS` / `CROSSWALK`）+ `Build_All(root)`；`SurfaceTool` 合并；幂等（先摘同名旧容器） |
| 2 | `src/world/Facade.gd`（**新增**） | 按 `OBSTACLES` 生成窗/基座/压顶，`SurfaceTool` 合并成 3 个 mesh；常量 `BAY_M` / `FLOOR_M` / `WINDOW_*` |
| 3 | `src/world/World.gd` | `_ready()` 顺序：`BuildVoxelWorld()` → `WorldLook.Apply_All()` → `_Build_Bounds()` → `BuildPersons()` → `BuildPedestrians()` → `StreetLayout.Build_All(self)` → `Facade.Build_All(self)`；新增常量 `STREET_NODE` / `FACADE_NODE` 与 `Get_Street_Root()`（自检用） |
| 4 | `src/tests/check_street.gd`（**新增**） | 断言路网 AABB / 路缘段数与"零碰撞" / 标线条数 / 斑马线条数 / 立面 mesh 数 / 幂等（连调两次节点数不变）/ `OBSTACLES` 表未变 |
| 5 | `assets/textures/*` | 路缘/屋顶/沥青三套图（Step 1 已入库的话此处只做接线） |
| 6 | `COMPONENTS.md` | §2 节点树补 `Street`/`Facade` 容器、§4.x 新增"街道与立面"一节、§10 自检清单加 `check_street` |

### 5 不做

- 不做完整街区网格、不移动/新增**有碰撞**的障碍；
- 路缘**不加碰撞**（明确：`collision_layer = 0`、无 `CollisionShape3D`）；不模拟压马路牙子的车身姿态；
- 不做红绿灯、路牌、车辆、行道树、行人行道砖以外的点缀（这些留给后续任务）；
- 不做夜间发光窗、不做贴花（decal）系统；
- 不改 `OBSTACLES` / `PEDESTRIANS` / 世界尺度 / 碰撞层。

### 6 完成判据

1. **路网在位**：主路 `AABB = (120, 8)`、支路 `AABB = (8, 24)`，顶面 `z = 0.01 ± 0.005`（自检读 mesh AABB）。
2. **路缘零碰撞**：`Street` 下**没有任何 `CollisionObject3D` / `CollisionShape3D`**，且所有 `MeshInstance3D.collision_layer == 0`（自检遍历）。
3. **标线数量**：主路中心虚线 `20 ± 2` 段、边线 `2` 条、支路虚线 `4 ± 1` 段、斑马线 `4` 条；且**合并后** `MeshInstance3D` 总数 ≤ 3（路面 2 + 标线 1）。
4. **立面**：`Facade` 只有 3 个 `MeshInstance3D`（Windows / Plinths / Caps），且窗数量 > 0、每栋建筑四壁都有窗（自检按 `OBSTACLES` 逐栋断言）。
5. **零回归**：既有 17 个自检 `failed=0`；`check_street` `total=N failed=0`；尤其 `check_bounds`（空气墙）、`check_lidar`（热区）、`check_camera_shots`（停车点）、`check_pedestrian_shots`（取景）不许红。
6. **物理语义不变**：`OBSTACLES` 常量表、`PEDESTRIANS`、`[sim_world]` 尺度、四个 layer 的分配**零改动**（`git diff` 逐条断言）。
7. **出图**：Xvfb 下 3 张街景图（路口平视 / 建筑侧视 / UAV 俯视），斑马线与路缘清晰可辨、无 z-fighting（路面/地面交界、窗/墙面交界两处重点看）。
8. **幂等**：`StreetLayout.Build_All()` / `Facade.Build_All()` 连续调用两次，`Street`/`Facade` 子节点数与 mesh 数不变。

---

## 四、Step 3 — 在新世界里复验 task_37 的 `Follow`（直线跟随）

> 人类 2026-10-28 追加并两次纠偏：**只做验收，不改任何逻辑**。观感升级后，用街道场景重跑一遍跟随，
> 证明"相机 → detector → 地面投影 → 跟随叶子"没被贴图 / HDRI / 标线 / 立面破坏。
> **不通过就如实汇报，不自行修复。**

### 1 验证对象（跑的是哪一版 Follow）

跑 **task_37 Step 4 + Step 5 之后**的 `Mission::Follow`：

- 入口：控制口 `{"cmd":"follow"}` → `Command::Auto(Set(vec![Mission::Follow { 0, 0 }]))`，与 Pictor 的 `TASK_SET` 走同一条 `robot_cmd_tx`；
- 叶子：`uav_sim/mission/follow.rs` **不经 D\*** —— `d = |target - me|`；`d > d_far` → `next_cell = 沿「me → target」方向的一格`；`d <= d_near` → `None`；外层仍是 `ReactiveFallback + Hold`；
- 参数（task_37 Step 5）：`d_far = 4.0 m`、`d_near = 2.5 m`、`stale = 0.8 s`；
- **没有** `Mission::FollowStraight`、也没有控制口 `follow_straight`（task_37 已撤销，本步不得引用）。

> task_37 已 **Step 1~5 全 PASS**（`c639610` / `12f9786` / `b7985d7` / `d46a339`），e2e 实测见 `Workbook/wb_37_detector.md` 的「收尾与 e2e 实测」：
> 8 m 处估计 (73.92, 63.99) vs 真值 (74, 64)（~8 cm）；行人 70→83.55 m、UAV 66→78.05 m 跟随；**稳态距离 ≈5 m**（阈值 4.0 + 检测/推理 ~300 ms 延迟）。
> **本步是它的复跑，判据按这个实测基线校准**（见 §四.6）。

> 因此本步的距离带是 **[2.5, 4.0] m**（一格 0.5 m 量化），**不是** task_37 Step 4 早期的 1.5~3.0 m。

### 2 已知限制（照抄 task_37 §五.1，不算世界侧的锅）

近距离时 executor 追"下一格格心"、`Turning` 过冲又不反向修正，可能原地打转、目标出画。task_37 已把叶子改成"朝人走一格"绕开大部分，但 **executor / nav / D\* 本身仍不改**。
→ 若出现转向振荡，按"已知限制"记录进结论，**不修**；本步不碰 executor / nav / D\* / 协议。

> 2026-10-29 实测补充：机位在**格角**时，第一格目标（格心）相对机头偏 18.4°，executor 过冲到 **47.2°**，超出相机半视场 43° → 人出画 → `Hold`。
> 因此 §四.5 增加 **S0b 归位到格心**：这是验证夹具的初始条件，不是逻辑修改。

### 3 前置（缺一条就别开工，先把环境补齐）

| 项 | 值 | 为什么 |
|---|---|---|
| task_37 Step 4 + Step 5 | 已 PASS | 被验对象；`Mission::Follow` + 无 D* 叶子 |
| `[[sim_world.sensor]] camera` | `slot = "camera_nose"`、`rate_hz = 10.0` | 1 Hz 下 `stale = 0.8 s` **必然**判丢失，跟随恒 Hold |
| `[sim_world.sensor.mount] pitch` | `-15.0`（**度**；yaw 仍是弧度） | 2026-10-28 前 `world_link` 不透传 `pitch`（相机永远 0°，10 m 被算成 4.66 m），修好后必须显式写，否则投影整体偏 |
| `[detector]` | `enabled = true`、`device = "cpu"`、`model = "yolov8n.safetensors"`、`conf_thres = 0.35`、`nms_thres = 0.45` | 权重拷进 `<实例>/Pleiades_Workspace/` |
| `[detector.camera]` | `fov_deg = 70.0`、`fov_axis = "vertical"`、`mount_*`、`mount_pitch_deg = -15.0`、`ground_z = 0`、`max_range_m = 12` | 投影内参/外参；**必须与 `[[sim_world.sensor]]` 的相机参数一致**（world 侧 `fov_deg` 同一个 70） |
| 控制口 | `[control].port`（实例配置；历史 e2e 用 **9098**） | `follow` / `stop` / `status` 都走它 |
| `[chassis].forward_speed` | **100**（= 2.0 m/s × 100% = 2.0 m/s） | 默认 `30` → 0.6 m/s，**比行人 1.26 m/s 慢一倍**，几何上永远追不上；`70`（1.4 m/s）只比人快 0.14 m/s，从 8 m 收敛到 4 m 要 28 s，也不够。用满档 100；这是实例配置，不是逻辑 |
| UAV 出生 | 世界按 `UAV_ALTITUDE_M = 2.5` 放在 **(64, 64, 2.5)**、`yaw = 0` | 正对 +X，与主路同向；起飞后不需要转向 |
| 行人落点 | hello 里带 `spawn = {x: 70, y: 64, z: 0, yaw: 0}` | World 支持并校验（障碍/重叠/边界），落点 = UAV 正前方 6 m（沿用 wb_37 e2e 的起点 70） |
| 静态人 | 世界中 **没有** 装饰人（`d46a339` 已删 `PERSONS`） | 否则 detector 按置信度选目标会被静态人抢走（wb_37 实测） |
| 渲染 | `Xvfb :99` + `--rendering-driver opengl3` + `--audio-driver Dummy` | headless 出不了相机图 |
| `programs` 软链 | `<实例>/programs -> 仓库 programs/` | registry 扫 CWD，缺了 `exec` 全废 |

**速度预算为什么必须写死**：`Quadrotor` 的 `max_speed = 2.0 m/s`，动作档位 = `speed% × max_speed`；行人 `max_speed = 1.26 m/s`。

- `forward_speed = 30`（默认）→ 0.6 m/s：**比人慢**，距离单调拉大，10 s 后出画，判据 1 必挂；
- `70` → 1.4 m/s：只快 0.14 m/s，从 8 m 收敛到 4 m 要 **28 s**，观测窗口内看不到距离带；
- `100` → 2.0 m/s：闭合速度 0.74 m/s，**约 5.4 s 收敛**到 `d_far`，之后按 `d_far/d_near` 做占空比跟随。

所以取 **100**。这不是跟随逻辑的错，是几何；报告里要记实际档位与实测最大速度。

### 4 验证分工（**agent 自己验，不要 Pictor，不要人类**）

**为什么不需要 Pictor**：Pictor 只是地面站（逐车算点 + 下发 + 渲染）。`follow` 命令与控制口走的是同一条 `robot_cmd_tx`，与 `TASK_SET` 等价。所以"世界 → sim → 行人 → 控制口"可以完全闭环。

**agent 亲自做完这六件事**：

1. 起 Xvfb 与世界；
2. 起 `pleiades-sim`（uav 栈、控制口按实例 `[control].port`），等到 `rate_hz=10` 的相机就绪日志；
3. 起行人客户端（`orion-simulation-world/tool/pedestrian/`），带 `spawn=(72,64)`；
4. 逐条注入 §四.5 的 S0~S6；
5. 采 `trace.csv`、截图、逐条判 §四.6 的 10 条；
6. 给结论（PASS / FAIL + 卡在哪条 + 实测值）。

**主观项**（如"街景是否像街道"）用截图 + 像素统计给可复核证据，标注"主观项，附图 N"。

**产物目录**：`RobotSim_TestEnv/results/task38_follow_<UTC时间戳>/`，含 `trace.csv`、`shots/*.png`、`world.log`、`sim.log`、`ped.log`、`verdict.md`。

### 5 场景与动作序列（直线跟随）

**站位依据**：UAV 由世界放在 (64, 64, 2.5)、`yaw = 0` 正对 +X；主路 `y∈[60,68]` 沿 +X 有 **120 m 净空**。
人放在 UAV 前方 6 m 的 (70, 64)（wb_37 e2e 实测起点），沿 y=64 一直向东走——**人和机始终在同一条直线上，全程不需要转向**，正好是"直线跟随"。

```text
y=68 ────────────────────────────────────────────  北缘
      ○ UAV(64,64,2.5) yaw=0   →→→   ● 人(70,64)  →→→  (≈95,64)
y=60 ────────────────────────────────────────────  南缘
      x=64          x=70                x≈95
```

| # | 动作 | 触发 / 判据锚点 |
|---|---|---|
| S0 | 起 Xvfb + 世界 → 起 sim | 等 `[camera] camera_sim 使用 slot=... rate_hz=10`；断言 `pose=(64,64,~2.5)`、`yaw≈0` |
| S0b | **先归位到格心**（2026-10-29 加） | `{"cmd":"goto","x":64.25,"y":64.25}` → 等 `nav_status=Idle` → **按实测位姿算「首格方位」**：`Δψ = 机头到格心(64.75,64.25) 的方位角`，断言 `|Δψ| ≤ 10°`（不卡绝对位置：sim 到达判据本身允许 ~0.26 m 误差，卡位置会误判；卡的应是"首拍要不要转"）。若 `|Δψ| > 10°`，重发一次 goto；仍不过则记 `FAIL(env)` 并停 |
| S1 | 行人接入 | `spawn=(70,64,0,yaw 0)`；等 `pedestrian spawned ... spawn=(70.00, 64.00)` |
| S2 | 触发跟随 | `{"cmd":"follow"}`；等 `task_ticks` 递增（树已建） |
| S3 | 人沿 +X 走 **20 s**（1.26 m/s → ≈(95,64)） | **观察窗口**；距离带只看**收敛后的后 10 s**（前 ~5 s 是追赶） |
| S4 | 人停 5 s | 判据 3 |
| S5 | 关掉行人客户端（模拟丢失） | 判据 4 |
| S6 | `{"cmd":"stop"}`，再补 `mission_count=0` 的 TASK_SET | 判据 5 |

**为什么必须先归位（实测数据，2026-10-29）**：机位 (64.0, 64.0) 正好是 0.5 m 网格的**格角**，叶子算 `me + 0.5 m` 落到的格心是 (64.75, 64.25)，相对机头 **18.4°**；executor 追格心、`Turning` 又过冲不反向修正，实测转到 **47.2°**，超过相机半视场 **43.03°**（fov 70° 垂直 × 640×480 实算）→ 人出画 → dets 归零 → `stale 800 ms` → `Hold`，全程机身只挪 0.213 m。这是 task_37「executor 追格心 + 过冲」的已知限制，**本任务不改**；归位到格心后首拍目标就在机头正前方（方位 ≈0°），不需要转向，剩下的只是小幅过冲（<20°），人始终在视场内。

> 归位**只改验证的初始条件**（测试夹具），不改判据、不改世界、不改任何逻辑；判据 1~10 的阈值一条没动。

**采样**：控制口 `{"cmd":"status"}` 10 Hz 采 `pose / nav_status / task_id / task_ticks`；同时读 detector 日志的 `frame_ts / 估计 (x,y) / 真值 (x,y) / 拒答原因`，落 `trace.csv`（沿用 `Tool/sim_queue_test.py` 的写法）。

### 6 完成判据

| # | 判据 | 通过线 |
|---|---|---|
| 1 | 跟随距离带 | S3 **后 10 s** 内 **≥80%** 采样 `d ∈ [3.5, 6.0] m`，且 `min d ≥ 2.5 m`；同时记录均值（wb_37 实测 ≈5 m，若均值 >6 m 视为发散） |
| 2 | 不撞人 / 不穿人 | 全程 `min d ≥ 2.0 m`（红线，撞了直接 FAIL） |
| 3 | 人停 → 机停 | 5 s 内水平位移 ≤ **0.5 m**；速度符号翻转 ≤ 1 次 |
| 4 | 丢失 → 停住 | 行人断开 ≥ 0.8 s 后，10 s 内水平位移 ≤ **0.5 m**，不乱飞 |
| 5 | 取消 | `stop` 与 `mission_count=0` 两条路径均 ≤ **2 tick（100 ms）** 内 `nav_status=Idle` |
| 6 | **新世界·检测** | S3 后 10 s 内 **≥90%** 的帧有 `person`（conf ≥ 0.5），bbox 底边在画面下半部 |
| 7 | **新世界·误检** | S3 全程非 person 且 conf ≥ 0.5 的检测 ≤ **5%** 的帧（窗格 / 斑马线 / 路缘不得被认成人） |
| 8 | **新世界·投影** | 3~6 m 内误差 **< 1 m**（task_37 §四.6）；6~8 m 不出现离谱点 |
| 9 | 无碰撞回归 | LiDAR 里不出现路缘 / 标线；`check_lidar` / `check_bounds` / `check_camera_shots` 照旧全绿 |
| 10 | 证据 | `trace.csv` + 3 张图（人在前 / 人停 / 丢失停住）+ `verdict.md` + workbook 一节 |

**只记录、不判 FAIL 的观察项**：侧向偏移 `|y-64|`、yaw 振荡次数、detector 实际帧率、单帧推理耗时。——它们受已知限制影响，写进结论供后续任务参考。

### 7 结果处理（**只汇报，不改**）

- **全过** → PASS：写 workbook + 证据，收工。
- **不过** → **不改代码、不改配置重跑**：写下"哪条不过 / 实测值 / 根因初判（世界侧 / 相机 / detector / executor 已知限制）"，**停下向人类汇报**。
- **唯一例外：harness 故障**（世界没起、行人没连上、端口被占、Xvfb 崩）→ 可以修 harness 后重跑一次，并在报告里注明"重跑原因"，不得借此改判据。

### 8 不做

- 不改跟随逻辑（`follow.rs` / `Mission` / 协议）、不改 detector / 投影、不改 executor / nav / D\*；
- 不为让判据变绿去调 follow 参数、`conf_thres`、判据阈值、世界外观；
- 不做多目标 / ID 跟踪 / 真机跟随 / ugv 跟随；
- 不新增常驻进程（复验脚本跑完即退，世界与 sim 由脚本拉起并收掉）。

### 9 风险

| # | 风险 | 规避 |
|---|---|---|
| R11 | 相机 1 Hz → 跟随恒 Hold | §四.3 写死 `rate_hz=10`，S0 用日志确认；不达标先查这条 |
| R12 | `forward_speed` 档位不够 → 追不上人或收敛太慢 | §四.3 写死 100；报告里记实际档位与实测最大速度 |
| R13 | `mount.pitch` 没写 → 投影按 0° 算 | §四.3 写死 -15；S0 用首帧尺寸/投影日志核对 |
| R14 | 悬停高度偏离 2.5 m → 脚出画（h=3.0 时近端 2.52 m > `d_near` 2.5） | S0 断言 `pose.z ∈ [2.3, 2.7]`；超了就中止并记为 harness 故障 |
| R15 | 转向振荡（executor 已知限制） | 只在观察项里记；不作 FAIL 依据，不修 |
| R16 | 贴图/HDRI 让画面过亮过暗，检测掉点 | 属"新世界专项"要暴露的问题：先记录数字，**不改世界**，汇报 |
| R17 | 三进程时序错（世界未就绪就发命令） | 固定顺序 世界 → sim → 行人 → 控制口；每步有超时与失败退出 |
| R18 | 稳态距离 ≈5 m 被判成"距离不达标" | wb_37 实测就是 ~5 m（阈值 4.0 + ~300 ms 检测延迟）；判据 1 的带按 [3.5, 6.0] 校准，**不要**按 d_near=2.5 判 |
| R19 | 控制口端口写错（9098/9099） | 以实例 `[control].port` 为准，脚本从 config 读，不硬编码 |

## 五、风险与规避

> Step 1/2 的风险见下表（R1~R10）；Step 3 的风险（R11~R17）见 §四.9。

| # | 风险 | 规避 |
|---|---|---|
| R1 | triplanar 在 `GridMap`（MultiMesh 式实例化）上是否按世界坐标采样 | Step 1 先用 `StandardMaterial3D` 的 `uv1_world_triplanar`（引擎内建，久经验证）；出图目视确认"墙缝连续"。若异常，退路是给 `GridMap` 换 per-cell 世界 UV（不写 shader） |
| R2 | HDRI 与 `DirectionalLight` 双太阳 | 选**晴天正午** HDRI，把 `DirectionalLight` 的方位/仰角对齐到 HDRI 太阳；光强下调到 ≈0.7，HDRI 只提供天光与背景 |
| R3 | Compatibility（opengl3）下 SSAO/SSIL 不生效 | 判据里**不出现** SSAO；接触阴影靠 ORM 的 AO + 阴影贴图；若在 Forward+ 下额外开 SSAO，不得作为验收项 |
| R4 | 像素判据漂移（`WHITE_MAX` / `MEAN_MAX` / `GRID_*`） | 先测基线再定；贴图整体压暗（地面 albedo 平均 ≈0.35~0.5），近白像素占比不升；阈值若必须改，按 §二.5-6 记录实测值 |
| R5 | 贴图 `.import` 漏提交 / 忘记 `--import` | 提交前 `git status` 检查每个 `*.jpg` 旁有 `*.import`；验收流程第一步固定 `--import` |
| R6 | 路缘误加碰撞 → 影响 LiDAR/急停/寻路 | 判据 2 直接遍历断言"无 CollisionObject3D"；代码里不 import `CollisionShape3D` 相关 |
| R7 | 路面与地面共面 z-fighting | 路面顶 `z = 0.01`、标线 `z = 0.012`；相机 `near` 保持现状；出图检查远处条纹 |
| R8 | 薄片（窗/基座/压顶）与体素面共面 | 一律外扩 `0.01 m`；禁用 `depth_draw_never` 之类取巧手段 |
| R9 | 1K/2K 贴图在 Compatibility 下的压缩格式 | 导入用默认 VRAM Compressed + mipmap；先小样验证再批量导入 |
| R10 | 新增自检改变"既有自检数"基线 | `COMPONENTS.md` 的自检清单与数字同步更新（17 → 19），并在 workbook 记录新旧数字 |

---

## 六、范围之外

- 完整街区路网、红绿灯、交通标志、行道树、路灯、车辆与杂物点缀；
- 夜间/黄昏光照、发光窗、体积雾、雨雪；
- 贴花（decal）系统、`ShaderMaterial` 程序化窗（本轮刻意不用 shader）；
- 任何 Rust 侧（`pleiades-*`）改动、任何协议改动；**task_37 的 follow / detector / 投影代码也不改**（Step 3 只复验；有问题回 task_37）；
- 模型（车/人/传感器）的材质升级。

---

## 七、开工前待确认

1. **HDRI 时段/天气**：默认按"晴天正午"选（与现有 `DirectionalLight` -55°/-35° 最接近）；若要黄昏/阴天，像素判据与光强整组要重测。
2. **立面风格**：默认"现代办公楼"（横向窗带 + 深色玻璃 + 混凝土）；若要砖墙/住宅风，换贴图即可，几何不变。
3. **新增自检命名**：`check_world_look.gd` / `check_street.gd`（若人类希望合并成一个 `check_look.gd`，开工前说）。
4. **缺口径**：`assets/` 增量上限按 **10 MB** 写；若人类希望更小，贴图降到 1K（观感损失在远景才可见）。
5. **Step 3 的行人落点**：默认 `spawn=(72,64)`、沿主路 y=64 向东走 20 s（≈25 m）（UAV 出生在 (64,64) 正对 +X，全程直线不转向）。若人类希望换成支路或别的位置，开工前说——换路线会牵动 `forward_speed` 与相机可见距离的核算。
6. **Step 3 的判据 5/6 阈值**（命中率 ≥90%、误检 ≤5%）是按"10 Hz 相机 + 3~8 m"估的；若实测相机帧率或距离不同，开工前可再定，但**只允许调这两条新判据**，不许动 task_37 的 1~4 条。

---

## 八、记录

- 2026-10-28：目标确定——世界观感精修，城市街道（沥青 + 路缘 + 标线）+ HDRI 天空 + 建筑立面感。
- 2026-10-28：决策——路缘**纯视觉不碰撞**；建筑**只加立面薄片**、不改体素体量与 `OBSTACLES`；街道布局由 agent 定，采用"主路 + 支路 T 型 + 斑马线"，只在空白区铺、与既有障碍冲突时改路段端点。
- 2026-10-28：拆两步——Step 1 材质管线与天空（不碰几何），Step 2 街道几何与建筑立面；`World.tscn` 全程零改动，材质/几何一律运行时建。
- 2026-10-28：材质来源定 CC0 四站（Poly Haven / ambientCG / 3DTextures.me / cgbookcase）；通道约定 `nor_gl` + `arm→ORM`；尺寸 1K~2K、总增量 ≤ 10 MB。
- 2026-10-28：明确**不用 shader**——立面窗/基座/压顶全部用薄片几何 + `StandardMaterial3D` 合并 mesh，避免自写 triplanar/normal 的维护成本。
- 2026-10-28：明确 SSAO 不列判据（自检跑 Compatibility 渲染器，SSAO 不生效）。
- 2026-10-28：**追加 Step 3**（人类要求）——在精修后的街道场景里复验 task_37 的 Follow 端到端；判据 1~4 与 task_37 §五.6 同口径不放宽，新增"新世界专项"3 条（检测命中率 / 误检率 / 投影误差）。
- 2026-10-28：Step 3 定位 = **只验证 + 最小修正**；不允许改 follow 参数、不允许降 `conf_thres` 凑命中率；世界侧允许调曝光/贴图亮度。前置写死 `rate_hz=10`（1 Hz 下 `stale=0.8s` 必然失效）。
- 2026-10-28：~~Step 3 的站位复用 Step 2 的支路（`pedestrian_01` (44,96)）~~ —— **已废弃，被下一条改为“主路东行直线”**（同一结论：不动 `PEDESTRIANS` 表，行人用 hello 的 `spawn` 落点）。
- 2026-10-28：**明确验证分工**（人类要求）——三步全部由 **agent 自己跑、自己出证据**，不需要人类现场操作，**也不需要 Pictor**：Follow 走 sim 控制口 `{"cmd":"follow"}`，内部构造 `AutoCmd::Set`，与 Pictor 的 `TASK_SET` 是同一条 `robot_cmd_tx` 路径。禁止把端到端留给人类，禁止为过判据改阈值/参数。
- 2026-10-28：**下载通道**记入 §二.2——直连失败走本机 mihomo（`127.0.0.1:7897`；`9090` 是控制口），代理只用于下载，不写进仓库默认值，实际通道记 workbook。
- 2026-10-28：**随 task_37 两次纠偏同步 Step 3**——① 撤销 `Mission::FollowStraight` / 控制口 `follow_straight`（人类确认那不是需求），复验回到 `{"cmd":"follow"}`；② task_37 Step 5 把 `Follow` 叶子改成**不经 D\***（`d > d_far` 时朝 `me → target` 方向给一格，`d <= d_near` 停），参数 `d_far=4.0 / d_near=2.5 / stale=0.8`，判据距离带因此改为 **[2.5, 4.0]**（不再是 1.5~3.0）。
- 2026-10-28：Step 3 站位改为**主路东行直线**——UAV 由世界放在 `(64,64,2.5)`、`yaw=0`；行人 hello 带 `spawn=(72,64)`，8 m 正前方，沿 y=64 向东走 20 s（≈25 m），全程不转向。
- 2026-10-28：Step 3 补两条**环境硬账**——`forward_speed` 必须 100（默认 30 = 0.6 m/s 追不上 1.26 m/s 的行人；70 = 1.4 m/s 只快 0.14 m/s，收敛要 28 s），`mount.pitch` 必须 -15（`world_link` 2026-10-28 才修好透传，不写就当 0°）。
- 2026-10-28：Step 3 的观测窗口改为**人走 20 s、只看收敛后的后 10 s**（前 ~5 s 是追赶段，距离必然 > 4 m）。
- 2026-10-28：Step 3 定位再次收窄为**纯验收**：失败就汇报，不改代码/配置重跑；唯一例外是 harness 自身故障（世界没起、行人没连上）。
- 2026-10-28：世界侧已变（commit `d46a339` 移除静态装饰人，`PERSONS` 不复存在；自检现为 17 个），本文件的行号与自检清单按最新代码订正。
- 2026-10-28：**按 wb_37 的 e2e 实测校准本步**——task_37 Step 1~5 全 PASS（`c639610`/`12f9786`/`b7985d7`/`d46a339`）；参考实测：8 m 处投影误差 ~8 cm、行人 70→83.55 / UAV 66→78.05 跟随、**稳态距离 ≈5 m**（阈值 4 + ~300 ms 延迟）。判据 1 的带因此改为 [3.5, 6.0]；行人起点改为 (70,64)。
- 2026-10-28：执行方式定稿——**用执行引擎串行跑 3 个 step**（参考 `Tool/execution_engine_task36.ts` 的流程，不看 task_37 的引擎），引擎 `Tool/execution_engine_task38.ts` + `.config.json`，中间产物 `tmp/task38/`，记录 `Workbook/wb_38_sim_world_refine.md`。
- 2026-10-29：**首轮引擎卡住的原因已查明**——不是 agent 或命令的问题，是当时用的 LLM provider 服务不稳定，子 agent 的调用挂住不返回（`dispatchsubagent` 因此一直等）。已 `cancel_program`，仓库**零改动**。人类已换成稳定的 provider。
- 2026-10-29：**明确不加任何硬性时间预算**（不设 plan/impl/review 的分钟数、不给命令套 `timeout`、不设"卡住就放弃"）——卡住是 provider 的问题，不该用预算去兜。Step 1 仍拆成 **1a 资产 / 1b 接线**，但那只是范围划分（资产与代码分开验收），不是超时对策。
- 2026-10-29：三步执行完毕——**1a/1b/2 全 PASS**；**Step 3 FAIL**（3 轮未过）。失败链：机位在格角 → 首格格心偏 18.43° → executor 过冲到 47.2° → 越过 43° 半视场 → 人出画 → Hold。S0b 归位因 `ARRIVAL_TOL_M=0.26` > goto 距离 0.25 m 而成为 no-op，且真移动会把机头转到 ~45°，两条路都过不了 10° 门槛（详见 workbook「收尾结论」）。世界侧被证实无问题：投影 3~6 m 误差中位 0.073~0.112 m、person conf 0.90、新贴图 0 误检。未改任何 Rust/阈值/外观。后续随 task_39 复跑。
