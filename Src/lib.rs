//Presented by KeJi
//Date ： 2026-04-13

#![allow(non_snake_case)]
#![allow(nonstandard_style)]

//! Pleiades - 边缘设备分布式推理运行时框架
//!
//! 本库提供以下模块：
//! - config: 配置文件解析模块
//! - network: P2P网络通信模块
//! - ml_engine: ML推理引擎模块
//! - orchestrator: 编排器模块
//! - storage: 存储管理模块
//! - session: LLM 文本交互通道模块（已升级为 session 模块）
//! - tensor_stream: 张量流子模块（Network 模块内部）
//! - peer_management: 节点管理模块
//! - event_bus: 全局事件总线模块

#[path = "Config/mod.rs"]
pub mod config;

#[path = "Network/mod.rs"]
pub mod network;

#[path = "ML_Engine/mod.rs"]
pub mod ml_engine;

#[path = "PeerManagement/mod.rs"]
pub mod peer_management;

#[path = "Orchestrator/mod.rs"]
pub mod orchestrator;

#[path = "Storage/mod.rs"]
pub mod storage;

#[path = "Session_Manager/mod.rs"]
pub mod session;


#[path = "EventBus/mod.rs"]
pub mod event_bus;

#[path = "TUI/mod.rs"]
pub mod tui;

#[path = "VM/mod.rs"]
pub mod vm;

// Config模块类型导出
pub use config::{Pleiades_Config, Log_Config, Network_Config, Runtime_Config, Storage_Config, Read_Config, Ensure_Config, Update_Config, Ensure_Identity};

// Network模块类型导出
pub use network::{NetworkConfig, Network_Service, NodeCommand, NodeHandle};
pub use network::{DataType, Network_Data};
pub use network::InboundRequest;
pub use network::{Network_Capability, Network_Error, Network_Inbound_Event, Network_Service_Capability};
pub use network::request_response;
pub use network::file_stream::protocol;
pub use network::tensor_stream;
pub use network::bandwidth_stream;
pub use network::{
    TENSOR_STREAM_PROTOCOL, TENSOR_EOF_OFFSET,
    Tensor_Buffer,
};

// ML_Engine模块类型导出
pub use ml_engine::{
    GGUF_Tensor_Packet, GGUF_Dtype, GGUF_Tensor_Error,
    GGUF_Tensor_Serialize, GGUF_Tensor_Deserialize,
};
pub use ml_engine::{
    Model_Arch_Info, Layer_Info, Tensor_Detail, GGUF_Layer_Weights,
    GGUF_Analyze, GGUF_Analyze_From_Content, GGUF_Analyze_And_Convert, GGUF_Load_Layer, GGUF_Split_Model, Resolve_Model_Path,
};
pub use ml_engine::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
    GGUF_Encode, GGUF_Decode, Inference_Config,
};
pub use ml_engine::MlSession;
pub use ml_engine::{analyze_model, split_model};

// PeerManagement模块类型导出
pub use peer_management::{
    PeerInfo, SupportedModel, PeerProfile, PeerManager, PeerHandle, create_peer_management,
};
pub use peer_management::{Peer_Management_Capability, Peer_Management_Error};

// Session模块类型导出（原 LLM_IO，已升级为 SessionManager）
pub use session::{Session_Capability, Session_Error, IoFrontend, IoHandle, SessionManager, SessionInfo};



// EventBus模块类型导出
pub use event_bus::{EventBus, Bus_Event};
