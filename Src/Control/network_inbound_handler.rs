//Presented by KeJi
//Date ： 2026-04-17

//! 网络入站请求处理器模块
//!
//! 专门处理网络入站请求，包含 Handle_Inbound 函数，导出 NetworkInboundHandler 结构体。
//! 从 network_control_handler.rs 中提取入站请求处理逻辑，进一步解耦代码。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::{info, warn, error, debug};

use super::network_control_command::Deserialize_Command;
use super::ui_message::Ui_Message;
use crate::network::data_protocol::DataType;
use crate::network::node_handle::{InboundRequest, NodeHandle};

// ============================================================
// NetworkInboundHandler 结构体
// ============================================================

/// 网络入站请求处理器
///
/// 封装所有入站请求的处理逻辑，提供统一的 Handle_Inbound 接口。
pub struct NetworkInboundHandler;

impl NetworkInboundHandler {
    /// 处理入站请求（其他节点发来的）
    ///
    /// 根据不同的 DataType 类型，调用相应的处理函数。
    /// 主要处理 Control_Command 命令，也处理文件传输、数据消息等。
    pub async fn Handle_Inbound(
        state: &mut super::control::Node_State,
        next_peer: &mut Option<PeerId>,
        pending_load: &mut Option<(String, usize, usize)>,
        node_handle: &NodeHandle,
        device: &str,
        req: InboundRequest,
        ui_tx: &mpsc::Sender<Ui_Message>,
    ) {
        info!(
            "Control: 收到入站请求 (id={}, peer={}, type={:?}, payload_len={}, state={:?})",
            req.request_id, req.peer, req.data_type, req.payload.len(), state
        );

        match req.data_type {
            // ===== 文件传输：始终接受 =====
            DataType::File => {
                Self::Send_Ui(ui_tx, Ui_Message::Log(format!(
                    "收到文件传输请求 (来自 {}), 自动接受", req.peer
                ))).await;
                if let Err(e) = node_handle
                    .Send_Response(req.request_id, DataType::Command, b"ACCEPT".to_vec())
                    .await
                {
                    error!("发送文件接受回复失败: {}", e);
                }
            }

            // ===== 命令处理 =====
            DataType::Command => {
                match Deserialize_Command(&req.payload) {
                    Ok(cmd) => {
                        // 调用 NetworkCommandHandler 处理控制命令
                        super::network_control_handler::NetworkCommandHandler::Handle_Control_Command(
                            state, next_peer, pending_load, node_handle, device,
                            req.request_id, req.peer, cmd, ui_tx,
                        ).await;
                    }
                    Err(e) => {
                        warn!("命令解析失败: {}", e);
                        if let Err(e) = node_handle
                            .Send_Response(req.request_id, DataType::Command, b"OK".to_vec())
                            .await
                        {
                            error!("发送回复失败: {}", e);
                        }
                    }
                }
            }

            // ===== 数据处理 =====
            DataType::Data => {
                debug!("收到 Data 消息 (来自 {}), 回复 OK", req.peer);
                let _ = node_handle
                    .Send_Response(req.request_id, DataType::Data, b"OK".to_vec())
                    .await;
            }

            // ===== Info 消息 =====
            DataType::Info => {
                debug!("收到 Info 消息 (来自 {}), 暂未处理", req.peer);
                let _ = node_handle
                    .Send_Response(req.request_id, DataType::Info, b"OK".to_vec())
                    .await;
            }

            // ===== 带宽测试消息 =====
            DataType::BandwidthTest => {
                debug!("收到带宽测试消息 (来自 {}), payload长度={}", req.peer, req.payload.len());
                
                // 带宽测试请求的payload包含数据包大小（小端字节序）
                if req.payload.len() >= 8 {
                    // 读取数据包大小
                    let size_bytes = u64::from_le_bytes([
                        req.payload[0], req.payload[1], req.payload[2], req.payload[3],
                        req.payload[4], req.payload[5], req.payload[6], req.payload[7],
                    ]);
                    
                    // 创建指定大小的响应数据包（填充零）
                    let response_payload = vec![0u8; size_bytes as usize];
                    
                    let _ = node_handle
                        .Send_Response(req.request_id, DataType::BandwidthTest, response_payload)
                        .await;
                } else {
                    warn!("带宽测试请求payload长度不足: {}", req.payload.len());
                    let _ = node_handle
                        .Send_Response(req.request_id, DataType::BandwidthTest, vec![0u8; 8])
                        .await;
                }
            }
        }
    }

    /// 发送 UI 消息的辅助函数（async 版本）
    async fn Send_Ui(ui_tx: &mpsc::Sender<Ui_Message>, msg: Ui_Message) {
        let _ = ui_tx.send(msg).await;
    }
}