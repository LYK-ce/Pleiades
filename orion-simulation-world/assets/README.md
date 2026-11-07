# assets/ — 真实资源目录

> Presented by KeJi
> Created Date ： 2026-10-22
> Modified Date ： 2026-11-07

本目录**只放真正的资源文件**：位图/纹理（`.png` `.jpg` `.webp`）、
3D 模型（`.glb` `.gltf` `.obj` `.fbx`）、材质与环境（`.material` `.tres` `.env`）、
音频（`.wav` `.ogg`）。

**不放**：`.gd` 脚本、`.tscn` 场景、自检脚本、文档 —— **代码与场景一律在 `src/`**
（权威布局见 `COMPONENTS.md` §1.1）。

## 1. 目录内容

| 路径 | 是什么 |
|---|---|
| `.gitkeep` | 占位（保留） |
| `humans/` | 人形资产，见 `humans/README.md`（Renderpeople 免费页，非 CC0） |
| `chassis/patrol_car/` | **task_50 Step 1**：巡逻车外观（Kenney `police`，CC0）×4 文件，**只入库未接线**，见 §7 |
| `props/cars/` | **task_50 Step 1**：社会车辆 2 款（`kenney_sedan` / `kenney_suv`，CC0），按口径 #7 归一（朝 +X、轮底 y=0、单文件 glb），见 `props/cars/README.md` |
| `props/buildings/` | **task_50 Step 1**：背景楼 4 栋（Kenney City Kit Commercial，CC0），**零碰撞只作背景**，见 `props/buildings/README.md` |
| `sensors/` `chassis/` `ugv_x3plus_lowpoly.glb` | 车辆/传感器外观模型（程序化装配用）；`chassis/uav_quadrotor.glb` = **task_40 Step 5 换的四旋翼外观**，见 §5 与 `ATTRIBUTION.md`；`chassis/ugv_m1.glb` = **task_42 Step 3 换的 UGV 外观（自有资产）**，见 §6 |
| `textures/` | **task_38 Step 1a**：4 套 PBR（12 文件）+ **task_49 Step 3**：室内 2 套（6 文件，`wall_indoor_*` / `floor_indoor_*`），共 6 套 18 文件，全部 CC0，见 §2 |
| `hdri/` | **task_38 Step 1a 新增**：1 张 2K 晴天正午 HDRI，CC0，见 §3 |

> 除 `humans/`（Renderpeople 免费页条款）与 `chassis/uav_quadrotor.glb`（**CC BY 3.0**，须署名，见 `ATTRIBUTION.md`）外，`textures/` 与 `hdri/` 全部为 **CC0 1.0**（Poly Haven / ambientCG 的公共领域授权），可商用、可修改、无归属义务；本仓库仍逐条记账来源以便复核。
> `chassis/ugv_m1.glb` 是本仓库 **Blender 自建的自有资产**（无第三方素材/贴图），不适用 CC0 / CC-BY，也不需要署名。
> **贴图与 `.import` 必须一起提交**：干净检出先跑 `godot --headless --path orion-simulation-world --import`，否则 `load()` 返回 `null`（`COMPONENTS.md` §9.1 **E28**）。

## 2. `textures/` — 6 套 PBR（CC0；task_38 Step 1a 4 套 + task_49 Step 3 室内 2 套）

通道口径（写死）：`*_color.jpg` → `albedo_texture`；`*_nor.jpg` → `normal_texture`（**OpenGL 约定，`nor_gl`，禁止 `nor_dx`**）；`*_orm.png` → `orm_texture`（**R=AO / G=Roughness / B=Metallic**）。

