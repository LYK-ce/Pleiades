# task_21_dht — DHT 节点发现（bootstrap + provider 机制）

> 状态：方案已确认（用户 2026-08-18），**尚未开始实施**
> Created Date ： 2026-08-18
> Modified Date ： 2026-08-18

---

## 一、目标与范围

让 Pleiades 节点在 **mDNS 组播不可靠（跨 AP / 大规模）** 时，能通过 **DHT（Kademlia provider 机制）在单播上完成节点发现**，为大规模部署铺路。

### 本次要改的（范围）

1. **bootstrap 入网**：`config.toml` 加种子节点列表 `bootstrap_peers`；列表非空则启动时 bootstrap，为空则不 bootstrap（保持现状）。
2. **监听端口配置**：加 `listen_port` 配置项（默认 `0` = 随机），方便以后固定种子节点端口。
3. **namespace 配置**：加 `dht_namespace` 配置项（默认 `"pleiades-nodes"`），支持多集群隔离。
4. **`start_providing` 自注册**：系统**启动时自动调用**，把本节点注册为 `dht_namespace` 的 provider。
5. **`get_providers` 发现机制**：把方法**完整做好**（Rust 能力 `discover_peers()` + oneshot 结果回传），但**系统内不写任何自动调用它的代码**；何时调用由后续自行决定。
6. **新增 `Src/Network/DHT/` 子模块（多文件）**：集中存放 DHT 相关逻辑（对齐 `Tensor_Stream/` 的多文件惯例），按职责拆分：
   - `mod.rs`：模块声明 + re-export + 命名空间常量与 key 辅助
   - `record.rs`：KV 操作（`put_record` / `get_record`）
   - `provider.rs`：provider 操作（`start_providing` / `get_providers`）+ `ProviderQueryTracker`
   - `event.rs`：Kademlia 事件处理（`handle_event`）
7. **迁移已有 KV 逻辑进 DHT 模块**：`command_handler.rs` 的 PutRecord/GetRecord 分支、`swarm_events.rs` 的 `Handle_Kademlia_Event` 整体收进 `DHT/` 模块。

### 本次不做（范围外）

- 不做周期自动发现循环、不做自动 dial（发现→连上由后续自行实现）。
- 不补 Lua 绑定（`put_record`/`get_record`/`discover_peers` 均暂不注册到 Lua）。
- 不迁移**命令管道层**（`NodeCommand::PutRecord/GetRecord` 变体、`NodeHandle::Put_Record/Get_Record`、`Network_Capability` 的 `put_record/get_record`）——它们是通用命令基础设施，留在原文件。
- 不动现有 mDNS / Gossipsub / PeerManagement 逻辑。

---

## 二、方案总览（机制 vs 策略）

```
机制侧（本次实现）：
  ① bootstrap 入网       —— config 种子节点 → add_address → bootstrap()（启动时，非空才做）
  ② start_providing 自注册 —— 启动时自动调用 DHT::start_providing(dht_namespace)
  ③ get_providers 发现   —— 提供 discover_peers() 方法 + oneshot 回传（不自动触发）
  ④ DHT 逻辑集中         —— KV/provider 操作 + 事件处理收进 DHT/ 模块（多文件）

策略侧（后续自行实现，本次不做）：
  · 何时调用 discover_peers()
  · 拿到 PeerId 列表后如何 dial / 建连（需时再补 dial_by_peer_id）
```

---

## 三、最新文件架构（改动后）

### 3.1 启动流程（bootstrap + 自注册）

```
main.rs: 读 config.Network → listen_port / bootstrap_peers / dht_namespace → NetworkConfig
        ↓
Network_Service::Init()
  · Kademlia::new(peer_id, MemoryStore) + set_mode(Server)   [已存在]
  · 遍历 bootstrap_peers → 从 /p2p/ 后缀解析真实 PeerId      [本次修复 PeerId::random bug]
        → kademlia.add_address(peer_id, addr)
        ↓
Network_Service::Start()
  · listen_on(/ip4/0.0.0.0/tcp/{listen_port})                [listen_port 本次接入 config]
  · if !bootstrap_peers.is_empty() → kademlia.bootstrap()    [触发条件本次修改]
  · DHT::start_providing(&mut kademlia, &config.dht_namespace)   [本次新增，逻辑在 DHT 模块]
```

