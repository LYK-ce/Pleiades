# wb_34_camera — task_34 工作记录

> Presented by KeJi
> Created Date ： 2026-10-24
> Modified Date ： 2026-10-24
> 任务：Task/task_34_camera.md（UAV 相机集成 + 仿真相机对齐；Step 5 由人类手工验证）

## 执行引擎启动
- 引擎：Tool/execution_engine_task34.ts + Tool/execution_engine_task34.config.json
- 计划：Step 1~4 串行，每 step 最多 3 轮（规划 → 实现 → 评审）
- 范围：pleiades-uav/、pleiades-sim/、orion-simulation-world/、新增 programs/user/camera_photo.lua
- 禁改：pleiades-base/、pleiades-ugv/、pleiades-terminal/、sim 控制口、既有 lua 脚本

## Step 1 — uav 两个独立相机 sensor（camera_pc 迁移 + camera_uav 新增）
- 范围：pleiades-uav/（+ 新增 programs/user/camera_photo.lua）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [tmp/task34/plan.md](tmp/task34/plan.md)（445 行，含 9 个文件清单、签名表、实施顺序、9 条可复制验收命令、15 条风险）。 **计划要点（3-5 行）** 1. 两个互不引用的子目录：`camera_pc/` 整段迁移旧 NV12 实现（只补 `open_stream()` 一处 + 统一 path 解析，`open/capture/close` 逐行不动）；`camera_uav/` 照 Pi 已验证路径（枚举→三级选格式→`Exact`→失败 `drop` 后 `Closest` 重开→`open_stream`，MJPG 直通绝不 `decode_image`，抓帧超时不 join）。 2. 纯函数下沉 `format.rs`（`Choose_Format`/`Best_At`/`Best_Raw_At`/`Closest_Score`/`Format_Rank`/`Parse_Prefer`/`Format_Table`）与 `frame.rs`（`Frame` + 直通/重编码），配置驱动 `prefer`/`resolutions`/`fps`，空列表=无偏好走 Closest；`config.rs` 两段 `fill_*` 缺才补/幂等/不覆盖，旧 `[camera]` 只保留解析并打一行「已忽略」。 3. `bootstrap.rs` 分别打开、各自 `warn`、不阻断；额外加**同 index 守卫**——因为核实到 nokhwa 的 Linux 绑定对同一 index 会共享同一个 `Arc<Mutex<Device>>` 而**不会**报 busy，双开只会互踩流，守卫把「后开者只 warn」变成确定性行为。 4. 验收 = `cargo check` 绿（warning 仍 14 条）+ `cargo test`（49 → ≈75，含格式选择/path 解析/错误映射）+ 6 条 grep 自检（无 `mod camera;`、无 `CameraIndex::String`、两目录零交

## Step 2 — Lua 契约补全（info）+ 新增拍照脚本 camera_photo.lua
- 范围：pleiades-uav/（+ 新增 programs/user/camera_photo.lua）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task34/plan.md)，包含 639 行，共 10 节。 **计划要点（3–5 行）** - 两个相机模块各加一个**同步** `info()`：闭包只把 `Info_Impl()` 的返回值转成留在 Lua 里的 table，四字段由 `width/height/format/age_ms` 固定下来（`format="NV12"|"MJPG"|…|"none"`，`age_ms` 用单调 `Instant`，无帧时取 0）。 - 数据来源采用三级回退：最近成功抓取的一帧 → 打开时协商到的格式 → 配置缺省。为此 `camera_pc` 把 worker 回传内容从裸字节扩成 `Pc_Frame`（含宽、高、格式以及帧被取到的时间戳），并把 `open_tx` 改为带回 `Pc_Open_Info`；`camera_uav` 则改成 `Captured_Frame{frame, at}`。两边刻意重复实现、互不引用，注释里也不出现对方模块名，避免破坏 Step 1 的 A5 验收项。 - 按计划新建 `programs/user/camera_photo.lua`（`camera_uav → camera_sim → camera_pc` 顺序探测；先打印 sensor、info、字节数，再用 `storage_acquire_write → write → release` 把 JPEG 写入 `camera_photo.jpg`；三个表都不存在时报错并 `return "no-camera"`；`capture()` 为 nil 时先判断 nil、类型和长度，绝不写 `#nil`）。 - 验收全部可复制：`cargo check`(14 warnings 持平)、`cargo test`(81→92，0 failed)、Lua 契约与越界 grep；本机无 lua/luac，用 `mlua` 编译 `camera_photo.lua` 的用例兜底，不装软件、不起进程。 - 已记录三处文档与现实矛盾（既有脚本仍在调用

