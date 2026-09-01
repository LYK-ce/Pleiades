//Presented by KeJi
//Created Date ： 2026-08-31
//Modified Date ： 2026-08-31

//! UM960 基站驱动（Task 24，terminal 侧）
//!
//! 复用 `spawn_port`（TX + RX 两个 tokio，无第三个）。生命周期状态机
//! `Init → Surveying → Stable → Broadcasting`，共享状态 `Arc<RwLock<Um960Phase>>`。
//! Surveying 失败静默停留（仅 30s 健康日志）；Init 命令每次无条件重发（掉电恢复默认）。

pub mod geo;
pub mod rtcm_parser;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use pleiades_base::network::{NodeHandle, TOPIC_RTK_RTCM};
use pleiades_base::robot::core::protocol::{encode_frame, COMPID_GROUND_STATION, MSGID_RTCM};
use pleiades_base::robot::util::serial::port::spawn_port;

use self::geo::{GgaFix, DEFAULT_GEOID};
use self::rtcm_parser::Rtc3Parser;

/// UM960 生命周期阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Um960Phase {
    Init,
    Surveying,
    Stable,
    Broadcasting,
}

/// Survey-in 期间 RX 回调内部可变状态（不需锁）
struct SurveyState {
    started_at: Instant,
    candidates: Vec<GgaFix>,
    last_quality: Option<u8>,
    last_sats: Option<u8>,
    last_log: Instant,
}

impl SurveyState {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            candidates: Vec::new(),
            last_quality: None,
            last_sats: None,
            last_log: Instant::now(),
        }
    }
}

/// UM960 设备句柄
pub struct Um960Device {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    phase: Arc<RwLock<Um960Phase>>,
    cancel: CancellationToken,
}

// ===== 命令常量 =====

const CMD_UNLOG_ALL: &str = "UNLOG COM1";
const CMD_GGA_OFF: &str = "UNLOG COM1 GPGGA";
const CMD_GGA_ON: &str = "GPGGA COM1 1";
const RTCM_CMDS: [&str; 6] = [
    "RTCM1006 COM1 10",
    "RTCM1033 COM1 10",
    "RTCM1074 COM1 1",
    "RTCM1084 COM1 1",
    "RTCM1094 COM1 1",
    "RTCM1124 COM1 1",
];

fn mode_base_cmd(survey_seconds: u64) -> String {
    format!("MODE BASE TIME {survey_seconds}")
}

