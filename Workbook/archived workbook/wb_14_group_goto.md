# Workbook — Task 14: Group Goto 群发任务

> 对应任务：`Task/task_14_group_goto.md`
> 创建日期：2026-08-12

---

## 2026-08-12 任务创建 + 方案定稿

- 方案讨论(人类逐项确认)：
  - 协议：TASK_SET 扩展 member_count + members[](变长 peer_id)；member_count==0 取消 / ==1 单车 / >1 群发；同批升级无过渡期
  - 分配：planning 目录(assignment + pathfinder)；确定性分配(peer_id 排序 + 同色格环序)；头车精确目标点；障碍/OOB 跳过；环上限 10；目标格 Occupied 不可达
  - 执行：Mission::Goto 扩展 {x,y,members}；main_loop Set 分支零改动(纯入队)；executor pop 后调 assignment 算 goal 再建 D*
  - hello 补 peer_id 字段(hex 编码,终端连接即得身份)
  - 子 agent 核查：整体可行；取消语义定为 member_count==0；main_loop 签名需加参(非零改动)；own_peer_id 三层贯穿
- 开始实施：2026-08-12

## 实施记录（2026-08-12 完成）

- **Step 1** planning 目录：新建 `core/planning/`（mod.rs + pathfinder.rs 迁入 + assignment.rs）；slam/mod.rs 删 pathfinder；executor use 改路径；core/mod.rs 注册
- **Step 2** 协议：messages.rs 新增 `TaskSetPayload{members,missions}`；encode/decode_task_set 重写（mission_count + member_count + members[] 变长 + missions[]）；长度校验；mod.rs re-export
- **Step 3** command.rs：`Mission::Goto{x,y,members}`（members 空 = 老单车语义）；AutoCmd::Set 零改动；Mission 补 PartialEq
- **Step 4** assignment.rs：build_slots（L[0]=精确点 + 同色格切比雪夫环 1..=10，Occupied/OOB 跳过，目标格 Occupied→Unreachable，越界→OutOfBounds）+ group_goto_mission（members 空→target；sort 字节升序；NotMember/InsufficientSlots）；10 单测全绿
- **Step 5** WebSocket/protocol.rs：member_count 三分支（0 取消 / 1 单车 / >1 群发取第一个 Goto）；5 单测全绿
- **Step 6** server.rs：hello 加 peer_id（hex 编码 local_peer_id）
- **Step 7** robot.rs：launch 算 loop_peer_id → main_loop 加参 → auto_tick 传 step
- **Step 8** executor.rs：own_peer_id 三层贯穿（step/step_impl/step_idle）；pop 后调 assignment 算 goal 再建 D*
- **Step 9** 验证：lib 200 passed / 1 failed（`vm::engine::test_sandbox_os_blocked` = 既有环境失败，与本次无关）；Robot 模块 83 全绿；assignment 10 全绿；`./build.sh check` 通过
- **Step 10** 文档：orion_protocol.md §3.5（新布局+三分支+同批升级）/§1.5（hello 已实施）；multi_robot_control.md §4（下发链路/定稿算法/边界语义）；架构文档 §3.10（文件结构/启动流程/Command）；robot_review_problem.md pathfinder 路径引用同步

## 遗留

- **Pictor 端同步（外部仓库，必须同批上线）**：TASK_SET 新布局编码（member_count + members）+ hello peer_id 解析 + 在线车表
- 双车联调（群发 goto 实车验证）
- `vm::engine::test_sandbox_os_blocked` 既有失败（与 Task 14 无关，环境 sandbox 未生效）

## 待讨论问题（2026-08-12 人类提出，已核实代码事实）

### Q1. 其他小车的位置 pathfinder 没用上

- 事实：D*（`planning/pathfinder.rs`）cost() 只读静态地图（grid.state），**未注入远端车位姿**；`ClusterInfoTable`（cluster/cluster_info.rs）已存远端车 x/y/yaw/vx/vy，但无人消费（注释自述"为后续寻路把车当障碍注入铺数据基础"）。即 multi_robot_control.md P3 阶段"动态障碍注入 D* Lite"未实施。

### Q2. 手动模式/停车状态下广播的"下一格"是什么值

- 事实：`state_notifier`（robot.rs:261-263）广播 `valid = es.sub_target.is_some()`、`sub_gx/sub_gy = es.sub_target.unwrap_or(0)`；`ExecuteState.sub_target` 仅由 executor `step()` 包装层同步（executor.rs:118）。
- 手动模式：auto_tick 不运行（robot.rs:436 仅 Auto 激活）→ executor 不写 execute_state：
  - 全程手动 → 初始 None → 广播 `valid=false, sub=(0,0)` ✓
  - **Auto→Manual 切换 → execute_state 冻结在最后 Auto 值 → 广播残留旧 sub_target（valid=true）** ⚠️ 疑似缺陷（切换模式时 execute_state 未同步清除）
- 停车（队列空）：到达/失败路径会把 sub_target 置 None → `valid=false` ✓；但被 Set 替换/模式切换打断时同样可能残留。

### Q3. 意图广播（sub_target）未用于交通控制

- 人类澄清（2026-08-12）：每车广播自己的 sub_target（下一格），**本意是用于交通控制**（多车避碰/冲突检测，multi_robot_control.md §5.2 意图广播：第一版"下一格"，POSE payload 已扩展 valid+sub_gx/sub_gy）——**但接收方目前没用上**。
- 事实：生产侧 ✓（executor → ExecuteState → state_notifier → POSE 广播，Task 13_1 已通）；消费侧 ✗（cluster_consumer 解析 POSE → `ClusterInfo.sub_target` 仅存表，**无任何冲突检测/让行决策消费**）。
- 即 multi_robot_control.md **P4 阶段（冲突检测 + 避碰/让行）未实施**；与 Q1（P3 动态障碍注入）同属"多车数据面 → 规划层"断链。

> 三个问题同源：Q1 = 远端车位姿未进 pathfinder（P3）；Q3 = 远端车意图未用于交通控制（P4）；Q2 = 意图广播本身在手动/切换场景的残留值缺陷。

- 结束时间：2026-08-12
