//Presented by KeJi
//Date ： 2026-04-10

//! 入站请求管理器
//!
//! 从 inbound_request_manager.rs 中拆分出的独立组件，负责：
//! 1. **入站请求管理**：为入站请求分配 ID，存储 ResponseChannel，转发给 Control 层
//! 2. **入站回复**：根据 request_id 取出 ResponseChannel，发送回复
//!
//! Network_Service 通过持有 `Inbound_Manager` 实例来使用这些功能。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use libp2p::{
    PeerId,
    request_response::ResponseChannel,
};
use tokio::sync::mpsc;
use tracing::{error, warn};

use super::data_protocol::Network_Data;
use super::node_handle::InboundRequest;

// ============================================================
// Inbound_Manager
// ============================================================

/// 入站请求管理器
///
/// 管理入站请求的分发和回复：
/// - `pending_replies`: request_id → ResponseChannel，用于 Control 层调用 Send_Response 时取出 channel
/// - `inbound_tx`: 入站请求转发通道，发送给 Control 层
pub struct Inbound_Manager {
    /// 入站 ResponseChannel 存储：request_id → ResponseChannel
    pending_replies: HashMap<u64, ResponseChannel<Network_Data>>,
    /// 入站请求发送器（转发给 Control 层）
    inbound_tx: mpsc::Sender<InboundRequest>,
    /// 入站请求 ID 自增计数器
    next_inbound_id: u64,
}

impl Inbound_Manager {
    /// 创建新的 Inbound_Manager
    ///
    /// # 参数
    /// - `inbound_tx`: 入站请求发送通道（发给 Control 层）
    pub fn New(inbound_tx: mpsc::Sender<InboundRequest>) -> Self {
        Self {
            pending_replies: HashMap::new(),
            inbound_tx,
            next_inbound_id: 1,
        }
    }

    /// 注册入站请求并转发给 Control 层
    ///
    /// 为入站请求分配唯一 ID，存储 ResponseChannel，
    /// 通过 inbound_tx 将请求（不含 libp2p 内部类型）转发给 Control 层。
    ///
    /// # 参数
    /// - `peer`: 发送请求的节点
    /// - `request`: 收到的请求数据
    /// - `channel`: libp2p 的 ResponseChannel（用于后续回复）
    ///
    /// # 返回
    /// 分配的 request_id
    pub async fn Register_Inbound(
        &mut self,
        peer: PeerId,
        request: Network_Data,
        channel: ResponseChannel<Network_Data>,
    ) -> u64 {
        let request_id = self.next_inbound_id;
        self.next_inbound_id += 1;
        self.pending_replies.insert(request_id, channel);

        let inbound = InboundRequest {
            request_id,
            peer,
            data_type: request.data_type,
            payload: request.payload,
        };

        if let Err(e) = self.inbound_tx.send(inbound).await {
            error!("转发入站请求失败: {}", e);
            self.pending_replies.remove(&request_id);
        }

        request_id
    }

    /// 发送回复（根据 request_id 取出 ResponseChannel）
    ///
    /// Control 层调用 Send_Response 时，通过此方法取出之前存储的 channel 并发送回复。
    ///
    /// # 参数
    /// - `request_id`: 入站请求 ID
    ///
    /// # 返回
    /// 取出的 ResponseChannel（如果存在）
    pub fn Take_Reply_Channel(&mut self, request_id: u64) -> Option<ResponseChannel<Network_Data>> {
        let channel = self.pending_replies.remove(&request_id);
        if channel.is_none() {
            warn!("未找到入站请求 request_id={}", request_id);
        }
        channel
    }

    /// 清理所有 pending 状态（节点停止时调用）
    pub fn Clear_All(&mut self) {
        self.pending_replies.clear();
    }
}
