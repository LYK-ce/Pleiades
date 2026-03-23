//Presented by KeJi
//Date: 2026-03-12

#![allow(non_snake_case)]

//! Pleiades - 边缘设备分布式推理运行时框架
//!
//! 本库提供以下模块：
//! - network: P2P网络通信模块
//! - runtime: 推理运行时模块
//! - Control: 控制层模块（待实现）

#[path = "Network/mod.rs"]
pub mod network;

#[path = "Runtime/mod.rs"]
pub mod runtime;

// 重新导出常用类型，便于测试使用
pub use network::{NetworkConfig, NetworkEvent, Node, NodeCommand, NodeHandle, PeerInfo};
pub use network::protocol;

// Runtime模块类型导出
pub use runtime::{Runtime, RuntimeConfig, RuntimeError, ModelInfo, Init_ONNX, RuntimeInitConfig};
pub use runtime::{TensorPacket, TensorPacketSet, DataType, TensorPacketError};
