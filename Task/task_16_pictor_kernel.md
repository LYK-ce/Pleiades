# Task 16: Pictor Kernel — Pleiades × Godot GDExtension 桥

> 创建日期：2026-08-15
> 状态：方案定稿（待实施）
> 范围：Rust 侧（Orion）改动——workspace 化 + 无头模式 + 命令 request-response 路由 + 地面站消费侧 + 桥 crate `pictor-kernel`
> 关联：`/vepfs-mlp2/c20250205/240804016/GodotProject/Pictor/docs/pleiades_godot_integration_guide.md`（集成指南）

---

## 一、背景

目标架构：**Pleiades（无头）= 逻辑层，Godot = 表现层，中间 GDExtension 桥（单进程）**。

| 代码库 | 位置 | 角色 |
|---|---|---|
| Orion（Rust） | `/Workspace/Orion/` | 逻辑层（libp2p 网络 / ORION 协议 / 地图合并 / LLM） |
| Pictor（Godot） | `/GodotProject/Pictor/` | 表现层（渲染 / UI / 输入） |

桥的物理形态 = Rust 编译的 `libpictor_kernel.so`（cdylib）+ Godot `.gdextension` 注册文件。

替换对象：现有 WebSocket 遥控链路（`Src/WebSocket/server.rs` → `parse_orion_frame` → `robot_cmd_tx`）。

---

## 二、已定决策

| 决策点 | 结论 |
|---|---|
| 仓库结构 | 方案 A：Orion 根目录 workspace 化，桥 crate 放 `SrcPictorKernel/`（与 `Src/` 并列）✅ 已实施 |
| 桥依赖 | `godot 0.5` + feature `api-4-7`（匹配 Pictor 的 Godot 4.7）✅ 已锁定 |
| Godot 暴露方式 | **方案 B**：一个方法 `send_command(peer_hex, frame)`，Godot 拼好 ORION 帧，桥只转发（零协议知识） |
| 命令入站路由 | 走 `robot_bus` 数据面：Network 层投帧 + 回 ACK，车端 `command_consumer` 解码 → `robot_cmd_tx` |

---

## 三、方案设计

### 3.1 出站方向（Godot → 车）：一个方法 `send_command`（方案 B ✅ 已确认）

**结论**：桥退化成"哑管道"——Godot 拼好完整 ORION 帧，桥只转发。

**现状依据**（Godot 已有完整协议实现 + fan-out，均为现状，非新增）：
- `src/websocket/protocol/`：`orion_frame.gd`（帧编解码）、`orion_messages.gd`（5 消息 payload 编解码 + `Build_Cmd` 组装完整帧）、`message_builder.gd`（命令构造）、`message_parser.gd`（解析）、`protocol_def.gd`（常量）+ `test/test_orion_protocol.gd`。
- `websocket_manager.gd::_on_cmd_send`：`for id in targets: ws.send_binary(frame)` 已做群发 fan-out，members 按 selected_ids 填充。

**分层**：

```
GDScript（现有 message_builder + orion_messages.Build_Cmd + websocket_manager）
   │  拼完整帧 + fan-out：for id in targets: kernel.send_command(peer_hex, frame)
   ▼
pictor-kernel：一个 handle send_command（薄胶水，同步）
   │  parse_peer_hex(hex → PeerId)
   ▼
NodeHandle::Send_Data_Try(peer, DataType::Robot, frame)   ← 只转发，不解析、不编码
```

**改动 1：`NodeHandle` 新增 `Send_Data_Try`（`Src/Network/node_handle.rs`）**

```rust
/// 发送数据（同步 fire-and-forget：channel 满即丢弃，不等待 Response）
pub fn Send_Data_Try(&self, peer: &PeerId, data_type: DataType, payload: Vec<u8>)
    -> Result<(), Box<dyn Error + Send + Sync>> {
    self.cmd_tx
        .try_send(NodeCommand::SendData { peer: *peer, data_type, payload, response_tx: None })
        .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })?;
    Ok(())
}
```

- 底层 `NodeCommand::SendData.response_tx` 已是 `Option`，`None` = 不回执；`command_handler.rs` 已处理 `None`，**网络层其余零改动**。
- `try_send` 同步非阻塞；失败仅代表 100 容量队列满/关闭。

**改动 2：pictor-kernel 一个 handle（薄胶水，同步）**

