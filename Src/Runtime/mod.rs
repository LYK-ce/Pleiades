//Presented by KeJi
//Date: 2026-03-12

//! Runtime模块 - 基于ONNX Runtime的推理运行时
//!
//! 提供模型加载、推理执行等功能

pub mod error;
pub mod tensor;
pub mod runtime;

pub use error::RuntimeError;
pub use tensor::{TensorPacket, TensorPacketSet, DataType, TensorPacketError};
pub use runtime::{Runtime, RuntimeConfig, ModelInfo, Init_ONNX, RuntimeInitConfig};
