//Presented by KeJi
//Date ： 2026-06-17

//! 机器人控制 trait 与具体实现
//!
//! 高层 API：前进、后退、转向、停车、传感器读取。
//! 底层依赖 serial_io::SerialIo 做串口收发，protocol 做帧打包。

use async_trait::async_trait;

use super::protocol::{
    self, FUNC_BEEP, FUNC_CAR_RUN, FUNC_MOTION, FUNC_MOTOR,
    FUNC_PWM_SERVO, FUNC_RGB, FUNC_RGB_EFFECT,
    FUNC_RESET_STATE,
};
use super::serial_io::SerialIo;
use super::types::{CarType, MotionState, RobotState};

// ============================================================
// RobotCapability trait
// ============================================================

/// 机器人控制能力接口
#[async_trait]
pub trait RobotCapability: Send + Sync {
    /// 打开串口连接（会启动后台接收）
    async fn open(&self, port: &str, baudrate: u32, car_type: CarType) -> Result<(), String>;

    /// 关闭串口
    async fn close(&self) -> Result<(), String>;

    /// 简化运动控制（FUNC_CAR_RUN）
    async fn set_car_run(&self, state: MotionState, speed: i16) -> Result<(), String>;

    /// 速度矢量控制（FUNC_MOTION）
    async fn set_motion(&self, vx: f32, vy: f32, vz: f32) -> Result<(), String>;

    /// 四轮独立 PWM（FUNC_MOTOR，范围 [-100, 100]，127=保持）
    async fn set_motor(&self, m1: i8, m2: i8, m3: i8, m4: i8) -> Result<(), String>;

    /// 蜂鸣器（duration_ms: 0=关, 1=长鸣, >=10=毫秒）
    async fn beep(&self, duration_ms: u16) -> Result<(), String>;

    /// PWM 舵机（id: 1-4, angle: 0-180）
    async fn set_servo(&self, id: u8, angle: u8) -> Result<(), String>;

    /// RGB 灯（led_id: 0-13 或 0xFF=全控）
    async fn set_rgb(&self, led_id: u8, r: u8, g: u8, b: u8) -> Result<(), String>;

    /// RGB 灯效
    async fn set_rgb_effect(&self, effect: u8, speed: u8) -> Result<(), String>;

    /// 复位（停车+关灯+关蜂鸣器）
    async fn reset_state(&self) -> Result<(), String>;

    /// 获取传感器状态快照
    async fn get_state(&self) -> RobotState;

    /// 串口是否已打开
    fn is_open(&self) -> bool;
}

// ============================================================
// Robot 具体实现
// ============================================================

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;

/// 机器人控制器
///
/// 封装串口 I/O + 车型参数，提供高层运动控制 API。
pub struct Robot {
    io: RwLock<Option<SerialIo>>,
    car_type: RwLock<CarType>,
    opened: AtomicBool,
}

impl Robot {
    pub fn new() -> Self {
        Self {
            io: RwLock::new(None),
            car_type: RwLock::new(CarType::X3Plus),
            opened: AtomicBool::new(false),
        }
    }

    /// 快捷方法：前进
    pub async fn forward(&self, speed: i16) -> Result<(), String> {
        self.set_car_run(MotionState::Forward, speed).await
    }

    /// 快捷方法：后退
    pub async fn backward(&self, speed: i16) -> Result<(), String> {
        self.set_car_run(MotionState::Backward, speed).await
    }

    /// 快捷方法：停车
    pub async fn stop(&self) -> Result<(), String> {
        self.set_car_run(MotionState::Stop, 0).await
    }

    /// 快捷方法：左移
    pub async fn left(&self, speed: i16) -> Result<(), String> {
        self.set_car_run(MotionState::Left, speed).await
    }

    /// 快捷方法：右移
    pub async fn right(&self, speed: i16) -> Result<(), String> {
        self.set_car_run(MotionState::Right, speed).await
    }

    /// 快捷方法：左旋
    pub async fn spin_left(&self, speed: i16) -> Result<(), String> {
        self.set_car_run(MotionState::SpinLeft, speed).await
    }

    /// 快捷方法：右旋
    pub async fn spin_right(&self, speed: i16) -> Result<(), String> {
        self.set_car_run(MotionState::SpinRight, speed).await
    }
}

impl Default for Robot {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RobotCapability for Robot {
    async fn open(&self, port: &str, baudrate: u32, car_type: CarType) -> Result<(), String> {
        let io = SerialIo::open(port, baudrate)?;
        *self.car_type.write().await = car_type;
        *self.io.write().await = Some(io);
        self.opened.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn close(&self) -> Result<(), String> {
        *self.io.write().await = None;
        self.opened.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn set_car_run(&self, state: MotionState, speed: i16) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        let car_type = *self.car_type.read().await;
        let data = protocol::pack_car_run(car_type as u8, state as u8, speed);
        io.send_frame(FUNC_CAR_RUN, &data).await
    }

    async fn set_motion(&self, vx: f32, vy: f32, vz: f32) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        let car_type = *self.car_type.read().await;
        let limits = car_type.motion_limits();
        let data = protocol::pack_motion(car_type as u8, vx, vy, vz, limits);
        io.send_frame(FUNC_MOTION, &data).await
    }

    async fn set_motor(&self, m1: i8, m2: i8, m3: i8, m4: i8) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        let data = protocol::pack_motor(m1, m2, m3, m4);
        io.send_frame(FUNC_MOTOR, &data).await
    }

    async fn beep(&self, duration_ms: u16) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        let data = protocol::pack_beep(duration_ms);
        io.send_frame(FUNC_BEEP, &data).await
    }

    async fn set_servo(&self, id: u8, angle: u8) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        let data = protocol::pack_servo(id, angle);
        io.send_frame(FUNC_PWM_SERVO, &data).await
    }

    async fn set_rgb(&self, led_id: u8, r: u8, g: u8, b: u8) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        io.send_frame(FUNC_RGB, &[led_id, r, g, b]).await
    }

    async fn set_rgb_effect(&self, effect: u8, speed: u8) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        io.send_frame(FUNC_RGB_EFFECT, &[effect, speed, 0xFF]).await
    }

    async fn reset_state(&self) -> Result<(), String> {
        let io = self.io.read().await;
        let io = io.as_ref().ok_or("串口未打开")?;
        io.send_frame(FUNC_RESET_STATE, &[0x5F]).await
    }

    async fn get_state(&self) -> RobotState {
        let io = self.io.read().await;
        match io.as_ref() {
            Some(io) => io.get_state().await,
            None => RobotState::default(),
        }
    }

    fn is_open(&self) -> bool {
        self.opened.load(Ordering::SeqCst)
    }
}
