# task_49_world — 多场景化世界：场景数据化 + 室内走廊（Guard 演示场）

> Presented by KeJi
> Created Date ： 2026-11-06
> Modified Date ： 2026-11-06
> 状态：Step 1 ~ Step 7 **已确认，执行中**（2026-11-06 起；执行引擎 `tmp/task49_tools/`）。
> 人类 2026-11-06 最终口径：detector **实配 2 台**；贴图只找**正规网站**、设次数上限、**找不到就放弃**（纯体素 + 纯色）；阶梯档位 5 / 10 / 20 / 30。
> 上游结论：2026-11-06 与人类的讨论 —— 「要多个场景，一次起一个，按需启动」/「现在演示的那个场景照旧，
> 另外要一个室内场景 + 一条走廊，用来演示 Guard」/「仍然是体素地图，建筑与道具就当成建模摆在那里」/
> 「室内需要的贴图也要一起准备好」/「最后一步做阶梯规模验证（5/10/20/30 台），detector 最多 5 台」；
> 以及 2026-11-06 的全项目代码梳理（`tmp/survey2/09_world_tools.md`）。

---

## 一、目标

### 1.1 三件事

| # | 交付 | 一句话 |
|---|---|---|
| 1 | **场景数据化** | 把「世界长什么样」从 `World.gd` 的常量里抽出来，变成一份场景数据 + 一个入口 `.tscn`；一份 `World.gd` 跑所有场景，一次只起一个 |
| 2 | **室内走廊场景** | 新增「室内走廊」场景（含一个 4 m 宽 24 m 长的走廊 + 远端小房间），专门用来演示 `Guard`（值守 → 出动 → 归位 → 迟滞） |
| 3 | **阶梯规模验证** | 在**城市场景**上按 5 → 10 → 20 → 30 台逐级加压，定位"第一个瓶颈在哪一层"（世界帧率 / 位姿下行 / 每节点 CPU / `Lagged`） |

### 1.2 现状（代码事实，2026-11-06）

| 事实 | 位置 |
|---|---|
| 世界几何全部硬编在一处：15 条 `OBSTACLES`（4 边界墙 + building_a/b + wall_c/d + pillar_e + 三级退台 + 3 散块） | `src/world/World.gd:46-68` |
| 出生点硬编 `(64, 64, 0)`，净空 3 m × 4 m | `World.gd:39-42` |
| 不可见边界：5 个 box（4 面 32 m 空气墙 + 32 m 顶盖），坐标随 128×128×32 写死 | `World.gd:80-91`、`_Build_Bounds()` `:159-181` |
| 尺度唯一来源是 `project.godot` 的 `[sim_world]`（voxel 0.5 / cells 256×256 / layers 64） | `World.gd:26-29`、`project.godot` |
| 街道（路面 / 28 段标线 / 35 片路缘）与立面（窗 / 基座 / 压顶）是**运行时建的纯 mesh，零碰撞** | `World.gd:121-122` → `StreetLayout.Build_All` / `Facade.Build_All` |
| 观感（地面/体素材质 + HDRI 天空 + 太阳）是**运行时替换**，不改 `.tscn` | `World.gd:117-119, 238-240` → `WorldLook.Apply_All` |
| 体素化入口：任意 AABB → 格，右开区间，单格厚度至少 1 层 | `World.gd:255-267`（`_fill_block_world` / `_set_voxel`） |
| 碰撞层：地面 1 / 体素 2 / 车辆 4；`Lidar2D` 的 `scan_mask = 1\|2\|4` | `World.tscn`、`Lidar2D.gd:42` |
| 世界尺寸是**跨语言契约**：`WORLD_MIN/MAX = 0.0/128.0`、`DEFAULT_ORIGIN = (64,64,0)`，三份 `main.rs` 各写一遍 | `pleiades-ugv/src/main.rs:23-28`、`pleiades-sim/src/main.rs:28-31`、`pleiades-uav/src/main.rs:24` |
| 自检脚本（`src/tests/check_*.gd`）**已于 task_46 Step 3 整体删除**；现在 `src/` 下无 tests 目录 | `COMPONENTS.md:199` |
| Guard 的场地相关参数：进 3.0 / 出 3.5 m、±30°、出动带 2.2 / 1.2 m、已在站 0.3 m | `pleiades-ugv/src/ugv/mission/guard.rs:53-63` |
| 车相机地面可视范围约 **0.33 ~ 7.7 m**（`cam_z=0.135`、`pitch=+1.7°`、`MIN_DEPRESSION=1°`） | `task_48` §1.1、`ugv/config.rs:83,89` |

### 1.3 为什么这样切（备选方案与结论）

| 方案 | 结论 |
|---|---|
| **场景 = 一份数据 + 一个入口 `.tscn`**（本任务） | `World.gd` 本来就已经是程序化生成（体素 / 空气墙 / 街道 / 立面 / 观感全是运行时建），新场景 ≈ 一张新 AABB 表 + 一个观感档 + 一个出生点。城市街道行为零变化 |
| 每个场景复制一份 `World.gd` + 全套脚本 | 五份拷贝、改一处要改五处；本仓已吃够"逐字节拷贝漂移"的苦（D-S1/D-S2） |
| 一个 `.tscn` 里塞所有场景，靠可见性切换 | 体素 `GridMap` 与碰撞是全局共享的，切换要清空重建；且"同时存在"会让雷达把另一个场景的墙也扫到 |
| 改成网格碰撞（导入建筑模型） | 会把重力从"体素栅格"变成"三角网格"，牵动雷达建图、D\* 栅格、四端寻路口径 —— 本任务明确不做（§六） |
| 为室内再写一套 `World` | 与第一条同罪，且 `WebSocketManager` / `Vehicle` / 传感器挂载全都不用变，多写无益 |

**一句话**：体素管"能不能走 / 雷达看到什么 / 怎么寻路"，mesh 只管"看起来像什么"。室内场景沿用这条分工。

---

## 二、已定口径（人类已确认，实施时不得改）

### 通用（#1 ~ #8）

