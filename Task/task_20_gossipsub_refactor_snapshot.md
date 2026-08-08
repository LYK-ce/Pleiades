# task_20_gossipsub_refactor_snapshot

## 概述

重构 GossipSub 广播实现:发送侧解耦 + 新增快照机制。

**动机**(用户评审意见 2026-08-08):
1. `publish` 写成三份(`publish_peer_info` / `publish_models` / `publish_sessions`)——应只留一个 `publish_gossipsub(topic, payload)` capability,由业务层自己选 topic、自己构造 payload
2. 三个 `Build_*_Payload` 是业务序列化逻辑,不该塞进 `Network/Gossipsub/` 模块——payload 构建移到业务层(PeerManagement)
3. 补发机制(`publish_local_state_to_gossipsub`)在 Network 层构造业务 payload,同样违反分层——用**快照机制**取代:Network 层只缓存 bytes,**对方订阅 topic 时按 topic 精准重放**

**设计决策**(与用户讨论收敛):
- 快照缓存(`HashMap<topic, bytes>`)放 `Gossipsub` 模块,作为通用消息层机制(类似 MQTT retained message),Network 层不解析业务结构
- payload 构建方法由 PeerManagement(业务层)提供
- 无周期性广播开销;对方订阅 topic 即拿到该 topic 的最近快照(gossipsub 消息只投递给已订阅者,因此**订阅后才重放**,连接建立时不重放)
- 接收侧不动(Network 层消化,保持现状)

## 目标形态

```
业务层(PeerManagement): Build_*_Payload(local) → Vec<u8>   ← payload 构建
调用方(core.rs / branch_user.rs):
    构造 payload → 选 topic → caps.network.publish_gossipsub(topic, payload)
Network 层:
    publish_gossipsub(topic, payload)  → gossip.publish + snapshot_cache.update   ← 唯一发布口
    Subscribed { topic }               → 重放该 topic 的快照(订阅了才发)          ← 快照机制
    Handle_Gossipsub_Event             → 接收消化(不变)
```

## 涉及文件

| # | 文件 | 改动 |
|---|------|------|
| 1 | `Src/PeerManagement/peer_manager.rs`(或 mod.rs) | **新增** `Build_Peer_Info_Payload` / `Build_Models_Payload` / `Build_Sessions_Payload`(业务层 payload 构建,纯函数) |
| 2 | `Src/Network/Gossipsub/mod.rs` | **新增** `SnapshotCache` 结构(`update` / `all`);**删除** 3×`Build_*_Payload` + 3×`publish_*`;保留 topic 常量 |
| 3 | `Src/Network/network_service.rs` | **新增字段** `snapshot_cache: SnapshotCache`(struct 初始化) |
| 4 | `Src/Network/command_handler.rs` | `GossipsubPublish` 分支:publish 成功后 `self.snapshot_cache.update(topic, payload)` |
| 5 | `Src/Network/swarm_events.rs` | **删除** `publish_local_state_to_gossipsub` 及其 `ConnectionEstablished`(:46)调用;`Subscribed`(:411)改为**按事件携带的 topic 精准重放** `self.snapshot_cache.get(topic)` |
| 6 | `Src/Network/mod.rs` | re-export 更新:删 `publish_peer_info` / `publish_models` / `publish_sessions` |
| 7 | `Src/Orchestrator/core.rs` | `do_flush`:删 3 个 `publish_*` 调用,改为 PeerManagement `Build_*` + `publish_gossipsub` ×3 |
| 8 | `Src/Orchestrator/core/branch_user.rs` | `SetName`(:158):改 `Build_Peer_Info_Payload` + `publish_gossipsub`(保留本地 EventBus 通知);session 创建(:399):改 `Build_Sessions_Payload` + `publish_gossipsub` |
| 9 | `Src/Orchestrator/mod.rs` | `StubNetwork::publish_gossipsub` 已存在,无需改(核对) |

## 详细步骤

### 步骤 1:PeerManagement 新增 payload 构建(业务层)

在 `Src/PeerManagement/` 新增 3 个纯函数(参考现 `Gossipsub/mod.rs` 的序列化逻辑,原样迁移):

```rust
impl PeerManager {   // 或模块级 pub fn
    /// peer-info topic: {"peer_id", "name"}
    pub fn Build_Peer_Info_Payload(local: &PeerInfo) -> Vec<u8>;
    /// models topic: Vec<SupportedModel> JSON
    pub fn Build_Models_Payload(local: &PeerInfo) -> Vec<u8>;
    /// sessions topic: Vec<SessionSummary> JSON
    pub fn Build_Sessions_Payload(local: &PeerInfo) -> Vec<u8>;
}
```

### 步骤 2:Gossipsub/mod.rs — 新增 SnapshotCache,删除旧函数

```rust
/// 快照机制:缓存每个 topic 最近发布的 payload(类似 MQTT retained message)
pub struct SnapshotCache {
    snapshots: HashMap<String, Vec<u8>>,
}
impl SnapshotCache {
    pub fn new() -> Self;
    pub fn update(&mut self, topic: &str, payload: Vec<u8>);   // 发布时更新
    pub fn get(&self, topic: &str) -> Option<Vec<u8>>;          // 按 topic 取快照(精准重放用)
}
```

