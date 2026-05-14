# PeerManager 设计文档

Presented by KeJi
Date ： 2026-05-13

## 1. 模块概述

`PeerManagement` 模块负责管理 Pleiades 分布式推理网络中**相关节点及其资源信息**。它不管理连接生命周期，不维护状态机，不参与调度决策——只做一个纯粹的**节点资源目录**。

### 核心定义

> **PeerManager = 相关节点资源目录。**
> 存在即在线，存在即相关。不在 map 里的节点，要么不知道，要么不关心。
> mDNS 场景存全部在线节点，WAN 场景存直接协作节点，定义统一，只是"相关"的范围不同。

### 模块结构

```
PeerManagement/
├── peer_info.rs        ← 数据结构定义（PeerInfo, SupportedModel, PeerProfile）
├── capability.rs       ← Trait 定义（Peer_Management_Capability + 错误类型）
├── peer_manager.rs     ← 核心实现（Arc<RwLock<HashMap<PeerId, PeerInfo>>>）
├── peer_handle.rs      ← Thin wrapper，impl trait，提供 Clone
└── mod.rs              ← 模块入口 + create_peer_management() 工厂函数
```

### 调用关系

```
Orchestrator / Network / Lua
         │
         ▼
  Box<dyn Peer_Management_Capability>    ← capability.rs (trait)
         │
         ▼
       PeerHandle                        ← peer_handle.rs (thin wrapper, Clone)
         │
         ▼
       PeerManager                       ← peer_manager.rs (Arc<RwLock<HashMap>>)
         │
         ▼
  PeerInfo / SupportedModel / PeerProfile ← peer_info.rs (数据结构)
```

---

## 2. 数据结构

### 2.1 PeerInfo

节点完整信息，由身份元信息和动态性能画像两层组成。

```
PeerInfo
├── 第一层：连接/身份元信息（创建时确定，极少变化）
│   ├── peer_id: PeerId              — libp2p 节点 ID
│   ├── addresses: Vec<Multiaddr>    — 网络地址列表
│   ├── local: bool                  — 是否为本地节点（替代旧 Status::Local）
│   ├── connected_at: Instant        — 连接建立时间
│   ├── last_active: Instant         — 最后活跃时间
│   └── supported_models: Vec<SupportedModel> — 持有的模型列表
│
└── 第二层：动态性能画像（运行时频繁更新）
    └── profile: PeerProfile
        ├── latency_ms: Option<u64>           — 最后 ping 延迟
        ├── bandwidth_mbps: Option<u64>       — 带宽
        ├── memory_mb: Option<u64>            — 空闲内存（动态）
        └── layer_time: HashMap<String, Duration> — 模型单层耗时
```

**字段变迁**（与旧版对比）：

| 旧字段 | 去留 | 说明 |
|--------|------|------|
| `local` | ✅ 新增 | 替代 Status::Local |
| `status: PeerStatus` | ❌ 删除 | 不存在即离线，存在即在线 |
| `latency_ms` | 移入 PeerProfile | 性能指标统一管理 |
| `bandwidth_mbps` | 移入 PeerProfile | 同上 |
| `capability: Option<PeerCapability>` | ❌ 删除 | 拆分为 PeerProfile + SupportedModel |
| `compute_score: f32` | ❌ 删除 | 不需要 |
| `has_gpu: bool` | ❌ 删除 | 不需要 |

**已删除的类型**：`PeerStatus` 枚举、`PeerCapability` 结构体

### 2.2 SupportedModel

描述节点持有的模型。调度时依据此结构决定模型分配。

```rust
struct SupportedModel {
    /// 模型唯一标识 — xxhash64(content) → u64
    /// 同一模型内容在所有节点上产生相同 id，跨节点可直接比对
    id: u64,

    /// 存储文件名（Storage file_id），便于日志和显示
    file_name: String,

    /// 256 位层位图
    /// bit N = 1 表示该节点持有模型第 N 层
    /// 全量持有: [0xFF; 32]（256 位全 1）
    /// 分片持有: 仅对应位为 1
    layer_bitmap: [u8; 32],
}
```

**设计思想**：

- **id（xxhash64）**：对模型文件内容计算 xxhash64 → u64。即使同名文件内容不同（微调版、不同量化版），id 也不同，不会误判为同一模型。与 Storage checksum 算法无关，统一使用 xxhash64。
- **file_name**：人读名称。调度 id 匹配作为"硬匹配"，file_name 作为辅助信息。
- **layer_bitmap（256 位）**：本质上替代了 `num_layers`。一个节点可以同时表示"我有模型 X 的全部层"（全 1）或"我有模型 X 的切片 10-19"（bit 10~19 = 1）。调度时用位图与运算判断节点是否满足分片需求：

```rust
// 找持有第 10-19 层的节点
let mask = ((1u128 << 20) - 1) ^ ((1u128 << 10) - 1);
peers.iter().filter(|p| {
    p.supported_models.iter().any(|m| {
        m.layer_has_range(10, 20) // 内部位图与运算
    })
})
```

### 2.3 PeerProfile

节点动态性能画像，运行时频繁更新。`Update_Profile` 方法接收此结构，字段为 `None` 时跳过不更新，`Some(v)` 时更新为 v。

```rust
struct PeerProfile {
    latency_ms: Option<u64>,                         // None ← 跳过不更新
    bandwidth_mbps: Option<u64>,                     // None ← 跳过不更新
    memory_mb: Option<u64>,                          // None ← 跳过不更新
    layer_time: Option<HashMap<String, Duration>>,   // None ← 跳过不更新
}
```

