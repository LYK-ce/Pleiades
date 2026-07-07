//Presented by KeJi
//Created Date ： 2026-03-30
//Modified Date ： 2026-07-03

//! GGUF_Models 模块 - 统一管理所有支持的 GGUF 格式模型实现
//!
//! 当前仅使用 Qwen3 / Qwen3MoE。
//! DeepSeek / Llama 模块注释保留，待后续恢复。

pub mod common;
pub mod qwen3;
// DeepSeek / Llama — 注释保留，待后续恢复
// #[cfg(feature = "deepseek")]
// pub mod deepseek_v3;
// #[cfg(feature = "deepseek")]
// pub mod deepseek_v4;
// #[cfg(feature = "llama")]
// pub mod llama;

// 重新导出公共基础类型
pub use common::{Rotary_Embedding, Mlp_Weights};
// 重新导出 Qwen3 相关类型
pub use qwen3::{
    Attention_Weights,
    Layer_Weights, Model_Weights,
    Qwen3_Config, Qwen3_Embedding_Stage, Qwen3_Output_Stage,
    Qwen3MoE_Model, Qwen3MoE_Layer,
};
