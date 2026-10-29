# wb_38_sim_world_refine — task_38 工作记录

> Presented by KeJi
> Created Date ： 2026-10-28
> Modified Date ： 2026-10-28
> 任务：Task/task_38_sim_world_refine.md（城市街道 + PBR + HDRI + 新世界 Follow 复验，Step 1~3）

## 执行引擎启动
- 开始时间：2026-10-28 15:02:56Z
- 引擎：Tool/execution_engine_task38.ts + Tool/execution_engine_task38.config.json（流程照搬 task_36）
- 运行标记：首轮（Step 1~3）（本次从 Step 0 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次
- 范围：世界材质/天空（Step 1）→ 街道几何/立面（Step 2）→ 新世界 Follow 复验（Step 3）
- 验收红线：Step 3 **只验不改**；不通过就如实汇报，不调阈值/不改 Rust

## 执行引擎启动
- 开始时间：2026-10-29 01:12:26Z
- 引擎：Tool/execution_engine_task38.ts + Tool/execution_engine_task38.config.json（流程照搬 task_36）
- 运行标记：重启（Step 1a/1b/2/3，拆分 Step 1）（本次从 Step 0 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次
- 范围：材质资产（1a）→ 材质接线/天空（1b）→ 街道几何/立面（2）→ 新世界 Follow 复验（3）
- 说明：上一轮在 Step 1 实现阶段卡死（dispatchsubagent 无超时），本轮拆步 + 全面加 timebox
- 验收红线：最后一步 **只验不改**；不通过就如实汇报，不调阈值/不改 Rust

## 执行引擎启动
- 开始时间：2026-10-29 01:15:35Z
- 引擎：Tool/execution_engine_task38.ts + Tool/execution_engine_task38.config.json（流程照搬 task_36）
- 运行标记：重启（Step 1a/1b/2/3，无时间预算）（本次从 Step 0 起）
- 计划：每个 step 最多 3 轮（规划 → 实现 → 评审）；子 agent 异常结束自动重试 1 次
- 范围：材质资产（1a）→ 材质接线/天空（1b）→ 街道几何/立面（2）→ 新世界 Follow 复验（3）
- 说明：上一轮卡住是 LLM provider 不稳定所致（已换服务）；本引擎不设任何时间预算
- 验收红线：最后一步 **只验不改**；不通过就如实汇报，不调阈值/不改 Rust

## Step 1 — Step 1a — 材质资产入库（CC0 贴图 + HDRI）（重启（Step 1a/1b/2/3，无时间预算））
- 起止：2026-10-29 01:15:35Z → 2026-10-29 01:51:17Z
- 范围：orion-simulation-world/assets/ 与 assets/README.md（只放资产，不写代码）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 `tmp/task38/plan.md`（529 行）。核心内容： **选型**：地面用 ambientCG `PavingStones136`（现代灰色方形铺装板），体素墙用 Poly Haven `concrete_wall_008`，沥青用 `asphalt_02`（文档指定），路缘用 `concrete_wall_005`（同系列中偏深），HDRI 用 `kloofendal_43d_clear_puresky` 2K。 **已实测**（不是估算）：12 张贴图加 1 张 HDRI，合计 **9 506 040 B（9.51 MB）**，在 10 MiB 的严读法下仍余下 0.97 MB；最大单张 975 KB。构建脚本已在临时根目录**试跑验证**，13 个文件的字节数逐个吻合（`BUILD OK 9506040`）；过程中修掉一个错误——HDRI 头是 10 字节，不是 11 字节。 **要点**：ORM 全部存为无损 PNG、512 平方（实测用 JPG 会把 B 通道从 0 污染到 33/255，因为压缩量化）；法线一律按 4:4:4 存储；全程不碰 `nor_dx`；不给 `.import` 手工改参数（已核对引擎默认就是 `mode=0` 加 mipmaps）。计划还给出了 12 条验收命令与判据（V1–V12），以及体积超限时的确定性 L0–L2 降级阶梯。 **上轮 issues**：无（首轮）。未改动任何源码或资产，`World.tscn` 的 diff 为空。

