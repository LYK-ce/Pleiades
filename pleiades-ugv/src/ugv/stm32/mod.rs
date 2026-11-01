//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-11-01

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
use tracing::{info, warn};

use constants::RPT_SPEED;
use protocol::{
    feed_state_machine, pack_beep, pack_motion, pack_motor,
    pack_reset, pack_rgb, pack_rgb_effect, pack_servo, update_state, RxState,
};
use pleiades_base::robot::util::serial::port;
use crate::ugv::types::CarType;
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::state::RobotState;
use crate::ugv::slam::odometry;

// ============================================================
// STM32Device — 控制句柄
// ============================================================

pub struct STM32Device {
    serial_cmd_tx: mpsc::Sender<Vec<u8>>,
    state: Arc<RwLock<RobotState>>,
    car_type: CarType,
    /// 四轮编码器计数（Task 23 B1：从 RobotState 摘出，设备内部维护）
    encoders: Arc<Mutex<[i32; 4]>>,
    cancel: CancellationToken,
}

impl STM32Device {
    /// 启动 STM32 设备。（Task 24：origin 已由 Robot::new 注入共享 RobotState，
    /// 本处 `_origin` 保留仅对齐签名，不再注入 local_state）
    pub fn spawn(port: &str, baudrate: u32, car_type: CarType, state: Arc<RwLock<RobotState>>, _origin: (f32, f32, f32)) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let state_clone = state.clone();
        let encoders = Arc::new(Mutex::new([0i32; 4]));
        let encoders_clone = encoders.clone();
        let mut sm: RxState = RxState::Head;
        let mut local_state = RobotState::default();
        // Task 24 写者改造：x/y/z 源头已移到共享 RobotState（由 Robot::new 注入 origin），
        // local_state 降级为纯传感器 staging（不再注入 origin）
        let mut last_speed_ts = Instant::now();
        let mut pending_dt: Option<f32> = None;

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
                                pending_dt = Some(dt);
                            }
                            last_speed_ts = now;
                        }
                        frame_parsed = true;
                    }
                }
                if frame_parsed {
                    if let Ok(mut guard) = state_clone.try_write() {
                        // Task 24 写者改造：传感器字段照写，x/y 读-改-写（同一把写锁内，
                        // 避免与 LG290P RTK 覆盖交错丢增量）
                        guard.vx = local_state.vx;
                        guard.vy = local_state.vy;
                        guard.vz = local_state.vz;
                        guard.battery = local_state.battery;
                        guard.attitude = local_state.attitude.clone();
                        guard.gyro = local_state.gyro.clone();
                        guard.accel = local_state.accel.clone();
                        guard.mag = local_state.mag.clone();
                        guard.z = local_state.z; // 车恒 0（D11：z 保持设备端现状）
                        if let Some(dt) = pending_dt.take() {
                            let (x0, y0) = (guard.x, guard.y);
                            odometry::accumulate(&mut *guard, dt);
                            // 排查用（真机偶发位置跳变）：记录本次积分的输入与结果
                            info!(
                                "[ODO] dt={dt:.3}s 来了 vx={:.3} vy={:.3} yaw={:.1}° | \
                                 更新 ({x0:.3},{y0:.3}) -> ({:.3},{:.3})",
                                guard.vx,
                                guard.vy,
                                guard.attitude.yaw.to_degrees(),
                                guard.x,
                                guard.y
                            );
                        }
                        // 不写 guard.rtk_fixed（LG290P 独写）
                    } else {
                        warn!("[STM32] try_write 失败，状态更新丢弃");
                    }
                }
            },
            cancel.clone(),
        )?;

        Ok(Self { serial_cmd_tx, state, car_type, encoders, cancel })
    }

    pub fn shutdown(&self) { self.cancel.cancel(); }

    // ─── 运动控制（同步，try_send 入队到 TX task）────────

    /// 连续速度下发（task_44 §1.3）：`v` m/s、`omega` rad/s（`omega > 0` = 左转/逆时针）
    ///
    /// → `0x12 FUNC_MOTION`：`vx = v`（本来就是 m/s）、`vy = 0`、`vz = omega_to_vz(v, omega)`。
    /// 非有限值（NaN/∞）经 `pack_motion` 的 `clamp` 后按 0 处理（`as i16` 饱和），不 panic。
    pub fn apply_velocity(&self, v: f32, omega: f32) -> Result<(), String> {
        let bytes = self.motion_bytes(v, omega);
        self.serial_cmd_tx.try_send(bytes).map_err(|e| format!("TX 通道满: {e}"))
    }

    /// 停车（安全关键）：`0x12` 全零运动帧（task_44 §1.2 #1：停车也走这条通道，
    /// 旧的离散动作通道已整体删除），TX 通道短暂拥塞时有界重试，不静默丢弃（Task 23 review 修复）
    pub fn stop(&self) -> Result<(), String> {
        let bytes = self.motion_bytes(0.0, 0.0);
        for _ in 0..10 {
            match self.serial_cmd_tx.try_send(bytes.clone()) {
                Ok(()) => return Ok(()),
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    return Err("TX 通道关闭".to_string());
                }
            }
        }
        Err("TX 通道持续拥塞，Stop 送达失败".to_string())
    }

    // ─── 命令/动作解析（A6：下沉到设备，设备选择性响应）────────
    pub fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        let result = match cmd {
            ManualCmd::Motion { v, omega } => self.apply_velocity(*v, *omega),
            // §1.2 #6：协议里已无 stop 动作，`Stop` 是内部命令 → 按 `(0, 0)` 执行
            ManualCmd::Stop => self.stop(),
            ManualCmd::Beep(ms) => self.beep(*ms),
            _ => return, // 车不支持 Takeoff/Land/LiDAR 命令，忽略
        };
        if let Err(e) = result {
            warn!("[STM32] 手动命令下发失败: {e}");
        }
    }

    /// `(v, omega)` → `0x12` 帧字节（`pack_motion` 内部还会按 `motion_limits` 再夹一次）
    fn motion_bytes(&self, v: f32, omega: f32) -> Vec<u8> {
        let limits = self.car_type.motion_limits();
        let vz = omega_to_vz(v, omega);
        pack_motion(self.car_type as u8, v, 0.0, vz, limits)
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
// `vz` 反解标定（task_44 §二 Step 4 实施步骤 2：2026-11-01 实测）
//
// `0x12` 的 `vz` **不是角速度**：原地转死区 `|vz| ≤ 0.2`、`ω ≈ 40 °/s × (vz − 0.2)`
// ⇒ 每单位 `vz` 对应 `40 °/s ≈ 0.70 rad/s`；边走边转的 ω 约为原地转的 2.8 倍
// ⇒ 按 `v == 0` 选两套系数（不分段会让弧线转过头）。
// ============================================================

/// 原地转死区：`|vz| ≤ 0.2` 车不动（实测）。
/// **`omega = 0` 时必须发 0，不能发 0.2，否则车会一直转**
const VZ_DEADBAND: f32 = 0.2;
/// 原地转：每单位 `vz` 对应 40 °/s ≈ 0.70 rad/s。← 待上车标定
const OMEGA_PER_VZ_INPLACE_RAD_S: f32 = 0.70;
/// 边走边转：同一 `vz` 下 ω 约为原地转的 2.8 倍 ⇒ 单位增益 ×2.8。← 待上车标定
const VZ_MOVING_GAIN: f32 = 2.8;
/// `vz` 单轴上限（= `motion_limits().2`，`pack_motion` 还会再夹一次）≈120 °/s 原地。← 待上车标定
const VZ_MAX: f32 = 3.2;

/// `(v, omega)` → `0x12` 的 `vz`（**同号**：`vz > 0` = 左转 = 逆时针 = `omega > 0`）
fn omega_to_vz(v: f32, omega: f32) -> f32 {
    if omega == 0.0 {
        return 0.0; // 硬条件：不转就必须发 0（不能发死区 0.2）
    }
    // §1.4 的档位里 `v` 只有 `0` 与 `0.45` 两种取值，故按 `v == 0` 分段即可
    let per_unit = if v == 0.0 {
        OMEGA_PER_VZ_INPLACE_RAD_S
    } else {
        OMEGA_PER_VZ_INPLACE_RAD_S * VZ_MOVING_GAIN // = 1.96 rad/s 每单位
    };
    let mag = (VZ_DEADBAND + omega.abs() / per_unit).min(VZ_MAX);
    if omega > 0.0 { mag } else { -mag }
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
        _origin: (f32, f32, f32),
    ) -> (Self, tokio::task::JoinHandle<()>) {
        use protocol::{feed_state_machine, update_state, RxState};

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
            // Task 24 写者改造：x/y/z 源头已移到共享 RobotState（Robot::new 注入 origin）
            let mut last_speed_ts = Instant::now();
            let mut pending_dt: Option<f32> = None;

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
                                                pending_dt = Some(dt);
                                            }
                                            last_speed_ts = now;
                                        }
                                        frame_parsed = true;
                                    }
                                }
                                if frame_parsed {
                                    if let Ok(mut guard) = state_clone.try_write() {
                                        // Task 24 写者改造：传感器字段照写，x/y 读-改-写（同一把写锁内）
                                        guard.vx = local_state.vx;
                                        guard.vy = local_state.vy;
                                        guard.vz = local_state.vz;
                                        guard.battery = local_state.battery;
                                        guard.attitude = local_state.attitude.clone();
                                        guard.gyro = local_state.gyro.clone();
                                        guard.accel = local_state.accel.clone();
                                        guard.mag = local_state.mag.clone();
                                        guard.z = local_state.z;
                                        if let Some(dt) = pending_dt.take() {
                                            odometry::accumulate(&mut *guard, dt);
                                        }
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

        (Self { serial_cmd_tx, state, car_type, encoders, cancel }, handle)
    }
}