更新来源：
- 心跳 → `latency_ms`
- 带宽测试 → `bandwidth_mbps`
- 内存查询 → `memory_mb`
- 推理 Profiling → `layer_time`

---

## 3. Trait 定义

### 3.1 Peer_Management_Capability

```rust
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
    async fn Cleanup_Timeout_Peers(&self, timeout_secs: u64) -> Result<usize, Peer_Management_Error>;
    async fn Clear(&self) -> Result<(), Peer_Management_Error>;
}
```

### 3.2 与旧版 trait 变更对照

| 旧方法 | 新方法 | 变更 |
|--------|--------|------|
| `List_Peers` | `Get_Peers` | 重命名，统一 Get 前缀 |
| `Get_Peer` | — | 不变 |
| `Contains_Peer` | — | 不变 |
| `Count` | — | 不变 |
| `Get_All_Peer_Ids` | ❌ 删除 | `Get_Peers().iter().map(\|p\| p.peer_id)` 可替代 |
| `Get_Idle_Peers` | ❌ 删除 | 概念随 PeerStatus 删除 |
| `Get_Busy_Peers` | ❌ 删除 | 概念随 PeerStatus 删除 |
| `Add_Peer` | `Upsert_Peer` | 重命名，去掉 Result |
| `Remove_Peer` | — | 不变 |
| `Update_Status` | ❌ 删除 | PeerStatus 已删除 |
| `Update_Heartbeat` | ❌ 删除 | 合并入 Update_Profile |
| `Update_Capability` | ❌ 删除 | 拆分为 Update_Profile + Update_Supported_Models |
| `Update_Bandwidth` | ❌ 删除 | 合并入 Update_Profile |
| `Cleanup_Timeout_Peers` | — | 不变 |
| `Clear` | — | 不变 |
| — | `Is_Empty` | 新增（PeerManager 已有实现，未暴露） |
| — | `Update_Profile` | 新增，替代 Heartbeat/Capability/Bandwidth |
| — | `Update_Supported_Models` | 新增 |

### 3.3 Peer_Management_Error

```rust
pub enum Peer_Management_Error {
    PeerNotFound(String),
    Timeout,
    Internal(String),
}
```

---

## 4. 核心实现

### 4.1 PeerManager

```rust
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    // local_peer_id 字段已移除，本地节点通过 PeerInfo.local 判断
}
```

**构造器**：
```rust
pub fn new(local_peer_id: PeerId) -> Self {
    let mut peers = HashMap::new();
    peers.insert(local_peer_id, PeerInfo::new_local(local_peer_id));
    Self {
        peers: Arc::new(RwLock::new(peers)),
    }
}
```
构造时自动创建本地 `PeerInfo` 并插入 map。外部不再需要手动注册。

**锁方案**：`tokio::sync::RwLock<HashMap>`。心跳等高频写操作存在全局锁竞争风险（见 §7）。

### 4.2 保护规则

所有删除/清理操作保护 `local == true` 的节点：

| 方法 | 保护规则 |
|------|----------|
| `remove_peer` | 返回 `None` 如果目标 `local == true` |
| `cleanup_timeout_peers` | `retain` 保留 `info.local == true` |
| `clear` | `retain` 保留 `info.local == true` |

### 4.3 PeerHandle

```rust
#[derive(Clone)]
pub struct PeerHandle {
    inner: Arc<PeerManager>,
}
```

Thin wrapper。存在的理由：`PeerHandle: Clone` + `impl Peer_Management_Capability`，允许多个调用方各自持有 `Box<dyn Peer_Management_Capability>` 指向同一 `Arc<PeerManager>`。

---

## 5. 工厂函数

```rust
/// 创建节点管理系统
/// 返回 (Arc<PeerManager>, Box<dyn Peer_Management_Capability>)
/// - Arc<PeerManager>：给 main.rs 直接操作（如后续分片注册）
/// - Box<dyn Peer_Management_Capability>：给 Network
pub fn create_peer_management(local_peer_id: PeerId) -> (Arc<PeerManager>, Box<dyn Peer_Management_Capability>) {
    let manager = Arc::new(PeerManager::new(local_peer_id));
    let handle = PeerHandle::new(manager.clone());
    (manager, Box::new(handle))
}
```

本地 `PeerInfo` 已在 `PeerManager::new()` 内自动创建，调用方无需手动注册。

---

## 6. Lua 绑定设计

Rust 内部按分层存储，Lua 绑定层扁平化输出：

```lua
-- 扁平化后的 Lua 访问
p.peer_id         -- PeerInfo.peer_id
p.local           -- PeerInfo.local
p.memory_mb       -- profile.memory_mb
p.latency_ms      -- profile.latency_ms
p.bandwidth_mbps  -- profile.bandwidth_mbps
p.supported_models -- { {id=..., file_name=..., bitmap={...}}, ... }
```

`caps:get_available_peers()` → 内部调用 `Get_Peers()` → 逐字段展平为 Lua table。

---

## 7. 已知风险

### 7.1 心跳全局写锁竞争

`Update_Profile`（原 `update_heartbeat`）高频调用时，每次需要获取 `RwLock<HashMap>` 全局写锁。多节点场景下，心跳更新可能阻塞读操作（`Get_Peer`, `Get_Peers`, `Count` 等）。

- **影响范围**：Network 心跳 → Orchestrator/Scheduler 查询 + Lua 调用链路
- **当前状态**：沿用现有 `tokio::sync::RwLock<HashMap>` 方案
- **备用方案**：`DashMap` 分片锁（引入新依赖，同步锁与 tokio 上下文不完美匹配）

详见 `design_doc/potential_risk.md`
