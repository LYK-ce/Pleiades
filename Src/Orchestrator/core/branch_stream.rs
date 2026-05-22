// Presented by KeJi
// Date ： 2026-05-18

//! B3: Stream 入站事件路由。
//!
//! 处理来自 Network_Service 的 Network_Inbound_Event（FileStream + TensorStream）。
//! 接收文件由 Rust 直接处理（基础设施操作），不经过 Lua。

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

                self.capabilities.event_bus.Publish(Bus_Event::Notify {
                    level: NotifyLevel::Info,
                    message: format!("接收文件完成: {} ({} bytes) from {}", file_name, file_size, peer_str),
                });

                // _guard drop → 释放写锁，文件注册到 Storage 索引
            }

            Network_Inbound_Event::TensorStreamArrived { peer: _, stream: _ } => {
                tracing::warn!("TensorStreamArrived: 尚未实现");
            }

            Network_Inbound_Event::SessionStreamArrived { peer, stream, session_id } => {
                tracing::info!(
                    "SessionStreamArrived: peer={}, session_id={}",
                    peer, session_id
                );
                let caps = self.capabilities.clone();
                tokio::spawn(async move {
                    let handle = caps.session_manager.clone();
                    // 通过 open_slot_tx 申请 slot
                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                    let _ = handle.open_slot_tx.send(
                        crate::session::manager::OpenSlotRequest {
                            session_id: session_id.clone(),
                            reply_tx,
                        }
                    );
                    match reply_rx.await {
                        Ok(Ok(slot_handle)) => {
                            tracing::info!("Session slot granted: session_id={}", session_id);
                            // spawn 桥接协程
                            spawn_session_bridge(stream, slot_handle);
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("Session slot rejected: session_id={}, error={}", session_id, e);
                        }
                        Err(_) => {
                            tracing::warn!("Session slot request dropped: session_id={}", session_id);
                        }
                    }
                });
            }
        }
    }
}

/// 桥接 libp2p stream ↔ SlotHandle
///
/// 方向 1: stream → prompt（从远端读文本，submit 到 Session Manager）
/// 方向 2: token → stream（从 Session Manager 收 token，写回远端）
fn spawn_session_bridge(stream: libp2p::Stream, handle: crate::session::SlotHandle) {
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use futures::AsyncReadExt;
    use futures::AsyncWriteExt;

    let stream = Arc::new(Mutex::new(stream));
    let sess_id = handle.session_id().to_string();
    let slot_id = handle.slot_id();
    let mut token_rx = handle.take_token_rx();

    // prompt send: 通过 open_slot_tx 直接发
    // But we don't have the tx directly... 
    // The bridge reads from stream and would need to submit prompt.
    // For now, the bridge is just a transport; reading from stream needs
    // a way to push data into the SessionManager's prompt channel.
    // The stream side reads input and should submit via shared_prompt_tx.
    // Since we don't have access to that, we use a simple workaround:
    // the remote side pushes tokens directly into the token_rx for text exchange.
    // 
    // For now: bridge handles token direction only. Prompt direction 
    // will be added when the stream protocol defines in-band prompt format.

    tokio::spawn(async move {
        // 方向 2: token → stream (primary direction)
        loop {
            tokio::select! {
                Some(token) = token_rx.recv() => {
                    let mut s = stream.lock().await;
                    if s.write_all(token.as_bytes()).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
        tracing::info!("Session bridge ended: sess={}, slot={}", sess_id, slot_id);
    });
}
