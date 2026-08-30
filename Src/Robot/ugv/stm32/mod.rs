//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-26

//! STM32 控制板设备驱动
//!
//! STM32Device — 封装串口协议 + TX/RX。
//! 协议层拆分见 constants.rs / protocol.rs。
//!
//! 运动控制方法均为同步（try_send），串口写入在独立 TX task 中异步完成。

pub mod constants;
pub mod protocol;

use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use constants::{MotionState, RPT_SPEED};
use protocol::{
    feed_state_machine, pack_beep, pack_car_run, pack_motion, pack_motor,
    pack_reset, pack_rgb, pack_rgb_effect, pack_servo, update_state, RxState,
};
use crate::robot::util::serial::port;
use crate::robot::ugv::types::CarType;
use crate::robot::core::command::ManualCmd;
use crate::robot::core::state::{MotionAction, RobotState};
use crate::robot::ugv::slam::odometry;

// ============================================================
// STM32Device — 控制句柄
// ============================================================

pub struct STM32Device {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    state: Arc<RwLock<RobotState>>,
    car_type: CarType,
    forward_speed: i16,
    turn_speed: i16,
    /// 四轮编码器计数（Task 23 B1：从 RobotState 摘出，设备内部维护）
    encoders: Arc<Mutex<[i32; 4]>>,
    cancel: CancellationToken,
}

impl STM32Device {
    /// 启动 STM32 设备。`origin` 为小车初始世界坐标 (x, y, z)（默认 64,64,0），
    /// 注入 local_state 的 x/y/z 作为积分起点（Task 9：RobotState 记录全局唯一坐标）。
    pub fn spawn(port: &str, baudrate: u32, car_type: CarType, state: Arc<RwLock<RobotState>>, origin: (f32, f32, f32), forward_speed: i16, turn_speed: i16) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let encoders = Arc::new(Mutex::new([0i32; 4]));
        let encoders_clone = encoders.clone();
        let mut sm: RxState = RxState::Head;
        let mut local_state = RobotState::default();
        // 注入初始世界坐标（全局唯一坐标起点，Task 9）
        local_state.x = origin.0;
        local_state.y = origin.1;
        local_state.z = origin.2;
        let mut last_speed_ts = Instant::now();

        let serial_cmd_tx = port::spawn_port(
            port, baudrate, 512,
            move |bytes| {
                let mut frame_parsed = false;
                for &byte in bytes {
                    if let Some((func, data)) = feed_state_machine(&mut sm, byte) {
                        if let Some(enc) = update_state(&mut local_state, func, &data) {
                            *encoders_clone.lock().unwrap() = enc;
                        }
                        if func == RPT_SPEED {
                            let now = Instant::now();
                            let dt = (now - last_speed_ts).as_secs_f32();
                            if dt > 0.0 && dt < 1.0 {
                                odometry::accumulate(&mut local_state, dt);
                            }
                            last_speed_ts = now;
                        }
                        frame_parsed = true;
                    }
                }
                if frame_parsed {
                    if let Ok(mut guard) = state_clone.try_write() {
                        // 单一写入者不变式：共享 RobotState 只在此处全量覆盖；
                        // x/y 仅在 local_state 内维护（origin 初始化 + accumulate 积分）
                        *guard = local_state.clone();
                    } else {
                        warn!("[STM32] try_write 失败，状态更新丢弃");
                    }
                }
            },
            cancel.clone(),
        )?;