```rust
#[func]
fn send_command(&self, peer_id: GString, frame: PackedByteArray) -> bool {
    let Some(peer) = parse_peer_hex(&peer_id.to_string()) else { return false };
    let Some(node) = self.node_handle() else { return false };   // 初始化未完成
    node.Send_Data_Try(&peer, DataType::Robot, frame.to_vec()).is_ok()
}

fn parse_peer_hex(s: &str) -> Option<PeerId> {
    hex::decode(s).ok().and_then(|b| PeerId::from_bytes(&b).ok())
}
```

**与现有代码的 1:1 对应**：
- `ws.send_binary(frame)` → `kernel.send_command(peer_hex, frame)`。
- peer_id 仍是 hex（Godot `_peer_ids` 已是 hex，`_Peer_Id_Bytes` 用 `hex_decode()`）。

**Pictor 侧保留/删除**：
- 保留：`src/websocket/protocol/`（帧/消息编解码，传输无关，换传输即复用）。
- 删除：`websocket_client.gd` / `websocket_manager.gd`（WS 传输层），其 fan-out 逻辑上移到桥调用方。

**权衡**：
- 协议双实现（Rust 车端 + GDScript 地面站）= 现状保持；Rust `messages.rs` roundtrip 测试当"协议真相"，GDScript `test_orion_protocol.gd` 对拍。
- 与 guide"删除 src/websocket/ 整套"的差异：只删传输层（client/manager），保留 protocol 编码层。

### 3.2 入站方向（车端收命令）：mpsc 命令通道 ✅ 已确认

**结论**：命令（request-response，单播）走**独立的 mpsc 通道**，不混入 `robot_bus` 遥测广播。

**原则**：广播进来的走广播总线，单播进来的走单播通道——控制面/数据面分离。

```
gossipsub（广播）        → robot_bus（broadcast） → cluster_consumer → 表/地图
request-response（单播） → mpsc 通道（unicast）    → command_consumer → robot_cmd_tx
```

**改动 1：`Network_Service` 启用 `DataType::Robot` 分支（`Src/Network/swarm_events.rs`）**

现状：`warn!("收到 DataType::Robot 消息（已废弃，走 gossipsub），忽略")`。
改为：收帧 → 投命令通道 + 回 ACK：

```rust
DataType::Robot => {
    // 命令帧 → 车端命令通道（unicast，不混入 robot_bus 遥测广播）
    if let Err(e) = self.robot_cmd_frame_tx.send(request.payload).await {
        warn!("robot 命令通道发送失败: {e}");
    }
    // 回 ACK 闭环 request-response（协议卫生；发送方 Send_Data_Try 不读回执）
    let response = Network_Data { data_type: DataType::Robot, payload: b"OK".to_vec() };
    if let Err(e) = self.swarm.behaviour_mut().request_response.send_response(channel, response) {
        error!("Robot 入站回复失败: {e}");
    }
}
```

**改动 2：`command_consumer` task（`Robot::launch` 里 spawn，与 `cluster_consumer` 对称）**

```rust
pub async fn command_consumer(
    mut rx: mpsc::Receiver<Vec<u8>>,       // 原始帧字节
    robot_cmd_tx: mpsc::Sender<Command>,   // 解码后命令
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(bytes) => {
                    if let Some(f) = decode_frame(&bytes) {
                        if let Some(cmd) = parse_orion_frame(&f) {
                            let _ = robot_cmd_tx.send(cmd).await;
                        }
                    }
                }
                None => break,
            },
            _ = cancel.cancelled() => break,
        }
    }
}
```

**改动 3：接线（`Src/bootstrap.rs`）**

- `core_bootstrap`：`let (robot_cmd_frame_tx, robot_cmd_frame_rx) = mpsc::channel::<Vec<u8>>(64);`，sender 传 `Network_Service::Init`，receiver 存 `CoreBootstrap`。
- `robot_bootstrap`：receiver 传 `Robot::launch`（新参数）。
- `Robot::launch`：spawn `command_consumer(robot_cmd_frame_rx, cmd_tx.clone(), cancel)`。

**改动 4：`parse_orion_frame` 搬家**

从 `Src/WebSocket/protocol.rs` 挪到 `Src/Robot/core/protocol/`（或 `Src/Robot/remote/`）——WebSocket 退役后它仍被 `command_consumer` 复用，不能留在 WS 目录。

**数据流**：

