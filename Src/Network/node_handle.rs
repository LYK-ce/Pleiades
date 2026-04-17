//Presented by KeJi
//Date ： 2026-04-03

//! 网络节点对外 API 句柄模块
//!
//! 包含 NodeHandle（对外暴露的 API，可 Clone 可 Send）和
//! NodeCommand（外部命令枚举，通过通道发送给 Network_Service 执行）。
//!
//! ## 主要 API
//! - `Send_Data`: 发送数据并等待 Response（同步语义）
//! - `Send_Response`: 回复入站请求（通过 request_id）
//! - `Send_File`: 文件传输（元数据协商 + 流式传输）
//! - `Send_File_Stream`: 直接流式发送文件（无协商）

use libp2p::{
    Multiaddr, PeerId,
};
use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use super::data_protocol::{DataType, Network_Data};

// ===== 入站请求结构 =====

/// 入站请求（由 Network_Service 转发给 Control 层）
///
/// 不包含 libp2p 内部类型（ResponseChannel），
/// Control 层通过 `request_id` 引用并回复。
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

/// 节点命令（外部通过NodeHandle发送给Node执行）
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
    /// 流式文件发送（默认已确认对方接受，直接建立流式传输）
    SendFileStream {
        peer: PeerId,
        file_path: PathBuf,
        /// 可选：传输完成后通知调用方
        completion_tx: Option<oneshot::Sender<Result<(), String>>>,
    },
    /// DHT写入
    PutRecord { key: Vec<u8>, value: Vec<u8> },
    /// DHT读取
    GetRecord { key: Vec<u8> },
    /// 主动连接
    Dial { addr: Multiaddr },
    /// 断开连接
    Disconnect { peer: PeerId },
    /// 回复入站请求（通过 request_id，供 Control 层使用）
    SendResponse {
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    },
    /// 获取已连接节点列表
    GetPeers {
        reply: oneshot::Sender<Vec<PeerId>>,
    },
    /// 获取指定节点的详细信息
    GetPeerInfo {
        peer: PeerId,
        reply: oneshot::Sender<Option<crate::PeerInfo>>,
    },

    // ===== Tensor Stream 命令 =====

    /// 创建 Tensor_Stream_Manager 对（inbound + outbound）
    CreateTensorStream {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// 打开到指定节点的出站张量流
    OpenTensorStream {
        peer: PeerId,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// 移交张量流所有权（从 Manager 取出 stream，用于构建 Tensor_IO_Handle）
    TakeTensorStreams {
        reply: oneshot::Sender<Result<(libp2p::Stream, libp2p::Stream), String>>,
    },
    /// 关闭张量流 Manager（清理）
    CloseTensorStream {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// 更新所有节点的带宽信息
    UpdateInfo {
        reply: oneshot::Sender<Result<(usize, usize), Box<dyn Error + Send + Sync>>>,
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
}

impl NodeHandle {
    /// 创建新的 NodeHandle（仅供 Network_Service::Init 内部使用）
    pub(crate) fn New(cmd_tx: mpsc::Sender<NodeCommand>, local_peer_id: PeerId) -> Self {
        Self { cmd_tx, local_peer_id }
    }

    /// 获取本地节点ID
    pub fn Get_Local_Peer_Id(&self) -> PeerId {
        self.local_peer_id
    }

    // ===== 核心 API（供 Control 层使用） =====

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

        // 等待 Response，30 秒超时
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            rx,
        ).await
            .map_err(|_| "Response timeout (30s)")?
            .map_err(|_| "Response channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })?;

        Ok(result)
    }

    /// 回复入站请求（通过 request_id）
    ///
    /// Control 层收到 InboundRequest 后，通过此方法回复。
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

    /// 发送文件（元数据协商 + 流式传输）
    ///
    /// 1. 通过 DataType::File 发送文件元数据通知
    /// 2. 等待对方回复（ACCEPT / REJECT）
    /// 3. 如果接受，启动流式传输
    ///
    /// # Arguments
    /// * `peer` - 目标节点 ID
    /// * `file_path` - 待发送文件路径
    pub async fn Send_File(
        &self,
        peer: &PeerId,
        file_path: PathBuf,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // 1. 构建文件元数据
        let file_name = file_path
            .file_name()
            .ok_or("Invalid file path")?
            .to_string_lossy();
        let file_size = tokio::fs::metadata(&file_path).await?.len();

        let mut metadata = Vec::new();
        let name_bytes = file_name.as_bytes();
        metadata.extend_from_slice(&(name_bytes.len() as u32).to_be_bytes());
        metadata.extend_from_slice(name_bytes);
        metadata.extend_from_slice(&file_size.to_be_bytes());

        // 2. 发送元数据通知并等待确认
        let response = self.Send_Data(peer, DataType::File, metadata).await?;

        // 3. 检查对方是否接受
        if response.payload != b"ACCEPT" {
            return Err("File transfer rejected by remote peer".into());
        }

        // 4. 启动流式传输
        self.Send_File_Stream(peer, file_path).await?;
        Ok(())
    }

    // ===== 底层 API =====

    /// 流式文件发送（直接传输，无协商）
    ///
    /// 通过流式传输协议发送文件，等待传输完成后才返回。
    /// 发送方打开流 → 写入分块数据，接收方边收边写磁盘。
    ///
    /// # Arguments
    /// * `peer` - 目标节点 ID
    /// * `file_path` - 待发送文件的路径
    pub async fn Send_File_Stream(
        &self,
        peer: &PeerId,
        file_path: PathBuf,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::SendFileStream {
            peer: *peer,
            file_path,
            completion_tx: Some(tx),
        }).await?;

        // 等待传输完成（600 秒超时）
        let result = tokio::time::timeout(
            Duration::from_secs(600),
            rx,
        ).await
            .map_err(|_| "File stream transfer timeout (600s)")?
            .map_err(|_| "File stream completion channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })?;

        Ok(result)
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

    /// 获取当前已连接的所有节点列表
    ///
    /// # Returns
    /// 已连接节点的 PeerId 列表
    pub async fn Get_Peers(&self) -> Result<Vec<PeerId>, Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::GetPeers { reply: tx }).await?;
        let peers = rx.await.map_err(|_| "GetPeers reply channel closed")?;
        Ok(peers)
    }

    /// 获取指定节点的详细信息
    ///
    /// # Arguments
    /// * `peer` - 要查询的节点 ID
    ///
    /// # Returns
    /// 节点信息（如果已连接），否则返回 None
    pub async fn Get_Peer_Info(
        &self,
        peer: &PeerId,
    ) -> Result<Option<crate::PeerInfo>, Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::GetPeerInfo { peer: *peer, reply: tx }).await?;
        let info = rx.await.map_err(|_| "GetPeerInfo reply channel closed")?;
        Ok(info)
    }

    // ===== Tensor Stream API =====

    /// 创建 Tensor_Stream_Manager 对（inbound + outbound）
    ///
    /// 在推理任务开始前调用。创建两个 Manager 负责流的建立。
    pub async fn Create_Tensor_Stream(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::CreateTensorStream {
            reply: tx,
        }).await?;
        rx.await
            .map_err(|_| "CreateTensorStream reply channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })
    }

    /// 打开到指定节点的出站张量流
    ///
    /// 在 PIPELINE_FLOW 处理时调用，建立到 next_peer 的持久化张量流。
    ///
    /// # Arguments
    /// * `peer` - 目标节点 ID
    pub async fn Open_Tensor_Stream(
        &self,
        peer: &PeerId,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::OpenTensorStream {
            peer: *peer,
            reply: tx,
        }).await?;
        rx.await
            .map_err(|_| "OpenTensorStream reply channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })
    }

    /// 移交张量流所有权，用于构建 Tensor_IO_Handle
    ///
    /// 从两个 Manager 中取出 inbound 和 outbound stream。
    /// 调用后 Manager 不再持有 stream，流的读写由 Tensor_IO_Handle 接管。
    ///
    /// # Returns
    /// (inbound_stream, outbound_stream) 元组
    pub async fn Take_Tensor_Streams(
        &self,
    ) -> Result<(libp2p::Stream, libp2p::Stream), Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::TakeTensorStreams {
            reply: tx,
        }).await?;
        rx.await
            .map_err(|_| "TakeTensorStreams reply channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })
    }

    /// 关闭张量流 Manager
    ///
    /// 清理 Manager 资源。实际的 stream 关闭由 Tensor_IO_Handle drop 处理。
    /// 推理结束时调用。
    pub async fn Close_Tensor_Stream(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::CloseTensorStream {
            reply: tx,
        }).await?;
        rx.await
            .map_err(|_| "CloseTensorStream reply channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })
    }

    /// 更新所有节点的带宽信息
    ///
    /// 对PeerManager中存储的所有peer进行带宽测试，逐个串行测试以避免网络干扰。
    /// 返回成功测试的节点数量和总节点数量。
    pub async fn Update_Info(&self) -> Result<(usize, usize), Box<dyn Error + Send + Sync>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx.send(NodeCommand::UpdateInfo {
            reply: tx,
        }).await?;
        rx.await
            .map_err(|_| "UpdateInfo reply channel closed")?
            .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })
    }

    /// 停止节点
    pub async fn Stop(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::Stop).await?;
        Ok(())
    }
}
