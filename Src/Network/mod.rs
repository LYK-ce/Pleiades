//Presented by KeJi
//Created Date ： 2026-04-24
//Modified Date ： 2026-08-18

//! 网络层模块
//!
//! 本模块负责P2P网络通信，包括：
//! - 节点发现（mDNS/Kademlia）
//! - 连接管理
//! - 统一数据传输（命令/数据/文件通知）
//! - 文件流传输（纯 raw data，元数据通过 Request-Response 协商）
//! - 张量流传输（持久化 stream，用于 pipeline 推理）
//! - 带宽流传输（iperf 风格固定时长推流测速）
//!
//! ## 对 Orchestrator 暴露的接口
//!
//! ### Capability trait（推荐）
//! - `Network_Capability`: 统一 trait，覆盖请求-响应、连接管理、文件流、张量流、DHT
//! - `Network_Service_Capability`: 基于 NodeHandle + stream::Control 的 Capability 实现
//! - `Network_Inbound_Event`: 入站文件流事件，由 Network_Service 转发给 Orchestrator（张量流已改为 rendezvous 匹配，不再转发）
//!
//! ### 底层操作（通过 NodeHandle）
//! - `Send_Data`: 发送数据并等待确认（同步语义）
//! - `Send_Response`: 回复入站请求（通过 request_id）
//!
//! ### 被动接收（通过 inbound_rx）
//! - `InboundRequest`: 其他节点发来的数据请求
//!
//! ## 内部组件
//! - `request_response`: 请求-响应传输子系统（编解码、入站路由、出站路由）
//!
//! 网络层只负责发送和接收字节流，不负责序列化/反序列化。

pub mod capability;
pub mod network_service;
pub mod node_handle;
#[path = "Request_Response/mod.rs"]
pub mod request_response;
#[path = "File_Stream/mod.rs"]
pub mod file_stream;
#[path = "Tensor_Stream/mod.rs"]
pub mod tensor_stream;
#[path = "Bandwidth_Stream/mod.rs"]
pub mod bandwidth_stream;
#[path = "Session_Stream/mod.rs"]
pub mod session_stream;
mod command_handler;
mod swarm_events;
#[path = "Gossipsub/mod.rs"]
pub mod Gossipsub;
#[path = "DHT/mod.rs"]
pub mod DHT;

// 重新导出常用类型
pub use capability::{Network_Capability, Network_Error, Network_Inbound_Event, Network_Service_Capability};
pub use network_service::{NetworkConfig, Network_Service};
pub use node_handle::{NodeCommand, NodeHandle, InboundRequest};
pub use request_response::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use file_stream::{
    FILE_STREAM_PROTOCOL, CHUNK_SIZE,
    FILE_HEADER_ACCEPT, FILE_HEADER_REJECT,
    Write_File_Stream_Header, Read_File_Stream_Header,
    Write_File_Stream_Ack, Read_File_Stream_Ack,
    Send_File_Data, Receive_File_Data,
};
pub use tensor_stream::protocol::{
    TENSOR_STREAM_PROTOCOL, TENSOR_EOF_OFFSET,
    Tensor_Buffer,
    Send_Tensor_Frame, Receive_Tensor_Frame, Send_EOF,
    Write_Tensor_Stream_Handshake, Read_Tensor_Stream_Handshake,
};
pub use tensor_stream::rendezvous::RendezvousMap;
pub use bandwidth_stream::{
    BANDWIDTH_STREAM_PROTOCOL, DEFAULT_DURATION_SECS, MAX_DURATION_SECS,
    Send_Bandwidth_Test, Receive_And_Count,
    Write_Bandwidth_Result, Read_Bandwidth_Result,
    Run_Bandwidth_Test,
};
pub use session_stream::protocol::{
    SESSION_STREAM_PROTOCOL,
    Write_Session_Handshake, Read_Session_Handshake,
    write_session_frame, read_session_frame,
};
// GossipSub 业务 topic 常量与快照缓存（payload 构建在业务层 PeerManagement）
pub use Gossipsub::{
    TOPIC_PEER_INFO, TOPIC_MODELS, TOPIC_SESSIONS,
    TOPIC_ROBOT_POSE, TOPIC_ROBOT_MAP,
    SnapshotCache,
};