| # | 项 |
|---|---|
| 1 | **场景 = 数据 + 入口**：一份 `World.gd`；新增场景只加数据 + 一个 `.tscn` 入口，不复制任何脚本 |
| 2 | **一次只起一个场景**，按需启动：两个入口各占 9999，同一时刻只能跑一个，不做运行期切换 |
| 3 | **城市街道场景（= 现在演示的那个）行为零变化**：`World.tscn` 不填场景名时走 `city` 档，其数值与 2026-11-06 的 `World.gd` 常量逐条相等 |
| 4 | **体素仍是唯一碰撞 / 雷达 / 建图 / 寻路口径**；建筑、街道、立面、室内道具一律**纯视觉、零碰撞** |
| 5 | 视觉件必须**落在 0.5 m 体素边界**上；墙的视觉面与体素外表面**共面**（否则会出现"车停在离墙 25 cm 处"或贴图错位） |
| 6 | **凸出物不进体素就不做**：门框、柱脚、踢脚线之类若比墙面凸出，要么不做，要么也进 `OBSTACLES` 变成整格 |
| 7 | **视觉件会影响 detector**：YOLO 看的是渲染画面，室内太暗就检不出人、Guard 直接失效。室内光照必须保证人在 **3 ~ 8 m** 处清晰可辨 |
| 8 | **世界尺度锁 128×128×32**：`WORLD_MIN/MAX` 与 `DEFAULT_ORIGIN` 是三份 `main.rs` 的硬编码契约，改尺寸另立 task。室内因此是"128 m 场地里隔出来的一条走廊" |

### 室内走廊（#9 ~ #16）

| # | 项 |
|---|---|
| 9 | 走廊**沿 x**：内表面 `x ∈ [50, 74]`（长 24 m）、内表面 `y = 62 / 66`（**净宽 4 m**）、墙厚 **0.5 m**（正好 1 格体素）、墙高 **3 m**（6 层格） |
| 10 | 两端：`x=50` 端**封死**；`x=74` 端留 **1.0 m 宽门洞**（`y ∈ [63.5, 64.5]`，2 格），通到一个内表面 `x ∈ [74.5, 79]`、`y ∈ [62, 66]` 的**小房间**（4.5 m × 4 m） |
| 11 | 出生点不动，仍是 **`(64, 64, 0)`** —— 它落在走廊中段，所以世界 `SPAWN_WORLD` 与 sim 侧 `DEFAULT_ORIGIN` **都不用改** |
| 12 | 值守点 `(58, 64)`、注视点 `(74, 64)`（= 门洞位置）、行人起点 `(72, 64)` |
| 13 | 天花板 **3.0 m，开关 `ceiling` 默认 `false`**：关掉才能用现有俯视观察相机录演示；打开就有"室内感"但只能从走廊内看 |
| 14 | 室内**不引入任何外部 `.glb` 模型**：墙 / 地 / 顶 / 门洞 / 踢脚线 / 墙裙全部程序化生成（`BoxMesh` / `SurfaceTool`）。道具（灭火器 / 指示牌 / 长椅…）按需再加，见 #18 |
| 15 | 贴图：**只收 CC0**，且**只能在正规站点**取（白名单：`polyhaven.com` / `ambientcg.com` / `poly.pizza` / `kenney.nl` / `sketchfab.com`）。Poly Haven 用 `nor_gl`（**禁 `nor_dx`**）、`arm` 即 ORM；ambientCG 给分离通道需**离线**拼 `*_orm.png`（R←AO、G←Rough、B←Metal）。`.import` 必须与贴图一起提交 |
| 16 | 不自设指标：**本任务不写任何自检 / 断言脚本**，验收只看现象（§八）。历史上那套 `src/tests/check_*.gd` 已随 task_46 Step 3 删除，不得复活 |

### 实现细节（#17 ~ #22）

| # | 项 |
|---|---|
| 17 | `StreetLayout` / `Facade` 只在 `city` 档跑；`WorldLook` 增加 `indoor` 档（跳过 HDRI 天空与太阳，改 `ambient_light_source = COLOR` + 室内灯） |
| 18 | 需要点缀道具时才加 `.glb`（Poly Pizza / Kenney，CC0 优先），单件 ≤ 1 MB、≤ 50k tris，并进 `assets/README.md` 台账 |
| 19 | 走廊与值守点之间**不得有障碍物**（Guard 出动走 Follow 子树，`follow` 不看地图、不绕障，只有归位走 D\*） |
| 20 | 不碰任何 Rust 代码；不动 Pictor；不动 `WebSocketManager` / `Vehicle` / 底盘 / 传感器 |
| 21 | **资产选型与下载单独成步**（Step 3）：只做到"文件在库 + `.import` 生成 + 台账齐"，**不写一行接线代码**；选型证据落 `tmp/task49/assets/`（不入库）。理由：下载是试错型工作、耗时不可控；与接线混在一起时"图没下对"与"材质接错"分不清 |
| 22 | **找不到就放弃，不自建**（人类 2026-11-06 定）：Step 3 在次数上限内没找到合规贴图 → **室内退化为"原始体素 + 纯色材质"**（墙面 / 地面 / 踢脚线用 `Make_Fallback_Voxel_Material` 那套 `StandardMaterial3D` + 现成 `FALLBACK_COLORS`），**不得**自造贴图、**不得**换站、**不得**用 CC-BY 凑数。退化也是合格交付，Step 4 必须两条路径都能起 |

### 规模验证（#23 ~ #27）

| # | 项 |
|---|---|
| 23 | **阶梯验证是最后一步（Step 7）**：档位 **5 → 10 → 20 → 30 台**，逐级加压；**上一级现象不退化才进下一级**，不跳档 |
| 24 | **detector 最多 5 台**（人类 2026-11-06 定）：其余节点 `[detector].enabled = false` → 零线程、不读帧。实配建议 **2 台**（1 UAV 封控 + 1 车 Guard），留出余量 |
| 25 | 阶梯验证跑**城市场景**（`city`）：室内走廊只有 24 m × 4 m，最多塞 1~2 台；30 台必须回到 128×128 的城市场景 |
| 26 | 验证前先加**三行诊断日志**（世界 fps / 位姿下行实测频率 / 连接数与车数），5 s 一次、用现成的 `stats_log` 门控。没有这三个数就只能看到"有点卡"，无法定位是哪一层 |
| 27 | 阶梯的四个数是**观察记录，不是断言**（口径 #16 不变）：不写脚本断言，人工读数 + 日志摘录存档 |

---

## 三、场景数据（SceneDef）

### 3.1 字段（`src/world/SceneDefs.gd`，纯常量表 + 两个只读访问器）

```gdscript
# 一个场景 = 一条数据。全部是常量表，禁止运行时改。
const SCENES := {
    "city": { ... },              # 现有城市街道：数值与 World.gd 2026-11-06 的常量逐条相等
    "corridor_indoor": { ... },   # 室内走廊
}
```

