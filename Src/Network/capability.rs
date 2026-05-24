//Presented by KeJi
//Date ： 2026-04-24

//! Network Capability 模块
//!
//! 定义 Network 层对外暴露的 Capability trait，
//! Orchestrator 通过 `Network_Capability` trait 接口调用网络操作。
//!
//! ## 设计原则
//! - **请求-响应**：委托 NodeHandle 通过命令通道
//! - **文件流传输**：直接操作 `stream::Control`，不经过 Network_Service 事件循环
//! - **张量流**：直接操作 `stream::Control`
//! - **DHT**：委托 NodeHandle
//!
//! ## 入站事件
//! Network_Service 将复杂入站事件（文件流、张量流）通过
//! `Network_Inbound_Event` 转发给 Orchestrator 处理。

#![allow(nonstandard_style)]

use async_trait::async_trait;
use libp2p::{Multiaddr, PeerId, StreamProtocol};
use libp2p_stream as stream;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::event_bus::EventBus;
use super::request_response::codec::{DataType, Network_Data};
use super::node_handle::NodeHandle;
use super::file_stream::protocol::{
    FILE_STREAM_PROTOCOL, Send_File_Data, Receive_File_Data,
    Write_File_Stream_Header,
};
use super::tensor_stream::protocol::TENSOR_STREAM_PROTOCOL;
use super::bandwidth_stream::protocol::{
    BANDWIDTH_STREAM_PROTOCOL, DEFAULT_DURATION_SECS,
    Run_Bandwidth_Test,
};

// ===== 错误类型 =====

/// Network 操作错误
///
/// 覆盖连接、流、超时、拒绝、通道关闭等场景。
#[derive(Debug)]
pub enum Network_Error {
    /// 连接相关错误（dial 失败、连接中断等）
    ConnectionFailed(String),
    /// 流打开失败（stream::Control.open_stream 失败）
    StreamOpenFailed(String),
    /// 流 I/O 错误（读写流时发生的 IO 错误）
    StreamIoError(String),
    /// 请求超时（send_data 等待响应超时）
    Timeout(String),
    /// 对方拒绝（远端节点明确拒绝操作）
    Rejected(String),
    /// 命令发送失败（内部通道已关闭，Network_Service 已停止）
    ChannelClosed(String),
}

impl fmt::Display for Network_Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Network_Error::ConnectionFailed(msg) => write!(f, "ConnectionFailed: {}", msg),
            Network_Error::StreamOpenFailed(msg) => write!(f, "StreamOpenFailed: {}", msg),
            Network_Error::StreamIoError(msg) => write!(f, "StreamIoError: {}", msg),
            Network_Error::Timeout(msg) => write!(f, "Timeout: {}", msg),
            Network_Error::Rejected(msg) => write!(f, "Rejected: {}", msg),
            Network_Error::ChannelClosed(msg) => write!(f, "ChannelClosed: {}", msg),
        }
    }
}

impl std::error::Error for Network_Error {}

// ===== Network Capability Trait =====

/// Network 层能力 trait
///
/// Orchestrator 通过此 trait 调用所有网络操作。
/// 实现方为 `Network_Service_Capability`（持有 NodeHandle + stream::Control）。
///
/// ## 方法分类
/// - **请求-响应**：`send_data`, `send_response`（委托 NodeHandle）
/// - **连接管理**：`dial`, `disconnect`（委托 NodeHandle）
/// - **文件流传输**：`open_file_stream`, `send_file_data`, `receive_file_data`（直接操作 stream::Control）
/// - **张量流**：`open_tensor_stream`（直接操作 stream::Control）
/// - **DHT**：`put_record`, `get_record`（委托 NodeHandle）
#[async_trait]
pub trait Network_Capability: Send + Sync {
    // ========================================
    // 请求-响应（委托 NodeHandle）
    // ========================================

