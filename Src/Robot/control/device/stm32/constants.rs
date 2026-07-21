//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-21

//! STM32 协议常量
//!
//! 对齐 ros_stm32_protocol.md 中定义的帧格式和功能码。

// ─── 运动状态（FUNC_CAR_RUN） ──────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MotionState {
    Stop = 0,
    Forward = 1,
    Backward = 2,
    Left = 3,
    Right = 4,
    SpinLeft = 5,
    SpinRight = 6,
    Park = 7,
}

// ─── 帧常量 ──────────────────────────────────────────

pub const HEAD: u8 = 0xFF;
pub const DEV_ID_HOST: u8 = 0xFC;
pub const DEV_ID_STM32: u8 = 0xFB;
pub const COMPLEMENT: u8 = 5;

// ─── 命令功能码 ───────────────────────────────────────

pub const FUNC_BEEP: u8 = 0x02;
pub const FUNC_PWM_SERVO: u8 = 0x03;
pub const FUNC_RGB: u8 = 0x05;
pub const FUNC_RGB_EFFECT: u8 = 0x06;
pub const FUNC_MOTOR: u8 = 0x10;
pub const FUNC_CAR_RUN: u8 = 0x11;
pub const FUNC_MOTION: u8 = 0x12;
pub const FUNC_RESET_STATE: u8 = 0x0F;

// ─── 上报功能码 ───────────────────────────────────────

pub const RPT_SPEED: u8 = 0x0A;
pub const RPT_MPU_RAW: u8 = 0x0B;
pub const RPT_IMU_ATT: u8 = 0x0C;
pub const RPT_ENCODER: u8 = 0x0D;
pub const RPT_ICM_RAW: u8 = 0x0E;
