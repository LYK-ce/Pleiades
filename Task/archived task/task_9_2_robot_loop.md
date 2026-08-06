# Task 9_2: Robot Loop Network 接入

> 状态：✅ 已实施完成（2026-08-06：bootstrap 抽取 + Robot 接入 + 三 task 广播/订阅 + 入站仅打印）；待多节点联调 + 实车验证。详见 `Workbook/wb_9_2_robot_loop.md`
> 创建日期：2026-08-06
> 最后更新：2026-08-06

## 目标

Jetson 车载完整节点：完整 bootstrap + Core 推理循环 + Robot 循环 + 网络数据面（位姿/地图广播）。

## 背景

### 部署形态（已确认）

| 二进制 | 入口 | 用途 |
|---|---|---|
| `orion-robot` | main_robot.rs | **Jetson 车载完整节点**：bootstrap + Core 推理 + Robot + 网络数据面 |
| `Pleiades` | main.rs | **PC 机**：纯推理节点（Robot 已移除，`6363e50`） |

### 前置状态（Task 9_1 已落地）

- `DataType::Robot=3`（codec.rs:41）、`From_U8` 臂（:55）
- `NodeCommand::Broadcast` + `NodeHandle::Broadcast()`（node_handle.rs:60-62, 146-153）
- `Handle_Command` Broadcast 臂遍历 peers（command_handler.rs:33-45）
- 入站 Robot 分流 → robot_bus + 回 OK（swarm_events.rs:195-210）
- main.rs 创建 robot_bus（:68）+ Init 注入（:110）

**Task 9_2 缺口**：Robot 侧三处（state_notifier/slam_task/main_loop）完全无网络概念；robot_bus 无消费者。

## ✅ 已明确的改动

> ✅ 2026-08-06：13 条改动全部实施完成（含 bootstrap.rs 新文件），`cargo check` 无新增警告，robot 37 / orion-robot 3 / network 4 测试全绿，双二进制构建通过。唯一增补决策：**入站数据本次不处理**（main_loop 订阅 robot_bus 仅打印，见已决策表 #11）。

| # | 文件 | 位置 | 改动 |
|---|---|---|---|
| 1 | `Src/bootstrap.rs`（**新**） | 整文件 | `core_bootstrap()`：抽取 main.rs Phase 1~6 → 返回 `CoreBootstrap` |
| 2 | `Src/bootstrap.rs`（**新**） | 整文件 | `robot_bootstrap()`：Robot::launch + 注入 + WS |
| 3 | `Src/lib.rs` | :14 附近 | 加 `#[path="bootstrap.rs"] pub mod bootstrap;` + re-export |
| 4 | `Src/main.rs` | :109 | `_node_handle` 改名 `node_handle` 随 CoreBootstrap 返回（捡回） |
| 5 | `Src/main.rs` | :24-161 | 替换为 `core_bootstrap()` + Core 分支（薄） |
| 6 | `Src/main_robot.rs` | :57-88 | core_bootstrap + robot_bootstrap；参数读配置 |
| 7 | `Src/Config/config.rs` | :92-94 | `Robot_Config` 加 5 字段（serial_port/baudrate/car_type/lidar_port/lidar_baudrate，全 Option） |
| 8 | `Src/Config/config.rs` | :31-33 | DEFAULT_CONFIG `[Robot]` 段补模板 |
| 9 | `Src/Robot/control/types.rs` | impl CarType | 加字符串解析（from_str）+ 测试 |
| 10 | `Src/Robot/core/robot.rs` | :65-72 | `launch` 加 `node_handle: Option<Arc<NodeHandle>>` + `robot_bus: Option<Arc<EventBus>>` |
| 11 | `Src/Robot/core/robot.rs` | :163-194 | `state_notifier`：pose_tx 同时 `Broadcast(DataType::Robot, pose_json)` |
| 12 | `Src/Robot/core/robot.rs` | :200-251 | `slam_task`：map_tx 同时广播 map_delta JSON |
| 13 | `Src/Robot/core/robot.rs` | :257-374 | `main_loop`：加 robot_bus 订阅分支处理入站 |

### core_bootstrap 返回结构草案

```rust
pub struct CoreBootstrap {
    pub config: Pleiades_Config,           // robot_bootstrap 读 [Robot] 段
    pub event_bus: Arc<EventBus>,          // TUI 订阅 / 广播本地信息
    pub robot_bus: Arc<EventBus>,          // → robot_bootstrap → main_loop
    pub node_handle: NodeHandle,           // 原 _node_handle（捡回）→ Robot 注入
    pub network_service: Network_Service,  // → 调用方 spawn Start()（需 mut）
    pub core: Core,                        // → core.run()（消费自身）
    pub user_cmd_tx: mpsc::Sender<UserCommand>, // → CLI REPL / TUI_Loop
}
```