| 字段 | 类型 | 语义 | `city` | `corridor_indoor` |
|---|---|---|---|---|
| `name` | String | 日志与诊断用 | `"city"` | `"corridor_indoor"` |
| `obstacles` | Array[Dictionary] | `{name, origin: Vector3, size: Vector3}`，**世界坐标 Z-up**，右开区间 | 现有 15 条 | §3.3 的 8 条 |
| `spawn` | Vector3 | 首台 avatar 的出生点 | `(64,64,0)` | `(64,64,0)` |
| `spawn_clear_radius_m` | float | 出生净空自检半径（只用于日志告警） | 3.0 | 1.5 |
| `spawn_clear_height_m` | float | 同上（高度） | 4.0 | 2.5 |
| `air_wall_height_m` | float | 空气墙高度（顶盖高度） | 32.0 | 32.0（不封顶，室外容器照旧） |
| `props_street` | bool | 是否建街道（`StreetLayout`） | true | false |
| `props_facade` | bool | 是否建立面（`Facade`） | true | false |
| `look` | String | 观感档：`"city"` / `"indoor"` | `"city"` | `"indoor"` |
| `ceiling` | bool | 是否建可见天花板 | false（现在没有） | **false**（可开） |
| `ceiling_z_m` | float | 天花板下表面高度 | — | 3.0 |

### 3.2 体素对齐规则（写死在数据里，不靠自觉）

1. 所有 `origin` / `size` 分量**必须是 0.5 的整数倍**（体素 = 0.5 m）。违反会被 `_fill_block_world` 的 `floor` 静默吃掉半格或长出一格。
2. **墙的厚度一律 0.5 m（= 1 格）**，不要写 0.2 / 0.3 —— 厚度 < 0.5 会被填成 1 格，视觉与碰撞就对不上。
3. **门洞宽度必须是 0.5 的整数倍**，且 ≥ 1.0 m（车宽 0.3 m + 两侧余量）。
4. 视觉件（地砖 overlay、踢脚线、门框、灯槽）的顶点坐标同样取 0.5 的整数倍，且与相邻体素的**外表面共面**。

### 3.3 `corridor_indoor` 的 `obstacles`（8 条，全部 0.5 整数倍）

```text
# 走廊：内表面 x∈[50,74]、y∈[62,66]（净宽 4 m）；墙厚 0.5；高 3 m
01 corridor_south  origin (50.0, 61.5, 0.0)  size (24.0, 0.5, 3.0)
02 corridor_north  origin (50.0, 66.0, 0.0)  size (24.0, 0.5, 3.0)
03 corridor_west   origin (49.5, 61.5, 0.0)  size ( 0.5, 5.0, 3.0)   # 西端封死

# 东墙带门洞：门洞 y∈[63.5, 64.5]（1.0 m = 2 格）
04 east_wall_low   origin (74.0, 61.5, 0.0)  size ( 0.5, 2.0, 3.0)   # y 61.5~63.5
05 east_wall_high  origin (74.0, 64.5, 0.0)  size ( 0.5, 2.0, 3.0)   # y 64.5~66.5

# 小房间：内表面 x∈[74.5,79]、y∈[62,66]
06 room_east       origin (79.0, 61.5, 0.0)  size ( 0.5, 5.0, 3.0)
07 room_north      origin (74.5, 66.0, 0.0)  size ( 5.0, 0.5, 3.0)
08 room_south      origin (74.5, 61.5, 0.0)  size ( 5.0, 0.5, 3.0)
```

体素规模估算：`24×1×6 ×2 + 1×10×6 + 1×4×6 ×2 + 1×10×6 + 5×1×6 ×2 ≈ 550 格`（现有 city 约 1.2 万格，性能无压力）。

### 3.4 场地与 Guard 参数的对应关系（为什么是这些数）

| Guard 参数 | 值 | 场地依据 |
|---|---|---|
| 警戒区半径（进/出） | 3.0 / 3.5 m | 人从 `x=72` 走进来，跨过 `x=61.5`（3.5 m）与 `x=61`（3.0 m）两个界 |
| 半角 | ±30° | 人在走廊中轴走，横向偏差 0 → 恒在扇形内；"看得见但不进区"靠**半径**实现，不靠角度 |
| 出动带 | `d > 2.2` 前进 / `d ≤ 1.2` 停 | 走廊净宽 4 m + 纵深 ≥ 8 m，够车从值守点推进到 1.2 m 且不贴墙 |
| 已到站判据 | 0.3 m | 值守点 `(58,64)` 距两端墙 ≥ 8 m，D\* 有绕行余量 |
| 相机可视 | 0.33 ~ 7.7 m | 人从 14 m 外走来，**7.7 m 起才被看见**（这一步是"看见但不动"的必要条件） |

---

## 四、涉及文件

| 文件 | 动作 | 内容 |
|---|---|---|
| `orion-simulation-world/src/world/SceneDefs.gd` | **新增** | 两个场景的常量表 + `Has(name)` / `Get(name)`（返回深拷贝） |
| `orion-simulation-world/src/world/World.gd` | 改 | `_ready()` 先解析场景名（导出属性 / 项目设置 / 默认 `city`），`OBSTACLES` / `SPAWN_WORLD` / 空气墙高度 / `StreetLayout`+`Facade` 开关全部改从 SceneDef 取；新增常量解析函数 |
| `orion-simulation-world/src/world/Interior.gd` | **新增**（Step 4） | 室内纯视觉件：地板 overlay（顶面 y=0.01）、踢脚线、门框、灯槽、可选天花板；全部 `BoxMesh` / `SurfaceTool`，**零碰撞** |
| `orion-simulation-world/src/world/WorldLook.gd` | 改（Step 4） | 新增 `Apply_Indoor(world)`（跳过天空与太阳，设 `ambient_light_source = COLOR` + 环境色；加 2~3 盏不投影的 `OmniLight3D`）与 `Make_Indoor_*_Material()` |
| `orion-simulation-world/src/scenes/WorldIndoor.tscn` | **新增**（Step 1） | 与 `World.tscn` 同构（同一套节点 + 同一个 `World.gd`），只多一个 `scene_name = "corridor_indoor"` |
| `orion-simulation-world/src/scenes/World.tscn` | 改（1 行，Step 1） | 填 `scene_name = "city"`（不填也默认 city；显式写出便于阅读） |
| `orion-simulation-world/assets/textures/wall_indoor_*` | **新增**（Step 3） | 室内墙面 PBR：`_color.jpg` / `_nor.jpg` / `_orm.png`（三通道，CC0；或自建） |
| `orion-simulation-world/assets/textures/floor_indoor_*` | **新增**（Step 3，可复用 `ground_paving_*` 替代） | 室内地面 PBR：同上三通道（CC0；或自建） |
| `orion-simulation-world/assets/README.md` | 改（Step 3） | 台账加 1~2 行：来源 URL / 资产 ID / 许可 / 通道拼法 / 尺寸 / 字节 / sha256 |
| `orion-simulation-world/COMPONENTS.md` | 改（Step 6 收尾） | §1.1 目录树加 `SceneDefs.gd` / `Interior.gd`；新增一节「多场景：场景数据 + 入口 `.tscn`」；§1.4 运行命令补室内入口 |
| `orion-simulation-world/src/net/WebSocketManager.gd` | 改（Step 7） | 只加**诊断日志**：世界 fps（`Engine.get_frames_per_second()`）、位姿下行实测频率（`_BroadcastPoses` 计数 / 窗口）、`GetStats()` 的 links / vehicles / pedestrians；沿用 `stats_log` 门控、5 s 一次。**不改任何行为** |
| `tmp/task49/assets/`（`candidates.json` / `notes.md` / 可选 `contact_sheet.png`） | **新增**（Step 3，**不入库**） | 选型证据：每条候选的 URL / 许可 / 尺寸 / 字节；每站查询数 / 耗时 / 失败项 / 用了直连还是代理 |
| `tmp/task49/scale/`（每档的日志摘录 / 四个数 / 收尾截图） | **新增**（Step 7，**不入库**） | 阶梯验证证据：5 / 10 / 20 / 30 各一份 |

