//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! STM32 控制板设备驱动
//!
//! 包含：协议常量、帧构建、接收状态机、传感器解析、命令打包。

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use super::super::serial::port;
use super::super::types::CarType;
use crate::robot::state::RobotState;
use tracing::{debug, info};

// ============================================================
// 运动状态（FUNC_CAR_RUN 协议专属）
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum MotionState {
    Stop = 0,
    Forward = 1,
    Backward = 2,
    Left = 3,
    Right = 4,
    SpinLeft = 5,
    SpinRight = 6,
    Park = 7,
}

// ============================================================
// 协议常量
const HEAD: u8 = 0xFF;
const DEV_ID_HOST: u8 = 0xFC;
const DEV_ID_STM32: u8 = 0xFB;
const COMPLEMENT: u8 = 5;

const FUNC_BEEP: u8 = 0x02;
const FUNC_PWM_SERVO: u8 = 0x03;
const FUNC_RGB: u8 = 0x05;
const FUNC_RGB_EFFECT: u8 = 0x06;
const FUNC_MOTOR: u8 = 0x10;
const FUNC_CAR_RUN: u8 = 0x11;
const FUNC_MOTION: u8 = 0x12;
const FUNC_RESET_STATE: u8 = 0x0F;

const RPT_SPEED: u8 = 0x0A;
const RPT_MPU_RAW: u8 = 0x0B;
const RPT_IMU_ATT: u8 = 0x0C;
const RPT_ENCODER: u8 = 0x0D;
const RPT_ICM_RAW: u8 = 0x0E;

// ============================================================
// 帧构建
// ============================================================

fn build_host_frame(func: u8, data: &[u8]) -> Vec<u8> {
    let mut frame = vec![HEAD, DEV_ID_HOST, 0, func];
    frame.extend_from_slice(data);
    frame[2] = (frame.len() - 1) as u8;
    let csum = (frame.iter().map(|&b| b as u32).sum::<u32>() + COMPLEMENT as u32) as u8;
    frame.push(csum);
    frame
}

// ============================================================
// 命令打包
// ============================================================

fn pack_car_run(car_type: u8, state: u8, speed: i16) -> Vec<u8> {
    let s = speed.to_le_bytes();
    build_host_frame(FUNC_CAR_RUN, &[car_type, state, s[0], s[1]])
}

fn pack_motion(car_type: u8, vx: f32, vy: f32, vz: f32, limits: (f32, f32, f32)) -> Vec<u8> {
    let (vx_max, vy_max, vz_max) = limits;
    let vx = (vx.clamp(-vx_max, vx_max) * 1000.0) as i16;
    let vy = (vy.clamp(-vy_max, vy_max) * 1000.0) as i16;
    let vz = (vz.clamp(-vz_max, vz_max) * 1000.0) as i16;
    let vx_b = vx.to_le_bytes(); let vy_b = vy.to_le_bytes(); let vz_b = vz.to_le_bytes();
    build_host_frame(FUNC_MOTION, &[car_type, vx_b[0], vx_b[1], vy_b[0], vy_b[1], vz_b[0], vz_b[1]])
}

fn pack_motor(m1: i8, m2: i8, m3: i8, m4: i8) -> Vec<u8> {
    let c = |v: i8| if v == 127 { 127u8 } else { v.clamp(-100, 100) as u8 };
    build_host_frame(FUNC_MOTOR, &[c(m1), c(m2), c(m3), c(m4)])
}

fn pack_beep(duration_ms: u16) -> Vec<u8> {
    let b = duration_ms.to_le_bytes();
    build_host_frame(FUNC_BEEP, &[b[0], b[1]])
}

fn pack_servo(servo_id: u8, angle: u8) -> Vec<u8> {
    build_host_frame(FUNC_PWM_SERVO, &[servo_id, angle.min(180)])
}

fn pack_rgb(led_id: u8, r: u8, g: u8, b: u8) -> Vec<u8> {
    build_host_frame(FUNC_RGB, &[led_id, r, g, b])
}

