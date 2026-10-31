# task_42_ugv_model — 无人车外观资产：搜索 → 自建 → 替换

> Presented by KeJi
> Created Date ： 2026-10-30
> Modified Date ： 2026-10-30
> 状态：**Step 1 ~ Step 3 定稿**（人类 2026-10-30 确认）。

## 一、目标

把 sim 世界里自建的 UGV 低模（`orion-simulation-world/assets/chassis/ugv_chassis.glb`，416 tris，Blender 参数化箱体）
换成一台**神似 ROSMASTER M1** 的四轮麦克纳姆底盘外观：
优先找免费资产（CC0 > CC-BY），找不到/不合格就 Blender 自建；运动学保持不变（仍差速），
尺寸保持现有 AABB，物理零改动。

参考图（人类本次会话提供，ROSMASTER M1 产品渲染）：`tmp/task42/step1/ref/rosmaster_m1.png`（970×600 PNG）。

---

## 二、已定口径（人类 2026-10-30 确认，执行时不再回头问）

| # | 项 | 定值 |
|---|---|---|
| 1 | 尺寸 | **保持 `0.30 × 0.25 × 0.25` m**（人类实测；模型即碰撞盒，task_33 D11）⇒ `UGV_SIZE` / `UGV_HULL` / `DiffDrive` 车体参数**一个不改** |
| 2 | 运动学 | 麦克纳姆轮**只做视觉**；真车就是当前差速模式，**不加横移**（`speed/turn` 不变） |
| 3 | 版权 | **非商用**，参考图"神似"即可；**不得**出现 ROSMASTER 字样/logo，不做品牌翻制；参考图只放 `tmp/`，**不入库** |
| 4 | 来源优先级 | 搜索（CC0 > CC-BY，CC-BY 须配齐 LICENSE + ATTRIBUTION + 台账）→ 都不合格则 Blender 自建（自有版权） |
| 5 | **tmp 保留** | 本任务结束**不清理** `tmp/task42/`（人类要求看 Step 1 搜到的全部候选）。这是对 `.github/instructions.md`「程序运行完毕后清理 tmp」的**明确例外**，引擎收尾不得 `rm -rf tmp/task42` |
| 6 | 资产名 | 新模型统一 `assets/chassis/ugv_m1.glb`；旧 `ugv_chassis.glb`（+`.import`）在 Step 3 验收通过后 `git rm`；`.blend` 源与 `build_models.py` 的旧 `ugv` 键保留 |
| 7 | 朝向/原点 | +X、底面 `y = 0`、足迹居中（与 UGV/UAV/human 同一套口径） |

---

## 三、Step 1 — 搜索与评判（找到 → Step 3；没找到 → Step 2）

### 3.1 停止标准（硬上限，先到先停）

| 站点 | 上限 | 说明 |
|---|---|---|
| Poly Pizza | 6 个词 × 最多 2 页 ≈ **80 个缩略图**；最多 **10 个**详情页 | 词表：`rover` / `ugv` / `robot car` / `mecanum` / `agv` / `inspection robot` |
| Sketchfab | 仅 `downloadable=true`，最多 **40 条** | **拿不到直下文件的不计入候选**（下载要登录 token 的一律排除） |
| Kenney | 最多 **3 套** | 风格化低模，仅兜底 |
| 合计 | **≤ 120 个候选 / ≤ 90 分钟**，先到者为准 | 到上限仍无合格件 ⇒ `found = false` |

> 参考 task_40 §六 的选型链与代理说明：直连优先，失败再加 `-x http://127.0.0.1:7897`（mihomo mixed-port）。
> Poly Pizza 的 `static.poly.pizza` 缩略图需要 `-A "Mozilla/5.0" -e "https://poly.pizza/"`，否则 403。

### 3.2 候选判据（7 条）

1. **四轮**（麦克纳姆辊子最好；0.3 m 尺度允许近似，但必须是轮式底盘）
2. **方盒车身 + 前保险杠 + 顶部平台**（顶部计算模块允许后加）
3. **许可** CC0 或 CC-BY（CC-BY 必须能配齐 LICENSE + ATTRIBUTION + 台账）；**排除** CC BY-SA / CC BY-NC / personal-only / 品牌机型翻制
4. **单 glb、≤ 50k tris、贴图 ≤ 2K**
5. **可导入 Godot、可归一**（+X、底面 `y=0`）
6. **缩放后能贴到 `0.30 × 0.25 × 0.25`**（形状不能太扁/太窄/太宽）
7. **与 UAV 风格协调**（低多边形 + 调色板贴图，见 3.5）

> `found = true` 需要 **3/4/5/6 硬满足** + 评审对 **1/2/7** 判"像"。

### 3.3 产物（`tmp/task42/step1/`，本任务保留）