---

## 五、Step

> 约定：每一步都写「改哪个文件 → 怎么改 → 怎么验（现象级，不写断言脚本）→ 会踩什么坑」。
> 主链是 1 → 2 → 4 → 5 → 6 → 7（场景数据 → 几何 → 视觉接线 → 天花板 → 演示 → 阶梯规模），**不跳步**；
> **Step 3（资产选型与入库）不在主链上**：它与 Step 1 / Step 2 无依赖，只要早于 Step 4 完成即可（可最先做）。

### Step 1 — 场景数据化（城市街道行为零变化）+ 两个入口

**依赖**：无。

1. **`SceneDefs.gd`（新增）**：把 `World.gd:39-42`（spawn / 净空）与 `:46-68`（15 条 `OBSTACLES`）**逐字**搬进 `SCENES["city"]`；再按 §3.3 写 `SCENES["corridor_indoor"]`（本步只落数据，几何还看不出效果）。
2. **`World.gd`**：
   - 加 `@export var scene_name: String = ""`；解析顺序 = 导出属性 → `ProjectSettings.get_setting("sim_world/world/scene", "")` → `"city"`；未知名字 `push_warning` 后回退 `"city"`（**不得 push_error**，验收要求 0 ERROR）。
   - `_ready()` 第一行取 `_def = SceneDefs.Get(_scene)`；`SPAWN_WORLD` / `OBSTACLES` 改为读 `_def`（`const` 保留作 `city` 的兜底来源，避免一次改太大）。
   - `_Build_Bounds()` 的高度改从 `_def.air_wall_height_m` 取（两档都是 32.0，行为不变）。
   - `StreetLayout.Build_All` / `Facade.Build_All` 用 `_def.props_street` / `_def.props_facade` 门控。
3. **`WorldIndoor.tscn`（新增）**：复制 `World.tscn` 的节点树（`World` / `WorldEnvironment` / `Sun` / `Ground` / `VoxelWorld` / `Vehicles` / `Systems`+`WebSocketManager` / `ObserverCamera`），脚本仍挂 `World.gd`，只加 `scene_name = "corridor_indoor"`；`Vehicles` 容器与 `node_paths` 相对路径逐字照抄（`../../Vehicles`）。
4. **验收（现象）**：
   ```bash
   # 语法
   for f in src/world/*.gd; do godot --headless --path orion-simulation-world --check-only --script "res://$f"; done
   # 城市场景：日志格数必须与改动前一致（改动前先记一次）
   timeout 180 godot --headless --path orion-simulation-world --quit 2>&1 | grep -nE "voxel world built|SCRIPT ERROR|Parse Error|ERROR"
   ```
   - `city` 的 `voxel world built: N cells` 与改动前**逐字相同**；
   - 无 `SCRIPT ERROR` / `Parse Error` / `ERROR`；
   - `WorldIndoor.tscn` 能起来（此时走廊是空的，车还站在 (64,64) 的空地上，属正常）。
5. **坑**：
   - `OBSTACLES` 是 `const`，被 `WebSocketManager`（`_Spawn_In_Obstacle`）与 `Facade`（经 `get_script_constant_map`）读；改成运行时数据后**这两处要跟着改读 SceneDef**，否则行人的 spawn 校验会继续用城市那张表。**本步必须一起改**。
   - `Facade.gd` 的注释明说"不 preload `World.gd`，用节点脚本常量表取 `OBSTACLES`" —— 改成 SceneDef 后这条依赖可以删，但别留半截。
   - 导出属性改名/新增后，`.tscn` 里没有该键是合法的（取默认值），不要为了"绑定"去写 `node_paths`。

### Step 2 — 室内走廊体素几何（墙 / 门洞 / 房间）

**依赖**：Step 1 绿。

**本步只做"能挡、能走、雷达看得见"**，一行视觉件都不写 —— 地板、踢脚线、门框、天花板全部留到 **Step 4**（它们要用 Step 3 的贴图，跟材质一起做才不用返工）。

1. **数据**：`corridor_indoor.obstacles` 按 §3.3 落地 8 条（0.5 m 整数倍，墙厚一律 0.5）。
2. **`World.gd`**：`indoor` 档下**不建**街道/立面（Step 1 已用 `props_street` / `props_facade` 门控）；本步不新增任何 `MeshInstance3D`。
3. **验收（现象）**：
   - 起 `WorldIndoor.tscn`，日志 `voxel world built: ~550 cells`（数量级对即可）；
   - 车从 `(64,64)` 出生后开向走廊两端：到 `x=50` 被墙挡住（`Blocked` / 停下），到 `x=74` 只有门洞那 1 m 能过 → 说明体素墙真的生效；
   - 雷达/建图在 Pictor 或日志里能看到走廊两面墙（而不是空白地图）；
   - 此时场景是"灰色光板世界"（只有体素与地面），**属预期**。
4. **坑**：
   - **`_fill_block_world` 是右开区间 + `maxi(end, start+1)`**：厚度 0.5 的墙能填 1 层，但若写 `size.y = 0.4` 会**仍然填 1 格**并偏移 0.1 m → 视觉与碰撞错位（§3.2 第 2 条）。
   - 门洞两侧的墙段必须**严格对齐**门洞边界（63.5 / 64.5），否则门洞会变成 1.5 格宽。
   - `Ground` 那层 128×1×128 的 `BoxShape3D` 顶面在 y=0：本步**不要**给地面加任何新碰撞体，否则会出现双层地面。
   - 体素是 `GridMap`：墙填满 0~3 m 的 6 层格，雷达（0.2 m 高）必命中；若只填 1 层，雷达能扫到但**车会从上方视觉穿模**（下一步的墙高 3 m 是为了这个）。