## Step 2 — Step 1b — 材质接线与天空（WorldLook + 自检 + 出图）（重启（Step 1a/1b/2/3，无时间预算））
- 起止：2026-10-29 01:51:17Z → 2026-10-29 02:17:23Z
- 范围：orion-simulation-world：src/world/ + src/tests/check_world_look.gd
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task38/plan.md)（747 行），并核对过：`src/`、`COMPONENTS.md`、`World.tscn` 均零改动，git status 与进场基线完全一致。 **计划摘要（供 workbook）** 计划见 tmp/task38/plan.md。要点：新增 WorldLook.gd（三贴图 PBR、世界空间 triplanar 6 m/2 m、PanoramaSkyMaterial、太阳对齐 HDRI 实测方向、energy 0.7）；World.gd 五处接线（LOOK_ENABLED + Apply_World_Look + 材质改取工厂）；新增 check_world_look.gd：S 组 30 项 + R 组 13 项（3 张 1600×900 图 + 太阳方向探针）；COMPONENTS.md 六处含 E30。实施 G0→G9：先取 17 headless + 3 Xvfb 改前基线再动代码；V1~V18 判据，10 条风险各带确定性分支。首轮无 issues，源码零改动。 **两条实测依据（不是估算）**：HDRI 最亮像素 (1229, 267) → `SUN_DIR = (0.4313, 0.6817, 0.5910)`（elev 42.98° / azim +36.12°）；10° 探针窗口在太阳处 100% 像素 HDR > 1.0、反日方向 mean 0.115，故 0.60/0.55/0.20 三条阈值有约 3 个数量级余量。

## Step 3 — Step 2 — 街道几何与建筑立面（重启（Step 1a/1b/2/3，无时间预算））
- 起止：2026-10-29 02:17:23Z → 2026-10-29 02:49:15Z
- 范围：orion-simulation-world：src/world/ + 新增 check_street
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/task38/plan.md](tmp/task38/plan.md)（502 行）。 **计划摘要**：新增 `StreetLayout.gd`（Road 2 面 / Markings 28 面 / Curbs 7 段×5 面，共 3 个 MeshInstance3D，零碰撞）与 `Facade.gd`（窗 60 / 基座 44 / 压顶 15，3 个 mesh），再加上 `WorldLook.Make_Pbr_Material()` 复用已入库的 asphalt/curb/concrete 贴图（**零新资产**）与 `World.gd` 四处接线。自算定值：主虚线 18、边线 2、支虚线 4、斑马线 4；路面 z=0.01、标线 0.03，压顶按下标错位，防止共面。层数公式加“一档下限”，否则 h=4 的 building_a 会算出 0 窗、与判据 4 冲突（已论证，各障碍层数 4→1/8→2/2→0）。`check_street` S28 + R13（3 张 1600×900），验收共 V1~V14，18 项基线逐字比对。上轮无 issues。

## 渲染环境与 GPU 说明（2026-10-29，人类要求记录）

**结论先说**：这台机器**有 GPU**（vLLM 正用 TP0/TP1 占着两张卡），e2e 里 6.6 fps 不是"机器没卡"，是 **Godot 这条渲染路径没走到 GPU 上**。不是世界做重了。

### 1 证据

- e2e 的 `RobotSim_TestEnv/results/task38_follow_20261029T040108Z/world.log` 第 4 行：
  `OpenGL API 4.5 (Core Profile) Mesa 25.2.8 - Compatibility - Using Device: Mesa - llvmpipe (LLVM 20.1.2, 256 bits)`
  → `llvmpipe` = Mesa 的 **CPU 软件光栅化器**。
- `verdict.md` §0 实测：相机出帧 **6.6 Hz**（配置 `rate_hz=10`）、世界钟/墙钟 **r=0.34**（物理 0.34× 实时）、S3 25.2 m 走了 46 s 墙钟。

