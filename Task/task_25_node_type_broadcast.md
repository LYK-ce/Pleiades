# task_25_node_type_broadcast — 节点类型广播（peer-info 携带 node_type）

> Created Date ： 2026-09-01
> Modified Date ： 2026-09-01
> 状态：方案已定（存 PeerInfo，String 型），待实施
> 关联文档：`Architecture/robot_arch.md`（§3.5）、`docs/design_doc/orion_protocol.md`、`docs/design_doc/pictor_bridge_sync.md`

---

## 一、目标

让 Pictor（地面站）知道连入的节点是无人车（car）还是无人机（uav），据此选择正确的 sprite（图标）。

## 二、设计决策（已定稿）

| # | 决策 |
|---|------|
| D1 | node_type 传输格式 = **字符串**（`"car"` / `"uav"` / `"ground_station"`） |
| D2 | **存进 `PeerInfo`**，字段类型 `String`（空串 = 未知），与 `peer_name` 完全对称 |
| D3 | 范围 = gossip 广播 + `peer_info_updated` 事件 + terminal/Pictor Godot 信号 |
| D4 | 不考虑旧版本节点兼容（解析不到 node_type 就传空字符串，Pictor 端兜底） |
| D5 | `node_type` 用 `String` 而非 `NodeType` 枚举：① 对称 `name`；② PeerManagement 保持「不依赖 Config」的纯净性；③ enum→string 转换留在 bootstrap（Config 层） |

## 三、数据流

```
车/机节点: config [Identity].node_type ──Get_Node_Type().as_str().to_string()──> String
        → create_peer_management(peer_id, name, node_type) → PeerInfo.node_type
        → Build_Peer_Info_Payload(&local)  从 local.node_type 读，JSON 加 "node_type"
        → publish_gossipsub(TOPIC_PEER_INFO)
              ↓
地面站 terminal: swarm_events 解析 "node_type" → updated.node_type（upsert 空值保留旧值）
        → peer_info_updated 事件 JSON 加 node_type
        → Godot 信号 peer_info_updated(peer_id, peer_name, node_type)
        → Pictor 按 node_type 选 sprite
```

---

## 四、涉及文件

| # | 文件 | 改动性质 |
|---|------|---------|
| 1 | `pleiades-base/src/Config/config.rs` | `NodeType` 加 `as_str()` |
| 2 | `pleiades-base/src/PeerManagement/peer_info.rs` | `PeerInfo` 加字段 + `new`/`new_local` 改 |
| 3 | `pleiades-base/src/PeerManagement/peer_manager.rs` | `new`/`Default`/`Build_Peer_Info_Payload`/`upsert_peer` 改 |
| 4 | `pleiades-base/src/PeerManagement/mod.rs` | `create_peer_management` 加参数 |
| 5 | `pleiades-base/src/bootstrap.rs` | 读 `Get_Node_Type` 并传入 |
| 6 | `pleiades-base/src/Orchestrator/core.rs` | do_flush name 事件加字段 |
| 7 | `pleiades-base/src/Orchestrator/core/branch_user.rs` | SetName 事件加字段 |
| 8 | `pleiades-base/src/Storage/storage_manager.rs` | models 事件加字段（一致性） |
| 9 | `pleiades-base/src/Network/swarm_events.rs` | 接收解析 + 事件加字段 |
| 10 | `pleiades-terminal/src/lib.rs` | Godot 信号 + BridgeEvent 透传 |
| 11 | `docs/design_doc/pictor_bridge_sync.md` | 同步信号签名（文档） |
| — | Pictor GDScript（外部仓库） | 信号处理器加参数（不在本仓库） |

> 注：`pleiades-base/tests/t04_*` / `t07_*` 已整体过期（引用已删除的 `PeerHandle`/`Get_Peers`，且 `create_peer_management`/`PeerManager::new` 单参），当前本就不编译，属既有问题，不在本任务范围；若后续修复测试需同步补 `name` + `node_type` 参数。

---

## 五、详细实施步骤

### 步骤 1：`Config/config.rs` — `NodeType` 加 `as_str()`

在 `impl NodeType` 里（`from_str` 旁）加：

```rust
/// 序列化为 config.toml 一致的字符串（peer-info 广播用）
pub fn as_str(&self) -> &'static str {
    match self {
        NodeType::GroundStation => "ground_station",
        NodeType::Car => "car",
        NodeType::Uav => "uav",
    }
}
```

### 步骤 2：`PeerManagement/peer_info.rs` — PeerInfo 加字段 + 构造函数

```rust
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub name: String,
    /// 节点类型：car/uav/ground_station，空串 = 未知（对称 name）
    pub node_type: String,
    pub addresses: Vec<Multiaddr>,
    // ... 其余字段不变
}

// new()（远程，类型未知）：加 node_type: String::new()
pub fn new(peer_id: PeerId, addresses: Vec<Multiaddr>) -> Self {
    Self {
        peer_id,
        name: String::new(),
        node_type: String::new(),
        addresses,
        // ...
    }
}

// new_local()：加参数 node_type
pub fn new_local(peer_id: PeerId, name: String, node_type: String) -> Self {
    Self {
        peer_id,
        name,
        node_type,
        addresses: Vec::new(),
        // ...
    }
}
```

### 步骤 3：`PeerManagement/peer_manager.rs` — new / Default / payload / upsert

