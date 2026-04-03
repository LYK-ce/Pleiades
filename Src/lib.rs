//Presented by KeJi
//Date ： 2026-04-01

#![allow(non_snake_case)]

//! Pleiades - 边缘设备分布式推理运行时框架
//!
//! 本库提供以下模块：
//! - config: 配置文件解析模块
//! - network: P2P网络通信模块
//! - runtime: 推理运行时模块
//! - Control: 控制层模块（待实现）

#[path = "Config/mod.rs"]
pub mod config;

#[path = "Network/mod.rs"]
pub mod network;

#[path = "Runtime/mod.rs"]
pub mod runtime;

// Config模块类型导出
pub use config::{Pleiades_Config, Log_Config, Network_Config, Runtime_Config, Read_Config};

// Network模块类型导出
pub use network::{NetworkConfig, NetworkEvent, Node, NodeCommand, NodeHandle, PeerInfo};
pub use network::{DataType, DataRequest, DataResponse};
pub use network::data_protocol;
pub use network::stream_protocol;

// Runtime模块类型导出
pub use runtime::{RuntimeError};
pub use runtime::{
    GGUF_Tensor_Packet, GGUF_Dtype, GGUF_Tensor_Error,
    GGUF_Tensor_Serialize, GGUF_Tensor_Deserialize,
};
pub use runtime::{
    Model_Arch_Info, Layer_Info, Tensor_Detail, GGUF_Layer_Weights,
    GGUF_Analyze, GGUF_Load_Layer, GGUF_Split_Model,
};
pub use runtime::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
    GGUF_Encode, GGUF_Decode, Inference_Config,
};
pub use runtime::GGUF_Runtime;
