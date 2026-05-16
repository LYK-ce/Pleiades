// Presented by KeJi
// Date ： 2026-05-16

//! B3: Stream 入站事件路由。
//!
//! 处理来自 Network_Service 的 Network_Inbound_Event（FileStream + TensorStream）。

use super::Core;
use crate::network::Network_Inbound_Event;

impl Core {
    /// 路由 Stream 入站事件 (B3)。
    pub async fn route_stream(&mut self, event: Network_Inbound_Event) {
        match event {
            Network_Inbound_Event::FileStreamArrived { peer: _, stream: _ } => {
                todo!("FileStreamArrived")
            }
            Network_Inbound_Event::TensorStreamArrived { peer: _, stream: _ } => {
                todo!("TensorStreamArrived")
            }
        }
    }
}
