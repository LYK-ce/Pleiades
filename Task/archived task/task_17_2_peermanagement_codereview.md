Presented by KeJi
Created Date ： 2026-06-15
Modified Date ： 2026-06-15

# Task 17.2: PeerManagement Code Review

> 状态：审查中
> 父任务：Task 17 (Code Review)

---

## 模块概要

`Src/PeerManagement/` — 5 个文件，~690 行。在内存中维护集群节点目录（`HashMap<PeerId, PeerInfo>` + `RwLock`）。

**模组等级：Level 0** — 仅依赖 `libp2p::PeerId`、`tokio::sync::RwLock`，不调用任何其他项目模块。

### 架构

```
PeerManagement/
├── mod.rs           ← 模块入口 + create_peer_management() 工厂
├── capability.rs    ← Peer_Management_Capability trait（16 方法）+ Error 类型
├── peer_info.rs     ← PeerInfo / SupportedModel / PeerProfile / SessionSummary
├── peer_manager.rs  ← PeerManager（HashMap<PeerId, PeerInfo> + RwLock）
└── peer_handle.rs   ← PeerHandle（impl Capability，薄委托层）

调用方:
  Network/swarm_events  → 节点发现/离开 → Upsert/Remove/Cleanup
  Network/mod.rs         → broadcast_local_info → Get_Local_Peer
  Storage/storage_manager → flush 同步模型 → Update_Supported_Models
  Session_Manager        → 会话变更 → Update_Local_Sessions
  VM/capability_binding  → Lua list_model_peers → Get_All_Peers
  Orchestrator           → 测试 Stub → 全部方法
```

---

## 发现的问题

### 1. SupportedModel 4 个死方法

**位置**: `peer_info.rs:53-116`

5 个方法中仅 `layer_range()` 被实际使用，其余 4 个零调用：

| 方法 | 调用次数 | 原因 |
|------|:---:|------|
| `full()` | 0 | `layer_bitmap` 由 ML_Engine 计算，PeerManagement 不做模型分析 |
| `shard()` | 0 | 同上，PeerManagement 不负责切分模型 |
| `has_layer_range()` | 0 | 调度逻辑在 Lua 脚本侧，Rust 侧不做层范围判断 |
| `layer_count()` | 0 | 同上 |
| `layer_range()` | **4** | TUI 显示 + 网络协议，将位图转可读字符串 |

`full()` 和 `shard()` 试图在 PeerManagement 层做 ML_Engine 的职责，从模块边界角度看也不应存在。

**修复**: 删除 `full`、`shard`、`has_layer_range`、`layer_count`，保留 `layer_range`。

---

### 2. PeerProfile.layer_time 零使用

**位置**: `peer_info.rs:149`

```rust
pub layer_time: Option<HashMap<String, Duration>>,
```

- 从未被写入（全代码库无 `layer_time: Some(...)` 构造）
- `get_peers` / `get_all_peers` 主动将其设为 `None` 再返回，即使写入也读不到
- 设计文档规划了「推理 Profiling → layer_time」但从未实现

**修复**: 移除 `layer_time` 字段，同步清理 `update_profile`、`Default`、`get_peers`/`get_all_peers` 中的剥离逻辑。

> 📝 **备忘**：之后实现推理 Profiling 时需要加回来，届时一并启用查询接口。

---

### 3. Upsert_Peer 丢失返回信息

**位置**: `peer_handle.rs:77-79` + `peer_manager.rs:35-47`

```rust
// peer_manager.rs — upsert_peer 返回 bool（true=更新，false=新增）
pub async fn upsert_peer(&self, mut peer_info: PeerInfo) -> bool { ... }

// peer_handle.rs — 但 PeerHandle 吞掉了这个返回值
async fn Upsert_Peer(&self, peer_info: PeerInfo) -> Result<(), Peer_Management_Error> {
    self.inner.upsert_peer(peer_info).await;
    Ok(())
}
```

`PeerManager::upsert_peer` 区分了「新增」和「更新已有」，但经过 trait 层后调用方无法知道是哪种情况。对于 swarm_events 来说，新增/更新可能需要不同处理（如新增时触发 EventBus 通知），但目前被吞掉了。