        Ok(Self { serial_cmd_tx, state, car_type, forward_speed, turn_speed, encoders, cancel })
    }

    pub fn shutdown(&self) { self.cancel.cancel(); }

    // ─── 运动控制（同步，try_send 入队到 TX task）────────

    pub fn forward(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Forward, speed) }
    pub fn backward(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Backward, speed) }
    pub fn stop(&self) -> Result<(), String> { self.send_car_run(MotionState::Stop, 0) }
    pub fn left(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Left, speed) }
    pub fn right(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::Right, speed) }
    pub fn spin_left(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::SpinLeft, speed) }
    pub fn spin_right(&self, speed: i16) -> Result<(), String> { self.send_car_run(MotionState::SpinRight, speed) }

    // ─── 命令/动作解析（A6：下沉到设备，设备选择性响应）────────
    pub fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        match cmd {
            ManualCmd::Forward(_) => { let _ = self.forward(self.forward_speed); }
            ManualCmd::Backward(_) => { let _ = self.backward(self.forward_speed); }
            ManualCmd::SpinLeft(_) => { let _ = self.spin_left(self.turn_speed); }
            ManualCmd::SpinRight(_) => { let _ = self.spin_right(self.turn_speed); }
            ManualCmd::Stop => { let _ = self.stop(); }
            ManualCmd::Beep(ms) => { let _ = self.beep(*ms); }
            _ => {} // 车不支持 Takeoff/Land/LiDAR 命令，忽略
        }
    }

    pub fn apply_action(&self, action: MotionAction) {
        match action {
            MotionAction::MoveForward => { let _ = self.forward(self.forward_speed); }
            MotionAction::MoveBackward => { let _ = self.backward(self.forward_speed); }
            MotionAction::TurnLeft => { let _ = self.spin_left(self.turn_speed); }
            MotionAction::TurnRight => { let _ = self.spin_right(self.turn_speed); }
            MotionAction::Stop => { let _ = self.stop(); }
        }
    }

    fn send_car_run(&self, state: MotionState, speed: i16) -> Result<(), String> {
        let bytes = pack_car_run(self.car_type as u8, state as u8, speed);
        self.serial_cmd_tx.try_send(bytes).map_err(|e| format!("TX 通道满: {e}"))
    }

    pub fn set_motion(&self, vx: f32, vy: f32, vz: f32) -> Result<(), String> {
        let limits = self.car_type.motion_limits();
        let bytes = pack_motion(self.car_type as u8, vx, vy, vz, limits);
        self.serial_cmd_tx.try_send(bytes).map_err(|e| format!("TX 通道满: {e}"))
    }

    pub fn set_motor(&self, m1: i8, m2: i8, m3: i8, m4: i8) -> Result<(), String> {
        self.serial_cmd_tx.try_send(pack_motor(m1, m2, m3, m4)).map_err(|e| format!("TX 通道满: {e}"))
    }

    pub fn beep(&self, duration_ms: u16) -> Result<(), String> {
        self.serial_cmd_tx.try_send(pack_beep(duration_ms)).map_err(|e| format!("TX 通道满: {e}"))
    }

    pub fn set_servo(&self, id: u8, angle: u8) -> Result<(), String> {
        self.serial_cmd_tx.try_send(pack_servo(id, angle)).map_err(|e| format!("TX 通道满: {e}"))
    }

    pub fn set_rgb(&self, led_id: u8, r: u8, g: u8, b: u8) -> Result<(), String> {
        self.serial_cmd_tx.try_send(pack_rgb(led_id, r, g, b)).map_err(|e| format!("TX 通道满: {e}"))
    }

    pub fn set_rgb_effect(&self, effect: u8, speed: u8) -> Result<(), String> {
        self.serial_cmd_tx.try_send(pack_rgb_effect(effect, speed)).map_err(|e| format!("TX 通道满: {e}"))
    }

    pub fn reset_state(&self) -> Result<(), String> {
        self.serial_cmd_tx.try_send(pack_reset()).map_err(|e| format!("TX 通道满: {e}"))
    }

    /// 读取当前传感器状态快照（async：需要持有 RwLock read guard）
    pub async fn get_state(&self) -> RobotState {
        self.state.read().await.clone()
    }
}

// ============================================================
// Mock spawn（测试用）
// ============================================================

#[cfg(test)]
impl STM32Device {
    /// 创建模拟设备：不依赖真实串口，通过 channel 模拟 TX/RX。
    ///
    /// - `rx_feed`: 向此 Receiver 发送 Vec<u8> 可模拟串口收到数据
    /// - `tx_sink`: 设备发出的命令字节会发送到此 Sender
    ///
    /// 返回设备句柄 + 后台 task JoinHandle（用于等待处理完成）。