| 文件 | 用途 | 来源（资产 ID + 通道） | 来源 URL | 许可 | 落地尺寸 | 处理（通道拼法 / 编码） | 字节 | sha256[:16] |
|---|---|---|---|---|---|---|---|---|
| `ground_paving_color.jpg` | 地面 albedo | ambientCG `PavingStones136` `Color` | https://ambientcg.com/get?file=PavingStones136_2K-JPG.zip | CC0 | 2048² | 2K 源直出，JPG q85 | 926 791 | 1dd37eb31da7c786 |
| `ground_paving_nor.jpg` | 地面 normal | 同上 `NormalGL` | 同上 | CC0 | 2048² | 2K 源直出，JPG q88 / 4:4:4 | 975 127 | 12e461cc552f7dbf |
| `ground_paving_orm.png` | 地面 ORM | 同上 `AmbientOcclusion`+`Roughness`（1K 包） | https://ambientcg.com/get?file=PavingStones136_1K-JPG.zip | CC0 | 512² | **离线拼**：R←AO、G←Roughness、B←0（该资产无 Metalness 贴图，非金属）；PNG 无损 | 323 079 | 69c820c0c2fbce11 |
| `concrete_color.jpg` | 体素墙 albedo | Poly Haven `concrete_wall_008` `diff` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/concrete_wall_008/concrete_wall_008_diff_1k.jpg | CC0 | 1024² | 1K 源直出，JPG q85 | 145 840 | f40bdb87067d7d14 |
| `concrete_nor.jpg` | 体素墙 normal | 同上 `nor_gl` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/concrete_wall_008/concrete_wall_008_nor_gl_1k.jpg | CC0 | 1024² | JPG q88 / 4:4:4 | 95 278 | 457256c729becbfd |
| `concrete_orm.png` | 体素墙 ORM | 同上 `arm` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/concrete_wall_008/concrete_wall_008_arm_1k.jpg | CC0 | 512² | **`arm` 即 ORM，原样搬**（R=AO/G=Rough/B=Metal）；PNG 无损 | 259 388 | 804971b554ea2244 |
| `asphalt_color.jpg` | 沥青 albedo | Poly Haven `asphalt_02` `diff` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/asphalt_02/asphalt_02_diff_1k.jpg | CC0 | 1024² | 1K 源直出，JPG q85 | 414 415 | 67e7cac202b50a17 |
| `asphalt_nor.jpg` | 沥青 normal | 同上 `nor_gl` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/asphalt_02/asphalt_02_nor_gl_1k.jpg | CC0 | 1024² | JPG q88 / 4:4:4 | 641 909 | 73a379a7b70a2811 |
| `asphalt_orm.png` | 沥青 ORM | 同上 `arm` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/asphalt_02/asphalt_02_arm_1k.jpg | CC0 | 512² | `arm` 原样搬；PNG 无损 | 499 433 | e986cca18bcb6db6 |
| `curb_color.jpg` | 路缘 albedo | Poly Haven `concrete_wall_005` `diff` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/concrete_wall_005/concrete_wall_005_diff_1k.jpg | CC0 | 1024² | 1K 源直出，JPG q85 | 101 125 | 6b8f6aaf8291d4cd |
| `curb_nor.jpg` | 路缘 normal | 同上 `nor_gl` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/concrete_wall_005/concrete_wall_005_nor_gl_1k.jpg | CC0 | 1024² | JPG q88 / 4:4:4 | 265 063 | 3fcce21b9da4a3ff |
| `curb_orm.png` | 路缘 ORM | 同上 `arm` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/concrete_wall_005/concrete_wall_005_arm_1k.jpg | CC0 | 512² | `arm` 原样搬；PNG 无损 | 234 303 | b7cdbc937deed462 |
| `wall_indoor_color.jpg` | 室内墙面 albedo | Poly Haven `white_plaster_02` `diff` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/white_plaster_02/white_plaster_02_diff_1k.jpg | CC0 | 1024² | 1K 源**字节级搬运**（未重编码） | 527 219 | a1ebbe091bd1ae93 |
| `wall_indoor_nor.jpg` | 室内墙面 normal | 同上 `nor_gl` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/white_plaster_02/white_plaster_02_nor_gl_1k.jpg | CC0 | 1024² | 同上，`nor_gl`（**未取 `nor_dx`**） | 652 310 | eb572ca3630d5bfd |
| `wall_indoor_orm.png` | 室内墙面 ORM | 同上 `arm` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/white_plaster_02/white_plaster_02_arm_1k.jpg | CC0 | 1024² | **`arm`→PNG 无损**（R=AO/G=Rough/B=Metal），源 191 017 B | 1 485 831 | 88a6b4c9eac89dc3 |
| `floor_indoor_color.jpg` | 室内地面 albedo | Poly Haven `terrazzo_tiles` `diff` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/terrazzo_tiles/terrazzo_tiles_diff_1k.jpg | CC0 | 1024² | 1K 源**字节级搬运**（未重编码） | 1 166 107 | 462e3e719cda0472 |
| `floor_indoor_nor.jpg` | 室内地面 normal | 同上 `nor_gl` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/terrazzo_tiles/terrazzo_tiles_nor_gl_1k.jpg | CC0 | 1024² | 同上，`nor_gl`（**未取 `nor_dx`**） | 77 954 | 56bbe2d960b47ddb |
| `floor_indoor_orm.png` | 室内地面 ORM | 同上 `arm` | https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/terrazzo_tiles/terrazzo_tiles_arm_1k.jpg | CC0 | 1024² | **`arm`→PNG 无损**（R=AO/G=Rough/B=Metal），源 477 964 B | 1 080 444 | 2e660006dc70648e |