### 2 为什么走了软件渲染

1. e2e 跑在 **`Xvfb :99`** 虚拟显示上（task_33/task_34 定的出图流程）：Xvfb 没有 GLX/DRI，Mesa 拿不到硬件设备，只能退回 llvmpipe；
2. 命令行**强制 `--rendering-driver opengl3`**；
3. `project.godot` 里 `renderer/rendering_method="gl_compatibility"`（Compatibility/OpenGL 路径）。

### 3 每帧成本（为什么 CPU 扛不住）

| 项 | 值/来源 |
|---|---|
| 主视口 | 1600×900 = 1.44 MP |
| 相机 SubViewport | 640×480 = 0.31 MP（每帧再渲一遍，合计 ≈1.75 MP） |
| MSAA | `project.godot:105` `msaa_3d=2` = Godot 枚举的 **4×** |
| 材质 | 地面/体素都用**世界空间 triplanar**：每像素最多 3 投影 × 3 图 = **9 次纹理采样**（WorldLook.gd） |
| 天空 | `PanoramaSkyMaterial` + 2K HDRI，每像素一次 panorama 查表 |
| 阴影 | 方向光 `shadow_enabled`，128 m 场景一张大 shadow map |

改前是"纯色 + 无贴图"（一次常量着色），改后是完整 PBR 管线 —— 在 CPU 上差的就是这个量级。

### 4 换到带 GPU 的本地机器怎么跑（不改代码）

1. **别用 Xvfb**：真桌面/真 `DISPLAY` 下 `godot --path orion-simulation-world`；
2. **别强塞 `--rendering-driver opengl3`**（默认会选 Vulkan/Forward+）；
3. 验证：启动日志出现 `Using Device: NVIDIA/AMD/Intel ...` 即为 GPU；出现 `llvmpipe` 仍是软件。

两个坑：
- `project.godot` 的 `renderer/rendering_method="gl_compatibility"` 仍是**项目级锁定**：在 GPU 上它照样用显卡（走驱动 GL），但用不到 SSAO/SSIL；要全特效改成 `forward_plus`（或 `--rendering-method forward_plus`），改完要重新复核像素判据。
- `Tool/sim_follow_test.py` 目前把 `Xvfb/opengl3` 写死在启动参数里 → 到本地跑跟随验证前，需给它加"真实显示/GPU"开关（或用桌面手起世界、脚本只发命令+采样）。

### 5 对既有自检的影响

像素判据（`check_camera_shots` 的 `VAR_MIN/MEAN/WHITE_MAX/GRID_*`）是在 llvmpipe 下标定的，文档留了 3~5 倍余量；换 GPU 后抗锯齿/精度不同，**第一次在本地跑要复核**，红了按"改前/改后实测值"记录后再调，不盲调。

### 6 决策

- **本轮不为此改任何东西**：fps 不阻塞 task 38（Step 3 的判据已按物理时间口径写，与帧率解耦）；
- 后续若要在 llvmpipe 上频繁跑 e2e，再考虑三件：`msaa_3d=0`、主视口降 1280×720、加 `LOOK_LOW_QUALITY` 低配开关（e2e 用低配跑逻辑、出图用高配）；
- 执行器/路径的根因（格心转向、Turning 过冲）归 **task_39**，不在本任务范围。

