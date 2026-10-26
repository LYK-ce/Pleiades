# assets/humans/ — 人形资产（支线 human_model Step 4）

> Presented by KeJi
> Created Date ： 2026-10-24
> Modified Date ： 2026-10-25
>
> 只放**最终选用 + 1 个备选**两件人形，来源、许可、处理方式、摆位与使用边界全部写在本文件。
> 组件口径见 `COMPONENTS.md` §3 第 9 个 `.tscn` / §4.9；导入坑见 §9.1 **E28**。

---

## 1. 目录内容

| 路径 | 是什么 |
|---|---|
| `rp_carla_rigged_001/model.glb` | **首选**人形（世界里的 2 个 `Person` 都用它） |
| `rp_carla_rigged_001/model.glb.import` | Godot 场景导入描述（`importer="scene"`，指向 `.godot/imported/model.glb-*.scn`） |
| `rp_carla_rigged_001/model_rp_carla_rigged_001_dif.jpg` | **basecolor** 贴图（导入时从 glb 内嵌图中抽出） |
| `rp_carla_rigged_001/model_rp_carla_rigged_001_dif.jpg.import` | 贴图导入描述 |
| `rp_carla_rigged_001/model_rp_carla_rigged_001_norm.png` | **normal** 贴图（同上抽出） |
| `rp_carla_rigged_001/model_rp_carla_rigged_001_norm.png.import` | 贴图导入描述 |
| `rp_eric_rigged_001/…`（同样 6 个文件） | **备选**人形（**在库但没摆进世界**） |
| `LICENSE_renderpeople.txt` | Renderpeople 免费页条款原文摘录（883 B，与 `tmp/human_model/normalized/` 里的原件逐字节相同） |
| `rp_nathan_animated_003_walk/…`（**6 个文件**） | **第三件（走路的人，支线 walking_person）**：`model.glb` + `.glb.import` + 抽出的 basecolor/normal 各 1 张 + 各 1 个 `.import`；**带 88 骨蒙皮 + 1 条走路动画**，见 §9（Step 3 只把目录名加了 `_walk` 后缀，`glb` 字节与 `sha256` 不变） |

> **贴图与 `.import` 不是可选文件**：Godot 4 的 glTF 导入默认 `gltf/embedded_image_handling=1`（Extract），
> 会把 glb 内嵌的图**落成 `.glb` 旁独立文件**。只提交 `model.glb` + `.import` 时，任何人重导都会
> `ERROR: Cannot open file …_dif.jpg`，贴图全丢（`COMPONENTS.md` §9.1 **E28**）。
> `.godot/` 是引擎缓存、被 `orion-simulation-world/.gitignore` 忽略，**不进 git**。

---

## 2. 来源 URL（逐条可复制）

| 项 | 值 |
|---|---|
| 页面 | `https://renderpeople.com/free-3d-people/` |
| 下载直链（首选 + 备选同源） | `https://renderpeople.com/sample/free/renderpeople_free_rigged_people_FBX.zip` |
| 外层包字节 / sha256 | `395 419 456 B` / `dadbe6fb…`（完整值见 `tmp/human_model/normalized/SOURCES.tsv`） |
| 抓取日期 | `2026-10-24` |
| 包内成员（首选） | `renderpeople_free_rigged_people_FBX.zip!rp_carla_rigged_001_FBX.zip!rp_carla_rigged_001_zup_a.fbx` |
| 包内成员（备选） | `renderpeople_free_rigged_people_FBX.zip!rp_eric_rigged_001_FBX.zip!rp_eric_rigged_001_zup_a.fbx` |
| 站点 | renderpeople.com（官方；无网盘 / 无聚合站 / 无转载站） |

下载与转换的完整台账在 `tmp/human_model/`（`downloads/README.md`、`normalized/SOURCES.tsv`、`normalized/README.md`）。
本次落库**没有重新下载**，直接使用 Step 3 已验收的 `tmp/human_model/normalized/<name>/model.glb`（8/8 `verdict=OK`）。

---

