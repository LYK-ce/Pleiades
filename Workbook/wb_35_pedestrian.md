# wb_35_pedestrian — task_35 工作记录

> Presented by KeJi
> Created Date ： 2026-10-25
> Modified Date ： 2026-10-25
> 任务：Task/task_35_pedestrian.md（行人底盘：Python 经 WS 直接驱动）

## 执行引擎启动
- 引擎：Tool/execution_engine_pedestrian.ts + Tool/execution_engine_pedestrian.config.json
- 计划：Step 1~5 串行，每 step 最多 3 轮（规划 → 实现 → 评审）
- 范围：仅 orion-simulation-world/（含 tool/）与 tmp/task35/；不涉及 SLAM/建图

## Step 1 — 世界侧行人底盘（场景 + ChassisModel + 目录 + 自检）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task35/plan.md)（522 行）：文件清单与逐字签名、`v/dir` 与 `action/speed` 解析规则、40 条自检断言（含期望数值）、衔接点、可复制验收命令 + 本轮亲测的 15 项基线 total、13 条风险。 规划期用一次性探针（已删除，仓库无残留）实测了两个致命点：`extends ChassisModel` 的脚本**能**挂 `CharacterBody3D` 根，但 `self as CharacterBody3D` 直接 Parse Error（必须经 `Node` 中转）；`Step()` 里若调 `move_and_slide()`，树外调用立刻 `ERROR: !is_inside_tree()` —— 所以定案"控制量/推进分家"，`move_and_slide()` 只出现在 `_physics_process`（60 帧实测 dx=1.2602 m、90° 阶跃 30 帧转够、dy=0）。 关键取舍已记录：行人自宿主推进；**不另建 AnimationPlayer 节点**（glb 内嵌那个就是动画节点，另建会让 `find_child` 命中空壳）；`DEFAULTS` 只加 step.md 的两键（任务文档的 `turn_rate_deg` 别名有"逐键一致"冲突，已在 §6 R7 给出评审若坚持时的三处同步方案）。 本轮只产出 `tmp/task35/{plan.md,logs/s1_*}`，`orion-simulation-world/` 一行未改、未 git add/commit、未动 Xvfb。

## Step 2 — 协议扩展：kind + motion_cmd{v,dir} + 超时停车
- 轮次：2 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task35/plan.md)。规划期间实测：headless `total=73 failed=0`；Xvfb `total=74 failed=2`（K4、H5）、整轮 50.95 s，反推 ≈70~100 ms/帧——K4 的 60 帧窗口里只走了 3 帧（x 位移 0.0499 m），H5 是「Heartbeat 每 60 引擎帧才巡检」被纯墙钟 2.5 s 等，两者同源于墙钟/引擎帧混用（H5 在 `git diff` 里未被改动）。 本轮落点：`_RemoveAvatar()` 改为不分类型统一清场（返回 bool、只在真摘到时 emit/print、判死刑连接交出 `link.vehicle`）+ `_CleanupLink` 所有权守卫 + `_ApplyMotionCommand` 按 `link.vehicle` 身份分流，配 K14/K15/K16 三条跨 kind 用例（信号计数精确数组相等，锁死"幽灵移除"）；K4/K5/K6 改每物理帧重发、H5 改「≥70 引擎帧 + ≥400 ms 墙钟」双条件，K12 追加 `not has("geometry")`。 验收口径写死为**两档都是硬门槛（权重 1）**：headless `total=76 failed=0`、Xvfb `total=77 failed=0`（J7 计入），并给出「既有 59 项一条不少」的标签 diff 机器化命令与 13 个既有自检的期望 total 表；文档改动 C1~C8 明确要求先跑后填数字。

