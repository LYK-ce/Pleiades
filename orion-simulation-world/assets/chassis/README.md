# chassis/ — 车辆外观资产

> Presented by KeJi
> Created Date ： 2026-11-07
> Modified Date ： 2026-11-07

本目录的**溯源台账**在 `assets/README.md` §5 / §6 / §7；本文件只回答一件事：
**`ugv_m1.glb` 这个名字下面现在装的是什么、为什么尺寸是这三个数**。

## ugv_m1.glb（task_50 Step 2 起 = 0.8 m 巡逻车）

| 项 | 值 |
|---|---|
| 内容 | Kenney Car Kit 3.1 `police.glb`（**CC0**，抓取与许可见 `assets/README.md` §7 的 `patrol_car/`） |
| 为什么叫 ugv_m1 | `src/scenes/Vehicle.tscn` 的 `ModelUGV` 把路径**写死**成 `res://assets/chassis/ugv_m1.glb`，而 `Vehicle.tscn`（`src/scenes/`）与 `Vehicle.gd`（`src/vehicle/`）**都不在 task_50 Step 2 的允许改动路径**内 ⇒ 唯一合法的换外观做法是**沿用文件名换内容**（外部可追溯性由本文件 + `assets/README.md` §7 承担） |
| 归一化 | glTF 根节点 `nodes[0].scale = (0.36666667, 0.38461538, 0.25806452)`（局部系 `x` = 宽 / `y` = 高 / `z` = 长，根节点自带 `R_y(+90°)`）⇒ 车头仍 **+X**、轮底仍 **y = 0**、水平居中（模型左右/前后对称，无需平移） |
| AABB | **0.800 × 0.500 × 0.550 m**（X 长 × Y 高 × Z 宽），`min = (-0.400, 0.000, -0.275)`、`max = (0.400, 0.500, 0.275)` = `ChassisCatalog.DEFAULTS.diff_drive_ugv` 的 `body_length / height / track_width`；碰撞盒由 `DiffDrive.Configure → ApplyBodySize(Vector3(body_length, height, track_width))` 定，**与外观逐轴相等**（D11：模型 = 碰撞盒） |
| 字节 | `140 629 B`，sha256 `d94afe2eb70962d3342ef32605fd012c2d03f9f2f5e6918c36578397e20d9970` |
| sidecar | `ugv_m1_colormap2.png`（512²，导入器从 glb 内嵌图集抽出）**与 `ugv_m1.glb`、两个 `.import` 一起提交**，否则干净检出重导即 `ERROR: Cannot open file`（`COMPONENTS.md` §9.1 **E28**） |
| 旧模型 | task_42 自建 M1 底盘（0.300 × 0.250 × 0.250 m，sha256 `2067d34f…`）的字节在 git 历史里：`git show 7c3625b:orion-simulation-world/assets/chassis/ugv_m1.glb`，需要时取回 |
| 生成脚本 | `tmp/task50/tools/scale_patrol.py`（只改 glTF JSON 的节点 `scale`，bin 块 / 顶点 / 贴图 / material 一个字节不动；`--bake` 分支是 AABB 对不上时的退路） |
| 上游原件 | `assets/chassis/patrol_car/model.glb`（3.100 × 1.300 × 1.500 m）**一字未动**，作溯源 |

## 为什么是"非等比缩放"

目标尺寸表（`Task/task_50_demo.md` §2.0）给的 UGV 是 **长 0.8 / 宽 ~0.55 / 高 ~0.5**，而 Kenney 原车是
`3.100 × 1.300 × 1.500`（长/宽 = 2.07）。逐轴缩到目标 = **长/宽 1.45** —— 车会变"胖"。
这是口径 #2 要的"像样的巡检机器人"体量；统一缩放只能得到 `0.800 × 0.335 × 0.387`，高度与宽度都不达标。

## 轮子

`src/scenes/chassis/DiffDrive.tscn` 里的四个 `Wheels/*` 是**纯视觉占位**（有外观模型时 `Vehicle._Mount_Model()`
会把整棵 `Chassis/Wheels` 子树的 MeshInstance3D 全部隐藏 ⇒ 实际看见的是 `ugv_m1.glb` 自己的四轮）。
它们的 `WHEEL_RADIUS = 0.115`、摆位 `±wheel_base/2` / `±track_width/2` 只是守住
`COMPONENTS.md` §4.4"场景占位轮位与默认参数一致"；**绝不**给它们加 `CollisionShape3D`
（碰撞盒只有一个，见上面 AABB 那格）。
