# PeerManager 设计文档

Presented by KeJi
Date ： 2026-05-19

## 1. 模块概述

`PeerManager` 是 Pleiades 的**节点资源目录**，负责维护网络中相关节点的身份信息、性能画像和模型持有状态。

### 核心定义

> **PeerManager = 相关节点资源目录。** 存在即在线，存在即相关。
> 不在 map 里的节点，要么不知道，要么不关心。
> mDNS 场景存全部在线节点，WAN 场景存直接协作节点，定义统一。
> 基于 `tokio::sync::RwLock<HashMap<PeerId, PeerInfo>>` 实现并发安全访问。

### 模块结构

```
PeerManagement/
├── mod.rs           ← 模块入口 + re-export + 工厂函数
├── capability.rs    ← Peer_Management_Capability trait + Peer_Management_Error
├── peer_info.rs     ← PeerInfo / SupportedModel / PeerProfile 数据结构
├── peer_manager.rs  ← PeerManager 核心实现（RwLock<HashMap>）
└── peer_handle.rs   ← PeerHandle（trait 实现，Arc<PeerManager> 的 thin wrapper）
```

### 架构总览

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  Network     │  │ Orchestrator │  │     Lua      │
│swarm_events  │  │   control    │  │caps.peer_mgr │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                  │
       ▼                 ▼                  ▼
┌──────────────────────────────────────────────────────┐
│            Box<dyn Peer_Management_Capability>       │
│                      (PeerHandle)                     │
│                          │                            │
│                    Arc<PeerManager>                   │
│                          │                            │
│          Arc<RwLock<HashMap<PeerId, PeerInfo>>>       │
│                          │                            │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │ PeerInfo │  │ PeerInfo │  │ PeerInfo │  ...      │
│  │ local: T │  │ remote   │  │ remote   │           │
│  │ profile  │  │ profile  │  │ profile  │           │
│  │ models   │  │ models   │  │ models   │           │
│  └──────────┘  └──────────┘  └──────────┘           │
└──────────────────────────────────────────────────────┘
```

---

## 2. 数据结构

### 2.1 PeerInfo — 节点完整信息

```rust
pub struct PeerInfo {
    pub peer_id: PeerId,                     // libp2p 节点标识
    pub addresses: Vec<Multiaddr>,           // 地址列表
    pub local: bool,                         // 是否为本地节点
    pub connected_at: Instant,               // 连接建立时间
    pub last_active: Instant,                // 最后活跃时间
    pub profile: PeerProfile,                // 动态性能画像
    pub supported_models: Vec<SupportedModel>, // 持有的模型列表
}
```

| 字段 | 层级 | 更新频率 |
|------|------|---------|
| `peer_id`, `addresses`, `local`, `connected_at` | 元信息层 | 创建时确定，极少变化 |
| `last_active` | 元信息层 | 每次 `Update_Profile` / `Update_Supported_Models` 自动刷新 |
| `profile` | 动态层 | 运行时频繁更新 |
| `supported_models` | 动态层 | 模型加载/卸载时更新 |

### 2.2 SupportedModel — 模型持有描述

```rust
pub struct SupportedModel {
    pub id: u64,              // xxhash64(内容) → 跨节点比对
    pub file_name: String,    // Storage file_id
    pub layer_bitmap: [u8; 32], // 256 位，bit N=1 表示持有第 N 层
}
```

**构造器：**

| 方法 | 用途 |
|------|------|
| `full(id, file_name)` | 全量持有，位图全 1 |
| `shard(id, file_name, start, end)` | 分片持有，`[start, end)` 置 1 |

**查询器：**

| 方法 | 用途 |
|------|------|
| `has_layer_range(start, end)` | 调度时判断节点是否满足分片需求 |
| `layer_count()` | 返回已持有层数 |

### 2.3 PeerProfile — 动态性能画像

```rust
pub struct PeerProfile {
    pub latency_ms: Option<u64>,                          // ping 延迟
    pub bandwidth_mbps: Option<u64>,                      // 带宽
    pub memory_mb: Option<u64>,                           // 空闲内存
    pub layer_time: Option<HashMap<String, Duration>>,    // 模型单层耗时
}
```

所有字段均为 `Option`——`Update_Profile` 时 `None` 跳过不更新，`Some(v)` 覆盖。

### 2.4 Peer_Management_Error — 错误类型

```rust
pub enum Peer_Management_Error {
    PeerNotFound(String),
    Timeout,
    Internal(String),
}
```

---

## 3. 设计原则

### 3.1 存在即在线

不维护 `Connected`/`Disconnected` 状态枚举。节点在 map 里 = 在线，不在 = 离线。本地节点通过 `PeerInfo.local` 标识，受保护不被 `Remove_Peer` / `Clear` / `Cleanup_Timeout_Peers` 误删。

### 3.2 双层结构

元信息层（`peer_id`, `addresses`, `local`）创建时确定，极少变化。动态层（`profile`, `supported_models`）运行时频繁更新。`last_active` 随任何更新自动刷新，用于超时清理。

### 3.3 Arc + RwLock 并发模型

```
Arc<PeerManager>
  └── RwLock<HashMap<PeerId, PeerInfo>>
        ├── 读：Get_Peers / Get_Peer / Count / Is_Empty / Contains_Peer
        └── 写：Upsert_Peer / Remove_Peer / Update_Profile / Update_Supported_Models
