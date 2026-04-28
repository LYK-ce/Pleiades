# PeerManager Capability 重构方案

**日期**：2026-04-28
**目标**：将 PeerManager 的对外接口统一为 `Peer_Management_Capability` trait，与 Storage/Network/ML_Engine 保持一致的 Capability 模式。

---

## 1. 当前架构

```
PeerManagement/
├── peer_info.rs      ← PeerInfo / PeerStatus / PeerCapability 数据结构
├── peer_manager.rs   ← PeerManager 核心实现（Arc<RwLock<HashMap>>）
├── peer_handle.rs    ← PeerHandle 对外调用接口（Arc<PeerManager> 的 thin wrapper）
└── mod.rs            ← 导出 + create_peer_management() 工厂函数
```

**调用关系**：
```
Network_Service ──→ PeerHandle ──→ PeerManager
    (连接/断开/心跳/带宽)

Control 层 ────→ PeerHandle ──→ PeerManager
    (display_peer/setup_inference)
    ↑ 即将移除，由 Orchestrator 代替

Orchestrator ──→ ??? (尚未集成)
```

**问题**：
- PeerHandle 是具体类型，不是 trait，与其他 Capability（trait + Box\<dyn\>）模式不一致
- Orchestrator 无法通过 Capabilities 访问 PeerManager
- 测试时无法替换为 Stub

---

## 2. 目标架构

```
PeerManagement/
├── peer_info.rs          ← 不变
├── capability.rs         ← 新增：Peer_Management_Capability trait + 错误类型
├── peer_manager.rs       ← 不变（核心实现）
├── peer_handle.rs        ← 重构：impl Peer_Management_Capability for PeerHandle
└── mod.rs                ← 更新导出
```

**调用关系**：
```
Network_Service ──→ Box<dyn Peer_Management_Capability>
    (连接/断开/心跳/带宽)

Orchestrator ───→ Capabilities.peer_manager: Box<dyn Peer_Management_Capability>
    (list_peers/get_peer/contains_peer/update_status)
```

---

## 3. Capability Trait 定义

### 3.1 `capability.rs`

```rust
//Presented by KeJi
//Date ： 2026-04-28

use async_trait::async_trait;
use libp2p::PeerId;
use super::peer_info::{PeerInfo, PeerStatus, PeerCapability};

/// 节点管理错误
#[derive(Debug, thiserror::Error)]
pub enum Peer_Management_Error {
    #[error("Peer not found: {0}")]
    PeerNotFound(String),
    #[error("Operation timeout")]
    Timeout,
    #[error("Internal error: {0}")]
    Internal(String),
}

/// 节点管理能力 trait
///
/// Orchestrator 和 Network 层通过此 trait 操作节点信息。
/// 实现方为 PeerHandle（持有 Arc<PeerManager>）。
#[async_trait]
pub trait Peer_Management_Capability: Send + Sync {
    // ─── 查询操作 ──────────────────────────────

    /// 获取所有节点信息
    async fn List_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error>;

    /// 获取单个节点信息
    async fn Get_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;

    /// 检查节点是否存在
    async fn Contains_Peer(&self, peer_id: &PeerId) -> Result<bool, Peer_Management_Error>;

    /// 获取空闲节点列表（状态为 Connected）
    async fn Get_Idle_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error>;

    /// 获取节点数量
    async fn Count(&self) -> Result<usize, Peer_Management_Error>;

    /// 获取所有节点 ID
    async fn Get_All_Peer_Ids(&self) -> Result<Vec<PeerId>, Peer_Management_Error>;

    // ─── 变更操作 ──────────────────────────────

    /// 添加或更新节点信息
    async fn Add_Peer(&self, peer_info: PeerInfo) -> Result<(), Peer_Management_Error>;

    /// 移除节点
    async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;

    /// 更新节点状态
    async fn Update_Status(&self, peer_id: &PeerId, status: PeerStatus) -> Result<(), Peer_Management_Error>;

    /// 更新节点心跳（包含延迟信息）
    async fn Update_Heartbeat(&self, peer_id: &PeerId, latency_ms: Option<u64>) -> Result<(), Peer_Management_Error>;

    /// 更新节点能力
    async fn Update_Capability(&self, peer_id: &PeerId, capability: Option<PeerCapability>) -> Result<(), Peer_Management_Error>;

    /// 更新节点带宽信息
    async fn Update_Bandwidth(&self, peer_id: &PeerId, bandwidth_mbps: Option<u64>) -> Result<(), Peer_Management_Error>;

    /// 清理超时节点，返回清理数量
    async fn Cleanup_Timeout_Peers(&self, timeout_secs: u64) -> Result<usize, Peer_Management_Error>;

    /// 清空所有节点
    async fn Clear(&self) -> Result<(), Peer_Management_Error>;
}
```

