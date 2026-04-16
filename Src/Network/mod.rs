//Presented by KeJi
//Date ： 2026-04-10

//! 网络层模块
//!
//! 本模块负责P2P网络通信，包括：
//! - 节点发现（mDNS/Kademlia）
//! - 连接管理
//! - 统一数据传输（命令/数据/文件通知）
//! - 流式文件传输
//! - 张量流传输（持久化 stream，用于 pipeline 推理）
//! - DHT分布式存储
//!
//! ## 对 Control 层暴露的接口
//!
//! ### 主动操作（通过 NodeHandle）
//! - `Send_Data`: 发送数据并等待确认（同步语义）
//! - `Send_Response`: 回复入站请求（通过 request_id）
//! - `Send_File`: 文件传输（元数据协商 + 流式传输）
//! - `Create_Tensor_Stream`: 创建张量流管理器
//! - `Open_Tensor_Stream`: 打开到指定节点的出站张量流
//! - `Send_Tensor`: 发送张量（fire-and-forget）
//! - `Receive_Tensor`: 接收张量
//! - `Close_Tensor_Stream`: 关闭张量流
//!
//! ### 被动接收（通过 inbound_rx）
//! - `InboundRequest`: 其他节点发来的数据请求
//!
//! ## 内部组件
//! - `inbound_manager`: 入站请求与响应路由管理
//! - `outbound_manager`: 出站响应路由管理
//! - `file_transfer_manager`: 文件传输管理（流式传输控制、文件保存）
//! - `tensor_stream_manager`: 张量流传输管理（持久流、共享缓冲区）
//!
//! 网络层只负责发送和接收字节流，不负责序列化/反序列化。

pub mod network_service;
pub mod node_handle;
pub mod data_protocol;
pub mod stream_protocol;
pub mod tensor_stream_protocol;
pub mod tensor_stream_manager;
pub mod inbound_manager;
pub mod outbound_manager;
pub mod file_transfer_manager;


// 重新导出常用类型
pub use network_service::{NetworkConfig, NetworkEvent, Network_Service};
pub use node_handle::{NodeCommand, NodeHandle, InboundRequest};
// PeerInfo 已经从 peer_management 模块重新导出，可以直接使用 crate::PeerInfo
pub use data_protocol::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use stream_protocol::{FILE_STREAM_PROTOCOL, Send_File_Stream, Receive_File_Stream, CHUNK_SIZE};
pub use tensor_stream_protocol::{
    TENSOR_STREAM_PROTOCOL, TENSOR_EOF_OFFSET,
    Tensor_Buffer, Send_Tensor_Frame, Receive_Tensor_Frame, Send_EOF,
};
pub use tensor_stream_manager::{Tensor_Stream_Manager, Tensor_IO_Handle};
