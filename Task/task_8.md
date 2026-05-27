# Task 8: TUI 优化

> Presented by KeJi
> Date: 2026-05-27

---

## 目标

优化 TUI 的显示体验和交互逻辑，使其更加直观和实用。

---

## 背景

当前 TUI 基于 ratatui 实现了 5 区域双输入框布局，但随着 Session / API 等新功能的加入，部分面板的显示内容已不匹配实际需求。

### 当前 TUI 布局

```
┌───────────────────────────┬────────────────┐
│ Log (60%)                 │ Network (40%)  │
│ 系统日志 + 事件通知         │ 节点列表         │
├──────────────────────────────┴──────────────────┤
│ Job (3行)                                       │
│ 推理进度 / 文件传输                               │
├─────────────────────────────────────────────────┤
│ Command Output (8行)                             │
│ 命令结果 + chat 流式 token                        │
├─────────────────────────────────────────────────┤
│ Prompt> (3行)                                   │
│ chat 对话输入                                    │
├─────────────────────────────────────────────────┤
│ pleiades> (3行)                                 │
│ 系统命令输入                                     │
└─────────────────────────────────────────────────┘
```

---

## 设计

### 1. Network 面板：显示 peer name 而非 peer_id

**问题**：当前 Network 面板显示截断的 PeerId（如 `12D3Koo...xyz`），对用户毫无意义。

**方案**：
- `Peer_Display` 新增 `name: String` 字段
- EventBus `peer_discovered` / `peer_connected` 事件 payload 增加 `peer_name`
- `Handle_Bus_Event` 解析 `peer_name` 并写入
- `network_panel.rs` 展示 name 而非 `Truncate_Peer_Id(peer_id)`
- 若 name 为空（旧节点），回退显示截断 peer_id

**影响文件**：
- `Src/TUI/app.rs` — `Peer_Display` 加 `name` 字段，`Update_Peer` 加 name 参数
- `Src/TUI/mod.rs` — `handle_state` 解析 `peer_name`
- `Src/TUI/network_panel.rs` — 渲染 name
- `Src/Network/swarm_events.rs` — `peer_connected` / `peer_discovered` 事件 payload 加 `peer_name`

### 2. Network 面板：显示每个 peer 持有的模型及层范围

**问题**：当前 Network 面板只显示节点名称和连接状态，不展示该节点持有哪些模型及其层范围。这些信息在 `PeerInfo.supported_models` 中已有，`dp` 命令也能看到，但 Network 面板不展示。

**方案**：
- `Peer_Display` 新增 `models: Vec<ModelDisplay>` 字段，`ModelDisplay` 包含 `file_name` 和 `layer_range`
- 网络层已有的 `DataType::Info` 同步机制（`"name|models_json"`）已能正确更新 PeerManager；**补全 EventBus 事件 payload**，在 `peer_connected` / `peer_discovered` 事件中带上 `models` JSON，TUI 解析后直接渲染
- **flush 时主动推送**：flush 操作重新扫描磁盘文件后，本地 `supported_models` 可能变化，发 `peer_models_updated` 事件通知 TUI 刷新本地节点的模型列表
- `network_panel.rs` 在每个 peer 下方缩进显示模型信息：
  ```
  ● alice-node    已连接
    qwen3_model.pgguf [0-27]
    llama_model.pgguf  [14-27]
  ○ bob-node      已断开
  ```
- 若 peer 无模型，不显示模型行

**影响文件**：
- `Src/TUI/app.rs` — `Peer_Display` 加 `models` 字段 + `ModelDisplay` 类型
- `Src/TUI/mod.rs` — `handle_state` 解析模型列表
- `Src/TUI/network_panel.rs` — 渲染模型信息
- `Src/Network/` (EventBus 发送端) — 事件 payload 加模型列表；新增 `peer_models_updated` 事件
- `Src/main.rs` — Phase 5.5 flush 后发 `peer_models_updated` 事件

### 3. Session 信息纳入 PeerInfo

**问题**：创建 Session 后，session 信息只在 SessionManager 内部，PeerManager 的本地节点信息中不包含 session 列表。其他节点无法通过 DHT / dp 命令获知本节点有哪些活跃会话。

**方案**：
- `PeerInfo` 新增 `sessions: Vec<SessionSummary>` 字段（`#[serde(skip)]`，仅本地使用，不通过 DHT 同步）
- `SessionSummary` 包含 `session_id`、`model_id`、`occupied_slots`、`total_slots`
- **create_session / destroy_session 时发 EventBus**：`SessionManager` 在创建/销毁 session 后，通过 EventBus 发送 `session_created` / `session_destroyed` 事件，payload 包含 session 摘要信息
- 事件监听方调用 `peer_manager.update_local_sessions()` 更新 `PeerInfo`
- Network 面板（空间允许时）和 `dp` 命令展示本地 session 信息

**事件发送时间点**：
| 触发操作 | EventBus 事件 | 网络推送 |
|---|---|---|
| `flush` | `peer_models_updated` | 向所有已连接节点重发 `DataType::Info` |
| `session create` | `session_created` | 向所有已连接节点重发 `DataType::Info` |
| `session destroy` | `session_destroyed` | 向所有已连接节点重发 `DataType::Info` |
| 对端 Info 到达 | `peer_connected` (补全) | —（被动接收） |

**数据流**：
```
SessionManager → EventBus → PeerManager.update_local_sessions() → PeerInfo.sessions → TUI
                  │
                  └→ Network: 向所有 peer 重发 DataType::Info（name + models + sessions）
```

**影响文件**：
- `Src/PeerManagement/peer_info.rs` — `PeerInfo` 加 `sessions` + `SessionSummary` 类型
- `Src/PeerManagement/manager.rs` — 加 `update_local_sessions()` 方法
- `Src/Session_Manager/manager.rs` — `create_session` / `destroy_session` 发 EventBus
- `Src/main.rs` 或 `Core` — 监听 session 事件 → 更新 PeerManager
- `Src/TUI/network_panel.rs` — 渲染 session 信息（可选，视面板空间）

---

## 实施计划

0. 从当前分支 `reforge` 创建新分支 `task8_advance`，所有改动在此分支上进行 ✅
1. Network 面板：peer_id → peer_name
2. Network 面板：显示 peer 持有的模型及层范围
3. Session 信息纳入 PeerInfo
4. Network 面板宽度调整：30% → 40%（Log 对应变为 60%）

---

## 人类评审

<!-- 在此区域写下评审意见 -->