### 3.2 方法选取说明

| 方法 | 来源 | 调用方 |
|------|------|--------|
| `List_Peers` | PeerHandle::list_peers | Orchestrator (DisplayPeer) |
| `Get_Peer` | PeerHandle::get_peer | Orchestrator (验证 peer 存在) |
| `Contains_Peer` | PeerHandle::contains_peer | Orchestrator (DistributeRun 校验) |
| `Get_Idle_Peers` | PeerHandle::get_idle_peers | Orchestrator (自动选择 Worker) |
| `Count` | PeerHandle::count | 通用 |
| `Get_All_Peer_Ids` | PeerHandle::get_all_peer_ids | 通用 |
| `Add_Peer` | PeerHandle::add_peer | Network (连接建立) |
| `Remove_Peer` | PeerHandle::remove_peer | Network (连接断开) |
| `Update_Status` | PeerHandle::update_status | Network (状态变更) |
| `Update_Heartbeat` | PeerHandle::update_heartbeat | Network (Ping) |
| `Update_Capability` | PeerHandle::update_capability | Network (能力发现) |
| `Update_Bandwidth` | PeerHandle::update_bandwidth | Network (带宽测试) |
| `Cleanup_Timeout_Peers` | PeerHandle::cleanup_timeout | Network (定时清理) |
| `Clear` | PeerHandle::clear | Network (shutdown) |

---

## 4. 实施步骤

### 步骤 1：新建 `capability.rs`

- **文件**：`PeerManagement/capability.rs`
- **内容**：`Peer_Management_Error` 错误类型 + `Peer_Management_Capability` trait 定义
- **命名规范**：方法名采用 Pascal_Snake_Case（与 ML_Engine_Capability 一致）

### 步骤 2：PeerHandle impl Peer_Management_Capability

- **文件**：`PeerManagement/peer_handle.rs`
- **改动**：
  1. 为 PeerHandle 添加 `#[async_trait] impl Peer_Management_Capability`
  2. 将现有方法逻辑复制到 trait impl 中，错误类型从 `PeerError` 转换为 `Peer_Management_Error`
  3. 原有的直接方法（`pub async fn list_peers` 等）可保留为便捷方法（委托 trait 方法），也可移除
- **设计决策**：移除原有 `PeerError`，统一使用 `Peer_Management_Error`。`PeerEvent` 保留（事件通知与 Capability 无关）

### 步骤 3：更新 `mod.rs` 导出

- **文件**：`PeerManagement/mod.rs`
- **改动**：
  1. 新增 `pub mod capability;`
  2. 导出 `Peer_Management_Capability` 和 `Peer_Management_Error`
  3. 移除旧的 `PeerError` 导出（用 `Peer_Management_Error` 替代）

### 步骤 4：更新 `lib.rs` 导出

- **文件**：`Src/lib.rs`
- **改动**：导出 `Peer_Management_Capability` 和 `Peer_Management_Error`

### 步骤 5：集成到 Orchestrator Capabilities

- **文件**：`Orchestrator/mod.rs`
- **改动**：
  ```rust
  pub struct Capabilities {
      pub storage: StorageManager,
      pub ml_engine: Box<dyn ML_Engine_Capability>,
      pub network: Box<dyn Network_Capability>,
      pub io_broker: LLM_IO_Broker,
      pub tensor_io_broker: Tensor_IO_Broker,
      pub peer_manager: Box<dyn Peer_Management_Capability>,  // 新增
      // ui: UiCapability 移除
  }
  ```
