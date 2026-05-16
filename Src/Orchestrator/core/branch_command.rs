// Presented by KeJi
// Date ： 2026-05-16

//! B2: Request-Response 入站请求路由。
//!
//! 处理来自 Network_Service 的 InboundRequest（仅 Command + File DataType）。

use super::Core;
use crate::network::{InboundRequest, DataType};

impl Core {
    /// 路由入站请求 (B2)。
    pub async fn route_inbound(&mut self, req: InboundRequest) {
        match req.data_type {
            DataType::Command => {
                // 解析 NetworkProtocol → 分发到对应处理逻辑
                // TODO: 未来实现 Establish_Tensor_Stream / Join_Pipeline / Profile_Request
                todo!("Inbound Command routing")
            }
            DataType::File => {
                // 文件元数据协商 → pending_file_receives
                todo!("Inbound File metadata")
            }
            _ => {
                // BandwidthTest / Data / Info 由 Network_Service 内部处理，不达到 Core
            }
        }
    }
}
