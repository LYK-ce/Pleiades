//Presented by KeJi
//Date ： 2026-06-17

//! 串口 I/O 层
//!
//! 基于 tokio-serial 实现异步串口读写。
//! 用 tokio::io::split 分离读写通道，避免 Mutex 竞争。
//! 后台任务持续读取，结果写入共享 RobotState 缓存。

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use super::protocol::{self, build_host_frame, RxStateMachine};
use super::types::RobotState;

// ============================================================
// SerialIo
// ============================================================

/// 串口 I/O 句柄
pub struct SerialIo {
    /// 写半端（独立于读半端，无需 Mutex）
    writer: Arc<tokio::sync::Mutex<tokio::io::WriteHalf<SerialStream>>>,
    /// 传感器状态缓存（后台任务持续更新）
    state: Arc<RwLock<RobotState>>,
}

impl SerialIo {
    /// 打开串口并启动后台接收任务。
    pub fn open(port_name: &str, baudrate: u32) -> Result<Self, String> {
        let port = tokio_serial::new(port_name, baudrate)
            .timeout(std::time::Duration::from_millis(10))
            .open_native_async()
            .map_err(|e| format!("无法打开串口 {}: {}", port_name, e))?;

        let (reader, writer) = tokio::io::split(port);
        let writer = Arc::new(tokio::sync::Mutex::new(writer));
        let state = Arc::new(RwLock::new(RobotState::default()));

        // spawn 后台接收任务（独占 reader）
        let rx_state = state.clone();
        tokio::spawn(async move {
            Self::receive_loop(reader, rx_state).await;
        });

        Ok(Self { writer, state })
    }

    /// 发送一帧到 STM32
    pub async fn send_frame(&self, func: u8, data: &[u8]) -> Result<(), String> {
        let frame = build_host_frame(func, data);
        let mut w = self.writer.lock().await;
        w.write_all(&frame)
            .await
            .map_err(|e| format!("串口写入失败: {}", e))?;
        w.flush()
            .await
            .map_err(|e| format!("串口 flush 失败: {}", e))?;
        Ok(())
    }

    /// 获取传感器状态快照
    pub async fn get_state(&self) -> RobotState {
        self.state.read().await.clone()
    }

    // ============================================================
    // 后台接收循环
    // ============================================================

    async fn receive_loop(
        mut reader: tokio::io::ReadHalf<SerialStream>,
        state: Arc<RwLock<RobotState>>,
    ) {
        let mut sm = RxStateMachine::new();

        loop {
            let mut byte = [0u8; 1];
            match reader.read(&mut byte).await {
                Ok(0) => {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                Ok(_) => {
                    if let Some((func, data)) = sm.feed(byte[0]) {
                        let mut s = state.write().await;
                        protocol::update_state(&mut s, func, &data);
                    }
                }
                Err(e) => {
                    tracing::warn!("串口读取错误: {}", e);
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
        }
    }
}