### 3.2 发现调用链（discover_peers，按需）

```
调用方（未来：TUI 命令 / Lua / Orchestrator）
  → Network_Capability::discover_peers()            [capability.rs，本次新增]
    → NodeHandle::Get_Providers()                   [node_handle.rs，本次新增，oneshot + 超时]
      → NodeCommand::GetProviders { reply }         [命令通道]
        → Network_Service::Handle_Command           [command_handler.rs，本次新增]
          → DHT::get_providers(&mut kademlia, &config.dht_namespace) → QueryId
          → ProviderQueryTracker.register(qid, reply)
        ↓（异步，Kademlia 查询完成后）
          → swarm 事件 → Handle_Kademlia_Event      [swarm_events.rs，改为一行委托]
            → DHT::handle_event(event, &mut provider_queries)
              → QueryResult::GetProviders(Ok) → ProviderQueryTracker.take(&id) → 回传 Vec<PeerId>
```

### 3.3 新增/修改的数据结构

| 位置 | 新增项 |
|---|---|
| `Src/Network/DHT/mod.rs` | `DEFAULT_NODE_NAMESPACE`、`node_namespace_key()`、子模块声明 + re-export |
| `Src/Network/DHT/record.rs` | `put_record()`、`get_record()` |
| `Src/Network/DHT/provider.rs` | `start_providing()`、`get_providers()`、`ProviderQueryTracker` |
| `Src/Network/DHT/event.rs` | `handle_event()` |
| `Network_Service` | 字段 `provider_queries: DHT::ProviderQueryTracker` |
| `NetworkConfig`（network_service.rs） | 新增 `dht_namespace: String`（默认 `"pleiades-nodes"`） |
| `Network_Config`（config.rs） | 新增 `listen_port`、`bootstrap_peers`、`dht_namespace` 三个字段 |
| `NodeCommand` | 新增变体 `GetProviders { reply }` |
| `Network_Capability` | 新增方法 `discover_peers()` |

---

## 四、涉及文件清单

| # | 文件 | 改动 | 类型 |
|---|---|---|---|
| 1 | `Src/Network/DHT/mod.rs` | **新建**：模块声明 + re-export + 命名空间常量/key 辅助 | 新模块 |
| 2 | `Src/Network/DHT/record.rs` | **新建**：KV 操作 `put_record` / `get_record` | 新模块 |
| 3 | `Src/Network/DHT/provider.rs` | **新建**：provider 操作 + `ProviderQueryTracker` | 新模块 |
| 4 | `Src/Network/DHT/event.rs` | **新建**：`handle_event`（Kademlia 事件处理） | 新模块 |
| 5 | `Src/Network/mod.rs` | 加 `#[path = "DHT/mod.rs"] pub mod DHT;` | 挂载 |
| 6 | `Src/Config/config.rs` | `Network_Config` 加 `listen_port`/`bootstrap_peers`/`dht_namespace`；模板加示例 | 配置 |
| 7 | `Src/main.rs` | `net_cfg` 从 config 读 `listen_port`/`bootstrap_peers`/`dht_namespace` | 配置接入 |
| 8 | `Src/Network/network_service.rs` | `NetworkConfig` 加 `dht_namespace`；修 `PeerId::random()`；bootstrap 触发条件；`start_providing` 改委托 DHT；`provider_queries` 字段 | 核心 |
| 9 | `Src/Network/node_handle.rs` | `NodeCommand` 加 `GetProviders`；`NodeHandle` 加 `Get_Providers` | 核心 |
| 10 | `Src/Network/command_handler.rs` | `PutRecord`/`GetRecord` 分支改为委托 DHT 模块；新增 `GetProviders` 分支（委托 DHT） | 核心 |
| 11 | `Src/Network/swarm_events.rs` | `Handle_Kademlia_Event` 改为一行委托 `DHT::handle_event` | 核心 |
| 12 | `Src/Network/capability.rs` | trait + impl 加 `discover_peers()` | 能力 |
| 13 | `Src/Orchestrator/mod.rs` | `StubNetwork` 补 `discover_peers` stub（编译必需） | 编译 |

