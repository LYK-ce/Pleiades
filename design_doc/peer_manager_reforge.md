# PeerManager 重构方案 v2

## 0. PeerManager 核心定义

> **PeerManager = 相关节点资源目录。**
> 存在即在线，存在即相关。不在 map 里的节点，要么不知道，要么不关心。
> mDNS 场景存全部在线节点，WAN 场景存直接协作节点，定义统一，只是"相关"的范围不同。

---

## 1. PeerInfo 数据结构重新分层

```
PeerInfo
│
├── 第 1 层：连接/身份元信息 ── 创建时确定，极少变化
│   ├── peer_id: PeerId
│   ├── addresses: Vec<Multiaddr>
│   ├── local: bool                        ← 标识本地节点，方便 Lua/外部快速判断
│   ├── connected_at: Instant
│   ├── last_active: Instant
│   └── supported_models: Vec<SupportedModel>   ← 独立访问
│
└── 第 2 层：动态性能画像 ── 运行时频繁更新
    └── profile: PeerProfile
        ├── latency_ms: Option<u64>
        ├── bandwidth_mbps: Option<u64>
        ├── memory_mb: Option<u64>           ← free memory，随运行变化
        └── layer_time: HashMap<String, Duration>
```

### 与旧版对比

| 旧字段 | 去留 | 说明 |
|--------|------|------|
| `local` | ✅ 新增 | `bool`，标识本地节点，替代 Status::Local |
| `status` | ❌ 删除 | 整个 PeerStatus 枚举移除，在线=存在于 map，离线=不在 map，本地=PeerInfo.local 判断 |
| `memory_mb` | 移入 PeerProfile | 变为 free memory，动态值 |
| `supported_models` | 类型从 `Vec<String>` 扩展为 `Vec<SupportedModel>` | 包含模型 id（xxhash64）、file_name、layer_bitmap |
| `latency_ms` | 移入 PeerProfile | 从 PeerInfo 顶层下沉 |
| `bandwidth_mbps` | 移入 PeerProfile | 从 PeerInfo 顶层下沉 |
| `compute_score` | ❌ 删除 | 不需要 |
| `has_gpu` | ❌ 删除 | 不需要 |
| `layer_time` | 移入 PeerProfile | 仍为模型单层耗时 |

### 删除的类型

- `PeerStatus` 枚举
- `PeerCapability` 结构体
- `PeerHardware` 结构体（不引入）

---

## 2. SupportedModel 结构

```rust
struct SupportedModel {
    /// 模型唯一标识 — xxhash64(content) → u64
    /// 同一模型内容在所有节点上产生相同 id，跨节点可直接比对
    /// 即使同名文件内容不同（微调/量化），id 也不同，不会误判
    id: u64,

    /// 存储文件名（Storage file_id），人读
    file_name: String,

    /// 256 位层位图，bit N = 1 表示该节点持有第 N 层
    /// 全量持有: [0xFF; 32]
    /// 分片持有 (层 10-19): 位 10~19 为 1，其余为 0
    /// 调度时用位图与运算判断节点是否满足分片需求
    layer_bitmap: [u8; 32],
}
```

### 设计思想

- **id**：xxhash64 全量内容哈希 → 固定 8 字节 → `u64` 直接比较。与 Storage 的 checksum 算法无关，统一使用 xxhash64
- **layer_bitmap**（256 位）：替代 `num_layers`。一个节点可同时表示"我有模型 X 的全部层"（全 1），或"我有模型 X 的切片 10-19"（bit 10~19 = 1）。调度时一步到位

---

## 3. PeerProfile 结构

```rust
struct PeerProfile {
    latency_ms: Option<u64>,                         // None ← 跳过不更新
    bandwidth_mbps: Option<u64>,                     // None ← 跳过不更新
    memory_mb: Option<u64>,                          // None ← 跳过不更新
    layer_time: Option<HashMap<String, Duration>>,   // None ← 跳过不更新
}
```

---

## 4. 最终方法清单

### Trait 方法

| Trait 方法 | 核心方法 | 说明 |
|------------|----------|------|
| `Upsert_Peer(peer_info)` | `upsert_peer` | 添加或覆盖节点，不返回 Result |
| `Remove_Peer(peer_id) -> PeerInfo` | `remove_peer` | 移除节点（保护 `info.local`） |
| `Get_Peer(peer_id) -> Option<PeerInfo>` | `get_peer` | 查询单个节点 |
| `Get_Peers() -> Vec<PeerInfo>` | `get_peers` | 查询全部（排除 `local == true`） |
| `Count() -> usize` | `count` | 节点数量 |
| `Is_Empty() -> bool` | `is_empty` | 是否为空 |
| `Contains_Peer(peer_id) -> bool` | `contains_peer` | 是否存在 |
| `Update_Profile(peer_id, PeerProfile)` | `update_profile` | 更新性能画像，字段为 `None` 跳过 |
| `Update_Supported_Models(peer_id, Vec<SupportedModel>)` | `update_supported_models` | 更新支持的模型列表 |
| `Cleanup_Timeout_Peers(timeout_secs) -> usize` | `cleanup_timeout_peers` | 清理超时节点（保护 `info.local`） |
| `Clear()` | `clear` | 清空（保留本地节点） |

### 删除的方法

| 原方法 | 原因 |
|--------|------|
| `Update_Status` | PeerStatus 枚举已删除 |
| `Update_Heartbeat` | 合并入 `Update_Profile` |
| `Update_Capability` | PeerCapability 拆分，改为 `Update_Profile` |
| `Update_Bandwidth` | 合并入 `Update_Profile` |
| `Get_Idle_Peers` | idle 概念随 Status 删除，改为 `Get_Peers` |
| `Get_Busy_Peers` | Busy 概念随 Status 删除 |
| `Get_All_Peer_Ids` | `Get_Peers().iter().map(\|p\| p.peer_id)` 可替代 |