**影响**：调用方需要通过额外查询才能知道节点是否已存在。

**选项**：
- A: trait 返回 `Result<bool, ...>`，`true` = 更新，`false` = 新增
- B: 拆为 `Add_Peer` + `Update_Peer` 两个方法
- C: 调用方自行先 `Contains_Peer` 再决定（当前变通做法）

---

### 4. sessions 更新绕过 PeerInfo，不走方法

**位置**: `peer_manager.rs:138-145`、`peer_info.rs`

`sessions` 是 PeerInfo 中唯一没有配套更新方法的字段：

```
update_profile()          ✅ PeerInfo 有方法  → PeerManager 调用它
update_supported_models() ✅ PeerInfo 有方法  → PeerManager 调用它
update_sessions()         ❌ 不存在           → PeerManager 直接戳 peer_info.sessions = sessions
```

`PeerManager::update_local_sessions` 直接赋值字段，导致：
- 绕过 `PeerInfo` 封装，风格不一致
- **不刷新 `last_active`**（对比另外两个更新方法都刷新了）
- 方法名叫 `Local` 但 `swarm_events.rs` 对远程节点也在用

**修复**：
1. `PeerInfo` 新增 `update_sessions()`，内部赋值 + 刷新 `last_active`
2. `PeerManager::update_local_sessions` 改为调用 `peer_info.update_sessions()`
3. 重命名去掉 `Local` → `update_sessions`（capability trait、peer_handle、peer_manager 同步改名）

---

### 5. 头注释格式未更新

**位置**: 全部 5 个文件

所有文件仍使用旧的 `//Date ： 2026-05-13` 格式，需更新为 `Created Date` + `Modified Date`。

---

### 6. 缩进不一致

**位置**: `mod.rs:1`、`peer_handle.rs:1`、`peer_manager.rs:1`

部分文件首行 `//Presented` 前有一个前导空格，部分没有。`capability.rs` 和 `peer_info.rs` 格式正确。

```rust
 //Presented by KeJi    ← 前导空格（mod.rs, peer_handle.rs, peer_manager.rs）
//Presented by KeJi     ← 正确（capability.rs, peer_info.rs）
```

---

### 7. mod.rs 核心定义含义不清

**位置**: `mod.rs:9`

```rust
//! PeerManager = 相关节点资源目录。存在即在线，存在即相关。
```

"存在即在线，存在即相关"——两个「存在」指代不同（前者指 PeerInfo 在 HashMap 中，后者指节点在网络中），但读起来像同义反复，新人不明所以。

**修复** → `PeerManager = 集群节点内存目录。HashMap 中有记录即视为在线，超时未活跃则被清理。`

---

### 8. peer_handle.inner() 暴露内部 Arc

**位置**: `peer_handle.rs:33-35`

```rust
/// 获取内部管理器引用（用于高级操作）
pub fn inner(&self) -> &Arc<PeerManager> {
    &self.inner
}
```

标注为「高级操作」，但实际上 `impl Peer_Management_Capability` 已覆盖所有公开 API。暴露内部 `Arc<PeerManager>` 让调用方可以绕过 trait 约束（如持有写锁超时、调用未公开方法等）。当前无调用方使用此方法。

**建议**：检查是否有调用方。若无，删除。

---

### 9. Update_Peer_Name 是多余方法

**位置**: `capability.rs:66`、`peer_manager.rs:96-103`、`swarm_events.rs:204`

唯一调用方 `swarm_events.rs`——节点被发现时 name 为空，等 Info 消息到达后再单独设 name。但 `Upsert_Peer` 本身就是「存在则更新」，当前它保留了 `connected_at`、`last_active`、`profile`、`supported_models`、`local`，唯独没保留 `name`。如果加上 name 保留逻辑（或改为只更新传入的非空字段），调用方直接再调一次 `Upsert_Peer` 即可，不需要单独的方法。

```rust
// swarm_events.rs — 当前两步走
self.peer_handle.Upsert_Peer(peer_info);           // ① 发现（name=""）
self.peer_handle.Update_Peer_Name(&peer, name);    // ② 收到名字后补上

// 修复后可以一步
self.peer_handle.Upsert_Peer(peer_info);           // ① 发现（name=""）
self.peer_handle.Upsert_Peer(updated_peer_info);   // ② 收到名字后再 upsert，Upsert 负责不覆盖已有好数据
```

