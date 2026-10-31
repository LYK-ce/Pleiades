# wb_42_ugv_model — task_42 工作记录

> Presented by KeJi
> Created Date ： 2026-10-30
> Modified Date ： 2026-10-31
> 任务：Task/task_42_ugv_model.md（无人车外观：搜索 → 自建 → 替换）

## 状态（2026-10-30）
- 任务文档 Step 1~3 已定稿（人类确认）。
- 参考图：`tmp/task42/step1/ref/rosmaster_m1.png`（ROSMASTER M1 产品渲染，仅作参考，不入库）。
- 已定口径：AABB 保持 `0.30×0.25×0.25`；麦克纳姆只做视觉、运动学仍差速；非商用、神似即可；
  **tmp/task42 全程保留不清理**。

## 计划
- Step 1：搜 Poly Pizza / Sketchfab / Kenney（≤120 候选 / 90 分钟），候选全部落 `tmp/task42/step1/`，
  评审 agent 结合参考图与 UAV 风格判 `verdict.json.found`。
- 分支：`found=true` → Step 3；`false` → Step 2（Blender 自建 `ugv_m1`）→ Step 3。
- Step 3：切 `Vehicle.tscn` / 两个自检路径 / 台账 / `--import` / 截图 / fps / `git rm` 旧模型。

## 待办
- [x] Step 1 搜索与评判（2026-10-30，found=true，选中 `poly.pizza:tzOLXetacM` CC0）
- [ ] Step 2（条件）Blender 自建 —— **found=true ⇒ 跳过**
- [ ] Step 3 接线与验收

## 执行引擎启动
- 开始时间：2026-10-30 12:44:19Z
- 引擎：tmp/task42_tools/execution_engine_task42.ts + .config.json（流程照搬 task_36/38/39/40）
- 运行标记：task_42 首轮（Step 1~3）
- 计划：Step 1 搜索/评判（最多 3 轮）→ found? Step 3 : Step 2（最多 3 轮）→ Step 3
- **tmp 保留**：tmp/task42/ 全程不清理

## Step 1 记录（实现轮，2026-10-30）

### 结论
- **`found = true`**：`poly.pizza` / `tzOLXetacM`「Rover」/ Quaternius / **CC0 1.0** / 5 242（详情页）→ 7 346 tris /
  单张 512² 调色板贴图 `Atlas` / 四轮（`Wheel_1.002`、`4.001`、`4.002`、`5`）。
- 覆盖量：Poly Pizza 6 词 104 行 → 去重 **85** 条（CC0 31 / CC-BY 54）、Sketchfab **24** 条（downloadable=true，
  匿名 API 无 `download` 字段 ⇒ 需 token，全排除）、Kenney **3** 套 ⇒ 合计 **112 ≤ 120**；耗时 **≈15 min ≤ 90 min**。

### 改动文件
- 新资产：`orion-simulation-world/assets/chassis/ugv_m1.glb`（333 464 B，sha256 `2d1ec10d…f62bc`）
  + `.glb.import` + `ugv_m1_Atlas.png`(+`.import`)（`--import` 抽出的调色板贴图）。
- 候选与证据（保留）：`tmp/task42/step1/`（`candidates.json` 112 条 / `candidates/` 112 张 / `contact_sheet.png` /
  `preview_ugv.png` / `ref/uav_style.png` / `notes.md` / `raw/` / `src/` 5 个原 glb / `tools/` 10 个脚本）。
- **未改**：`assets/ATTRIBUTION.md`（CC0 不需要）、任何源码、`Task/`、`.github/`、`Cargo.*`。

### 归一参数
- `rotate-z = 90`（原件车头朝 −Y）；非等比 `k = (0.058149, 0.054558, 0.051204)`；
  末态 AABB `(-0.150,-0.125,0.000)..(0.150,0.125,0.250)`（Blender 系，z=高）；导出 `export_yup` / 无 Draco。