### 删除的内部方法（PeerInfo）

| 原方法 | 原因 |
|--------|------|
| `query_status()` | PeerStatus 已删除 |
| `query_profile()` | PeerProfile 已存在，直接 `info.profile.latency_ms` 即可 |

### 构造器

`new(peer_id)` — 自动创建本地 PeerInfo (`peer_id`, `addresses = []`, `local = true`) 并插入 map。`local_peer_id` 字段移除。

### 已知风险

- **心跳锁竞争**：`Update_Profile`（原 `update_heartbeat`）高频调用时，全局 `RwLock<HashMap>` 写锁可能成为性能瓶颈。当前沿用现有实现，记录为已知风险。（详见 `design_doc/potential_risk.md`）

---

## 5. 全部讨论项结论

| # | 问题 | 结论 |
|---|------|------|
| 1 | `connected_at` 断连重连不更新 | ✅ 按新定义：离开 map 再 upsert = 新 PeerInfo，`Instant::now()` |
| 2 | `PeerEvent` 完全未使用 | ✅ 删除 |
| 3 | 本地节点需手动注册 | ✅ `new()` 自动创建 |
| 4 | 无 Serialize/Deserialize 支持 | ✅ 暂不需要，未来加一行 derive 即可 |
| 5 | `query_profile()` / `query_status()` | ✅ 删除 |
| 6 | `Upsert_Peer` 永远返回 `Result` | ✅ 去掉 `Result`，改为直接返回 `()` |

---

## 6. Lua 绑定

Rust 内部按两层存储，Lua 绑定层扁平化输出：

```lua
-- Lua 侧仍然扁平访问
p.local           -- 来自 PeerInfo.local
p.memory_mb       -- 来自 profile.memory_mb
p.latency_ms      -- 来自 profile.latency_ms
p.bandwidth_mbps  -- 来自 profile.bandwidth_mbps
p.supported_models -- Vec<SupportedModel>，展平为 Lua table:
                   -- {{id = 123456, file_name = "qwen.gguf", bitmap = {0xFF, ...}}, ...}
```

绑定函数负责展平，Lua 脚本无需感知 Rust 内部结构。

---

## 7. 实施计划

> 注意：仅修改 `Src/PeerManagement/` 目录内的代码，不修改其他模块（即使编译不通过）。

### Phase 1：数据结构（`peer_info.rs`）

1. 删除 `PeerStatus` 枚举
2. 删除 `PeerCapability` 结构体
3. 新增 `SupportedModel` 结构体（`id: u64`, `file_name: String`, `layer_bitmap: [u8; 32]`）
4. 新增 `PeerProfile` 结构体（`latency_ms`, `bandwidth_mbps`, `memory_mb`, `layer_time` 均为 `Option`）
5. 重构 `PeerInfo`：删除 `status`, `latency_ms`, `bandwidth_mbps`, `capability`；新增 `local: bool`, `profile: PeerProfile`, `supported_models: Vec<SupportedModel>`
6. 删除 `PeerInfo::update_status()`, `update_heartbeat()`, `update_capability()`, `update_bandwidth()`, `query_status()`, `query_profile()`
7. 新增 `PeerInfo::update_profile(profile: PeerProfile)` — 逐个字段：`None` 跳过，`Some(v)` 更新
8. 新增 `PeerInfo::update_supported_models(models: Vec<SupportedModel>)`

### Phase 2：核心实现（`peer_manager.rs`）

1. 删除 `local_peer_id` 字段
2. `new(peer_id)` → 自动创建本地 `PeerInfo` 并 upsert 到 map
3. 删除 `update_status()`, `update_heartbeat()`, `update_capability()`, `update_bandwidth()`
4. 新增 `update_profile(peer_id, profile) -> bool`
5. 新增 `update_supported_models(peer_id, models) -> bool`
6. `get_idle_peers()` → `get_peers()`（排除 `local == true`）
7. 删除 `get_busy_peers()`, `get_all_peer_ids()`
8. 所有保护逻辑改为检查 `info.local`

### Phase 3：Trait 层（`capability.rs`）

1. 更新 import（移除 `PeerStatus`, `PeerCapability`，引入 `SupportedModel`, `PeerProfile`）
2. 删除方法：`Get_All_Peer_Ids`, `Update_Status`, `Update_Heartbeat`, `Update_Capability`, `Update_Bandwidth`, `Get_Idle_Peers`, `Get_Busy_Peers`
3. 重命名：`List_Peers` → `Get_Peers`, `Add_Peer` → `Upsert_Peer`（去掉 `Result`，返回 `()`）
4. 新增：`Update_Profile`, `Update_Supported_Models`, `Is_Empty`

### Phase 4：Handle 层（`peer_handle.rs`）

1. 删除 `PeerEvent` 枚举
2. 同步 trait 方法变更

### Phase 5：模块导出（`mod.rs`）

1. 删除 `PeerEvent` 导出
2. 新增 `SupportedModel`, `PeerProfile` 导出
3. `create_peer_management()` — 适配新构造器签名

### Phase 6：清理

1. 删除 `Src/PeerManagement/peer_manager_reforge.md`（旧文档）
2. `Src/PeerManagement/task.md` 标记完成

---

## 8. Phase 9：设计文档

将完整设计写入 `design_doc/peer_manager_design.md`。