## Step 3 — 世界布局：用外部驱动的 PEDESTRIANS 替换自动绕圈的 WALKERS
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task35/plan.md)（404 行，含 6 张事实/数据表、15 条风险与可复制验收命令）。 - **核心改动 3 处代码**：`World.gd` 把 `WALKERS/BuildWalkers()` 整节换成 `PEDESTRIANS/BuildPedestrians()`（2 人：世界 `(44,96)` 朝东、`(52,102)` 朝西，进与 manager 同名的 `World/Pedestrians`，摆完必须 `Configure(ped,{})` 对齐 `_yaw_target`，否则朝向会被拧回 0）；`check_person_walk` 20→15（保留资产 8+场景 4，C 组改为容器/选点净空/幂等，删自动走动 D5 与旧环路渲染 R6）；`check_person_walk_shots` 4/14 总数不变但主语改为外部驱动行人（`SetCommand` 驱动、R7 尺子从弧长换成 `GetPose().x`）——**第三处是 step.md 名单外但"全部自检全绿"强制**，已单列理由。 - **选点全部算过**：最近障碍 3.0/8.06 m、离 4 面边界 ≥26 m、离 spawn 37.7/39.8 m、离 `check_lidar` 真墙热区 17.1/14.4 m（> 12 m range，保 F7 `2.0000`/F8 `152`）、离协议 spawn 点 `(64,56)/(70,70)/(72,78)` 全部 >30 m（避免叠人）。 - **衔接面已核**：manager 第 2 轮就为 `BuildPedestrians()` 重建容器写好了 `is_instance_valid` 重解析；`check_ws_manager` 只查容器名不数节点；`check_bounds` 的 Mover `mask=1`、`check_person_assets` 只动 Persons、`check_person_shots/camera_shots` 机位均在 19 m 外。 -

## Step 4 — Python 客户端（orion-simulation-world/tool/pedestrian/）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task35/plan.md)。全篇共 11 节，内容包括：文件清单与职责、协议字段、客户端与 `ws_min` 的 API 签名、巡逻状态机、与既有代码的衔接点、A~G 七组可直接复制执行的验收命令、15 条风险与对应处理办法，以及提交前自检清单。 本轮实测并固定下来的关键事实：headless 世界默认监听 9999，且**按真实时间推进**（`--max-fps 60` 下 `v=1.0 驱动 3 s → dx=2.90 m`，ratio 0.97；16 s 长连跨 10 s 心跳不失联），`pose` 10 Hz，默认出生点 (64,56)，停发 300 ms 会自动停车，`--quit` 基线 `EXIT=0`、0 ERROR。结论是用 `--max-fps 60` 验收，**禁用 `--fixed-fps`**（会让仿真比墙钟快）。plan.md 中列出了证据日志，位于 `tmp/task35/logs/plan_*`。 方案要点：只新增 `tool/.gdignore`（0 字节；先建它再跑 Godot）、`tool/pedestrian/{ws_min,pedestrian_client,patrol}.py`（正好对应 step 要求的“三个 .py”）、`README.md`，并在 `orion-simulation-world/.gitignore` 追加一行 `__pycache__`；**不改任何 GDScript**，所以 Step 1~3 的 14+2 个自检与 `check_ws_manager`(76)/`check_pedestrian`(40) 都不可能回归。 两处需要实现者特别注意：`spawn` 在世界侧是“warn + 回退默认点”而非拒绝，必须比对 `chassis_ready.spawn` 回显并 exit 5；`--linger` 是验证 300 ms 超时的唯一手段（靠 10 Hz pose 观测，判据写 0.30~0.45 s）。 本轮只写了 `tmp/task35/`（pla

## Step 5 — 端到端验证：走、转、超时、人撞车、雷达可见、三帧出图
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task35/plan.md)（556 行，§0~§7）。 **本轮规划的核心发现（均已实跑验证，写进 §2）**：`--fixed-fps` 在 headless 下关掉实时同步，世界跑到墙钟的 **21~37 倍**（`--max-fps` 救不了，人在 3 s 内跑 64 m 撞上东侧空气墙）→ 第 1~3 项必须 `--headless --max-fps 60`（实测 ratio 0.97/1.00、pose 10.0 Hz），第 4/5/6 项才用 `--fixed-fps 60`；这是最容易让实现者"统一参数"后全线假红的一处。规划期同场量到的真值（可实现性证明）：直行 `v_eff=1.0003 m/s`、`dir` 阶跃在 **0.500 s** 内收敛（=180 °/s）、弧半径 **0.3199 m vs v/ω=0.3183（+0.5%）**、停发后 **0.300 s（世界）** `v=0` 且随后 0.9 s 位移 **0.000000 m**、idle 日志恰好 1 行 0 ERROR。 **交付设计**：生产代码一行不改，只新增 `check_pedestrian_physics.gd`（S/P/L 三组 ≈24 项，车静止 + 推 300 帧不穿透/车位移 <1 mm + 雷达 ±45° 命中 2.25 m）与 `check_pedestrian_shots.gd`（A 4 + R 10，`(44,96)` + `(48,101)` 老机位拍走/转/停，headless 下 R 组 10 条 skip 不计 total）；`tmp/task35/` 下 `e2e_drive.py`（复用 `pedestrian_client.py`，零 numpy，纯 stdlib 圆拟合）、`s5_e2e.sh`、`s5_checks.sh`（17 项既有基线逐字比对 + 4 项渲染档）。 **三条最危险的坑已定位**：雷达必须装在**车头** `x=0.15/z=0.6`（装原点差 0.15 m 必红、装 0