处理说明：
- **降采样**：只有 ORM 降到 512²（task_38 Step 1a 那 4 套；task_49 Step 3 的 2 张 ORM 未超 1.5 MB，故保持 1024²。AO/粗糙度是低频数据；`*_nor/_color` 不动分辨率）；albedo/normal 与源同尺寸，**没有做 2K→1K 降采样**。
- **重编码**：全部用 `PIL` 重编码以压进 10 MB 预算（源 1K jpg 合计 6.7 MB、2K 源合计 30 MB+）；normal 一律 4:4:4、ORM 一律 PNG 无损。
- **不重采样色彩空间**、不做色彩校正；`nor_dx` / `NormalDX` 全程未下载、未落地。
- `.import` 由 `godot --headless --path orion-simulation-world --import` 生成（`compress/mode=0` 无损 + `mipmaps/generate=true`）；**不要手改**。
- **task_49 Step 3 两套室内贴图**（`wall_indoor_*` / `floor_indoor_*`）：`_color` 与 `_nor` 为 1K 源**字节级搬运**
  （未重编码、未降采样，`sha256` 与源一致，可直接复核）；`_orm` 由 Poly Haven `arm` 1K JPG **无损转 PNG**
  （`arm` 的 R/G/B 本身就是 AO/Roughness/Metallic，**零像素改动**）；两张 PNG 均未超 1.5 MB，故都保持源尺寸 1024²
  （既有 4 套 ORM 当年是 512²，是本批之后的唯一尺寸差异）。两套均**未取 `nor_dx` / `NormalDX`**。

## 3. `hdri/` — 1 张 2K 晴天正午 HDRI（CC0）

| 文件 | 用途 | 来源 | 来源 URL | 许可 | 尺寸 | 处理 | 字节 | sha256[:16] |
|---|---|---|---|---|---|---|---|---|
| `sky_midday_2k.hdr` | 天空（`PanoramaSkyMaterial`） | Poly Haven HDRI `kloofendal_43d_clear_puresky` | https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/2k/kloofendal_43d_clear_puresky_2k.hdr | CC0 | 2048×1024（Radiance RLE, 32-bit_rle_rgbe） | 字节级搬运，未重编码/未降采样 | 4 624 289 | 462a3cc63bfcd9fe |

- 选它：Poly Haven 分类同时带 `clear`（晴）+ `midday`（正午）+ `pure skies`（纯天空、无地平线建筑，不会和街道几何穿帮）。
- Godot 侧挂法（Step 1b）：`PanoramaSkyMaterial.panorama = load("res://assets/hdri/sky_midday_2k.hdr")`；`background_mode=2` / `ambient_light_source=3` 保持不变。