---

## 五、详细实施步骤

> 按依赖顺序执行。新代码遵循 Pascal case 函数名、lower_snake 变量名规范；修改文件头部更新 `Modified Date`。

### 阶段 0：新建 DHT 子模块（多文件）

#### 步骤 0.1 — 新建 `Src/Network/DHT/mod.rs`（模块根：声明 + re-export + 命名空间）

```rust
//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT（Kademlia）节点发现模块
//!
//! 集中存放 DHT 相关逻辑，按职责拆分：
//! - mod.rs：模块声明 + 命名空间常量与 key 构造
//! - record.rs：KV 记录操作（put_record / get_record）
//! - provider.rs：provider 记录操作 + 查询跟踪
//! - event.rs：Kademlia 事件处理
//!
//! 注意：真正操作 swarm 的代码仍由 Network_Service 持有，本模块只提供
//! 以 `kad::Behaviour` 为参数的封装函数。

mod event;
mod provider;
mod record;

pub use event::handle_event;
pub use provider::{get_providers, start_providing, ProviderQueryTracker};
pub use record::{get_record, put_record};

use libp2p::kad;

/// 默认节点发现命名空间：所有 Pleiades 节点都注册为该 key 的 provider
/// 实际值由 config.toml 的 [Network].dht_namespace 决定，此常量仅作默认值。
pub const DEFAULT_NODE_NAMESPACE: &str = "pleiades-nodes";

/// 构造节点发现命名空间对应的 DHT Key（Key::new 会哈希字节）
pub fn node_namespace_key(namespace: &str) -> kad::Key {
    kad::Key::new(namespace.as_bytes())
}
```

#### 步骤 0.2 — 新建 `Src/Network/DHT/record.rs`（KV 操作）

```rust
//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT KV 记录操作（put_record / get_record）

use libp2p::{
    kad::{self, store::MemoryStore},
    PeerId,
};

/// 写入 DHT KV 记录
pub fn put_record(
    kademlia: &mut kad::Behaviour<MemoryStore>,
    key: &[u8],
    value: Vec<u8>,
    publisher: PeerId,
) {
    let record = kad::Record {
        key: kad::RecordKey::new(key),
        value,
        publisher: Some(publisher),
        expires: None,
    };
    if let Err(e) = kademlia.put_record(record, kad::Quorum::One) {
        tracing::error!("DHT写入失败: {:?}", e);
    } else {
        tracing::info!("DHT写入: {:?}", key);
    }
}

/// 查询 DHT KV 记录（结果经事件异步返回，见 event::handle_event）
pub fn get_record(kademlia: &mut kad::Behaviour<MemoryStore>, key: &[u8]) {
    let record_key = kad::RecordKey::new(key);
    kademlia.get_record(record_key);
    tracing::info!("DHT查询: {:?}", key);
}
```

#### 步骤 0.3 — 新建 `Src/Network/DHT/provider.rs`（provider 操作 + 查询跟踪）