## 3. 许可（原文逐字 + 归属）

Renderpeople 免费页原句（未改写）：

> all free models are subject to the same terms of use as the charged models in the shop, so they may be used for commercial purposes, like visualization.

- 出处页：`https://renderpeople.com/free-3d-people/`（原文快照：`tmp/human_model/probe_logs/rp_free.html`）
- 本地留档：本目录 `LICENSE_renderpeople.txt`（`sha256 aa4f15834af96a738e36ca1d3c92a58d7bc6223b06bcad3fb76ea22b461432fa`，883 B）
- 站点另售 **ML / CV 授权**（`https://humandataset.com/`），本支线**不涉及、不使用**该授权。
- 归属：人物模型 © Renderpeople（`rp_carla_rigged_001`、`rp_eric_rigged_001` 均出自上述免费页的 Rigged 合集）。

> ⚠️ 条款覆盖的是"可视化 / 商用展示"，**不覆盖**"训练模型 / 发布权重 / 再分发网格"。
> 本支线的用法边界见 §7。

---

## 4. 处理方式（数字全部取自 Step 3 的 `metrics.json` / `convert.json`）

| 项 | `rp_carla_rigged_001`（首选） | `rp_eric_rigged_001`（备选） |
|---|---|---|
| 源单位修正 `unit_fix_k` | `+0`（源即米） | `null`（源即米） |
| `height_before_m` → 归一后 | `1.730672` → **`1.7307 m`**（`scale_applied=1.000000`） | `1.863522` → **`1.8400 m`**（`scale_applied=0.987378`，压到 1.85 上限内） |
| 三角数（**未 decimate**） | `19 988` | `20 542` |
| 顶点 / mesh / surface / 材质 | `11 586` / `1` / `1` / `1` | `11 433` / `1` / `1` / `1` |
| 贴图（glb 内嵌） | basecolor `2048²` JPEG（`812 383 B`）+ normal `1024²` PNG（`1 360 158 B`） | basecolor `2048²` JPEG（`622 266 B`）+ normal `1024²` PNG（`1 325 332 B`） |
| AABB（Godot 轴序，米） | `P(-0.1532, 0.0000, -0.6143) S(0.3502, 1.7307, 1.1969)` | `P(-0.1957, 0.0000, -0.6392) S(0.3482, 1.8400, 1.2929)` |
| 脚底 | `y = 0.000000`（glTF `POSITION.min[1] == 0`） | 同 |
| 朝向 | 脚趾残差 `-2.67°` + 视觉复核 **`faces_+X`** | `+3.15°` + 视觉复核 **`faces_+X`** |
| 骨骼 / 动画 | `skins=0` / `animations=0`（已烘焙成静态 mesh） | 同 |
| glb 字节 | `2 664 856 B`（2.54 MiB） | `2 438 336 B`（2.33 MiB） |
| glb sha256 | `bc9677b8d990b2016fd88759b7d999a525990730584b731a1f4b4cced4d24690` | `12e959ba199c4f599ee8c7db4d60ddd9a9af1d5fdee9c5e7d7c4f52873458538` |

统一口径（与 `tmp/human_model/normalized/README.md` §0 一致）：**米制、人朝 +X、脚底 `y=0`、
Y-up（glTF/Godot 轴序）、单 mesh 单 surface、无骨骼无动画、三角数 ≤ 150k**。

- **没有**做 decimate（两件都没触发：`19 988` / `20 542` 都远低于 150k 预算）。
- **没有**在 Godot 节点上补 `scale` 或 `y` 偏移：模型原点已是脚底中心，放世界时 `y` 直接给地面高度。
- 贴图**没有**降采样到 1k：basecolor 保持 `2048²`（YOLO 判据优先，法线才用 `1024²`）。

复现 / 重转链（离线，从 `tmp/human_model` 出发）：

