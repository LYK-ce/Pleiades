# task_50_demo — 国赛答辩演示：散场封路 → 一人奔跑 → 无人机跟踪 → 四车围控

> Presented by KeJi
> Created Date ： 2026-11-07
> Modified Date ： 2026-11-07
> 状态：**目标 + Step 1 已写；其余 Step 待补**
> 上游：
> - 比赛方案 `XA-202615 大规模无人集群联合多域防控技术方案研究`（≥5 台异构平台、≥3 类典型场景、
>   协同封控四要素、单人多机操控、5~10 min 演示视频）。
> - 2026-11-07 与人类的讨论（黑板三版草图）。

---

## 一、目标

### 1.1 一句话

用 **16 台（12 车 + 4 机）** 在仿真世界里拍一段 **约 85 秒（1×）** 的素材，剪成 **45 秒**放进 **6 分钟国赛答辩**：

> 一场大型活动散场 —— **8 台车把马路封成"车墙"，让人流安全过街**；人群中 **一人突然奔跑脱离**；
> **1 台无人机锁定跟踪**（另 3 台照常巡逻）；**4 台车抢到他的前方合围**，封控圈闭合。

### 1.2 这段片子要在答辩里承担什么

| # | 要证明的事 | 对应的画面 |
|---|---|---|
| 1 | **大规模异构集群**：16 台同时在线、一套协议、一个指挥员 | 全景里 16 个点同时动/就位 |
| 2 | **协同封控四要素** | 通道管控（车墙）+ 动态隔离（跟踪）+ 快速围控（合围）+ 风险收敛（收口） |
| 3 | **空地协同**：天上的眼睛 → 地面的处置 | 机载画面框住目标 → 4 台车切入 |
| 4 | **讲得下去**：给评委一个 45 秒的钩子 | 让剩下的 5 分钟有人愿意听架构与算法 |

### 1.3 做到什么程度算完成

1. **素材齐**：6 段可用镜头 —— 全景 / 车墙就位 / 人流过街 / 一人脱离 / 机载 YOLO 画面 / 四车合围，1× 拍摄，能拼成一个完整故事。
2. **规模真**：16/16 在线，Pictor 上能同时看到 16 台（或就位 / 或移动）。
3. **一次跑通**：中间不做任何人工干预，85 秒跑完不出错。
4. **报得出数**：车墙成型时间、检出时延、合围闭合时间、端到端时延，四个数能写在指标页上。
5. **答得上话**：口径是"**1 个人操控 16 台**"；被问到"车为什么追不上人"时，答案是"**车不是去追，是去算**"。

### 1.4 不追求什么（先说清楚，免得越做越大）

- 不追求"真跑"动画、不做抢包道具、不演抢夺 —— 观众看到"一个人突然跑"就够了。
- 不追求车列几何严密 —— 要的是"看起来路被封了、人从两列中间过"，不做物理封堵、不验证轿车过不过得去。
- 不改 Rust、不动 `city` 场景、不上真机 —— 这一轮只做仿真里的这一段画面。

---

## 二、Step 1 — 资产步：三个人物 + 巡逻车 + 轿车 + 场馆建筑（只入库，不接线）

### 2.0 已定口径（2026-11-07 人类确认，后续步沿用）

1. **整场走真实比例**：人 1.84 m、轿车按真实乘用车、建筑按真实楼体、路面放宽到 ~8 m。
2. **UGV 放大到"长 0.8 m"**（宽约 0.55 m、高约 0.5 m）—— 不是 1.8~2 m 的真车，是"像样的巡检机器人"。
3. **车的视觉尺寸与碰撞尺寸同步放大**（碰撞在下一步改，本步只记录目标值）。
4. **找不到合规的大 UGV 模型 → 用现有 `ugv_m1.glb` 放大到 0.8 m 长**（有退路，不空着；台账注明"放大版"）。
5. **轿车 / 建筑找不到 → 就空着**（没有退路；与 UGV 不同）。
6. **「封路」= 南北两排、各 4 台均匀停在马路上**：
   - **北面 4 台**均匀停在斑马线北侧的马路上；**南面 4 台**均匀停在斑马线南侧的马路上；
   - **车与车不用贴紧**，均匀摆开即可，不追求严丝合缝、不做物理封堵；
   - 画面上要让观众看懂"车把路挡了、车辆过不去"；
   - **行人从南北两排之间（斑马线）通过**。
7. **轿车 2 款，且必须能动**：它不是静态布景，要作为**可移动的 avatar** 用（下一层接线才让它跑）⇒
   入库时按**车辆口径**归一：米制、朝 **+X**、轮胎底面 **y = 0**、单文件 glb、无骨骼动画依赖。
8. **建筑 3~5 栋，第 3 站 KayKit 可以开，楼高不设限制**：只作背景，**零碰撞**（不进体素/不进 obstacles/不加碰撞体）。
9. **雷达统一装车顶**；相机（若开）装**车头前缘**。
10. **12 台车一律不开相机**：`[camera].enabled = false`、`[detector].enabled = false`（车端没有视觉）。
11. **只有"抓人"的那台无人机开相机 + detector**，其余 3 台不开相机、不开 detector ⇒ **全场 detector = 1**。
12. **车比人跑得快**（人类 2026-11-07 定）：车 `max_speed = 2.2 m/s`（跑者 1.8）、`max_turn_rate = 180 °/s`；
    跟踪机 `max_speed = 3.0 m/s`（世界 quadrotor clamp 2.0 要放开）。
13. **车不用 `Guard`、也不用 `Follow`**（人类 2026-11-07 定）：
    - **8 台车墙**：`Goto(车位)` + `Face(朝路)` → **到位就不动**；
    - **4 台围控**：一开始在旁边**待命**（静止），事发后 `Goto(环点)` + `Face(人)` → **到位就不动**；
    - 车端**没有任何值守/警戒/追人行为**，也不需要"看见人"——人的坐标由剧本/地面站给。
14. **轿车只平移**（人类 2026-11-07 定）：两台轿车各自沿**固定朝向**直线开到"UGV 那排车的前面"停下，
    **不转向、不倒车、不掉头、不差速**；朝向在 spawn 时定死。
15. **第三个场景叫 `venue`，全视觉背景、零碰撞**：场馆、高楼、路面、斑马线一律纯 mesh；
    **`SceneDefs["venue"].obstacles = []`（不加任何障碍）**；**人一开始就出生在场馆门口外面**，不从楼里走出来。
16. **不展示地图**：Pictor 的地图/建图不作为演示内容（空图也无所谓）⇒ 体素世界 0 格不是问题。
17. **相机与录屏由人类处理**：本任务**不调机位**、不写录屏脚本；`ObserverCamera` 保持 `World.tscn` 原样。

#### 目标尺寸表（后续步都按这张表走）

| 资产 | 目标尺寸 | 依据 |
|---|---|---|
| 行人 | **1.84 m 高**（已有） | nathan 已归一；sophia/manuel 保留自然身高（±5%） |
| UGV | **长 0.8 m / 宽 ~0.55 / 高 ~0.5** | 人类 2026-11-07 定："0.8 米长就可以了" |
| 轿车 | **真实乘用车**（长 ~4.5 / 宽 ~1.8 / 高 ~1.5） | 真实比例口径；**要能当车体动起来** |
| 建筑 | **高度不设限**（背景用） | 只作背景、零碰撞，多高都行 |
| 路面 | **~8 m**（双车道） | 两辆真实轿车能并排 |

### 2.1 这一步要什么

三件事，都在"资产"层：

1. **三个人物**：nathan / sophia / manuel 都进 `assets/humans/`，并且**三个人都能走** ——
   办法是把 nathan 那条走路 clip 复用到另外两个身体上。人群才有"三种长相 / 三种身高 / 三套衣服"，
   不再是同一个人复制 N 份。
2. **一台巡逻车 + 几辆轿车**：找 **CC0 低模**的"巡逻 / 警用 / 安防"四轮车（替换现在自建的低模），
   以及 **2~3 款社会车辆（轿车）**——摆在斑马线两端"被截停"的位置上，让"封路让人过街"一眼看得懂。
   风格接受卡通。**限额搜索，找不到就保持现状**（见 §2.5 A/B）。
3. **几个大型场馆 / 建筑**：找 3~5 个体量大的建筑模型，摆在**演示区外围**当背景，让这个 128×128 的世界
   看起来像"城市里的一个路口"，而不是"一块空地"。**纯背景：零碰撞、不进体素、不影响雷达/建图/寻路**，
   也不放进 `SceneDefs`（摆位是场景那一步）。见 §2.5 C。

