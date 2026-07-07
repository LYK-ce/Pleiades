//Presented by KeJi
//Created Date ： 2026-05-16
//Modified Date ： 2026-06-17

//! ML_Engine模块 - ML推理引擎
//!
//! 提供 GGUF 模型解析、加载、Qwen3 推理、编解码、采样等功能。
//!
//! ## 架构
//! - `context`: MlContext 推理上下文（7 个公开方法 + 状态查询）
//! - `capability`: 独立操作（analyze_model / split_model）
//! - `gguf_model`: 模型抽象（load / unload / inference / encode / decode）
//! - `gguf_model_manager`: 底层 GGUF 操作（analyze / load_layer / split）
//! - `lua_tensor`: Tensor 的 mlua UserData 包装（跨 Lua 边界）

pub mod lua_tensor;
pub mod device;
pub mod gguf_model_manager;
pub mod gguf_model;
pub mod context;
pub mod capability;

#[path = "GGUF_Models/mod.rs"]
pub mod gguf_models;

pub use gguf_model_manager::{
    Model_Arch_Info, Layer_Info, Tensor_Detail, GGUF_Layer_Weights,
    GGUF_Analyze_From_Content, GGUF_Analyze_And_Convert, GGUF_Load_Layer, GGUF_Split_Model, Resolve_Model_Path,
};
pub use gguf_models::{
    Rotary_Embedding, Mlp_Weights, Attention_Weights,
    Layer_Weights, Model_Weights, Qwen3_Config,
};
pub use gguf_model::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
};
pub use gguf_models::common::model::Model;

pub use context::MlContext;
pub use capability::{analyze_model, split_model};
pub use device::Parse_Device_Str;
