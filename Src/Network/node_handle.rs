//Presented by KeJi
//Date ： 2026-04-24

//! 网络节点对外 API 句柄模块
//!
//! 包含 NodeHandle（对外暴露的 API，可 Clone 可 Send）和
//! NodeCommand（外部命令枚举，通过通道发送给 Network_Service 执行）。
//!
//! ## 主要 API
//! - `Send_Data`: 发送数据并等待 Response（同步语义）
//! - `Send_Response`: 回复入站请求（通过 request_id）
//! - `Dial`: 主动连接到指定地址
//! - `Disconnect`: 断开与指定节点的连接
//! - `Put_Record` / `Get_Record`: DHT 操作

use libp2p::{
    Multiaddr, PeerId,
};
use std::error::Error;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use super::request_response::codec::{DataType, Network_Data};

// ===== 入站请求结构 =====

/// 入站请求（由 Network_Service 转发给 Orchestrator）
///
/// 不包含 libp2p 内部类型（ResponseChannel），
/// Orchestrator 通过 `request_id` 引用并回复。
#[derive(Debug)]
pub struct InboundRequest {
    /// Network_Service 内部分配的请求编号
    pub request_id: u64,
    /// 发送方节点 ID
    pub peer: PeerId,
    /// 数据类型标记
    pub data_type: DataType,
    /// 原始载荷字节流
    pub payload: Vec<u8>,
}

// ===== 节点命令枚举 =====

/// 节点命令（外部通过NodeHandle发送给Network_Service执行）
#[derive(Debug)]
pub enum NodeCommand {
    /// 统一数据发送（可选等待 Response）
    SendData {
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
        /// 可选：用于回传 Response 的 oneshot 发送端
        /// None = fire-and-forget，Some = 等待 Response
        response_tx: Option<oneshot::Sender<Result<Network_Data, String>>>,
    },
    /// 回复入站请求（通过 request_id）
    SendResponse {
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    },
    /// DHT写入
    PutRecord { key: Vec<u8>, value: Vec<u8> },
    /// DHT读取
    GetRecord { key: Vec<u8> },
    /// GossipSub 发布（广播业务状态到 topic）
    GossipsubPublish { topic: String, payload: Vec<u8> },
    /// 主动连接
    Dial { addr: Multiaddr },
    /// 断开连接
    Disconnect { peer: PeerId },
    /// 广播到所有已知节点（fire-and-forget，不等待 Response，Task 9_1）
    Broadcast {
        data_type: DataType,
        payload: Vec<u8>,
    },
    /// 停止节点
    Stop,
}

/// 节点句柄（对外API，可Clone可Send）
#[derive(Clone)]
pub struct NodeHandle {

    /// 命令发送器
    cmd_tx: mpsc::Sender<NodeCommand>,
    /// 本地节点ID
    local_peer_id: PeerId,
    /// Request-Response 超时（秒）
    response_timeout: u64,
}

impl NodeHandle {
    /// 创建新的 NodeHandle（仅供 Network_Service::Init 内部使用）
    pub(crate) fn New(cmd_tx: mpsc::Sender<NodeCommand>, local_peer_id: PeerId, response_timeout: u64) -> Self {
        Self { cmd_tx, local_peer_id, response_timeout }
    }

    /// 获取本地节点ID
    pub fn Get_Local_Peer_Id(&self) -> PeerId {
        self.local_peer_id
    }

    // ===== 核心 API =====

    /// 发送数据并等待 Response（同步语义）
    ///
    /// 内部通过 oneshot channel 等待对方回复，带 30 秒超时。
    ///
    /// # Arguments
    /// * `peer` - 目标节点 ID
    /// * `data_type` - 数据类型标记（Command / Data / File）
    /// * `payload` - 已序列化的字节流（由上层负责序列化）
    ///
    /// # Returns
    /// 对方的 Network_Data
    pub async fn Send_Data(
        &self,
        peer: &PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<Network_Data, Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::SendData {
            peer: *peer,
            data_type,
            payload,
            response_tx: Some(tx),
        }).await?;

        // 等待 Response，使用配置的超时
        let result = tokio::time::timeout(
            Duration::from_secs(self.response_timeout),
            rx,
        ).await
            .map_err(|_| format!("Response timeout ({}s)", self.response_timeout))?
            .map_err(|_| "Response channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })?;

        Ok(result)
    }

    /// 广播到所有已知节点（fire-and-forget，立即返回，不等待 Response）
    ///
    /// 由 Network_Service 事件循环遍历 peer 列表逐个发送（Task 9_1）。
    pub fn Broadcast(&self, data_type: DataType, payload: Vec<u8>) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx
            .try_send(NodeCommand::Broadcast { data_type, payload })
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })?;
        Ok(())
    }

    /// 回复入站请求（通过 request_id）
    ///
    /// Orchestrator 收到 InboundRequest 后，通过此方法回复。
    /// Network_Service 内部会用 request_id 找到对应的 ResponseChannel 发送。
    ///
    /// # Arguments
    /// * `request_id` - 入站请求的编号（来自 InboundRequest.request_id）
    /// * `data_type` - 响应数据类型
    /// * `payload` - 响应载荷
    pub async fn Send_Response(
        &self,
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::SendResponse {
            request_id,
            data_type,
            payload,
        }).await?;
        Ok(())
    }

    /// DHT写入
    pub async fn Put_Record(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::PutRecord { key, value }).await?;
        Ok(())
    }

    /// DHT读取
    pub async fn Get_Record(&self, key: Vec<u8>) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::GetRecord { key }).await?;
        Ok(())
    }

    /// 主动连接到指定地址
    pub async fn Dial(&self, addr: Multiaddr) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::Dial { addr }).await?;
        Ok(())
    }

    /// 断开与指定节点的连接
    pub async fn Disconnect(&self, peer: &PeerId) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::Disconnect { peer: *peer }).await?;
        Ok(())
    }

    /// 发布消息到 GossipSub topic（广播业务状态）
    pub async fn Gossipsub_Publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::GossipsubPublish {
            topic: topic.to_string(),
            payload,
        }).await?;
        Ok(())
    }

    /// 停止节点
    pub async fn Stop(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::Stop).await?;
        Ok(())
    }
}
