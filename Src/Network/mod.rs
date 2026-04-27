//Presented by KeJi
//Date ： 2026-04-24

//! 网络层模块
//!
//! 本模块负责P2P网络通信，包括：
//! - 节点发现（mDNS/Kademlia）
//! - 连接管理
//! - 统一数据传输（命令/数据/文件通知）
//! - 文件流传输（纯 raw data，元数据通过 Request-Response 协商）
//! - 张量流传输（持久化 stream，用于 pipeline 推理）
//! - DHT分布式存储
//!
//! ## 对 Orchestrator 暴露的接口
//!
//! ### Capability trait（推荐）
//! - `Network_Capability`: 统一 trait，覆盖请求-响应、连接管理、文件流、张量流、DHT
//! - `Network_Service_Capability`: 基于 NodeHandle + stream::Control 的 Capability 实现
//! - `Network_Inbound_Event`: 入站文件流/张量流事件，由 Network_Service 转发给 Orchestrator
//!
//! ### 底层操作（通过 NodeHandle）
//! - `Send_Data`: 发送数据并等待确认（同步语义）
//! - `Send_Response`: 回复入站请求（通过 request_id）
//!
//! ### 被动接收（通过 inbound_rx）
//! - `InboundRequest`: 其他节点发来的数据请求
//!
//! ## 内部组件
//! - `inbound_manager`: 入站请求与响应路由管理
//! - `outbound_manager`: 出站响应路由管理
//!
//! 网络层只负责发送和接收字节流，不负责序列化/反序列化。

pub mod capability;
pub mod network_service;
pub mod node_handle;
pub mod data_protocol;
pub mod stream_protocol;
pub mod tensor_stream_protocol;
pub mod inbound_manager;
pub mod outbound_manager;


// 重新导出常用类型
pub use capability::{Network_Capability, Network_Error, Network_Inbound_Event, Network_Service_Capability};
pub use network_service::{NetworkConfig, Network_Service};
pub use node_handle::{NodeCommand, NodeHandle, InboundRequest};
pub use data_protocol::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use stream_protocol::{FILE_STREAM_PROTOCOL, CHUNK_SIZE, Send_File_Data, Receive_File_Data};
pub use tensor_stream_protocol::{
    TENSOR_STREAM_PROTOCOL, TENSOR_EOF_OFFSET,
    Tensor_Buffer, Tensor_IO_Handle,
    Send_Tensor_Frame, Receive_Tensor_Frame, Send_EOF,
};