- 补 `NosePlate`（楔形前唇 `x∈[0.100,0.150] y∈[-0.068,0.068] z∈[0.008,0.036]`，材质 `UGV_Bumper`）⇒ M4 by-name 命中
  （节点 AABB 中心 `x = 0.125 > 0.02`）。

### 验收结果（全部绿）
- `python3 tmp/task42/step1/tools/verify_glb.py orion-simulation-world/assets/chassis/ugv_m1.glb` → `RESULT: PASS`
  （Draco 无 / 无 camera / glTF2 / AABB 0.30·0.25·0.25 ±1e-3 / nose 命中 / tris 7 358 / 贴图 1 张 512²）。
- `godot --headless --path orion-simulation-world --import` → 退出 0，无 ERROR，生成 `ugv_m1.glb.import`。
- `candidates.json` 自检：112 条、9 字段齐、id 唯一、`preview` 全部存在且非空。

### 下一步
- found=true ⇒ **Step 2 跳过**，直接 Step 3：`Vehicle.tscn` 换 `ugv_m1.glb`、跑 `check_models.gd`、台账、截图、`git rm` 旧 `ugv_chassis.glb`。

## Step 1 — Step 1 — 搜索无人车资产并评判（找到 → Step 3；没找到 → Step 2）（task_42 首轮（Step 1~3））
- 起止：2026-10-30 12:44:19Z → 2026-10-30 13:11:43Z
- 范围：只做搜索/评判/（找到时）归一：候选与证据全部落 tmp/task42/step1/；不改任何源码
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/task42/plan.md](tmp/task42/plan.md)（456 行）；仓库 `git status -uall` 与进场基线逐行一致，未改任何源码。 关键实测（省去执行期重复探测）：Poly Pizza 有 `/api/search/<q>` JSON API 且无服务端分页（`?Page` 无效），6 词去重 **85 条（31 CC0）**；Sketchfab 匿名 API 无 `download` 字段（需 OAuth）⇒ 全排除；Kenney 3 套 ⇒ 合计 **112 ≤ 120**。判据固化为硬门 G1–G4（许可/≤50k tris/可导入可归一/能贴 0.30×0.25×0.25）＋「像」三问（四轮/方盒+前唇+顶台/风格），二值出 `found`。最可能入选 `poly.pizza/tzOLXetacM`（Quaternius "Rover"，CC0，4 轮＋方盒＋顶部平台），缺前唇由归一阶段补 `NosePlate`（占前 10 mm，AABB 不变）。归一/导出链路已实跑验证：末态 AABB 精确 `(-0.15,-0.125,0)..(0.15,0.125,0.25)`，Blender 4.5.14 + Godot 4.7.2 `--import`，总耗时 ≈4 min。Step 2 备忘已记：`export_all.sh` 的 100 KB 共用体积门禁须按模型区分。 **Workbook 摘要（约 200 字）**：Step 1 计划已落 `tmp/task42/plan.md`。实测三站直连可用；Poly Pizza 走 `/api/search`、无分页，6 词去重 85 条（31 CC0）；Sketchfab 匿名无 download 字段→全排除；Kenney 3 套，合计 112≤120。判据定为硬门 G1–G4 ＋「像」三问，二值出 found；最可能入选 `tzOLXetacM`（Quaternius Rover，CC0，4 轮＋方盒＋顶台），缺前唇由归一补 `NosePlate`（占前 
- Step 1 分支：found=true（跳过 Step 2）
- verdict.json：{   "verdict": "PASS",   "found": true,   "chosen": {     "site": "poly.pizza",     "id": "tzOLXetacM",     "title": "Rover",     "author": "Quaternius",     "license": "CC0 1.0",     "url": "https://poly.pizza/m/tzOLXetacM",     "tris": 7358   },   "reasons": [     "选件 poly.pizza/tzOLXetacM「Rover」/ Quaternius / CC0 1.0（详情页 \"Licence\":\"CC0 1.0\"，raw/poly_detail_tzOLXetacM.html；HTML 与逐词 API JSON 交叉一致），非品牌翻制；CC0 无需 LICENSE_ugv_m1.txt / ATTRIBUTION 改动。",     "落地件 orion-simulation-world/assets/chassis/ugv_m1.glb：AABB 逐轴 0.30000/0.25000/0.25000（误差 0 ≤ 1cm），min=(-0.15,0.00,-0.125)（底面 y=0、足迹居中）；独立 GLB 解析与 Godot 实例化复测一致，max x=0.150 由前轮决定、补的 NosePlate 未撑大 AABB。",     "tris=7358 ≤ 50000（详情页 5242）；单张 Atlas 512² 调色板贴图 ≤2K；无 Draco、无 camera/light/anim。",     "nose 命中：节点名 NosePlate（小写含 nose），节点 AABB 中心 x=0.125 > NOSE_MIN_X(0.02)；Godot load() 复核 center=(0.125,0.022,0.0) 一致。",     "视觉核对（read_image 四图）：preview_ugv.png 方盒车身 + 顶部平台（太阳板/碟形天线）+ 4 轮接地 + 前唇可辨、+X 标注正确；uav_style.png 为低多边形 + 调色板贴图，与选件同风格（P1/P2/P3 判像）。",     "覆盖量达 §3.1：Poly Pizza 6 词（104 行→去重 85 缩略图、10 详情页）；Sketchfab 24 条（downloadable=true，匿名响应无 download 字段⇒token 排除）；Kenney 3 套；合计 112 ≤120；耗时 889 s ≤90 min；分页实测（Page=1 空 / Page=2·3 同首屏）有原始响应

## Step 2 — Step 2 — 没找到：Blender 自建 M1 风格底盘
- 跳过：Step 1 判定 found=true，本 step 条件 when="not_found" 不满足

## 人类裁决（2026-10-31）
- Step 1 review 判 `found=true`（poly.pizza `tzOLXetacM` / Quaternius "Rover"，CC0，4 轮方盒，7358 tris）。
- 人类看 `tmp/task42/step1/preview_ugv.png` 后**否决**：月球车造型（碟形天线/车窗/太阳能板），与 ROSMASTER M1 不像。
- 处理：`tmp/task42/step1/verdict.json` 改为 `found=false`（原评审结论见本节记录；verdict.json 曾被引擎轮次清理误删，已修引擎：只在 Step 1 轮次清理）；从 **Step 2** 重跑；Step 1 候选/对比图全部保留。
- 引擎修正：`FROM_STEP=2`、`RUN_TAG` 改为「Step 2~3（Step 1 搜到 rover，人类否决 → 转自建）」。

## 执行引擎启动（重跑 Step 2~3）
- 开始时间：2026-10-31 01:11:13Z
- 引擎：tmp/task42_tools/execution_engine_task42.ts + .config.json
- 运行标记：Step 2~3（Step 1 搜到 rover，人类否决 → 转自建）
- 前置：Step 1 已 PASS 但 found=true 被人类否决 → verdict 改 found=false → 从 Step 2 起

## Step 2 记录（实现轮，2026-10-31，第 1 / 3 轮）

### 结论
- **自建 `ugv_m1` 完成**：`Tool/blender/models/build_models.py` 新增模型键 `ugv_m1` + `Build_Ugv_M1()`（旧 `ugv` 键与
  `Build_Ugv()` 一行未改，可逐字复现）；`export_all.sh` 新增 ⑤ 段（`ORION_BUILD_LEGACY=0` 可只建新件）。
- 数字：`objects=31  tris=2016`（上限 12 000）/ AABB 精确 `(-0.150,-0.125,0.000)..(0.150,0.125,0.250)` /
  glb **84 584 B**（≤150 KB）/ 4 种材质、0 贴图（⇒ 无 sidecar）/ glb 层 AABB `size=(0.30, 0.25(高), 0.25)`、`min.y=0`。
- 幂等：连跑两次 `sha256sum` 相同（`650d9181…3920f17`）。

### 与计划的一处口径偏差（C1，已裁决并落实）
- `§四` 写「轮心 `x=±0.12`、半径 ≈0.045」，与「AABB 精确 0.30 m」数学冲突（`0.12+0.045=0.165 > 0.150`）。
  **保半径、收轮心**：`M1_WHEEL_X = 0.105`，轮外缘正好压 `x=±0.150`；`y=±0.1125 / z=0.045 / 宽 0.025` 照原文。

### 命令与结果
- `ORION_BUILD_LEGACY=0 ORION_BLEND_OUT_DIR=$PWD/tmp/task42/step2 bash Tool/blender/models/export_all.sh` → EXIT=0，日志见
  `tmp/task42/step2/build.log`（`.blend` 落 tmp，未在 `Tool/blender/models/` 新增文件）。
- `python3 tmp/task42/step2/tools/verify_glb.py …/ugv_m1.glb` → `RESULT: PASS (7/7 …)`。
- `godot --headless --path orion-simulation-world --import` → EXIT=0；首跑报 2 条 Step 1 遗留 uid 的 stale cache ERROR
  （无源文件引用），第二次纯跑 `ERROR=0`，`ugv_m1.glb.import` 的 `uid` 仍 `uid://dwy17ous1dlyq`。
- 自检不回归：`check_sensor_models.gd` `total=29 failed=0`、`check_models.gd` `total=24 failed=0`（均 `SELF-CHECK OK`）。
- 预览：`tmp/task42/step2/preview_ugv.png`（1000×750，正面 3/4，含地面）；对照图 `preview_compare.png`。
- 清理：删 Step 1 候选遗留的 `ugv_m1_Atlas.png`(+`.import`)；`ugv_chassis.glb` 未动（Step 3 才删）。

## 执行引擎启动（Step 2 第 2 轮）
- 开始时间：2026-10-31 01:30:25Z
- 运行标记：Step 2 第 2 轮（按侧视轮廓修正：去喇叭塔 / 摄像头前置）
- 前置：人类看 v1 预览后要求删喇叭塔、摄像头前置、按侧视轮廓重做

## Step 2 记录（实现轮，2026-10-31，第 2 / 3 轮）

### 结论
- **v2 重做完成（PASS）**：`Build_Ugv_M1()` 按侧视轮廓重建，16 个 mesh / **1 792 tris** / 4 材质 / 0 贴图 /
  glb **67 836 B**（v1：31 mesh / 2 016 tris / 84 584 B）。
- v1 的「后部喇叭塔」（`RearModule/HeatsinkPlate/Fin0..5/TopCap/AntennaL,R`）、「悬空传感器横杆」
  （`SensorBar/SensorBracketL,R/SensorFace`）、尖楔 `NoseWedge` **全部删除**；新增 `Roof` / `Lidar` / `LidarBand` /
  `FrontCamera` + `CamLens`（朝 +x）/ `RearPanel`，`NosePlate` 改为方正浅灰横向保险杠。
- 轮子几何一字未改，仅把**轮毂**刷成浅灰（槽 1）：参考图「银色轮毂 + 黑辊子」，`Wheel*` 仍是单节点。
- `Assert_Ugv_M1()` 新增两条硬断言：①除激光雷达外任何件 `z_max ≤ 0.215`（禁喇叭塔）；②`FrontCamera` 必须在前端、
  车身顶面高度。两条都做了反向验证（临时改常量 → `EXIT=1` + `RuntimeError`，已还原并重跑，glb sha 逐字节相同）。

### AABB 与命名
- Blender：`min=(-0.150,-0.125,0.000) max=(0.150,0.125,0.250)`、`size=(0.3000,0.2500,0.2500)`（逐轴误差 0）。
- glb（Y-up）：`size=(0.300000,0.250000,0.250000)`、`min=(-0.150000,0.000000,-0.125000)`。
- 节点名：`NosePlate`（+`NoseLed`）/ `WheelFL/FR/RL/RR` 不变；`verify_glb` `d1/d2/d3/d5` 全绿。

### 命令与结果
- `ORION_BUILD_LEGACY=0 ORION_BLEND_OUT_DIR=$PWD/tmp/task42/step2 bash Tool/blender/models/export_all.sh` → `EXIT=0`，
  `[build] ugv_m1 objects=16 tris=1792 …` + `[export_all] 跳过 ①~④` + `[export_all] OK 67836B …/ugv_m1.glb`；
  `.blend` 落 `tmp/task42/step2/`（`Tool/blender/models/` 未新增文件）。连跑两次 `sha256sum` 相同 ⇒ 幂等。
- `python3 tmp/task42/step2/tools/verify_glb.py …/ugv_m1.glb` → `RESULT: PASS (7/7 断言全绿; tris=1792 nodes=16 bytes=67836)`。
- `godot --headless --path orion-simulation-world --import` → `EXIT=0`，三次全部 `ERROR=0`（第 1 轮的 stale uid ERROR 未复现）；
  `ugv_m1.glb.import` 的 `uid` 仍 `uid://dwy17ous1dlyq`。
- 自检不回归：`check_models.gd` `total=24 failed=0`、`check_sensor_models.gd` `total=29 failed=0`（均 `SELF-CHECK OK`）。
- 预览：`tmp/task42/step2/preview_ugv.png`（3/4）、`preview_side.png`（侧视）、`preview_compare.png`、
  `preview_compare_side.png`（与画板同框）；`read_image` 逐条判过 7 条造型判据（无后部塔 / 摄像头前端低位 / 无悬空横杆 /
  车头非尖楔 / 麦克纳姆可辨 / 配色 / 长条方盒下车体）。v1 预览保留为 `preview_v1_speaker.png`。
- 范围：`git status -uall` 排序集合与 `tmp/task42/git_baseline.txt` 完全一致（`GIT_CLEAN_FINAL`）；chassis 目录无
  `ugv_m1_Atlas.png` sidecar；`ugv_chassis.glb` 未动（Step 3 才 `git rm`）；无 `git add`/`commit`、无 `cargo fmt`。
- 资产台账：`ugv_m1.glb` 84 584 B → **67 836 B**；sha256 `650d9181…3920f17` → `47f98139…6ba3e28`。
- 详细实施记录见 `tmp/task42/impl.md`（含与计划的 6 条偏差：D1 make_compare 标题、D2 侧视镜像、D3 沿用 §9.1 两处
  半宽偏离、D4 雷达"中后段"文字 vs 定值、D5 `check_models` 仍指旧资产、D6 `min z` 打印 `0.0000`）。

### 下一步
- Step 3：`Vehicle.tscn` → `ugv_m1.glb`、两个自检脚本换路径、`assets/README.md` 台账、`--import`、`xvfb-run` 截图、fps、
  `git rm ugv_chassis.glb`(+`.import`)。

## 执行引擎启动（Step 2 第 3 轮）
- 开始时间：2026-10-31 01:41:28Z
- 运行标记：Step 2 第 3 轮（压低车身 / 深色车顶 / 摄像头落位）
- 前置：v2 预览曝光车顶浅灰 / 车身偏高 / 摄像头偏高，按 v3 反馈重做

## Step 2 记录（实现轮，2026-10-31，第 3 / 3 轮）

### 结论
- **v3 完成（PASS）**：只动人类指定的三件 —— ① `Roof` 材质 `light → dark`；② 上半身整体压低
  （`Hull` 顶 0.155→0.125、甲板 0.2045→0.200、侧甲板/尾板顶 →0.120）；③ 前摄像头从车顶高度落到
  **车身顶面**（z 0.1245..0.160，比车顶底低 0.0395 m），引擎盖斜坡改到 `(0.090,0.1245)→(0.020,0.200)`。
- 拓扑/命名/管线全部保留：16 → **15** 个 mesh 节点（`CamLens` 并入 `FrontCamera`，镜头端面正好
  x=0.1500 ⇒ 该节点 AABB 恰为人类表格 `x[0.115,0.150]`）；**tris 1792 不变**、材质 4 种、0 贴图。
- AABB 仍逐轴精确 `0.30 × 0.25 × 0.25`（Blender `min=(-0.150,-0.125,0.000) max=(0.150,0.125,0.250)`）；
  glb 84 584 → **67 752 B**（较 v2 的 67 836 B 少 84 B）。
- `Assert_Ugv_M1()` 的 v2 两条软规则换成 v3 **逐件硬断言**（7 个件的 z 区间 + 摄像头三轴区间 +
  「摄像头比车顶低 ≥0.030」+「斜坡退到摄像头后方 ≥0.020」+「除雷外部件 z ≤ 0.210」+ 车顶材质必须是深灰）。

### 命令与结果
- 构建：`ORION_BUILD_LEGACY=0 ORION_BLEND_OUT_DIR=$PWD/tmp/task42/step2 bash Tool/blender/models/export_all.sh`
  → `EXIT=0`；`[build] ugv_m1 objects=15 tris=1792 … size=(0.3000,0.2500,0.2500)` + `[export_all] OK 67752B`；
  重跑 4 次 `sha256` 全等 `2067d34f…2169545` ⇒ 幂等。
- 反向验证 5 条（只改 `tmp/…/neg_test/v3/` 副本）全 `EXIT=1`：`roof_light` / `hull_tall` / `cam_high` /
  `lens_far`（由 `Assert_Model` 的 AABB 兜底）/ `deck_forward`，各自按预期文案拦下。首轮 `cam_high`
  曾暴露新断言自身的一处 `%` 格式化缺陷（仍 EXIT=1，但报 `TypeError`），已修好并重跑。
- `python3 tmp/task42/step2/tools/verify_glb.py …/ugv_m1.glb` → `RESULT: PASS (a~i 组全绿; tris=1792
  nodes=15 bytes=67752)`（新增 i1..i7：车顶材质名 / Hull / 侧甲板 / Deck / Roof / Lidar / FrontCamera）。
- `godot --headless --path orion-simulation-world --import` → `EXIT=0`，`ERROR=0`；`uid` 仍 `uid://dwy17ous1dlyq`。
- 新写 Godot 探针 `tmp/task42/step2/tools/probe_m1.gd`（Step 3 的 M3/M3b/M4 预演，AABB 口径与
  `check_models.gd` 的 `_aabb_of_local_in` 一致）→ `PROBE_EXIT=0`、`P1..P7` 全绿、
  `model AABB pos=(-0.15,0.0,-0.125) size=(0.3,0.25,0.25) mesh_nodes=15`。
- 自检不回归：`check_models.gd` `total=24 failed=0`、`check_sensor_models.gd` `total=29 failed=0`（均 OK）。
- 预览：`preview_ugv.png`（正面 3/4）/ `preview_side.png`（侧视）均 1000×750，另出
  `preview_compare.png` / `preview_compare_side.png` 对照图；看图 V1..V7 全过（深色车顶、车身顶面 ≈50 %、
  摄像头坐车身顶面、斜引擎盖从摄像头后方起坡、雷达带蓝扫描窗并撑满 0.250、麦克纳姆可辨、无后部塔）。
- 范围：`git status -uall` 前后路径集合逐行相同；chassis 目录无 sidecar；`Tool/blender/models/` 无
  `__pycache__`；`ugv_chassis.glb` sha 仍 `089b1bbf…035878`（Step 3 才删）；无 `git add/commit`、无 `cargo fmt`。
- 详细实施记录与 **10 条偏差（D1..D10）** 见 `tmp/task42/impl.md`。

### 下一步
- Step 3：`Vehicle.tscn` → `ugv_m1.glb`、两个自检脚本换路径、`assets/README.md` 台账、`--import`、
  `xvfb-run` 截图、fps、`git rm ugv_chassis.glb`(+`.import`)。

## 人类确认（2026-10-31）
- 人类看 Step 2 第 3 轮预览（`tmp/task42/step2/preview_ugv.png`）后确认：**就用这一版，不再改模型**。
- 定稿特征：深色车顶、车身压低（顶面 0.125）、摄像头坐车身顶面（0.125..0.160）、斜引擎盖到 0.200、平顶 0.200..0.210、顶置激光雷达 0.210..0.250、浅灰前保险杠 + 蓝 LED、麦克纳姆轮视觉件。
- 后续：Step 3 接线/自检/台账/截图/fps/删旧模型；模型不再迭代。

## Step 2 — Step 2 — 没找到：Blender 自建 M1 风格底盘（Step 2 第 3 轮（压低车身 / 深色车顶 / 摄像头落位））
- 起止：2026-10-31 01:41:28Z → 2026-10-31 01:55:58Z
- 范围：只做资产：扩展 build_models.py 新增 ugv_m1 + 导出 glb + 预览（不动旧 ugv 键、不动其它资产、不动源码）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/task42/plan.md](tmp/task42/plan.md)，本会话仅写 `tmp/task42/`，仓库路径集合与进场基线逐行一致。 **摘要（约 190 字）**：计划分范围、定值、改动、命令、风险、验收六段。核心口径已在 Blender 实跑校验：AABB 逐轴零误差、tris 1792、材质 4。改动仅 `build_models.py`：Roof 改 dark；Hull z 0.030..0.125、侧甲板 0.045..0.120、RearPanel 同压；Deck 斜坡 `(0.090,0.1245)→(0.020,0.200)`；Roof 0.1995..0.210；Lidar 0.2095..0.250；摄像头壳和蓝镜头合并成单节点 `FrontCamera`（x 0.115..0.150、z 0.1245..0.160），镜头端面 0.150 兼撑 AABB。仅隐藏下表面嵌 0.5 mm，可见面全按人类定值；断言含“摄像头低于车顶”“斜坡在摄像头后方”两条硬门，另有 5 条反向验证、`verify_glb` i 组、新 Godot 探针（M3/M4 预演）及 10 条判据。

## Step 3 — Step 3 — 接线替换 + 自检 + 台账（Step 2 第 3 轮（压低车身 / 深色车顶 / 摄像头落位））
- 起止：2026-10-31 01:55:58Z → 2026-10-31 02:30:01Z
- 范围：切 Vehicle.tscn / 两个自检路径 / 台账 / --import / 截图 / fps / git rm 旧模型（物理零改动）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task42/plan.md`（526 行，未改任何源码，`git status` 无新增路径）。 **摘要（<200 字）**：本步只改 3 处路径（`Vehicle.tscn:18`、`check_models.gd:39`、`check_sensor_models.gd:30`）＋注释同步，`UGV_SIZE`/`UGV_HULL`/M4~M6/DiffDrive 零改动。顺序：改前基线（19 headless＋4 渲染自检＋`world_fast` 3 次 fps）→ 接线 → `--import`×2 → 改后同套自检并逐条 diff `total=` → 截图（CameraSensor 出 1280×720，投影角点自断言框住车/人/UAV）→ fps（≥11 且 ≥改前−1）→ 台账写「自有资产」（sha256 `2067d34f…`、67 752 B、1 792 tri）→ 全绿后 `git rm` 旧 glb+`.import` 再补 `--import`。紧的要盯：tris 1792/上限 2000；`ATTRIBUTION.md` 不动。

## 引擎结束
- 结束时间：2026-10-31 02:30:01Z
- 状态：ALL_STEPS_DONE
- tmp/task42 保留