```rust
//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT provider 记录操作（start_providing / get_providers）+ 查询跟踪

use libp2p::{
    kad::{self, store::MemoryStore},
    PeerId,
};
use std::collections::HashMap;
use tokio::sync::oneshot;

use super::node_namespace_key;

/// 自注册为 namespace 的 provider（供 get_providers 发现）
pub fn start_providing(kademlia: &mut kad::Behaviour<MemoryStore>, namespace: &str) {
    match kademlia.start_providing(node_namespace_key(namespace)) {
        Ok(()) => tracing::info!("DHT 自注册成功: {}", namespace),
        Err(e) => tracing::warn!("DHT 自注册失败: {:?}", e),
    }
}

/// 查询 namespace 的 provider 列表，返回 QueryId（结果经事件回传）
pub fn get_providers(kademlia: &mut kad::Behaviour<MemoryStore>, namespace: &str) -> kad::QueryId {
    kademlia.get_providers(node_namespace_key(namespace))
}

/// 进行中的 get_providers 查询跟踪表
///
/// `get_providers` 返回 QueryId，结果经 swarm 事件异步返回，
/// 这里用 QueryId 关联 oneshot 通道，把结果回传给调用方。
pub struct ProviderQueryTracker {
    pending: HashMap<kad::QueryId, oneshot::Sender<Result<Vec<PeerId>, String>>>,
}

impl ProviderQueryTracker {
    pub fn new() -> Self {
        Self { pending: HashMap::new() }
    }

    /// 注册一次查询（发起 get_providers 时调用）
    pub fn register(&mut self, qid: kad::QueryId, tx: oneshot::Sender<Result<Vec<PeerId>, String>>) {
        self.pending.insert(qid, tx);
    }

    /// 取出查询的回传通道（收到 GetProviders 结果时调用）
    pub fn take(&mut self, qid: &kad::QueryId) -> Option<oneshot::Sender<Result<Vec<PeerId>, String>>> {
        self.pending.remove(qid)
    }
}
```

#### 步骤 0.4 — 新建 `Src/Network/DHT/event.rs`（Kademlia 事件处理）

```rust
//Presented by KeJi
//Created Date ： 2026-08-18
//Modified Date ： 2026-08-18

//! DHT Kademlia 事件处理

use libp2p::kad;

use super::provider::ProviderQueryTracker;

/// 处理 Kademlia 事件（含 GetProviders 结果回传）
pub fn handle_event(event: kad::Event, queries: &mut ProviderQueryTracker) {
    match event {
        kad::Event::OutboundQueryProgressed { id, result, .. } => match result {
            kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FoundRecord(peer_record))) => {
                tracing::info!("DHT记录查询成功: {:?}", peer_record.record.key);
            }
            kad::QueryResult::GetRecord(Ok(kad::GetRecordOk::FinishedWithNoAdditionalRecord { .. })) => {
                tracing::debug!("DHT记录查询完成，无更多记录");
            }
            kad::QueryResult::GetRecord(Err(e)) => {
                tracing::warn!("DHT记录查询失败: {:?}", e);
            }
            kad::QueryResult::PutRecord(Ok(_)) => {
                tracing::info!("DHT记录写入成功");
            }
            kad::QueryResult::PutRecord(Err(e)) => {
                tracing::error!("DHT记录写入失败: {:?}", e);
            }
            kad::QueryResult::Bootstrap(Ok(_)) => {
                tracing::info!("Kademlia引导成功");
            }
            kad::QueryResult::Bootstrap(Err(e)) => {
                tracing::warn!("Kademlia引导失败: {:?}", e);
            }
            kad::QueryResult::GetProviders(Ok(ok)) => {
                tracing::info!("DHT 发现 {} 个 provider 节点", ok.providers.len());
                if let Some(tx) = queries.take(&id) {
                    let _ = tx.send(Ok(ok.providers));
                }
            }
            kad::QueryResult::GetProviders(Err(e)) => {
                tracing::warn!("DHT 查询 provider 失败: {:?}", e);
                if let Some(tx) = queries.take(&id) {
                    let _ = tx.send(Err(format!("{:?}", e)));
                }
            }
            _ => {}
        },
        kad::Event::RoutingUpdated { peer, .. } => {
            tracing::debug!("路由表更新: {}", peer);
        }
        _ => {}
    }
}
```

#### 步骤 0.5 — `Src/Network/mod.rs`：挂载模块

`Gossipsub` 声明（约 L49-50）后加：

```rust
#[path = "DHT/mod.rs"]
pub mod DHT;
```

---

### 阶段 1：配置层

#### 步骤 1.1 — `Src/Config/config.rs`：`Network_Config` 加字段

`Network_Config` 结构体（约 L85-94）追加三个字段：

