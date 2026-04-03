//Presented by KeJi
//Date ： 2026-04-01

//! 网络层模块
//!
//! 本模块负责P2P网络通信，包括：
//! - 节点发现（mDNS/Kademlia）
//! - 连接管理
//! - 统一数据传输（命令/数据/文件通知）
//! - 流式文件传输
//! - DHT分布式存储
//!
//! 网络层只负责发送和接收字节流，不负责序列化/反序列化。

pub mod node;
pub mod node_handle;
pub mod data_protocol;
pub mod stream_protocol;


// 重新导出常用类型
pub use node::{NetworkConfig, NetworkEvent, Node, PeerInfo};
pub use node_handle::{NodeCommand, NodeHandle};
pub use data_protocol::{DataType, DataRequest, DataResponse, PleiadesCodec, DATA_PROTOCOL};
pub use stream_protocol::{FILE_STREAM_PROTOCOL, Send_File_Stream, Receive_File_Stream, CHUNK_SIZE};
