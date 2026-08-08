# Task 18: Identify Gossipsub — 接入 libp2p 标准 identify + gossipsub 模块

> 状态：方案已确认（identify ✅ + gossipsub ✅），待实施
> Created Date ： 2026-08-08
> Modified Date ： 2026-08-08

---

## 背景

系统自研了两套"本可由 libp2p 标准模块提供"的机制，Task 18 一次性替换：

| 自研机制 | 替代的 libp2p 标准模块 | 状态 |
|----------|----------------------|------|
| `DataType::Info` 交换（`name\|models\|sessions`，request-response） | **identify**（`libp2p::identify`） | ✅ 方案已定 |
| `broadcast_local_info`（O(n) 逐 peer 广播业务状态） | **gossipsub**（`libp2p::gossipsub`，0.49.0） | ✅ 方案已定 |

**职责划分（已确认）**：

```
mDNS      → 发现节点（谁在？在哪？）            [现成，不变]
identify  → 识别节点（是谁？支持什么协议？监听地址？） [自动，连接建立即交换]
gossipsub → 业务状态广播（name / models+层位图 / sessions） [3 个 topic]
```

**本次同时落地 identify + gossipsub**，自研 Info 机制**一次性全部移除**（无需过渡保留）。

---

## 一、identify 集成

### 目标

1. 启用 libp2p `"identify"` feature，加入 `PleiadesNetworkBehaviour`
2. identify 自动接管"连接建立后交换协议级元信息"（protocols / listen_addrs / public_key / versions）
3. `identify::Event::Received` 记录对端元信息（日志 + 更新 PeerManager addresses）

### 涉及文件

| 文件 | 位置 | 改动 |
|------|------|------|
| `Cargo.toml` | 36-46 | features 加 `"identify"` |
| `Src/Network/network_service.rs` | 18-27 | use 加 `identify` |
| `Src/Network/network_service.rs` | 108-120 | `PleiadesNetworkBehaviour` 加 `pub identify: identify::Behaviour` |
| `Src/Network/network_service.rs` | 216-256 | `with_behaviour` 闭包创建 identify |
| `Src/Network/swarm_events.rs` | match 分发 | 加 `PleiadesNetworkBehaviourEvent::Identify(event)` 分支 |
| `Src/Network/swarm_events.rs` | 新增函数 | `Handle_Identify_Event` |

### 实施要点

```rust
// with_behaviour 闭包内（key: &Keypair）
let identify = identify::Behaviour::new(
    identify::Config::new(
        "/pleiades/1.0.0".to_string(),   // protocol_version
        key.public().clone(),            // PublicKey（注意不是 PeerId）
    )
);
```

- 可选配置：`.with_agent_version("pleiades/0.1.0")`、`.with_interval(...)`、`.with_push_listen_addr_updates(true)`
- `Handle_Identify_Event::Received`：记日志（agent/protocols/listen_addrs）+ `Upsert_Peer(PeerInfo::new(peer_id, info.listen_addrs))` 更新地址列表
- **决策点已定（A/A）**：
  - 不新增 PeerInfo.protocols 字段（同构节点无消费方，protocols 打日志即可）
  - ConnectionEstablished 手动 Info 发送**删除**（gossipsub 本次一起落地接管业务状态；仅保留 `DataType::Info` 枚举变体，见 2.4）

---

## 二、gossipsub 集成

### 2.1 设计（已确认）

**3 个 topic**：

| Topic | 内容 | 变更频率 | 消费方 |
|-------|------|---------|--------|
| `"pleiades/peer-info"` | `{"peer_id", "name"}`（未来扩展算力/内存等硬件信息） | 低频 | TUI、调度器（资源评估） |
| `"pleiades/models"` | `Vec<SupportedModel>`（id/file_name/layer_bitmap 层位图） | 中频 | 调度器（模型分配）⭐ |
| `"pleiades/sessions"` | `Vec<SessionSummary>`（session_id/model_id/槽位） | 高频 | 调度器、TUI |

**消息认证**：`MessageAuthenticity::Signed(Keypair)`——系统已有持久化 Ed25519 keypair；符合默认 `ValidationMode::Strict`；接收端 `message.source` 即作者 PeerId。