```
Network_Service DataType::Robot 分支
   └─ robot_cmd_frame_tx.send(帧字节) + ACK        ← 新 mpsc（传原始帧）
              │
command_consumer（Robot::launch）                  ← 解码
   └─ decode_frame → parse_orion_frame → Command
              │
robot_cmd_tx（已有 mpsc）→ main_loop → STM32
```

**说明**：地面站是命令发送方、不接收命令，因此**无需** `command_consumer`——它只存在于车端节点。

### 3.3 无头模式（`run_headless`）✅ 已确认

**结论**：新增 `CoreBootstrap::run_headless()`，跳过 TUI；桥在后台线程 + 自己的 tokio runtime 上跑它。

**改动：`Src/bootstrap.rs` 加 `run_headless()`（约 10 行）**

现有 `run()` = spawn 网络循环 → spawn TUI → `core.run()` 阻塞；无头版去掉中间 TUI：

```rust
pub async fn run_headless(self) {
    let mut network_service = self.network_service;
    tokio::spawn(async move {
        if let Err(e) = network_service.Start().await { /* log */ }
    });
    // 无 TUI_Loop；user_cmd_tx 直接 drop
    self.core.spawn_initial_flush();
    self.core.run().await;
}
```

现有 `main.rs` / `main_robot.rs` 继续用 `run()`，不改。

**启动/关闭时序（Godot 宿主，单进程）**

- 老架构（WS）：Pleiades 与 Pictor 两独立进程，Pleiades 先跑、Pictor 后连。
- 新架构（桥）：**Godot 是宿主**，Pleiades 是被加载的 `.so`，顺序反转。

```
Godot 启动 → dlopen .so → 注册 PleiadesKernel 类
  → 实例化节点 _ready → std::thread::spawn(|| { Runtime + core_bootstrap + run_headless })
```

- 关闭对称：`_exit_tree` → 通知优雅停机（cancel）→ join 线程 → 再卸载 `.so`。

**未就绪窗口**：`core_bootstrap` 异步、耗时，`NodeHandle` 就绪前 `send_command` 返回 false；桥就绪后发 `ready` 信号，Godot 收到前不发送。

**无头 ≠ 精简**：地面站跑完整 `core_bootstrap`（config / 身份 / 日志 / 网络 / Core / ML），仅去 TUI——guide 要求 LLM 并入 Pleiades，地面站需要 ML。

**订阅时序**：GroundStation 同步层须在 `run_headless()`（内含 `spawn_initial_flush`）之前订阅 `event_bus` / `robot_bus`，否则漏掉初始一次性事件（broadcast 无重放）。

### 3.4 地面站消费侧（GroundStation + 同步层）✅ 已确认

**结论**：GroundStation 复用 `cluster_consumer` 消费 `robot_bus` 遥测；同步层监听 `event_bus` + `robot_bus`，把数据经"信号 + 主线程 poll"交给 Godot。

**要同步的数据（两套视图并存）**：

| 视图 | 数据源 | 目标信号 | 说明 |
|---|---|---|---|
| 节点列表（网络面板） | `event_bus`（mDNS+连接） | `peer_connected` / `peer_disconnected` | 收拢所有 node（车+地面站），复刻 TUI network_panel |
| 节点名字 | `event_bus`（peer-info gossip） | `peer_info_updated` | `peer_connected` 不带名字，名字单独来 |
| 地图车（位姿） | `robot_bus` → `cluster_consumer` → `ClusterInfoTable` | `pose_received`（按车发） | 车在哪 / 朝向 |
| 合并地图 | `robot_bus` → `cluster_consumer` → `OccupancyGrid` | `map_updated`（全量+增量） | 合并已在 Rust，Godot 直接渲染 |

**节点 ≠ 车（两套视图）**：
- 节点列表 = peer 层（mDNS+连接），不管车还是地面站都进列表。
- 地图车 = pose，只有发 pose 的（车）才画在地图上。
- 不做 node_type 区分（YAGNI；将来需要时用"has pose"现成信号，不必改 config）。

**结构（三个小块）**：

- ① `cluster_consumer`（**复用**）：订阅 `robot_bus` → 解码 → `ClusterInfoTable` + `OccupancyGrid`。
- ② 事件监听（新 task）：订阅 `event_bus` → 解析 State JSON 的 `type`（`peer_connected`/`peer_disconnected`/`peer_discovered`/`peer_info_updated`）。
- ③ 转发（sync task）：读 ①② 的产出 → 塞入线程安全队列 → Godot `poll()` 排空并 emit 信号。

