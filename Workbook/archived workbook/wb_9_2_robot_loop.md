# Workbook — Task 9_2: Robot Loop Network 接入

> 对应任务：`Task/task_9_2_robot_loop.md`
> 创建日期：2026-08-06

---

## 2026-08-06 部署形态确认 + main.rs Robot 移除

**部署形态**（人类确认）：
- `orion-robot`（main_robot.rs）→ 装在车上：Robot 控制 + 网络数据面（Task 9_2 注入点）
- `Pleiades`（main.rs）→ 跑在 PC 机：分布式推理系统（纯推理，无 Robot）
- 两个二进制分工，互不干扰

**main.rs Robot 移除**（4 处，约 17 行）：
1. `use pleiades::robot::{CarType, Robot};` 删除
2. `let vehicle_id = peer_name.clone();` 删除（仅 WS 用）
3. Phase 5.6 块（ws_bind / Robot::launch / websocket::start）删除——**硬耦合问题随之消失**（PC 机不再有串口依赖）
4. `robot.shutdown();` 删除

**保留**：
- `robot_bus`（Task 9_1 需要，main.rs:68 创建 + Init 传参 :110）✅
- `Robot_Config`（config.rs，未来配置 robot 用）✅——用户明确要求保留

**注意**：不能用 master 的 main.rs 直接覆盖——master 无 robot_bus（Task 9_1 改动），覆盖会丢且 Init 签名不匹配。正确做法 = 当前 main.rs 删 Robot 部分。

**验证**：`cargo check` ✅；`cargo build --bin Pleiades` ✅；grep 确认 main.rs 无 Robot 引用（仅 robot_bus）

## 待办（Task 9_2 主体）

1. `main_robot.rs`：注入 `Arc<NodeHandle>` + `robot_bus`
2. `state_notifier`：发 pose_tx 同时 `NodeHandle::Broadcast(DataType::Robot, pose_json)`
3. `slam_task`：发 map_tx 同时 broadcast map_delta（有 delta 才发）
4. `main_loop`：订阅 robot_bus 处理入站（存 remote_vehicles / 展示）
5. payload 协议定义（位姿/地图 JSON，含 peer_id）
6. 全量地图发布时机
7. 联调验证（多节点完整系统）

## 2026-08-06 实施完成（bootstrap 抽取 + Robot 网络接入 + 入站仅打印）

**目标**：orion-robot 成为完整车载节点（bootstrap + Core + Robot + 网络数据面）；Pleiades 保持纯推理。

**改动文件**（7 个，其中 bootstrap.rs 为新增）：
1. `Src/bootstrap.rs`（**新**）：`core_bootstrap()` 抽取 main.rs Phase 1~6 → `CoreBootstrap`{config/event_bus/robot_bus/node_handle/network_service/core/user_cmd_tx}；`robot_bootstrap()`（读 [Robot] 段 → launch 注入 → WS，vehicle_id=peer_name）；`CoreBootstrap::run(self)`（network_service.Start() spawn + TUI/CLI 分支 + core.run() 阻塞）
2. `Src/lib.rs`：`#[path="bootstrap.rs"] pub mod bootstrap;`（session mod 后）
3. `Src/main.rs`（161→15 行）：`core_bootstrap().await?` + `boot.run().await`
4. `Src/main_robot.rs`：core_bootstrap → parse_origin（保持，日志初始化后调用，warn 可见）→ robot_bootstrap（`Arc::new(boot.node_handle.clone())`，**必须 clone**——boot 后续 run() 消费自身）→ boot.run() → robot.shutdown()；tracing 初始化移除（core_bootstrap 负责）
5. `Src/Config/config.rs`：`Robot_Config` +5 字段（serial_port/baudrate/car_type/lidar_port/lidar_baudrate，全 Option）；DEFAULT_CONFIG [Robot] 段补模板
6. `Src/Robot/control/types.rs`：`CarType::from_str`（trim + 大小写不敏感，兼容 X3_Plus/X3-Plus）+ 8 断言测试
7. `Src/Robot/core/robot.rs`：
   - `launch` +3 参数：`node_handle: Option<Arc<NodeHandle>>`、`robot_bus: Option<Arc<EventBus>>`、`peer_name: String`，透传三 task（None 行为不变，纯本地可用）
   - `state_notifier`：100ms 发 pose_tx 后广播 `robot_pose` JSON（peer_id + peer_name + x/y/yaw/vx/vy/ts）
   - `slam_task`：有 delta 才广播 `robot_map_delta` JSON（deltas 数组）——`map_tx.send(typed.clone())` 后组广播
   - `main_loop`：`robot_bus.as_ref().map(|b| b.Subscribe())` → select! 加 `robot_ev = recv_robot_event(&mut robot_rx)` 臂，`Bus_Event::Stream` 仅 `info!` 打印（人类确认：入站不处理）
   - 新增 `recv_robot_event` helper（Option 化接收；Lagged warn 后继续无忙循环；None 时永远 pending）