## 4. 体积记账

| 项 | 字节 |
|---|---|
| 本步新增 12 贴图 + 1 HDRI | 9 506 040 |
| 本步新增 13 个 `.import` | 12 431 |
| **task_40 Step 5 增量**（新 glb 299 376 + 2 个 `.import` 2 231 + 抽出贴图 2 882 + 许可 19 467 + 署名 5 289 − 删旧件 20 460 − 删旧 `.import` 1 185 = 资产净 **+307 600**；本文件台账 +3 001） | **+310 601** |
| **task_42 Step 3 增量**（新 glb 67 752 + `.glb.import` 1 148 − 删旧 `ugv_chassis.glb` 21 888 − 旧 `.import` 1 163 = **+45 849**；本文件台账自身增长另计） | **+45 849** |
| **task_49 Step 3 增量**（室内 2 套 6 张贴图 4 989 865 + 6 个 `.import` 5 808 = **+4 995 673**；2K 源未取、`nor_dx` 未下载） | **+4 995 673** |
| `assets/` 总体积（`du -sb`，2026-11-06 实测 ≈ **28.29 MB**；本文件自身字节也算在内，故精确值随文档微调而变） | ≈ 29 663 711 |

判据：贴图单张 ≤ 1.5 MB（最大 `wall_indoor_orm.png` 1 485 831）、HDRI ≤ 6 MB、`assets/` 增量 **9 524 411** ≤ 10 MB —— 全部通过。
task_40 Step 5 另核：模型 glb **299 376 B ≤ 1 MB**、贴图 128×128 ≤ 2K、tris **4 564 ≤ 50k**。

## 5. `chassis/` — 四旋翼外观 1 件（**CC BY 3.0**，task_40 Step 5）

| 文件 | 是什么 | 来源 | 来源 URL | 许可 | 处理 | 字节 | sha256[:16] |
|---|---|---|---|---|---|---|---|
| `chassis/uav_quadrotor.glb` | `Vehicle/ModelUAV` 的四旋翼外观（`ApplyChassis("quadrotor_uav")` 时可见） | Poly Pizza `Drone` / **NateGazzard**（上传 2023-12-31，站内 `Tris: 4564`） | https://poly.pizza/m/DNbUoMtG3H（直下 https://static.poly.pizza/4eb88feb-cb2d-46c3-980d-3e704868361c.glb） | **CC BY 3.0** | Blender 归一：朝 +X / 底面 `y=0` / 非等比缩放到 AABB `0.64 × 0.14 × 0.64` / `Body` 正面面片重涂 `UAV_NoseRed` / 导出 GLB Y-up（详见 `ATTRIBUTION.md` §3） | 299 376 | 7aa77d6b21213819 |
| `chassis/uav_quadrotor.glb.import` | 场景导入描述（`importer="scene"`） | — | — | — | `godot --headless --path orion-simulation-world --import` 生成，**不要手改** | 1 169 | — |
| `chassis/uav_quadrotor_ColorPalette.png` | **Godot 从 glb 里抽出的调色板贴图**（128×128，同 `humans/*_dif.jpg` 的先例） | 同上 | 同上 | CC BY 3.0 | 导入时自动落地（`gltf/embedded_image_handling=1`），像素未改 | 2 882 | — |
| `chassis/uav_quadrotor_ColorPalette.png.import` | 贴图导入描述 | — | — | — | 同上，**必须与 png 一起提交**（`COMPONENTS.md` §9.1 **E28**） | 1 062 | — |
| `chassis/LICENSE_uav_quadrotor.txt` | CC BY 3.0 Unported **许可全文**（与 `humans/LICENSE_renderpeople.txt` 同一命名习惯） | <https://creativecommons.org/licenses/by/3.0/legalcode.txt> | 同左 | — | 字节级搬运，未改一字 | 19 467 | — |

