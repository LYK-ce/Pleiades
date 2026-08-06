# Workbook — Task 9: Location Set and Main Loop

> 对应任务：`Task/task_9_location_set_and_main_loop.md`
> 创建日期：2026-08-06

---

## 2026-08-06 Location Set 实施（S1~S8 完成）

**背景**：历史遗留——RobotState 只记相对位移 odom_x/y，64.0 锚点散落 4 处（state 无位置、robot.rs:174/207、executor.rs:111），消费方各自 `64.0+odom`。Task 9 收口：RobotState 直接记录全局唯一世界坐标。

**已确认设计**：
- RobotState.x/y = 全局世界坐标，启动时 = origin（默认 64,64）
- 消费方直读零换算；更新只在 RobotState 上（accumulate 直接积分 x/y）
- origin 注入点 = STM32Device::spawn 的 local_state 初始化（唯一写入路径）
- 单一写入者不变式：共享态只被 RX 回调 `*guard = local_state.clone()` 全量覆盖

**改动文件**（7 + 测试）：
1. `Src/Robot/core/state.rs`：odom_x/y → x/y + 模块 doc 位置语义/单一写入者不变式
2. `Src/Robot/slam/odometry.rs`：积分目标 x/y（公式不变）+ doc；新增 test_accumulate_world_coords
3. `Src/Robot/control/device/stm32/mod.rs`：spawn(+origin) 注入 local_state；:71 不变式注释；spawn_mock 同步 + mock 3 处调用补 (64,64)；新增 test_mock_origin_injected
4. `Src/Robot/core/robot.rs`：launch(+origin)；创建共享态后立即注入（关 (0,0) 窗口，spawn notifier 前）；spawn 传 origin；state_notifier/slam_task 删 64.0 直读
5. `Src/Robot/core/executor.rs:111`：`(wx,wy) = (robot_state.x, robot_state.y)`，删 64.0
6. `Src/main_robot.rs`：parse_origin（`orion-robot [x y]`，缺省 64,64，非法/越界 warn 回退）+ 3 测试
7. `Src/main.rs:135`：launch 补默认 (64.0, 64.0)

**验证**：`cargo test --lib robot` 36 passed（34 原 + 2 新）；`cargo test --bin orion-robot` 3 passed；`cargo build --bin orion-robot` / `--bin Pleiades` 通过；`./build.sh check` 无新增警告。⚠️ checksum/parser 的 64.0 是 LiDAR 角度缩放，勿动。

**踩坑记录**：
- transform anchor 编辑需先 anchors:true 读文件；insert_after 位置不当易产生重复行（doc 注释重复 3 次才清干净）——小文件重写更稳

**待办**：
- 实车验证：`./orion-robot 66.5 63.25` → WS 遥测应显示 (66.5, 63.25)；不传参默认 (64,64)
- Main Loop 部分：需求待人类补充
- 后端若想配置化 origin：`Robot_Config`（config.rs）加字段 + main.rs 读取
