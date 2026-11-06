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