fn pack_rgb_effect(effect: u8, speed: u8) -> Vec<u8> {
    build_host_frame(FUNC_RGB_EFFECT, &[effect, speed, 0xFF])
}

fn pack_reset() -> Vec<u8> {
    build_host_frame(FUNC_RESET_STATE, &[0x5F])
}

// ============================================================
// 接收状态机
// ============================================================

enum RxState {
    Head,
    Data { idx: u8, expected_len: u8, func: u8, data: Vec<u8> },
}

fn feed_state_machine(sm: &mut RxState, byte: u8) -> Option<(u8, Vec<u8>)> {
    let mut current = RxState::Head;
    std::mem::swap(sm, &mut current);
    let result = match &mut current {
        RxState::Head => {
            if byte == HEAD { *sm = RxState::Data { idx: 0, expected_len: 0, func: 0, data: Vec::new() }; }
            else { *sm = current; }
            None
        }
        RxState::Data { idx, expected_len, func, data } => {
            let mut outcome = None;
            match *idx {
                0 => { if byte != DEV_ID_STM32 { *sm = RxState::Head; return None; } }
                1 => *expected_len = byte,
                2 => *func = byte,
                n if n < *expected_len => { data.push(byte); }
                n if n == *expected_len => {
                    let chk = byte;
                    let sum = (*expected_len).wrapping_add(*func)
                        .wrapping_add(data.iter().fold(0u8, |a, &b| a.wrapping_add(b)));
                    outcome = if sum == chk { Some((*func, data.clone())) } else { None };
                    *sm = RxState::Head;
                    return outcome;
                }
                _ => { *sm = RxState::Head; return None; }
            }
            *idx += 1; *sm = current; outcome
        }
    };
    result
}

// ============================================================
// 传感器解析
// ============================================================

fn i16l(b: &[u8]) -> i16 { i16::from_le_bytes([b[0], b[1]]) }
fn i32l(b: &[u8]) -> i32 { i32::from_le_bytes([b[0], b[1], b[2], b[3]]) }

fn update_state(state: &mut RobotState, func: u8, data: &[u8]) {
    match func {
        RPT_SPEED => {
            if data.len() >= 7 {
                state.vx = i16l(&data[0..2]) as f32 / 1000.0;
                state.vy = i16l(&data[2..4]) as f32 / 1000.0;
                state.vz = i16l(&data[4..6]) as f32 / 1000.0;
                state.battery = data[6] as f32 / 10.0;
            }
        }
        RPT_IMU_ATT => {
            if data.len() >= 6 {
                state.attitude.roll = i16l(&data[0..2]) as f32 / 10000.0;
                state.attitude.pitch = i16l(&data[2..4]) as f32 / 10000.0;
                state.attitude.yaw = i16l(&data[4..6]) as f32 / 10000.0;
            }
        }
        RPT_ENCODER => {
            if data.len() >= 16 {
                state.encoders = [i32l(&data[0..4]), i32l(&data[4..8]),
                                  i32l(&data[8..12]), i32l(&data[12..16])];
            }
        }
        RPT_MPU_RAW => {
            if data.len() >= 18 {
                let gr = 1.0 / 3754.9; let ar = 1.0 / 1671.84;
                state.gyro.gx = i16l(&data[0..2]) as f32 * gr;
                state.gyro.gy = i16l(&data[2..4]) as f32 * -gr;
                state.gyro.gz = i16l(&data[4..6]) as f32 * -gr;
                state.accel.ax = i16l(&data[6..8]) as f32 * ar;
                state.accel.ay = i16l(&data[8..10]) as f32 * ar;
                state.accel.az = i16l(&data[10..12]) as f32 * ar;
                state.mag.mx = i16l(&data[12..14]) as f32;
                state.mag.my = i16l(&data[14..16]) as f32;
                state.mag.mz = i16l(&data[16..18]) as f32;
            }
        }
        RPT_ICM_RAW => {
            if data.len() >= 6 {
                let r = 1.0 / 1000.0;
                state.gyro.gx = i16l(&data[0..2]) as f32 * r;
                state.gyro.gy = i16l(&data[2..4]) as f32 * r;
                state.gyro.gz = i16l(&data[4..6]) as f32 * r;
            }
        }
        _ => {}
    }
}

