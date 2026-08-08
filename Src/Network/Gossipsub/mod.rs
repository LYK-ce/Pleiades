//Presented by KeJi
//Created Date ： 2026-08-08
//Modified Date ： 2026-08-08

//! GossipSub 业务状态广播模块
//!
//! 负责三个业务 topic 的发布：
//! - `pleiades/peer-info`  — 节点身份（name；未来扩展算力/内存等硬件信息）
//! - `pleiades/models`     — 模型能力（SupportedModel 列表 + 层位图）
//! - `pleiades/sessions`   — 会话状态（SessionSummary 列表）
//!
//! 提供两层 API：
//! - `Build_*_Payload`：纯函数，构造消息 payload（供 Network_Service 事件循环内直接发布）
//! - `publish_*`：高层发布（Orchestrator 层调用，走 Network_Capability 命令通道）
//!
//! 注意：gossipsub 不回流本机，本地 TUI 刷新需显式发布 EventBus 事件。

use crate::event_bus::EventBus;
use crate::peer_management::{Peer_Management_Capability, PeerInfo};

// ===== Topic 常量 =====

/// peer-info topic：节点身份（name；未来扩展算力/内存等硬件信息）
pub const TOPIC_PEER_INFO: &str = "pleiades/peer-info";
/// models topic：模型能力（SupportedModel 列表 + 层位图）
pub const TOPIC_MODELS: &str = "pleiades/models";
/// sessions topic：会话状态（SessionSummary 列表）
pub const TOPIC_SESSIONS: &str = "pleiades/sessions";

// ===== Payload 构造（纯函数，供事件循环内直接发布） =====

/// 构造 peer-info 消息 payload：`{"peer_id": "...", "name": "..."}`
pub fn Build_Peer_Info_Payload(local: &PeerInfo) -> Vec<u8> {
    serde_json::json!({
        "peer_id": local.peer_id.to_string(),
        "name": local.name,
    }).to_string().into_bytes()
}

/// 构造 models 消息 payload：`Vec<SupportedModel>` JSON
pub fn Build_Models_Payload(local: &PeerInfo) -> Vec<u8> {
    serde_json::to_string(&local.supported_models)
        .unwrap_or_else(|_| "[]".to_string())
        .into_bytes()
}

/// 构造 sessions 消息 payload：`Vec<SessionSummary>` JSON
pub fn Build_Sessions_Payload(local: &PeerInfo) -> Vec<u8> {
    serde_json::to_string(&local.sessions)
        .unwrap_or_else(|_| "[]".to_string())
        .into_bytes()
}

// ===== 高层发布函数（Orchestrator 层调用） =====

/// 发布本地节点身份（name）到 peer-info topic，并通知 TUI 刷新
///
/// 调用方需先确保 PeerManager 本地节点信息已更新。
pub async fn publish_peer_info(
    peer_manager: &dyn Peer_Management_Capability,
    network: &dyn crate::network::Network_Capability,
    event_bus: &EventBus,
) {
    use crate::event_bus::Bus_Event;

    let local = match peer_manager.Get_Local_Peer().await {
        Ok(l) => l,
        Err(_) => return,
    };

    let _ = network
        .publish_gossipsub(TOPIC_PEER_INFO, Build_Peer_Info_Payload(&local))
        .await;

    // gossipsub 不回流本机，本地 TUI 刷新需显式发布
    event_bus.Publish(Bus_Event::State {
        payload: serde_json::json!({
            "type": "peer_info_updated",
            "peer_id": local.peer_id.to_string(),
            "peer_name": local.name,
            "is_local": true,
            "models": [],
            "sessions": [],
        }).to_string(),
    });
}

/// 发布本地模型能力（SupportedModel 列表 + 层位图）到 models topic，并通知 TUI 刷新
pub async fn publish_models(
    peer_manager: &dyn Peer_Management_Capability,
    network: &dyn crate::network::Network_Capability,
    event_bus: &EventBus,
) {
    use crate::event_bus::Bus_Event;

    let local = match peer_manager.Get_Local_Peer().await {
        Ok(l) => l,
        Err(_) => return,
    };

    let _ = network
        .publish_gossipsub(TOPIC_MODELS, Build_Models_Payload(&local))
        .await;

    let models_display: Vec<serde_json::Value> = local
        .supported_models
        .iter()
        .map(|m| {
            serde_json::json!({
                "file_name": m.file_name,
                "layer_range": m.layer_range(),
            })
        })
        .collect();

    event_bus.Publish(Bus_Event::State {
        payload: serde_json::json!({
            "type": "peer_info_updated",
            "peer_id": local.peer_id.to_string(),
            "peer_name": "",
            "is_local": true,
            "models": models_display,
            "sessions": [],
        }).to_string(),
    });
}

/// 发布本地会话状态（SessionSummary 列表）到 sessions topic，并通知 TUI 刷新
pub async fn publish_sessions(
    peer_manager: &dyn Peer_Management_Capability,
    network: &dyn crate::network::Network_Capability,
    event_bus: &EventBus,
) {
    use crate::event_bus::Bus_Event;

    let local = match peer_manager.Get_Local_Peer().await {
        Ok(l) => l,
        Err(_) => return,
    };

    let _ = network
        .publish_gossipsub(TOPIC_SESSIONS, Build_Sessions_Payload(&local))
        .await;

    let sessions_display: Vec<serde_json::Value> = local
        .sessions
        .iter()
        .map(|s| {
            serde_json::json!({
                "session_id": s.session_id,
                "model_id": s.model_id,
                "occupied_slots": s.occupied_slots,
                "total_slots": s.total_slots,
            })
        })
        .collect();

    event_bus.Publish(Bus_Event::State {
        payload: serde_json::json!({
            "type": "peer_info_updated",
            "peer_id": local.peer_id.to_string(),
            "peer_name": "",
            "is_local": true,
            "models": [],
            "sessions": sessions_display,
        }).to_string(),
    });
}
