//Presented by KeJi
//Date ： 2026-04-10

//! 文件传输管理器
//!
//! 从 node.rs 中提取出的独立组件，负责：
//! 1. **流式传输控制**：持有 `stream::Control` 句柄，管理流式传输协议的注册与连接
//! 2. **文件保存**：管理文件保存目录
//! 3. **发送文件流**：在独立任务中打开流并发送文件
//! 4. **接收文件流**：在独立任务中接收流并保存文件
//!
//! Network_Service 通过持有 `File_Transfer_Manager` 实例来使用这些功能。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use libp2p::{PeerId, StreamProtocol};
use libp2p_stream as stream;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info};

use super::network_service::NetworkEvent;
use super::stream_protocol::{
    FILE_STREAM_PROTOCOL, Receive_File_Stream, Send_File_Stream,
};

// ============================================================
// File_Transfer_Manager
// ============================================================

/// 文件传输管理器
///
/// 管理流式传输控制句柄和文件保存目录，提供：
/// - `Accept_Incoming`: 注册流式传输协议，返回入站流迭代器
/// - `Spawn_Send`: 在独立任务中发送文件流
/// - `Spawn_Receive`: 在独立任务中接收文件流
pub struct File_Transfer_Manager {
    /// 流式传输控制句柄（可 Clone）
    stream_control: stream::Control,
    /// 文件保存目录
    save_dir: PathBuf,
}

impl File_Transfer_Manager {
    /// 创建新的 File_Transfer_Manager
    ///
    /// # 参数
    /// - `stream_control`: libp2p-stream 的 Control 句柄
    /// - `save_dir`: 接收文件的保存目录
    pub fn New(stream_control: stream::Control, save_dir: PathBuf) -> Self {
        Self {
            stream_control,
            save_dir,
        }
    }

    /// 注册流式传输协议并返回入站流迭代器
    ///
    /// 在 Network_Service::Start() 中调用，获取 IncomingStreams 用于 select! 监听入站流。
    ///
    /// # Returns
    /// `stream::IncomingStreams` — 入站流迭代器
    ///
    /// # Panics
    /// 如果协议注册失败则 panic
    pub fn Accept_Incoming(&mut self) -> stream::IncomingStreams {
        self.stream_control
            .accept(StreamProtocol::new(FILE_STREAM_PROTOCOL))
            .expect("流式传输协议注册失败")
    }

    /// 在独立任务中发送文件流
    ///
    /// 打开到目标节点的流式连接，发送文件数据。
    /// 传输完成后通过 `completion_tx` 通知调用方。
    /// 进度通过 `event_sender` 上报。
    ///
    /// # 参数
    /// - `peer`: 目标节点 ID
    /// - `file_path`: 待发送文件路径
    /// - `completion_tx`: 可选的完成通知通道
    /// - `event_sender`: 网络事件发送器（用于上报进度和错误）
    pub fn Spawn_Send(
        &self,
        peer: PeerId,
        file_path: PathBuf,
        completion_tx: Option<oneshot::Sender<Result<(), String>>>,
        event_sender: mpsc::Sender<NetworkEvent>,
    ) {
        let mut control = self.stream_control.clone();
        let protocol = StreamProtocol::new(FILE_STREAM_PROTOCOL);

        // 在独立任务中处理流式发送，避免阻塞事件循环
        tokio::spawn(async move {
            let result = match control.open_stream(peer, protocol).await {
                Ok(mut stream) => {
                    match Send_File_Stream(&mut stream, &file_path, event_sender.clone(), peer).await {
                        Ok(()) => {
                            info!("流式文件发送完成: {} -> {}", file_path.display(), peer);
                            Ok(())
                        }
                        Err(e) => {
                            error!("流式文件发送失败: {} -> {}: {}", file_path.display(), peer, e);
                            let _ = event_sender
                                .send(NetworkEvent::FileStreamError {
                                    peer,
                                    error: e.to_string(),
                                })
                                .await;
                            Err(e.to_string())
                        }
                    }
                }
                Err(e) => {
                    error!("打开流式传输连接失败 -> {}: {}", peer, e);
                    let _ = event_sender
                        .send(NetworkEvent::FileStreamError {
                            peer,
                            error: e.to_string(),
                        })
                        .await;
                    Err(e.to_string())
                }
            };
            // 通知调用方传输完成
            if let Some(tx) = completion_tx {
                let _ = tx.send(result);
            }
        });
    }

    /// 在独立任务中接收文件流
    ///
    /// 从入站流中读取文件数据并保存到 `save_dir`。
    /// 完成后通过 `event_sender` 上报 `FileStreamReceived` 或 `FileStreamError`。
    ///
    /// # 参数
    /// - `peer_id`: 发送方节点 ID
    /// - `stream`: 入站的 libp2p::Stream
    /// - `event_sender`: 网络事件发送器（用于上报进度和结果）
    pub fn Spawn_Receive(
        &self,
        peer_id: PeerId,
        mut stream: libp2p::Stream,
        event_sender: mpsc::Sender<NetworkEvent>,
    ) {
        let save_dir = self.save_dir.clone();

        // 在独立任务中处理流式接收，避免阻塞事件循环
        tokio::spawn(async move {
            match Receive_File_Stream(&mut stream, &save_dir, event_sender.clone(), peer_id).await {
                Ok(file_path) => {
                    info!("流式文件接收完成: {} from {}", file_path.display(), peer_id);
                    let _ = event_sender
                        .send(NetworkEvent::FileStreamReceived {
                            peer: peer_id,
                            file_path,
                        })
                        .await;
                }
                Err(e) => {
                    error!("流式文件接收失败 from {}: {}", peer_id, e);
                    let _ = event_sender
                        .send(NetworkEvent::FileStreamError {
                            peer: peer_id,
                            error: e.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    /// 获取文件保存目录的引用
    pub fn Get_Save_Dir(&self) -> &PathBuf {
        &self.save_dir
    }
}
