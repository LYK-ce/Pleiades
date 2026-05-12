# PeerManager 本地节点纳入方案

## 1. 背景

当前 PeerManager 仅管理**远程**对等节点，本地节点（Coordinator）不包含在内。`local_peer_id` 由 `Network_Service` 单独持有，通过 `get_local_peer_id()` 查询。

Scheduler 目前分为两条数据来源：
- `peer_manager.Get_Idle_Peers()` → 远程 Worker 列表
- `network.get_local_peer_id()` → 协调者身份

**引入 Profile 功能后**，需要将本机 Profile 测得的 `layer_time` 与远程节点的 `layer_time` 统一写入同一存储，以便 Scheduler 后续通过单一接口查询所有节点的性能数据来做层划分。

当前假设 Profile 回调只写远程节点，本机耗时无处存放；Scheduler 需同时从 PeerManager 和另一个渠道获取 `layer_time`，不够统一。

## 2. 目标

将本地节点注册到 PeerManager 中，实现：

1. **统一存储** — 所有节点（含本地）的能力数据存放在 PeerManager 的同一个 HashMap 中
2. **自动保护** — 本地节点不被意外的移除、清理、断连操作删除
3. **Scheduler 统一读取** — 后续 Scheduler 改造时，可从 PeerManager 统一获取所有节点的 `layer_time` 做层划分

## 3. 设计

### 3.1 PeerStatus 新增 `Local` 变体

`Src/PeerManagement/peer_info.rs`

```rust
pub enum PeerStatus {
    Local,        // 新增：本地协调节点
    Connected,    // 已连接，空闲
    Busy,         // 已连接，忙碌（执行推理任务）
    Connecting,   // 连接建立中
    Disconnected, // 已断开连接
}
```

### 3.2 PeerManager 保存 `local_peer_id`

`Src/PeerManagement/peer_manager.rs`

```rust
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    local_peer_id: PeerId,  // 新增：本地节点 ID
}
```

构造方式变更——从 `Network_Service` 获取 `local_peer_id` 后传入：

```rust
impl PeerManager {
    pub fn new(local_peer_id: PeerId) -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            local_peer_id,
        }
    }
}
```

### 3.3 初始化时注册自己

`Src/main.rs`（或 `create_peer_management` 工厂函数）

`local_peer_id` 在 `Network_Service::Init` 中由 `PeerId::from(keypair.public())` 生成。需要提前生成，传入 `PeerManager::new`，然后立即注册自己。

```rust
// main.rs 初始化顺序调整
let local_peer_id = PeerId::from(keypair.public());  // 提前生成

let peer_manager_arc = Arc::new(PeerManager::new(local_peer_id));
// 注册本地节点
let self_info = PeerInfo {
    peer_id: local_peer_id,
    addresses: vec![],
    latency_ms: None,
    bandwidth_mbps: None,
    connected_at: Instant::now(),
    last_active: Instant::now(),
    status: PeerStatus::Local,
    capability: Some(PeerCapability::new()),  // 初始能力为空，Profile 后填充
};
peer_manager_arc.upsert_peer(self_info).await;
```

### 3.4 本地节点自动保护

在以下危险操作中内置 `local_peer_id` 检查，不依赖调用方自行过滤：

| 方法 | 改动 |
|------|------|
| `remove_peer` | `peer_id == self.local_peer_id` 时拒绝，返回 `None` |
| `clear` | `retain` 保留 `local_peer_id`，不清空自身 |
| `cleanup_timeout_peers` | `retain` 中额外判断 `peer_id != self.local_peer_id`，不检查自身超时 |
| `update_status` | `peer_id == self.local_peer_id` 时拒绝变更为 `Disconnected`/`Connecting`，仅允许 `Local`/`Connected`/`Busy` |

```rust
/// 移除节点（自动保护本地节点）
pub async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
    if *peer_id == self.local_peer_id {
        warn!("拒绝移除本地节点");
        return None;
    }
    let mut peers = self.peers.write().await;
    peers.remove(peer_id)
}
```

### 3.5 查询类方法过滤本地节点

`get_idle_peers` 和 `get_busy_peers` 供 Scheduler 选取 Worker 使用，应排除 `Local` 状态的节点：

