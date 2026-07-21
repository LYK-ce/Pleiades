//Presented by KeJi
//Created Date ： 2026-07-20
//Modified Date ： 2026-07-20

//! YDLIDAR Tmini 设备驱动
//!
//! LidarDevice — 封装 tmini 协议状态机 + async 串口 TX/RX。
//! 复用 Orion 的 spawn_port() 通用串口抽象，RX 回调使用 tmini 协议解析，
//! 点云写入 LidarState.scan。

pub mod constants;
pub mod types;
pub mod parser;
pub mod checksum;

// Re-export 核心类型
pub use types::{LaserPoint, LaserScan};

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use constants::*;
use parser::{feed_byte, parse_points, do_process_simple, ParseState};
use types::ScanPacket;
use crate::robot::control::serial::port;
use crate::robot::state::LidarState;

// ============================================================
// LidarDevice — 控制句柄
// ============================================================

pub struct LidarDevice {
    cmd_tx: mpsc::Sender<Vec<u8>>,
    cancel: CancellationToken,
}

impl LidarDevice {
    /// 启动 LiDAR 设备：内部 spawn TX + RX tokio task
    ///
    /// RX 回调使用 tmini 协议状态机解析数据，
    /// 收到完整包后解码为 LaserScan 并通过 try_write() 写回共享 state。
    pub fn spawn(
        port_path: &str,
        baudrate: u32,
        state: Arc<RwLock<LidarState>>,
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let mut sm = ParseState::new();

        let cmd_tx = port::spawn_port(
            port_path,
            baudrate,
            4096, // LiDAR 点云包可达 ~250B，4K 够用
            move |bytes| {
                for &b in bytes {
                    if feed_byte(&mut sm, b).is_some() {
                        // 有完整包到达
                        let packets: Vec<ScanPacket> = sm
                            .take_packets()
                            .into_iter()
                            .filter(|p| {
                                p.raw.len() >= TRI_PACKHEADSIZE
                                    && p.raw[0] == PH1
                                    && p.raw[1] == PH2
                            })
                            .collect();

                        if !packets.is_empty() {
                            let nodes = parse_points(&packets, NODE_QUAL8);
                            if !nodes.is_empty() {
                                let scan = do_process_simple(
                                    &nodes, 0.0, false, 0, 360.0,
                                );
                                // info!(
                                //     "[LiDAR] scan: {} pts, freq={:.1}Hz",
                                //     scan.points.len(),
                                //     scan.scan_freq
                                // );
                                if let Ok(mut guard) = state_clone.try_write() {
                                    guard.scan = Some(scan);
                                } else {
                                    warn!("[LiDAR] try_write 失败，扫描结果丢弃");
                                }
                            }
                        }
                    }
                }
            },
            cancel.clone(),
        )?;

        Ok(Self { cmd_tx, cancel })
    }

    /// 发送扫描启动命令 (PHA5 + CMD_SCAN)
    pub async fn start_scan(&self) -> Result<(), String> {
        self.cmd_tx
            .send(vec![PHA5, CMD_SCAN])
            .await
            .map_err(|_| "TX channel 已关闭".into())
    }

    /// 发送停止命令 (PHA5 + CMD_FORCE_STOP → PHA5 + CMD_STOP)
    pub async fn stop_scan(&self) -> Result<(), String> {
        self.cmd_tx
            .send(vec![PHA5, CMD_FORCE_STOP])
            .await
            .map_err(|_| "TX channel 已关闭".to_string())?;
        self.cmd_tx
            .send(vec![PHA5, CMD_STOP])
            .await
            .map_err(|_| "TX channel 已关闭".to_string())
    }

    /// 优雅退出：cancel TX + RX tokio task
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

// ============================================================
// Mock spawn（测试用）
// ============================================================

#[cfg(test)]
impl LidarDevice {
    /// 创建模拟设备：不依赖真实串口，通过 channel 模拟 TX/RX。
    ///
    /// - `rx_feed`: 向此 Receiver 发送 Vec<u8> 可模拟串口收到数据
    /// - `tx_sink`: 设备发出的命令字节会发送到此 Sender
    ///
    /// 返回设备句柄 + 后台 task JoinHandle。
    pub fn spawn_mock(
        mut rx_feed: mpsc::Receiver<Vec<u8>>,
        tx_sink: mpsc::Sender<Vec<u8>>,
        state: Arc<RwLock<LidarState>>,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        use parser::{feed_byte, parse_points, do_process_simple, ParseState};
        use types::ScanPacket;

        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<Vec<u8>>(32);
        let mock_cancel = cancel.clone();

        let handle = tokio::spawn(async move {
            let mut sm = ParseState::new();

            loop {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(bytes) => { let _ = tx_sink.send(bytes).await; }
                            None => break,
                        }
                    }
                    rx = rx_feed.recv() => {
                        match rx {
                            Some(bytes) => {
                                let mut got_scan = false;
                                let mut latest_scan = None;
                                for &b in &bytes {
                                    if feed_byte(&mut sm, b).is_some() {
                                        let packets: Vec<ScanPacket> = sm
                                            .take_packets()
                                            .into_iter()
                                            .filter(|p| {
                                                p.raw.len() >= TRI_PACKHEADSIZE
                                                    && p.raw[0] == PH1
                                                    && p.raw[1] == PH2
                                            })
                                            .collect();
                                        if !packets.is_empty() {
                                            let nodes = parse_points(&packets, NODE_QUAL8);
                                            if !nodes.is_empty() {
                                                latest_scan = Some(do_process_simple(
                                                    &nodes, 0.0, false, 0, 360.0,
                                                ));
                                                got_scan = true;
                                            }
                                        }
                                    }
                                }
                                if got_scan {
                                    if let Ok(mut guard) = state_clone.try_write() {
                                        guard.scan = latest_scan;
                                    } else {
                                        warn!("[LiDAR mock] try_write 失败，扫描结果丢弃");
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                    _ = mock_cancel.cancelled() => break,
                }
            }
        });

        (Self { cmd_tx, cancel }, handle)
    }
}