```rust
pub struct Network_Config {
    pub LAN: Option<bool>,
    pub WAN: Option<bool>,
    pub Transport_Protocol: Option<String>,
    pub cleanup_interval: Option<u64>,
    pub timeout_interval: Option<u64>,
    pub heartbeat_interval: Option<u64>,
    pub heartbeat_timeout: Option<u64>,
    pub request_response_timeout: Option<u64>,
    pub listen_port: Option<u16>,               // 新增：p2p 监听端口，0=随机
    pub bootstrap_peers: Option<Vec<String>>,   // 新增：DHT 种子节点列表
    pub dht_namespace: Option<String>,          // 新增：DHT 节点发现命名空间
}
```

#### 步骤 1.2 — `Src/Config/config.rs`：config 模板加示例

`[Network]` 段模板（约 L22-31）追加：

```toml
[Network]
LAN = true
WAN = false
Transport_Protocol = "TCP"
# p2p 监听端口，0 = 随机（种子节点建议固定端口）
listen_port = 0
# DHT 节点发现命名空间（同一集群的所有节点必须一致，区分大小写）
dht_namespace = "pleiades-nodes"
# DHT 种子节点列表（完整 Multiaddr，必须带 /p2p/<PeerId> 后缀）
# 留空 = 不启用 DHT bootstrap（仅靠 mDNS）
bootstrap_peers = []
# 节点管理相关配置
cleanup_interval = 300
# ...
```

#### 步骤 1.3 — `Src/Network/network_service.rs`：`NetworkConfig` 加 `dht_namespace` 字段

`NetworkConfig` 结构体（约 L70-92）加字段：

```rust
pub struct NetworkConfig {
    pub lan_enabled: bool,
    pub wan_enabled: bool,
    pub transport_protocol: String,
    pub listen_port: u16,              // 0 表示随机
    pub dht_namespace: String,         // 新增：DHT 节点发现命名空间
    pub bootstrap_peers: Vec<String>,
    // ...
}
```

`impl Default for NetworkConfig`（约 L94-109）加：`dht_namespace: crate::network::DHT::DEFAULT_NODE_NAMESPACE.to_string(),`

#### 步骤 1.4 — `Src/main.rs`：`net_cfg` 从 config 读值

`net_cfg` 构造（约 L92-103）中，替换硬编码并新增一行：

```rust
// 原来：
// listen_port:        0,
// bootstrap_peers:    Vec::new(),
// 改为：
listen_port:     n.and_then(|n| n.listen_port).unwrap_or(0),
dht_namespace:   n.and_then(|n| n.dht_namespace.clone())
                    .unwrap_or_else(|| crate::network::DHT::DEFAULT_NODE_NAMESPACE.to_string()),
bootstrap_peers: n.and_then(|n| n.bootstrap_peers.clone()).unwrap_or_default(),
```

---

### 阶段 2：bootstrap 入网修复

#### 步骤 2.1 — `Src/Network/network_service.rs`：修 `PeerId::random()`

`Init()` 的引导节点循环（约 L345-354）改为从 `/p2p/` 后缀提取真实 PeerId：

```rust
for addr_str in &node.config.bootstrap_peers {
    if let Ok(addr) = addr_str.parse::<Multiaddr>() {
        // 从 Multiaddr 末尾 /p2p/<PeerId> 提取真实 PeerId（替代 PeerId::random）
        let peer_id = match addr.iter().last() {
            Some(libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
            _ => {
                warn!("bootstrap 地址缺少 /p2p/<PeerId> 后缀，跳过: {}", addr);
                continue;
            }
        };
        info!("添加引导节点: {}", addr);
        node.swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
    }
}
```

#### 步骤 2.2 — `Src/Network/network_service.rs`：bootstrap 触发条件

`Start()` 中（约 L370）触发条件从 `wan_enabled` 改为「种子非空」：

```rust
// 原来：if self.config.wan_enabled { ... }
// 改为：
if !self.config.bootstrap_peers.is_empty() {
    if let Err(e) = self.swarm.behaviour_mut().kademlia.bootstrap() {
        warn!("Kademlia引导失败: {}", e);
    }
}
```

---

### 阶段 3：`start_providing` 自注册

#### 步骤 3.1 — `Src/Network/network_service.rs`：`Start()` 里自注册（委托 DHT 模块）