## Step 4 — Step 3 — 新世界 Follow 复验（e2e，只验不改）（重启（Step 1a/1b/2/3，无时间预算））
- 起止：2026-10-29 02:49:15Z → 2026-10-29 05:25:56Z
- 范围：Tool 复验脚本 + RobotSim_TestEnv 实例配置/结果（不碰 Rust）
- 轮次：3 / 3
- 结果：FAIL（已中断）
- 规划要点：计划已写入 [plan.md](tmp/task38/plan.md)（431 行）。 **关键发现：按 §四.5 的设想，S0b 无法通过**——`goto(64.25,64.25)` 的逐轴距离只有 0.25 m，小于 `ARRIVAL_TOL_M=0.26`（navigator.rs:46/69，②格内逼近直接 `arrive()`）→ 因此 goto 不会改变机身位置，Δψ 恒为 18.43° > 10°；反过来，如果 goto 真的让机身移动，executor 会把机头转向 45°，Δψ≈−45° 同样超过 10°。因此安排两跑：默认 strict 证明会「并停」→ FAIL(env)；`--s0b record` 作为主产物跑完 S1~S6。另外修：判据 6 字面改帧占比（`frame6_ok`/`literal_s3_frames`、§3.4 拆行）、删 4 个死常量并让 `D_FAR/D_NEAR/STALE` 真被引用、impl 数值逐字取自 verdict。G0~G9、V1~V15 均到行号。
- 3 轮未通过的问题：
判据 1 未真满足（Task/task_38_sim_world_refine.md:319，§四.6「完成判据」第 1 条）：期望 S3 后 10 s 内 d∈[3.5,6.0] m 的采样占比 ≥80% 且 min d ≥2.5 m；实际 RobotSim_TestEnv/results/task38_follow_20261029T050752Z/verdict.md:99 与 verdict.json judges.1 为 share=0.00、min=18.626、mean=24.465、n=323（min 达标但 share=0.00 < 0.80）→ 判据 FAIL，verdict 的 overall=FAIL(env) 里也列了 [1,6]。复现：python3 -c "import csv;R='RobotSim_TestEnv/results/task38_follow_20261029T050752Z';s=[r for r in csv.DictReader(open(R+'/trace.csv')) if r['seg']=='S3'];x=max(float(r['ped_x']) for r in s);ds=[float(r['d_m']) for r in s if r['d_m'] and float(r['ped_x'])>=x-12.6];print(len(ds),round(sum(1 for d in ds if 3.5<=d<=6.0)/len(ds),4),min(ds))" → 323 0.0 18.626。
判据 6 未真满足，且主口径窗口算错（Task/task_38_sim_world_refine.md:324 §四.6-6；Tool/sim_follow_test.py:1819-1825）：期望「S3 后 10 s 内」person(conf≥0.5 且 bbox 底边>240) 的帧占比 ≥0.90；实际 0/75（share=0.00）→ FAIL。verdict.md:104 报的 0/117 把 S3 结束后（墙钟 >26512598.723）的 42 帧（S4/S5）也算进 win_frames——该循环只判 `fr['wall'] >= s3.t_start`，没有上界；同一脚本判据 7 的窗口（:1849-1851）用的正是 `<= s3 的 t_stop`。正确窗口（S3 内且 ped_x≥82.604）为 75 帧、good=0，结论仍是 FAIL，但 n 与 §四.6 的窗口不一致。复现：python3 -c "import csv;R='RobotSim_TestEnv/results/task38_follow_20261029T050752Z';fr=list(csv.DictReader(open(R+'/detector_frames.csv')));ped=[(float(p['mono']),float(p['x'])) for p in csv.DictReader(open(R+'/ped_trace.csv'))];sel=[f for f in fr if float(f['wall'])>=26512527.943 and any(abs(m-float(f['wall']))<=0.25 and x>=82.604 for m,x in ped)];print(len(sel),len([f for f in sel if float(f['wall'])<=26512598.723]))" → 117 75（脚本口径 117 / S3 内 75）。
S0b 验收前置未真满足（Task/task_38_sim_world_refine.md:301 §四.5）：期望两次 goto(64.25,64.25) 后 |Δψ|≤10°；实际两次尝试均 Δψ=+18.43°、moved=0.000 m（goto 为 no-op，sim 日志两条 `[Nav] 已在目标格且逐轴误差 <= 0.26 m，到达 (64.25, 64.25)`），verdict.md:65-72 记 FAIL(env) → e2e 不是在 §四.5 规定的初始条件下执行。且 §四.5 要求 S0b 不过即 strict 停，而含判据 1~10 的正式产物 ...T050752Z 是 `--s0b record` 跑出来的（S0b 不过仍继续 S1~S6）；strict 跑 ...T050716Z_s0b-strict 停在 S0b，没有 S1~S6/判据 1~10（trace.csv 只有 S0j/S0b 段、ped_trace 仅表头）。即：§四.5 的合规验收路径从未跑到 e2e。复现：grep -n '已在目标格' RobotSim_TestEnv/results/task38_follow_20261029T050752Z/sim_trace.log → 2 条；grep -c 'pedestrian spawned' RobotSim_TestEnv/results/task38_follow_20261029T050716Z_s0b-strict/world.log → 0；python3 -c "import csv,collections;print(collections.Counter(r['seg'] for r in csv.DictReader(open('RobotSim_TestEnv/results/task38_follow_20261029T050716Z_s0b-strict/trace.csv'))))" → {'S0j':100,'S0b':10}。
重跑未按 step.md 在 verdict.md 注明：RobotSim_TestEnv/results/task38_follow_20261029T050716Z_s0b-strict/verdict.md:13 写 `attempt=1` + `rerun_reason=（未重跑）`，但 tmp/task38/impl.md:191（§7 B2）自述该目录是首跑目录 …T050634Z_s0b-strict（harness 缺陷 B1）被删后重跑的产物；step.md「唯一例外：harness 自身故障…可修脚本重跑一次，并在 verdict.md 注明」要求在该 verdict 里写明重跑原因。复现：grep -n '本轮尝试' RobotSim_TestEnv/results/task38_follow_20261029T050716Z_s0b-strict/verdict.md；grep -n 'B2' tmp/task38/impl.md。

