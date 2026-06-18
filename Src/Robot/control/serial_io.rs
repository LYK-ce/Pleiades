//Presented by KeJi
//Date ： 2026-06-17

//! 串口 I/O 层
//!
//! 基于 tokio-serial 实现异步串口读写。
//! 内部 spawn 后台任务持续读取串口数据，通过 RxStateMachine 解析帧，
//! 结果写入共享的 RobotState 缓存。

use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use super::protocol::{self, build_host_frame, RxStateMachine};
use super::types::RobotState;

// ============================================================
// SerialIo
// ============================================================

/// 串口 I/O 句柄
pub struct SerialIo {
    /// 共享串口（Mutex 保护读写互斥）
    port: Arc<Mutex<SerialStream>>,
    /// 传感器状态缓存（后台任务持续更新）
    state: Arc<RwLock<RobotState>>,
}

impl SerialIo {
    /// 打开串口并启动后台接收任务。
    pub fn open(port_name: &str, baudrate: u32) -> Result<Self, String> {
        let port = tokio_serial::new(port_name, baudrate)
            .open_native_async()
            .map_err(|e| format!("无法打开串口 {}: {}", port_name, e))?;

        let port = Arc::new(Mutex::new(port));
        let state = Arc::new(RwLock::new(RobotState::default()));

        // spawn 后台接收任务
        let rx_port = port.clone();
        let rx_state = state.clone();
        tokio::spawn(async move {
            Self::receive_loop(rx_port, rx_state).await;
        });

        Ok(Self { port, state })
    }

    /// 发送一帧到 STM32
    pub async fn send_frame(&self, func: u8, data: &[u8]) -> Result<(), String> {
        let frame = build_host_frame(func, data);
        let mut port = self.port.lock().await;
        port.write_all(&frame)
            .await
            .map_err(|e| format!("串口写入失败: {}", e))?;
        port.flush()
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

    async fn receive_loop(port: Arc<Mutex<SerialStream>>, state: Arc<RwLock<RobotState>>) {
        let mut sm = RxStateMachine::new();

        loop {
            // 逐字节读取（和 Python ser.read(1) 一样）
            let mut byte = [0u8; 1];
            let result = {
                let mut p = port.lock().await;
                tokio::io::AsyncReadExt::read(&mut *p, &mut byte).await
            };

            match result {
                Ok(0) => {
                    // 超时或无数据，短暂休眠避免忙等
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                Ok(_) => {
                    if let Some((func, data)) = sm.feed(byte[0]) {
                        // 解析成功，更新状态缓存
                        let mut s = state.write().await;
                        protocol::update_state(&mut s, func, &data);
                    }
                }
                Err(e) => {
                    tracing::warn!("串口读取错误: {}", e);
                    // 短暂休眠后重试
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
        }
    }
}
