# wb_8 — TUI 优化

> 2026-05-27 start
> branch: task8_advance (from reforge)

## 计划

1. Network 面板：peer_id → peer_name
2. Network 面板：显示 peer 持有的模型及层范围
3. Session 信息纳入 PeerInfo + Info 推送
4. Network 面板宽度 30% → 40%

## 进度

- [x] 1. peer_id → peer_name
  - Peer_Display.name, Update_Peer 加 name 参数
  - swarm_events peer_connected 带 peer_name
  - Info 入站发 peer_info_updated
  - network_panel 优先显示 name，空回退 peer_id
  - handle_state 所有 peer 事件解析 peer_name

- [x] 2. 显示 peer 模型
  - ModelDisplay { file_name, layer_range }
  - Peer_Display.models + Update_Peer_Models
  - parse_models_json / parse_one_model
  - network_panel flat_map 灰色子行
  - swarm_events Info 入站带 models_display

- [x] 3. Session 纳入 PeerInfo
  - SessionSummary 类型 + PeerInfo.sessions
  - Peer_Management_Capability::Update_Local_Sessions
  - SessionManager.publish_session_event 带完整 sessions 列表
  - Core B1 Session handler 直接调 Update_Local_Sessions (不绕 EventBus)
  - Session handler 发 Info 到 peer + peer_info_updated 给 TUI

- [x] 3b. Info payload 扩展
  - build_local_info_payload: "name|models_json|sessions_json"
  - swarm_events: splitn(3) 解析，分模型/session 更新
  - TUI: Peer_Display.sessions + parse_sessions_json + 黄色子行

- [x] 4. 宽度 30% → 40%

- [x] 额外: flush 后发 Info + peer_info_updated
- [x] 额外: 启动时发 peer_info_updated 显示本地节点

## 文件

- Src/TUI/app.rs — Peer_Display, ModelDisplay, SessionDisplay
- Src/TUI/mod.rs — handle_state, parse_*, 布局
- Src/TUI/network_panel.rs — 渲染
- Src/Network/swarm_events.rs — Info 入站/发送
- Src/Network/mod.rs — build_local_info_payload
- Src/PeerManagement/ — SessionSummary, update_local_sessions, trait
- Src/Session_Manager/manager.rs — publish_session_event
- Src/Orchestrator/core/branch_user.rs — Session handler + flush handler
- Src/main.rs — Phase 5.5 本地节点发布

---

## Code Review (2026-05-27)

### 总评

Task 8 整体实现质量良好，核心功能（peer_name、模型显示、Session 信息纳入 PeerInfo、Info 三段 payload）均正确实现。以下按严重程度列出发现的问题。

---

### 🔴 Bug / 逻辑缺陷

#### B1. `ConnectionEstablished` 不使用 `build_local_info_payload`，Info payload 为两段格式

**文件**: `Src/Network/swarm_events.rs:49-55`

```rust
// 当前 (ConnectionEstablished):
let payload = format!("{}|{}", local.name, models).into_bytes();

// build_local_info_payload 生成三段格式:
format!("{}|{}|{}", local.name, models_json, sessions_json)
```

**问题**: `ConnectionEstablished` 发送的 Info payload 是 `"name|models"` (两段)，而 `build_local_info_payload` 及 Session handler/flush handler 发送的是 `"name|models|sessions"` (三段)。虽然 `splitn(3, '|')` 解析器能容错 (第三段缺省为 `"[]"`)，但**首次连接时本地节点的 sessions 信息永远不会通过 Info 发送给对端**，导致对端 Network 面板无法显示该节点的 session 信息。

**建议**: `ConnectionEstablished` 改用 `build_local_info_payload(&local)` 统一格式。

---

#### B2. `Has_Active_Session()` 恒返回 `false`

**文件**: `Src/TUI/app.rs:390-392`

```rust
pub fn Has_Active_Session(&self) -> bool {
    false
}
```

**问题**: 这是一个未完成的 stub。任何依赖此方法的逻辑（如 Prompt 输入栏的「无活跃会话」提示）将永远处于禁用状态。虽然当前 `Render_Prompt` 没有调用此方法，但若未来使用会造成误判。