### Step 3 — 贴图 / 建模资产：选型与入库（**只入库，不接线；找不到就放弃**）

**依赖**：无。本步与 Step 1 / Step 2 **没有依赖关系**，可以最先做或并行做（本仓执行引擎仍按串行跑，只是它不阻塞任何人）。

**为什么单独成步**：选型与下载是"找资产"，接线是"用资产"。混在一起时，一旦墙面发黑/发白就分不清是"图没下对"还是"材质接错"，而且下载经常要试错、耗时不可控。所以本步的完成判据只有一条：**要么"文件在 `assets/` + `.import` + 台账"，要么"本次没有贴图，写清放弃理由"**；本步**不写一行接线代码**（不碰 `WorldLook` / `Interior` / `World.gd`）。

#### 1. 站点白名单（人类 2026-11-06 定：**必须正规网站，找不到就算了**）

| 允许 | 用途 | 备注 |
|---|---|---|
| `polyhaven.com` | 贴图 / HDRI | CC0，`nor_gl` + `arm` |
| `ambientcg.com` | 贴图 | CC0，分离通道需离线拼 ORM |
| `poly.pizza` | 道具 `.glb`（仅当 #4 需要时） | CC0 |
| `kenney.nl` | 道具 `.glb` | CC0，风格化 |
| `sketchfab.com` | 仅 `downloadable=true` + CC0 | 实测需登录 token → 不可直接取，**不可用则跳过** |

**明确禁止**：任何聚合站 / 转载站 / 网盘 / 论坛附件 / "免费下载"二次站；需要注册绕过、需要破解、来源不明的图**一律不要**。宁可不做，不凑数。

#### 2. 次数上限（先到先停，超限即按"没找到"处理）

| 项 | 上限 |
|---|---|
| 站点数 | ≤ **3** 个（按上表顺序试） |
| 每站查询词 | ≤ **2** 个 |
| 每站结果页 | ≤ **1** 页 |
| 每站详情页 | ≤ **6** 个 |
| 实际下载候选 | ≤ **3** 套（"套" = 3 个通道文件） |
| 总用时 | ≤ **30 min** |

#### 3. 合规判据（全部满足才算"找到"）

- 许可 = **CC0**（CC-BY 不收；人类口径是"找不到就算了"，不值得为署名工作量凑数）；
- 三通道齐：`_color` / `_nor`（`nor_gl`，**禁 `nor_dx`**）/ `_orm`（Poly Haven 的 `arm` 直接用；ambientCG 需离线拼 R←AO、G←Rough、B←Metal）；
- 单张 ≤ 1.5 MB、`assets/` 总增量 ≤ 10 MB；
- 贴图**可平铺**（室内墙面 2 m / 地面 4 m 一个循环，接缝不能明显）。

#### 4. 要做就做这两套（可只做墙面）

- `wall_indoor_*`：浅色涂料 / 抹灰（低频干净）；
- `floor_indoor_*`：自流平 / 水磨石 / 抛光混凝土。

#### 5. 找不到时的唯一动作

**停**。不换站、不自造、不降许可、不用别处的图凑。给出一份"放弃说明"：

- `tmp/task49/assets/notes.md` 写清：试了哪几个站、每个站查询词与结果、超限在哪一项、为什么没有合规件；
- `assets/README.md` **不加行**（没有资产就没有台账）；
- 在最终回复与 `impl.md` 里明确写一句「**本轮无贴图，室内走纯色降级**」，Step 4 会按退化路径实现。

#### 6. 入库流程（找到时才执行）

1. 落地 `assets/textures/`，命名与现有四套一致：`_color.jpg` / `_nor.jpg` / `_orm.png`；
2. `assets/README.md` §2 加行：来源 URL / 资产 ID / 许可 / 通道拼法 / 落地尺寸 / 处理 / 字节 / `sha256[:16]`；
3. 证据落 `tmp/task49/assets/`（不入库）：`candidates.json`（每条 site/id/url/许可/尺寸/字节）、`notes.md`（每站查询数、耗时、失败项、用了直连还是代理）；
4. `godot --headless --path orion-simulation-world --import`，`.import` 与贴图**一并提交**（E28）。

#### 7. 验收（现象）

- **找到**：`--import` 0 `ERROR`；`ls assets/textures/wall_indoor*` 三件齐且各有 `.import`；`assets/README.md` 有行且 `sha256[:16]` 与 `sha256sum` 对得上；
- **没找到**：`tmp/task49/assets/notes.md` 非空且包含"上限命中项 / 无合规件"的说明；**没有**新增任何贴图文件（`git status` 里 `assets/` 无新增）—— 两条路径**都算本步 PASS**；
- 不在这两步里：任何"把材质挂上去"的动作都属于 Step 4。

#### 8. 坑

- `nor_dx` 下错方向 → 光照看着"凸变凹"；Poly Haven 只取 `nor_gl`；
- `--import` 后要在 `git status` 里确认 `.import` 与贴图**成对**，只提交一个会让干净检出 `load()` 返回 null；
- 别下 2K/4K"看着更爽"的图（单张 ≤ 1.5 MB 的预算超了要重编码）；
- 直连优先，失败才 `curl -x http://127.0.0.1:7897`；**代理不得写进任何脚本 / 配置默认值**，用了什么通道记进 `notes.md`；
- **不要**因为"想让它好看"而放宽白名单或许可 —— 人类已明确"找不到就找不到"。

### Step 4 — 室内视觉件 + 材质与光照接线

**依赖**：Step 2 绿（几何定了）、Step 3 绿（贴图在库）。

**本步才第一次出现"看得见"的东西**：视觉件与材质一起做（视觉件要用 Step 3 的贴图，所以两步必须相邻）。

1. **`Interior.gd`（新增，纯视觉零碰撞，与 `StreetLayout` 同构）**
   - `Build_All(world, def)`：先摘同名旧容器（幂等），再建：
     - `Floor`（`MeshInstance3D`）：覆盖走廊 + 房间内表面的平面，**顶面 y = 0.01**（与街道 `ROAD_TOP_Z` 同一手法，避免与 `Ground` 顶面 z-fighting）
     - `Skirting`（`MeshInstance3D`）：四面墙脚 0.15 m 高、≤ 0.03 m 厚的薄片
     - `DoorFrame`（`MeshInstance3D`）：门洞两侧框，凸出 ≤ 0.03 m
     - `Ceiling`：见 Step 5（本步先不做，`def.ceiling` 门控留空）
   - 顶点坐标取 0.5 的整数倍（§3.2），节点 transform 恒等，顶点直接烘成 Godot 坐标（`WorldTransform.PosToGodot`）
