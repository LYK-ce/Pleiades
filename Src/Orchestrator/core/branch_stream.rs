// Presented by KeJi
// Date ： 2026-05-18

//! B3: Stream 入站事件路由。
//!
//! 只做路由 + spawn，不在此执行任何业务 I/O。
//! 文件接收等耗时操作在 spawn 的 task 内完成，避免阻塞 Core 主循环。

use super::Core;
use crate::network::{Network_Inbound_Event, Read_File_Stream_Header, read_session_frame, write_session_frame};
use crate::event_bus::{Bus_Event, NotifyLevel};
use crate::session::capability::SlotHandle;

impl Core {
    /// 路由 Stream 入站事件 (B3)。
    ///
    /// 所有耗时的 I/O 操作通过 `tokio::spawn` 异步化，
    /// 主循环仅负责匹配事件类型并分发，毫秒级返回。
    pub async fn route_stream(&mut self, event: Network_Inbound_Event) {
        match event {
            Network_Inbound_Event::FileStreamArrived { peer, mut stream } => {
                let caps = self.capabilities.clone();
                tokio::spawn(async move {
                    let peer_str = peer.to_base58();
                    tracing::info!("收到入站文件流 from {}", peer_str);

                    let (file_name, file_size, _checksum) = match Read_File_Stream_Header(&mut stream).await {
                        Ok(h) => h,
                        Err(e) => {
                            tracing::error!("读文件流 header 失败 from {}: {}", peer_str, e);
                            return;
                        }
                    };

                    let (dest_path, _guard) = match caps.storage.acquire_write(&file_name).await {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("Storage acquire_write 失败: {}", e);
                            return;
                        }
                    };

                    if let Err(e) = caps.network.receive_file_data(&mut stream, &dest_path, file_size).await {
                        tracing::error!("接收文件数据失败: {}", e);
                        return;
                    }

                    tracing::info!("文件接收完成: {} ({} bytes) from {}", file_name, file_size, peer_str);

                    caps.event_bus.Publish(Bus_Event::Notify {
                        level: NotifyLevel::Info,
                        message: format!("接收文件完成: {} ({} bytes) from {}", file_name, file_size, peer_str),
                    });
                });
            }
            Network_Inbound_Event::SessionStreamArrived { peer, session_id, mut stream } => {
                let session_mgr = self.session_mgr.clone();
                let peer_str = peer.to_base58();
                tracing::info!("收到入站 Session 流 from {}, session={}", peer_str, session_id);

                tokio::spawn(async move {
                    // 1. 分配 slot
                    let handle: SlotHandle = {
                        let mut mgr = session_mgr.lock().unwrap();
                        match mgr.allocate_slot(session_id) {
                            Ok(h) => h,
                            Err(e) => {
                                tracing::error!("Session {} bridge: slot 分配失败: {}", session_id, e);
                                return;
                            }
                        }
                    };

                    let prompt_tx = handle.prompt_tx;
                    let mut token_rx = handle.token_rx;

                    // 2. 双向 bridge: stream ↔ mpsc
                    // libp2p::Stream 不实现 tokio AsyncRead/AsyncWrite，用 Arc<Mutex> 共享
                    let stream = std::sync::Arc::new(tokio::sync::Mutex::new(stream));

                    // prompt 上行: stream → prompt_tx
                    let stream_read = stream.clone();
                    let sid = session_id;
                    let read_task = tokio::spawn(async move {
                        tracing::info!("Session {} bridge: read task started", sid);
                        loop {
                            let mut s = stream_read.lock().await;
                            match read_session_frame(&mut *s).await {
                                Ok(prompt) => {
                                    tracing::info!("Session {} bridge: recv prompt '{}'", sid, prompt);
                                    drop(s);
                                    if prompt_tx.send(prompt).is_err() {
                                        tracing::warn!("Session {} bridge: prompt_tx closed", sid);
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Session {} bridge read error: {}", sid, e);
                                    break;
                                }
                            }
                        }
                        tracing::info!("Session {} bridge: read task ended", sid);
                    });

                    // token 下行: token_rx → stream
                    let stream_write = stream.clone();
                    let write_task = tokio::spawn(async move {
                        tracing::info!("Session {} bridge: write task started", sid);
                        while let Some(token) = token_rx.recv().await {
                            tracing::info!("Session {} bridge: send token '{}'", sid, token);
                            let mut s = stream_write.lock().await;
                            if write_session_frame(&mut *s, &token).await.is_err() {
                                tracing::warn!("Session {} bridge: write error", sid);
                                break;
                            }
                        }
                        tracing::info!("Session {} bridge: write task ended", sid);
                    });

                    // 等任一侧断开后清理
                    tokio::select! {
                        _ = read_task => {}
                        _ = write_task => {}
                    }
                });
            }
        }
    }
}
