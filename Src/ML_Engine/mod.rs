//Presented by KeJi
//Date : 2026-04-13

//! ML_Engine模块 - ML推理引擎
//!
//! 提供 GGUF 模型解析、加载、Qwen3 推理、Session 管理、指令驱动执行引擎等功能
//!
//! ## 架构
//! - `capability`: ML_Engine_Capability trait（对外接口）
//! - `service`: ML_Engine_Service 实现（持有 Storage + Session 注册表）
//! - `ml_inference_service`: 旧服务层（向后兼容，待迁移后删除）
//! - `ml_thread_engine`: Session/Thread 引擎核心（Session, Session_Handle, Execute）
//! - `ml_thread_engine_instruction`: 指令集定义
//! - `ml_thread_register`: 寄存器系统

pub mod error;
pub mod gguf_tensor;
pub mod ml_inference_service;
pub mod gguf_model_manager;
pub mod gguf_model;
pub mod ml_thread_engine_instruction;
pub mod ml_thread_engine;
pub mod ml_thread_register;
pub mod capability;
pub mod service;

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
};
pub use gguf_model::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model_Inference,
    GGUF_Encode, GGUF_Decode, Inference_Config,
};

// ML Service 层导出（旧 API，向后兼容 Control 层）
pub use ml_inference_service::{Create_Session, Split_Model, Analyze_Model};

// Capability 层导出（新 API，供 Orchestrator 使用）
pub use capability::{ML_Engine_Capability, ML_Engine_Error, ML_Session_Config};
pub use service::ML_Engine_Service;

// 指令集导出
pub use ml_thread_engine_instruction::{
    Instruction, Inference_Input, Set_Target,
    Pipeline_Params, Pipeline_Result, Model_Info,
};

// Session/Thread 引擎导出
pub use ml_thread_engine::{
    Inference_Backend, Session, Session_Handle, Session_Config,
    Session_Command, Session_Thread, Execute,
};

// 寄存器系统导出
pub use ml_thread_register::{
    Register_File, Text_Reg, Token_Reg, Tensor_Reg, Flag_Reg, Meta_Reg,
    TEXT1, TEXT2, TEXT3, TEXT4,
    TOKENID1, TOKENID2, TOKENID3, TOKENID4,
    TENSOR1, TENSOR2, TENSOR3, TENSOR4,
    FLAG1, FLAG2, FLAG3, FLAG4,
    META1, META2, META3, META4, META5, META6, META7, META8,
};