2. **`WorldLook.gd`**
   - `Apply_All` 拆成 `Apply_City(world)` / `Apply_Indoor(world)`，`Apply_All` 保留为按档分派（`World.gd` 少改）
   - `Apply_Indoor`：**不换天空、不动太阳**；`env.background_mode = BG_COLOR`（深灰）+ `ambient_light_source = AMBIENT_SOURCE_COLOR` + `ambient_light_color` 取中性偏亮（口径 #7）；再加 2~3 盏 `OmniLight3D`（不投影、小能量）
   - `Make_Indoor_Wall_Material()` / `Make_Indoor_Floor_Material()`：走现有 `Make_Pbr_Material(prefix, tile_m)`，墙 `2.0` / 地 `4.0`（薄片无 UV，必须靠内部的 `uv1_triplanar + uv1_world_triplanar`）
   - **无贴图降级路径（口径 #22，必须实现）**：Step 3 没找到贴图时，这两个工厂函数返回 `StandardMaterial3D` + `FALLBACK_COLORS` 纯色（墙浅灰、地面深灰、踢脚深一档），**不得**因此报错或跳过视觉件；`Apply_Indoor` 的 ambient/灯**照常**（室内观感靠光照，不靠贴图）
3. **`World.gd`**：`def.look == "indoor"` 时调 `Interior.Build_All(self, def)` + `WorldLook.Apply_Indoor(self)`。
4. **验收（现象）**：
   - 起室内场景：**看不到天空**、走廊里**能看清人**（走进 3~8 m 时 detector 日志出现 `[detector] → 世界=(x,y) 距离=…`）；
   - 墙面/地面：**有贴图**则必须是纹理不是纯色；**无贴图**（Step 3 放弃）则必须是纯色且有明暗层次 —— **两条都算 PASS**，但不得"既没贴图又是一片死灰"；
   - 起城市场景：天空、太阳、地面材质与改动前**现象一致**（HDRI 天空、对向太阳、街道照旧）。
5. **坑**：
   - `llvmpipe` 软渲染基线只有 ~11 fps：**灯不要开阴影**，`OmniLight3D` ≤ 3 盏；
   - `Apply_All` 现在会**复制** `Environment` 与 `Sky` 再回写（`WorldLook.gd:135-149`）；`indoor` 档要在复制件上改，别直接改 `.tscn` 的共享 SubResource；
   - `PanoramaSkyMaterial` 若保留，室内会被天光照得发白；`indoor` 档必须切 `BG_COLOR`；
   - 亮度是**功能参数**不是审美参数（口径 #7）：验收以"人能被检出"为准，不以"好看"为准；
   - 地板 overlay 是纯 mesh，**不要加碰撞体**（`Ground` 那层 `BoxShape3D` 已经在地面），否则双层地面；
   - 踢脚线 / 门框凸出 ≤ 0.03 m：雷达与碰撞都看不见它们，车会擦着过去 —— 本任务接受这个装饰性误差，但**不得做厚**，也不得在走廊中间放任何凸出物（口径 #19）。

### Step 5 — 天花板开关 + 观察相机（可选，默认关）

**依赖**：Step 4 绿。

1. **`Interior.gd`**：`def.ceiling == true` 时建 `Ceiling`（下表面 `ceiling_z_m = 3.0`），纯视觉零碰撞。
2. **观察相机**：`ObserverCamera` 现在在 `(64, 26, 108)`（Godot 坐标）俯视整场 —— 天花板一开就只剩屋顶。两个选项，**默认选 A**：
   - **A（默认）**：`ceiling = false`，完全不动相机，保留俯视录演示；
   - **B**：`ceiling = true`，`WorldIndoor.tscn` 里把 `ObserverCamera` 实例的位置改成走廊内部的平视机位（例如 `x=56` 处回望门洞），需要单独调一次。
3. **验收（现象）**：开关两种取值各起一次 —— `false` 时俯视能看到车与走廊；`true` 时俯视看到屋顶（预期），从走廊内的机位能看到天花板。
4. **坑**：天花板若做成体素（进 `OBSTACLES`）会挡住 UAV 上升与雷达上层，**只做 mesh**；且天花板高度 3.0 m 时 UAV（2.5 m 巡航）**不该进这个场景**。

### Step 6 — 演示跑通（Guard 四条现象）

**依赖**：Step 1~5 全绿。

**目标**：用最新代码在室内场景跑一次**现象级**验证 —— 值守不动 / 出动 / 归位 / 迟滞。不设数值门槛，不做基线对照。

1. **环境**

| 角色 | 启动 | 端口 |
|---|---|---|
| 世界 | `xvfb-run -a -s "-screen 0 1600x900x24" godot --display-driver x11 --rendering-driver opengl3 --audio-driver Dummy --path orion-simulation-world res://src/scenes/WorldIndoor.tscn` | 9999 |
| sim 车 | `pleiades-sim`，`chassis = diff_drive_ugv`，**`[detector].enabled = true`** | 9098（控制口） |
| 行人 | `ped_console.py --peer-id human-01 --spawn 72,64,0,180` | 9101 |

> 世界**必须带渲染器**：headless 是 dummy 渲染，`CameraSensor` 不出帧 → detector 无目标 → Guard 退化为"只值守不追"。
> sim 车配置要有 `camera` 传感器的 `[[sim_world.sensor]]`，否则世界没有相机可出帧。

2. **注入**

```
sim 控制口：goto(58,64)         # 6 m，一条 Goto 足够（< 27 m 的 60 s 上限）
sim 控制口：guard(58,64 → 74,64)
行人：goto(60,64) → 停 → 退回 (72,64) → 再走进来
```

3. **看什么**

| # | 现象 |
|---|---|
| **G1** | 人走进走廊、进入 7.7 m 内但还在 3.5 m 外 → **车不动**（日志有 `[detector] → 世界=…`，但 `[Guard]` 无进区） |
| **G2** | 人继续走到 3.0 m 内 → 车朝人前进 → `d ≈ 1.2 m` 停住并朝人（`v=0`、`yaw` 指向人），日志出现进区 |
| **G3** | 人退到 3.6 m 外 → 车回值守点 `(58,64)` → 转回注视方向（门洞） |
| **G4** | 人在 3.0 ~ 3.5 m 之间来回走 → 车**不反复**出动/归位（只在跨 3.5 m 时才归） |

4. **判据**：现象出现 = PASS；车完全不动 / 任务不进队 / 日志报错或 panic / 连接掉线 / **Guard 任务被弹走** / 车在警戒区外乱动 = FAIL。

