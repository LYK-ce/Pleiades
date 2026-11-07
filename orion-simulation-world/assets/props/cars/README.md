# assets/props/cars/ — 社会车辆（task_50 Step 1 入库，2 款）

> Presented by KeJi
> Created Date ： 2026-11-07
> Modified Date ： 2026-11-07
>
> **2 款**社会车辆（`sedan` / `suv`），按 `Task/task_50_demo.md` **口径 #7（车辆口径）** 归一：
> **米制、朝 `+X`、轮胎底面 `y = 0`、单文件 glb（贴图内嵌）、无骨骼动画依赖**。
> **只入库、不摆位、不进 `SceneDefs`、不建 `.tscn`**（摆位与"只平移"的驱动是场景/接线那两步的事）。
> 目标尺寸表见 §2.0：真实乘用车（长 ~4.5 / 宽 ~1.8 / 高 ~1.5），**本步不改缩放**，只登记原始 AABB。

---

## 1. 目录内容

| 路径 | 是什么 |
|---|---|
| `kenney_sedan/model.glb` | 社会车辆 ①：**轿车**（三厢，`Kenney Car Kit 3.1` 的 `sedan`） |
| `kenney_sedan/model.glb.import` | Godot 场景导入描述（`importer="scene"`） |
| `kenney_sedan/model_colormap2.png` | **basecolor 图集**（512² PNG，从 glb 内嵌图抽出） |
| `kenney_sedan/model_colormap2.png.import` | 贴图导入描述 |
| `kenney_suv/…`（同样 4 个文件） | 社会车辆 ②：**SUV**（`suv`，与 ① 造型/配色都不同） |

> 每款 **4 个文件**：`model.glb` + `.glb.import` + 抽出的 `model_colormap2.png` + `.png.import`。
> **贴图与 `.import` 必须一起提交**（E28 同款）：干净检出只带 `model.glb` 会 `ERROR: Cannot open file …colormap2.png`。

---

## 2. 台账（逐款可复核）

| 资产 ID | 来源 URL | 资产 ID（包内） | 许可 | 三角数 | 字节 | sha256[:16] | 原始 AABB（x × y × z） | 归一后 AABB（X 长 × Y 高 × Z 宽） | 地面高度 | 建议配色 | 归一旋转 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `kenney_sedan` | `https://kenney.nl/assets/car-kit` | `sedan` | **CC0**（包内 `License.txt`：`Car Kit (3.1)` / `Creative Commons Zero`） | `2 032` | `124 077` | `c7dbab5c56c0af7f` | `1.500 × 1.300 × 2.550` | `2.550 × 1.300 × 1.500` | **`0.000`** | 蓝/白车身（图集自带），摆位时按 R9 放中远景即可 | `R_y(+90°)` |
| `kenney_suv` | 同上 | `suv` | **CC0** | `2 474` | `152 849` | `8eb21c5e6ade1c85` | `1.500 × 1.300 × 2.700` | `2.700 × 1.300 × 1.500` | **`0.000`** | 深色车身 + 车顶行李箱（图集自带），与 ① 一眼可分 | `R_y(+90°)` |

- **两款不同款**：`sedan` = 低矮三厢；`suv` = 高车身 + 顶置行李架 ⇒ 造型与配色都不同（满足"固定 2 款、造型或配色不同"）。
- **原始 AABB** 指的是 Kenney 出厂朝向（车长沿 **Z**、车头朝 **+Z**、宽沿 X、高沿 Y）下的包围盒；**归一后**车长沿 **+X**。
- **地面高度一律 `0.000`**：归一前 `min.y` 已经是 `0.000`（Kenney 的车轮底面正好落在 y=0），归一后再核对一次仍是 `0.000`。
- 尺寸 `2.55 / 2.70 m 长` 比真实乘用车（~4.5 m）小 —— 这是 **Kenney 卡通低模的出厂尺度**；按 §2.5「尺寸不限、AABB 记录在案」，缩放留给场景那一步（R12：本步不动任何 scale）。
- 三角数 / 字节都远低于上限（≤ 50k tris / ≤ 2 MB）。

---

## 3. 处理方式（口径 #7 判定流程，逐条落地）