- **附带清理**：移除 `UiCapability` 和 `set_compute_preference`（由 Core 内部状态替代）

### 步骤 6：更新 Core DisplayPeer 分支

- **文件**：`Orchestrator/core.rs`
- **改动**：
  ```rust
  UserCommand::DisplayPeer { reply } => {
      match self.capabilities.peer_manager.List_Peers().await {
          Ok(peers) => {
              let peer_strs: Vec<String> = peers.iter()
                  .map(|p| format!("{}", p.peer_id))
                  .collect();
              let _ = reply.send(Ok(peer_strs));
          }
          Err(e) => {
              let _ = reply.send(Err(format!("{}", e)));
          }
      }
  }
  ```

### 步骤 7：更新 Network_Service

- **文件**：`Network/network_service.rs`
- **改动**：
  1. `peer_handle: PeerHandle` → `peer_handle: Box<dyn Peer_Management_Capability>`
  2. 所有 `self.peer_handle.add_peer()` 等调用更新为 trait 方法名（`Add_Peer` 等）
  3. 构造函数参数类型同步更新

### 步骤 8：更新所有测试 Stub

- **涉及文件**：`Orchestrator/core.rs` 测试、`executor/handler_*.rs` 测试、`tests/common/mod.rs`
- **改动**：
  1. 新增 `StubPeerManager`（impl `Peer_Management_Capability`，所有方法默认返回空/Ok）
  2. Capabilities 初始化加入 `peer_manager: Box::new(StubPeerManager)`
- **注意**：Network_Service 测试可能需要更新（如果使用了 PeerHandle 直接构造）

### 步骤 9：清理旧代码

- 移除 `peer_handle.rs` 中旧的直接方法（如果步骤 2 选择移除）
- 移除 `PeerError`（统一为 `Peer_Management_Error`）
- 更新 `create_peer_management()` 返回类型
- 清理 Control 层的 PeerHandle 引用（Control 层即将废弃，可选）

---

## 5. 依赖图

```
步骤 1 (capability.rs trait 定义)
    ↓
步骤 2 (PeerHandle impl trait)
    ↓
步骤 3 (mod.rs 导出)  +  步骤 4 (lib.rs 导出)
    ↓
步骤 5 (Orchestrator Capabilities 集成)
    ↓
步骤 6 (Core DisplayPeer) ─── 步骤 7 (Network_Service 适配) ─── 步骤 8 (测试 Stub)
    ↓
步骤 9 (清理旧代码)
```

---

## 6. 与其他 Capability 的对比

| 维度 | StorageCapability | Network_Capability | ML_Engine_Capability | Peer_Management_Capability |
|------|------------------|--------------------|---------------------|---------------------------|
| 文件 | Storage/capability.rs | Network/capability.rs | ML_Engine/capability.rs | PeerManagement/capability.rs |
| trait bound | Send + Sync | Send + Sync | Send + Sync | Send + Sync |
| 错误类型 | StorageError | Network_Error | ML_Engine_Error | Peer_Management_Error |
| 实现方 | StorageManager | Network_Service_Capability | ML_Inference_Service | PeerHandle |
| Capabilities 字段 | `storage: StorageManager` | `network: Box<dyn>` | `ml_engine: Box<dyn>` | `peer_manager: Box<dyn>` |
| 方法命名 | snake_case | snake_case | Pascal_Snake_Case | Pascal_Snake_Case |

---

## 7. 注意事项

1. **PeerHandle 仍为具体实现类** — trait 是 `Peer_Management_Capability`，PeerHandle impl 它。与 `StorageManager impl StorageCapability` 完全对称。
2. **Network_Service 内部可继续持有 PeerHandle** — 但字段类型改为 `Box<dyn Peer_Management_Capability>`，保持灵活性。
3. **`PeerEvent` 不纳入 trait** — 事件通知是 push 模式，与 Capability 的 pull 模式正交。PeerEvent 可保留为独立定义。
4. **`create_peer_management()` 更新** — 返回 `(Arc<PeerManager>, Box<dyn Peer_Management_Capability>)`