**本步只入库，不接线**：不改 `Pedestrian.tscn` / `WebSocketManager.gd` / `ChassisCatalog.gd` /
`Vehicle.tscn`，不创建行人场景、不改 spawn 逻辑、不改任何碰撞尺寸。接线是下一步。

### 2.2 现状（2026-11-07 实测）

| 事实 | 值 |
|---|---|
| 免费源页面 | `https://renderpeople.com/free-3d-people/` → 区块 **3D ANIMATED PEOPLE**（格式只有 C4D/MAX/MAYA/U3D/FBX/UE4，**没有 GLB**） |
| 下载直链 | `https://renderpeople.com/sample/free/renderpeople_free_animated_people_FBX.zip` |
| 外层包 | `280 263 443 B`，内含 **3 个**内层 zip + 一张优惠券 PDF |
| 已在库 | `rp_nathan_animated_003_walk`（走路，124.4 MB 内层包，glb `3 263 492 B`，88 骨、1 条 clip、2.2333 s） |
| 待入库 ① | `rp_sophia_animated_003_idling_FBX.zip`（`40 321 042 B`）：**女**，白无袖上衣 + 破洞牛仔 + 浅色鞋；**idle**（站立待机） |
| 待入库 ② | `rp_manuel_animated_001_dancing_FBX.zip`（`115 231 371 B`）：**男**，灰 T 恤 + 牛仔 + 白运动鞋；**dancing**（跳舞） |
| 免费源有没有"跑" | **没有**。三个免费动画人物只有 walking / idling / dancing 三条 |
| 骨架约定 | `rp_<name>_animated_00X_<部件>`，nathan 为 88 骨（root/hip/spine_01..03/upperarm_l/upperleg_twist_r…）⇒ 三人**很可能同构**，只需换前缀 |
| 贴图 | 三人原始贴图**全是 8192²**（manuel 的 norm 有 35.1 MB）⇒ 必须降到 nathan 的口径（dif 2048² / norm 1024²） |
| 工具链现状 | **本机有 Blender 4.5.14 LTS**：`/vepfs-mlp2/c20250205/240804016/tool/blender/blender`（包装器；实体在 `blender-4.5.14-linux-x64/`，无头 `-b` 可用，`ldd` 无缺库）。另有 Godot `4.7.2.stable.custom_build` 内含 **ufbx** 可读 FBX。**没有** assimp / FBX2glTF |
| 另外两件静态模型 | `rp_carla_rigged_001` / `rp_eric_rigged_001` **无 skin、无动画**（A-pose 静态网格）⇒ 只能当远景静态人，本步不动它们 |
| 现车模 | `assets/chassis/ugv_m1.glb`（自建低模，task_42「搜不到就自建」的产物）+ `assets/ugv_x3plus_lowpoly.glb`（备用） |
| 车的物理尺寸 | `ChassisCatalog.DEFAULTS.diff_drive_ugv`：`max_speed 1.5` / `track_width 0.25` / `height 0.25` ⇒ **约 0.3 m 宽，玩具比例** |
| 车模风格约束 | 人物是写实扫描件，车是低模 —— 天然不一致；人类 2026-11-07 定：**车走卡通低模可以接受**，不为风格统一而放弃 |

### 2.3 三个人在演示里的定位

| 人 | 体型/服装 | 动画（入库后） | 用途 |
|---|---|---|---|
| `nathan` | 已入库（走路男） | walk（原生，已有） | 主力走动人群 |
| `sophia` | 女，白上衣 + 牛仔 | **walk（复用 nathan）** + idle（原生保留） | 走动人群（差异化）/ 路边站立的人 |
| `manuel` | 男，灰 T + 牛仔 | **walk（复用 nathan）** + dancing（原生保留） | 走动人群（差异化）/ 可选"街头表演" |

> 保留原生 clip 的理由：idle 正好当"站着等人"的人（比 A-pose 静态模型自然得多），而 dancing 在"活动散场"的语境里也说得通（街头艺人）。**删掉就再也拿不回来**，成本只有几 KB。

### 2.4 做法