**修复**: `upsert_peer` 增加 name 保留逻辑，删除 `Update_Peer_Name`（trait、PeerHandle、PeerManager 三处同步清理）。

---

### 10. Get_Peers 与 Get_All_Peers 重复

**位置**: `peer_manager.rs:67+107`、`capability.rs:34+37`

两个方法的唯一区别是是否包含本地节点：

```rust
get_peers()      → filter(|p| !p.local)    // 排除本地
get_all_peers()  → 无过滤                  // 含本地
```

调用量：`Get_All_Peers` 4 处，`Get_Peers` 仅 1 处（`command_handler.rs` Stop 命令）。且那唯一的调用方完全可以改用 `Get_All_Peers` + 自行 filter。

**修复**: 删除 `Get_Peers`（trait、PeerHandle、PeerManager），唯一调用方改为 `Get_All_Peers` + `filter(|p| !p.local)`，或直接调 `Clear()`。

---

### 11. PeerProfile.bandwidth_mbps / memory_mb 零使用

**位置**: `peer_info.rs:99+101`

| 字段 | 写入 | 读取 |
|------|:---:|:---:|
| `latency_ms` | ✅ ping 处理器 | ✅ TUI + Lua |
| `bandwidth_mbps` | ❌ 从未设置 | ❌ 无读取 |
| `memory_mb` | ❌ 从未设置 | 仅 TUI 显示 `None` |

**修复**: 删除 `bandwidth_mbps`、`memory_mb`，同步清理 `Default`、`update_profile` 中的对应分支。

> 📝 **备忘**：之后实现带宽测试/内存采集时加回来。

---

### 12. 5 个 trait 方法零调用

| 方法 | 调用方 | 状况 |
|------|:---:|------|
| `Contains_Peer` | 0 | 死代码 |
| `Count` | 0 | 死代码 |
| `Is_Empty` | 0 | 死代码 |
| `Get_Peer` | 0 | 死代码 |
| `Cleanup_Timeout_Peers` | 0 | 死代码 |

全部是 HashMap 薄封装，零业务逻辑。需要时一分钟即可加回。

**修复**: 从 capability trait → PeerHandle → PeerManager → StubPeerManager 四层同步删除。

> 📝 **备忘**：5 个方法均为 HashMap 标准操作，之后按需加回。

---

### 13. PeerHandle 中间层应移除，PeerManager 直接 impl trait

**位置**: `peer_handle.rs` 全部 + `capability.rs`

当前架构与其他模块不一致：

```
EventBus:  EventBus 直接暴露方法（无 trait，无 handle）
Storage:   StorageManager 直接 impl trait（无 handle）
PeerManagement: PeerManager → PeerHandle → trait（多一层）
```

`PeerHandle` 的存在理由是将 `PeerManager` 的 `Option`/`bool` 返回值转换为 `Result`。应改为 `PeerManager` 内部直接返回 `Result`，删除整个 `peer_handle.rs`，与其他模块统一。

**修复**: 
1. `PeerManager` 方法直接返回 `Result`
2. 删除 `PeerHandle` + `StubPeerManager`
3. `PeerManager` 直接 `impl Peer_Management_Capability`
4. Network/Capabilities `Box<dyn>` → `Arc<dyn>`（PeerHandle 曾持有内部 Arc，删除后 Box 不再需要）

> ✅ **已完成**（2026-06-15）。仅保留 `for PeerManager`（删 `for Arc<PeerManager>`），`Arc<dyn Capability>` 自动 deref 到 `&PeerManager`。

---

## 不予修改

| 事项 | 理由 |
|------|------|
| trait 16 个方法过多 | 查询 8 个 + 变更 8 个，按 CRUD 分类清晰，不算膨胀 |
| StubPeerManager 在 Orchestrator 目录 | 测试 Stub 放此处是为了避免循环依赖，位置合理 |

---

## 人类评审

<!-- 在此区域写下评审意见 -->