**跨线程发信号决策（A/B/C）**：

| 方案 | 机制 | 代价 |
|---|---|---|
| A. 跨线程直接发信号 | 开 `experimental-threads`，后台线程直接 `emit_signal` | 官方标"high risk of unsoundness"（未定义行为风险），不采用 |
| B. 信号 + 主线程 poll ✅ | 后台塞 channel；Godot `_process` 调 `poll()`，`poll()` 在主线程排空队列并 emit 信号 | Godot 侧多一次 `poll()`；零 unsoundness |
| C. 纯 pull（不走信号） | Godot 轮询拿数据自己刷新 | 放弃 Godot 信号惯用法 |

**决策：采用 B**。Godot 侧仍是纯正信号连接（`pose_received.connect(...)` 等），只是信号发射点落在主线程的 `poll()` 里，不开 `experimental-threads`。

**GroundStation 位置**：放 pleiades（`Src/Robot/` 下，与 `cluster_consumer` 同层）——消费逻辑是通用机器人域、可单测；桥只做"读快照 + 发信号"。

**信号粒度（已确认，与现有 WS 模型保持一致）**：
- pose：**按车发** `pose_received(peer_id, pose)`——复刻现有 `EventBus.pose_received`（`renderer_2d._on_pose` 几乎不动，只换 vehicle_id→peer_id）。
- map：**全量 + 增量**——保持现有 `map_full`（初始化）+ `map_delta`（更新）两套；合并从 Godot 挪到 Rust，Rust 发"合并后的全量/增量"。

### 3.5 桥类 PleiadesKernel（`SrcPictorKernel/lib.rs`）✅ 已确认

**定位**：Godot 里唯一的 Rust 类（`#[derive(GodotClass)]` extends Node），是桥的全部对 Godot 接口。

**字段**：

```rust
#[derive(GodotClass)]
#[class(base = Node)]
struct PleiadesKernel {
    base: Base<Node>,
    node_handle: Arc<OnceLock<NodeHandle>>,       // 出站发送（core_bootstrap 就绪后填充）
    out_queue: Arc<Mutex<VecDeque<BridgeEvent>>>, // 同步队列（后台 → 主线程 poll）
    shutdown: Arc<AtomicBool>,                     // 优雅停机标志
    worker: Option<JoinHandle<()>>,                // 后台线程句柄
}
```

**信号（上行，Rust → Godot）**：

| 信号 | 参数 | 来源 |
|---|---|---|
| `ready` | — | 后台 core_bootstrap 完成、NodeHandle 就绪 |
| `pose_received` | `peer_id: String, x, y, yaw, vx, vy` | ClusterInfoTable 快照（按车发） |
| `map_updated` | `data: PackedByteArray` | OccupancyGrid（全量 + 增量） |
| `peer_connected` / `peer_disconnected` | `peer_id: String` | event_bus（mDNS+连接） |
| `peer_info_updated` | `peer_id: String, peer_name: String` | event_bus（peer-info gossip） |

**handle（下行，Godot → Rust）**：

| handle | 参数 | 说明 |
|---|---|---|
| `send_command` | `peer_id: String, frame: PackedByteArray` | hex→PeerId → `Send_Data_Try`（同步，立即返回 bool） |
| `get_peers` | — | 返回节点列表快照（Godot 初始化用） |
| `poll` | — | 主线程排空 `out_queue` → emit 信号 |

注：方案 B 下 `send_command` 同步返回 bool，故 guide 里的 `cmd_result` 信号可省略（结果即返回值）。

**生命周期**：

- `_ready`：`std::thread::spawn` 后台线程（`Runtime + core_bootstrap + GroundStation + run_headless`）；就绪后填 `node_handle` + 发 `ready`。
- 后台同步 task：周期读 table/grid/event_bus → 塞 `out_queue`。
- Godot `_process`：调 `poll()` → 排空队列 → emit 信号。
- `_exit_tree`：置 `shutdown` → 后台优雅停机 → `join` 线程 → 卸载。

---

## 四、改动文件清单

**新建**：

| 文件 | 内容 |
|---|---|
| `SrcPictorKernel/Cargo.toml` | 桥 crate 清单（cdylib + pleiades + godot）✅ 已建 |
| `SrcPictorKernel/lib.rs` | PleiadesKernel 类 + 信号 + handle + poll ✅ 骨架已建，待填 |
| `Src/Robot/core/command_consumer.rs` | 命令帧 → Command（`decode_frame` + `parse_orion_frame`） |
| `Src/Robot/core/ground_station.rs` | GroundStation 消费侧（table + grid + spawn `cluster_consumer`） |