1. **下载与解包**
   - 外层：`unzip renderpeople_free_animated_people_FBX.zip` → `rp_sophia_…_FBX.zip` / `rp_manuel_…_FBX.zip` / `rp_nathan_…_FBX.zip` / 优惠券 PDF。
   - 内层：**成员名是反斜杠**（`tex\rp_sophia_animated_003_dif.jpg`），Linux `unzip` 解出来是单层怪文件名 ⇒ 必须用
     "`\`→`/` 归一 + 拒绝 `..` + 扩展名白名单"的方式解（nathan 那轮已踩过，见 [humans/README.md §9.2](orion-simulation-world/assets/humans/README.md#L212)）。
2. **FBX → glb**，口径**逐条沿用 nathan**（[README.md §9.3](orion-simulation-world/assets/humans/README.md#L220)）：
   米制、人朝 **+X**、脚底 **y = 0**、Y-up（glTF/Godot 轴序）、**保留 skin + animation**、根位移置零（原地循环）。
   - **工具链 A（优先，且是原始链路）**：**Blender 4.5.14 headless**
     `/vepfs-mlp2/c20250205/240804016/tool/blender/blender -b --factory-startup --python <script.py>`。
     nathan 当年就是这条链路（单位修正 / 朝向 +90° / 裁 T-pose 首帧 / 根位移置零 / 身高归一 / 导出 glb），
     复用它的做法最稳。批处理时注意 `--factory-startup`，别加载用户配置。
   - **工具链 B（备选）**：Godot 4.7 的 ufbx 导入 → `GLTFDocument.append_from_scene()` → `write_to_filesystem()`。
     只在 A 走不通时用（ufbx 对 FBX 的骨架/动画细节不如 Blender 可控），不两条并行。
3. **裁首帧 T-pose**：nathan 的源 `frame 1` 是零位 T-pose（不是步态帧），已删；sophia / manuel 要**先验有没有**，有就同样删。
4. **骨架比对与套 clip**
   - 先比对三个 FBX 的骨骼数与命名后缀（期望都是 88 骨、同后缀、只差前缀）；
   - 一致 ⇒ 把 nathan 的 walk 动作改名套到 sophia / manuel 的骨架上（Blender 里改名后赋 action，或 Godot 里换 `AnimationPlayer` 的轨道路径）；
   - 同时保留各自原生 clip（idle / dancing）作为**第二条**动画。
5. **属性归一**
   - 身高：**保留各自自然身高**（人群需要身高差），只把脚底校到 y = 0；
   - 贴图降采样到 nathan 口径：dif ≤ 2048²、norm ≤ 1024²（8192² 原图不进 glb，与 nathan 一致）；
   - 三角数记录在案（nathan 是 21 506 tri，manuel/sophia 可能更高，超过 50k 要 decimate）。
6. **入库**
   - 目录：`assets/humans/rp_sophia_animated_003_walk/`、`assets/humans/rp_manuel_animated_001_walk/`
     （目录名带 `_walk` = "走路版"，与 nathan 的命名法一致）；
   - 每个目录 **6 个文件**：`model.glb` + `model.glb.import` + `model_<name>_dif.jpg` + `.import` + `model_<name>_norm.png` + `.import`（照抄 nathan 的 6 文件约定）；
   - `LICENSE_renderpeople.txt` 不新增（三件同一套条款），但 `assets/humans/README.md` **要各加一节**：
     来源 URL / 包内成员 / 外层与内层字节 / sha256 / 处理方式（单位、朝向、身高、根位移、贴图降采样）/ 许可归属。
7. **导入验收**：`godot --headless --path orion-simulation-world --import` → 0 ERROR；两个新 glb 都能 `load()`。

### 2.5 低模资产：巡逻车 + 轿车 + 场馆建筑（CC0，限额搜索）

**为什么做**：这一段的核心画面是"**车把斑马线两头封住，人从中间过**"。要让这个画面成立，路上得
**真的有车**，四周得**真的有楼**——否则就是"几台小车停在空地上"。

- **A 巡逻车**：替换现在 0.3 m 宽的玩具比例自建低模（`assets/chassis/ugv_m1.glb`），这是我们自己的 8 台车墙/4 台围控车的外观；
- **B 轿车 2~3 款**：摆在被截停的位置（车墙外侧），让"封路"这件事一眼看得懂；**纯布景，不动、不接线**；
- **C 场馆 / 建筑 3~5 个**：摆在演示区外围当背景，把"一块空地"变成"城市路口"；**纯背景，零碰撞**。

**风格接受卡通**（人类 2026-11-07 明确：Kenney / Quaternius 那种 blocky 也可以）。

#### 1. 站点与顺序（先到先停）

| 顺序 | 站 | 找什么 |
|---|---|---|
| 1 | `kenney.nl` | Vehicle Kit / Car Kit 的**巡逻车**与**轿车 / 掀背 / SUV**；City Kit / Commercial 的**楼体** |
| 2 | `quaternius.com` | Modular / Ultimate Vehicles 的**巡逻车**与**轿车**；Simple / Modular Buildings 的**楼体** |
| 3 | `kaylousberg.itch.io`（KayKit） | 仅当 1、2 都没找到合意楼体时，找 **City Builder Bits** 的楼体（CC0） |

**只试这三站，且第 3 站只为建筑开**（车仍然只试前两站）。
不顺延到 Sketchfab / Free3D / CGTrader / TurboSquid / 任何聚合站 / 网盘 / 论坛附件。

#### 2. 次数上限（先到先停，超限即按"没找到"处理）

| 项 | 上限 |
|---|---|
| 站点数 | **≤ 3**（车只试前 2 站；第 3 站仅建筑） |
| 每站查询词 | **≤ 3**（车 2 个 + 建筑 1 个） |
| 每站结果页 | **≤ 1** |
| 每站详情页 | **≤ 6** |
| 实际下载候选 | **≤ 7 套**（巡逻车 ≤ 1 + 轿车 ≤ 3 + 建筑 ≤ 3） |
| 总用时 | **≤ 45 min** |

#### 3. 合规判据（全部满足才算"找到"）

- 许可 = **CC0**（写进台账；CC-BY 不收，避免署名工作量）；
- **车**：四轮；
  - 巡逻车：有"巡逻 / 警用 / 安防"观感（不要卡车、工程车、坦克）；**尺寸不限**（模型原始多大都行，入库记录 AABB，缩放到 0.8 m 长是下一步的事）；
  - 轿车：普通乘用车（轿车 / 掀背 / SUV），**固定 2 款**，造型或配色**互相不同**；**必须能当车体用**（见口径 #7）；
- **建筑**：**3~5 栋**楼体，**高度不设限**（只作背景，零碰撞）；
  互相不同款（办公楼 / 商场 / 高层 / 裙楼，至少两种观感）；
- 单文件 **≤ 2 MB**、**≤ 50k tris**（建筑可放宽到 **≤ 100k tris / ≤ 5 MB**）；
- 格式能进 Godot：`glb` / `gltf` / `fbx` / `obj+mtl` 任一；
- 贴图随包或可用纯色替代，无来源不明的内嵌贴图。

#### 4. 找不到时的动作（三类不同）

| 类别 | 找不到怎么办 |
|---|---|
| **巡逻车** | **有退路**：把现有 `assets/chassis/ugv_m1.glb` 放大到 **0.8 m 长**（口径 #4），在台账里注明"放大版，非新模型" |
| **轿车** | **空着**：不摆布景车辆，"封路"只靠 8 台 UGV + 镜头语言 |
| **建筑** | **空着**：世界保持现状（体素城市 + 街道视觉件），不另加楼体 |

三类共同遵守：

- 不换站（建筑最多到第 3 站）、不去 Sketchfab、不自己建模；
- 在 `tmp/task50/assets/search/notes.md` 写清：试了哪几站、查询词、结果页、超限在哪一项、为什么没有合规件；
- 在最终回复与台账里分别写一句「**本轮无 XX 资产**」（车 / 轿车 / 建筑三句分开写）；
- **本步仍算 PASS**（与 task_49 Step 3「找不到就放弃」同一口径）。

#### 5. 找到时做什么（A 巡逻车，仍然只入库）

1. 落地 `assets/chassis/patrol_car/model.glb`（或 `.gltf` + 贴图目录）；
2. `assets/chassis/README.md` 加一行：来源 URL / 资产 ID / 许可 / 三角数 / 字节 / `sha256[:16]`；
3. `--import` 0 ERROR，`load()` 成功，能实例化；
4. **不改接线**：不动 `ChassisCatalog.DEFAULTS`（`track_width` / `height` 是碰撞尺寸）、
   不动 `Vehicle.tscn` 的 `ApplyChassis`、**不改任何 scale**。

#### 6. 找到时做什么（B 轿车，纯布景）

1. 落地 `assets/props/cars/<asset_id>/model.glb`（一款一个子目录，含贴图）；
2. `assets/props/cars/README.md`（新建）建台账：来源 URL / 资产 ID / 许可 / 三角数 / 字节 / `sha256[:16]` / 建议配色；
3. `--import` 0 ERROR，三款都能 `load()`；
4. **只入库**：不放进 `SceneDefs`、不建 `.tscn`、不做摆放（摆位是场景那一步的事）；
5. 记录**每款的原始尺寸（AABB）** —— 下一步定"车多大、路多宽"要用。

#### 7. 找到时做什么（C 场馆 / 建筑，纯背景）

1. 落地 `assets/props/buildings/<asset_id>/model.glb`（一栋一个子目录，含贴图）；
2. `assets/props/buildings/README.md`（新建）建台账：来源 URL / 资产 ID / 许可 / 三角数 / 字节 / `sha256[:16]`；
3. `--import` 0 ERROR，每个都能 `load()`；
4. **零碰撞是硬要求**：不加 `CollisionShape3D`、不进 `SceneDefs.obstacles`、不进 `GridMap`；
   验收时要能证明"雷达扫不到它"（LiDAR 只扫碰撞层 1|2|4）；
5. **只入库**：不摆位、不建 `.tscn`、不写进任何场景；
6. 记录**每栋的原始尺寸（AABB）**与**三角数** —— 下一步按 128 m 世界定缩放要用。

> **为什么不顺手改尺寸**：换了大车模却不配套改碰撞体，会出现"看着撞上了、其实穿过去"，比不换更糟。
> 视觉尺寸与碰撞尺寸的取舍、建筑摆在哪个位置、要不要缩放，都是**下一步**的事。

### 2.6 验收（现象级）

- `--import` 退出码 0，日志零 `ERROR`；两个目录各 **6 个文件**且 `.glb` 与 `.import` 成对；
- 两个新 glb 在 Godot 里能实例化，`AnimationPlayer` 里能看到 **≥2 条** clip（walk + 原生）；
- 播 walk 时：脚底贴地（不悬空 / 不陷地）、朝 +X、原地循环不漂移；
- 三个人站在一起，身高/服装/朝向肉眼可分（截图存档）；
- **巡逻车**：找到 ⇒ `assets/chassis/patrol_car/` 有文件 + `.import` + 台账一行 + 能 `load()`；
  没找到 ⇒ `tmp/task50/assets/car/notes.md` 非空且写清超限项与"保持现状"；
- **轿车**：**2 款**（口径 #7）⇒ `assets/props/cars/<id>/` 各有文件 + `.import` + 台账 + **AABB 与地面高度**（让它能当车体）；
  只找到 1 款 ⇒ 登记 1 款并注明；一款都没有 ⇒ 同一份 `notes.md` 写清超限项与"本轮无社会车辆布景"；
  一款都没有 ⇒ 同一份 `notes.md` 写清超限项与"本轮无社会车辆布景"；
- **建筑**：目标 3~5 栋，**至少 2 栋**即可推进 ⇒ `assets/props/buildings/<id>/` 各有文件 + `.import` + 台账（**不设高度门槛**）；
  且验证过「**雷达扫不到**」（无碰撞体、不在 `SceneDefs.obstacles`）；一栋都没有 ⇒ 同一份 `notes.md` 写清并注明"本轮无城市背景"；
- **找到与没找到都算 PASS**，但必须写清楚是哪种；
- `city` 场景零回归（格数与现象不变）。

### 2.7 本步不做

- 不接线：不改 `Pedestrian.tscn` / `WebSocketManager.gd` / `ChassisCatalog.gd` / `Vehicle.tscn`，不建新行人场景；
- 不改造 `carla` / `eric`（静态模型保持原样）；
- 不做跑步动画（免费源没有；要跑另开，见 §2.8）；
- 不引任何聚合站 / 转载站 / 网盘（车的搜索也只限 Kenney / Quaternius 两站）；
- **不改车的碰撞尺寸、不改任何 scale**（换车模之后"视觉 vs 碰撞"的取舍是下一步）；
- **轿车只入库**：不摆位、不进 `SceneDefs`、不建 `.tscn`（摆放是场景那一步）；
- **建筑只入库，且零碰撞**：不加 `CollisionShape3D`、不进 `SceneDefs.obstacles` / `GridMap`、不摆位、不建 `.tscn`；
- 不动 `tmp/` 之外的下载缓存（证据留 `tmp/task50/assets/`，不入库）。

### 2.8 坑与风险

| # | 风险 | 说明 / 处置 |
|---|---|---|
| R1 | **骨架 rest pose 不一致** | 三人同系列但 T-pose/A-pose 可能不同 ⇒ 套 clip 会扭曲。先拿 sophia 单独验；不行就退回"只用各自原生动画"（nathan 当唯一走路的人） |
| R2 | **FBX 转换链** | 本机有 Blender 4.5.14（`/vepfs-mlp2/c20250205/240804016/tool/blender/blender`），**优先用它**（nathan 原链路）；Godot ufbx 只作备选。两条不并行 |
| R3 | **贴图太大** | 原始 8192²、manuel norm 35 MB。必须降到 2048/1024，否则单个 glb 会到几十 MB，16 台同屏直接崩 |
| R4 | **根位移** | 走路 clip 带根位移会导致人从胶囊里"走丢"。套完必须把 root 的位移通道置零（nathan 的做法） |
| R5 | **反斜杠** | 内层 zip 的 `tex\…` 名称，`unzip` 会解出怪文件、贴图 relink 失败（nathan 已踩） |
| R6 | **许可** | 条款覆盖"可视化/商业展示"，**不覆盖**再分发网格与训练模型。本演示只渲染，OK；但不要把 glb 单独对外发布 |
| R7 | **`.import` 与贴图必须成对提交** | 只提交 glb 会让干净检出时 `Cannot open file …_dif.jpg`（E28 同款） |
| R8 | **manuel 是跳舞** | 若套 walk 失败，他在"散场"里只能当街头表演；镜头要提前想好，别到拍的时候才发现没有走路的人 |
| R9 | **车模风格突兀** | 卡通低模 + 写实扫描人物同框会违和。处置：车放中远景、别给近景特写；或给车统一纯色涂装压掉卡通感。**风格不可兼得时，优先"封路能看懂"，不优先"写实"** |
| R10 | **限额被突破** | 一旦"再看一个站""再下一套"，这一步就无底洞。处置：**上限写死在 §2.5 表里，超一项就停**，停完写 `notes.md`，本步按 PASS 收 |
| R11 | **车模尺寸与碰撞脱节** | 换了大车模但碰撞仍是 0.25 m —— 视觉上撞上了、物理上穿过去。处置：本步**明确不改碰撞**，把它登记成下一步的第一件事 |
| R12 | **换车前后尺度不一致** | 现在是 0.3 m 的车 + 4 m 的路；口径定稿后是 0.8 m 的车 + 8 m 的路 + 真实轿车。**本步只登记各模型的原始 AABB、不动任何 scale**，比例换算留到下一步，避免半新半旧的中间态 |
| R13 | **建筑一撞就穿** | 建筑零碰撞 ⇒ 车/人可以直接穿过去。因为只当背景、且摆在演示区外围，正常镜头碰不到；但**镜头万一扫到穿模就是硬伤**。处置：摆位时必须让建筑整体落在**车与人的活动范围之外**，并留 ≥ 5 m 余量 |
| R14 | **建筑挡住镜头/遮挡演示** | 观察相机是高位俯视，建筑一高就可能挡住宅区。处置：摆位时逐栋核对俯视视线；必要时把建筑压扁或后移（摆位是下一步） |
| R15 | **建筑风格与体素世界冲突** | 我们世界是体素+写实扫描人，楼是卡通低模 ⇒ 三种观感同框。处置：同 R9，接受"看得懂优先"；楼放远景、别给近景 |
| R16 | **相机/雷达要重标的连带成本** | 车从 0.3 m 放到 0.8 m，相机与 LiDAR 安装高度全变。detector 的 `mount_*` / `fov` / `ground_z` 与"可视 0.33~7.7 m"都是按 `cam_z=0.135` 标的，不重标则 Guard 的 3 m 警戒区、Follow 的 2.4 m 距离带全部失真。**本步不碰**，但必须在下一步的第一批任务里 |
| R17 | **轿车要动 ⇒ 模型口径必须对** | 若把一款"布景版"模型直接接成车体，会因为朝向/贴地/单位不对而返工。处置：入库时就按口径 #7 归一（米制、朝 +X、轮底 y=0、单文件 glb），并在台账里写明 AABB 与地面高度 |
| R18 | **楼高不设限 ⇒ 体积失控** | 背景楼没有高度限制，可能找到 60 m、几十万面、二十 MB 的模。**tris ≤ 100k / 单文件 ≤ 5 MB 这两条上限仍然生效**，超了就换一款或降面；楼高本身不限 |

### 2.9 待人类确认

> 比例、封路形态、轿车数量与"能动"、建筑站数与高度，均已于 2026-11-07 定稿（见 §2.0 #1~#8）。
> **本步无待定项。**

---

## 三、Step 2 — 新车落地：碰撞 / 雷达 / 相机 / 速度

### 3.1 这一步要什么

把 Step 1 拿到的 **0.8 m 巡逻车**从"一个模型"变成"一台能跑的车"：按新尺寸重算四样东西 ——
**碰撞体、雷达安装位姿、相机安装位姿、速度上限**。

**全部落在 Godot 侧 + sim config**，**不改 Rust**（真车参数一个都不动）。

### 3.2 现状（代码事实，2026-11-07 核）

| 事实 | 位置 |
|---|---|
| 底盘默认参数 | `ChassisCatalog.DEFAULTS.diff_drive_ugv`：`max_speed 1.5` / `max_turn_rate 90` / `wheel_base 0.30` / `track_width 0.25` / `height 0.25` |
| **碰撞只有一处** | `DiffDrive.tscn`：四个轮子是**纯视觉**，不参与物理；碰撞形状由 `Vehicle.SetBodySize()` 定尺寸（`ChassisModel.ApplyBodySize` → `Vehicle.SetBodySize`，且"原点 = 底面中心"由 Vehicle 抬升，底盘不自己写 y） |
| 雷达由 sim 配置装配 | `[[sim_world.sensor]]`（`slot` / `sensor`）+ `[sim_world.sensor.mount]`（`x` / `y` / `z` / `yaw`，可带 `pitch`）+ `[sim_world.sensor.params]`（`range` / `rays` / `rate_hz`） |
| 相机 9 键 | `[detector.camera]`：`mount_x` / `mount_y` / `mount_z` / `mount_yaw_deg` / `mount_pitch_deg` / `fov_deg` / `fov_axis` / `ground_z` / `max_range_m` |
| 相机默认值 | `mount (0.20, 0, 0.045)`、`yaw 90°`（前视）、`pitch −15°`、`fov 70°`（vertical）、`max_range 12 m` |
| 速度两处 | `[sim_world.chassis_params].max_speed`（世界 clamp）+ `[chassis].cruise_speed`（决策侧巡航值） |
| 现在的相机标定 | 真车按 `cam_z = 0.135 m` / `pitch +1.7°` 标，可视地面 **0.33~7.7 m**（task_48 §1.1）；sim 侧是另一套默认值 |

### 3.3 做法（五条必须一起改）

1. **车体尺寸推导**（长 0.8 m 为基准，按外观比例）：
   - `wheel_base ≈ 0.50 m`、`track_width ≈ 0.44 m`、`height ≈ 0.50 m`（**执行时按实际模型 AABB 微调**）；
   - 写进 `[sim_world.chassis_params]`，与模型外观逐项对齐。
2. **碰撞体**：`SetBodySize` 用上面的 `track_width` / `height`；验收标准是"车贴着墙停时，视觉与墙接触，不悬空、不嵌进去"。
3. **雷达装车顶**（口径 #9）：`x ≈ 0.10`（车头方向）、`z ≈ 0.30~0.35`（原来 0.2）、`pitch 0`；
   `range 12` / `rays 360` / `rate_hz 10` 不变。**扫的是 2D 平面**，平面高度变了能扫到的墙层也变 —— 要重新确认"两侧墙/场馆扫得到"。
4. **相机：车不开**（口径 #10）⇒ 本步**不做车的相机标定**（`[detector.camera]` 那 9 个键对车一律不生效）。
   若将来要开，位置按口径 #9 装**车头前缘**（`mount_z` 会从 0.045 抬到 ≈ 0.45、`pitch` 需重算）——**本步不做**。
5. **速度放开**（口径 #12）：
   - `[sim_world.chassis_params] max_speed`：**1.5 → 2.2**（跑者 1.8，车要抢在前面）；
   - `max_turn_rate`：**90 → 180 °/s**；
   - `[chassis] cruise_speed`：**0.45 → 2.0**；
   - 跟踪机：世界 `quadrotor_uav` 的 `max_speed` 从 2.0 放开到 **3.0**（经 `chassis_params` 覆盖，不改 `ChassisCatalog.DEFAULTS`）。

### 3.4 验收（现象级）

- 车**贴地**：不悬空、不陷地，四轮着地；
- 碰撞与外观一致：把车开到墙边停住，视觉上"贴住"而不是"隔 20 cm"或"穿进去"；
- 雷达：日志能看到两侧墙/场馆被扫到（不是空白）；
- **相机/检测**：12 台车 + 3 台巡逻机的 `[detector]` 全关（日志无它们的 detector 行）；
  **只有跟踪机**能出 `[detector] → 世界=(x,y)`，且在 **3~8 m** 处稳定判"采纳"（不是清一色"拒答"）；
- 速度：`[Sim] UGV 位置` 差分 ≈ **2.2 m/s**，且能在以 1.8 m/s 跑的演员抵达前**抢先占位**（不是追人）；`omega` 实测可达 ≈ π rad/s；
- 8 台并排不互相挤（间距按 0.55 m 车宽重排，见 §3.3 #1 的尺寸表）。

### 3.5 本步不做

- **不改 Rust**：`pleiades-ugv/src/config.rs` 里的 `cam_z` / `pitch` / 相机内参是真车标定，本轮不动；
- 不改 `Mission` / 行为树 / 执行器 / 急停逻辑；
- 不改 detector 算法与投影公式；
- 不做车墙摆位与班组编排（那是场景步）；
- 不动 `city` 场景。

### 3.6 坑与风险

| # | 风险 | 处置 |
|---|---|---|
| S1 | **只改外观不改碰撞** | 会"看着撞上了、其实穿过去"。碰撞与外观**同一次改**，验收专门看"贴墙" |
| S2 | **雷达抬高后扫不到矮东西** | 2D 雷达是**一个平面**：抬到 0.35 m 就扫不到 0.3 m 高的路缘/矮墙。要么降低、要么接受"只扫墙面" |
| S3 | **相机抬高后 pitch 不对** | pitch 偏一点，人的框底边就不在画面下半部 → 投影**全部拒答**（现象是"看得见人但系统不用"）。改完必须实测 3~8 m |
| S4 | **8 台同屏 + 4 路 detector 的帧率** | 车变大、贴图变大，软渲染更容易掉帧。按 task_49 Step 7 的处置表走（关 PBR / 640×480） |
| S5 | **底盘参数与外观"两套真相"** | `chassis_params` 改了一处、模型缩放改了另一处，就会对不上。**统一以 §3.3 的尺寸表为唯一来源** |
| S6 | **提速后执行器阈值失真** | 死区 8° / 弧线 12° / `PASS_THROUGH_M 0.5 m` 都是按 0.45 m/s 定的；2.2 m/s 时每拍（50 ms）走 11 cm、弧线半径 `R = v/ω = 2.2/π ≈ 0.70 m`，比车长 0.8 m 还小 ⇒ 可能画圈/过冲。**改完必须复跑一次直线与 8°~12° 弧线**，必要时只调 `PASS_THROUGH_M` 与死区，不动树 |
| S7 | **车端无视觉 ⇒ 一切靠"给人坐标"** | 车不用 `Guard`/`Follow`（口径 #13），只做 `Goto` + `Face`。所以**围控环点与"朝人"的坐标必须由剧本/地面站算好**，人一动就要重发 `[Goto, Face]`；**不能**指望车自己发现人。目标一停，车也就停——这正是我们要的"到位不动" |

### 3.7 待人类确认

> 雷达位置、相机归属、速度数值均已于 2026-11-07 定稿（见 §2.0 #9~#13 与 §3.3）。
> **本步无待定项。**

---

## 四、Step 3 — 模拟世界：行人可选模型 + 轿车底盘

### 4.1 这一步要什么

两件事，**都只动 `orion-simulation-world`（Godot 侧），不碰 Rust**：

1. **修**：行人能**选模型**（nathan / sophia / manuel 三选一），这样人群才有三种长相；
2. **加**：一个 **`sedan` 底盘**，让两台轿车能"沿直线开到车墙前面停下"（口径 #14：**只平移**）。

### 4.2 现状（代码事实，2026-11-07 核）

| 事实 | 位置 |
|---|---|
| 行人 Body 硬编码 nathan 的 glb | `src/scenes/props/Pedestrian.tscn` 的 `[node name="Body"]` + `ext_resource … rp_nathan_animated_003_walk/model.glb` |
| 底盘目录只有三个 | `ChassisCatalog.SCENES`：`diff_drive_ugv` / `quadrotor_uav` / `pedestrian`；`DEFAULTS` 各带参数 |
| 未知 chassis 直接拒连 | `WebSocketManager.gd:555`：`_CatalogHas()` 不通过 → `_CloseLink(..., "unknown chassis")` |
| `params` 已能透传 | `chassis_hello.params` → `ApplyChassis(chassis_name, params, scene)`（`WebSocketManager.gd:615`）→ `ChassisModel.Configure` |
| 行人支路是单独分流的 | `kind = "pedestrian"` → `Pedestrians` 容器、不占 `_PickSpawn` 排位（`WebSocketManager.gd:563`）；其余 `kind = "robot"` 进 `Vehicles` |
| 动画播放器是"找出来的" | `Pedestrian._Find_Anim_Player()` 用 `find_child("AnimationPlayer", true, false)`，**不绑节点名**；`Play_Walk()` 取 `get_animation_list()[0]` |
| 碰撞只有一处 | 各底盘 `Configure → ApplyBodySize → Vehicle.SetBodySize`；轮子/外观不参与物理 |

### 4.3 做法 A — 行人可选模型（修）

1. **参数**：`chassis_hello.params.model ∈ {"nathan", "sophia", "manuel"}`，缺省 `"nathan"`（老客户端不传 → 行为不变）。
2. **实现**：`Pedestrian.gd` 里 `preload` 三个 glb 的 `PackedScene`；`Configure()` 读 `params.model`，
   与当前 Body 不同就**换实例**（幂等：同名不重建）；换完重新 `_Find_Anim_Player()`。
3. **clip 约定**：行人仍播 `clip[0]` ⇒ **入库时必须保证 `clip[0] = walk`**（sophia/manuel 的 idle/dancing 排在后面）。这条写进 Step 1 的入库验收。
4. **兜底**：未知模型名 → `push_warning` + 回落 `nathan`（不拒绝 spawn，避免演示中途掉线）。

### 4.4 做法 B — 轿车底盘（加，只平移）

1. **登记**：`ChassisCatalog.SCENES["sedan"] = "res://src/scenes/chassis/Sedan.tscn"`；
   `DEFAULTS["sedan"] = { max_speed, max_turn_rate: 0, body_length ≈ 4.5, body_width ≈ 1.8, height ≈ 1.5 }`
   （真实乘用车尺度，口径 #1）。
2. **`Sedan.gd extends ChassisModel`**：
   - `SetCommand` **只认 `v`**（`clampf(v, 0, max_speed)`），`dir` / `action` 一律忽略（口径 #14：不转向、不倒车）；
   - `_physics_process` 里按当前朝向 `move_and_slide()`；**无加速度斜坡**（和行人一致，`v=0` 当帧归零）；
   - 车身尺寸走 `ApplyBodySize`，碰撞盒只有一个。
3. **`Sedan.tscn`**：`Node3D` + `Sedan.gd`，`Body` 留空（模型在 `Configure` 里按 `params.model` 装两款之一）。
4. **朝向**：spawn 时用 `params.heading_deg`（世界系，0 = +X 东）定死，缺省朝北/沿路。
5. **容器**：轿车用 `kind = "robot"` + `chassis = "sedan"` → 进 `Vehicles`（不走行人支路）。

### 4.5 验收（现象级）

- 三个人物都能 spawn 出来，**长得不一样**；不传 `model` 时还是 nathan（向后兼容）；
- 三个模型的 `clip[0]` 都是 walk（播出来是走路，不是站着或跳舞）；
- `chassis = "sedan"` 能 spawn，不报 `unknown chassis`；
- 手动发一帧 `motion_cmd {v: 1.0}`：车沿**固定朝向**直线前进；`{v: 0}` 立刻停；发 `dir` 无效（不转向）；
- 两台车能各走各的直线、互不干涉（路线错开）；
- `city` 场景零回归（格数与现象不变）。

### 4.6 本步不做

- 不改 Rust（`sim` / `ugv` / `base` 一行不动）；
- 不做轮子转动/差速/倒车/转向动力学；
- 不做轿车驱动脚本（本步只保证"能 spawn、能收 `motion_cmd`、能直线走"；驱动随人群/剧本脚本一起做）；
- 不摆位、不进 `SceneDefs`（摆放是场景步）；
- 不给车开相机（口径 #10）。

### 4.7 坑与风险

| # | 风险 | 处置 |
|---|---|---|
| T1 | **`clip[0]` 不是 walk** | sophia 的原生 clip 是 idle、manuel 是 dancing；入库时若排在前面，行人会"站着滑行"。**Step 1 入库验收必须钉死 `clip[0] = walk`** |
| T2 | **换模型后动画播放器找错** | 换实例后必须重新 `_Find_Anim_Player()`；`Pedestrian.tscn` 的注释也警告过"别另建空 AnimationPlayer" |
| T3 | **轿车 4.5 m 长 vs 8 m 路** | 两台车一停就占掉半条路，与 UGV 车墙的位置关系要留够；摆位时按真实尺寸算 |
| T4 | **轿车与 UGV 的碰撞层** | 若 sedan 的 mask 不含车的层，会互相穿过——"被拦住"就不成立。摆位前先定 mask |
| T5 | **`kind` 与 `chassis` 不匹配** | `kind="pedestrian"` 但 `chassis="sedan"` 会被 warn 后按 robot 处理（`:563-567`）。轿车**只用** `kind="robot"` |

---

## 五、Step 4 — 世界搭建：第三个场景 `venue`

### 5.1 这一步要什么

在现有两个场景（`city` / `corridor_indoor`）之外，加**第三个场景 `venue`**：一条 8 m 宽的路 + 斑马线 +
场馆 + 几栋背景高楼，作为整段演示的舞台。

**按口径 #15**：**全部是视觉件、零碰撞**（场馆和楼都不进体素），`obstacles` 留空；人**出生在门口外面**，
不从楼里走出来。世界尺度仍锁 128×128×32。

### 5.2 现状（代码事实）

| 事实 | 位置 |
|---|---|
| 场景数据表已有两条 | `SceneDefs.SCENES`：`city`（15 条 AABB）/ `corridor_indoor`（8 条 AABB） |
| 一份 `World.gd` 跑所有场景 | `World.gd` 的 `_ready()` 先解析场景名（导出属性 → `ProjectSettings` → `city`），再取 `_def` |
| 观感档两档 | `WorldLook.gd` 的 `Apply_City` / `Apply_Indoor` |
| 视觉件的既有做法 | `StreetLayout`（路面/标线）、`Facade`（立面）、`Interior`（室内）都是**纯 mesh、零碰撞** |
| 入口 `.tscn` | `World.tscn`（city）/ `WorldIndoor.tscn`（corridor_indoor），只差 `scene_name` |
| 观察相机 | `ObserverCamera` 在 `World.tscn` 里给机位（city 是高位俯视） |

### 5.3 场地平面（示意值，执行时按 0.5 m 对齐微调）

```text
世界 0..128（x 东 / y 南 / z 上，世界坐标）

           y=128
   ┌─────────────────────────────┐
   │  背景楼 B3/B4（纯视觉）        │
   │                             │
   │        ┌──────────────┐     │
   │        │  场馆（纯视觉）│     │  x∈[70,100] y∈[70,100]
   │        │   门洞在西门  │     │  门洞 x=70, y∈[80,84]
   │        └───────┬──────┘     │
   │            出生点 (68, 82)    │
   │  ═══════════════╪══════════   │  道路 x∈[56,64]（8 m 宽），南北贯通
   │   ▲ 车墙北排 y≈54（4 台）      │
   │   ▓ 斑马线 y∈[58,66]          │  6 条白条，横跨 x 56..64，纯 mesh
   │   ▼ 车墙南排 y≈70（4 台）      │
   │                             │
   │  背景楼 B1/B2（纯视觉）        │
   └─────────────────────────────┘
           y=0
```

- **道路**：`x ∈ [56, 64]`（8 m 宽，口径 #1），南北贯通 `y ∈ [0,128]`；
- **斑马线**：`y ∈ [58, 66]`（8 m 宽），6 条白色条纹，**纯 mesh 零碰撞**（顶面 z=0.01，与街道同一手法）；
- **车墙位**：北排 4 台 `y ≈ 54`、南排 4 台 `y ≈ 70`，每排 `x = 57 / 59 / 61 / 63`（间距 2 m，均匀摆开，口径 #6）；
- **行人通道**：两排之间 `y ∈ [58, 66]`，东西向穿行；
- **场馆**：`x ∈ [70,100]`、`y ∈ [70,100]`，**BoxMesh 外墙 6~8 m 高**（纯视觉，零碰撞），西墙 `x=70` 留 `y ∈ [80,84]` 的门洞（纯视觉，不做通行判定）；
- **出生点**：`(68, 82)` —— 就在门洞外面，人一出生就在门口（口径 #15）；
- **背景楼 3~5 栋**：西侧 `x ∈ [20,50]` 两栋、北侧 `y ∈ [100,126]` 两栋、东侧 `x ∈ [104,124]` 一栋；**全部纯视觉、零碰撞**，且**不挡俯视镜头**。

### 5.4 做法

1. **`SceneDefs.gd` 加 `venue` 一条**（字段照 `city` 的结构，新增一个 `buildings`）：
   ```gdscript
   "venue": {
       "name": "venue",
       "obstacles": [],                    # 口径 #15：全视觉，零碰撞
       "spawn": Vector3(68.0, 82.0, 0.0),  # 门口外面
       "spawn_clear_radius_m": 1.5,
       "spawn_clear_height_m": 2.5,
       "air_wall_height_m": 32.0,
       "props_street": false,              # 不建 city 的街道
       "props_facade": false,              # 不建 city 的立面
       "look": "venue",                    # 新观感档
       "ceiling": false,
       "ceiling_z_m": 0.0,
       "buildings": [                      # 新增字段：背景楼（纯视觉）
           # {"model": "res://assets/props/buildings/<id>/model.glb",
           #  "pos": Vector3(30.0, 110.0, 0.0), "rot_y_deg": 0.0, "scale": 1.0},
       ],
   },
   ```
2. **`World.gd`**：`_def.look == "venue"` 时调 `Plaza.Build_All(self, _def)`（与 `Interior.Build_All` 同构）；
   `buildings` 里每栋按 `model/pos/rot_y_deg/scale` 实例化到一个 `Buildings` 容器（**不加碰撞体**）。
3. **`Plaza.gd`（新，纯视觉件）**：
   - `Floor`：路面（8 m 宽，DarkGray，顶面 z=0.01，避免与 `Ground` z-fighting）；
   - `Zebra`：6 条白条（`BoxMesh` 或 `SurfaceTool`，顶点坐标取 0.5 整数倍）；
   - `Sidewalk`：路两侧各 2 m 宽的人行道（略高于路面 0.05 m 的薄片）；
   - `Venue`：场馆外墙（4 面，留西门洞）+ 门口雨棚（可选）；
   - `Buildings`：按 `def.buildings` 实例化背景楼。
   - **零碰撞**：全部 `MeshInstance3D`，不加 `CollisionShape3D`、不写 `GridMap`。
4. **`WorldLook.gd` 加 `venue` 档**：`Apply_Venue(world)` —— 保留天空与太阳（这是室外白天），只调环境色/雾/曝光，
   与 `Apply_City` 的差别放在"冷暖与曝光"上，不重写整套。
5. **`WorldVenue.tscn`（新）**：复制 `World.tscn` 的节点树，只加 `scene_name = "venue"`；
   **`ObserverCamera` 一字不动**（口径 #17：机位与录屏由人类处理）。
6. **`COMPONENTS.md`**：目录树、`.tscn` 清单、新增一节「多场景：venue」、运行命令补一条。

### 5.5 验收（现象级）

- `godot --headless --path orion-simulation-world --check-only --script res://src/world/Plaza.gd` 无 Parse Error；
- 起 `WorldVenue.tscn`：日志 `voxel world built: 0 cells`（obstacles 为空）且 **0 ERROR**；
- 场景里能看到：**8 m 宽的路 + 6 条白条 + 两侧人行道 + 场馆 + 背景楼**（机位与构图由人类定，口径 #17）；
- 车/人**可以穿过**场馆与楼（零碰撞，属预期）—— 但剧本不会走到那里；
- `city` 与 `corridor_indoor` **零回归**（格数与画面不变）。

### 5.6 本步不做

- 不给场馆/楼加碰撞（口径 #15）；不加体素；
- 不改世界尺度（128×128×32）；
- 不做多场景同时存在、不做运行期切换；
- 不做车辆/行人摆位（那是剧本步）；
- 不改 `city` / `corridor_indoor` 的任何数值。

### 5.7 坑与风险

| # | 风险 | 处置 |
|---|---|---|
| V1 | **零碰撞 ⇒ 车/人能穿场馆** | 属预期；但镜头若扫到穿模就是硬伤。剧本路线必须完全避开场馆与楼 |
| V2 | **体素世界 0 格 ⇒ 地图是空的** | 口径 #16：**不展示地图**，所以无所谓。雷达在空场景里只会扫到人和车（它们是运行时的动态体），不产生静态地图——这是预期 |
| V3 | **背景楼可能挡视线** | 楼高不设限（口径 #8）。本任务不调机位（口径 #17），只保证楼都在**演示区外围**（`x<50` / `x>100` / `y>100`）；最终构图由人类录屏时处理 |
| V4 | **路面与 `Ground` z-fighting** | 路面顶面固定在 `z = 0.01`（与 StreetLayout 的 `ROAD_TOP_Z` 同一手法），别与 `Ground` 共面 |
| V5 | **斑马线条纹被当成体素** | 斑马线必须是 `MeshInstance3D`；一旦进 `obstacles` 就会挡车、进雷达、进 D\* |
| V6 | **`buildings` 字段是新增的** | `World.gd` 与 `Plaza.gd` 都要按"缺省空数组"处理，`city` / `corridor_indoor` 不带这个键也必须能跑（向后兼容） |

### 5.8 待人类确认

> 场景名、零障碍、零碰撞、不展示地图、机位不调，均已于 2026-11-07 定稿（见 §2.0 #15~#17）。
> **本步无待定项。**

---

## 六、Step 5 — 验证：这个场景能不能跑起来

### 6.1 这一步要什么

**只验"能跑"，不演剧本。** 把 Step 1~4 的成果拼起来，确认：

- 新场景 `venue` 能起、不报错；
- **16 台设备（12 车 + 4 机）** 全部能上线、能收到指令、能走；
- **2 台轿车**能 spawn、能直线走到位停车；
- **3 个行人模型**都能 spawn、都能走（不是站着滑行）；
- 同机跑这么多 avatar **不掉线、不 panic、帧率还撑得住**。

车墙到位、围控合围、跑者脱离这些**编排**都不在本步（那是剧本步）。

### 6.2 前置

| 依赖 | 说明 |
|---|---|
| Step 1 | 三个行人模型 + 巡逻车模 + 2 款轿车 + 背景楼，全部入库且 `--import` 0 ERROR |
| Step 2 | 新车 `chassis_params`（0.8 m 车身 / 雷达车顶 / 速度 2.2 / 转向 180°/s）已生效 |
| Step 3 | `params.model` 可选行人模型；`sedan` 底盘已登记且只认 `v` |
| Step 4 | `SceneDefs["venue"]` + `WorldVenue.tscn` + `Plaza.gd` 已落地 |

### 6.3 分两级验（先小后大）

| 级 | 规模 | 验什么 | 用时 |
|---|---|---|---|
| **L1 冒烟** | 1 车 + 1 机 + 1 轿车 + 2 行人 | 场景起得来、四类 avatar 各一个能 spawn、能走、能停 | ~10 min |
| **L2 全场** | 12 车 + 4 机 + 2 轿车 + 10 行人（**28 个 avatar**） | 全量上线、位姿频率、帧率、无掉线 | ~30 min |

**L1 不过就不进 L2**（否则排查面太大）。

### 6.4 L2 的检查清单（现象级，按级排序）

**A 致命（任一不满足 = FAIL）**

1. 世界起得来：`WorldVenue.tscn` 日志 **0 ERROR**（`grep -nE "SCRIPT ERROR|Parse Error|ERROR"` 零命中）；
2. 28 个 avatar **全部注册**：Pictor 列表 28 行 / 世界日志 `vehicle spawned` 与 `pedestrian spawn` 计数对得上；
3. 无 panic、无进程退出：16 个 sim 进程 + 2 个轿车客户端 + 行人客户端全程存活；
4. 无反复掉线：日志里 `连接建立` / `断开` 不出现抖动循环；
5. **行人播的是走路**（不是站着滑行、不是跳舞）—— 三个模型都要逐个确认。

**B 重要**

6. 位姿频率 ≥ 9 Hz（`[scale] pose_hz` 或日志差分）；
7. 世界帧率：软渲染下 ≥ 15 fps（低于 10 fps 基本没法录）；
8. 轿车：spawn 朝向正确、直线前进、`v=0` 立刻停、**不转向**；
9. 车辆：贴地、不悬空、转向能到 180 °/s（`omega` 实测）；
10. 行人：从出生点 (68,82) 走到斑马线，全程不卡住（`pos` 单调变化）。

**C 观感（记录，不判 FAIL）**

11. 有没有穿模（车穿人、人穿车、轿穿场馆）；
12. 三个行人的身高/衣服是否肉眼可分；
13. 相机/构图（口径 #17：人类自己处理，本步只记录截图）。

### 6.5 判据

- **PASS** = A 全过 + B 至少 8/10；
- **FAIL** = A 任一不过，或"车/人完全不动"、"任务不进队"、"日志报错/panic"、"连接掉线"；
- 不设数值门槛的部分（C）只记录，不进判据。

### 6.6 收尾

- kill 全部进程（16 sim + 2 sedan + 行人 + 世界 + Pictor）；
- `ss -ltnp` 确认 **9090~9105 / 9999 / 9010** 及行人端口全空；
- 证据落 `tmp/task50/s5/`（日志摘录 + 截图 + 一行结论），不入库。

### 6.7 坑与风险

| # | 风险 | 处置 |
|---|---|---|
| V1 | **28 个 avatar 同机抢资源** | 先 L1 再 L2；L2 若帧率 < 10 fps，按 task_49 Step 7 的处置表关 PBR / 降窗口，**不改演示内容** |
| V2 | **同机 mDNS 不可靠** | 16 台必须走**静态 `bootstrap_peers`**（task_49 Step 7 已证） |
| V3 | **端口冲突** | 9090~9105 预先分配并写进 `ports.json`；起之前先 `ss -ltnp` 看有没有上一轮残留 |
| V4 | **行人 spawn 卡在场馆里** | 出生点 (68,82) 在门洞外；若世界把 spawn 点判成"在障碍里"会 push，注意日志告警 |
| V5 | **轿车 4.5 m 长挡路** | L2 时轿车会占掉半条 8 m 路，正好是"被拦停"的观感；但要注意别把车墙位堵死 |
| V6 | **零碰撞 ⇒ 车/人穿场馆** | 属预期（口径 #15）；只要剧本路线不经过场馆，镜头就看不到 |

---

## 七、Step 6 — Pictor 编队（formation）：assist 只发给围控编队

> 状态：**已实现（2026-11-08，review PASS；Pictor 工作树未提交）**
> 范围：**只改 Pictor（Godot 地面站）**。不改 ORION 协议、不改车端 / kernel / orion Rust。

### 7.0 背景：现状会把车墙一起拉走

task_41 落地的 `CoopController`（`Pictor/src/cooperation/coop_controller.gd:181-222`）收到 UAV 的
assist request 后：

1. 取 **VehicleRegistry 里所有在线 car**（`node_type == "car"` 且位姿非 `(0,0)`，`_online_cars()` `:162-178`）；
2. 排除请求者（`:191`）；
3. `N = 剩余车数` → 3 m 环上 N 个等角点 → 最近一对一 → **逐车一帧 `TASK_SET`**。

task_50 的场上在线车是 **12 台**（8 车墙 + 4 围控）。assist 一来，`N = 12`，**8 台车墙车也会收到
`TASK_SET`**；而 `TASK_SET` 是**整队替换**语义（车端 `apply_auto_cmd` → `Set` → reset），
车墙的 `Goto(车位) + Face(路)` 会被整个顶掉 ⇒ **车墙当场散架，全往人那边开**。

结论：编队是**地面站侧**概念，车端无感；不引入新协议字段，也不让车知道自己是哪个编队。

### 7.1 目标（可机判）

1. 所有 device 注册时**默认无编队**（`formation == ""`）。
2. `VehicleRegistry` 成为编队的**唯一数据源**：join / leave / query / members。
3. `WsCommandServer` 新增 `formation` 命令（join / leave / list），可用
   `Pictor/tool/pictor_ws_ctl.py` 从外部 WS 驱动。
4. `CoopController` 只对 `containment` 编队内的在线 car 派发；**零成员 → warn + 一帧不发**；
   **不回退全员**。
5. 除 `containment` 之外，其他编队 id 只是被记录，不改变任何行为（留给后续车墙/巡逻编队用）。
6. **零协议改动**：`msgid 7` 仍是 `x f32 | y f32`；`TASK_SET` 仍是 1 + Σlen；车端一行不改。

### 7.2 数据模型（Pictor / `VehicleRegistry`）

`Pictor/src/vehicle_registry/vehicle_registry.gd` 现有 `vehicles: Dictionary`
（`{vid → {name,x,y,yaw,node_type}}`，`:11`）。本步加：

```gdscript
## vid → formation_id（"" 不存在 = 无编队）。**独立于 vehicles**，
## 这样 _on_vehicle_unregistered 擦掉 vehicles 条目后，重连仍能恢复编队。
var formations: Dictionary = {}

func Join_Formation(vehicle_id: String, formation_id: String) -> bool
func Leave_Formation(vehicle_id: String) -> bool
func Get_Formation(vehicle_id: String) -> String
func Get_Formation_Members(formation_id: String) -> Array[String]   // 排序，确定性
```

- `formation_id`：`strip_edges()` 后必须非空；建议限定 `[a-z0-9_-]{1,32}`（非法 → 调用方拒绝）。
- `join` 幂等（已在该编队 → `true`，不重复写）；`leave` 幂等（不在任何编队 → `true`）。
- **断线重连保留**：`_on_vehicle_unregistered` 只擦 `vehicles`，**不擦** `formations`；
  `_on_vehicle_registered` 时按 `formations` 恢复（无需额外广播）。
- `vehicles` 列表输出（`ws_command_server.gd:_Vehicle_List()` `:675-690`）增加
  `"formation": Get_Formation(vid)`，供脚本/UI 读。
- 不做 UI 徽标（可选，不列入验收）。

### 7.3 WS 契约（`WsCommandServer`）

`_Handle_Command()`（`ws_command_server.gd:256-275`）新增一个分支；报文沿用现有 `cmd` 风格：

```json
{"id":7,"cmd":"formation","action":"join","targets":["<vid 或车名>"],"formation":"containment"}
{"id":8,"cmd":"formation","action":"leave","targets":["<vid 或车名>"]}
{"id":9,"cmd":"formation","action":"list","formation":"containment"}
```

- `targets` 复用 `_String_Array` + `_Resolve_Target()`（`:638-650`），**先全部解析成功再改**（原子）。
- 应答：

```json
{"ack":7,"ok":true,"action":"join","formation":"containment","members":["vid1","vid2","vid3","vid4"]}
```

- 错误（`ok:false`，**注册表不变**）：`action` 未知 / `formation` 空或非法（仅 `join` 必填）/
  `targets` 空 / 任一 target 解析不到。
- `action` 大小写不敏感（`JOIN` 也接受）；`formation` id **不折大小写**（`Containment` 非法）。
- `leave` 带一个没有成员的合法 `formation` → `ok:true, members: []`（不当错误；与实现一致）。
- `list` 的 `formation` 可省略 = `members` 返回全编队成员并集，另带 `formations` 全表
  `{formation_id: [vid...]}`。
- `pictor_ws_ctl.py` 增加 `formation` 子命令：
  `formation --target <vid|name> --join containment` / `--leave` / `--list containment`
  （`tool/pictor_ws_ctl.py:1018-1094` 的 subparser 区）。

### 7.4 CoopController 修改

- 新增常量 `const COOP_FORMATION := "containment"`（唯一来源）。
- `_online_cars()` 的候选集增加过滤：`registry.Get_Formation(id) == COOP_FORMATION`；
  或在 `_online_cars()` 之后新增 `_formation_cars()`，**不改 `_online_cars()` 的语义**（单测兼容）。
- 其余不变：排除请求者（`:191`）→ N = 成员数 → `ring_points` → `assign_one_to_one` →
  逐车一帧 `[Goto, Face]`。
- 日志追加 `编队=containment 成员=N`，便于追溯。
- **零成员**（无人在编队 / 都在离线 / 都在 `(0,0)`）→ `print("[Coop] … 编队 containment 无在线成员 → 忽略")`
  + **一帧不发**；**不得**回退到全部在线车。

### 7.5 测试与验收

**测试（全部 headless，真跑）**

| 文件 | 新增覆盖 |
|---|---|
| `Pictor/src/test/test_coop_controller.gd` | ① 12 车中 4 台在 `containment` → 恰好 4 帧、每帧 `[Goto, Face]`、8 台非成员 0 帧；② 0 成员 → 0 帧；③ 4 台编队中 1 台无位姿 → N=3；④ 请求者是编队内 car → 排除后 N=3；⑤ 非编队成员不受影响 |
| `Pictor/tool/check_ws_command_server.gd` | 新增 A15 join / A16 leave / A17 list + `vehicles` 带 formation / A18 错误路径（未知 vid、空 formation、未知 action）且注册表不变 |
| `Pictor/src/test/test_formation.gd`（新增） | 数据层 F1~F10：默认空 / join-leave 幂等 / 非法 id / 断线重连保留 / members 排序 / 全表 |
| `Pictor/tool/pictor_ws_ctl.py` | `formation` 子命令 dry-run / 参数解析（不要求真连 Pictor） |

> 基线修正：`check_ws_command_server.gd` 的 A9 断言仍是 task_44 之前的 `msgid 4 + action 9`，
> 基线实测 `total=14 failed=1`；本步按 task_44 的 `msgid 8 + mode 1` 修正（只改断言，不改被测代码），
> 修正后 `total=18 failed=0`。

命令（PowerShell）：

```powershell
& "C:\workspace\Tool\Steam\steamapps\common\Godot Engine\godot.windows.opt.tools.64.exe" `
  --headless --path C:\workspace\Workspace\Workspace\Pleiades_orion\Pictor -s res://src/test/test_coop_controller.gd
& "C:\workspace\Tool\Steam\steamapps\common\Godot Engine\godot.windows.opt.tools.64.exe" `
  --headless --fixed-fps 60 --path C:\workspace\Workspace\Workspace\Pleiades_orion\Pictor -s res://tool/check_ws_command_server.gd
```

**判据（全过才 PASS）**

1. 12 车 + 4 台 `containment`：assist → **恰好 4 条 `cmd_send`**，targets 各 1 台且都是成员；
   8 台非成员 **0 条**。
2. 0 成员：assist → **0 条** + 一行忽略日志。
3. 默认无编队：新 `vehicle_registered` 后 `Get_Formation(vid) == ""`。
4. WS `join/leave/list` ack 正确；非法请求 `ok:false` 且注册表不变（原子）。
5. `vehicles` 事件每条带 `formation` 字段。
6. 现有 `test_coop_controller.gd` 全部 PASS、`check_ws_command_server.gd` A1~A14 不回归
   （`=== ALL PASS: true ===` / `SELF-CHECK OK` / exit 0）。
7. `git -C Pictor diff --stat` 只含本步允许文件；**orion 仓库除本 Task 文档外零改动**；
   `src/websocket/protocol/*`、`src/kernel/kernel_bridge.gd`、orion Rust 全部零改动。

### 7.6 边界 / 风险

| # | 风险 | 处置 |
|---|---|---|
| F1 | `TASK_SET` 整队替换：编队成员收到 assist 会顶掉自己当前任务 | 既定语义，接受；编队只决定「谁收到」 |
| F2 | 成员掉线 / 无位姿 → N 变小，环点重算 | 按在线成员算；0 → 不发 |
| F3 | 两个 UAV 同时请求 → 后到覆盖先到 | 本步不加限流（task_41 §5.7 同口径） |
| F4 | 重连后编队丢失 | `formations` 独立于 `vehicles`，按 vid 保留 |
| F5 | `check_ws_command_server.gd` 的 A 组依赖 `-s` 下 EventBus | 沿用现有自检；A1 SKIP 不算 PASS |
| F6 | `pictor_ws_ctl.py` 有两个副本（`Pictor/tool/` 与 `RobotSim_TestEnv/tools/`） | 本步只改仓库内；夹具副本后续手工同步 |

### 7.7 证据与收尾

- 测试输出、`git diff --stat`、关键日志摘录落 `orion/tmp/task50_s6/`，不入库。
- 不改 `Task/` 其他内容；不改 `Workbook/` 以外的 orion 文件（Workbook 由执行引擎收口时追加）。