```

`PeerHandle` 实现 `Clone`，多个调用方可各自持有 `Box<dyn Peer_Management_Capability>` 指向同一 `Arc<PeerManager>`。

### 3.4 SupportedModel 位图调度

256 位 `layer_bitmap` 替代简单的层数。一个节点可同时持有多个模型的不同分片。调度时用位图与运算一步判断：

```rust
if model.has_layer_range(10, 20) {
    // 该节点持有层 10~19，可以分配推理任务
}
```

---

## 4. 接口

### 4.1 Peer_Management_Capability trait

```rust
#[async_trait]
pub trait Peer_Management_Capability: Send + Sync {
    // 查询
    async fn Get_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error>;
    async fn Get_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;
    async fn Contains_Peer(&self, peer_id: &PeerId) -> Result<bool, Peer_Management_Error>;
    async fn Count(&self) -> Result<usize, Peer_Management_Error>;
    async fn Is_Empty(&self) -> Result<bool, Peer_Management_Error>;

    // 变更
    async fn Upsert_Peer(&self, peer_info: PeerInfo);
    async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;
    async fn Update_Profile(&self, peer_id: &PeerId, profile: PeerProfile) -> Result<(), Peer_Management_Error>;
    async fn Update_Supported_Models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> Result<(), Peer_Management_Error>;

    // 生命周期
    async fn Cleanup_Timeout_Peers(&self, timeout_secs: u64) -> Result<usize, Peer_Management_Error>;
    async fn Clear(&self) -> Result<(), Peer_Management_Error>;
}
```

### 4.2 工厂函数

```rust
pub fn create_peer_management(local_peer_id: PeerId)
    -> (Arc<PeerManager>, Box<dyn Peer_Management_Capability>)
```

返回原始管理器（供高级操作）和 trait object（供常规消费）。本地节点自动创建并注册。

---

## 5. 核心流程

### 5.1 节点发现（mDNS / Ping）

```
Network swarm_events
  ├── PeerDiscovered → Upsert_Peer(PeerInfo::new(peer_id, addresses))
  ├── Ping 成功       → Update_Profile(PeerProfile { latency_ms: Some(rtt) })
  └── Ping 超时       → Remove_Peer(&peer_id)
```

### 5.2 超时清理

```
Cleanup_Timeout_Peers(timeout_secs)
  └── peers.retain(|_, info| info.local || !info.is_timeout(timeout_secs))