### robot_bootstrap 签名草案

```rust
pub async fn robot_bootstrap(
    config: &Pleiades_Config,          // 读 [Robot] 段
    node_handle: Arc<NodeHandle>,      // 出向 Broadcast
    robot_bus: Arc<EventBus>,          // 入向订阅
    origin: (f32, f32),                // CLI 传入（保持，默认 64,64）
    peer_name: String,                 // config [Identity].peer_name = 车名（WS vehicle_id + 广播 payload name）
) -> Result<Robot, String>             // 返回 Robot（shutdown 用）
```

### 测试影响

- Robot 36 个单测：无 launch 调用 → 不受签名影响
- config.rs 无测试 → Robot_Config 扩展无破坏（新字段必须 Option，否则旧 config.toml 反序列化失败）
- main_robot.rs 3 个 parse_origin 测试：受 origin 去向影响
- Network 4 个测试：无影响

## 问题清单（10 项，全部已决策——见下方已决策表）

| # | 问题 | 选项 |
|---|---|---|
| 1 | **cli_mode / TUI/CLI 处理** | core_bootstrap 内部决定 vs 留 main.rs；orion-robot 是否跑 TUI/CLI（车载大概率纯后台 → 需"无 UI 直接 core.run()"模式） |
| 2 | **network_service.Start()** | core_bootstrap 内部 spawn vs 返回由调用方 spawn |
| 3 | **robot_bootstrap 返回值** | 返回 `Robot`（shutdown 用）vs 更小句柄 |
| 4 | **launch 注入方案** | (a) launch 加参数透传三 task（推荐，改动集中）vs (b) 重构 launch 组装/手动 spawn |
| 5 | **origin 去向** | CLI 保留 / 进 `[Robot].origin_x/y` 配置 / 两者（CLI 覆盖配置） |
| 6 | **payload 协议** | 出向 JSON 统一带 `peer_id`（`Get_Local_Peer_Id()`）？本地回环是否需过滤（确认 command_handler Broadcast 遍历是否跳过 local） |
| 7 | **map_full 广播触发** | 定时低频（并入 slam_task 节流）vs 变更即广播 vs 按需 |
| 8 | **Robot_Config 缺省值** | 沿用硬编码值（/dev/myserial、115200、X3Plus、/dev/rplidar、230400）？car_type 解析失败 warn 回退 X3Plus？ |
| 9 | **bootstrap.rs 位置** | `Src/bootstrap.rs` vs `Src/Config/bootstrap.rs` |
| 10 | **orion-robot 并发结构** | robot_bootstrap() → core.run()（Robot 循环已在 launch 内 spawn 后台跑）——确认时序 |

### 已决策（2026-08-06 讨论）

| # | 问题 | 决策 |
|---|---|---|
| 1 | TUI/CLI 处理 | **统一 TUI，CLI 模式已去除**（人类 2026-08-06 决策：`cli` 参数与 orion-robot 位置参数冲突，不再支持）；`network_service.Start()` + TUI + `core.run()` 仍在 `CoreBootstrap::run(self)` 内共用 |
| 10 | 并发结构 | `core_bootstrap()`（初始化，不阻塞）→ `robot_bootstrap()`（初始化完返回，main_loop 后台跑，不阻塞）→ `boot.run()`（core.run() 阻塞接管，两循环并行） |
| 5 | origin 去向 | **保持 CLI**（`orion-robot 66.5 63.25`，无参数默认 64,64），不进配置；现有 parse_origin 保留，robot_bootstrap 收 origin 参数 |
| 2 | network_service.Start() | 进 `CoreBootstrap::run(self)` 内部 spawn（调用方不用管，两入口一致） |
| 3 | robot_bootstrap 返回值 | 返回 `Robot`（main_robot.rs 持有，退出时 `shutdown()`） |
| 4 | launch 注入方案 | (a) `Robot::launch` 加 `node_handle: Option<Arc<NodeHandle>>` + `robot_bus: Option<Arc<EventBus>>` 透传三 task |
| 6 | payload 协议 | 广播 JSON 带 `peer_id` + `peer_name` 双字段；**peer_name = 车名 = WS vehicle_id，全链路统一**；入站靠 peer_id 识别（Broadcast 已跳过 local，无回环） |
| 8 | Robot_Config 缺省值 | 沿用硬编码（/dev/myserial、115200、X3Plus、/dev/rplidar、230400）；car_type 解析失败 warn 回退 X3Plus |
| 9 | bootstrap.rs 位置 | `Src/bootstrap.rs`（与 main.rs 同层，lib.rs 挂 `#[path="bootstrap.rs"] pub mod bootstrap;`） |
| 7 | map_full 广播 | **本次不做**（先只广播位姿 + 地图 delta；传输能力已在 Task 9_1 具备，全量地图留后续） |
| 11 | 入站数据处理（人类 2026-08-06 确认） | **本次不处理**——main_loop 订阅 robot_bus，收到 `Bus_Event::Stream` 仅 `info!` 打印 payload（不存储/展示/融合）；swarm_events.rs 保持 Publish + 回 OK 不动 |