#[cfg(test)]
mod mock_tests {
    use super::*;
    use crate::ugv::stm32::protocol::stm32_report_frame;

    /// task_44 §1.2 #1：运动只走 `0x12`；(v, omega) → (vx, vy=0, vz 反解)
    #[tokio::test]
    async fn test_mock_apply_velocity_command() {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (tx_sink, mut tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (_rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::X3Plus, state, (64.0, 64.0, 0.0));
        dev.apply_velocity(0.45, 0.5).unwrap();

        // 验证 TX 发出了正确的命令帧
        let sent = tx_rx.recv().await.unwrap();
        assert_eq!(sent[3], constants::FUNC_MOTION); // 功能码 = 0x12
        assert_eq!(sent[4], CarType::X3Plus as u8);
        assert_eq!(i16::from_le_bytes([sent[5], sent[6]]), 450, "vx = v（m/s → mm/s）");
        assert_eq!(i16::from_le_bytes([sent[7], sent[8]]), 0, "vy 恒 0");
        let vz = i16::from_le_bytes([sent[9], sent[10]]);
        // v ≠ 0 ⇒ 「边走边转」那套系数：vz = 0.2 + 0.5/1.96 ≈ 0.4551 ⇒ 455
        // （原地转的 0.9143 ⇒ 914 由 `calib_tests::test_in_place_gold` 单独锁）
        assert!((vz - 455).abs() <= 1, "边走边转 0.5 rad/s 的 vz ≈ 455，实际 {vz}");
    }

    /// §1.2 #1：停车也走 `0x12` 全零（不走旧离散动作通道）—— 急停 / reset / suspend 共用这条路径
    #[tokio::test]
    async fn test_mock_stop_is_zero_motion() {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (tx_sink, mut tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (_rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::M1, state, (64.0, 64.0, 0.0));
        dev.stop().unwrap();

        let sent = tx_rx.recv().await.unwrap();
        assert_eq!(sent[3], constants::FUNC_MOTION);
        assert_eq!(sent[4], CarType::M1 as u8);
        assert_eq!(i16::from_le_bytes([sent[5], sent[6]]), 0, "vx = 0");
        assert_eq!(i16::from_le_bytes([sent[7], sent[8]]), 0, "vy = 0");
        assert_eq!(i16::from_le_bytes([sent[9], sent[10]]), 0, "vz = 0（不是死区 200）");
    }

    /// §1.2 硬条件：`omega = 0` 就必须 `vz = 0`，否则车会一直原地转
    #[tokio::test]
    async fn test_mock_apply_velocity_zero_omega_zero_vz() {
        let state = Arc::new(RwLock::new(RobotState::default()));
        let (tx_sink, mut tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (_rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::M1, state, (64.0, 64.0, 0.0));
        dev.apply_velocity(0.45, 0.0).unwrap();

        let sent = tx_rx.recv().await.unwrap();
        assert_eq!(i16::from_le_bytes([sent[5], sent[6]]), 450, "vx = 450");
        assert_eq!(i16::from_le_bytes([sent[9], sent[10]]), 0, "vz 必须是 0，不能是 200");
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
        // Task 24 写者改造：origin 由 Robot::new 注入共享 state（测试里手动模拟），
        // mock 首帧 vx=0 零位移后 x/y 保持 origin（z 恒 0）
        let state = Arc::new(RwLock::new(RobotState::default()));
        {
            let mut g = state.write().await;
            g.x = 66.5;
            g.y = 63.25;
            g.z = 0.0;
        }
        let (tx_sink, _tx_rx) = mpsc::channel::<Vec<u8>>(32);
        let (rx_tx, rx_feed) = mpsc::channel::<Vec<u8>>(32);

        let (_dev, _handle) = STM32Device::spawn_mock(rx_feed, tx_sink, CarType::X3Plus, state.clone(), (66.5, 63.25, 0.0));

        // 喂一帧 vx=0 的 SPEED 帧触发字段级更新；无位移，x/y 应保持 origin
        let vx = 0i16;
        let mut data = vec![0u8; 7];
        data[0..2].copy_from_slice(&vx.to_le_bytes());
        let frame = stm32_report_frame(constants::RPT_SPEED, &data);
        rx_tx.send(frame).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let s = state.read().await;
        assert!((s.x - 66.5).abs() < 1e-3, "共享态 x 应保持 origin: {}", s.x);
        assert!((s.y - 63.25).abs() < 1e-3, "共享态 y 应保持 origin: {}", s.y);
        assert!((s.z - 0.0).abs() < 1e-3, "共享态 z 应为 0: {}", s.z);
    }
}

// ============================================================
// `vz` 反解标定（task_44 §二 Step 4 实施步骤 2；数值全部「待上车标定」）
// ============================================================

#[cfg(test)]
mod calib_tests {
    use super::*;

    /// §1.2 硬条件：`omega = 0` ⇒ `vz = 0`（**精确相等**，不能是死区 0.2）
    #[test]
    fn test_zero_omega_is_zero_vz() {
        assert_eq!(omega_to_vz(0.0, 0.0), 0.0);
        assert_eq!(omega_to_vz(0.45, 0.0), 0.0);
    }

    /// 原地转金标：`vz = 0.2 + 0.5/0.70 = 0.9142857`（≈ +914 线上单位）
    #[test]
    fn test_in_place_gold() {
        let vz = omega_to_vz(0.0, 0.5);
        assert!((vz - 0.9142857).abs() < 1e-5, "实际 {vz}");
        assert_eq!((vz * 1000.0) as i16, 914);
    }

    /// 边走边转金标：同一 omega 下 `vz = 0.2 + 0.5/1.96 = 0.4551020`（≈ +455），
    /// 且**必须小于**原地转的 vz（两套系数确实生效）
    #[test]
    fn test_moving_gold_differs() {
        let vz = omega_to_vz(0.45, 0.5);
        assert!((vz - 0.4551020).abs() < 1e-5, "实际 {vz}");
        assert_eq!((vz * 1000.0) as i16, 455);
        assert!(vz < omega_to_vz(0.0, 0.5), "边走边转的 vz 必须小于原地转（分两套系数）");
    }

    /// 符号：`vz` 与 `omega` 同号（`vz > 0` = 左转 = 逆时针），且左右对称
    #[test]
    fn test_sign_matches_omega() {
        assert!(omega_to_vz(0.0, 0.5) > 0.0);
        assert!(omega_to_vz(0.0, -0.5) < 0.0);
        assert_eq!(omega_to_vz(0.0, -0.5), -omega_to_vz(0.0, 0.5));
        assert_eq!(omega_to_vz(0.45, -0.5), -omega_to_vz(0.45, 0.5));
    }

    /// 上限：`|vz| ≤ 3.2`（≈120 °/s 原地），夹死不溢出
    #[test]
    fn test_cap_at_vz_max() {
        assert_eq!(omega_to_vz(0.0, 3.0), VZ_MAX);
        assert_eq!(omega_to_vz(0.0, -3.0), -VZ_MAX);
    }

    /// 全域扫描：两档 `v` × 一串 `omega` → 全部有限且不越界
    #[test]
    fn test_sweep_within_protocol_limit() {
        for v in [0.0_f32, 0.45] {
            for omega in [0.0_f32, 0.1, -0.1, 0.5, -0.5, 1.0, -1.0, 2.0, -2.0, 10.0, -10.0] {
                let vz = omega_to_vz(v, omega);
                assert!(vz.is_finite(), "v={v} omega={omega} 产出了非有限值");
                assert!(vz.abs() <= VZ_MAX, "v={v} omega={omega} → vz={vz} 越界");
            }
        }
    }
}
