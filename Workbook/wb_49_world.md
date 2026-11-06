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