---

## Code Review 记录（2026-08-06，子 agent 只读审查）

**结论**：无 P0，可合入。以下问题记录留待后续处理（✅ = 已解决）。

### P1

| # | 位置 | 问题 | 状态 |
|---|---|---|---|
| 1 | bootstrap.rs:55 + main_robot.rs | `cli` 参数与位置参数冲突（`orion-robot cli` 被 parse_origin 当坐标解析） | ✅ **已解决**：CLI 模式移除，统一 TUI |
| 2 | config.rs 文件头 | Modified Date 未更新（2026-06-15） | ✅ **已解决**：→ 2026-08-06 |
| 3 | robot.rs:214,284 | 广播失败 `warn!` 无退避——弱网下 cmd_tx 持续满 → 10Hz warn 风暴 | ⬜ 待处理（建议：连续失败 N 次降频 debug! / 每 5s 一次，成功复位） |

### P2

| # | 位置 | 问题 | 状态 |
|---|---|---|---|
| 4 | robot.rs:266-287 | slam_task 在 `grid` 写锁内做 JSON 序列化 + broadcast（锁持有时间非最短） | ⬜ 待处理（建议：锁内只取 deltas，锁外组 JSON） |
| 5 | robot.rs:427 | `recv_robot_event` 的 `Closed => None` 若 sender 全 drop 会忙循环（当前 main_loop 自持 Arc 保活，实际不可达——隐性陷阱） | ⬜ 待处理（建议：注释注明依赖，或 Closed 返回哨兵让 main_loop break） |
| 6 | swarm_events.rs:199 | 入站 Robot payload 走 `from_utf8_lossy`——当前 JSON 安全；未来 map_full 二进制会被损坏 | ⬜ 待处理（建议：文档注明 robot_bus 仅支持 UTF-8 JSON） |
| 7 | robot.rs:131-136 | LiDAR spawn/start_scan 失败 `?` 提前返回时已 spawn 的 STM32 串口任务未 shutdown（预存问题） | ✅ **已解决**（2026-08-06）：两处失败路径改为 match/if-let，返回前 `stm32.shutdown()` |
| P2-1 | bootstrap.rs:200-203 | **LiDAR 空串禁用失效**（核查新发现）：`Some("")` 被 filter 剔除后 or_else 又补回 /dev/rplidar → 无 LiDAR 的车配置 `lidar_port=""` 会尝试打开设备失败、启动报错退出；与三处注释/文档声明矛盾 | ✅ **已解决**（2026-08-06）：改为三态 match——显式值用配置 / 空串禁用（None）/ 缺省 /dev/rplidar；三态实测通过（default→/dev/rplidar、empty→None、custom→/dev/ttyUSB0） |

### P3

| # | 位置 | 问题 | 状态 |
|---|---|---|---|
| 9 | robot.rs:207,277 | `Get_Local_Peer_Id().to_string()` 每 100ms/200ms 各分配一次（peer id 不变，可缓存） | ⬜ 待处理（建议：launch 时算一次传闭包） |
| 10 | robot.rs:272-283 | `typed.clone()` 给 map_tx 后又 iter 组 JSON——两次分配 | ⬜ 待处理（建议：复用一次） |
| 11 | lib.rs 文件头 | 无 Modified Date | ⬜ 待处理（预存风格） |
| 12 | bootstrap.rs | `core_bootstrap`/`robot_bootstrap` snake_case（与项目既有风格一致，任务已豁免） | ⬜ 记录 |
| 13 | bootstrap.rs:212-215 | robot_bootstrap info! 只打 lidar_port 不打 lidar_baudrate | ⬜ 待处理 |

---

## 人类评审

<!-- 在此区域写下评审意见 -->


> ✅ **2026-08-06 实测修复（P0）**：`core_bootstrap()` 抽取后 `_log_guard`（tracing_appender WorkerGuard）随函数返回被 drop → 日志 worker 线程关闭，`robot_bootstrap` 及之后所有日志静默丢失（实测现象：车端日志停在 `Orchestrator Core 初始化完成`，WS 却正常）。修复：guard 移入 `CoreBootstrap` 字段，由 `run(self)` 持有到进程退出。