## 引擎结束
- 结束时间：2026-10-29 05:25:56Z
- 状态：ABORTED — Step 4（Step 3 — 新世界 Follow 复验（e2e，只验不改））3 轮未通过
- git status（结束时）：
```
 M orion-simulation-world/COMPONENTS.md
 M orion-simulation-world/assets/README.md
 M orion-simulation-world/src/world/World.gd
?? Task/task_38_sim_world_refine.md
?? Task/task_39_executor_and_path.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task38.config.json
?? Tool/execution_engine_task38.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Tool/sim_follow_test.py
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_38_sim_world_refine.md
?? orion-simulation-world/assets/hdri/sky_midday_2k.hdr
?? orion-simulation-world/assets/hdri/sky_midday_2k.hdr.import
?? orion-simulation-world/assets/textures/asphalt_color.jpg
?? orion-simulation-world/assets/textures/asphalt_color.jpg.import
?? orion-simulation-world/assets/textures/asphalt_nor.jpg
?? orion-simulation-world/assets/textures/asphalt_nor.jpg.import
?? orion-simulation-world/assets/textures/asphalt_orm.png
?? orion-simulation-world/assets/textures/asphalt_orm.png.import
?? orion-simulation-world/assets/textures/concrete_color.jpg
?? orion-simulation-world/assets/textures/concrete_color.jpg.import
?? orion-simulation-world/assets/textures/concrete_nor.jpg
?? orion-simulation-world/assets/textures/concrete_nor.jpg.import
?? orion-simulation-world/assets/textures/concrete_orm.png
?? orion-simulation-world/assets/textures/concrete_orm.png.import
?? orion-simulation-world/assets/textures/curb_color.jpg
?? orion-simulation-world/assets/textures/curb_color.jpg.import
?? orion-simulation-world/assets/textures/curb_nor.jpg
?? orion-simulation-world/assets/textures/curb_nor.jpg.import
?? orion-simulation-world/assets/textures/curb_orm.png
?? orion-simulation-world/assets/textures/curb_orm.png.import
?? orion-simulation-world/assets/textures/ground_paving_color.jpg
?? orion-simulation-world/assets/textures/ground_paving_color.jpg.import
?? orion-simulation-world/assets/textures/ground_paving_nor.jpg
?? orion-simulation-world/assets/textures/ground_paving_nor.jpg.import
?? orion-simulation-world/assets/textures/ground_paving_orm.png
?? orion-simulation-world/assets/textures/ground_paving_orm.png.import
?? orion-simulation-world/src/tests/check_street.gd
?? orion-simulation-world/src/tests/check_street.gd.uid
?? orion-simulation-world/src/tests/check_world_look.gd
?? orion-simulation-world/src/tests/check_world_look.gd.uid
?? orion-simulation-world/src/world/Facade.gd
?? orion-simulation-world/src/world/Facade.gd.uid
?? orion-simulation-world/src/world/StreetLayout.gd
?? orion-simulation-world/src/world/StreetLayout.gd.uid
?? orion-simulation-world/src/world/WorldLook.gd
?? orion-simulation-world/src/world/WorldLook.gd.uid
```

