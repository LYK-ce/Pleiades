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
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

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
        let mut packet_buf: Vec<ScanPacket> = Vec::with_capacity(32);
        let mut last_zero = Instant::now();

        let cmd_tx = port::spawn_port(
            port_path,
            baudrate,
            4096, // LiDAR 点云包可达 ~250B，4K 够用
            move |bytes| {
                for &b in bytes {
                    if feed_byte(&mut sm, b).is_some() {
                        // 有完整包到达，累积到 buffer
                        let mut got_zero = false;
                        for pkt in sm.take_packets() {
                            if pkt.raw.len() >= TRI_PACKHEADSIZE
                                && pkt.raw[0] == PH1
                                && pkt.raw[1] == PH2
                            {
                                if pkt.zero {
                                    got_zero = true;
                                }
                                packet_buf.push(pkt);
                            }
                        }

                        // 零位包到达 → 累积的包 = 完整一圈 → 组装全帧输出
                        if got_zero && !packet_buf.is_empty() {
                            let nodes = parse_points(&packet_buf, NODE_QUAL8);
                            if !nodes.is_empty() {
                                let scan = do_process_simple(
                                    &nodes, 0.0, false, 0, 360.0,
                                );
                                if let Ok(mut guard) = state_clone.try_write() {
                                    guard.scan = Some(scan);
                                } else {
                                    warn!("[LiDAR] try_write 失败，扫描结果丢弃");
                                }
                            }
                            packet_buf.clear();
                            last_zero = Instant::now();
                        }

                        // 超时保护：2 秒未收到零位包，清空缓存
                        if last_zero.elapsed() > Duration::from_secs(2) {
                            packet_buf.clear();
                            last_zero = Instant::now();
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
        info!("[LiDAR] 发送启动扫描命令 CMD_SCAN");
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
            let mut packet_buf: Vec<ScanPacket> = Vec::with_capacity(32);
            let mut last_zero = Instant::now();

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
                                for &b in &bytes {
                                    if feed_byte(&mut sm, b).is_some() {
                                        let mut got_zero = false;
                                        for pkt in sm.take_packets() {
                                            if pkt.raw.len() >= TRI_PACKHEADSIZE
                                                && pkt.raw[0] == PH1
                                                && pkt.raw[1] == PH2
                                            {
                                                if pkt.zero { got_zero = true; }
                                                packet_buf.push(pkt);
                                            }
                                        }
                                        if got_zero && !packet_buf.is_empty() {
                                            let nodes = parse_points(&packet_buf, NODE_QUAL8);
                                            if !nodes.is_empty() {
                                                let scan = do_process_simple(
                                                    &nodes, 0.0, false, 0, 360.0,
                                                );
                                                if let Ok(mut guard) = state_clone.try_write() {
                                                    guard.scan = Some(scan);
                                                } else {
                                                    warn!("[LiDAR mock] try_write 失败，扫描结果丢弃");
                                                }
                                            }
                                            packet_buf.clear();
                                            last_zero = Instant::now();
                                        }
                                        if last_zero.elapsed() > Duration::from_secs(2) {
                                            packet_buf.clear();
                                            last_zero = Instant::now();
                                        }
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
