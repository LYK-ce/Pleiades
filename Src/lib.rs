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
//! - control: 控制层模块
//! - tui: TUI 终端界面模块

#[path = "Config/mod.rs"]
pub mod config;

#[path = "Network/mod.rs"]
pub mod network;

#[path = "ML_Engine/mod.rs"]
pub mod ml_engine;

#[path = "Control/mod.rs"]
pub mod control;

#[path = "TUI/mod.rs"]
pub mod tui;

#[path = "PeerManagement/mod.rs"]
pub mod peer_management;

// Config模块类型导出
pub use config::{Pleiades_Config, Log_Config, Network_Config, Runtime_Config, Read_Config, Ensure_Config, Update_Config, Ensure_Identity};

// Network模块类型导出
pub use network::{NetworkConfig, NetworkEvent, Network_Service, NodeCommand, NodeHandle};
pub use network::{DataType, Network_Data};
pub use network::InboundRequest;
pub use network::data_protocol;
pub use network::stream_protocol;
pub use network::tensor_stream_protocol;
pub use network::tensor_stream_manager;
pub use network::{
    TENSOR_STREAM_PROTOCOL, TENSOR_EOF_OFFSET,
    Tensor_Buffer, Tensor_Stream_Manager, Tensor_IO_Handle,
};

// ML_Engine模块类型导出
pub use ml_engine::{RuntimeError};
pub use ml_engine::{
    GGUF_Tensor_Packet, GGUF_Dtype, GGUF_Tensor_Error,
    GGUF_Tensor_Serialize, GGUF_Tensor_Deserialize,
};
pub use ml_engine::{
    Model_Arch_Info, Layer_Info, Tensor_Detail, GGUF_Layer_Weights,
    GGUF_Analyze, GGUF_Load_Layer, GGUF_Split_Model,
};
pub use ml_engine::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
    GGUF_Encode, GGUF_Decode, Inference_Config,
};
// ML Service 层（独立函数 API，不再有 ML_Service_Handle）
pub use ml_engine::{Create_Session, Split_Model, Analyze_Model};
// 指令集
pub use ml_engine::{
    Instruction, Inference_Input, Set_Target,
    Pipeline_Params, Pipeline_Result, Model_Info,
    Engine_Input, Engine_Output,
};
// Session/Thread 引擎
pub use ml_engine::{
    Inference_Backend, Session, Session_Handle, Session_Config,
    Session_Command, Session_Thread, Execute,
};

// Control模块类型导出
pub use control::{CLI_Command, Control_Loop, Node_State};
pub use control::{Control_Command, Serialize_Command, Deserialize_Command};
pub use control::Ui_Message;

// TUI模块类型导出
pub use tui::TUI_Loop;

// PeerManagement模块类型导出
pub use peer_management::{PeerInfo, PeerStatus, PeerCapability, PeerManager, PeerHandle, PeerEvent, PeerError, create_peer_management};
