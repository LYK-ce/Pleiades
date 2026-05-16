//Presented by KeJi
//Date ： 2026-05-16

//! Request-Response 传输子系统
//!
//! 提供基于 TLV 帧格式的请求-响应协议，包含：
//! - codec: TLV 编解码器 (PleiadesCodec)、数据类型枚举 (DataType)、网络数据帧 (Network_Data)
//! - inbound: 入站请求路由管理 (Inbound_Manager)
//! - outbound: 出站响应路由管理 (Outbound_Manager)

pub mod codec;
pub mod inbound;
pub mod outbound;

pub use codec::{DataType, Network_Data, PleiadesCodec, DATA_PROTOCOL};
pub use inbound::Inbound_Manager;
pub use outbound::Outbound_Manager;