删除:`Build_Peer_Info_Payload` / `Build_Models_Payload` / `Build_Sessions_Payload` / `publish_peer_info` / `publish_models` / `publish_sessions`。
保留:`TOPIC_PEER_INFO` / `TOPIC_MODELS` / `TOPIC_SESSIONS` 常量。

### 步骤 3:network_service.rs — 持有 SnapshotCache

- `struct Network_Service` 新增字段 `snapshot_cache: SnapshotCache`
- 初始化处(`Init` / `new`):`snapshot_cache: SnapshotCache::new()`

### 步骤 4:command_handler.rs — 发布时更新快照

```rust
NodeCommand::GossipsubPublish { topic, payload } => {
    let topic_hash = gossipsub::TopicHash::from_raw(topic.clone());
    match self.swarm.behaviour_mut().gossipsub.publish(topic_hash, payload.clone()) {
        Ok(_) => { self.snapshot_cache.update(&topic, payload); ... }
        Err(e) => warn!(...),   // 无订阅者不更新快照?→ 决策:仍更新(业务状态已变,先缓存,等有订阅者再重放)
    }
}
```

> ⚠️ 决策点:无订阅者(publish 失败)时是否仍更新快照?建议**仍更新**——快照是"最近状态",与是否有订阅者无关,新节点加入时重放即可。

### 步骤 5:swarm_events.rs — 删补发,改按 topic 精准重放

- 删除 `publish_local_state_to_gossipsub`(方法本体 + ConnectionEstablished:46 调用处)
- **`ConnectionEstablished` 不重放**——对方可能尚未完成订阅,gossipsub 只投递给已订阅者,此时发布无效
- **`Subscribed` 事件按 topic 精准重放**(gossipsub 消息只投递给已订阅者,订阅完成后发布必达):

```rust
// Handle_Gossipsub_Event 的 Subscribed 分支:
gossipsub::Event::Subscribed { topic, .. } => {
    debug!("节点订阅 gossipsub topic: {}", topic);
    // 按 topic 精准重放:只重放对方刚订阅的 topic 的快照
    if let Some(payload) = self.snapshot_cache.get(topic.as_str()) {
        let _ = self.swarm.behaviour_mut().gossipsub.publish(
            gossipsub::TopicHash::from_raw(topic.as_str().to_string()),
            payload,
        );
    }
}
```

> 说明:对方启动订阅 3 个 topic → 收到 3 次 `Subscribed` 事件 → 各重放对应 topic 的快照,无冗余。
> `SnapshotCache` 需提供 `get(topic) -> Option<Vec<u8>>`(步骤 2 补充)。

### 步骤 6:core.rs do_flush — 业务层构造 + 发布

```rust
// 删:crate::network::publish_peer_info/models/sessions(...) ×3
// 改为:
let local = match caps.peer_manager.Get_Local_Peer().await { Ok(l) => l, Err(_) => return text };
caps.network.publish_gossipsub(TOPIC_PEER_INFO, PeerManager::Build_Peer_Info_Payload(&local)).await;
caps.network.publish_gossipsub(TOPIC_MODELS,    PeerManager::Build_Models_Payload(&local)).await;
caps.network.publish_gossipsub(TOPIC_SESSIONS,  PeerManager::Build_Sessions_Payload(&local)).await;
// 本地 TUI 事件:由 Storage::Flush() 内部 Sync_Models_To_Peer_Manager 发,不重复发
```

### 步骤 7:branch_user.rs — SetName / session 创建

- `SetName`(:149-171):`publish_peer_info(...)` 改为 `Build_Peer_Info_Payload` + `publish_gossipsub(TOPIC_PEER_INFO, ...)`;**保留**原 `peer_info_updated` 本地事件(publish_peer_info 内部发的,改为本分支显式发)
- session 创建(:399):`publish_sessions(...)` 改为 `Build_Sessions_Payload` + `publish_gossipsub(TOPIC_SESSIONS, ...)`;同样保留本地事件

### 步骤 8:Network/mod.rs — re-export 更新

`pub use Gossipsub::{...}` 删 `publish_peer_info, publish_models, publish_sessions`,保留 `TOPIC_*` 常量与 `SnapshotCache`。

### 步骤 9:编译与验证

- [ ] `./build.sh check` 无 error
- [ ] `cargo test --lib`(112 passed / 1 failed 预存 os 沙箱问题)
- [ ] 3 节点部署实测:
  - flush 后远端能看到本机模型(即时广播)
  - **新节点后启动**,能拿到其他节点的快照(快照重放验证)
  - set-name 传播到远端
  - session 创建状态同步
  - 无重复/闪烁(本地事件不再双发)

## 不做的事(本次范围外)

- 接收侧 `Handle_Gossipsub_Event` 不动(Network 消化,现状保持)
- 集成测试 71 错误修复(另行立项)
- 旧 `broadcast_local_info` 设计不评价、不回溯

## 状态

- 开始: 2026-08-08
- 结束: TBD
