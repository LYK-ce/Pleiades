# Workbook — Task 13: Multi-Robot Control

> 对应任务：`Task/task_13_multirobot_arch.md`
> 创建日期：2026-08-10

---

## 2026-08-10 任务创建 + Task 10/11/12 归档

**背景**：2026-08-10 三轮子 agent 调研收敛出多车协同设计 `docs/design_doc/multi_robot_control.md`（任务分配 / 散布形态 / 围堵 / 多车避碰 / 三层模型）。人类决策：Task 10/11/12 归档，新建 Task 13 承接多车方向。

**归档清单**：
- `Task/archived task/`：task_10_robot_info_handle.md、task_11_robot_update.md、task_12_broadcast.md
- `Workbook/archived workbook/`：wb_10、wb_12（wb_11 不存在）
- `robot_review_problem.md` 保留为唯一问题池（未归档）

**Task 10/11/12 终态**：
- Task 10（Robot Info Handle）：待启动即归档——目标方向（入站位姿存储/消费）并入 Task 13 P4 前置
- Task 11（Robot Update）：进行中即归档——A~E 完善方向中与多车相关者（P1 动态障碍/导航可靠性、B 性能组）由 Task 13 P0/P4 承接；其余（N3 Lua 绑定、N5 WS、N9 re-export、E 卫生组）仍在问题池待后续
- Task 12（gossipsub 广播）：✅ 完成（commit 7a363d5）——遗留双车联调验证未做，Task 13 验证计划沿用

**Task 13 范围速览**：P0 D* 动态障碍接口（=问题池 P1）→ P1 任务协议+task topic+确定性分配 → P2 散布 → P3 障碍注入 → P4 冲突检测+规避状态机 → P5 点云过滤 → P6 僵持协商。

**关键前置（设计文档 vs 代码断层，2026-08-10 梳理确认）**：
1. DStarLite 无动态障碍增删接口（P1 症状）
2. 入站位姿无消费路径（Task 10 方向）
3. gossip map 无全量快照（SnapshotCache 只存最近 delta）
4. 入站丢失 author（StreamRaw 无 peer_id；扩展涉 EventBus 接口需人类批准）

**待人类拍板**：任务文件"待决策问题" 1~9（意图广播范围/围堵半径/点云过滤/task topic/lane_dir/跨车时间/ETA/任务幂等/map 全量）

## 依赖关系

- 前置：问题池 P1（D* 动态障碍）、Task 10 方向（入站消费）、gossip 基础设施（ML_review 已 merge：bd0ec43/387ad72）
- 边界：gossip 基础设施归 ML_review，本任务仅消费（同 Task 12 模式）
