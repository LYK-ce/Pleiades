//Presented by KeJi
//Date : 2026-03-30

//! Runtime模块 - 推理运行时
//!
//! 提供 GGUF 模型解析、加载、Qwen3 推理等功能

pub mod error;
pub mod gguf_tensor;
pub mod gguf_runtime;
pub mod gguf_model_manager;
pub mod gguf_model;

#[path = "GGUF_Models/mod.rs"]
pub mod gguf_models;

pub use error::RuntimeError;
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
    Model_First_Half, Model_Second_Half, Build_Causal_Mask,
};
pub use gguf_model::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
    GGUF_Encode, GGUF_Decode, Inference_Config,
};
pub use gguf_runtime::GGUF_Runtime;
