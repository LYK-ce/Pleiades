// Presented by KeJi
// Date ： 2026-05-18

//! B3: Stream 入站事件路由。
//!
//! 处理来自 Network_Service 的 Network_Inbound_Event（FileStream + TensorStream）。
//! 接收文件由 Rust 直接处理（基础设施操作），不经过 Lua。

use super::Core;
use crate::network::{Network_Inbound_Event, Read_File_Stream_Header};
use crate::event_bus::event::Bus_Event;

impl Core {
    /// 路由 Stream 入站事件 (B3)。
    pub async fn route_stream(&mut self, event: Network_Inbound_Event) {
        match event {
            Network_Inbound_Event::FileStreamArrived { peer, mut stream } => {
                let peer_str = peer.to_base58();
                tracing::info!("收到入站文件流 from {}", peer_str);

                // 1. 读取 in-band header
                let (file_name, file_size, checksum) = match Read_File_Stream_Header(&mut stream).await {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::error!("读文件流 header 失败 from {}: {}", peer_str, e);
                        return;
                    }
                };

                tracing::info!(
                    "文件流 header: name={}, size={}, checksum={}",
                    file_name, file_size, if checksum.is_empty() { "(none)" } else { &checksum }
                );

                // 2. 获取写锁 + 目标路径
                let (dest_path, _guard) = match self.capabilities.storage.acquire_write(&file_name).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("Storage acquire_write 失败: {}", e);
                        return;
                    }
                };

                // 3. 接收文件数据
                if let Err(e) = self.capabilities.network.receive_file_data(&mut stream, &dest_path, file_size).await {
                    tracing::error!("接收文件数据失败: {}", e);
                    return;
                }

                tracing::info!("文件接收完成: {} ({} bytes) from {}", file_name, file_size, peer_str);

                self.capabilities.event_bus.Publish(Bus_Event::Log {
                    message: format!("接收文件完成: {} ({} bytes) from {}", file_name, file_size, peer_str),
                });

                // _guard drop → 释放写锁，文件注册到 Storage 索引
            }

            Network_Inbound_Event::TensorStreamArrived { peer: _, stream: _ } => {
                tracing::warn!("TensorStreamArrived: 尚未实现");
            }
        }
    }
}