// ============================================================
// STM32Device — 控制句柄
// ============================================================

pub struct STM32Device {
    cmd_tx: mpsc::Sender<Vec<u8>>,
    state: Arc<RwLock<RobotState>>,
    car_type: CarType,
    cancel: CancellationToken,
}

impl STM32Device {
    pub fn spawn(port: &str, baudrate: u32, car_type: CarType, state: Arc<RwLock<RobotState>>) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let mut sm: RxState = RxState::Head;
        let mut local_state = RobotState::default();

        let cmd_tx = port::spawn_port(
            port, baudrate, 512,
            move |bytes| {
                debug!("[STM32 RX] raw: {:02X?}", bytes);
                for &byte in bytes {
                    if let Some((func, data)) = feed_state_machine(&mut sm, byte) {
                        info!("[STM32] 解析帧 func=0x{func:02X}, data={:02X?}", data);
                        update_state(&mut local_state, func, &data);
                    }
                }
                // 有更新时写回共享缓存
                let s = local_state.clone();
                info!("[STM32] 状态更新: vx={:.3}, vy={:.3}, vz={:.3}, bat={:.2}V, roll={:.3}, pitch={:.3}, yaw={:.3}, enc={:?}",
                    s.vx, s.vy, s.vz, s.battery, s.attitude.roll, s.attitude.pitch, s.attitude.yaw, s.encoders);
                let state = state_clone.clone();
                tokio::spawn(async move { *state.write().await = s; });
            },
            cancel.clone(),
        )?;

        Ok(Self { cmd_tx, state, car_type, cancel })
    }

    pub fn shutdown(&self) { self.cancel.cancel(); }

    // ─── 运动控制 ────────

    pub async fn forward(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Forward, speed).await }
    pub async fn backward(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Backward, speed).await }
    pub async fn stop(&self) -> Result<(), String> { self.send_car_run(MotionState::Stop, 0).await }
    pub async fn left(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Left, speed).await }
    pub async fn right(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Right, speed).await }
    pub async fn spin_left(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::SpinLeft, speed).await }
    pub async fn spin_right(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::SpinRight, speed).await }

    async fn send_car_run(&self, state: MotionState, speed: i16) -> Result<(), String> {
        let bytes = pack_car_run(self.car_type as u8, state as u8, speed);
        self.cmd_tx.send(bytes).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn set_motion(&self, vx: f32, vy: f32, vz: f32) -> Result<(), String> {
        let limits = self.car_type.motion_limits();
        let bytes = pack_motion(self.car_type as u8, vx, vy, vz, limits);
        self.cmd_tx.send(bytes).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn set_motor(&self, m1: i8, m2: i8, m3: i8, m4: i8) -> Result<(), String> {
        self.cmd_tx.send(pack_motor(m1, m2, m3, m4)).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn beep(&self, duration_ms: u16) -> Result<(), String> {
        self.cmd_tx.send(pack_beep(duration_ms)).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn set_servo(&self, id: u8, angle: u8) -> Result<(), String> {
        self.cmd_tx.send(pack_servo(id, angle)).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn set_rgb(&self, led_id: u8, r: u8, g: u8, b: u8) -> Result<(), String> {
        self.cmd_tx.send(pack_rgb(led_id, r, g, b)).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn set_rgb_effect(&self, effect: u8, speed: u8) -> Result<(), String> {
        self.cmd_tx.send(pack_rgb_effect(effect, speed)).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn reset_state(&self) -> Result<(), String> {
        self.cmd_tx.send(pack_reset()).await.map_err(|_| "TX channel 已关闭".into())
    }

    pub async fn get_state(&self) -> RobotState {
        self.state.read().await.clone()
    }
}