    /// 发送数据并等待对方响应
    ///
    /// 内部通过 NodeHandle.Send_Data 实现，30 秒超时。
    ///
    /// # 参数
    /// - `peer`: 目标节点 ID
    /// - `data_type`: 数据类型标记
    /// - `payload`: 原始载荷字节流
    ///
    /// # 返回
    /// 对方的响应 `Network_Data`
    ///
    /// # 用法
    /// ```ignore
    /// let response = network.send_data(peer, DataType::Command, payload).await?;
    /// ```
    async fn send_data(
        &self,
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<Network_Data, Network_Error>;

    /// 回复入站请求
    ///
    /// # 参数
    /// - `request_id`: 入站请求的编号（由 InboundRequest 提供）
    /// - `data_type`: 数据类型标记
    /// - `payload`: 响应载荷字节流
    ///
    /// # 用法
    /// ```ignore
    /// network.send_response(request_id, DataType::Command, b"OK".to_vec()).await?;
    /// ```
    async fn send_response(
        &self,
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Network_Error>;

    // ========================================
    // 连接管理（委托 NodeHandle）
    // ========================================

    /// 主动连接到指定地址
    ///
    /// # 参数
    /// - `addr`: 目标节点的 Multiaddr 地址
    async fn dial(&self, addr: Multiaddr) -> Result<(), Network_Error>;

    /// 断开与指定节点的连接
    ///
    /// # 参数
    /// - `peer`: 目标节点 ID
    async fn disconnect(&self, peer: PeerId) -> Result<(), Network_Error>;

    // ========================================
    // 文件流传输（直接操作 stream::Control）
    // ========================================

    /// 打开到目标节点的文件流连接
    ///
    /// 返回 raw `libp2p::Stream`，调用方在自己的 tokio task 中使用。
    /// 不通过 Network_Service 事件循环，不 spawn 新任务。
    ///
    /// # 参数
    /// - `peer`: 目标节点 ID
    ///
    /// # 返回
    /// 已打开的 `libp2p::Stream`
    ///
    /// # 用法（发送端完整流程）
    /// ```ignore
    /// // 1. 发送文件元数据，等待对方 ACCEPT
    /// let metadata = encode_file_metadata(file_name, file_size);
    /// let response = network.send_data(peer, DataType::File, metadata).await?;
    /// if response.payload != b"ACCEPT" { return Err("rejected"); }
    ///
    /// // 2. 获取 Storage 读锁
    /// let (path, _guard) = storage.acquire_read(file_id).await?;
    ///
    /// // 3. 打开文件流并发送纯数据
    /// let mut stream = network.open_file_stream(peer).await?;
    /// network.send_file_data(&mut stream, &path).await?;
    /// // _guard drop → 释放读锁
    /// ```
    async fn open_file_stream(&self, peer: PeerId) -> Result<libp2p::Stream, Network_Error>;

    /// 通过已打开的流发送文件数据（纯 raw data，无 header）
    ///
    /// 文件元数据（文件名、大小）已通过 Request-Response 协商完成，
    /// 流中只包含分块的文件原始数据。
    /// 在调用方的 tokio task 中执行，不 spawn 新任务。
    ///
    /// # 参数
    /// - `stream`: 由 `open_file_stream` 返回的流
    /// - `file_path`: 待发送文件的路径（由 StorageManager.acquire_read 返回）
    async fn send_file_data(
        &self,
        stream: &mut libp2p::Stream,
        file_path: &Path,
    ) -> Result<(), Network_Error>;

    /// 从入站流接收文件数据并写入指定路径（纯 raw data，无 header）
    ///
    /// 文件元数据（文件名、大小）已通过 Request-Response 协商获得，
    /// 调用方据此通过 StorageManager 获取写锁和目标路径。
    /// 在调用方的 tokio task 中执行，不 spawn 新任务。
    ///
    /// # 参数
    /// - `stream`: 入站流（由 `Network_Inbound_Event::FileStreamArrived` 提供）
    /// - `dest_path`: 目标文件路径（由 StorageManager.acquire_write 返回）
    /// - `file_size`: 文件大小（由元数据协商获得）
    ///
    /// # 用法（接收端完整流程）
    /// ```ignore
    /// // 1. 获取 Storage 写锁
    /// let (dest_path, _guard) = storage.acquire_write(&file_name).await?;
    ///
    /// // 2. 从入站流接收文件数据
    /// network.receive_file_data(&mut stream, &dest_path, file_size).await?;
    ///
    /// // 3. _guard drop → 释放写锁，文件已注册到 Storage 索引
    /// ```
    async fn receive_file_data(
        &self,
        stream: &mut libp2p::Stream,
        dest_path: &Path,
        file_size: u64,
    ) -> Result<(), Network_Error>;

    /// 发送文件高层接口（内部串联动作为原子操作）。
    ///
    /// 串联: open_file_stream → Write_File_Stream_Header → send_file_data。
    async fn send_file(
        &self,
        peer: PeerId,
        file_path: &Path,
    ) -> Result<(), Network_Error>;

    // ========================================
    // 张量流（直接操作 stream::Control + RendezvousMap）
    // ========================================

    /// 打开到目标节点的张量流（自动写入 handshake）
    ///
    /// 返回 raw `libp2p::Stream`，由 ML Thread Lua 持有。
    ///
    /// # 参数
    /// - `peer`: 目标节点 ID
    /// - `inference_id`: 推理会话唯一标识，写入 handshake 供对端 rendezvous 匹配
    ///
    /// # 返回
    /// 已打开并完成 handshake 的 `libp2p::Stream`
    async fn open_tensor_stream(
        &self, peer: PeerId, inference_id: u64,
    ) -> Result<libp2p::Stream, Network_Error>;

    /// 等待对端发来的张量流（rendezvous 匹配）
    ///
    /// 通过 RendezvousMap 匹配对端 open_tensor_stream 发来的入站流。
    /// 内部 block_on oneshot，可指定超时时间。
    ///
    /// # 参数
    /// - `inference_id`: 推理会话唯一标识
    /// - `timeout_secs`: 超时秒数
    ///
    /// # 返回
    /// 匹配到的入站 `libp2p::Stream`，或超时错误
    async fn accept_tensor_stream(
        &self, inference_id: u64, timeout_secs: u64,
    ) -> Result<libp2p::Stream, Network_Error>;

    // ========================================
    // DHT（委托 NodeHandle）
    // ========================================

    /// DHT 写入
    ///
    /// # 参数
    /// - `key`: 键（原始字节）
    /// - `value`: 值（原始字节）
    async fn put_record(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Network_Error>;

    /// DHT 读取
    ///
    /// # 参数
    /// - `key`: 键（原始字节）
    async fn get_record(&self, key: Vec<u8>) -> Result<(), Network_Error>;

    // ========================================
    // 节点信息
    // ========================================

    /// 获取本地节点的 PeerId
    ///
    /// Coordinator 编排 Pipeline 时需要 local PeerId 生成全局唯一 inference_id。
    fn get_local_peer_id(&self) -> PeerId;

    // ========================================
    // 带宽测试
    // ========================================

    /// 测试与指定节点之间的带宽（Mbps）
    ///
    /// iperf 风格固定时长推流测速，默认 3 秒。
    /// 内部使用 `Bandwidth_Stream`，对端 Network_Service 自动计数并回传结果。
    ///
    /// # 参数
    /// - `peer`: 目标节点 ID
    ///
    /// # 返回
    /// 带宽值（Mbps）
    async fn test_bandwidth(&self, peer: PeerId) -> Result<u64, Network_Error>;
}

// ===== 入站事件枚举 =====

/// Network 层转发给 Orchestrator 的入站事件
///
/// Network_Service 在 select! 循环中接收到复杂入站事件后，
/// 通过 `orchestrator_event_tx` 通道转发给 Orchestrator 处理。
///
/// ## 简单事件（Network 内部处理，不转发）
/// - mDNS 发现/离开 → 仅 Kademlia 添加地址
/// - Ping 心跳 → peer_handle.Update_Profile
/// - 连接建立/断开 → peer_handle.Upsert_Peer / Remove_Peer
///
/// ## 复杂事件（转发给 Orchestrator）
    /// - 入站文件流 → `FileStreamArrived`
pub enum Network_Inbound_Event {
    /// 入站文件流（远端节点主动发送文件）
    ///
    /// Orchestrator 收到后 compile 接收作业 → spawn Job，
    /// 将 stream 存入 SlotFile 供 Executor 使用。
    FileStreamArrived {
        /// 发送方节点 ID
        peer: PeerId,
        /// 入站的 raw libp2p::Stream（所有权移交给 Orchestrator）
        stream: libp2p::Stream,
    },
}

// ===== Network_Service_Capability 实现 =====

/// Network Capability 的实际实现
///
/// 持有 `NodeHandle`（简单命令走命令通道）和两个 `stream::Control`
/// （文件流和张量流直接 open，不经过 Network_Service 事件循环）。
///
/// ## 职责划分
/// - **NodeHandle 委托**：send_data, send_response, dial, disconnect, put_record, get_record
/// - **stream::Control 直接调用**：open_file_stream, open_tensor_stream
/// - **stream_protocol 函数调用**：send_file_data, receive_file_data
pub struct Network_Service_Capability {
    /// 简单操作走命令通道（Send_Data, Send_Response, Dial, Disconnect, DHT）
    node_handle: NodeHandle,
    /// 文件流直接 open（不经过 Network_Service 事件循环）
    file_stream_control: stream::Control,
    /// 张量流直接 open（不经过 Network_Service 事件循环）
    tensor_stream_control: stream::Control,
    /// 带宽测试流直接 open（不经过 Network_Service 事件循环）
    bandwidth_stream_control: stream::Control,
    /// 张量流 rendezvous 匹配（与 Network_Service Event Loop 共享）
    pub tensor_rendezvous: std::sync::Arc<super::tensor_stream::rendezvous::RendezvousMap>,
    /// 事件总线（文件传输进度上报等）
    event_bus: Arc<EventBus>,
}

impl Network_Service_Capability {
    /// 创建 Network_Service_Capability
    ///
    /// # 参数
    /// - `node_handle`: NodeHandle（可 Clone，用于命令通道操作）
    /// - `file_stream_control`: 文件流的 stream::Control（由 Network_Service::Init 创建）
    /// - `tensor_stream_control`: 张量流的 stream::Control（由 Network_Service::Init 创建）
    /// - `bandwidth_stream_control`: 带宽测试流的 stream::Control（由 Network_Service::Init 创建）
    /// - `tensor_rendezvous`: RendezvousMap（与 Network_Service Event Loop 共享）
    /// - `event_bus`: 事件总线（文件传输进度上报等）
    pub fn New(
        node_handle: NodeHandle,
        file_stream_control: stream::Control,
        tensor_stream_control: stream::Control,
        bandwidth_stream_control: stream::Control,
        tensor_rendezvous: std::sync::Arc<super::tensor_stream::rendezvous::RendezvousMap>,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self {
            node_handle,
            file_stream_control,
            tensor_stream_control,
            bandwidth_stream_control,
            tensor_rendezvous,
            event_bus,
        }
    }
}

#[async_trait]
impl Network_Capability for Network_Service_Capability {
    // ========================================
    // 请求-响应（委托 NodeHandle）
    // ========================================

    async fn send_data(
        &self,
        peer: PeerId,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<Network_Data, Network_Error> {
        self.node_handle
            .Send_Data(&peer, data_type, payload)
            .await
            .map_err(|e| Network_Error::ChannelClosed(e.to_string()))
    }

    async fn send_response(
        &self,
        request_id: u64,
        data_type: DataType,
        payload: Vec<u8>,
    ) -> Result<(), Network_Error> {
        self.node_handle
            .Send_Response(request_id, data_type, payload)
            .await
            .map_err(|e| Network_Error::ChannelClosed(e.to_string()))
    }

    // ========================================
    // 连接管理（委托 NodeHandle）
    // ========================================

    async fn dial(&self, addr: Multiaddr) -> Result<(), Network_Error> {
        self.node_handle
            .Dial(addr)
            .await
            .map_err(|e| Network_Error::ConnectionFailed(e.to_string()))
    }

    async fn disconnect(&self, peer: PeerId) -> Result<(), Network_Error> {
        self.node_handle
            .Disconnect(&peer)
            .await
            .map_err(|e| Network_Error::ConnectionFailed(e.to_string()))
    }

    // ========================================
    // 文件流传输（直接操作 stream::Control）
    // ========================================

    async fn open_file_stream(&self, peer: PeerId) -> Result<libp2p::Stream, Network_Error> {
        self.file_stream_control
            .clone()
            .open_stream(peer, StreamProtocol::new(FILE_STREAM_PROTOCOL))
            .await
            .map_err(|e| Network_Error::StreamOpenFailed(format!("file stream: {}", e)))
    }

    async fn send_file_data(
        &self,
        stream: &mut libp2p::Stream,
        file_path: &Path,
    ) -> Result<(), Network_Error> {
        Send_File_Data(stream, file_path, &self.event_bus)
            .await
            .map_err(|e| Network_Error::StreamIoError(format!("send_file_data: {}", e)))
    }

    async fn receive_file_data(
        &self,
        stream: &mut libp2p::Stream,
        dest_path: &Path,
        file_size: u64,
    ) -> Result<(), Network_Error> {
        Receive_File_Data(stream, dest_path, file_size, &self.event_bus)
            .await
            .map_err(|e| Network_Error::StreamIoError(format!("receive_file_data: {}", e)))
    }

    async fn send_file(
        &self,
        peer: PeerId,
        file_path: &Path,
    ) -> Result<(), Network_Error> {
        let file_name = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let metadata = tokio::fs::metadata(file_path)
            .await
            .map_err(|e| Network_Error::StreamIoError(format!("metadata: {}", e)))?;
        let file_size = metadata.len();

        let mut stream = self.open_file_stream(peer).await?;
        Write_File_Stream_Header(&mut stream, &file_name, file_size, "")
            .await
            .map_err(|e| Network_Error::StreamIoError(format!("header: {}", e)))?;
        self.send_file_data(&mut stream, file_path).await
    }

    // ========================================
    // 张量流（直接操作 stream::Control + RendezvousMap）
    // ========================================

    async fn open_tensor_stream(
        &self, peer: PeerId, inference_id: u64,
    ) -> Result<libp2p::Stream, Network_Error> {
        let mut stream = self.tensor_stream_control
            .clone()
            .open_stream(peer, StreamProtocol::new(TENSOR_STREAM_PROTOCOL))
            .await
            .map_err(|e| Network_Error::StreamOpenFailed(format!("tensor stream: {}", e)))?;

        super::tensor_stream::protocol::Write_Tensor_Stream_Handshake(&mut stream, inference_id)
            .await
            .map_err(|e| Network_Error::StreamIoError(format!("handshake: {}", e)))?;

        Ok(stream)
    }

    async fn accept_tensor_stream(
        &self, inference_id: u64, timeout_secs: u64,
    ) -> Result<libp2p::Stream, Network_Error> {
        let rx = self.tensor_rendezvous.register_accept(inference_id);
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(_)) => Err(Network_Error::StreamIoError("rendezvous sender dropped".into())),
            Err(_) => Err(Network_Error::Timeout(format!(
                "accept_tensor_stream timeout after {}s", timeout_secs
            ))),
        }
    }

    // ========================================
    // DHT（委托 NodeHandle）
    // ========================================

    async fn put_record(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), Network_Error> {
        self.node_handle
            .Put_Record(key, value)
            .await
            .map_err(|e| Network_Error::ChannelClosed(e.to_string()))
    }

    async fn get_record(&self, key: Vec<u8>) -> Result<(), Network_Error> {
        self.node_handle
            .Get_Record(key)
            .await
            .map_err(|e| Network_Error::ChannelClosed(e.to_string()))
    }

    // ========================================
    // 节点信息
    // ========================================

    fn get_local_peer_id(&self) -> PeerId {
        self.node_handle.Get_Local_Peer_Id()
    }

    // ========================================
    // 带宽测试
    // ========================================

    async fn test_bandwidth(&self, peer: PeerId) -> Result<u64, Network_Error> {
        let mut stream = self.bandwidth_stream_control
            .clone()
            .open_stream(peer, StreamProtocol::new(BANDWIDTH_STREAM_PROTOCOL))
            .await
            .map_err(|e| Network_Error::StreamOpenFailed(format!("带宽测试流打开失败: {}", e)))?;

        Run_Bandwidth_Test(&mut stream, DEFAULT_DURATION_SECS).await
            .map_err(|e| Network_Error::StreamIoError(format!("带宽测试失败: {}", e)))
    }
}

// ===== 测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_error_display() {
        let error = Network_Error::ConnectionFailed("dial timeout".to_string());
        assert_eq!(error.to_string(), "ConnectionFailed: dial timeout");

        let error = Network_Error::StreamOpenFailed("protocol mismatch".to_string());
        assert_eq!(error.to_string(), "StreamOpenFailed: protocol mismatch");

        let error = Network_Error::StreamIoError("broken pipe".to_string());
        assert_eq!(error.to_string(), "StreamIoError: broken pipe");

        let error = Network_Error::Timeout("30s exceeded".to_string());
        assert_eq!(error.to_string(), "Timeout: 30s exceeded");

        let error = Network_Error::Rejected("peer refused file".to_string());
        assert_eq!(error.to_string(), "Rejected: peer refused file");

        let error = Network_Error::ChannelClosed("service stopped".to_string());
        assert_eq!(error.to_string(), "ChannelClosed: service stopped");
    }

    #[test]
    fn test_network_error_is_std_error() {
        let error: Box<dyn std::error::Error> =
            Box::new(Network_Error::Timeout("test".to_string()));
        assert!(error.to_string().contains("Timeout"));
    }

    #[test]
    fn test_network_error_debug() {
        let error = Network_Error::ConnectionFailed("test".to_string());
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("ConnectionFailed"));
    }

    #[test]
    fn test_network_inbound_event_variants_exist() {
        // 验证 Network_Inbound_Event 的类型定义可用于 match
        // 由于 libp2p::Stream 无法在测试中轻松构造，
        // 此测试仅验证类型和 match 臂的编译正确性
        fn _match_event(event: Network_Inbound_Event) {
            match event {
                Network_Inbound_Event::FileStreamArrived { peer: _, stream: _ } => {}
            }
        }
    }
}
