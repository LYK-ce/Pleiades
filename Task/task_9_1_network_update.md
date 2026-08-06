# Task 9_1: Network Update

> 状态：✅ 已实施完成（S1~S6，2026-08-06）；待 Task 9_2（Robot 侧接入）联调 + 实车/多节点验证。详见 `Workbook/wb_9_1_network_update.md`
> 创建日期：2026-08-06
> 最后更新：2026-08-06

## 目标

为无人集群控制（车↔车位姿/地图广播）改造 Pleiades 主枝的 Network 模块，提供三个能力：

1. **robot_bus**：独立的 Robot 数据事件总线（与全局 EventBus 隔离，高频数据不污染全局）
2. **DataType::Robot**：新增传输层数据类型 + 入站分流处理（直接推送 robot_bus，绕过 Core）
3. **Broadcast**：网络广播能力（当前只有点对点 send_data）

**范围边界**：本任务只改 Pleiades 主枝内容（Network 模块 + main.rs 组装）。payload 协议（位姿/地图 JSON 格式、peer_id 约定）由 Robot 侧 Task 9 负责——Network 层只搬运字节，不关心内容。

## 背景

- 原计划 Robot 分支（Pleiades-Orion）不动 Pleiades 主枝的 Network，但无人集群控制必须依赖 P2P 数据面 → 不可避免需要主枝配合
- 当前 Network 能力：点对点 send_data（强制等 Response，超时 30s）、无广播、DataType 仅 Command/Data/File/Info
- 入站 Data/Info 被 Network 内部消化（不转发 Core），Command/File 走 Core B2

## 方案设计

### 1. robot_bus（独立 EventBus）

- `main.rs` 创建第二个 `Arc<EventBus>` 实例（robot_bus），容量与全局总线一致（1024）
- `Network_Service` 构造/Init 增加 robot_bus 参数（直接加参数，不做 EventBus 数组化——改动面小，EventBus 核心不动）
- Robot 侧（Task 9）注入同一 robot_bus 供 main_loop 订阅
- 职责：只承载 `robot_pose` / `robot_map_delta` / `robot_map_full` 事件

### 2. DataType::Robot + 入站处理

**文件：`Src/Network/Request_Response/codec.rs`**
- 枚举加 `Robot = 3`（3 为 From_U8 中的空缺值）
- `From_U8` 加臂：`3 => Ok(DataType::Robot)`

**文件：`Src/Network/swarm_events.rs`（Handle_Request_Response_Event 分流）**
- 加臂：`DataType::Robot => { robot_bus.Publish(...); 回 OK }`
- payload **原样转发**（不发包、不解析——序列化是上层职责）
- 回最小响应 `"OK"`：闭合 libp2p request_response 协议状态机（fire-and-forget 只保证发送侧 API 不等；不回则发送方每个请求挂 30s 超时 + error! 日志风暴，100ms 频率下不可接受）
- **不走 Register_Inbound / Core**（高频数据避免污染 Core 主循环；消费者直接订阅 robot_bus）

### 3. Broadcast

**文件：`Src/Network/node_handle.rs`**
- `NodeCommand` 加变体：`Broadcast { data_type: DataType, payload: Vec<u8> }`
- `NodeHandle` 加方法：`Broadcast(data_type, payload)` → `cmd_tx.send(NodeCommand::Broadcast)` 立即返回（fire-and-forget）

**文件：`Src/Network/command_handler.rs`（Handle_Command）**
- 加臂：`NodeCommand::Broadcast` → `Get_All_Peers().await` 遍历 → 逐个 `send_request`（**不注册 oneshot** = fire-and-forget，现成逻辑支持 `response_tx: None`）
- 事件循环负载：位姿 100ms × N peers，`send_request` 仅入队，轻量可接受

### 4. main.rs 组装（主枝入口）

- 创建 robot_bus（`EventBus::New(1024)`）
- `Network_Service` 初始化时传入 robot_bus
- （Robot 侧注入：`Arc<NodeHandle>` + robot_bus 传给 `Robot::launch`——见 Task 9）

## 文件计划

| 文件 | 改动 |
|---|---|
| `Src/Network/Request_Response/codec.rs` | DataType 加 Robot=3 + From_U8 臂 |
| `Src/Network/swarm_events.rs` | 分流加 Robot 臂（robot_bus.Publish + 回 OK） |
| `Src/Network/node_handle.rs` | NodeCommand 加 Broadcast 变体 + NodeHandle::Broadcast 方法 |
| `Src/Network/command_handler.rs` | Handle_Command 加 Broadcast 臂（遍历 peers fire-and-forget） |
| `Src/main.rs` | 创建 robot_bus + 注入 Network_Service |

> ⚠️ Task 9_2 后续会重构：`main.rs` 的 Phase 1~6 将抽取为 `Src/bootstrap.rs::core_bootstrap()`（含 robot_bus 创建与注入），`main.rs` 变薄——robot_bus 相关逻辑随抽取迁移，行为不变。

## 实施步骤

| 步骤 | 内容 | 验证 |
|---|---|---|
| S1 | codec.rs：DataType::Robot=3 + From_U8 | codec 单测 |
| S2 | node_handle.rs：NodeCommand::Broadcast + NodeHandle::Broadcast 方法 | 编译 |
| S3 | command_handler.rs：Broadcast 臂（Get_All_Peers 遍历 + fire-and-forget） | 编译 |
| S4 | swarm_events.rs：Robot 分流臂（robot_bus.Publish + 回 OK） | 编译 |
| S5 | main.rs：robot_bus 创建 + Network_Service 注入 | `./build.sh check` |
| S6 | 回归：Network/全量测试 + 实车/多节点验证（与 Task 9 联调） | `./build.sh test` |

## 待决策问题

（无——设计已收敛）

## 已决策

| # | 问题 | 决策 |
|---|------|------|
| 1 | robot_bus 引入方式 | 直接加参数（不改 EventBus 核心，不做数组化） |
| 2 | 入站 Robot 数据路径 | 直接 Publish robot_bus，不走 Register_Inbound/Core（Info 模式） |
| 3 | 入站是否回复 | 回最小响应 "OK"（闭合协议状态机，防发送方超时风暴） |
| 4 | 入站 payload | 原样转发（序列化/解析是上层职责） |
| 5 | 广播实现 | NodeCommand::Broadcast + 事件循环遍历 peers + fire-and-forget |
| 6 | payload 协议归属 | Robot 侧 Task 9 负责（Network 只搬运字节） |
| 7 | 全量地图 | 走 Broadcast（65KB < 2GB 帧上限），不落盘 |
| 8 | 使用时机衔接（Task 9_2 确认） | **map_full 本次不广播**（传输能力已具备，全量地图留后续）；payload 协议已定：位姿/地图 delta JSON 带 `peer_id` + `peer_name` 双字段，peer_name=车名 |

---

## 人类评审

<!-- 在此区域写下评审意见 -->
