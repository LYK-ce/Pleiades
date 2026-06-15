# PeerManager 设计文档

Presented by KeJi
Created Date ： 2026-05-19
Modified Date ： 2026-06-15

---

## 目录

- [1. 模块概述](#1-模块概述)
- [2. 结构体定义](#2-结构体定义)
  - [PeerInfo](#peerinfo)
  - [SupportedModel](#supportedmodel)
  - [PeerProfile](#peerprofile)
  - [SessionSummary](#sessionsummary)
  - [PeerManager](#peermanager)
- [3. 模块方法](#3-模块方法)
  - [查询](#查询)
  - [变更](#变更)
- [4. 使用示例](#4-使用示例)
- [5. 已知限制](#5-已知限制)

---

## 1. 模块概述

**模组等级：Level 0** — 仅依赖 `libp2p::PeerId`、`tokio::sync::RwLock`，不调用任何其他项目模块。为全系统最底层基础设施之一。

PeerManager = 集群节点内存目录。`RwLock<HashMap<PeerId, PeerInfo>>` 维护网络中节点的身份、性能和模型持有状态。存在即在线——HashMap 中有记录视为活跃，超时未更新则由调用方清理。

---

## 2. 结构体定义

### PeerInfo

节点完整信息。10 个字段分三层：身份元信息、生命周期时间戳、动态能力。

```rust
pub struct PeerInfo {
    // ── 身份 ──
    pub peer_id: PeerId,
    pub name: String,
    pub addresses: Vec<Multiaddr>,
    pub local: bool,

    // ── 生命周期 ──
    pub connected_at: Instant,
    pub last_active: Instant,

    // ── 能力 ──
    pub profile: PeerProfile,
    pub supported_models: Vec<SupportedModel>,
    pub sessions: Vec<SessionSummary>,
}
```

| 字段 | 更新频率 | 说明 |
|------|---------|------|
| `peer_id` | 创建时确定 | libp2p 节点唯一标识 |
| `name` | 用户手动设置 | `set-name` TUI 命令 |
| `addresses` | 发现时更新 | mDNS 获得的地址列表 |
| `local` | 创建时确定 | 本地节点为 true，受保护不被误删 |
| `connected_at` | 创建时确定 | 首次发现的时间点 |
| `last_active` | `update_profile`/`update_supported_models`/`update_sessions` 自动刷新 | 最后一次活动时间 |
| `profile` | 运行时频繁更新 | 延迟等性能数据 |
| `supported_models` | flush 时同步 | 持有的模型及层范围 |
| `sessions` | Session 创建/销毁时同步 | 活跃推理会话（仅本地节点有意义） |

**构造器**：

| 方法 | 用途 |
|------|------|
| `new(peer_id, addresses)` | 创建远程节点，name 为空，local=false |
| `new_local(peer_id, name)` | 创建本地节点，local=true |

**方法**：

| 方法 | 可见性 | 说明 |
|------|--------|------|
| `display_name()` | pub | `"gpu-node-0#12D3"` 或 `"unknown"` |
| `update_profile(profile)` | pub(crate) | 逐字段覆盖，None 跳过，刷新 last_active |
| `update_supported_models(models)` | pub(crate) | 全量替换模型列表，刷新 last_active |
| `update_sessions(sessions)` | pub(crate) | 全量替换会话列表，刷新 last_active |

---

### SupportedModel

节点持有的模型描述，用于分布式调度时判断节点是否能处理特定层的推理请求。

```rust
pub struct SupportedModel {
    pub id: u32,              // 模型唯一标识 = xxhash32(模型内容)
    pub file_name: String,    // Storage file_id
    pub layer_bitmap: [u8; 32], // 256 位，bit N=1 表示持有第 N 层
}
```

**方法**：

| 方法 | 说明 |
|------|------|
| `layer_range()` → String | 位图转可读字符串，如 `"0-31"` 或 `"0-3,5,7-9"` |

`layer_bitmap` 序列化为 64 字符 hex 字符串（如 `"ff00a3..."`），便于 JSON 传输。

---

### PeerProfile

节点动态性能画像。所有字段为 `Option`——`Update_Profile` 时 `None` 跳过，`Some(v)` 覆盖。

```rust
pub struct PeerProfile {
    pub latency_ms: Option<u64>,  // ping 延迟（毫秒）
}
```

仅保留 `latency_ms`，由 Network 层 ping 处理器写入。`bandwidth_mbps`、`memory_mb`、`layer_time` 预留字段暂未实现，已移除。

---

### SessionSummary

推理会话摘要，仅在本地节点的 `PeerInfo.sessions` 中有意义，用于跨节点同步会话信息。

```rust
pub struct SessionSummary {
    pub session_id: u64,
    pub model_id: String,
    pub occupied_slots: usize,
    pub total_slots: usize,
}
```

---

### PeerManager

核心存储组件。`RwLock<HashMap<PeerId, PeerInfo>>` 的薄封装，所有方法直接操作 HashMap。

```rust
pub struct PeerManager {
    peers: RwLock<HashMap<PeerId, PeerInfo>>,
}
```

---

## 3. 模块方法

对外的 `Peer_Management_Capability` trait（10 方法），`PeerHandle` 实现。

### 查询

| 方法 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `Get_All_Peers` | 无 | `Vec<PeerInfo>` | 所有节点（含本地） |
| `Get_Local_Peer` | 无 | `PeerInfo` | 本地节点 |
| `Get_Peer_By_Name` | `name: &str` | `PeerInfo` | 按 `"name#XXXX"` 精确匹配 |

### 变更

| 方法 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `Upsert_Peer` | `peer_info: PeerInfo` | `bool` | 新增或更新。true=更新已有，false=新增节点。保留已有数据：仅覆盖非空字段 |
| `Set_Local_Name` | `name: &str` | `()` | 设置本地节点名称 |
| `Remove_Peer` | `peer_id: &PeerId` | `PeerInfo` | 移除节点，保护本地 |
| `Update_Profile` | `peer_id, profile` | `()` | 更新性能画像 |
| `Update_Supported_Models` | `peer_id, models` | `()` | 更新模型列表 |
| `Update_Sessions` | `peer_id, sessions` | `()` | 更新会话列表 |
| `Clear` | 无 | `()` | 清空所有远程节点 |

---

## 4. 使用示例

```rust
use pleiades::peer_management::{create_peer_management, PeerInfo, SupportedModel, PeerProfile};

// 创建
let (pm, capability) = create_peer_management(local_peer_id, "gpu-node-0".into());

// 发现远程节点
capability.Upsert_Peer(PeerInfo::new(remote_peer_id, addresses)).await?;

// Ping 成功 → 更新延迟
capability.Update_Profile(&remote_peer_id, PeerProfile {
    latency_ms: Some(12),
}).await?;

// 同步模型信息
capability.Update_Supported_Models(&local_peer_id, vec![
    SupportedModel {
        id: model_id,
        file_name: "qwen3.pgguf".into(),
        layer_bitmap: [0xFF; 32],
    },
]).await?;

// 查询
let peers = capability.Get_All_Peers().await?;
for p in &peers {
    println!("{} [latency={:?}ms]", p.display_name(), p.profile.latency_ms);
    for m in &p.supported_models {
        println!("    {} [layers: {}]", m.file_name, m.layer_range());
    }
}
```

---

## 5. 已知限制

| 限制 | 说明 |
|------|------|
| 无持久化 | 节点信息仅存在于内存。重启后需重新发现，模型信息依赖 Storage flush 重建 |
| 写锁瓶颈 | `Update_Profile` 高频调用时 `RwLock::write` 阻塞读操作。当前节点数少（<100），影响可控 |
| 无超时清理 | `Cleanup_Timeout_Peers` 已移除（无调用方）。超时清理需由调用方自行实现 |