`Start()` 的 bootstrap 块之后（约 L374 后）、事件循环之前，加：

```rust
// 自注册为 Pleiades 节点 provider（供 get_providers 发现）
crate::network::DHT::start_providing(
    &mut self.swarm.behaviour_mut().kademlia,
    &self.config.dht_namespace,
);
```

> 说明：逻辑封装在 `DHT::start_providing` 内（含成功/失败日志）。

---

### 阶段 4：命令处理迁移（KV + GetProviders 收进 DHT 委托）

#### 步骤 4.1 — `Src/Network/node_handle.rs`：`NodeCommand` 加 `GetProviders` 变体

`NodeCommand` 枚举（约 L66 `GetRecord` 后）加：

```rust
/// DHT 查询 provider 列表（节点发现），结果通过 oneshot 回传
GetProviders { reply: oneshot::Sender<Result<Vec<PeerId>, String>> },
```

> `PutRecord`/`GetRecord` 变体保持不变（命令管道层不迁移）。

#### 步骤 4.2 — `Src/Network/node_handle.rs`：`NodeHandle` 加 `Get_Providers`

`NodeHandle` impl（`Get_Record` 方法约 L168 后）加，仿照 `Send_Data` 的 oneshot+超时模式：

```rust
/// 查询 DHT provider 列表（节点发现），带超时
pub async fn Get_Providers(&self) -> Result<Vec<PeerId>, Box<dyn Error + Send + Sync>> {
    let (tx, rx) = oneshot::channel();
    self.cmd_tx.send(NodeCommand::GetProviders { reply: tx }).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(self.response_timeout),
        rx,
    ).await
        .map_err(|_| format!("DHT get_providers timeout ({}s)", self.response_timeout))?
        .map_err(|_| "DHT reply channel closed")?
        .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })?;
    Ok(result)
}
```

> `Put_Record`/`Get_Record` 方法保持不变。

#### 步骤 4.3 — `Src/Network/network_service.rs`：`Network_Service` 加 `provider_queries` 字段

- `Network_Service` 结构体加字段：

```rust
/// 进行中的 DHT get_providers 查询跟踪（QueryId → 结果回传通道）
provider_queries: crate::network::DHT::ProviderQueryTracker,
```

- `Init()` 中 `Self { ... }` 初始化处（约 L327-343）加：`provider_queries: crate::network::DHT::ProviderQueryTracker::new(),`

#### 步骤 4.4 — `Src/Network/command_handler.rs`：`PutRecord`/`GetRecord` 分支改为委托 DHT 模块

`Handle_Command` 的 `PutRecord` 分支（约 L33-49）与 `GetRecord` 分支（约 L50-54）改为：

```rust
NodeCommand::PutRecord { key, value } => {
    crate::network::DHT::put_record(
        &mut self.swarm.behaviour_mut().kademlia,
        &key,
        value,
        self.local_peer_id,
    );
}
NodeCommand::GetRecord { key } => {
    crate::network::DHT::get_record(
        &mut self.swarm.behaviour_mut().kademlia,
        &key,
    );
}
```

> 迁移后 `command_handler.rs` 不再直接引用 `kad::RecordKey`/`kad::Record`/`kad::Quorum`；若 `kad` import 不再被使用，移除（`gossipsub` 仍用于 GossipsubPublish 分支，保留）。

#### 步骤 4.5 — `Src/Network/command_handler.rs`：新增 `GetProviders` 分支（委托 DHT 模块）

`NodeCommand::GetRecord` 分支后加：

```rust
NodeCommand::GetProviders { reply } => {
    let qid = crate::network::DHT::get_providers(
        &mut self.swarm.behaviour_mut().kademlia,
        &self.config.dht_namespace,
    );
    self.provider_queries.register(qid, reply);
    info!("DHT 查询 provider 列表: query_id={:?}", qid);
}
```

---

### 阶段 5：事件处理迁移

#### 步骤 5.1 — `Src/Network/swarm_events.rs`：`Handle_Kademlia_Event` 改为一行委托

`Handle_Kademlia_Event`（约 L131-162）整体替换为：