## 收尾结论（2026-10-29 05:25Z）

| Step | 结果 | 说明 |
|---|---|---|
| 1a 材质资产 | **PASS** | 4 套 CC0 PBR + 2K HDRI，assets 增量 9.3 MB |
| 1b 材质接线 + 天空 | **PASS** | `WorldLook.gd`（triplanar 6 m/2 m、PanoramaSky、太阳对齐）、`check_world_look` 全绿 |
| 2 街道 + 立面 | **PASS** | `StreetLayout.gd`（2 路面 + 28 标线 + 7 段路缘，3 mesh、零碰撞）、`Facade.gd`（60 窗/44 基座/15 压顶，3 mesh） |
| 3 新世界 Follow 复验 | **FAIL（3 轮未过，引擎中断）** | 卡在动作层，不是世界/检测/投影 |

**Step 3 的失败链（实测，可复核）**

1. 机位 (64,64) 落在 0.5 m 网格的**格角**；叶子给的首格目标格心是 (64.75,64.25)，相对机头 **18.43°**；
2. executor 追格心、`Turning` 过冲不反向修正，实测机头转到 **47.2°**，越过相机半视场 **43.03°** → 人出画；
3. dets 归零 → `stale 800 ms` → 树进 `Hold`；S2→S3 机身只动 **0.213 m**，于是判据 1（距离带 share=0.00、min=18.63）与判据 6（person 帧 0/75）双 FAIL；
4. **S0b 归位按原设计不可行**（review 实测）：`goto(64.25,64.25)` 的逐轴距离 0.25 m < `ARRIVAL_TOL_M=0.26` → navigator 直接判"已在目标格"、`moved=0.000`，Δψ 恒 18.43°；反过来若真让它移动，executor 会把机头转到 ~45°，Δψ≈−45°，同样过不了 10° 门槛。

**世界侧其实是被证实的**（这部分可以放心）：

- 投影（判据 8）**PASS**：3~6 m 误差中位 **0.073~0.112 m**、max 0.221~0.405 m；6~8 m 无离谱点；
- 检测（判据 6 的成因）：人出画前有 **19 帧 person、conf 最高 0.90**，末帧距离 7.62 m；
- 误检（判据 7）：新贴图/窗格/斑马线在 S3 内 **0 个**非 person（conf≥0.5）→ 新世界没有引入误检。

**结论**：Step 1/2 达成目标（城市街道 + PBR + HDRI + 立面），Step 3 的失败**完全落在执行器/路径层**，即 `Task/task_39_executor_and_path.md` 的范围（4 连通→8 连通、连续运动控制）。本任务按纪律**没有**改任何 Rust、没有调 follow 参数/`conf_thres`/判据阈值、没有为了过判据改世界外观。

**后续**：task 39 落地后，用同一套 `Tool/sim_follow_test.py` 原样复跑 Step 3（那时 S0b 可以删掉——8 连通 + 连续运动后不再有"格心 18°"的起步偏差）。

**harness 已知瑕疵（不影响上述结论，留给下次修）**：判据 6 的窗口上界算错（把 S4/S5 的 42 帧算进来，正确窗口 75 帧、结论不变）、重跑未在 verdict 注明、`--pass-b` 是空开关。