    /// `origin`: 初始世界坐标 (x, y, z)，与生产 spawn 语义一致。
    pub fn spawn_mock(
        mut rx_feed: mpsc::Receiver<Vec<u8>>,
        tx_sink: mpsc::Sender<Vec<u8>>,
        car_type: CarType,
        state: Arc<RwLock<RobotState>>,
        origin: (f32, f32, f32),
    ) -> (Self, tokio::task::JoinHandle<()>) {
        use protocol::{feed_state_machine, update_state, RxState};
        use constants::{FORWARD_SPEED, TURN_SPEED};

        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let encoders = Arc::new(Mutex::new([0i32; 4]));
        let encoders_clone = encoders.clone();
        let (serial_cmd_tx, mut cmd_rx) = mpsc::channel::<Vec<u8>>(32);
        let mock_cancel = cancel.clone();

        // 后台 task：TX 转发 + RX 模拟
        let handle = tokio::spawn(async move {
            let mut sm: RxState = RxState::Head;
            let mut local_state = RobotState::default();
            // 注入初始世界坐标（与生产 spawn 一致，Task 9）
            local_state.x = origin.0;
            local_state.y = origin.1;
            local_state.z = origin.2;
            let mut last_speed_ts = Instant::now();

            loop {
                tokio::select! {
                    // TX: 把命令转到 tx_sink
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(bytes) => { let _ = tx_sink.send(bytes).await; }
                            None => break,
                        }
                    }
                    // RX: 从 rx_feed 读取模拟数据
                    rx = rx_feed.recv() => {
                        match rx {
                            Some(bytes) => {
                                let mut frame_parsed = false;
                                for &b in &bytes {
                                    if let Some((func, data)) = feed_state_machine(&mut sm, b) {
                                        if let Some(enc) = update_state(&mut local_state, func, &data) {
                                            *encoders_clone.lock().unwrap() = enc;
                                        }
                                        if func == RPT_SPEED {
                                            let now = Instant::now();
                                            let dt = (now - last_speed_ts).as_secs_f32();
                                            if dt > 0.0 && dt < 1.0 {
                                                odometry::accumulate(&mut local_state, dt);
                                            }
                                            last_speed_ts = now;
                                        }
                                        frame_parsed = true;
                                    }
                                }
                                if frame_parsed {
                                    if let Ok(mut guard) = state_clone.try_write() {
                                        *guard = local_state.clone();
                                    } else {
                                        warn!("[STM32 mock] try_write 失败，状态更新丢弃");
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

        (Self { serial_cmd_tx, state, car_type, forward_speed: FORWARD_SPEED, turn_speed: TURN_SPEED, encoders, cancel }, handle)
    }
}

#[cfg(test)]
mod mock_tests {
    use super::*;
    use crate::robot::ugv::stm32::protocol::stm32_report_frame;

    #[tokio::test]
    async fn test_mock_forward_command() {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (tx_sink, mut tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::X3Plus, state, (64.0, 64.0, 0.0));
        dev.forward(50).unwrap();

        // 验证 TX 发出了正确的命令帧
        let sent = tx_rx.recv().await.unwrap();
        assert_eq!(sent[3], constants::FUNC_CAR_RUN); // 功能码
        let speed = i16::from_le_bytes([sent[6], sent[7]]);
        assert_eq!(speed, 50);
    }

    #[tokio::test]
    async fn test_mock_rx_state_update() {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (tx_sink, _tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (_dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::X3Plus, state.clone(), (64.0, 64.0, 0.0));

        // 构造一条 SPEED 上报帧并喂入
        let vx = (0.250f32 * 1000.0) as i16;
        let mut data = Vec::new();
        data.extend_from_slice(&vx.to_le_bytes());
        data.extend_from_slice(&0i16.to_le_bytes()); // vy=0
        data.extend_from_slice(&0i16.to_le_bytes()); // vz=0
        data.push(120); // battery=12.0V

        let frame = stm32_report_frame(constants::RPT_SPEED, &data);
        rx_tx.send(frame).await.unwrap();

        // 给 mock task 一点时间处理
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let s = state.read().await;
        assert!((s.vx - 0.250).abs() < 0.001);
        assert!((s.battery - 12.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_mock_fragmented_rx() {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (tx_sink, _tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (_dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::X3Plus, state.clone(), (64.0, 64.0, 0.0));

        // 构造 SPEED 帧，分两次发送
        let vx = 100i16;
        let mut data = vec![0u8; 7];
        data[0..2].copy_from_slice(&vx.to_le_bytes());

        let frame = stm32_report_frame(constants::RPT_SPEED, &data);
        let (p1, p2) = frame.split_at(frame.len() / 2);

        rx_tx.send(p1.to_vec()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        rx_tx.send(p2.to_vec()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let s = state.read().await;
        assert!((s.vx - 0.100).abs() < 0.001, "分包到达应正确解析 vx");
    }

    #[tokio::test]
    async fn test_mock_origin_injected() {
        // Task 9：origin 注入 local_state，首帧覆盖写后共享态 x/y 应为 origin
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (tx_sink, _tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (_dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::X3Plus, state.clone(), (66.5, 63.25, 0.0));

        // 喂一帧 vx=0 的 SPEED 帧触发全量覆盖；无位移，x/y 应保持 origin
        let vx = 0i16;
        let mut data = vec![0u8; 7];
        data[0..2].copy_from_slice(&vx.to_le_bytes());
        let frame = stm32_report_frame(constants::RPT_SPEED, &data);
        rx_tx.send(frame).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let s = state.read().await;
        assert!((s.x - 66.5).abs() < 1e-3, "共享态 x 应为 origin: {}", s.x);
        assert!((s.y - 63.25).abs() < 1e-3, "共享态 y 应为 origin: {}", s.y);
        assert!((s.z - 0.0).abs() < 1e-3, "共享态 z 应为 origin: {}", s.z);
    }
}