- **抓取日期**：2026-10-29；源文件 `259 676 B` / `sha256 = bc9f0d765e0fdb838a9baf26b472f955a11bb2509a3337e918054241b134638d`（下载脚本见 `tmp/task40/`，直连成功、未走代理）。
- **署名**：`"Drone" by NateGazzard — https://poly.pizza/m/DNbUoMtG3H — CC BY 3.0`（完整署名 + 改动说明 + 选型证据见 `ATTRIBUTION.md`）。
- **碰撞盒未随外观变更**：模型 AABB `0.64 × 0.14 × 0.64` 与 §6.5 的目标 `0.64 × 0.13 × 0.64` 只差 **1.0 cm**（y 轴，且 `0.11 ≤ y ≤ 0.15` 与 `check_sensor_models` 的 `0.14 ±2%` 都要求 y 取 0.14）⇒ **`UAV_HULL` 仍是 `(0.40, 0.12, 0.40)`，碰撞盒与动力学一行未改**。
- **贴图与 `.import` 必须一起提交**；旧件 `assets/uav_quadrotor_lowpoly.glb`（20 460 B）+ 其 `.import` 已 `git rm`（git 可回溯）。

## 6. `chassis/ugv_m1.glb` — UGV 外观（**自有资产**，task_42 Step 3）

| 文件 | 是什么 | 来源 | 许可 | 处理 | 规格 | 字节 | sha256 |
|---|---|---|---|---|---|---|---|
| `chassis/ugv_m1.glb` | `Vehicle/ModelUGV` 的 UGV 外观（`ApplyChassis("diff_drive_ugv")` 时可见） | **本仓库自建**：`Tool/blender/models/build_models.py` 的 `Build_Ugv_M1()`（旧 `ugv` 键一行未改，可逐字复现旧件）+ `Tool/blender/models/export_all.sh` ⑤ 段 | **自有资产**（Blender 参数化建模，无第三方素材/贴图，不适用 CC0/CC-BY，无需署名；`.blend` 源只落 `tmp/task42/step2/ugv_m1.blend`，按 §二.5 不入库） | Blender 4.5.14 建（方盒下车体 + 斜引擎盖 + 平顶 + 顶置激光雷达 + 浅灰前唇 `NosePlate` + 蓝 LED + 4 支麦克纳姆视觉轮）；导出 GLB（Y-up、无 Draco、0 贴图、4 材质） | 15 网格 / **1 792 tri** ≤ 12 000 / AABB **`0.3000 × 0.2500 × 0.2500`** / 原点 = 底面中心、车头 **+X** / 4 材质 0 贴图 | **67 752** | `2067d34f277da2c2d3a52f14d520f6880d652489eff317f51614cde1e2169545` |
| `chassis/ugv_m1.glb.import` | 场景导入描述（`importer="scene"`，`uid="uid://dwy17ous1dlyq"`） | — | — | `godot --headless --path orion-simulation-world --import` 生成，**不要手改** | — | 1 148 | — |

- **碰撞盒未随外观变更**：AABB 与 `0.30 × 0.25 × 0.25` 逐轴相等 ⇒ `DiffDrive` 常量 / `UGV_HULL` / `CollisionShape3D` / `check_models` 的 M5/M6 **一行未改**（物理零改动）。
- **旧件已删**：`chassis/ugv_chassis.glb`（21 888 B）+ `ugv_chassis.glb.import`（1 163 B）在本步验收全绿后 `git rm`（git 可回溯）。旧 `ugv` 构建键与 `Tool/blender/models/ugv_chassis.blend` **保留**：不带 `ORION_BUILD_LEGACY=0` 重跑 `export_all.sh` 会重新生成该旧 glb，属**预期**（可复现性），需要时 `git status` 会看到它重新变成未跟踪文件。
- **贴图与 `.import` 必须一起提交**（老规矩：干净检出先 `--import`，否则 `load()` 返回 `null`）。

---

## 7. `chassis/patrol_car/` — 巡逻车外观（**CC0**，task_50 Step 1）