**建议**: 实现为 `!self.peers.iter().any(|p| !p.sessions.is_empty())` 或基于 `active_job_id` 判断，或标注 `// TODO: Task 8 stub`。

---

### 🟠 冗余代码

#### R1. `ConnectionEstablished` 中 `models_json` 死变量

**文件**: `Src/Network/swarm_events.rs:47-60`

```rust
let (local_name, models_json) =
    if let Ok(local) = self.peer_handle.Get_Local_Peer().await {
        let models = serde_json::to_string(&local.supported_models)
            .unwrap_or_else(|_| "[]".to_string());
        // ... Info send ...
        (local.name, models)    // ← models 绑定到 models_json
    } else {
        (String::new(), "[]".to_string())
    };

self.event_bus.Publish(Bus_Event::State {
    payload: serde_json::json!({
        "type": "peer_connected",
        "peer_id": peer_id.to_string(),
        "peer_name": local_name,   // ← 只用了 local_name
    }).to_string(),
});
// models_json 此后从未被使用
```

**问题**: 变量 `models_json` 在元组解构后从未使用。`models` 变量在 `if let` 块内已用于构造 Info payload，但外层的 `models_json` 是死代码。

**建议**: 改为只返回 `local_name`，或直接用 `build_local_info_payload` 重构（同时解决 B1）。

---

#### R2. `peer_connected` 与 `peer_info_updated` 事件重复更新同一节点

**文件**: `Src/TUI/mod.rs:173-187` (handle_state), `Src/Network/swarm_events.rs:59-65`

**数据流**:
```
ConnectionEstablished
  ├→ Info 发送 (name|models)
  └→ EventBus::peer_connected → TUI: Update_Peer(id, name, true)

对端 Info 回复到达
  └→ EventBus::peer_info_updated → TUI: Update_Peer(id, name, true) + models + sessions
```

**问题**: 同一节点在短时间内被 `Update_Peer` 调用两次（`peer_connected` 一次 + `peer_info_updated` 一次）。功能上正确但浪费 TUI 渲染周期。`peer_info_updated` 是 `peer_connected` 的超集，可以只用后者。

**建议**: 要么 (1) `peer_connected` 只发通知日志，TUI 不更新节点列表，等 `peer_info_updated` 一次性设置；要么 (2) 合并为单一事件。当前行为可接受（低开销），但属于逻辑冗余。

---

#### R3. Session handler 与 Flush handler 中重复的 Info 发送逻辑

**文件**: `Src/Orchestrator/core/branch_user.rs`

Session 创建和 Flush 完成后都有几乎相同的代码块：
1. 获取 local peer
2. 获取 sessions → 序列化
3. 构造 `models_json`
4. 构造三段 Info 字符串
5. 遍历 peers 发送 Info
6. 发布 `peer_info_updated` EventBus

**问题**: 两处 ~40 行几乎相同的逻辑。未来如果 Info payload 格式再变，需要改两处。

**建议**: 提取为一个辅助方法，例如 `Core::sync_local_info_to_peers()` 或独立函数 `broadcast_local_info(caps, peer_manager, session_mgr)`。

---

### 🟡 不一致 / 风格问题

#### I1. `peer_disconnected` EventBus payload 不含 `peer_name`

**文件**: `Src/Network/swarm_events.rs:74-80`

```rust
self.event_bus.Publish(Bus_Event::State {
    payload: serde_json::json!({
        "type": "peer_disconnected",
        "peer_id": peer_id.to_string(),
        // ← 没有 "peer_name"
    }).to_string(),
});
```

而 TUI `handle_state` 会尝试读取 `peer_name`：
```rust
let peer_name = v["peer_name"].as_str().unwrap_or("");
```

**问题**: 虽然功能正确（缺省为 `""`），但与其他 peer 事件 (`peer_connected`, `peer_info_updated`) 不一致。

**建议**: 在 `ConnectionClosed` 分支中补发 `peer_name`，从 PeerManager 中获取已记录的名称。

