//Presented by KeJi
//Created Date ： 2026-06-17
//Modified Date ： 2026-06-17

//! 公共基础设施模块
//!
//! 包含被多个模型实现共享的基础类型：
//! - Rotary_Embedding: Rotary Position Embedding
//! - Mlp_Weights: 标准 SiLU-gated MLP

mod rope;
mod mlp;

pub use rope::Rotary_Embedding;
pub use mlp::Mlp_Weights;
