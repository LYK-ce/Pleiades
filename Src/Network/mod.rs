//Presented by KeJi
//Date ： 2026-04-03

//! 网络层模块
//!
//! 本模块负责P2P网络通信，包括：
//! - 节点发现（mDNS/Kademlia）
//! - 连接管理
//! - 统一数据传输（命令/数据/文件通知）
//! - 流式文件传输
//! - DHT分布式存储
//!
//! ## 对 Control 层暴露的接口
//!
//! ### 主动操作（通过 NodeHandle）
//! - `Send_Bytes`: 发送数据并等待确认（同步语义）
//! - `Send_Reply`: 回复入站请求（通过 request_id）
//! - `Send_File`: 文件传输（元数据协商 + 流式传输）
//!
//! ### 被动接收（通过 inbound_rx）
//! - `InboundRequest`: 其他节点发来的数据请求
//!
//! 网络层只负责发送和接收字节流，不负责序列化/反序列化。

pub mod node;
pub mod node_handle;
pub mod data_protocol;
pub mod stream_protocol;


// 重新导出常用类型
pub use node::{NetworkConfig, NetworkEvent, Node, PeerInfo};
pub use node_handle::{NodeCommand, NodeHandle, InboundRequest};
pub use data_protocol::{DataType, DataRequest, DataResponse, PleiadesCodec, DATA_PROTOCOL};
pub use stream_protocol::{FILE_STREAM_PROTOCOL, Send_File_Stream, Receive_File_Stream, CHUNK_SIZE};