```text
tmp/task42/step1/
├── ref/rosmaster_m1.png        # 参考图（从会话附件复制）
├── ref/uav_style.png           # 我们 UAV 模型的渲染图（风格基准）
├── candidates/                 # 全部候选缩略图：<site>_<id>.webp/png
├── candidates.json             # 每条：site/id/title/author/license/url/preview/tris/notes
├── contact_sheet.png           # ≤12 张对比图，带标题/许可
├── notes.md                    # 每站查询数/候选数/耗时/命令/失败项（覆盖量证据）
└── preview_ugv.png             # found=true 时：归一后的模型渲染
```

`verdict.json`（**评审**写，引擎据此分支）：

```json
{"verdict":"PASS","found":true,"chosen":{"site":"poly.pizza","id":"...","title":"...","author":"...","license":"CC0 1.0","url":"...","tris":1234},"reasons":["..."]}
```

- `found=false` **同样是 PASS**——只要覆盖量达标、候选落盘完整、没有漏掉明显合格件。
- 引擎分支只看 `verdict.json.found`：`true` → 跳过 Step 2；`false`/缺失 → 执行 Step 2。

### 3.4 找到时：本 Step 内完成归一

1. 下载原 glb；记录 URL / 许可原文 / 字节 / sha256 / 抓取日期。
2. Blender 归一：清非 MESH、断父保变换、`transform_apply`、朝 +X、底面 `y=0`、缩放到目标 AABB（允许非等比）、导出 GLB（Y-up、无 Draco）。
3. **必须保证存在名字含 `nose` 的 mesh**（优先把前脸命名为 `NosePlate`；没有就补一块前脸板）——`check_models.gd` 的 M4 只认 by-name（节点名含 nose）或 by-mat（材质名含 nose）。
4. 导出到 `orion-simulation-world/assets/chassis/ugv_m1.glb`，`godot --headless --path orion-simulation-world --import` 生成 `.import`。
5. 渲染 `tmp/task42/step1/preview_ugv.png`（Blender 或 Godot 截图）。
6. CC-BY 时：`assets/chassis/LICENSE_ugv_m1.txt` + `assets/ATTRIBUTION.md` 追加条目（Step 3 校验）。

### 3.5 UAV 风格基准

实现 agent 用 Blender 渲染 `orion-simulation-world/assets/chassis/uav_quadrotor.glb` 到
`tmp/task42/step1/ref/uav_style.png`（失败则用源模型预览图代替并在 `notes.md` 记录）。
评审 agent 必须**看图**：`read_image` 参考图、`uav_style.png`、`contact_sheet.png`、`preview_ugv.png`。

### 3.6 验收

- 覆盖量：`notes.md` 的每站查询数/候选数 ≥ 3.1 的上限要求；
- `candidates/` 与 `candidates.json` 一一对应，无空条目；
- `contact_sheet.png` ≤12 张、带标题/许可；
- 评审判 PASS（`found` 可为 `false`）；
- `found=true` 时：`ugv_m1.glb` 存在、AABB 逐轴 ≤ 1 cm 误差、tris ≤ 50k、nose 命中、许可台账齐、`preview_ugv.png` 可辨。

---

## 四、Step 2 — 没找到：Blender 自建（条件执行）

### 4.1 造型要素（**以侧视轮廓为准**，人类 2026-10-31 反馈）

> 侧视草图：`tmp/task42/step1/ref/side_view_blackboard.png`（画板导出，比例不一定准，**轮廓优先**）。
> 参考图后部隆起的"台子"实际是**喇叭**，**不建模**。

侧视轮廓（左 = 车头 +X）：

```text
        摄像头                                 激光雷达
        ┌──┐                                  ┌──┐
        │  │        ╱─────────────────────────│  │
        │  │      ╱   （引擎盖斜坡 → 平顶）    └──┘
   ┌────┴──┴────╱──────────────────────────────┐
   │            （下车体长条方盒）              │
   └───────────────────────────────────────────┘
```

- **下车体**：长条方盒（深灰），x `-0.14..0.14`、y `-0.10..0.10`、z `0.03..0.125`；
- **前保险杠**：浅灰横向板（略宽于车身），节点名 **`NosePlate`**，x `0.13..0.15`、z `0.05..0.115`，可加蓝 LED；
- **前摄像头**：小方盒装在最前端、**车身顶面高度**（不是半空），x `0.115..0.150`、y `±0.028`、z `0.125..0.160`，黑壳 + 朝前镜头面；
- **引擎盖**：从摄像头后方斜坡升到平顶（`(x=0.09,z=0.125) → (x=0.02,z=0.20)`）；
- **车顶**：平顶平台 `x -0.14..0.02`、z `0.205..0.215`；
- **激光雷达**：车顶后段小方盒/圆柱，x `-0.06..-0.015`、y `±0.0225`、z `0.210..0.250`（顶面撑 AABB）；
- **四轮**：麦克纳姆轮（轮毂 + 两排 45° 辊子），**只做视觉**；
- **配色**：深灰 / 黑 / 浅灰 / 少量蓝色点缀；材质数 ≤ 5。

> 第 1 版（v1）的"后部高塔 + 散热鳍片 + 悬空白色传感器横杆"已被人否决：
> 后部隆起是喇叭（删），摄像头应在最前端低位（改）。