```rust
/// 处理Kademlia事件（委托 DHT 模块）
pub(super) async fn Handle_Kademlia_Event(&mut self, event: kad::Event) {
    crate::network::DHT::handle_event(event, &mut self.provider_queries);
}
```

> 迁移后 `swarm_events.rs` 不再内联 GetRecord/PutRecord/Bootstrap 分支；`kad` import 仍需保留（`Handle_Kademlia_Event` 签名用到 `kad::Event`）。
> 若 clippy 报 `unused_async`，可将方法改为同步并在调用处（约 L29）去掉 `.await`。

---

### 阶段 6：能力层 `discover_peers`

#### 步骤 6.1 — `Src/Network/capability.rs`：trait 加 `discover_peers`

`Network_Capability` trait 的 DHT 段（`get_record` 约 L296 后）加：

```rust
/// DHT 节点发现：查询全网 Pleiades 节点的 PeerId 列表（按需调用）
async fn discover_peers(&self) -> Result<Vec<PeerId>, Network_Error>;
```

#### 步骤 6.2 — `Src/Network/capability.rs`：impl 加实现

`Network_Service_Capability` 的 DHT 段（`get_record` 约 L596 后）加：

```rust
async fn discover_peers(&self) -> Result<Vec<PeerId>, Network_Error> {
    self.node_handle
        .Get_Providers()
        .await
        .map_err(|e| Network_Error::Timeout(e.to_string()))
}
```

> `put_record`/`get_record` 的 trait 方法与 impl 保持不变（仍委托 NodeHandle）。

#### 步骤 6.3 — `Src/Orchestrator/mod.rs`：`StubNetwork` 补 stub

`StubNetwork` 的 `get_record` stub（约 L70）后加：

```rust
async fn discover_peers(&self) -> Result<Vec<libp2p::PeerId>, Network_Error> { unimplemented!("stub") }
```

---

### 阶段 7：编译验证

- [ ] `./build.sh check` 无 error
- [ ] `cargo test --lib` 通过（注意既有 112 passed / 1 failed 预存问题）

> 功能验证（后续部署时）：
> - 单机无种子：`bootstrap_peers = []` → 启动不 bootstrap，行为与现状一致
> - 配置种子后：日志出现「添加引导节点」「DHT 自注册成功」「Kademlia引导成功」
> - 调用 `discover_peers()`：返回已注册节点的 PeerId 列表

---

## 六、验收标准

1. `Src/Network/DHT/` 模块（4 文件：mod.rs / record.rs / provider.rs / event.rs）存在且被 `Network/mod.rs` 挂载，DHT 逻辑（KV / provider / 事件处理）集中于此。
2. `config.toml` 含 `listen_port`（默认 0）、`dht_namespace`（默认 `"pleiades-nodes"`）、`bootstrap_peers`（默认空）三项，向后兼容。
3. `bootstrap_peers` 非空时启动执行 bootstrap，空时跳过。
4. `bootstrap_peers` 中的地址带 `/p2p/<PeerId>` 时正确解析（无 `PeerId::random`）。
5. 启动日志出现「DHT 自注册成功」。
6. `discover_peers()` 可被调用并返回 `Vec<PeerId>`（无任何自动调用点）。
7. 已有 KV 命令（`put_record`/`get_record`）行为不变，仅逻辑迁移到 DHT 模块。

---

## 七、已确认决策

| # | 决策项 | 结论 |
|---|---|---|
| 1 | 模块命名 | `DHT/` |
| 2 | config 字段名 | `dht_namespace` |
| 3 | 已有 KV 代码 | **一并迁移**（逻辑收进 `DHT/`，命令/能力管道层不迁） |
| 4 | 模块组织 | `DHT/` 拆为 4 文件（mod.rs / record.rs / provider.rs / event.rs） |

---

## 八、状态

- 开始：2026-08-18（方案确认）
- 实施：已完成（2026-08-18），`./build.sh check` 通过（lib + lib-test 零新增警告）
- 待办：功能验证（部署后实测 bootstrap / start_providing / discover_peers）
- 结束：TBD
