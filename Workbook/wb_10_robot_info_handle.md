# Workbook — Task 10: Robot Info Handle

> 对应任务：`Task/task_10_robot_info_handle.md`
> 创建日期：2026-08-06

---

## 2026-08-06 创建（承接 Task 9 系列问题）

**背景**：Task 9 / 9_1 / 9_2 全部实施完成并通过全面核查（子 agent），已归档。
Task 9_2 决策 #11 的扩展点（入站 robot 数据仅打印）留待本任务演进。

**Task 10 内容**：
1. 目标待人类补充（robot info handle 的具体范围——入站位姿/地图的存储/展示/融合？）
2. 承接问题清单：P1#3（warn 退避）、P2#4（锁内组 JSON）、P2#5（Closed 忙循环）、
   P2#6（from_utf8_lossy）、P3#9~13、核查 P3（重复注释/编号/Modified Date）、
   文档偏差 2 处——详见 task_10 文档

**Task 9 系列归档**（2026-08-06）：
- Task: task_9_location_set_and_main_loop / task_9_1_network_update / task_9_2_robot_loop → `Task/archived task/`
- Workbook: wb_9_* / wb_9_1_* / wb_9_2_* → `Workbook/archived workbook/`

**Task 9 系列最终状态**：
- Task 9 Location Set：✅ 完成（7 文件落地，odom 零残留）
- Task 9_1 Network：✅ 完成（robot_bus / DataType::Robot / Broadcast）
- Task 9_2 Robot Loop：✅ 完成（13 条清单 + 决策 #1~#11；实测修复 P0×2 + P2-1 + P2#7）
- 剩余未修（已迁 Task 10）：P1#3、P2#4~6、P3#9~13 等


---

## 2026-08-06 全量代码梳理（子 agent）

**范围**：只读探索 `Src/Robot/` 全部 + `Src/WebSocket/` + Network 数据面（robot_bus/DataType::Robot/Broadcast）+ bootstrap/main_robot/main + VM robot 绑定。分支 Pleiades-Orion，工作区干净。

**关键结论**：
- 遥控代码在 `Src/WebSocket/`（非 Robot/websocket.rs）
- 入站 Robot 数据唯一入口：`Src/Network/swarm_events.rs:195-210`（from_utf8_lossy → robot_bus → main_loop 仅 info! 打印，robot.rs:336-341）
- `register_robot_caps`（capability_binding.rs:754-759）空 stub 无调用点=死代码
- Cargo：`Pleiades`(main.rs 纯推理) + `orion-robot`(main_robot.rs 车载)，无 default-run

**Task 10 问题清单核查**（13/15 仍存在）：
- 仍存在：P1#3(robot.rs:226/296 warn 无退避)、P2#4(robot.rs:277-299 锁内组包+broadcast)、P2#5(robot.rs:438 Closed→None)、P2#6(swarm_events.rs:199 lossy)、P3#9(:218 peer_id 每 tick 分配)、P3#10(:280-294 两次分配)、P3#13(bootstrap.rs:212-215 缺 lidar_baudrate)、核查 P3-1(executor.rs:110-111 重复注释)、P3-2(robot.rs:106/109 编号重复)、P3-3(Modified Date 未 bump)、P3-4(main_robot.rs:65 "TUI/CLI")
- 已消失：文档偏差-1（robot_bootstrap 实际无 peer_name 参数）、文档偏差-2（main.rs 已 14 行无 launch）

**新发现问题**：N1 state_notifier 读锁贯穿组包（P2#4 同类）；N2 auto_tick 每 50ms 全量 clone 三态（grid 65KB+）；N3 register_robot_caps 死代码；N4 executor.rs:235/239 双重转向日志；N5 WS server.rs:101-111 send 静默忽略失败；N6 dispatch StopLidarScan info! 级别不一致；N7 grid.rs build_map_full info! 刷屏；N8 路径偏差；N9 lib.rs 无 robot re-export；N10 pathfinder.rs 无单元测试

**实施建议**（存疑待人类确认）：入站处理最小落地=远端 pose/map_delta 存储(RemoteRobotInfo 按 peer_id HashMap)+过期清理+透出；解析器独立 fn 便于单测；P2#6 二进制通道涉 Network 超分支权限需批准；修复顺序=纯修复→性能→健壮性。

**决策点（待人类）**：①入站数据存哪/给谁看 ②过期时长 ③P2#6 是否允许碰 Bus_Event ④是否实现 robot Lua 绑定