### 4.2 实现

- 扩展 `Tool/blender/models/build_models.py`：新增模型键 `ugv_m1` 与 `Build_Ugv_M1()`；**不改**旧 `ugv` 键（旧资产可复现）。
- AABB 断言：`(-0.150, -0.125, 0.000) .. (0.150, 0.125, 0.250)`（Blender 坐标：x 长 / y 宽 / z 高）。
- 节点命名：`NosePlate` + `WheelFL/FR/RL/RR` + 车身件（车身件名字不作断言）。
- 轮子：半径 ≈ 0.045、宽 ≈ 0.025，轮心 `x=±0.12`、`y=±0.1125`、`z=0.045`；辊子沿圆周 45° 斜置。
- 三角数上限：**≤ 12 000**（辊子会涨面；全局 `TRI_LIMIT = 2000` 需按模型区分，不能一刀切）。
- 导出 `orion-simulation-world/assets/chassis/ugv_m1.glb`；渲染 `tmp/task42/step2/preview_ugv.png`。
- 幂等：重复跑结果一致；断言失败 → 退出非 0，不产出半成品。

### 4.3 验收

- 脚本退出 0；AABB 精确到 1e-4；tris ≤ 12 000；
- 预览图与原图对照：方盒 + 前唇 + 顶部模块 + 麦克纳姆轮可辨；
- 其他模型（`lidar` / `camera` / 旧 `ugv`）不回归。

---

## 五、Step 3 — 接线与替换（两个分支共用）

### 5.1 改动

| 文件 | 动作 |
|---|---|
| `orion-simulation-world/src/scenes/Vehicle.tscn` | `ModelUGV` 的 `ext_resource` → `res://assets/chassis/ugv_m1.glb` |
| `orion-simulation-world/src/tests/check_models.gd` | `UGV_MODEL_SCENE` → 新路径；`UGV_SIZE` / `UGV_HULL` **不变**；M4 `NosePlate` 断言不变 |
| `orion-simulation-world/src/tests/check_sensor_models.gd` | `UGV_GLB` → 新路径 |
| `orion-simulation-world/assets/README.md` | 台账行：来源 / 许可 / 处理 / 字节 / sha256 |
| `orion-simulation-world/assets/ATTRIBUTION.md` | 仅 CC-BY 时追加 |
| `orion-simulation-world/assets/chassis/ugv_chassis.glb`(+`.import`) | `git rm`（验收通过后） |
| `orion-simulation-world/COMPONENTS.md` | 若写了旧模型路径则同步 |

### 5.2 验收

1. `godot --headless --path orion-simulation-world --import` → 0 ERROR，`.import` 一起提交；
2. `check_models.gd` / `check_sensor_models.gd` 全绿；
3. `xvfb-run -a` 截图可辨（方盒 / 前唇 / 顶部模块 / 辊子），与写实人形、UAV 同框不违和；
4. `RobotSim_TestEnv/world_fast/` 下 fps ≥ 改前基线（11 fps）；
5. **动力学零改动**：`DiffDrive.gd` 常量、`UGV_HULL`、碰撞盒、`check_models` 的 M5/M6 逐条不变；
6. `git diff` 只含上表 + `Task/` + `Workbook/`。

---

## 六、不做

- 不改运动学、不加横移、不改 `nav/` / 行为树 / 协议 / Pictor；
- 不改其它资产（UAV / 传感器 / 行人 / 世界）；
- 不做 LOD / 动画 / 骨骼 / 贴图烘焙；
- 不做品牌 logo/字样（仅"神似"；
- **不清理 `tmp/task42/`**（本任务例外）。

---

## 七、风险

1. 搜到的模型可能没有 `nose` 节点 → Step 1 归一时必须补/改名，否则 M4 红。
2. 搜到的模型轮子位置/轮径与底盘不匹配 → 归一后以 AABB 底面为准，视觉上可能悬空/穿地，需在 `preview_ugv.png` 里确认。
3. 麦克纳姆辊子面数 → 自建 tris 预算 12k；搜来的 ≤50k 但 fps 需实测。
4. GLB 材质/贴图导入 Godot 可能丢材质 → `--import` 后看 `check_models` 与截图。
5. 旧模型删除时机 → Step 3 验收通过后再 `git rm`，避免中途无法回退。
6. `check_models` M4 只认名字含 `nose` 的 mesh 或材质名含 `nose` —— 两条路至少走一条。

---

## 八、已定参数速查

| 项 | 值 |
|---|---|
| 目标 AABB | `0.30 × 0.25 × 0.25` m |
| 朝向 / 原点 | +X / 底面 `y=0` / 足迹居中 |
| 运动学 | 差速，`speed/turn` 不变 |
| 搜索上限 | 120 候选 / 90 分钟 |
| 新资产名 | `ugv_m1.glb` |
| tris | 搜索 ≤ 50k；自建 ≤ 12k |
| tmp | `tmp/task42/` **保留** |
