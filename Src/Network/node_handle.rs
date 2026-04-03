//Presented by KeJi
//Date ： 2026-04-01

//! 网络节点对外 API 句柄模块
//!
//! 包含 NodeHandle（对外暴露的 API，可 Clone 可 Send）和
//! NodeCommand（外部命令枚举，通过通道发送给 Node 执行）。

use libp2p::{
    request_response::ResponseChannel,
    Multiaddr, PeerId,
};
use std::error::Error;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::data_protocol::{DataType, DataResponse};

/// 节点命令（外部通过NodeHandle发送给Node执行）
#[derive(Debug)]
pub enum NodeCommand {
    /// 统一数据发送
    SendData {
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    },
    /// 流式文件发送（默认已确认对方接受，直接建立流式传输）
    SendFileStream {
        peer: PeerId,
        file_path: PathBuf,
    },
    /// DHT写入
    PutRecord { key: Vec<u8>, value: Vec<u8> },
    /// DHT读取
    GetRecord { key: Vec<u8> },
    /// 主动连接
    Dial { addr: Multiaddr },
    /// 断开连接
    Disconnect { peer: PeerId },
    /// 发送响应
    SendResponse {
        channel: ResponseChannel<DataResponse>,
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
}

impl NodeHandle {
    /// 创建新的 NodeHandle（仅供 Node::Init 内部使用）
    pub(crate) fn New(cmd_tx: mpsc::Sender<NodeCommand>, local_peer_id: PeerId) -> Self {
        Self { cmd_tx, local_peer_id }
    }

    /// 获取本地节点ID
    pub fn Get_Local_Peer_Id(&self) -> PeerId {
        self.local_peer_id
    }

    /// 统一数据发送接口
    ///
    /// # Arguments
    /// * `peer` - 目标节点 ID
    /// * `data_type` - 数据类型标记（Command / Data / File）
    /// * `payload` - 已序列化的字节流（由上层负责序列化）
    pub async fn Send_Data(
        &self,
        peer: &PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::SendData {
            peer: *peer,
            data_type,
            payload,
        }).await?;
        Ok(())
    }

    /// 流式文件发送
    ///
    /// 通过流式传输协议发送文件，默认已收到对方接收确认。
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
        self.cmd_tx.send(NodeCommand::SendFileStream {
            peer: *peer,
            file_path,
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

    /// 发送响应
    pub async fn Send_Response(
        &self,
        channel: ResponseChannel<DataResponse>,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.cmd_tx.send(NodeCommand::SendResponse {
            channel,
            data_type,
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
