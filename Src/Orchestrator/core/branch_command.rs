// Presented by KeJi
// Date ： 2026-05-19

//! B2: Request-Response 入站请求路由。
//!
//! 处理来自 Network_Service 的 InboundRequest（仅 Command + File DataType）。

use super::Core;
use crate::event_bus::{Bus_Event, NotifyLevel};
use crate::network::{InboundRequest, DataType};

impl Core {
    /// 路由入站请求 (B2)。
    ///
    /// 当前 Command 和 File 入站暂未实现，通过 EventBus 输出日志通知，
    /// 避免 `todo!()` panic 导致进程崩溃。
    pub async fn route_inbound(&mut self, req: InboundRequest) {
        match req.data_type {
            DataType::Command => {
                // TODO: 未来实现 Establish_Tensor_Stream / Join_Pipeline / Profile_Request
                self.capabilities.event_bus.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Info,
                    message: format!(
                        "[route_inbound] Command 入站暂未实现 (peer: {})",
                        req.peer
                    ),
                });
                tracing::info!("[route_inbound] Command from {} not yet implemented", req.peer);
            }
            DataType::File => {
                // TODO: 未来实现文件元数据协商 → pending_file_receives
                self.capabilities.event_bus.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Info,
                    message: format!(
                        "[route_inbound] File 入站暂未实现 (peer: {})",
                        req.peer
                    ),
                });
                tracing::info!("[route_inbound] File from {} not yet implemented", req.peer);
            }
            _ => {
                // BandwidthTest / Data / Info 由 Network_Service 内部处理，不达到 Core
            }
        }
    }
}