**发布时机**：
1. 连接建立后（替代 ConnectionEstablished 手动 Info 发送）
2. Flush 后（现 `core.rs:193-195`）
3. session 创建后（现 `branch_user.rs:393`）
4. 收到 `Event::Subscribed` 时补发（弥补 gossipsub 无历史回放）

### 2.2 涉及文件

| 文件 | 位置 | 改动 |
|------|------|------|
| `Cargo.toml` | 36-46 | features 加 `"gossipsub"` |
| `Src/Network/network_service.rs` | 108-120 | 加 `pub gossipsub: gossipsub::Behaviour` |
| `Src/Network/network_service.rs` | 216-256 | 闭包内建 Config + `Behaviour::new(...)`（见 2.3） |
| `Src/Network/network_service.rs` | Start()（listen_on 后） | 订阅 3 个 topic |
| `Src/Network/swarm_events.rs` | match 分发 | 加 `Gossipsub` 分支 |
| `Src/Network/swarm_events.rs` | 新增函数 | `Handle_Gossipsub_Event`（迁入 195-253 的解析/TUI 逻辑） |
| `Src/Network/swarm_events.rs` | 45-52 | **删除** ConnectionEstablished 手动 Info 发送 |
| `Src/Network/swarm_events.rs` | 195-253 | **删除** 入站 Info 解析（逻辑迁入 gossipsub handler；match 需加忽略分支处理残留 Info 消息） |
| `Src/Network/Request_Response/codec.rs` | 40, 50 | **保留**（`DataType::Info` 变体 + From_U8 分支，唯一保留项） |
| `Src/VM/capability_binding.rs` | 345 | **删除** `"Info"` 映射（已验证 programs/ 无脚本使用） |
| `Src/Network/mod.rs` | 81-160 删除，新增 | topic 常量 + `publish_peer_info/publish_models/publish_sessions` |
| `Src/Network/command_handler.rs` | Handle_Command | 加 `NodeCommand::GossipsubPublish` 分支 |
| `Src/Network/node_handle.rs` | 27-43, 51-75 | `NodeCommand` 加变体 + `NodeHandle::Gossipsub_Publish` 方法 |
| `Src/Network/capability.rs` | trait + impl | 加 `publish_gossipsub` 方法 |
| `Src/Orchestrator/core.rs` | 193-195 | `broadcast_local_info` → `publish_models` + `publish_sessions`（原调用移除） |
| `Src/Orchestrator/core/branch_user.rs` | 393 | → `publish_sessions`（原调用移除） |
| `Src/Orchestrator/mod.rs` | 56-78 | `StubNetwork` 补 `publish_gossipsub` stub |
| `tests/t07_network_integration.rs` | 291-321 | **不动**（`DataType::Info` 保留，TC-05 继续编译） |

### 2.3 实施要点

#### ⚠️ 0.49.0 API 纠正（子agent 实测源码）

```rust
// 正确构造（0.49.0）：
// pub fn new(privacy: MessageAuthenticity, config: Config) -> Result<Self, &'static str>
// ❌ 不是 new(peer_id, config)（旧版本 API）

let gossipsub_config = gossipsub::ConfigBuilder::default().build()?;   // Result<Config, ConfigBuilderError>
let gossipsub = gossipsub::Behaviour::new(
    gossipsub::MessageAuthenticity::Signed(key.clone()),   // 作者 = key 所有者
    gossipsub_config,
)?;   // Result<Self, &'static str>
```

- 默认 `ValidationMode::Strict` 要求签名，`Signed` 正好满足；用 `Anonymous`/`Author` 需改配置，不采用
- `subscribe(&Topic) -> Result<bool, SubscriptionError>`；`publish(topic: impl Into<TopicHash>, data) -> Result<MessageId, PublishError>`
- **无订阅者时 publish 返回 `PublishError::NoPeersSubscribedToTopic`**——正常现象，仅 warn 不报错

#### 发布路径（关键设计）：NodeCommand 通道

core.rs / branch_user.rs 只有 `Capabilities`（无 swarm 访问），gossipsub publish 必须走与现有 `SendData` 同构的通道：

```
Orchestrator（core.rs / branch_user.rs）
  → Network_Capability::publish_gossipsub(topic, payload)   [trait 新方法]
    → NodeHandle::Gossipsub_Publish(topic, payload)          [NodeCommand::GossipsubPublish]
      → Network_Service::Handle_Command
        → swarm.behaviour_mut().gossipsub.publish(topic, payload)
```