```bash
cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion/tmp/human_model
python3 tools/s3_convert.py --job rp_carla_rigged_001     # 单位修正 / 朝向 +90° / 导出 glb
python3 tools/s3_measure.py --job rp_carla_rigged_001     # 复量 H / 脚底 / AABB / tris / 材质
bash tools/s3_verify.sh                                   # V1–V9，末行 VERIFY_ALL ok=9 fail=0
```

---

## 5. 换用备选模型（只改两处，自检会盯住一致性）

```ini
; src/scenes/props/Person.tscn  —— 第 2 个 ext_resource 的 path
[ext_resource type="PackedScene" path="res://assets/humans/rp_eric_rigged_001/model.glb" id="2_model"]
```

```gdscript
# src/props/Person.gd —— 常量
const MODEL_GLB := "res://assets/humans/rp_eric_rigged_001/model.glb"
```

改完必须：

```bash
godot --headless --path orion-simulation-world --import                                  # 先导入（新 glb 会再抽 2 张贴图）
godot --headless --path orion-simulation-world --fixed-fps 60 --script res://src/tests/check_person_assets.gd
```

`check_person_assets.gd` 的 `script-model-const`（脚本常量 == `.tscn` 的 `Body.scene_file_path`）
与 `asset-aabb-*` / `asset-tri-budget-*` / `asset-sha256-*` 会把"换了模型但常量没同步"和"字节被悄悄替换"都变成红灯。

---

## 6. 世界里怎么摆的（`World.gd` 的 `PERSONS` 常量表）

| 名 | 世界坐标 `(x, y, z)` | yaw | 到出生点 `(64,64)` | 说明 |
|---|---|---|---|---|
| `person_01` | `(60, 76, 0)` | `0°` | `12.65 m` ≫ 3 m 净空 | 出生点西南开阔地；最近障碍 `wall_c` 16.0 m |
| `person_02` | `(68, 70, 0)` | `0°` | `7.21 m` ≫ 3 m 净空 | 出生点东南开阔地；最近障碍 `pillar_e` 12.8 m |

- **yaw 一律 0**：模型原生朝 **+X（世界东）**，Step 5 从 +X 侧取景正好拍到正脸。
- 两人用**同一件模型**（首选），因为只有一个 `Person.tscn`（`COMPONENTS.md` §9.4 不允许乱加 `.tscn`），
  也避免世界里多挂一份 2.5 MB 贴图。

### 6.1 与 `step.md` 举例 `(60,84)` 的差异（**有意为之，必须记录**）

`step.md` 举例 `(60, 84)`，但**实测事实冲突**：`src/tests/check_camera_shots.gd` 的
`SPOT_WORLD := Vector3(60.0, 84.0, 0.0)` 正是那里的停车点，该自检（Xvfb 档 R 组）会在同一坐标摆
一台 UGV 并连拍 3 张。人放在 `(60,84)` 会：

1. **与车重叠**（人无碰撞，不会被物理推开，只会视觉穿模）；
2. 无谓扰动 `check_camera_shots` 的 R3/R4 图像统计与车体投影基线。

取舍：沿同一条 `x = 60` **南移 8 m 到 `(60, 76)`** —— 保持 `step.md` 的原意（出生点西南侧空地、两人相距约
10 m、都在开阔地面），并保留 `step.md` 的第二个例子 `(68, 70)`。坐标仍是**固定写死**的常量，可被自检逐位断言
（`world-persons-placement` / `world-persons-spawn-clear` / `world-persons-obstacle-clear`）。

---

## 7. 使用边界（本支线的自限，不是许可条款）

- **只做本地仿真的视觉验证**：Godot 截图 → YOLOv8n 推理看"能不能认出人"（Step 5）。
- **不训练**任何模型、**不发布**权重、**不再分发**网格 / 贴图。
- 不进任何训练集、不上传对象存储、不作为对外素材。
- 若将来这些资产要**随仓库公开发布**：应换成 **MIT** 的 Microsoft Rocketbox 件（`rb_*`，Step 3 已留有
  `normalized/downloads/rocketbox/LICENSE.md`）或 **CC BY 4.0** 的 Khronos 件，并把对应 LICENSE 与
  `SOURCES.tsv` 一并带进本目录；`rp_*` 在两件之外**不再新增**。
