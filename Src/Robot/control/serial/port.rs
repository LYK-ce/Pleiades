//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! 通用串口设备驱动
//!
//! 提供 `spawn_port` 通用函数。
//! 每个设备负责 pack（命令→字节），spawn_port 只负责收发。
//! RX 方向通过回调直接处理，不经过中间事件通道。

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use serial2_tokio::SerialPort;

// ============================================================
// spawn_port — 通用端口启动
// ============================================================

/// 启动一个串口设备，返回字节发送器。
///
/// 内部创建 TX + RX 两个 tokio task，共享同一个 `Arc<SerialPort>`。
/// RX 收到字节后直接调用 `on_bytes` 回调处理。
///
/// 返回 `mpsc::Sender<Vec<u8>>`：发送字节（设备自己 pack）。
pub fn spawn_port(
    port: &str,
    baudrate: u32,
    read_buf_size: usize,
    on_bytes: impl FnMut(&[u8]) + Send + 'static,
    cancel: CancellationToken,
) -> Result<mpsc::Sender<Vec<u8>>, String>
{
    let serial = SerialPort::open(port, baudrate)
        .map_err(|e| format!("无法打开串口 {}: {}", port, e))?;
    let serial = Arc::new(serial);

    let (cmd_tx, cmd_rx) = mpsc::channel::<Vec<u8>>(32);

    // TX task
    let tx_port = serial.clone();
    let tx_cancel = cancel.clone();
    tokio::spawn(async move { tx_loop(tx_port, cmd_rx, tx_cancel).await });

    // RX task
    let rx_port = serial.clone();
    let rx_cancel = cancel.clone();
    tokio::spawn(async move { rx_loop(rx_port, on_bytes, read_buf_size, rx_cancel).await });

    Ok(cmd_tx)
}

// ============================================================
// TX Loop
// ============================================================

async fn tx_loop(
    port: Arc<SerialPort>,
    mut cmd_rx: mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
) {
    loop {
        select! {
            _ = cancel.cancelled() => {
                info!("TX task 退出");
                return;
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(bytes) => {
                        if let Err(e) = port.write(&bytes).await {
                            error!("串口写入失败: {e}");
                            return;
                        }
                    }
                    None => {
                        info!("TX channel 关闭");
                        return;
                    }
                }
            }
        }
    }
}

// ============================================================
// RX Loop
// ============================================================

async fn rx_loop(
    port: Arc<SerialPort>,
    mut on_bytes: impl FnMut(&[u8]) + Send + 'static,
    read_buf_size: usize,
    cancel: CancellationToken,
) {
    let mut buf = vec![0u8; read_buf_size];
    loop {
        select! {
            _ = cancel.cancelled() => {
                info!("RX task 退出");
                return;
            }
            result = port.read(&mut buf) => {
                match result {
                    Ok(0) => continue,
                    Ok(n) => on_bytes(&buf[..n]),
                    Err(e) => {
                        error!("串口读取失败: {e}");
                        return;
                    }
                }
            }
        }
    }
}