```rust
/// 获取空闲节点列表（排除 Local，仅含远程 Connected 节点）
pub async fn get_idle_peers(&self) -> Vec<PeerInfo> {
    let peers = self.peers.read().await;
    peers.values()
        .filter(|p| p.status == PeerStatus::Connected)
        .cloned()
        .collect()
}

/// 获取忙碌节点列表（排除 Local，仅含远程 Busy 节点）
pub async fn get_busy_peers(&self) -> Vec<PeerInfo> {
    let peers = self.peers.read().await;
    peers.values()
        .filter(|p| p.status == PeerStatus::Busy)
        .cloned()
        .collect()
}
```

`get_all_peers` / `count` / `get_all_peer_ids` / `is_empty` 保持返回所有节点（含本地），用于 `DisplayPeer` 等展示场景。

### 3.6 Scheduler 适配

`Src/Scheduler/service.rs` 当前逻辑：

```rust
let node_count = workers.len() + 1; // +1 for Coordinator
```

`workers` 来自 `Get_Idle_Peers`，已自动排除本地节点。Coordinator 的 `layer_time` 将通过 `input.coord_layer_time` 传入（后续改造）。**当前调度逻辑不需要结构性改动**。

后续 Scheduler 改造时，可新增类似 `get_all_peer_layer_times(model_id)` 的方法统一读取所有节点的能力。

### 3.7 Network_Service 适配

`Network_Service` 不需要改动 `Remove_Peer` / `Update_Status(Disconnected)` 等危险操作的调用方代码——PeerManager 内部已经自动保护本地节点。

唯一可能的变化：`Stop` 命令中 `Clear()` 调用。当前 `Clear()` 会保留本地节点，Stop 后本地节点仍然存在，程序退出无所谓。如果未来有动态重启网络的需求，`Clear()` 保留本地节点是正确的行为。

### 3.8 PeerCapability trait 不变

`Peer_Management_Capability` trait 的方法签名不变，本地节点的保护由 `PeerManager` 内部完成，对调用方透明。

## 4. 完整修改清单

| 序号 | 文件 | 改动 |
|------|------|------|
| 1 | `Src/PeerManagement/peer_info.rs` | `PeerStatus` 新增 `Local` 变体 |
| 2 | `Src/PeerManagement/peer_manager.rs` | 新增 `local_peer_id` 字段；`new` 接受 `local_peer_id` 参数；`remove_peer` / `clear` / `cleanup_timeout_peers` / `update_status` 四方法内置本地保护；`get_idle_peers` / `get_busy_peers` 按状态字段精确匹配排除 Local |
| 3 | `Src/PeerManagement/mod.rs` | `create_peer_management` 工厂函数接受 `local_peer_id` 参数，构造后注册自己 |
| 4 | `Src/main.rs` | 提前生成 `local_peer_id`，传入工厂函数 |
| 5 | `Src/Scheduler/service.rs` | 无需改动（`Get_Idle_Peers` 已排除本地） |
| 6 | `Src/Network/network_service.rs` | 无需改动（保护内置于 PeerManager） |
| 7 | `Src/Orchestrator/Orchestrator_VM/scheduler_handler.rs` | 无需改动 |
| 8 | `Src/Orchestrator/core/branch_user.rs` | 无需改动（`List_Peers` 返回含本地的列表，自然展示） |
| 9 | 测试代码 | `PeerManager::new()` 调用需传入 `PeerId::random()` 作为测试用的 `local_peer_id` |

## 5. 不影响的部分

| 组件 | 说明 |
|------|------|
| `PeerCapability` 结构体 | 不变 |
| `PeerInfo` 结构体 | 不变 |
| `PeerHandle` | 不变（透传） |
| `Peer_Management_Capability` trait | 方法签名不变 |
| `Network_Service` 事件处理 | `Add_Peer`/`Remove_Peer` 调用方代码不变 |
| `Orchestrator_VM` 指令 | 不变 |
| TUI | 不直接依赖 PeerManager，不变 |
| Profile 设计 | 回调中写入 `layer_time` 的方式不变，本地节点和远程节点统一通过 `update_capability` 写入 |

## 6. 实施步骤

### Step 1 — 数据结构层
1. `peer_info.rs` — `PeerStatus` 新增 `Local`
2. `peer_manager.rs` — 新增 `local_peer_id` + 四种保护 + 查询过滤
3. `peer_manager.rs` — `new()` 签名变更

### Step 2 — 初始化层
4. `mod.rs` — `create_peer_management` 接受 `local_peer_id`，注册本地节点
5. `main.rs` — 提前生成 `local_peer_id`，传入

### Step 3 — 测试适配
6. 所有测试中 `PeerManager::new()` / `create_peer_management()` 调用适配

### Step 4 — 验证
7. `cargo test --lib peer_management` 验证
8. `cargo test` 全量回归