- 本目录**只放这 2 件**（step 硬性要求）；其余 6 件候选留在 `tmp/human_model/normalized/` 作证据与替换池，
  不进仓库。
  > ⚠️ 该条是 **human_model 支线**的边界。支线 **walking_person** 随后又加进第 3 件
  > `rp_nathan_animated_003_walk`（**带走路动画**，用途与静态人形不同，见 §9）；三件的来源 / 许可 / 处理
  > 口径一致，都是 Renderpeople 免费页的官方包。

---

## 8. 边界小结（评审最关心的四条）

1. **零碰撞**：`Person.tscn` 整棵子树没有任何 `CollisionShape3D` / `PhysicsBody3D` / `Area3D`
   （`check_person_assets` 的 `asset-no-collision-*` / `scene-no-collision` / `world-persons-no-collision` 三重锁）。
   加碰撞会让 `check_lidar` 的 `F7-real-wall-distance=2.0000` / `F8-real-wall-hits=152` 基线出血，也会让 UGV 把行人当障碍绕行。
2. **不改 `World.tscn`**：`Persons` 容器由 `World.gd::BuildPersons()` 运行时建（与 `Bounds` 同策略、幂等）。
3. **必须先 `--import`**：`.godot/` 缓存缺失时 `load(model.glb)` 返回 `null`（`.tscn` 本身仍能加载，但 `Body` 变
   `null`）——**实测**：`Person` 节点仍会被建出来却是**空壳（0 个 mesh，画面里看不到人）**，引擎打 **12 行**
   `ERROR: Cannot open file 'res://.godot/imported/…*.scn'`，`check_person_assets` 报 **`total=32 failed=14`**。
   所以「0 ERROR」验收会假红，但**不会**静默通过（`COMPONENTS.md` §9.1 E28）。
4. **A-pose 是判据优先的选择**：`rp_carla` 双臂张开约 1.2 m，看着不自然，但对"检测人形"是**正收益**
   （轮廓清晰、脸无遮挡）。不整流；若 Step 5 检测不达标，再评估换 `rp_posed_00178_29`（姿态自然但围巾遮脸 + 50k tris）。

---

## 9. 第三件资产：走路的人（支线 walking_person Step 2/3）

> 与 §1~§8 的**静态**人形是**两条独立支线**：`Person` 站桩、`PersonWalking` 走动；
> 前者一字未改（`check_person_assets` 的 32 项仍逐位钉死它的高度 / 三角数 / sha256）。
> **Step 3 起两条支线共用同一个脚本** `src/props/Person.gd`（走动能力并入，`walk_speed` / `path` / `loop`
> 三字段默认静态；`PersonWalking.gd` 已删除），资产目录改名为 `rp_nathan_animated_003_walk/`。

### 9.1 目录内容（**6 个文件**；许可摘录在上一级目录）

| 路径 | 是什么 |
|---|---|
| `rp_nathan_animated_003_walk/model.glb` | 走路人形（`PersonWalking.tscn` 的 `Body`）：`3 263 492 B`（`sha256 0e907fc9…`，改名前后不变） |
| `rp_nathan_animated_003_walk/model.glb.import` | Godot 场景导入描述（`importer="scene"`） |
| `rp_nathan_animated_003_walk/model_rp_nathan_animated_003_dif.jpg` | **basecolor** `2048²` JPEG（`774 945 B`，导入时从 glb 内嵌图抽出） |
| `rp_nathan_animated_003_walk/model_rp_nathan_animated_003_dif.jpg.import` | 贴图导入描述 |
| `rp_nathan_animated_003_walk/model_rp_nathan_animated_003_norm.png` | **normal** `1024²` PNG（`1 396 855 B`，同抽出） |
| `rp_nathan_animated_003_walk/model_rp_nathan_animated_003_norm.png.import` | 贴图导入描述 |