---

#### I2. `SessionSummary` 使用 `usize` 但序列化时转为 `u64`

**文件**: `Src/PeerManagement/peer_info.rs:149-154`

```rust
pub struct SessionSummary {
    pub occupied_slots: usize,
    pub total_slots: usize,
}
```

在 `parse_sessions_json` (mod.rs) 中：
```rust
slots: format!("{}/{}",
    v.get("occupied_slots")?.as_u64()?,  // 期望 u64
    v.get("total_slots")?.as_u64()?),
```

**问题**: Rust 类型为 `usize`，但 JSON 序列化/反序列化走 `u64`。在 64 位平台上等价，但 32 位平台上 `usize` 截断会导致反序列化失败。

**建议**: 统一使用 `u64` 或 `u32`。Session 槽位数不会超过 `u32::MAX`。

---

### 🟢 设计质量

| 方面 | 评价 |
|------|------|
| **数据流清晰度** | ✅ Info payload 三段格式 `name\|models\|sessions` 设计合理，`splitn(3)` 解析器向后兼容旧两段格式 |
| **事件驱动** | ✅ EventBus 事件类型 (`peer_connected`, `peer_info_updated`, `session_created`) 语义清晰 |
| **TUI 渲染** | ✅ `network_panel` 的 `flat_map` 子行渲染模式干净，灰色模型行 + 黄色 session 行区分度好 |
| **回退策略** | ✅ name 为空时回退显示截断 peer_id；models/sessions 为空时不显示子行 |
| **代码命名** | ✅ `Pascal_snake_case` 遵循项目规范；`Update_Peer`, `Update_Peer_Models`, `Update_Peer_Sessions` 命名一致 |

---

### 📋 改动统计

| 文件 | 新增行数 (估) | 风险 |
|------|:---:|:---:|
| `Src/TUI/app.rs` | ~30 | 低 |
| `Src/TUI/mod.rs` | ~60 | 低 |
| `Src/TUI/network_panel.rs` | ~25 | 低 |
| `Src/Network/swarm_events.rs` | ~40 | ⚠️ B1 |
| `Src/Network/mod.rs` | ~8 | 低 |
| `Src/PeerManagement/*` | ~30 | 低 |
| `Src/Session_Manager/manager.rs` | ~20 | 低 |
| `Src/Orchestrator/core/branch_user.rs` | ~60 | ⚠️ R3 |
| `Src/main.rs` | ~15 | 低 |
| **总计** | **~288** | |

---

### 总结

代码实现质量整体良好，核心功能正确。主要关注点：

1. **B1（必须修）**: `ConnectionEstablished` 的 Info payload 缺 sessions 段，改用 `build_local_info_payload`
2. **R3（建议修）**: Session/Flush handler 中重复的 Info 广播逻辑提取为公共函数
3. **R1（顺手修）**: 死变量 `models_json` 在解决 B1 时可一并消除
4. **B2（低优先级）**: `Has_Active_Session()` stub 标注 TODO

其余问题为风格/一致性建议，不阻塞合并。

## 修复记录 (2026-05-27)

### B1 修复 ✅
- `swarm_events.rs` ConnectionEstablished: Info payload 从两段 `"name|models"` → 三段 `build_local_info_payload(&local)`
- 同时消除 R1 死变量 `models_json`

### R3 修复 ✅
- 提取 `broadcast_local_info()` → `Src/Network/mod.rs`
  - 输入: `&dyn Peer_Management_Capability`, `&dyn Network_Capability`, `&EventBus`
  - 流程: Get_Local_Peer → build_local_info_payload → 遍历远程 peer 发 Info → EventBus peer_info_updated
- Session handler: 从 ~50 行 → `broadcast_local_info(...)` 一行调用
- Flush handler: 从 ~40 行 → `broadcast_local_info(...)` 一行调用
- **顺手修复**: Flush handler 原来用 `Update_Supported_Models` 前的旧 `local` 构造 payload（stale data），新函数内部重新 `Get_Local_Peer` 获取最新数据
