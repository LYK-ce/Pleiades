# assets/props/buildings/ — 城市背景楼（task_50 Step 1 入库，零碰撞）

> Presented by KeJi
> Created Date ： 2026-11-07
> Modified Date ： 2026-11-07
>
> **只入库、不摆位、不建 `.tscn`、不进 `SceneDefs`、不进 `obstacles`、不进 `GridMap`、不加任何 `CollisionShape3D`。**
> 台账口径见 `Task/task_50_demo.md` §2.5 C / §2.6；世界尺度契约见 §2.0（#1 真实比例 / #8 建筑 3~5 栋、零碰撞）。

---

## 1. 目录内容

| 路径 | 是什么 |
|---|---|
| `kenney_building-n/model.glb` | 背景楼（中型商业楼，见下表） |
| `kenney_building-n/model.glb.import` | Godot 场景导入描述（`importer="scene"`） |
| `kenney_building-n/model_colormap2.png` | **basecolor 图集**（512² PNG，导入时从 glb 内嵌图抽出，见 §3） |
| `kenney_building-n/model_colormap2.png.import` | 贴图导入描述 |
| `kenney_building-j/…`（同样 4 个文件） | 背景楼（商业裙楼） |
| `kenney_building-skyscraper-c/…`（同样 4 个文件） | 背景楼（高层塔楼） |
| `kenney_building-skyscraper-e/…`（同样 4 个文件） | 背景楼（高层塔楼，另一款） |

> 每栋 **4 个文件**：`model.glb` + `.glb.import` + 抽出的 `model_colormap2.png` + `.png.import`。
> **贴图与 `.import` 不是可选文件**（E28 同款）：只提交 `model.glb` 时干净检出会 `ERROR: Cannot open file …colormap2.png`。

---

## 2. 台账（四栋，逐栋可复核）

| 资产 ID | 来源 URL | 许可 | 三角数 | 字节 | sha256[:16] | AABB（x 长 × y 高 × z 宽，米） | 观感一句话 |
|---|---|---|---|---|---|---|---|
| `kenney_building-n` | `https://kenney.nl/assets/city-kit-commercial` | **CC0**（包内 `License.txt` 原文：Creative Commons Zero） | `4 350` | `348 138` | `0a62124f497101f7` | `2.3200 × 2.4800 × 1.8200` | 深色玻璃条窗的中型商业楼，正面一整片幕墙 |
| `kenney_building-j` | 同上 | **CC0** | `5 246` | `447 550` | `cd5968db575c57b7` | `2.0836 × 1.6930 × 1.3400` | 宽扁的商业裙楼，带雨棚与绿色檐口，横向体量 |
| `kenney_building-skyscraper-c` | 同上 | **CC0** | `1 704` | `156 430` | `e7891dcb95229905` | `1.2800 × 4.0800 × 1.3879` | 高瘦塔楼，蓝玻璃整面、楼顶带设备间 |
| `kenney_building-skyscraper-e` | 同上 | **CC0** | `1 156` | `108 786` | `4a524547ae7dc594` | `1.2950 × 4.0800 × 1.2420` | 同高但更素的板式塔楼（横向窗带），与 C 一眼可分 |

- **互相不同款**：两栋 4.08 m 高层（`skyscraper-c` / `skyscraper-e`，幕墙观感不同）+ 两栋 1.7~2.5 m 多层（`building-n` 幕墙竖向 / `building-j` 横向裙楼）⇒ 至少两种观感，实际四种。
- **三角数 / 字节**都远低于上限（≤ 100k tris / ≤ 5 MB）：最大一栋 `5 246 tris / 447 550 B`。
- **AABB 是归一后的实测值**（Godot 轴序、`min.y = 0`、水平中心在原点）。

---

## 3. 处理方式

1. **来源包**：`kenney_city-kit-commercial.zip`（`4 096 974 B`，包内 `License.txt` 写明 `City Kit Commercial (2.1)` / `License: (Creative Commons Zero, CC0)`）。
2. **取件**：只取 `Models/GLB format/building-*.glb`（Kenney 原始 glb 的贴图是**外部相对路径** `Textures/colormap.png`）。
3. **转成单文件 glb**：在临时工程 `tmp/task50/proj/` 里 `load()` → `GLTFDocument.append_from_scene()` → **`generate_buffer()`** 导出（`generate_buffer` 把图**内嵌进 BIN**，不是 `write_to_filesystem` 的外部 sidecar）⇒ 图集变成 glb 内的 `colormap2`（512² PNG）。同时把 `min.y` 抬到 `0`、水平中心归零（**不改尺寸、不做缩放**）。
4. **导入**：`godot --headless --path orion-simulation-world --import` → 0 ERROR，Godot 按 `gltf/embedded_image_handling=1`（Extract）把内嵌图落成 `model_colormap2.png` + `.import`。
5. **缩放到真实楼高不在本步**：按 `Task/task_50_demo.md` §2.5 C-6，本步**只记录原始 AABB**，缩放留给场景那一步（128 m 世界定比例）。

复现命令（离线，全部在 `tmp/task50/`）：

```bash
cd /vepfs-mlp2/c20250205/240804016/Workspace/Orion
unzip -o -q tmp/task50/assets/search/kenney/kenney_city-kit-commercial.zip \
  -d tmp/task50/assets/search/kenney/city "Models/GLB format/*" License.txt
cp -r "tmp/task50/assets/search/kenney/city/Models/GLB format/." tmp/task50/proj/city/
godot --headless --path tmp/task50/proj --import
for b in building-n building-j building-skyscraper-c building-skyscraper-e; do
  godot --headless --path tmp/task50/proj --script res://normalize_asset.gd \
    -- "res://city/$b.glb" "res://proto/kn_$b.glb" 0
done
# 逐栋拷成 assets/props/buildings/kenney_<id>/model.glb 后跑一次 orion 的 --import
```

---

## 4. 零碰撞证明（本步验收）

- **资产侧**：每栋 glb 只有一个 `MeshInstance3D` 子节点，`find_children("*","CollisionObject3D")` = **0**、`CollisionShape3D` = **0**（回读探针 `collisions=0/0`）。
- **场景侧**：本步**不动 `src/` 一行** ⇒ `SceneDefs["venue"].obstacles = []` 原样、没有 `GridMap`、没有 `.tscn` 引用这些路径（`grep -rn "kenney_building" orion-simulation-world/src/` = 0 命中）。
- 因此 LiDAR（只扫碰撞层 1|2|4）扫不到它们 —— 这是"背景楼零碰撞"的硬要求，本步已证明资产侧与场景侧两侧都干净。

## 5. 使用边界（留给摆位那一步）

- **只作远景背景**：真实楼体比例、零碰撞 ⇒ 车/人会穿过去（R13），摆位时必须整体落在车与人的活动范围之外并留 ≥ 5 m 余量。
- 观察相机是高位俯视（R14）：高楼会挡住宅区，摆位时逐栋核对俯视视线，必要时压扁或后移。
- 卡通低模 + 写实扫描人 + 体素世界 = 三种观感同框（R15）：接受"看得懂优先"，楼放远景。
- 条款：Kenney CC0，可商用、无需署名（本仓库仍在此记账）。