fn um960_command_bytes(cmd: &str) -> Vec<u8> {
    format!("{cmd}\r\n").into_bytes()
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

/// 两帧 GGA 是否足够稳定（水平 + 垂直均 < 0.5m）
fn survey_complete(prev: &GgaFix, curr: &GgaFix) -> bool {
    let (e, n, u) = geo::enu(
        prev.lat,
        prev.lon,
        fix_height(prev),
        curr.lat,
        curr.lon,
        fix_height(curr),
    );
    (e * e + n * n).sqrt() < 0.5 && u.abs() < 0.5
}

impl Um960Device {
    /// 启动 UM960 基站驱动。
    ///
    /// - `node_handle.Get_Local_Peer_Id().to_bytes()` 作为 RTCM 帧 `sysid`；
    /// - 复用 `spawn_port`（TX + RX 两个 tokio），RX 回调按 phase 处理（Surveying 按行切 GGA、Broadcasting 喂 Rtc3Parser 切帧广播）；
    /// - 返回后顺序发 Init 三条命令（无条件重发），切 `Surveying`。
    pub fn spawn(
        port: &str,
        baudrate: u32,
        survey_seconds: u64,
        node_handle: Arc<NodeHandle>,
    ) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let phase = Arc::new(RwLock::new(Um960Phase::Init));
        let phase_rx = phase.clone();
        let cmd_tx_shared: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(None));
        let cmd_tx_rx = cmd_tx_shared.clone();
        let node_handle_rx = node_handle.clone();
        let sysid = node_handle.Get_Local_Peer_Id().to_bytes();
        let sysid_rx = sysid.clone();

        // RX 回调可变状态
        let mut line_buf: Vec<u8> = Vec::new();
        let mut survey_state = SurveyState::new();
        let mut rtcm_parser = Rtc3Parser::new();

        let on_bytes = move |bytes: &[u8]| {
            let current = *phase_rx.read().unwrap();
            match current {
                Um960Phase::Surveying => {
                    line_buf.extend_from_slice(bytes);
                    while let Some(pos) = line_buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = line_buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes);
                        let Some(fix) = geo::parse_gga(&line) else {
                            continue;
                        };

                        survey_state.last_quality = Some(fix.quality);
                        survey_state.last_sats = Some(fix.satellites);

                        if fix.quality != 7 {
                            // 非基站原点帧清空候选（保证连续性）
                            survey_state.candidates.clear();
                        } else {
                            survey_state.candidates.push(fix);
                            if survey_state.candidates.len() >= 2
                                && survey_state.started_at.elapsed().as_secs() >= survey_seconds
                            {
                                let n = survey_state.candidates.len();
                                let prev = survey_state.candidates[n - 2].clone();
                                let curr = survey_state.candidates[n - 1].clone();
                                if survey_complete(&prev, &curr) {
                                    // 切 Stable → 关 GGA + 开 RTCM → Broadcasting
                                    *phase_rx.write().unwrap() = Um960Phase::Stable;
                                    if let Some(tx) = cmd_tx_rx.lock().unwrap().as_ref() {
                                        let _ = tx.try_send(um960_command_bytes(CMD_GGA_OFF));
                                        for cmd in RTCM_CMDS {
                                            let _ = tx.try_send(um960_command_bytes(cmd));
                                        }
                                    }
                                    rtcm_parser.reset();
                                    *phase_rx.write().unwrap() = Um960Phase::Broadcasting;
                                    line_buf.clear();
                                    survey_state.candidates.clear();
                                    info!("[UM960] Survey-in 完成，进入 Broadcasting");
                                    break;
                                }
                            }
                        }

                        // 30s 健康日志
                        if survey_state.last_log.elapsed().as_secs() >= 30 {
                            info!(
                                "[UM960] Surveying：已 {}s / 目标 {}s，quality={:?}，sats={:?}",
                                survey_state.started_at.elapsed().as_secs(),
                                survey_seconds,
                                survey_state.last_quality,
                                survey_state.last_sats,
                            );
                            survey_state.last_log = Instant::now();
                        }
                    }
                }
                Um960Phase::Broadcasting => {
                    for rtcm in rtcm_parser.feed(bytes) {
                        let frame =
                            encode_frame(MSGID_RTCM, &sysid_rx, COMPID_GROUND_STATION, &rtcm);
                        if let Err(e) =
                            node_handle_rx.Gossipsub_Publish_Try(TOPIC_RTK_RTCM, frame)
                        {
                            warn!("[UM960] RTCM 广播失败: {e}");
                        }
                    }
                }
                _ => {
                    // Init/Stable：忽略 RX 字节（Stable 期间关 GGA 后可能仍有残留）
                }
            }
        };

        let serial_cmd_tx = spawn_port(port, baudrate, 4096, on_bytes, cancel.clone())?;

        // 回填 TX 句柄（RX 回调切 Stable 时发命令需要）
        *cmd_tx_shared.lock().unwrap() = Some(serial_cmd_tx.clone());

        // Init 三条命令（每次无条件重发：UM960 未 saveconfig，掉电配置恢复默认）
        let _ = serial_cmd_tx.try_send(um960_command_bytes(CMD_UNLOG_ALL));
        let _ = serial_cmd_tx.try_send(um960_command_bytes(&mode_base_cmd(survey_seconds)));
        let _ = serial_cmd_tx.try_send(um960_command_bytes(CMD_GGA_ON));
        *phase.write().unwrap() = Um960Phase::Surveying;
        info!("[UM960] 已进入 Surveying（survey_seconds={survey_seconds}）");

        Ok(Self {
            serial_cmd_tx,
            phase,
            cancel,
        })
    }

    /// 停止驱动（取消 RX/TX task）
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}