> 用途：**替换现在 0.3 m 宽的玩具比例自建低模**（8 台车墙 / 4 台围控车的外观）。
> **本步只入库，不接线**：`ChassisCatalog.DEFAULTS`（`track_width` / `height` 是碰撞尺寸）、`Vehicle.tscn` 的 `ApplyChassis`、
> 任何 `scale` **一行未改**（口径 #3：视觉与碰撞同步放大是 Step 2 的事）。

| 文件 | 是什么 | 来源 | 许可 | 处理 | 规格 | 字节 | sha256 |
|---|---|---|---|---|---|---|---|
| `chassis/patrol_car/model.glb` | 巡逻车外观（白/黑车身 + 车顶警灯 + 格栅，**四轮**） | `https://kenney.nl/assets/car-kit` → 包内 `Models/GLB format/police.glb`（`195 336 B`） | **CC0**（包内 `License.txt`：`Car Kit (3.1)` / `License: (Creative Commons Zero, CC0)`） | Kenney 出厂贴图是外部相对路径 `Textures/colormap.png` ⇒ 用 `GLTFDocument.generate_buffer()` 重导出成**单文件 glb**（图集 `colormap2` 内嵌 512² PNG / 7 825 B）；同时 **`R_y(+90°)`** 把车头从 `+Z` 转到 **`+X`**、`min.y → 0`、水平居中。旋转/平移表达在**场景根节点变换**上（顶点未动） | AABB **`3.1000 × 1.3000 × 1.5000`**（X 长 × Y 高 × Z 宽）/ 原车头 `+Z` → 归一后 **`+X`** / **2 304 tri** / 6 网格 / 1 材质 / 0 骨骼动画（`clips=[]`、`skins=0`）/ 轮底 `y = 0.000` | **140 585** | `0df154cc7af055fe…` |
| `chassis/patrol_car/model.glb.import` | 场景导入描述（`importer="scene"`） | — | — | `godot --headless --path orion-simulation-world --import` 生成，**不要手改** | — | 1 168 | — |
| `chassis/patrol_car/model_colormap2.png` | **Godot 抽出的 basecolor 图集**（512×512，同 `humans/*_dif.jpg` / `chassis/uav_quadrotor_ColorPalette.png` 的先例） | 同上 | CC0 | 导入时自动落地（`gltf/embedded_image_handling=1` Extract），像素未改 | 512² PNG | 7 825 | — |
| `chassis/patrol_car/model_colormap2.png.import` | 贴图导入描述 | — | — | 同上，**必须与 png 一起提交**（`COMPONENTS.md` §9.1 **E28**） | — | — | — |

- **抓取日期**：2026-11-07；包 `kenney_car-kit.zip` = `4 814 237 B`（许可证全文在包内 `License.txt`，本仓库不另存）。
- **AABB 与地面高度**：`3.1000 × 1.3000 × 1.5000`，轮底 `y = 0.000`。目标尺寸表（§2.0 目标尺寸表）里 **UGV 是"长 0.8 m / 宽 ~0.55 / 高 ~0.5"**，本车 **3.1 m 长** ⇒ **本步不缩放**（口径 #2 / R12：视觉与碰撞一起改是 Step 2 的事），台账只登记原尺寸。
- **车辆口径复核**（虽然是 UGV 外观，同样按 #7 检查）：米制、朝 **`+X`**、轮底 **`y = 0`**、单文件 glb、**无骨骼动画依赖**。
- **零碰撞**：资产里没有任何 `CollisionObject3D` / `CollisionShape3D`（回读 `collisions=0/0`）；本步不改 `src/` 一行。
- **找不到的退路没用上**：Kenney 第 1 站即命中合规件（四轮 + 警用观感 + CC0），因此**不需要**把 `chassis/ugv_m1.glb` 非等比放大到 0.8 m（口径 #4 的退路保留未用；`ugv_m1.glb` 原文件未动）。
- **风格**：卡通低模（R9）⇒ 摆位时放中远景、别给近景特写；"封路能看懂"优先于写实。
- 条款：Kenney CC0，可商用、无需署名（本仓库仍在此记账）。