#### 事件处理（Handle_Gossipsub_Event）

```rust
match event {
    gossipsub::Event::Message { message, .. } => {
        let Some(author) = message.source else { return; };   // Signed 模式 = 作者
        match message.topic.as_str() {
            TOPIC_PEER_INFO => /* JSON {peer_id,name} → Upsert_Peer(name) + TUI 通知 */,
            TOPIC_MODELS    => /* JSON Vec<SupportedModel> → Update_Supported_Models(&author) + TUI */,
            TOPIC_SESSIONS  => /* JSON Vec<SessionSummary> → Update_Sessions(&author) + TUI */,
            _ => {}
        }
    }
    gossipsub::Event::Subscribed { peer_id, .. } => { /* 可选：补发对应 topic */ }
    gossipsub::Event::Unsubscribed { .. } => {}
    gossipsub::Event::GossipsubNotSupported { peer_id } => { /* 日志 */ }
    gossipsub::Event::SlowPeer { .. } => {}
}
```

TUI 通知逻辑（`peer_info_updated`，现 swarm_events.rs:219-243）**原样迁入**，schema 不变（peer_id/peer_name/is_local/models/sessions）。

#### ⚠️ 关键陷阱：`upsert_peer` 不保留 sessions

`peer_manager.rs:30-49` `upsert_peer` 的保留语义：保留 old 的 `connected_at/last_active/profile/supported_models/local`，**但 sessions 不保留**（整体覆盖）。

gossipsub 三 topic **分消息到达**后，若 `peer-info` 消息（只含 name）先到，会**清掉**此前 `sessions` 消息写入的 sessions。

**解决方案**（实施时选一）：
- a. peer-info 分支先 `Get_All_Peers()` 查已有 entries，合并 name 后再 `Upsert_Peer`
- b. 给 `peer_manager.rs upsert_peer` 补一行：sessions 为空时保留 old.sessions

推荐 **b**（一处改动，根治）。

### 2.4 Info 机制处置（用户决策 2026-08-08）

**只保留 `DataType::Info` 枚举变体（type 定义），所有 Info 业务逻辑删除**，gossipsub 成为唯一业务状态广播通道。

| # | 项 | 位置 | 处置 |
|---|-----|------|------|
| 1 | `build_local_info_payload` | `Network/mod.rs:81-87` | **删除**（业务逻辑） |
| 2 | `broadcast_local_info` | `Network/mod.rs:99-160` | **删除**（业务逻辑） |
| 3 | ConnectionEstablished 手动 Info 发送 | `swarm_events.rs:45-52` | **删除**（连接建立改走 gossipsub publish） |
| 4 | 入站 `DataType::Info` 解析 | `swarm_events.rs:195-253` | **删除**（逻辑迁入 gossipsub handler） |
| 5 | `DataType::Info` 变体 + From_U8 分支 | `codec.rs:40, 50` | **保留**（唯一保留项——type 定义） |
| 6 | Lua 绑定 `"Info"` 映射 | `capability_binding.rs:345` | **删除**（已验证 programs/ 无脚本使用） |
| 7 | Flush 后 broadcast 调用 | `core.rs:193-195` | **切换**为 publish_models + publish_sessions |
| 8 | session 创建后 broadcast 调用 | `branch_user.rs:393` | **切换**为 publish_sessions |

**⚠️ 编译注意**：
- 删除 195-253 后，`swarm_events.rs` 的 `match request.data_type` 不再穷尽（`DataType::Info` 仍在枚举中）——需加忽略分支：`DataType::Info => { warn!("收到 Info 类型消息（已废弃），忽略"); }` 或并入 catch-all
- `DataType::Info` 变体保留：被 `From_U8` 引用 + 测试 TC-05 引用，无 dead_code 警告
- `t07_network_integration.rs` TC-05（#[ignore]）：编译通过（type 保留），但入站已无 Info 解析，若运行会失败——**保持 #[ignore] 或改写为 Data 类型**
- **保留项**：`DataType::Command/Data/File`（控制消息可靠单播）、mDNS/Kademlia/Ping/Stream、TUI 消费端、EventBus 事件类型（`peer_discovered/peer_left/peer_connected/peer_disconnected/peer_info_updated` 全部保留）、`PeerManagement/*`（签名够用）