**修改**：

| 文件 | 改动 |
|---|---|
| `Cargo.toml` | workspace 化 ✅ 已改 |
| `Src/Network/node_handle.rs` | + `Send_Data_Try` |
| `Src/Network/network_service.rs` | + `robot_cmd_frame_tx` 字段 + Init 参数 |
| `Src/Network/swarm_events.rs` | `DataType::Robot` 分支启用（投帧 + ACK） |
| `Src/bootstrap.rs` | + `run_headless`；core_bootstrap 建命令 mpsc；robot_bootstrap 传 receiver |
| `Src/Robot/core/robot.rs` | `Robot::launch` + spawn `command_consumer` |
| `Src/Robot/core/protocol/` | + `parse_orion_frame` 迁入（从 `WebSocket/protocol.rs`） |

**退役（可暂留调试）**：

| 文件 | 说明 |
|---|---|
| `Src/WebSocket/`（server.rs / protocol.rs / mod.rs） | WS 遥控退役；`parse_orion_frame` 先迁走 |

---

## 五、文件架构（改造后）

```
Orion/                                   workspace 根
├── Cargo.toml                           [workspace] + Pleiades package ✅
├── Src/                                 pleiades 库（逻辑层）
│   ├── main.rs / main_robot.rs          两个 bin（照旧）
│   ├── bootstrap.rs                     + run_headless + 命令通道接线
│   ├── Network/
│   │   ├── node_handle.rs               + Send_Data_Try
│   │   ├── network_service.rs           + robot_cmd_frame_tx
│   │   └── swarm_events.rs              + DataType::Robot 启用
│   ├── Robot/
│   │   └── core/
│   │       ├── robot.rs                 + command_consumer spawn
│   │       ├── command_consumer.rs      【新】命令入站消费
│   │       ├── ground_station.rs        【新】地面站消费侧
│   │       ├── protocol/                + parse_orion_frame 迁入
│   │       └── cluster/ ...             （照旧，被复用）
│   └── WebSocket/                       退役（可暂留调试）
└── SrcPictorKernel/                     pictor-kernel crate（桥，cdylib）
    ├── Cargo.toml                       ✅ 已建
    └── lib.rs                           【待填】PleiadesKernel 类
```

---

## 六、实施步骤（按依赖顺序）

**第 1 步：`Send_Data_Try`（出站基础）**
- 文件：`Src/Network/node_handle.rs`
- 内容：照 `Gossipsub_Publish_Try` 加 `Send_Data_Try`（`try_send` + `response_tx: None`）。
- 验证：`cargo check` + 单测（队列满 → Err；正常 → Ok）。

**第 2 步：`run_headless`（无头基础）**
- 文件：`Src/bootstrap.rs`
- 内容：加 `run_headless()`（跳过 TUI spawn）。
- 验证：`cargo check`。

**第 3 步：命令入站链路（车端）**
- 文件：`network_service.rs`（+`robot_cmd_frame_tx`）、`swarm_events.rs`（启用分支）、`bootstrap.rs`（建 mpsc 接线）、`robot.rs`（spawn）、新建 `command_consumer.rs`、`parse_orion_frame` 搬家。
- 验证：单测 `command_consumer`（帧 → Command）；roundtrip。

**第 4 步：GroundStation 消费侧**
- 文件：新建 `Src/Robot/core/ground_station.rs`
- 内容：table + grid + spawn `cluster_consumer` + `snapshot()` 方法。
- 验证：单测（喂遥测帧 → table/grid 更新）。

**第 5 步：桥 crate（PleiadesKernel）**
- 文件：`SrcPictorKernel/lib.rs`
- 内容：类 + 信号 + handle + poll + 后台线程 + 同步 task。
- 验证：`cargo build -p pictor-kernel` 产出 `libpictor_kernel.so`。

**第 6 步：Godot 侧接入（对应 guide P3）**
- Pictor：拆 WS 栈，接桥信号/handle，`_process` 调 `poll()`。
- 验证：Godot 显示真实 pose/map，群发 Goto 跑通。

**第 7 步：WS 退役 + e2e（对应 guide P4）**
- 退役 `Src/WebSocket/`；断线重连验证。
- 验证：拔网线 → 自动恢复；多车联调。
