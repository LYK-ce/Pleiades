# wb_50_demo — 国赛答辩演示：散场封路 + 一人奔跑 + 无人机跟踪 + 四车围控（资产 → 落地 → 场景 → 冒烟） 工作记录

> Presented by KeJi
> Created Date ： 2026-11-07
> Modified Date ： 2026-11-07
> 任务：Task/task_50_demo.md

## 执行引擎启动
- 开始：2026-11-07 12:53:37Z
- 引擎：tmp/task50_tools/（流程照搬 task_49）
- 标记：task_50 首轮
- 计划：S1 → S2 → S3 → S4 → S5

## Step 1 — 资产步：三个人物 + 巡逻车 + 2 款轿车 + 背景楼（只入库，不接线）（task_50 首轮）
- 起止：2026-11-07 12:53:37Z → 2026-11-07 13:54:58Z
- 范围：orion-simulation-world/assets/（humans / chassis / props）+ tmp/task50/ 证据；不写接线代码
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 7c3625b
- 规划要点：计划已写入 [plan.md](tmp/task50/plan.md)（580 行，含 9 节：改动清单到行号、脚本全文、P0~P6 命令、17 条判据、13 条风险）。全程未碰任何源码（`git status orion-simulation-world/` 为空）。 关键实证（已在 `tmp/task50/proj/` 跑通，非推测）：Godot 4.7 的 ufbx 能直接导入 FBX，sophia/manuel 都与 nathan 同为 88 骨、命名同构 → walk 可直接套用，无需 Blender；两人原生 clip 第 0 帧都是 T-pose（`hand y` 1.411/1.486 → 之后 0.85/0.9），故必须裁剪首帧；两人都朝 +Z（依据 `eye_l` 相对 `head` 的偏移 +0.090/+0.074 判断），需绕 Y +90°；Godot 的 `get_animation_list()` **按名字排序**（实测 JSON 顺序 `[walk, idle]` 仍返回 `[idle, walk]`），因此 clip 必须命名为 `0_walk`/`1_idle`/`1_dancing`；glTF 默认把图嵌成 PNG，需后处理 dif→JPEG，才能得到 `model_*_dif.jpg`/`_norm.png` 的 6 文件命名。 **摘要（约 195 字，可直接写进 workbook）** 计划见 tmp/task50/plan.md。已实证：Godot ufbx 直读 FBX，两人 88 骨与 nathan 同构、walk 可套；首帧 T-pose 须裁；均朝 +Z，绕 Y +90° 即 +X；Godot 按名排序 clip，须命名 0_walk/1_idle/1_dancing 才保 clip[0]=walk；导出内嵌 PNG，须后处理 dif→JPEG 才得 6 文件命名。成品 1.7480/1.8883 m、脚底 y=0、贴图 2048/1024。车/楼限额搜索（kenney/quaternius 可达，KayKit 超

## Step 2 — 新车落地：碰撞 / 雷达 / 相机 / 速度（task_50 首轮）
- 起止：2026-11-07 13:54:58Z → 2026-11-07 14:18:44Z
- 范围：orion-simulation-world/src/chassis/ 与 src/scenes/chassis/ 与 assets/chassis/ 与 RobotSim_TestEnv/ 的实例 config；不改 Rust
- 轮次：1 / 3
- 结果：PASS
- 提交：ok f2f254f
- 规划要点：计划已写入 `tmp/task50/plan.md`（632 行）。工作树未被改动；`git status` 仍只有进场时的那三项基线，没有改动任何源码。 计划摘要（约 190 字，可直接写入 workbook）： 计划见 tmp/task50/plan.md。`Vehicle.tscn` 不在允许路径，所以更换外观的唯一入口是重写 `assets/chassis/ugv_m1.glb`：改动 glTF JSON 的 scale，把巡逻车归一为 0.80×0.50×0.55，并保持不变 bin。由于模型就是碰撞盒（D11），碰撞盒必须等于外观，因此新增 `body_length 0.80`，并保留 `wheel_base 0.50`、`track_width 0.55`、`height 0.50`。雷达按口径 #9 装到车顶 `(0.02,0,0.50)`（0.30~0.35 会埋进 0.50 高的车体）。速度 2.2、180、cruise 2.0，以及跟踪机 3.0 通过 chassis_params 配置，车端 detector 关闭。xvfb 探针验证 2.2、π、贴墙和扫两侧墙，并记录 Rust `SpeedBinding` 把实速二次限制到 1.5、π/2 的已知缺口。 两个必须先看的硬发现： 1. **外观入口受限**：`Vehicle.tscn:18` 把 ModelUGV 固定指向 `ugv_m1.glb`，而 `src/scenes/`、`src/vehicle/` 都不在允许路径，因此只能重写该 glb 的字节；文件名沿用会在新增的 `assets/chassis/README.md` 中说明。 2. **Rust 侧速度上限**：`pleiades-sim/.../action_map.rs` 的 `MAX_CRUISE_SPEED=1.5`、`MAX_CRUISE_YAW_RATE=π/2` 会把 `cruise_speed 2.0` 限制成 1.5。因此脚本或行为树驱动的车实测达不到 2.2 和 π；世界侧 clamp 可以用直发 `motio