---

## 三、依赖影响

- **identify**：新增 `libp2p-identify 0.47.0`，传递依赖全部已在 lock（无冲突）
- **gossipsub**：新增 4 个 crate 需联网下载：
  - `libp2p-gossipsub 0.49.0`、`async-channel`（2.x）、`hashlink`（0.9.x，与现有 0.10 并存）、`hex_fmt`（0.3.x）
  - `libp2p-identity` 需加 `rand` feature（rand 0.8.5 已在 lock，无新 crate）
- 其余传递依赖（libp2p-core 0.43.2 / libp2p-swarm 0.47.1 / quick-protobuf 0.8.1 / unsigned-varint 等）全部已在 lock
- gossipsub 内部序列化走 quick-protobuf；我们的 payload 是应用层 JSON（serde_json），与库无关

---

## 四、验证计划

1. `./build.sh check` — 快速语法检查（需先联网拉取新 crate）
2. `./build.sh` + `./build.sh deploy` — 编译部署
3. 启动验证：
   - 日志出现 identify 交换记录（`identify 收到: ... proto=[...]`）
   - TUI 网络面板正常显示 peer 的 models/sessions（数据经 gossipsub 到达）
4. 双卡 2 节点 + 单卡 3 节点回归（instructions.md 测试 1/2）：
   - mDNS 发现 ✅、模型加载 ✅、流水线桥接 ✅、多轮对话 ✅
5. 重点回归场景：
   - 节点 B 启动晚于 A：B 应通过"连接建立后 publish + Subscribed 补发"拿到 A 的业务状态
   - Flush 后 models 广播正常
   - session 创建后 sessions 广播正常

---

## 五、同步修改（编译必需）

| 项 | 说明 |
|----|------|
| `Src/Orchestrator/mod.rs` StubNetwork | trait 加 `publish_gossipsub` 后需补 stub |
| `tests/t07_network_integration.rs` TC-05 | **保持 #[ignore]**（`DataType::Info` type 保留可编译；入站已无 Info 解析，运行会失败） |
| `swarm_events.rs` match | 删除 Info 分支后需加忽略分支（`DataType::Info => warn 忽略`） |
| 注释（可选） | `network_service.rs:18`、`inbound.rs:7-8` 的 "Data / Info" 文案 |

---

## 六、文件改动总览

```
修改（14 处）：
  Cargo.toml                              — features 加 "identify" + "gossipsub"
  Cargo.lock                              — 自动更新（5 个新 crate + rand feature）
  Src/Network/network_service.rs          — use + 2 字段 + 闭包创建 + 订阅 3 topic
  Src/Network/swarm_events.rs             — 2 个新分支 + Handle_Identify/Handle_Gossipsub
                                          — 删 45-52（Info 发送）+ 删 195-253（Info 解析）+ match 加 Info 忽略分支
  Src/Network/mod.rs                      — 删 build_local_info_payload + broadcast_local_info
                                          — 新增 3 topic 常量 + publish_peer_info/publish_models/publish_sessions
  Src/Network/command_handler.rs          — GossipsubPublish 分支
  Src/Network/node_handle.rs              — 命令变体 + 方法
  Src/Network/capability.rs               — publish_gossipsub trait + impl
  Src/Network/Request_Response/codec.rs   — 不动（Info 变体保留，唯一保留项）
  Src/VM/capability_binding.rs            — 删 "Info" 映射
  Src/Orchestrator/core.rs                — broadcast 调用 → publish_models + publish_sessions
  Src/Orchestrator/core/branch_user.rs    — broadcast 调用 → publish_sessions
  Src/Orchestrator/mod.rs                 — StubNetwork 补 stub
  Src/PeerManagement/peer_manager.rs      — upsert_peer 保留 sessions（陷阱修复）

不动：Src/TUI/*、Src/EventBus/*、Src/PeerManagement/capability.rs、Src/Storage/*、Src/Session_Manager/*、mDNS 链路、
  codec.rs（Info 变体）、tests/t07_network_integration.rs（TC-05 保持 #[ignore]）
```