5. **收尾**：kill 全部进程，`ss -ltnp` 确认 9999 / 9098 / 9101 无残留。

6. **坑**：
   - `guard` 是**持续任务**，不取消不会自己结束；换演示必须重新下发 `TASK_SET` 或 `mission_count = 0`；
   - 整队替换语义：`guard` 会替换队列，若先前的 `goto` 还在跑会被清掉（演示顺序要按 §2 来）；
   - 车出生在 `(64,64)`、值守点 `(58,64)` 只有 6 m，但走廊西端封口在 `x=50` —— 归位时 D\* 有 8 m 余量，不会贴着墙算。

### Step 7 — 阶梯规模验证（5 → 10 → 20 → 30 台）

**依赖**：Step 1~6 全绿（尤其 Step 6 已跑通，说明"1~2 台的链路"是正确的；规模验证只在正确链路上放大）。

**目标**：不是"压到崩"，而是**定位第一个瓶颈在哪一层**。每一档只记四个数：世界 fps / 位姿下行实测频率 / 每节点 CPU / `Lagged` 次数。

1. **前置：三行诊断日志**（口径 #26；在 `WebSocketManager` 里加，不改行为）
   - `[scale] fps=…`：`Engine.get_frames_per_second()`，5 s 一次；
   - `[scale] pose_hz=…`：`_BroadcastPoses` 的实际下行次数 / 窗口秒数（**这是最先退化的一项**：它被帧率卡住，fps 掉到 3 就是 3 Hz）；
   - `[scale] links=… vehicles=… pedestrians=…`：现成 `GetStats()`。
2. **形态**
   - **城市场景**（`World.tscn` / `city`，口径 #25）；
   - N 个 `pleiades-sim --headless`，**每个一个独立目录**（`RobotSim_TestEnv/<name>/` 的现成形态：私有 `.config/` + `Log/` + `Pleiades_Workspace/`）；
   - `.config`：`[control]` 端口各不相同（9090…9119），`peer_name` 唯一，`[detector].enabled` 只有前 **2 台**为 `true`（口径 #24），其余 `false`；
   - **同机必须用静态 peer 表**（`[Network].bootstrap_peers`）：N 个进程抢同一个 5353 端口，mDNS 在本机多进程下不可靠 —— 卡在这里会误判成"CPU 顶不住"；
   - Pictor 可以不接（用 sim 控制口注入即可），避免把地面站的渲染开销混进世界侧的读数。
3. **每档的观察值（不是断言，是记录）**

   | 台数 | detector | 世界 fps | 位姿下行 | 每节点 CPU | `Lagged` |
   |---|---|---|---|---|---|
   | 5 | 2 | ≥ 30 | ≥ 10 Hz | < 50% 单核 | 0 |
   | 10 | 2 | ≥ 20 | ≥ 10 Hz | < 50% | 0 |
   | 20 | 2 | ≥ 10 | ≥ 8 Hz | < 60% | 0 |
   | 30 | 2~5 | ≥ 5 | ≥ 5 Hz | < 70% | 0 |

   > 这些数字是"还可以继续往上加"的经验门槛，不是验收判据本身；判据是**现象不退化**（车都在动、任务能进队、地图在长、没有掉线）。
4. **哪一项先破，就改哪一项（对应表）**

   | 先破的项 | 说明 | 处置（都属于 Step 7 范围内的配置级改动） |
   |---|---|---|
   | 世界 fps | 软渲染 + 30 个 avatar | 关阴影、窗口降到 640×480、`World.look` 关 PBR/HDRI（`LOOK_ENABLED = false` 走单色材质，2K triplanar 是软渲染最大开销）、必要时减 avatar 外观 |
   | 位姿下行 | 被帧率卡住（`_pose_acc` 按帧累加） | 同上；或把下行周期与帧解耦（改成按墙钟累加，最小改动是把 `_pose_acc` 用 `Time.get_ticks_msec` 而不是 `delta`） |
   | 每节点 CPU | 决策栈（D\* 20 ms 预算 × 20 Hz） | 降 `plan_budget_ms`；或让空闲车不发任务（空队列时不 tick 树） |
   | `Lagged` | `EventBus(1024)` 被 290+ 帧/s 追上 | 加 bus 容量；或把 MAP_DELTA 广播频率从 1 s 放到 2 s |
   | 连接数/内存 | 每节点 N-1 条全连接 | 静态表已避开 mDNS 问题，但连接数本身要"限 dial"或缩小 `ComponentLink` 的 4 MiB 双缓冲（按角色设小，见 §七） |
5. **收尾**：kill 全部 sim / 世界 / ped，`ss -ltnp` 确认 9090~9119、9999 无残留；证据写 `tmp/task49/scale/`（四档各一份日志摘录 + 四个数 + 一句话结论）。
6. **坑**：
   - **同机 30 个 sim 的 mDNS**：不配静态表几乎必然"发现不到"，先解决这个再谈性能；
   - **出生点环**：`WebSocketManager._PickSpawn` 是 8 方位 × 每环 +2 m，30 台会铺到第 4 环（半径 8 m），城市场景出生点周围 8 m 内是空的（最近建筑在 (24,24)/(88,28)），够用；但**不要**在室内走廊做这一档；
   - **端口占用**：每个 sim 的 `[control]` 与 world 的 9999 都要预先分好，重启失败大多是上一轮没杀干净；
   - **`/` 分区只剩 ~100 MB**：30 个实例的 `Log/` 会写日志，`TMPDIR`/`XDG_*` 全部重定向到 `tmp/`（task_48 的做法），否则跑一半写满根分区；
   - 这一档跑出来的数只对**本机 + llvmpipe + 回环**成立，不能直接换算成"30 台真机的上限"（真机有 WiFi 丢包与带宽限制，是另一个变量）。

---

## 六、边界（本任务明确不做）

- **不改世界尺度**（128×128×32），不动 `WORLD_MIN/MAX` / `DEFAULT_ORIGIN`，不动任何 Rust 代码。
- **不引入网格碰撞**：不改体素/雷达/建图/D\* 口径。
- **不做多场景同时存在**，不做运行期切换场景。
- **不写任何自检 / 断言脚本**（历史那套已删，不复活）。
- **不动** `WebSocketManager` / `Vehicle` / 底盘 / 传感器 / Pictor / `pleiades-sim` 代码。
- **不做**场景编辑器插件、不做地形/高度场、不做天气/时间系统。
- **不给 Guard 加避障**（出动走 Follow 不绕障，靠"走廊里不放障碍"保证，口径 #19）。
- **不做**走廊尽头的多岔路 / 十字路口（单走廊 + 单房间，够演示）。
- **不做 50 台以上**、不做真机 WiFi 带宽/丢包压测（本任务的阶梯只到 30 台，且跑在本机回环上）。
- **不做**连接策略重构（限 dial / 邻居选择 / `PeerManager` 上限）：Step 7 只**观察**这些项的代价，真要改另立 task。

