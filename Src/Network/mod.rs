//Presented by KeJi
//Date ： 2026-04-24

//! 网络层模块
//!
//! 本模块负责P2P网络通信，包括：
//! - 节点发现（mDNS/Kademlia）
//! - 连接管理
//! - 统一数据传输（命令/数据/文件通知）
//! - 文件流传输（纯 raw data，元数据通过 Request-Response 协商）
//! - 张量流传输（持久化 stream，用于 pipeline 推理）
//! - 带宽流传输（iperf 风格固定时长推流测速）
//!
//! ## 对 Orchestrator 暴露的接口
//!
//! ### Capability trait（推荐）
//! - `Network_Capability`: 统一 trait，覆盖请求-响应、连接管理、文件流、张量流、DHT
//! - `Network_Service_Capability`: 基于 NodeHandle + stream::Control 的 Capability 实现
//! - `Network_Inbound_Event`: 入站文件流事件，由 Network_Service 转发给 Orchestrator（张量流已改为 rendezvous 匹配，不再转发）
//!
//! ### 底层操作（通过 NodeHandle）
//! - `Send_Data`: 发送数据并等待确认（同步语义）
//! - `Send_Response`: 回复入站请求（通过 request_id）
//!
//! ### 被动接收（通过 inbound_rx）
//! - `InboundRequest`: 其他节点发来的数据请求
//!
//! ## 内部组件
//! - `request_response`: 请求-响应传输子系统（编解码、入站路由、出站路由）
//!
//! 网络层只负责发送和接收字节流，不负责序列化/反序列化。

pub mod capability;
pub mod network_service;
pub mod node_handle;
#[path = "Request_Response/mod.rs"]
pub mod request_response;
#[path = "File_Stream/mod.rs"]
pub mod file_stream;
#[path = "Tensor_Stream/mod.rs"]
pub mod tensor_stream;
#[path = "Bandwidth_Stream/mod.rs"]
pub mod bandwidth_stream;
#[path = "Session_Stream/mod.rs"]
pub mod session_stream;
mod command_handler;
mod swarm_events;

// 重新导出常用类型
pub use capability::{Network_Capability, Network_Error, Network_Inbound_Event, Network_Service_Capability};
pub use network_service::{NetworkConfig, Network_Service};
pub use node_handle::{NodeCommand, NodeHandle, InboundRequest};
pub use request_response::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use file_stream::{
    FILE_STREAM_PROTOCOL, CHUNK_SIZE,
    FILE_HEADER_ACCEPT, FILE_HEADER_REJECT,
    Write_File_Stream_Header, Read_File_Stream_Header,
    Write_File_Stream_Ack, Read_File_Stream_Ack,
    Send_File_Data, Receive_File_Data,
};
pub use tensor_stream::protocol::{
    TENSOR_STREAM_PROTOCOL, TENSOR_EOF_OFFSET,
    Tensor_Buffer,
    Send_Tensor_Frame, Receive_Tensor_Frame, Send_EOF,
    Write_Tensor_Stream_Handshake, Read_Tensor_Stream_Handshake,
};
pub use tensor_stream::rendezvous::RendezvousMap;
pub use bandwidth_stream::{
    BANDWIDTH_STREAM_PROTOCOL, DEFAULT_DURATION_SECS, MAX_DURATION_SECS,
    Send_Bandwidth_Test, Receive_And_Count,
    Write_Bandwidth_Result, Read_Bandwidth_Result,
    Run_Bandwidth_Test,
};
pub use session_stream::protocol::{
    SESSION_STREAM_PROTOCOL,
    Write_Session_Handshake, Read_Session_Handshake,
    write_session_frame, read_session_frame,
};

/// 构建本地节点信息 payload（"name|models_json|sessions_json"），用于 Info 协议交换
pub fn build_local_info_payload(local: &crate::peer_management::PeerInfo) -> String {
    let models_json = serde_json::to_string(&local.supported_models)
        .unwrap_or_else(|_| "[]".to_string());
    let sessions_json = serde_json::to_string(&local.sessions)
        .unwrap_or_else(|_| "[]".to_string());
    format!("{}|{}|{}", local.name, models_json, sessions_json)
}

/// 广播本地节点 Info 到所有远程节点，并通知 TUI 刷新
///
/// 内部流程：
/// 1. 从 PeerManager 获取最新的本地 PeerInfo
/// 2. 用 `build_local_info_payload` 构造三段 payload
/// 3. 向所有远程节点发送 `DataType::Info`
/// 4. 通过 EventBus 发布 `peer_info_updated`（含 models + sessions）
///
/// 调用方需先更新 PeerManager（如 `Update_Local_Sessions` / `Update_Supported_Models`），
/// 再调用本函数广播最新状态。
pub async fn broadcast_local_info(
    peer_manager: &dyn crate::peer_management::Peer_Management_Capability,
    network: &dyn crate::network::Network_Capability,
    event_bus: &crate::event_bus::EventBus,
) {
    use crate::event_bus::Bus_Event;

    let local = match peer_manager.Get_Local_Peer().await {
        Ok(l) => l,
        Err(_) => return,
    };

    let info = build_local_info_payload(&local);

    // 向所有远程节点发送 Info
    if let Ok(peers) = peer_manager.Get_All_Peers().await {
        for peer in &peers {
            if peer.peer_id != local.peer_id {
                let _ = network
                    .send_data(peer.peer_id, request_response::DataType::Info, info.clone().into_bytes())
                    .await;
            }
        }
    }

    // 通知 TUI 刷新
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
            "peer_name": local.name,
            "is_local": true,
            "models": models_display,
            "sessions": sessions_display,
        })
        .to_string(),
    });
}