1. **取件**：`kenney_car-kit.zip`（`4 814 237 B`，包内 `License.txt` = CC0）→ `Models/GLB format/{sedan,suv}.glb`（原始 glb 的贴图是**外部相对路径** `Textures/colormap.png`）。
2. **量原始 AABB**（实例化后取各 `MeshInstance3D` 世界包围盒的并集）：`sedan 1.500 × 1.300 × 2.550` / `suv 1.500 × 1.300 × 2.700`。
3. **车长轴** = AABB 最大水平轴 = **Z**；**前脸判定**用**轮子节点名**（Kenney 自带 `wheel-front-*` / `wheel-back-*`）：`sedan` 前轮 `z = +0.660`、后轮 `z = −0.660` ⇒ **车头朝 `+Z`**；`suv` 前轮 `z = +0.760`、后轮 `−0.560` ⇒ 同样朝 **`+Z`**（`police` 另有一块 `grill` 在 `z = +1.475` 复核一致）。
4. **旋转**：`0°/±90°/180°` 中把车长轴转到 **`+X`**、车头朝 **`+X`** 的那个角 = **`R_y(+90°)`**（`+Z → +X`）。
5. **平移**：`min.y → 0`（本来就是 0），水平中心归到原点（`x/z` 居中）。
6. **导出**：`GLTFDocument.append_from_scene()` → **`generate_buffer()`** ⇒ **单文件 glb**（图集 `colormap2` 内嵌为 bufferView，512² PNG，7.8 KB）；**旋转/平移表达在场景根节点变换上**（顶点未动，与 humans 同口径）。
7. **导入**：`godot --headless --path orion-simulation-world --import` → 0 ERROR；Godot 抽出 `model_colormap2.png` + `.import`（`gltf/embedded_image_handling=1` Extract）。

复现命令（离线，全部在 `tmp/task50/`）：

```bash
cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion
unzip -o -q tmp/task50/assets/search/kenney/kenney_car-kit.zip -d tmp/task50/assets/search/kenney/car \
  "Models/GLB format/*" License.txt
mkdir -p tmp/task50/proj/cars && cp "tmp/task50/assets/search/kenney/car/Models/GLB format/{sedan,suv}.glb" tmp/task50/proj/cars/ \
  && cp -r "tmp/task50/assets/search/kenney/car/Models/GLB format/Textures" tmp/task50/proj/cars/
godot --headless --path tmp/task50/proj --import
for c in sedan suv; do
  godot --headless --path tmp/task50/proj --script res://normalize_asset.gd -- "res://cars/$c.glb" "res://proto/kn_$c.glb" 90
done
# 拷成 assets/props/cars/kenney_<id>/model.glb 后跑一次 orion 的 --import
```

---

## 4. 能当车体的四条证明（本步验收）

| 口径 #7 要求 | 证据 |
|---|---|
| 米制 | 尺寸即米（`2.55 / 2.70 m` 长、`1.50 m` 宽、`1.30 m` 高），无 cm 缩放残留 |
| 朝 `+X` | 导出 glb 的根节点旋转四元数 `(0, 0.7071, 0, 0.7071)` = `R_y(+90°)`；前轮（`wheel-front-*`）在 `x > 0` 一侧（`sedan x=+0.385` / `suv x=+0.460`） |
| 轮底 `y = 0` | 回读 `min.y = 0.000`（四轮世界 AABB 底面全在 `y=0`） |
| 单文件 glb | 贴图内嵌（`images[0].bufferView`，`mimeType=image/png`），不依赖任何外部 `.bin`/`.png` 才能 `load()` |
| 无骨骼动画依赖 | 回读 `clips=[]`、`skins=0`、`skinned=false`（纯静态网格，接成车体不用改任何 `AnimationPlayer`） |
| AABB 与地面高度 | 见 §2 台账两列，均为回读实测值 |

---

## 5. 使用边界（留给接线 / 摆位那一步）

- **只入库**：本步不放进 `SceneDefs`、不建 `.tscn`、不做摆放；回读探针 `collisions=0/0`（资产里没有任何 `CollisionObject3D`）。
- **不改任何 scale**：口径 #3 要求"视觉尺寸与碰撞尺寸同步放大"，但那是 Step 2 的事；本步一行 `scale` 没动，`ChassisCatalog.DEFAULTS` / `Vehicle.tscn` 也没动。
- **只平移**（口径 #14）：两台轿车各自沿固定朝向直线开到 UGV 那排车前停下，不转向、不倒车、不掉头、不差速 —— 朝向在 spawn 时定死，本步的 `+X` 就是那个"定死"的基准。
- 卡通低模 + 写实扫描人物同框（R9）：摆位时放中远景，必要时统一纯色涂装压掉卡通感；**"封路能看懂"优先于"写实"**。
- 条款：Kenney CC0，可商用、无需署名（本仓库仍在此记账）。
