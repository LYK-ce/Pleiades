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