**决策落地**（task_9_2 已决策表）：#1 两入口一致 TUI/CLI（run() 内 args 检测）；#2 Start() 进 run()；#3 返回 Robot；#4 launch 透传；#5 origin 保持 CLI；#6 payload 带 peer_id+peer_name、peer_name=车名=WS vehicle_id（main_robot 原硬编码 orion_robot 已删）；#8 缺省沿用硬编码 + car_type 失败回退 X3Plus；#9 bootstrap.rs 位置；#10 core_bootstrap→robot_bootstrap→run()；**#11 新增：入站仅打印**（swarm_events.rs 不动）

**验证**：`cargo check` ✅ 无新增 warning（27 条均为既有）；`cargo test --lib robot` 37 passed（36+from_str）；`--bin orion-robot` 3 passed；`--lib network` 4 passed；全量 `--lib` 149/1（唯一失败 `vm test_sandbox_os_blocked` 为原有）；Pleiades + orion-robot 构建 ✅

**踩坑**：
- transform anchor 替换文本若自带闭合行，会与保留行重复（launch 签名 `) -> Result`、两处 spawn `});` 各重复一次，手动删）
- pattern 替换会保留原行缩进前缀，替换文本需自带完整缩进（两次缩进错位）
- `robot_bus.as_ref().map(EventBus::Subscribe)` 类型不匹配（Option<&Arc> vs &EventBus），改闭包 `|b| b.Subscribe()`
- main_robot 部分 move：`Arc::new(boot.node_handle)` 移走字段后 boot.run() 报 E0382，改 `.clone()`

**待办**：① 多节点联调（两机互跑 orion-robot/Pleiades，日志确认位姿/地图广播互通 + 入站打印）；② 实车验证（串口 + LiDAR + WS）；③ 旧 config.toml 无 [Robot] 新字段 → 走缺省（如需用配置更新实际文件）


## 2026-08-06 移除 CLI 模式（人类决策）+ Code Review

**背景**：review 发现 `CoreBootstrap::run()` 的 `args().nth(1)=="cli"` 检测与 main_robot 的 parse_origin（位置参数 x y）抢第一个参数——`orion-robot cli` 会 warn"参数数量错误"回退默认坐标；`orion-robot cli 66.5 63.25` 坐标被丢。

**人类决策**：直接去掉 CLI 模式，不与位置参数抢。

**改动**：`bootstrap.rs` CoreBootstrap::run() 删除 cli_mode 检测 + CLI 分支（spawn_stdout_subscriber/spawn_stdin_repl 调用），统一 TUI；`Src/CLI/` 模块保留（pub mod，无调用者，无 dead_code 警告，属主枝 ML_review 范围不删）。

**Review 结论**（子 agent 只读审查）：无 P0，可合入；13 条清单 + 决策 #1~#11 全落地；P1 遗留：① cli 冲突（本次已通过去 CLI 解决）② config.rs 文件头 Modified Date 未更新 ③ 广播失败 warn 无退避（弱网 10Hz 日志风暴风险）；P2/P3 非阻塞（锁内组 JSON、Closed 忙循环隐患、peer_id 每 tick 分配等）。

