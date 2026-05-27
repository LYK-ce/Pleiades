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
