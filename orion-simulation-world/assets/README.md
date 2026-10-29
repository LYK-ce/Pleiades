# assets/ — 真实资源目录

> Presented by KeJi
> Created Date ： 2026-10-22
> Modified Date ： 2026-10-29

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
| `sensors/` `chassis/` `uav_quadrotor_lowpoly.glb` `ugv_x3plus_lowpoly.glb` | 车辆/传感器外观模型（程序化装配用） |
| `textures/` | **task_38 Step 1a 新增**：4 套 PBR 贴图（12 个文件），CC0，见 §2 |
| `hdri/` | **task_38 Step 1a 新增**：1 张 2K 晴天正午 HDRI，CC0，见 §3 |

> 除 `humans/`（Renderpeople 免费页条款）外，`textures/` 与 `hdri/` 全部为 **CC0 1.0**（Poly Haven / ambientCG 的公共领域授权），可商用、可修改、无归属义务；本仓库仍逐条记账来源以便复核。
> **贴图与 `.import` 必须一起提交**：干净检出先跑 `godot --headless --path orion-simulation-world --import`，否则 `load()` 返回 `null`（`COMPONENTS.md` §9.1 **E28**）。

## 2. `textures/` — 4 套 PBR（CC0，task_38 Step 1a）

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

处理说明：
- **降采样**：只有 ORM 降到 512²（AO/粗糙度是低频数据；`*_nor/_color` 不动分辨率）；albedo/normal 与源同尺寸，**没有做 2K→1K 降采样**。
- **重编码**：全部用 `PIL` 重编码以压进 10 MB 预算（源 1K jpg 合计 6.7 MB、2K 源合计 30 MB+）；normal 一律 4:4:4、ORM 一律 PNG 无损。
- **不重采样色彩空间**、不做色彩校正；`nor_dx` / `NormalDX` 全程未下载、未落地。
- `.import` 由 `godot --headless --path orion-simulation-world --import` 生成（`compress/mode=0` 无损 + `mipmaps/generate=true`）；**不要手改**。

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
| `assets/` 总体积（`du -sb`，含既有资产与 `.import`） | 24 306 445 |

判据：贴图单张 ≤ 1.5 MB（最大 `ground_paving_nor.jpg` 975 127）、HDRI ≤ 6 MB、`assets/` 增量 **9 524 411** ≤ 10 MB —— 全部通过。