## Step 3 — 模拟世界：行人可选模型 + 轿车底盘（只平移）（task_50 首轮）
- 起止：2026-11-07 14:18:44Z → 2026-11-07 14:50:51Z
- 范围：orion-simulation-world/src/（chassis、scenes、net）加新增 Sedan 组件；不改 Rust
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 72ee3be
- 规划要点：计划已写入 [plan.md](tmp/task50/plan.md)（710 行，共 10 节），未改动任何源码（`git status` 仍与进场基线相同）。 计划摘要（约 190 字，可直接写进 workbook）： 计划见 tmp/task50/plan.md。改 3 文件：Catalog 加 sedan；Pedestrian 按 model 换 Body 并重找动画器（未知 warn 回落）；Vehicle 加 SELF_VISUAL_CHASSIS 防占位盒包车。新增 Sedan.gd/tscn：只认 v、yaw 恒 0 只平移；朝向 spawn 写宿主 rotation.y，位置仍由 Vehicle 推进；碰撞=外观 4.5×1.5×1.8。验收 import/quit/check-only + headless 探针 + xvfb 图。硬发现：Rust 镜像只认 2 chassis，sedan 需独立 ws 客户端驱动。

## Step 4 — 世界搭建：第三个场景 venue（task_50 首轮）
- 起止：2026-11-07 14:50:51Z → 2026-11-07 15:18:14Z
- 范围：orion-simulation-world/src/world 与 src/scenes 与 COMPONENTS.md；全视觉零碰撞
- 轮次：1 / 3
- 结果：PASS
- 提交：ok d91bb10
- 规划要点：计划已写入 [plan.md](tmp/task50/plan.md)（804 行，15 节）。规划期只读，未改任何源码（`git status` 与进场基线一致，无残留进程）。 **摘要（约 190 字，可直接写进 workbook）** 计划见 tmp/task50/plan.md。改 6 处：SceneDefs 加 venue（obstacles=[]、spawn(68,82)、look=venue、新增 buildings 4 栋）；新建 Plaza.gd（19 面：路1/白条6/人行道4/场馆7/门洞1，全 mesh 零碰撞）；World.gd 加 3 行分派；WorldLook 加 Apply_Venue（只调环境色与曝光）；新建 WorldVenue.tscn（复制 WorldIndoor 仅改 scene_name）；COMPONENTS 6 处。实测：4 栋楼 scale 5 摆位全入框不挡镜头；city 12104 格 / corridor 852 格为回归基线。验收 import/quit/check-only + xvfb 出图亮度判据 + city 像素 sha256 对照。 **两条硬结论（已实测，非推测）**：4 栋楼 glb 每栋 1 mesh、0 碰撞体、`min.y=0`、水平居中，摆位只需 position+yaw+均匀 scale；`ambient_light_sky_contribution < 1` 是 SKY 源下唯一有效的"环境色"旋钮，不改 `ambient_light_source`（不违 E30-③）。

## Step 5 — 验证：这个场景能不能跑起来（只验能跑）（task_50 首轮）
- 起止：2026-11-07 15:18:14Z → 2026-11-07 16:11:24Z
- 范围：tmp/task50/ 证据与 RobotSim_TestEnv/ 实例；零源码改动
- 轮次：1 / 3
- 结果：PASS
- 提交：ok e60dd32
- 规划要点：计划已写入 [plan.md](tmp/task50/plan.md)（693 行；仓库源码零改动，`git status` 与进场基线一致，探针进程与端口已收干净）。 **摘要（约 195 字，可直接写进 workbook）** 计划见 [plan.md](tmp/task50/plan.md)（693 行，零源码改动）：新增 tools/ 六脚本、RobotSim_TestEnv/t50_s5/ 16 套夹具、s5/ 证据。世界一律 venue + xvfb/opengl3/Dummy；L1（5 avatar）过才进 L2（28）。16 台走静态 bootstrap_peers（先空表抓 PeerId 再写表）；端口 9999 / 9091~9106（9090 属 mihomo）/ 40401~40416，行人不监听。实测空 venue 1600×900 = 17 fps，备四档降窗口阶梯。行人先连走 13 s、车后连（实测车把行人当雷达障碍）；车速 1.2 m 短腿 + stop 兜底。omega 世界 180 / 链路 90 两值都测。A5 另跑近机位副本（远端仅 40 px）。收尾 trap + ss 三次快照。

## 引擎结束
- 结束：2026-11-07 16:11:26Z
- 状态：ALL_STEPS_DONE
- git status：
```
M Workbook/wb_50_demo.md
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
```