---

## 七、风险

| 风险 | 说明 / 处置 |
|---|---|
| **视觉与体素不对齐** | 墙薄 0.1 m 或坐标不是 0.5 倍数 → 车停在离墙半格处 / 贴图错位。§3.2 四条硬规则 + Step 2 坑 1 |
| **室内太暗 → YOLO 失效** | 这是**功能**风险不是审美风险（口径 #7）。Step 4 验收必须看 `[detector] → 世界=…` 是否出现 |
| **世界 headless 起 → 相机零帧** | 室内演示必须带 `xvfb-run + x11 + opengl3`，否则 Guard 永远"只值守" |
| **天花板挡住俯视观察** | 所以默认 `ceiling = false`（口径 #13），要开就同步调相机机位（Step 5） |
| **`OBSTACLES` 改数据化后漏改消费方** | `WebSocketManager._Spawn_In_Obstacle`、`Facade.Build_All`、`World._warn_if_spawn_blocked` 三处都读它；Step 1 必须一起改 |
| **城市场景被悄悄改变** | Step 1 用 `voxel world built: N cells` 逐字对比 + 现象对照（天空/太阳/街道照旧） |
| **门洞被体素吃掉** | 门洞 < 0.5 m 或两侧墙段没对齐 → 门没了。数据里门洞 = 2 格、墙段严格对齐 63.5 / 64.5 |
| **贴图缺 `.import`** | `load()` 返回 null、墙面纯色。`--import` 后与贴图一起提交（E28） |
| **软渲染掉帧** | 室内灯 ≤ 3 盏、不投影；`Interior` 的 mesh 面数控制在几百 |
| **走廊里误放障碍** | Guard 出动不绕障（口径 #19）；装饰件凸出 ≤ 3 cm 且只在墙脚 |
| **阶梯验证卡在 mDNS 而不是性能** | 同机 30 个进程抢 5353，mDNS 不可靠 → 会误判成 CPU 顶不住。前置必须配静态 `bootstrap_peers`（口径 #26 / Step 7 坑 1） |
| **位姿下行随帧率掉频** | `_BroadcastPoses` 用 `delta` 累加，fps 掉到 3 就只有 3 Hz；sim 侧位姿龄变大、建图与位姿不同源。Step 7 的 `pose_hz` 就是盯这一项；处置见 Step 7 §4 |
| **世界侧 90~120 条 WS 的 4 MiB 双缓冲** | 30 台 × 3 连接 × 8 MiB = 上界 720 MB（RAM 够，但纯浪费）；按角色设小是 Step 7 允许的配置级改动 |
| **`Lagged` 丢帧 → 地图永久偏移** | `EventBus(1024)` 在 290+ 帧/s 下约 3.5 s 缓冲；`apply_delta` 加性不可逆，丢一帧就偏。Step 7 记录 `Lagged` 次数，处置见 §4 |

---

## 八、验收总表

| 检查项 | 命令 / 证据 | Step |
|---|---|---|
| 脚本语法（逐文件） | `godot --headless --path orion-simulation-world --check-only --script res://src/world/<文件>.gd` | 1、4 |
| 城市场景格数不变 | `voxel world built: N cells` 与改动前逐字相同 | 1 |
| 日志无错 | `grep -nE "SCRIPT ERROR\|Parse Error\|ERROR"` → 零命中 | 1~5 |
| 室内走廊体素规模 | `voxel world built: ~550 cells` | 2 |
| 墙真的挡住车 | 控制口 `goto(50,64)` 被挡（停在墙前）；只有门洞那 1 m 能过 | 2 |
| **选型证据齐全** | `tmp/task49/assets/candidates.json` + `notes.md`（含每站查询数 / 耗时 / 通道） | 3 |
| **资产入库** | 三通道文件齐 + 各自 `.import` + `assets/README.md` 台账 + `sha256sum` 对得上 + `--import` 0 ERROR | 3 |
| 入选件合规 | 许可 = CC0（或 CC-BY 有 LICENSE/ATTRIBUTION）；单张 ≤ 1.5 MB；**无 `nor_dx`** | 3 |
| 室内视觉件 | 地板 / 踢脚线 / 门框在场景里可见，且**零碰撞**（雷达扫不到它们） | 4 |
| 室内观感 | 无天空 / 有材质纹理 / 有环境光 / 人走进 3~8 m 时 `[detector] → 世界=…` 出现 | 4 |
| 天花板开关 | `ceiling=false/true` 各起一次，现象符合预期 | 5 |
| Guard 四条现象 | G1 不动 / G2 上前到 ~1.2 m / G3 归位 / G4 迟滞不抽 | 6 |
| 城市场景零回归 | 天空 / 太阳 / 街道 / 15 条障碍现象与改动前一致 | 1、4 |
| 三行诊断日志 | `[scale] fps=…` / `pose_hz=…` / `links=… vehicles=…`，5 s 一次 | 7 |
| 阶梯 5 台 | 2 台 detector；四个数记档；现象不退化（车在动、任务能进队、地图在长、无掉线） | 7 |
| 阶梯 10 / 20 台 | 同上，逐级只在前一级不退化时进 | 7 |
| 阶梯 30 台 | 同上；`Lagged` 计数与首次退化项写进 `tmp/task49/scale/` | 7 |

---

## 九、人类已确认（2026-11-06）

1. **走廊数值**：长 24 m / 净宽 4 m / 墙高 3 m / 门洞 1 m / 房间 4.5×4 m —— 按此实施（口径 #9/#10）。
2. **天花板默认 `false`**：按此实施（口径 #13）；要开天花板时同步调观察相机（Step 5 选项 B）。
3. **贴图**：**只找正规网站、只收 CC0、设次数上限、找不到就放弃**（口径 #15 / #22 / Step 3 §1~§2）；放弃时室内走纯色降级。
4. **道具**：本轮**不做**点缀道具（保持"零外部模型"，全程序化）。
5. **小房间**：要做（人从房间里走出来，给演示一个自然起点）。
6. **现有城市场景保持不变**（口径 #3）。
7. **detector**：上限 5 台，**实配 2 台**（1 UAV 封控发现人 + 1 台 Guard 值守车）；5 台只在阶梯最后一档试（口径 #24）。
8. **阶梯档位**：5 / 10 / 20 / 30 四档，四个观察数（fps / `pose_hz` / 每节点 CPU / `Lagged`），不写脚本断言（口径 #23 / #27）。
