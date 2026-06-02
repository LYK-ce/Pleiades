//Presented by KeJi
//Date : 2026-03-30

//! GGUF_Models 模块 - 统一管理所有支持的 GGUF 格式模型实现
//!
//! 当前支持的模型:
//! - Qwen3: 通义千问3系列量化模型
//! - DeepSeek V3.2: DeepSeek V3.2 系列模型 (MLA + MoE)

pub mod qwen3;
pub mod qwen3_moe;
pub mod deepseek_v3;
pub mod deepseek_v4;
pub mod llama;

// 重新导出 Qwen3 相关类型，便于外部使用
pub use qwen3::{
    Gguf, Rotary_Embedding, Mlp_Weights, Attention_Weights,
    Layer_Weights, Model_Weights, Qwen3_Config,
};