```rust
// ① PeerManager::new 加参数，透传 new_local：
pub fn new(local_peer_id: PeerId, name: String, node_type: String) -> Self {
    let mut peers = HashMap::new();
    peers.insert(local_peer_id, PeerInfo::new_local(local_peer_id, name, node_type));
    Self { peers: RwLock::new(peers) }
}

// ② Default impl（L201-204）补默认 node_type（易漏！）：
//    Self::new(PeerId::random(), String::new(), String::new())

// ③ Build_Peer_Info_Payload：签名不变，JSON 加字段：
pub fn Build_Peer_Info_Payload(local: &PeerInfo) -> Vec<u8> {
    serde_json::json!({
        "peer_id": local.peer_id.to_string(),
        "name": local.name,
        "node_type": local.node_type,
    }).to_string().into_bytes()
}

// ④ upsert_peer 加保留逻辑（对称 name，L74-76 旁）：
if peer_info.node_type.is_empty() {
    peer_info.node_type = old.node_type.clone();
}
```

### 步骤 4：`PeerManagement/mod.rs` — create_peer_management 加参数

```rust
pub fn create_peer_management(local_peer_id: PeerId, name: String, node_type: String) -> std::sync::Arc<PeerManager> {
    std::sync::Arc::new(PeerManager::new(local_peer_id, name, node_type))
}
```

### 步骤 5：`bootstrap.rs` — 读 node_type 并传入

```rust
use crate::config::{..., Get_Node_Type, Get_Peer_Name, ...};  // 追加 Get_Node_Type

// Phase 3 内，peer_name 之后：
let peer_name = Get_Peer_Name(&config);
let node_type = Get_Node_Type(&config).as_str().to_string();

// 传参（L134）：
create_peer_management(local_peer_id, peer_name, node_type)
```

### 步骤 6：`Orchestrator/core.rs` — do_flush 第 1 个 name 事件加字段

`do_flush` 里第一个 `peer_info_updated`（L212-221，`peer_name = local.name`）JSON 加一行：

```rust
"node_type": local.node_type,
```

> 第二个（sessions 事件，peer_name 为空）**不加**。

### 步骤 7：`Orchestrator/core/branch_user.rs` — SetName 事件加字段

`UserCommand::SetName` 分支的 `peer_info_updated`（L166-173）JSON 加一行：

```rust
"node_type": local.node_type,
```

### 步骤 8：`Storage/storage_manager.rs` — models 事件加字段（一致性）

`Sync_Models` 的 `peer_info_updated`（L206-214，带 `local.name`）JSON 加一行：

```rust
"node_type": local.node_type,
```

### 步骤 9：`Network/swarm_events.rs` — TOPIC_PEER_INFO 分支解析 + 事件加字段

```rust
super::TOPIC_PEER_INFO => {
    let value = serde_json::from_slice::<serde_json::Value>(&message.data).ok();
    let name = value.as_ref()
        .and_then(|v| v["name"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let node_type = value.as_ref()
        .and_then(|v| v["node_type"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    if !name.is_empty() {
        let mut updated = PeerInfo::new(author, vec![]);
        updated.name = name.clone();
        updated.node_type = node_type.clone();   // 空串由 upsert 保留旧值
        let _ = self.peer_handle.Upsert_Peer(updated).await;
    }
    self.event_bus.Publish(Bus_Event::State {
        payload: serde_json::json!({
            "type": "peer_info_updated",
            "peer_id": author.to_string(),
            "peer_name": name,
            "node_type": node_type,
            "is_local": false,
            "models": [],
            "sessions": [],
        }).to_string(),
    });
}
```

### 步骤 10：`pleiades-terminal/src/lib.rs` — 信号 + BridgeEvent 透传

```rust
// ① BridgeEvent::PeerInfo（L46-49）加字段：
PeerInfo { peer_id: String, name: String, node_type: String },

// ② Godot 信号（L114）加第 3 参：
fn peer_info_updated(peer_id: GString, peer_name: GString, node_type: GString);

// ③ poll() emit（L163-167）：
BridgeEvent::PeerInfo { peer_id, name, node_type } => {
    let peer = GString::from(peer_id.as_str());
    let pname = GString::from(name.as_str());
    let ntype = GString::from(node_type.as_str());
    self.signals().peer_info_updated().emit(&peer, &pname, &ntype);
}

// ④ 事件解析（L337-343）：
"peer_info_updated" => {
    let name = v["peer_name"].as_str().unwrap_or("").to_string();
    let node_type = v["node_type"].as_str().unwrap_or("").to_string();
    out_queue.lock().unwrap().push_back(BridgeEvent::PeerInfo { peer_id, name, node_type });
}
```

### 步骤 11：文档 + Pictor（外部）

- `docs/design_doc/pictor_bridge_sync.md:57`：信号签名 `peer_info_updated(peer_id, peer_name)` → 加 `node_type`。
- Pictor 侧 GDScript 信号处理器同步加一个参数（外部仓库，不在本 workspace）。

---

## 六、验证

- `cargo check -p pleiades-base -p pleiades-terminal`（0 error）
- 实机/双节点：车节点启动 → 地面站日志/Pictor 应拿到 `node_type = "car"`；机节点 → `"uav"`；地面站 → `"ground_station"`
- 手动 flush（`SetName` / flush 命令）后 peer-info 广播仍带 node_type

## 七、讨论记录

- 2026-09-01：D1 定字符串；D2 从「不存 PeerInfo（Capabilities 透传）」改为「存 PeerInfo，String 型（对称 peer_name）」；D3 范围含 terminal/Pictor 信号；D4 不考虑旧版兼容；D5 用 String 而非 NodeType 枚举（保持 PeerManagement 纯净、对称 name、enum→string 转换留在 bootstrap）。
- 2026-09-01：子 agent 梳理确认——`Build_Peer_Info_Payload` 仅 2 处调用、`peer_info_updated` 共 8 处构造（需加 node_type 的 4 处：core do_flush name 事件 / branch_user SetName / storage models / swarm_events PEER_INFO）、`PeerManager::default()` 易漏改、测试 t04/t07 已整体过期。
