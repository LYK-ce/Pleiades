//Presented by KeJi
//Date ： 2026-04-10

//! 出站响应路由管理器
//!
//! 从 inbound_request_manager.rs 中拆分出的独立组件，负责：
//! **出站 Response 路由**：将 Send_Bytes 的 Response 通过 oneshot 路由回调用方
//!
//! Network_Service 通过持有 `Outbound_Manager` 实例来使用这些功能。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use libp2p::request_response::OutboundRequestId;
use tokio::sync::oneshot;
use tracing::debug;

use super::data_protocol::Network_Data;

// ============================================================
// Outbound_Manager
// ============================================================

/// 出站响应路由管理器
///
/// 管理出站请求的 Response 回传映射：
/// - `pending_responses`: OutboundRequestId → oneshot，用于将 Response 路由回 Send_Bytes 调用方
pub struct Outbound_Manager {
    /// 出站 Response 路由：OutboundRequestId → oneshot Sender
    pending_responses: HashMap<OutboundRequestId, oneshot::Sender<Result<Network_Data, String>>>,
}

impl Outbound_Manager {
    /// 创建新的 Outbound_Manager
    pub fn New() -> Self {
        Self {
            pending_responses: HashMap::new(),
        }
    }

    /// 注册出站请求的 Response 回传通道
    ///
    /// 在 SendData 命令发出后调用，存储 oneshot Sender，
    /// 当 Response 到达时通过 `Route_Response` 回传。
    ///
    /// # 参数
    /// - `outbound_id`: libp2p 分配的出站请求 ID
    /// - `response_tx`: 回传 Response 的 oneshot Sender
    pub fn Register_Outbound(
        &mut self,
        outbound_id: OutboundRequestId,
        response_tx: oneshot::Sender<Result<Network_Data, String>>,
    ) {
        self.pending_responses.insert(outbound_id, response_tx);
    }

    /// 路由出站 Response 回调用方
    ///
    /// 收到 Response 时调用，用 OutboundRequestId 匹配并回传。
    ///
    /// # 参数
    /// - `outbound_id`: 出站请求 ID
    /// - `response`: 收到的响应数据
    pub fn Route_Response(&mut self, outbound_id: OutboundRequestId, response: Network_Data) {
        if let Some(tx) = self.pending_responses.remove(&outbound_id) {
            let _ = tx.send(Ok(response));
        } else {
            debug!("收到未追踪的响应 (request_id={:?})", outbound_id);
        }
    }

    /// 路由出站失败通知
    ///
    /// 出站请求发送失败时调用，通知等待方。
    ///
    /// # 参数
    /// - `outbound_id`: 出站请求 ID
    /// - `error`: 错误信息
    pub fn Route_Failure(&mut self, outbound_id: OutboundRequestId, error: String) {
        if let Some(tx) = self.pending_responses.remove(&outbound_id) {
            let _ = tx.send(Err(error));
        }
    }

    /// 清理所有 pending 状态（节点停止时调用）
    ///
    /// 通知所有等待 Response 的调用方节点已停止。
    pub fn Clear_All(&mut self) {
        for (_, tx) in self.pending_responses.drain() {
            let _ = tx.send(Err("Network_Service stopped".to_string()));
        }
    }
}
