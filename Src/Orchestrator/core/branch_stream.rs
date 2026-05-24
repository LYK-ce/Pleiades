// Presented by KeJi
// Date ： 2026-05-18

//! B3: Stream 入站事件路由。

use super::Core;
use crate::network::{Network_Inbound_Event, Read_File_Stream_Header};
use crate::event_bus::{Bus_Event, NotifyLevel};

impl Core {
    /// 路由 Stream 入站事件 (B3)。
    pub async fn route_stream(&mut self, event: Network_Inbound_Event) {
        match event {
            Network_Inbound_Event::FileStreamArrived { peer, mut stream } => {
                let peer_str = peer.to_base58();
                tracing::info!("收到入站文件流 from {}", peer_str);

                let (file_name, file_size, checksum) = match Read_File_Stream_Header(&mut stream).await {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::error!("读文件流 header 失败 from {}: {}", peer_str, e);
                        return;
                    }
                };

                let (dest_path, _guard) = match self.capabilities.storage.acquire_write(&file_name).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("Storage acquire_write 失败: {}", e);
                        return;
                    }
                };

                if let Err(e) = self.capabilities.network.receive_file_data(&mut stream, &dest_path, file_size).await {
                    tracing::error!("接收文件数据失败: {}", e);
                    return;
                }

                tracing::info!("文件接收完成: {} ({} bytes) from {}", file_name, file_size, peer_str);

                self.capabilities.event_bus.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Info,
                    message: format!("接收文件完成: {} ({} bytes) from {}", file_name, file_size, peer_str),
                });
            }
        }
    }
}