## 引擎结束
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M .gitignore
 M orion-simulation-world/.gitignore
 M orion-simulation-world/COMPONENTS.md
 M orion-simulation-world/src/chassis/ChassisCatalog.gd
 M orion-simulation-world/src/net/WebSocketManager.gd
 M orion-simulation-world/src/net/WireProtocol.gd
 M orion-simulation-world/src/tests/check_ws_manager.gd
 M orion-simulation-world/src/world/World.gd
?? Task/task_35_pedestrian.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_human_model.config.json
?? Tool/execution_engine_human_model.ts
?? Tool/execution_engine_pedestrian.config.json
?? Tool/execution_engine_pedestrian.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/execution_engine_walking_person.config.json
?? Tool/execution_engine_walking_person.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_35_pedestrian.md
?? orion-simulation-world/assets/humans/LICENSE_renderpeople.txt
?? orion-simulation-world/assets/humans/README.md
?? orion-simulation-world/assets/humans/rp_carla_rigged_001/model.glb
?? orion-simulation-world/assets/humans/rp_carla_rigged_001/model.glb.import
?? orion-simulation-world/assets/humans/rp_carla_rigged_001/model_rp_carla_rigged_001_dif.jpg
?? orion-simulation-world/assets/humans/rp_carla_rigged_001/model_rp_carla_rigged_001_dif.jpg.import
?? orion-simulation-world/assets/humans/rp_carla_rigged_001/model_rp_carla_rigged_001_norm.png
?? orion-simulation-world/assets/humans/rp_carla_rigged_001/model_rp_carla_rigged_001_norm.png.import
?? orion-simulation-world/assets/humans/rp_eric_rigged_001/model.glb
?? orion-simulation-world/assets/humans/rp_eric_rigged_001/model.glb.import
?? orion-simulation-world/assets/humans/rp_eric_rigged_001/model_rp_eric_rigged_001_dif.jpg
?? orion-simulation-world/assets/humans/rp_eric_rigged_001/model_rp_eric_rigged_001_dif.jpg.import
?? orion-simulation-world/assets/humans/rp_eric_rigged_001/model_rp_eric_rigged_001_norm.png
?? orion-simulation-world/assets/humans/rp_eric_rigged_001/model_rp_eric_rigged_001_norm.png.import
?? orion-simulation-world/assets/humans/rp_nathan_animated_003_walk/model.glb
?? orion-simulation-world/assets/humans/rp_nathan_animated_003_walk/model.glb.import
?? orion-simulation-world/assets/humans/rp_nathan_animated_003_walk/model_rp_nathan_animated_003_dif.jpg
?? orion-simulation-world/assets/humans/rp_nathan_animated_003_walk/model_rp_nathan_animated_003_dif.jpg.import
?? orion-simulation-world/assets/humans/rp_nathan_animated_003_walk/model_rp_nathan_animated_003_norm.png
?? orion-simulation-world/assets/humans/rp_nathan_animated_003_walk/model_rp_nathan_animated_003_norm.png.import
?? orion-simulation-world/src/chassis/Pedestrian.gd
?? orion-simulation-world/src/chassis/Pedestrian.gd.uid
?? orion-simulation-world/src/props/Person.gd
?? orion-simulation-world/src/props/Person.gd.uid
?? orion-simulation-world/src/scenes/props/Pedestrian.tscn
?? orion-simulation-world/src/scenes/props/Person.tscn
?? orion-simulation-world/src/scenes/props/PersonWalking.tscn
?? orion-simulation-world/src/tests/check_pedestrian.gd
?? orion-simulation-world/src/tests/check_pedestrian.gd.uid
?? orion-simulation-world/src/tests/check_pedestrian_physics.gd
?? orion-simulation-world/src/tests/check_pedestrian_physics.gd.uid
?? orion-simulation-world/src/tests/check_pedestrian_shots.gd
?? orion-simulation-world/src/tests/check_pedestrian_shots.gd.uid
?? orion-simulation-world/src/tests/check_person_assets.gd
?? orion-simulation-world/src/tests/check_person_assets.gd.uid
?? orion-simulation-world/src/tests/check_person_shots.gd
?? orion-simulation-world/src/tests/check_person_shots.gd.uid
?? orion-simulation-world/src/tests/check_person_walk.gd
?? orion-simulation-world/src/tests/check_person_walk.
```