> **许可摘录不重复放**：`LICENSE_renderpeople.txt` 在 `assets/humans/` 根（三件同一套条款，见 §3）。
> 目录里因此是 **6 个文件**（此前文档写成 7 个，是把根目录的许可文件算进去了 —— Step 3 顺手对齐）。
> **改名注意**：`.import` 的 `dest_files` 是按**资源路径**算的，改目录名后必须删旧 `.import` 再跑 `--import`，
> 否则 `load()` 找不到 `.godot/imported/*.scn`（E28 同款症状）。

### 9.2 来源（逐条可复制，全部实测过）

| 项 | 值 |
|---|---|
| 页面 | `https://renderpeople.com/free-3d-people/`（HTTP 200 / 184 773 B） |
| 区块 | **3D ANIMATED PEOPLE**（页面上的格式按钮：C4D / MAX / MAYA / U3D / **FBX** / UE4，**没有 GLB**） |
| 下载直链 | `https://renderpeople.com/sample/free/renderpeople_free_animated_people_FBX.zip`（HTTP 200，`280 263 443 B`，sha256 `21071e853c68733a879bfbc444a6ddeabc2b36689631f38b33210d3839923b67`） |
| 外层成员 | `renderpeople_free_animated_people_FBX.zip!rp_nathan_animated_003_walking_FBX.zip`（`124 443 363 B`，sha256 `db4344b9ea6dd86fa8af0e0eb061d5dd77b13fe8954d4ff1ee812d1ad40be3ce`） |
| 内层成员 | `…_walking_FBX.zip!rp_nathan_animated_003_walking.fbx`（`24 646 304 B`）+ `tex/` 下 5 张 jpg + 官方预览图 `…_walking_A.jpg` |
| 抓取日期 | `2026-10-25` |
| 站点 | renderpeople.com（官方；无网盘 / 无聚合站 / 无转载站） |

> **反斜杠坑**：内层包的成员名形如 `tex\rp_nathan_animated_003_dif.jpg`。Linux `unzip` 不把 `\`
> 当分隔符，会解出名叫 `tex\rp_...jpg` 的**单层怪文件**，后面的贴图 relink 直接找不到。
> 必须用 `tmp/walking_person/tools/w_unzip.py`（`\`→`/` 归一 + 拒绝 `..` + 扩展名白名单）。

### 9.3 处理方式（数字取自 `tmp/walking_person/normalized/rp_nathan_animated_003/convert.json` + `glb_info.txt`）

| 项 | 值 |
|---|---|
| 源单位 | `EMPTY(scale 0.01)` → 落到 armature + mesh（**两处都要** `transform_apply`，否则顶点还在 cm 空间） |
| 裁帧 | 源 `frame 1` 是**零位 T-pose**（不是步态的一帧）→ 删 880 条 fcurve 上 `co.x==1` 的键，保留 `2..69`（**68 键**），时长 `2.266667 → 2.233333 s`；接缝 `1.0336 → 0.1145 m`（`ratio 8.79 → 0.97`） |
| 根位移 | 只在 `…_root` 的 `.location` Z 通道：极差 **`2.865983 m`**（保留帧口径）→ **置 0**（原地走） |
| 身高 | 动画周期内最高 `1.860080 m`（裁帧后）→ `k = 1.84/1.86008 = 0.989205` → **`1.8400 m`**；glb `POSITION`（rest 姿态）为 `1.848954`（比动画峰值高 9 mm，仍在自检 ±1 cm 容差内） |
| 抬脚 | 周期内最低 `-0.013566 m` → `lift = 0.013419 m` → 落地后 `POSITION.min[1] = 0.000510` |
| 转身 | 绕 Blender Z **+90°**（走路方向 `-Y` → `+X`），导出 `export_yup=True` 后仍朝 **+X** |
| 骨骼 / 动画 | `88 骨`（含手指/面部/眼球）+ **1 条** `Armature\|Take 001\|BaseLayer`，**68 键 @30 fps = 2.2333 s**（原地循环，接缝 `0.1145 m` / `ratio 0.97`） |
| 三角数 | `21 506`（`10 828` 顶点，未 decimate，远低于 150k 预算） |
| AABB（Godot 轴序） | `P(-0.1313, 0.0005, -0.9286) S(0.3425, 1.8490, 1.8722)` → 脚底 `y≈0`、x 跨度 < z 跨度（朝 ±X） |
| 步速 | `2.865983 / 2.2333 × 0.989205 = **1.2694 m/s**`（资产自带步速）；代码里 `CLIP_MPS = 1.260` 与之差 **0.74%**（< plan 的 1% 判据，故不改常量，滑步残差记录在 `tmp/walking_person/normalized/README.md` §2.2） |
| glb 字节 / sha256 | `3 263 492 B` / `0e907fc9e08d6ed9d47326b49b481ad48923f521e2da354aeaf8efdd2b0e1f66` |
| 贴图 | glb 内嵌 basecolor `2048²` JPEG + normal `1024²` PNG（8192² 原图**不进 glb**） |

统一口径与静态那两件一致：**米制、人朝 +X、脚底 `y=0`、Y-up（glTF/Godot 轴序）**；
差别只有一条：**保留 skin + animation**（所以 `export_apply=False`，绝不能烘焙 Armature 修改器）。

复现链路（离线，从 `tmp/walking_person` 出发）：

```bash
cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion
W=tmp/walking_person
python3 $W/tools/w_unzip.py --zip $W/downloads/renderpeople/renderpeople_free_animated_people_FBX.zip \
  --out $W/downloads/renderpeople --only 'rp_nathan_animated_003_walking_FBX.zip'
