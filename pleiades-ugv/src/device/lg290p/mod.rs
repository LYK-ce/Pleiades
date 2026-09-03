//Presented by KeJi
//Created Date ： 2026-08-31
//Modified Date ： 2026-09-03

//! LG290P 流动站驱动（Task 24，ugv 侧）
//!
//! 3-task：① TX（spawn_port）② RX（spawn_port，on_bytes 读 GGA）③ robot_bus 订阅（RTCM 写串口）。
//! 2 态状态机：`WaitingFirstFix → Tracking`（仅 RX 回调内部使用，无锁）。
//! 定位：offset 更新 `RobotState.x/y`（z 不更新 D11、yaw 不动）；`rtk_fixed` 实时写（FIXED true / 失锁 false）。

pub mod geo;

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use pleiades_base::event_bus::{Bus_Event, EventBus};
use pleiades_base::robot::core::protocol::{decode_frame, MSGID_RTCM};
use pleiades_base::robot::core::state::RobotState;
use pleiades_base::robot::util::serial::port::spawn_port;

use self::geo::{GgaFix, DEFAULT_GEOID};

/// LG290P 定位状态机（2 态，仅 RX 回调内部使用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lg290pPhase {
    /// 等首次 RTK_FIXED（约 10s~几分钟），期间不更新 x/y，只写 rtk_fixed
    WaitingFirstFix,
    /// 已记录基准，进入 offset 更新
    Tracking,
}

/// LG290P 设备句柄
pub struct Lg290pDevice {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    cancel: CancellationToken,
}

/// 椭球高 = MSL + geoid（geoid 非有限用默认）
fn fix_height(fix: &GgaFix) -> f64 {
    fix.alt_msl
        + if fix.geoid.is_finite() {
            fix.geoid
        } else {
            DEFAULT_GEOID
        }
}

/// 处理一行 GGA（同步，跑在 on_bytes 里）。
///
/// - FIXED（quality==4）：WaitingFirstFix 记录基准 + 切 Tracking + 写 rtk_fixed=true；
///   Tracking 时 enu 差分 → offset 更新 x/y（y 取反、z 不更新、yaw 不动）+ rtk_fixed=true。
/// - 失锁/非 FIXED：x/y 保持，rtk_fixed=false。
fn handle_gga_line(
    fix: &GgaFix,
    phase: &mut Lg290pPhase,
    base: &mut Option<GgaFix>,
    state: &Arc<RwLock<RobotState>>,
    origin: (f32, f32, f32),
) {
    if fix.quality == 4 {
        let height = fix_height(fix);
        match *phase {
            Lg290pPhase::WaitingFirstFix => {
                // 基准 = 车首次 FIXED 位置（D10，不需要基站坐标下行）
                *base = Some(fix.clone());
                *phase = Lg290pPhase::Tracking;
                info!(
                    "[LG290P] 首次 RTK_FIXED，offset 基准已确定（lat={:.8}, lon={:.8}），可动车",
                    fix.lat, fix.lon
                );
                if let Ok(mut guard) = state.try_write() {
                    guard.rtk_fixed = true;
                } else {
                    warn!("[LG290P] try_write 失败，rtk_fixed 更新丢弃");
                }
            }
            Lg290pPhase::Tracking => {
                let b = base.as_ref().expect("Tracking 状态 base 应已记录");
                let (e, n, _u) = geo::enu(b.lat, b.lon, fix_height(b), fix.lat, fix.lon, height);
                if let Ok(mut guard) = state.try_write() {
                    guard.x = origin.0 + e as f32;
                    guard.y = origin.1 - n as f32; // y 取反（ENU N 北为正 → 世界 y 南为正）
                    guard.rtk_fixed = true;
                    // z 不更新（D11）、yaw 不动
                } else {
                    warn!("[LG290P] try_write 失败，位置更新丢弃");
                }
            }
        }
    } else {
        // 失锁/非 FIXED：位置保持，rtk_fixed 实时反映 false
        if let Ok(mut guard) = state.try_write() {
            guard.rtk_fixed = false;
        } else {
            warn!("[LG290P] try_write 失败，rtk_fixed 更新丢弃");
        }
    }
}

/// task③：订阅 robot_bus → RTCM 帧（msgid==6）→ 写串口喂 LG290P。
async fn rtcm_relay_loop(
    mut rx: broadcast::Receiver<Bus_Event>,
    tx: mpsc::Sender<Vec<u8>>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            msg = rx.recv() => {
                match msg {
                    Ok(Bus_Event::StreamRaw { payload }) => {
                        if let Some(frame) = decode_frame(&payload) {
                            if frame.msgid == MSGID_RTCM {
                                let _ = tx.try_send(frame.payload);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        warn!("[LG290P] robot_bus 落后，跳过");
                    }
                }
            }
        }
    }
}

impl Lg290pDevice {
    /// 启动 LG290P 流动站驱动。
    ///
    /// - RX 回调按行切 GGA → `parse_gga` → FIXED 才 offset 更新；
    /// - 第 3 个 task 订阅 robot_bus 取 RTCM 帧写串口；
    /// - LG290P 默认 Rover 模式，不发送接收机模式配置。
    pub fn spawn(
        port: &str,
        baudrate: u32,
        robot_bus: Arc<EventBus>,
        state: Arc<RwLock<RobotState>>,
        origin: (f32, f32, f32),
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let state_clone = state.clone();

        // RX 回调可变状态
        let mut phase = Lg290pPhase::WaitingFirstFix;
        let mut base: Option<GgaFix> = None;
        let mut line_buf: Vec<u8> = Vec::new();

        let mut last_log = Instant::now();
        let on_bytes = move |bytes: &[u8]| {
            line_buf.extend_from_slice(bytes);
            while let Some(pos) = line_buf.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = line_buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line_bytes);
                if let Some(fix) = geo::parse_gga(&line) {
                    // 30s 健康日志（调试：区分「收不到 GGA」与「还没 FIXED」）
                    if last_log.elapsed().as_secs() >= 30 {
                        info!(
                            "[LG290P] 状态：quality={}, sats={}, phase={:?}",
                            fix.quality, fix.satellites, phase
                        );
                        last_log = Instant::now();
                    }
                    handle_gga_line(&fix, &mut phase, &mut base, &state_clone, origin);
                }
            }
        };

        let serial_cmd_tx = spawn_port(port, baudrate, 512, on_bytes, cancel.clone())?;

        // task③：robot_bus 订阅 → RTCM 帧写串口
        tokio::spawn(rtcm_relay_loop(
            robot_bus.Subscribe(),
            serial_cmd_tx.clone(),
            cancel.clone(),
        ));

        Ok(Self {
            serial_cmd_tx,
            cancel,
        })
    }

    /// 停止驱动（取消 RX/TX/relay task）
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}
