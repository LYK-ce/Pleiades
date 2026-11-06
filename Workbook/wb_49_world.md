# wb_49_world — 多场景化世界：场景数据化 + 室内走廊（Guard 演示场）+ 阶梯规模验证 工作记录

> Presented by KeJi
> Created Date ： 2026-11-06
> Modified Date ： 2026-11-06
> 任务：Task/task_49_world.md

## 执行引擎启动
- 开始：2026-11-06 12:09:28Z
- 引擎：tmp/task49_tools/（流程照搬 task_48）
- 标记：task_49 首轮
- 计划：S1 → S2 → S3 → S4 → S5 → S6 → S7

## Step 1 — 场景数据化（城市零变化）+ 两个入口（task_49 首轮）
- 起止：2026-11-06 12:09:28Z → 2026-11-06 12:20:38Z
- 范围：SceneDefs.gd（新增）+ World.gd 数据化 + WebSocketManager 的 spawn 校验 + 两个入口 .tscn
- 轮次：1 / 3
- 结果：PASS
- 提交：ok d48005d
- 规划要点：计划已写入 [plan.md](tmp/task49/plan.md)（721 行，逐文件与行号）。摘要：①新增 `SceneDefs.gd`：`SCENES` 两档，`Has`/`Get` 深拷贝；city 的 15 条逐字搬入；indoor 骨架 `obstacles=[]`，8 条用注释预置，留待 Step 2。②`World.gd` 15 处：`scene_name`→项目设置→city；`_def` 最先解析；三个访问器在数据未就绪时回退 city；街道、立面按开关控制；空气墙几何表不动，只改日志。③`WebSocketManager` 9 处只换读数来源（含 `_PickSpawn`）。④新增 `WorldIndoor.tscn`（E20 三行照抄），`World.tscn` 加 1 行。判据：city 仍为 12104 cells；四行几何日志 diff 为空；import、quit 零 ERROR；未知名回退验证后须还原；体素对齐四条规则逐条核对。

## Step 2 — 室内走廊体素几何（墙 / 门洞 / 房间）（task_49 首轮）
- 起止：2026-11-06 12:20:38Z → 2026-11-06 12:27:56Z
- 范围：SceneDefs 的 corridor_indoor.obstacles（8 条，0.5 对齐）
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 72c2eaf
- 规划要点：计划已写入 [plan.md](tmp/task49/plan.md)（222 行，逐文件、逐行号、可直接照做）。 **摘要（≤200 字）** 计划见 [plan.md](tmp/task49/plan.md)。只改 [SceneDefs.gd](orion-simulation-world/src/world/SceneDefs.gd) 两处：55–65 行注释和 68 行 `"obstacles"`，写入 8 条 AABB，坐标一字不动。关键：8 条原始共计 864 格，房间南北墙与东墙在角部重叠 12 格，按 `_set_voxel` 去重后为 **852 格**（同一算法在 city 上得出 12664→12104，与引擎实测一致），不是文档估算的 ≈550；只有当实际值不等于 852 时才逐条排查，禁止改坐标凑数。门洞为 cell 127、128，恰好 2 格空；西端 cell 99 处封死。验收标准：city 仍为 12104 格且四行日志不变，indoor 出现 `obstacles=8` 和 `852 cells`，没有出生点净空告警，import 和 quit 零 ERROR，改动前后 diff 仅涉及 1 个文件且不含 Mesh 或 Collision。②–⑤ 的车和雷达现象按 step.md:42 禁止启动长驻进程，延后到 Step 6 复验，计划里附了手算证明与放行时的命令。

## Step 3 — 贴图资产：选型与入库（只入库，找不到就放弃）（task_49 首轮）
- 起止：2026-11-06 12:27:56Z → 2026-11-06 12:39:38Z
- 范围：assets/textures/ + assets/README.md 台账 + tmp/task49/assets/ 证据；不写接线代码
- 轮次：1 / 3
- 结果：PASS
- 提交：ok bf252a2
- 规划要点：计划已写入 [plan.md](tmp/task49/plan.md)（514 行，逐文件 : 行号）。`src/`、`assets/` 均零改动，`git status` 与基线一致。 **摘要（写入 workbook，≤200 字）** > Step 3 走「找到」路径：白名单第 1 站 polyhaven.com 一次命中，两套 CC0 贴图的三通道 URL 已用 HEAD 实测得到 200 状态码，字节数也已固定，分别是 `white_plaster_02`（墙面，527219/652310/191017）与 `terrazzo_tiles`（地面，1166107/77954/477964）。规划期只用 HEAD 探测，没有下载任何内容。执行期不调用任何 API：先把 6 个 1K 文件下载到 tmp/dl，再按字节硬门校验，对 4 个 color、nor 文件做字节级 `cp`，把 2 个 `arm` 文件无损转成 `_orm.png`（须为 .png，因为 WorldLook.gd:184 硬编码），最后让接缝比 ≤3.0，使 `--import` 0 ERROR 且 `ls` 显示 12 行成对。README 改 6 处，从大行号往小行号改，并让 sha、字节逐行对齐；台账另记体积。放弃路径已固定为「删 6 图+6 import、git checkout README」。四条体素规则逐条声明不触碰。首轮，无 issues。

## Step 4 — 室内视觉件 + 材质与光照接线（task_49 首轮）
- 起止：2026-11-06 12:39:38Z → 2026-11-06 12:59:05Z
- 范围：Interior.gd（新增）+ WorldLook 的 indoor 档 + World.gd 分派
- 轮次：1 / 3
- 结果：PASS
- 提交：ok 1c7bee9
- 规划要点：计划已写入 [tmp/task49/plan.md](tmp/task49/plan.md)（911 行，逐文件:行号，含可直接照抄的 `Interior.gd` 全文与探针代码）。 **摘要（写 workbook）**：计划见 [plan.md](tmp/task49/plan.md)（逐文件:行号）。`Interior.gd` 建 12 四边形/3 mesh（地板 z=0.01、踢脚 0.15、门框宽 0.5；全 0.5 整数倍、与墙面共面、零碰撞、幂等）；`WorldLook` 拆 Apply_City/Indoor（BG_COLOR+环境色+3 盏不投影灯）+4 工厂+2 纯色键；`World.gd` 6 处按档分派，tscn 零改动。坑：先改 WorldLook 再建 Interior；渲染须 xvfb+opengl3；降级须连 `.import` 一起移；城市按截图 sha256 判。 规划期已用一次性探针实测确认（证据在 `tmp/task49/shots_before/` 与 `tmp/task49/tools/`）：city 渲染 sha256 三次重跑逐位相同（可作零回归硬判据）、渲染跑退出必现 2 行 leaked ERROR 属既有基线（0-ERROR 门禁只能用 headless `--quit`）、只移贴图源文件不会触发降级、`ObserverCamera` 的 `_physics_process` 会抹掉探针机位。
