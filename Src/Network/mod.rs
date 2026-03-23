//Presented by KeJi
//Date: 2026-03-12

//! 网络层模块
//!
//! 本模块负责P2P网络通信，包括：
//! - 节点发现（mDNS/Kademlia）
//! - 连接管理
//! - 消息传输（命令/文件/张量）
//! - DHT分布式存储

pub mod node;
pub mod protocol;

// 重新导出常用类型
pub use node::{NetworkConfig, NetworkEvent, Node, NodeCommand, NodeHandle, PeerInfo};
pub use protocol::{
    CommandRequest, CommandResponse, FileRequest, FileResponse, PleiadesCodec, PleiadesRequest,
    PleiadesResponse, TensorDtype, TensorRequest, TensorResponse, FILE_CHUNK_SIZE,
    TENSOR_CHUNK_SIZE,
};
