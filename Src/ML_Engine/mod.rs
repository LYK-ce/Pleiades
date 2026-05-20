//Presented by KeJi
//Date ： 2026-05-16

//! ML_Engine模块 - ML推理引擎
//!
//! 提供 GGUF 模型解析、加载、Qwen3 推理、编解码、采样等功能。
//!
//! ## 架构
//! - `context`: MlContext 推理上下文（7 个公开方法 + 状态查询）
//! - `capability`: 独立操作（analyze_model / split_model）
//! - `gguf_model`: 模型抽象（load / unload / inference / encode / decode）
//! - `gguf_model_manager`: 底层 GGUF 操作（analyze / load_layer / split）
//! - `gguf_tensor`: 张量序列化（网络传输用）
//! - `lua_tensor`: Tensor 的 mlua UserData 包装（跨 Lua 边界）

pub mod gguf_tensor;
pub mod lua_tensor;
pub mod gguf_model_manager;
pub mod gguf_model;
pub mod context;
pub mod capability;

#[path = "GGUF_Models/mod.rs"]
pub mod gguf_models;

pub use gguf_tensor::{
    GGUF_Tensor_Packet, GGUF_Dtype, GGUF_Tensor_Error,
    GGUF_Tensor_Serialize, GGUF_Tensor_Deserialize,
};
pub use gguf_model_manager::{
    Model_Arch_Info, Layer_Info, Tensor_Detail, GGUF_Layer_Weights,
    GGUF_Analyze, GGUF_Analyze_From_Content, GGUF_Analyze_And_Convert, GGUF_Load_Layer, GGUF_Split_Model, Resolve_Model_Path,
};
pub use gguf_models::{
    Gguf, Rotary_Embedding, Mlp_Weights, Attention_Weights,
    Layer_Weights, Model_Weights, Qwen3_Config,
};
pub use gguf_model::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
    GGUF_Encode, GGUF_Decode, Inference_Config,
};

pub use context::MlSession;
pub use capability::{analyze_model, split_model};