## Step 3 — 世界侧 camera 零件（SensorCatalog + type=2 帧下发）
- 范围：orion-simulation-world/（Godot 工程）
- 轮次：1 / 3
- 结果：PASS
- 规划要点：计划已写入 [`tmp/task34/plan.md`](tmp/task34/plan.md)（552 行）。本轮只读代码 + 跑基线，**未改任何产品代码**（`git status` 里的 `pleiades-uav/**` 是 Step 1 的既有改动，与本轮无关）。 **计划要点** 1. **只加不改**：`SensorCatalog` 加 `camera` 场景/默认参数、`WireProtocol.BuildCameraFrame`（15 B 头 + 原始 JPEG）、`ComponentLink` 双 4 MiB 缓冲、`CameraSensor` 加 `rate_hz` 累加器 + `SetStream/Tick_Stream/Emit_Frame`；`Capture_Jpeg()` 五步与 `UPDATE_ONCE` 原样保留——`WebSocketManager`/`Vehicle`/`.tscn` 一行不动（manager 早有 `has_method("SetStream")` 钩子，相机加方法即入装配线）。 2. **headless 的三条腿**：`Can_Capture()==false` → 「只 warn 一次、到点计数、绝不出帧」由 `A17`（树外实例，headless/Xvfb 都确定）+ `J6`（端到端 0 帧）双向钉死；帧头/魔数用假 JPEG 走 `Emit_Frame` 真路径验（`A9/A14/A15`），真 JPEG 链路留 `B10/J7` 在真渲染器下跑。 3. **断言数按设计变动**：`check_camera_sensor 12→18`（`A13~A18`，`[skip]×10`）、`check_ws_manager 51→59`（`A9/A10` + `J1~J6`，`[skip]×1`）；其余 12 个自检的 `total` 必须与**本轮实测基线**逐字一致（`project` 无 total / `world 26` / `camera 32` / `vehicle 40` / `diff_dr

## Step 4 — sim 侧 camera 传感器 + camera_sim 注入 Lua
- 范围：pleiades-sim/
- 轮次：2 / 3
- 结果：PASS
- 规划要点：计划已写入 [plan.md](tmp/task34/plan.md)（364 行，含 9 节：现状盘点 / R0 越界决策 / 文件清单 / 签名与 trait 实现点 / 衔接链 / 实施顺序 / 可复制验收命令 / 风险 12+3 条 / 本轮不做）。 **要点总结（3–5 行）** 1. 实测起点：第 1/2/3/4/7 项已在上两轮落地（工作树未提交，基线 `cargo check` 2 warnings、`cargo test` 256 passed），本轮只补第 5 项 `device_cap.rs`、第 6 项 `capabilities` 接线，并清掉 prev_issues 4/5 的两处 `#[allow(dead_code)]` 与一处自指注释。 2. 唯一实际障碍是 `DeviceCapability::register_lua_caps(&Lua)` 迫使添加 `mlua` 直接依赖（已复核 base 无 re-export、无泛型适配器、impl 必须写出类型名），故按评审上一轮要求的做法落地**唯一申报越界**：`pleiades-sim/Cargo.toml` 2 行 + `Cargo.lock` 自动 1 行（`+ "mlua",`），并已写明“拒绝该越界即等于再走一次方案 B（已被判 FAIL）”。 3. 模块挂载用 `#[path = "device_cap.rs"]` 挂在 `sim_handler.rs` 内（模块路径 `crate::sim::sim_handler::device_cap`），从而**不动 `sim/mod.rs`**、不产生第二处越界；`sim_bootstrap` 只加 1 个 `capabilities` 参数，`on_tick` / `control.rs` 一行不改。 4. 三条硬判据：`cargo check` **恰好 2 warnings**（多一条即说明接线漏了）、`cargo test` **262 passed**（256+6）、`grep -rE "pleiades_ugv|plei

## 引擎结束
- 状态：ALL_STEPS_DONE
- git status（结束时）：
```
 M .gitignore
 M Cargo.lock
 M orion-simulation-world/COMPONENTS.md
 M orion-simulation-world/src/net/ComponentLink.gd
 M orion-simulation-world/src/net/WireProtocol.gd
 M orion-simulation-world/src/sensors/CameraSensor.gd
 M orion-simulation-world/src/sensors/SensorCatalog.gd
 M orion-simulation-world/src/tests/check_camera_sensor.gd
 M orion-simulation-world/src/tests/check_ws_manager.gd
 M pleiades-sim/Cargo.toml
 M pleiades-sim/src/bootstrap.rs
 M pleiades-sim/src/main.rs
 M pleiades-sim/src/sim/sensors/mod.rs
 M pleiades-sim/src/sim/sim_handler.rs
 M pleiades-sim/src/world_link/catalog.rs
 M pleiades-uav/src/bootstrap.rs
 M pleiades-uav/src/config.rs
 D pleiades-uav/src/device/camera/mod.rs
 M pleiades-uav/src/device/mod.rs
?? Task/task_34_camera.md
?? Tool/blender/models/camera_module.blend1
?? Tool/blender/models/lidar_tmini.blend1
?? Tool/blender/models/ugv_chassis.blend1
?? Tool/execution_engine_auto_survey.config.json
?? Tool/execution_engine_auto_survey.ts
?? Tool/execution_engine_task34.config.json
?? Tool/execution_engine_task34.ts
?? Tool/execution_engine_uav_camera.config.json
?? Tool/execution_engine_uav_camera.ts
?? Tool/investigation/engine.config.json
?? Tool/investigation/pose_jump_report.md
?? Workbook/wb_33_uav_camera_test.md
?? Workbook/wb_34_camera.md
?? pleiades-sim/src/sim/device_cap.rs
?? pleiades-sim/src/sim/sensors/camera/frame.rs
?? pleiades-sim/src/sim/sensors/camera/mod.rs
?? pleiades-uav/src/device/camera_pc/mod.rs
?? pleiades-uav/src/device/camera_uav/format.rs
?? pleiades-uav/src/device/camera_uav/frame.rs
?? pleiades-uav/src/device/camera_uav/mod.rs
?? programs/user/camera_photo.lua
/etc/profile: line 32: /etc/ShellCrash/menu.sh: No such file or directory

```