python3 $W/tools/w_unzip.py --zip $W/downloads/renderpeople/rp_nathan_animated_003_walking_FBX.zip \
  --out $W/downloads/renderpeople/rp_nathan_animated_003_walking
python3 $W/tools/w_tex.py --src $W/downloads/renderpeople/rp_nathan_animated_003_walking/tex \
  --out $W/work/tex/rp_nathan_animated_003 \
  --basecolor rp_nathan_animated_003_dif.jpg --normal rp_nathan_animated_003_norm.jpg \
  --gloss rp_nathan_animated_003_gloss.jpg --json $W/work/tex/rp_nathan_animated_003/index.json
/vepfs-mlp2/c20250205/240804016/tool/blender/blender -b --factory-startup --python-exit-code 1 \
  --python $W/tools/w_convert.py -- \
  --fbx  $W/downloads/renderpeople/rp_nathan_animated_003_walking/rp_nathan_animated_003_walking.fbx \
  --tex  $W/work/tex/rp_nathan_animated_003 \
  --out  $W/normalized/rp_nathan_animated_003/model.glb \
  --report $W/normalized/rp_nathan_animated_003/convert.json \
  --target-height 1.84 --face-plus-x --strip-root-motion --trim-setup-frame
python3 $W/tools/w_glb_info.py $W/normalized/rp_nathan_animated_003/model.glb --check   # exit 0
python3 $W/tools/w_glb_info.py $W/normalized/rp_nathan_animated_003/model.glb --loop    # ratio 0.97
```

### 9.4 使用边界

- **不加碰撞体**（`PersonWalking` 子树零物理节点）：与 §8-1 同一条铁律；`check_lidar` 48 项逐字不变（`check_person_walk` 的 `scene-no-collision` 锁）。
- **不进装配线**、不接雷达 / 建图 / 决策链、**不做 YOLO 验证**（这一件是"会走的视觉"，不是检测样本）。
- **世界不再自动摆走动的人**（task_35 Step 3：`World.WALKERS` / `BuildWalkers()` 已删）；走动资产与
  `PersonWalking.tscn` 保留给 §9 的人形自检，运行时改由 `Pedestrian.tscn` 的 `Body` 使用
  （资产自带步速 1.2694 m/s，`Pedestrian` 的 `CLIP_MPS = 1.26` → 滑步残差 0.74%）。
- 条款同 §3：覆盖"可视化 / 商用展示"，**不覆盖**训练与再分发网格。