**待办追加**：④ config.rs 文件头 Modified Date 补 2026-08-06；⑤ 广播失败 warn 退避（可选）

**完整问题清单**（P1/P2/P3 逐条 + 状态）见 `Task/task_9_2_robot_loop.md` 的「Code Review 记录」小节。用户 2026-08-06 指示：先测试，问题暂不修。

## 2026-08-06 实测发现并修复 P0：日志 guard 生命周期 bug

**现象**（用户实车联调）：Pictor 能注册 WS（Binah @ ws://10.100.80.239:9090）并收到 map_full，但地图全 Unknown（[0:0 1:0 2:65536]，0 Free / 0 Occupied / 65536 Unknown）——grid 从未被 slam_task 更新；车端日志停在 `Orchestrator Core 初始化完成`，**没有** `Robot 配置` / `LiDAR 设备已启动` 等行。

**根因**：`core_bootstrap()` 里 `let (non_blocking, _log_guard) = tracing_appender::non_blocking(log_file)`——`_log_guard` 是**函数局部变量**，core_bootstrap 返回即 drop。tracing-appender 0.2.4 的 `WorkerGuard::drop` 发送 `Msg::Shutdown` 关闭日志 worker（non_blocking.rs:282-300 源码确认）→ 之后所有日志静默丢失。原版 main.rs 中 guard 是 main() 局部变量活到退出，抽取后生命周期被截断。

**修复**：`CoreBootstrap` 加 `_log_guard: tracing_appender::non_blocking::WorkerGuard` 字段，`run(self)` 持有到 `core.run().await` 结束（进程退出前）。

**验证**：/tmp 下跑 orion-robot → 日志文件完整包含 `Core bootstrap 完成` / `初始世界坐标 origin` / `Robot 配置: port=/dev/myserial baud=115200 car=X3Plus lidar=/dev/rplidar ws=0.0.0.0:9090 peer_name=new_peer` ✅（修复前这些行不存在）

**⚠️ 遗留确认项**：车端旧 config.toml 缺 `[Robot]` 新字段（serial_port/baudrate/car_type/lidar_port/lidar_baudrate）→ `robot_bootstrap` 的 `lidar_port` 为 None → launch 走 `_` 臂 `LiDAR 未配置，跳过` → 地图永不更新（全 Unknown）。修复日志后车端可见 `Robot 配置: ... lidar=None` + `LiDAR 未配置，跳过`。**待用户更新车端 config.toml 或代码补缺省**（决策 #8：缺省沿用硬编码 /dev/rplidar、230400）。

## 2026-08-06 核查收尾：P2-1 + P2#7 修复

**P2-1（LiDAR 空串禁用失效）**：`Some("")` 被 `filter(!is_empty())` 剔除后 `or_else` 又补回 /dev/rplidar → 空串无法禁用，且无 LiDAR 的车配置 `lidar_port=""` 会尝试打开设备失败 → 启动报错退出。修复：`bootstrap.rs` 三态 match——显式值用配置 / 空串（含纯空白）禁用（None）/ 字段缺失缺省 /dev/rplidar。三态实测：default→/dev/rplidar、empty→None、custom→/dev/ttyUSB0 ✅。

**P2#7（LiDAR 失败路径 STM32 泄漏）**：`robot.rs` launch 中 LiDAR `spawn`/`start_scan` 失败 `?` 提前返回时，已 spawn 的 STM32 串口后台任务未 shutdown（当前单进程退出 runtime 兜底，将来长驻进程会泄漏）。修复：两处失败路径改 match/if-let，返回前 `stm32.shutdown()`。

**验证**：`cargo check` 0 errors；robot 37 passed。Task 9_2 Code Review 记录已同步（#7、P2-1 → 已解决）。

**剩余未修**：P1#3（warn 退避）、P2#4（锁内组 JSON）、P2#5（Closed 忙循环）、P2#6（from_utf8_lossy）、P3#9~13（分配/注释/日期等）+ 核查 P3（executor 重复注释、robot 重复编号、3 文件 Modified Date）。
