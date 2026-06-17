//Presented by KeJi
//Created Date ： 2026-03-30
//Modified Date ： 2026-06-17

//! GGUF_Models 模块 - 统一管理所有支持的 GGUF 格式模型实现
//!
//! 当前支持的模型:
//! - Qwen3: 通义千问3系列量化模型
//! - Qwen3 MoE: Qwen3 MoE 变体
//!
//! 暂时移除（代码保留在磁盘，Review 完成后恢复）:
//! - DeepSeek V3.2 (GGUF_Models/deepseek_v3.rs)
//! - DeepSeek V4   (GGUF_Models/deepseek_v4/)
//! - Llama 3.1     (GGUF_Models/llama.rs)

pub mod qwen3;
pub mod qwen3_moe;
#[cfg(feature = "deepseek")]
pub mod deepseek_v3;
#[cfg(feature = "deepseek")]
pub mod deepseek_v4;
#[cfg(feature = "llama")]
pub mod llama;

// 重新导出 Qwen3 相关类型，便于外部使用
pub use qwen3::{
    Gguf, Rotary_Embedding, Mlp_Weights, Attention_Weights,
    Layer_Weights, Model_Weights, Qwen3_Config,
};