```

`is_timeout` 检查 `last_active.elapsed() >= timeout_secs`。本地节点 (`local=true`) 始终保留。

### 5.3 模型信息同步

```
Storage flush() → FileEntry { model_id, layer_bitmap, ... }
  └── Lua / Orchestrator 构造 SupportedModel
        └── Update_Supported_Models(peer_id, models)
```

调度时 `Get_Peers()` → 遍历 `supported_models` → `has_layer_range()` 匹配。

---

## 6. 消费方集成

### 6.1 main.rs — 初始化

```rust
let (peer_manager_arc, peer_capability) = create_peer_management(local_peer_id);
```

`peer_capability` 传入 `Network_Service` 供 swarm 事件处理。

### 6.2 Network — swarm_events

```rust
// Ping 成功 → 更新延迟
self.peer_handle.Update_Profile(&peer_id, PeerProfile {
    latency_ms: Some(rtt_ms),
    ..PeerProfile::default()
}).await;

// Ping 超时 → 移除节点
self.peer_handle.Remove_Peer(&peer_id).await;
```

### 6.3 Orchestrator

通过 trait object 查询节点列表、更新模型信息、调度分片。

### 6.4 Lua 桥接

`caps.peer_mgr` 暴露 `get_peers()` / `get_peer()` 等查询方法，Lua 脚本用于节点发现和调度决策。

---

## 7. 并发安全

### 7.1 锁模型

| 操作 | 锁类型 |
|------|--------|
| `Get_Peers` / `Get_Peer` / `Count` / `Is_Empty` / `Contains_Peer` | `RwLock::read` |
| `Upsert_Peer` / `Remove_Peer` / `Update_Profile` / `Update_Supported_Models` / `Clear` | `RwLock::write` |
| `Cleanup_Timeout_Peers` | `RwLock::write` |

读操作并发无阻塞，写操作互斥。

### 7.2 本地节点保护

```rust
pub async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
    let mut peers = self.peers.write().await;
    if peers.get(peer_id).map_or(false, |i| i.local) {
        return None;
    }
    peers.remove(peer_id)
}
```

`Remove_Peer` / `Clear` / `Cleanup_Timeout_Peers` 均检查 `local` 标志，防止误删本地节点。

---

## 8. 已知限制

### 8.1 心跳锁竞争

`Update_Profile` 高频调用时（每秒一次 per peer），`RwLock::write` 可能成为瓶颈，阻塞所有读操作。当前场景节点数不多（<100），影响可控。未来可考虑 `dashmap` 或分片锁。

### 8.2 无持久化

节点信息仅存在于内存。重启后需重新发现。`supported_models` 依赖 Storage 和 ML Analyze 重建。

### 8.3 无序列化支持

`PeerInfo` 不含 `Serialize`/`Deserialize`。网络传输由 Network 层负责，PeerManager 仅做本地索引。

### 8.4 `Instant` 不可跨平台传输

`connected_at` 和 `last_active` 使用 `std::time::Instant`，不可序列化。仅用于本地超时计算，不跨节点传递。

---

## 9. 重构历史

| 变更 | 说明 |
|------|------|
| 初始实现 | `PeerStatus`(Connected/Disconnected/Local) + `PeerCapability`(has_gpu/cpu_cores/gpu_name) + `PeerEvent` + flat `PeerInfo` |
| PeerManager Reforge | 删除 `PeerStatus`/`PeerCapability`/`PeerEvent`；新增 `SupportedModel`(id + layer_bitmap)、`PeerProfile`；`PeerInfo` 双层化；`Update_Heartbeat` → `Update_Profile`；`Update_Status(Disconnected)` → `Remove_Peer` |

---

## 10. TODO

### 性能优化

- 高频 `Update_Profile` 场景考虑分片锁或 lock-free 结构
- `Get_Peers()` 返回 `Vec<PeerInfo>` 全部 clone，可考虑返回引用或 `Arc`

### 运维

- 添加节点变更事件通知（TUI 实时刷新）
- 添加心跳成功率统计
