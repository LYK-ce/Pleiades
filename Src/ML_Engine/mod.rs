//Presented by KeJi
//Date : 2026-04-13

//! ML_Engine模块 - ML推理引擎
//!
//! 提供 GGUF 模型解析、加载、Qwen3 推理、Session 管理、指令驱动执行引擎等功能
//!
//! ## 架构
//! - `capability`: ML_Engine_Capability trait（对外接口）
//! - `service`: ML_Engine_Service 实现（持有 Storage + Session 注册表）
//! - `session`: Session/Thread 引擎核心
//! - `pipeline`: Pipeline 参数与结果类型
//! - `ml_vm`: ML_VM 执行引擎（基于 Vm_Base）

pub mod gguf_tensor;
pub mod gguf_model_manager;
pub mod gguf_model;
pub mod pipeline;
pub mod session;
pub mod ml_vm;
pub mod capability;
pub mod service;

#[path = "GGUF_Models/mod.rs"]
pub mod gguf_models;

pub use gguf_tensor::{
    GGUF_Tensor_Packet, GGUF_Dtype, GGUF_Tensor_Error,
    GGUF_Tensor_Serialize, GGUF_Tensor_Deserialize,
};
pub use gguf_model_manager::{
    Model_Arch_Info, Layer_Info, Tensor_Detail, GGUF_Layer_Weights,
    GGUF_Analyze, GGUF_Load_Layer, GGUF_Split_Model,
};
pub use gguf_models::{
    Gguf, Rotary_Embedding, Mlp_Weights, Attention_Weights,
    Layer_Weights, Model_Weights, Qwen3_Config,
};
pub use gguf_model::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
    GGUF_Encode, GGUF_Decode, Inference_Config,
};

pub use capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
pub use service::ML_Engine_Service;

pub use pipeline::{
    Pipeline_Params, Pipeline_Result, Model_Info,
};
