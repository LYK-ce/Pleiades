//Presented by KeJi
//Date ： 2026-06-17

//! STM32 串口协议实现
//!
//! 参照 ros_stm32_protocol.md 和 my_rossmaster.py。
//! 本模块为纯函数，不依赖串口，可独立单元测试。

use super::types::RobotState;

// ============================================================
// 协议常量
// ============================================================

/// 帧头
pub const HEAD: u8 = 0xFF;

/// 下发帧设备 ID（Jetson → STM32）
pub const DEV_ID_HOST: u8 = 0xFC;

/// 上报帧设备 ID（STM32 → Jetson）
pub const DEV_ID_STM32: u8 = 0xFB;

/// 校验和补码常数 = 257 - 0xFC = 5
const COMPLEMENT: u8 = 5;

// ---- 功能码（下发） ----

pub const FUNC_AUTO_REPORT: u8 = 0x01;
pub const FUNC_BEEP: u8 = 0x02;
pub const FUNC_PWM_SERVO: u8 = 0x03;
pub const FUNC_PWM_SERVO_ALL: u8 = 0x04;
pub const FUNC_RGB: u8 = 0x05;
pub const FUNC_RGB_EFFECT: u8 = 0x06;
pub const FUNC_MOTOR: u8 = 0x10;
pub const FUNC_CAR_RUN: u8 = 0x11;
pub const FUNC_MOTION: u8 = 0x12;
pub const FUNC_SET_MOTOR_PID: u8 = 0x13;
pub const FUNC_SET_CAR_TYPE: u8 = 0x15;
pub const FUNC_UART_SERVO: u8 = 0x20;
pub const FUNC_UART_SERVO_ID: u8 = 0x21;
pub const FUNC_ARM_CTRL: u8 = 0x23;
pub const FUNC_ARM_OFFSET: u8 = 0x24;
pub const FUNC_AKM_DEF_ANGLE: u8 = 0x30;
pub const FUNC_AKM_STEER: u8 = 0x31;
pub const FUNC_REQUEST_DATA: u8 = 0x50;
pub const FUNC_VERSION: u8 = 0x51;
pub const FUNC_RESET_STATE: u8 = 0x0F;
pub const FUNC_RESET_FLASH: u8 = 0xA0;

// ---- 上报功能码 ----

pub const RPT_SPEED: u8 = 0x0A;
pub const RPT_MPU_RAW: u8 = 0x0B;
pub const RPT_IMU_ATT: u8 = 0x0C;
pub const RPT_ENCODER: u8 = 0x0D;
pub const RPT_ICM_RAW: u8 = 0x0E;

// ============================================================
// 帧构建
// ============================================================

/// 构建下发帧（Jetson → STM32）
///
/// 自动填充 LEN 字段并计算校验和。
/// 算法：`frame[2] = len(frame) - 1`，`checksum = (sum(frame) + COMPLEMENT) & 0xFF`
pub fn build_host_frame(func: u8, data: &[u8]) -> Vec<u8> {
    let mut frame = vec![HEAD, DEV_ID_HOST, 0, func];
    frame.extend_from_slice(data);
    // LEN = 从 DEV_ID(索引1) 到 DATA 末尾的总字节数 = len(frame) - 1
    // Python: frame[2] = len(frame) - 1
    frame[2] = (frame.len() - 1) as u8;
    // 校验和 = (sum(全部字节) + COMPLEMENT) & 0xFF
    let csum = (frame.iter().map(|&b| b as u32).sum::<u32>() + COMPLEMENT as u32) as u8;
    frame.push(csum);
    frame
}

// ============================================================
// 校验和验证
// ============================================================

/// 验证上报帧校验和
///
/// 公式: `(LEN + FUNC + sum(DATA)) & 0xFF == CHKSUM`
pub fn verify_report(dev_id: u8, len: u8, func: u8, data: &[u8], chksum: u8) -> bool {
    if dev_id != DEV_ID_STM32 {
        return false;
    }
    let sum = len.wrapping_add(func).wrapping_add(data.iter().fold(0u8, |a, &b| a.wrapping_add(b)));
    sum == chksum
}

// ============================================================
// 接收状态机
// ============================================================

/// 接收状态机状态
enum RxState {
    /// 等待帧头 0xFF
    Head,
    /// 接收帧内容
    Data {
        idx: u8,
        expected_len: u8,
        func: u8,
        data: Vec<u8>,
    },
}

/// 接收状态机
///
/// 逐字节喂入，帧完整时返回 `(func, data)`。
/// 用法和 Python 的 `_rx_loop` 完全一致。
pub struct RxStateMachine {
    state: RxState,
}

impl RxStateMachine {
    pub fn new() -> Self {
        Self {
            state: RxState::Head,
        }
    }

    /// 喂入一个字节。帧完整时返回 `Some((func, data))`，否则返回 `None`。
    pub fn feed(&mut self, byte: u8) -> Option<(u8, Vec<u8>)> {
        // 先取出状态进行处理，避免借用冲突
        let mut current = RxState::Head;
        std::mem::swap(&mut self.state, &mut current);

        let result = match &mut current {
            RxState::Head => {
                if byte == HEAD {
                    self.state = RxState::Data {
                        idx: 0,
                        expected_len: 0,
                        func: 0,
                        data: Vec::new(),
                    };
                } else {
                    self.state = current;
                }
                None
            }
            RxState::Data {
                idx,
                expected_len,
                func,
                data,
            } => {
                let mut outcome = None;

                match *idx {
                    0 => {
                        if byte != DEV_ID_STM32 {
                            self.state = RxState::Head;
                            return None;
                        }
                    }
                    1 => {
                        *expected_len = byte;
                    }
                    2 => {
                        *func = byte;
                    }
                    n if n < *expected_len => {
                        data.push(byte);
                    }
                    n if n == *expected_len => {
                        let chk = byte;
                        let parsed_func = *func;
                        let parsed_len = *expected_len;

                        // data 包含所有实际数据字节（idx 3 到 expected_len-1，共 LEN-3 字节）
                        // chk 是第 expected_len 个字节（校验和），不在 data 中
                        let csum = parsed_len
                            .wrapping_add(parsed_func)
                            .wrapping_add(data.iter().fold(0u8, |a, &b| a.wrapping_add(b)));

                        outcome = if csum == chk {
                            Some((parsed_func, data.clone()))
                        } else {
                            None
                        };

                        // 将状态放回，然后返回
                        self.state = RxState::Head;
                        return outcome;
                    }
                    _ => {
                        self.state = RxState::Head;
                        return None;
                    }
                }
                *idx += 1;
                self.state = current;
                outcome
            }
        };
        result
    }

    /// 重置状态机
    pub fn reset(&mut self) {
        self.state = RxState::Head;
    }
}

impl Default for RxStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// 传感器数据解析
// ============================================================

/// 解析速度 + 电量上报 (0x0A, 7 bytes)
///
/// 返回 (vx, vy, vz, battery) —— 已归一化（vx/vy/vz: m/s, battery: V）
pub fn parse_speed(data: &[u8]) -> Option<(f32, f32, f32, f32)> {
    if data.len() < 7 {
        return None;
    }
    let vx = i16::from_le_bytes([data[0], data[1]]) as f32 / 1000.0;
    let vy = i16::from_le_bytes([data[2], data[3]]) as f32 / 1000.0;
    let vz = i16::from_le_bytes([data[4], data[5]]) as f32 / 1000.0;
    let battery = data[6] as f32 / 10.0;
    Some((vx, vy, vz, battery))
}

/// 解析姿态角上报 (0x0C, 6 bytes)
///
/// 返回 (roll, pitch, yaw) —— 已归一化为弧度
pub fn parse_imu_att(data: &[u8]) -> Option<(f32, f32, f32)> {
    if data.len() < 6 {
        return None;
    }
    let roll = i16::from_le_bytes([data[0], data[1]]) as f32 / 10000.0;
    let pitch = i16::from_le_bytes([data[2], data[3]]) as f32 / 10000.0;
    let yaw = i16::from_le_bytes([data[4], data[5]]) as f32 / 10000.0;
    Some((roll, pitch, yaw))
}

/// 解析编码器上报 (0x0D, 16 bytes)
///
/// 返回 [enc1, enc2, enc3, enc4] —— int32 原始值
pub fn parse_encoder(data: &[u8]) -> Option<[i32; 4]> {
    if data.len() < 16 {
        return None;
    }
    let enc1 = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let enc2 = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let enc3 = i32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let enc4 = i32::from_le_bytes([data[12], data[13], data[14], data[15]]);
    Some([enc1, enc2, enc3, enc4])
}

/// 解析 MPU9250 原始数据 (0x0B, 18 bytes)
///
/// 返回 (gx, gy, gz, ax, ay, az, mx, my, mz)
pub fn parse_mpu_raw(data: &[u8]) -> Option<(f32, f32, f32, f32, f32, f32, f32, f32, f32)> {
    if data.len() < 18 {
        return None;
    }
    let gyro_ratio = 1.0 / 3754.9;
    let gx = i16::from_le_bytes([data[0], data[1]]) as f32 * gyro_ratio;
    let gy = i16::from_le_bytes([data[2], data[3]]) as f32 * -gyro_ratio;
    let gz = i16::from_le_bytes([data[4], data[5]]) as f32 * -gyro_ratio;

    let accel_ratio = 1.0 / 1671.84;
    let ax = i16::from_le_bytes([data[6], data[7]]) as f32 * accel_ratio;
    let ay = i16::from_le_bytes([data[8], data[9]]) as f32 * accel_ratio;
    let az = i16::from_le_bytes([data[10], data[11]]) as f32 * accel_ratio;

    // 磁力计不需要缩放
    let mx = i16::from_le_bytes([data[12], data[13]]) as f32;
    let my = i16::from_le_bytes([data[14], data[15]]) as f32;
    let mz = i16::from_le_bytes([data[16], data[17]]) as f32;

    Some((gx, gy, gz, ax, ay, az, mx, my, mz))
}

/// 解析 ICM20948 原始数据 (0x0E)
///
/// 只解析陀螺仪部分（6 bytes），各轴直接 ÷1000
pub fn parse_icm_raw(data: &[u8]) -> Option<(f32, f32, f32)> {
    if data.len() < 6 {
        return None;
    }
    let ratio = 1.0 / 1000.0;
    let gx = i16::from_le_bytes([data[0], data[1]]) as f32 * ratio;
    let gy = i16::from_le_bytes([data[2], data[3]]) as f32 * ratio;
    let gz = i16::from_le_bytes([data[4], data[5]]) as f32 * ratio;
    Some((gx, gy, gz))
}

/// 根据功能码将解析结果写入 RobotState
pub fn update_state(state: &mut RobotState, func: u8, data: &[u8]) {
    match func {
        RPT_SPEED => {
            if let Some((vx, vy, vz, battery)) = parse_speed(data) {
                state.vx = vx;
                state.vy = vy;
                state.vz = vz;
                state.battery = battery;
            }
        }
        RPT_IMU_ATT => {
            if let Some((roll, pitch, yaw)) = parse_imu_att(data) {
                state.attitude.roll = roll;
                state.attitude.pitch = pitch;
                state.attitude.yaw = yaw;
            }
        }
        RPT_ENCODER => {
            if let Some(enc) = parse_encoder(data) {
                state.encoders = enc;
            }
        }
        RPT_MPU_RAW => {
            if let Some((gx, gy, gz, ax, ay, az, mx, my, mz)) = parse_mpu_raw(data) {
                state.gyro.gx = gx;
                state.gyro.gy = gy;
                state.gyro.gz = gz;
                state.accel.ax = ax;
                state.accel.ay = ay;
                state.accel.az = az;
                state.mag.mx = mx;
                state.mag.my = my;
                state.mag.mz = mz;
            }
        }
        RPT_ICM_RAW => {
            if let Some((gx, gy, gz)) = parse_icm_raw(data) {
                state.gyro.gx = gx;
                state.gyro.gy = gy;
                state.gyro.gz = gz;
            }
        }
        _ => {}
    }
}

// ============================================================
// 运动控制数据打包
// ============================================================

/// 打包 FUNC_CAR_RUN 数据: [car_type, state, speed_lo, speed_hi]
pub fn pack_car_run(car_type: u8, state: u8, speed: i16) -> Vec<u8> {
    let speed_bytes = speed.to_le_bytes();
    vec![car_type, state, speed_bytes[0], speed_bytes[1]]
}

/// 打包 FUNC_MOTION 数据: [car_type, vx_lo, vx_hi, vy_lo, vy_hi, vz_lo, vz_hi]
pub fn pack_motion(car_type: u8, vx: f32, vy: f32, vz: f32, limits: (f32, f32, f32)) -> Vec<u8> {
    let (vx_max, vy_max, vz_max) = limits;
    let vx = (vx.clamp(-vx_max, vx_max) * 1000.0) as i16;
    let vy = (vy.clamp(-vy_max, vy_max) * 1000.0) as i16;
    let vz = (vz.clamp(-vz_max, vz_max) * 1000.0) as i16;
    let vx_b = vx.to_le_bytes();
    let vy_b = vy.to_le_bytes();
    let vz_b = vz.to_le_bytes();
    vec![
        car_type,
        vx_b[0], vx_b[1],
        vy_b[0], vy_b[1],
        vz_b[0], vz_b[1],
    ]
}

/// 打包 FUNC_MOTOR 数据: [m1, m2, m3, m4]，范围 [-100, 100]，127=保持
pub fn pack_motor(m1: i8, m2: i8, m3: i8, m4: i8) -> Vec<u8> {
    let clamp = |v: i8| -> u8 {
        if v == 127 { 127 } else { v.clamp(-100, 100) as u8 }
    };
    vec![clamp(m1), clamp(m2), clamp(m3), clamp(m4)]
}

/// 打包 FUNC_BEEP 数据: [dur_lo, dur_hi]
pub fn pack_beep(duration_ms: u16) -> Vec<u8> {
    duration_ms.to_le_bytes().to_vec()
}

/// 打包 FUNC_PWM_SERVO 数据: [servo_id, angle]
pub fn pack_servo(servo_id: u8, angle: u8) -> Vec<u8> {
    vec![servo_id, angle.min(180)]
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ─── 帧构建测试 ───────────────────────────────────────

    #[test]
    fn test_build_host_frame_car_run_forward() {
        // 示例来自协议文档: FF FC 07 11 02 01 32 00 4D
        let data = pack_car_run(0x02, 0x01, 50);
        let frame = build_host_frame(FUNC_CAR_RUN, &data);
        assert_eq!(
            frame,
            vec![0xFF, 0xFC, 0x07, 0x11, 0x02, 0x01, 0x32, 0x00, 0x4D],
            "前进帧构建错误"
        );
    }

    #[test]
    fn test_build_host_frame_car_run_stop() {
        // CAR_RUN stop: [FF,FC,07,11,02,00,00,00,1A]
        let data = pack_car_run(0x02, 0x00, 0);
        let frame = build_host_frame(FUNC_CAR_RUN, &data);
        assert_eq!(
            frame,
            vec![0xFF, 0xFC, 0x07, 0x11, 0x02, 0x00, 0x00, 0x00, 0x1A],
            "停止帧构建错误"
        );
    }

    #[test]
    fn test_build_host_frame_motion() {
        // vx=500 (0.5m/s), data 7 bytes, LEN=10
        let data = pack_motion(0x02, 0.5, 0.0, 0.0, (0.7, 0.7, 3.2));
        let frame = build_host_frame(FUNC_MOTION, &data);
        assert_eq!(
            frame,
            vec![0xFF, 0xFC, 0x0A, 0x12, 0x02, 0xF4, 0x01, 0x00, 0x00, 0x00, 0x00, 0x13],
            "速度矢量帧构建错误"
        );
    }

    #[test]
    fn test_build_host_frame_beep() {
        // BEEP 50ms: [FF,FC,05,02,32,00,39]
        let data = pack_beep(50);
        let frame = build_host_frame(FUNC_BEEP, &data);
        assert_eq!(
            frame,
            vec![0xFF, 0xFC, 0x05, 0x02, 0x32, 0x00, 0x39],
            "蜂鸣器帧构建错误"
        );
    }

    #[test]
    fn test_build_host_frame_motor() {
        // 四轮 50%: [FF,FC,07,10,32,32,32,32,DF]
        let data = pack_motor(50, 50, 50, 50);
        let frame = build_host_frame(FUNC_MOTOR, &data);
        assert_eq!(
            frame,
            vec![0xFF, 0xFC, 0x07, 0x10, 0x32, 0x32, 0x32, 0x32, 0xDF],
            "四轮PWM帧构建错误"
        );
    }

    #[test]
    fn test_build_host_frame_pwm_servo() {
        // 四路全部 90°: [FF,FC,07,04,5A,5A,5A,5A,73]
        let data = vec![0x5A, 0x5A, 0x5A, 0x5A];
        let frame = build_host_frame(FUNC_PWM_SERVO_ALL, &data);
        assert_eq!(
            frame,
            vec![0xFF, 0xFC, 0x07, 0x04, 0x5A, 0x5A, 0x5A, 0x5A, 0x73],
            "舵机帧构建错误"
        );
    }

    // ─── 校验和验证 ───────────────────────────────────────

    #[test]
    fn test_verify_report_valid_speed() {
        // 模拟一帧速度上报: 0xFF 0xFB 0x0A 0x0A [7 bytes data] CHK
        // LEN=10, FUNC=0x0A, DATA=[vx_l, vx_h, vy_l, vy_h, vz_l, vz_h, bat]
        let data = vec![0xE8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x4A]; // vx=1000(1.0m/s), vy=0, vz=0, bat=74(7.4V)
        let chk = 0x0Au8
            .wrapping_add(0x0A)
            .wrapping_add(data.iter().fold(0u8, |a, &b| a.wrapping_add(b)));
        assert!(verify_report(DEV_ID_STM32, 0x0A, 0x0A, &data, chk));
    }

    #[test]
    fn test_verify_report_wrong_dev_id() {
        assert!(!verify_report(0xFC, 0x0A, 0x0A, &[0; 7], 0));
    }

    #[test]
    fn test_verify_report_bad_checksum() {
        assert!(!verify_report(DEV_ID_STM32, 0x0A, 0x0A, &[0; 7], 0xFF));
    }

    // ─── 接收状态机 ───────────────────────────────────────

    #[test]
    fn test_rx_state_machine_valid_speed_frame() {
        // 构造完整速度上报帧
        let data = vec![0xE8u8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x4A];
        let chk = 0x0Au8
            .wrapping_add(0x0A)
            .wrapping_add(data.iter().fold(0u8, |a, &b| a.wrapping_add(b)));

        let mut sm = RxStateMachine::new();

        // 喂入帧头
        assert!(sm.feed(HEAD).is_none());
        // 跳过垃圾字节不应该影响
        assert!(sm.feed(0xAA).is_none());
        // 重新开始
        assert!(sm.feed(HEAD).is_none());
        // 设备ID
        assert!(sm.feed(DEV_ID_STM32).is_none());
        // LEN
        assert!(sm.feed(0x0A).is_none());
        // FUNC
        assert!(sm.feed(0x0A).is_none());
        // DATA (7 bytes)
        for &b in &data {
            assert!(sm.feed(b).is_none());
        }
        // CHKSUM
        let result = sm.feed(chk);
        assert!(result.is_some(), "应该返回解析结果");
        let (func, parsed_data) = result.unwrap();
        assert_eq!(func, 0x0A);
        assert_eq!(parsed_data, data);
    }

    #[test]
    fn test_rx_state_machine_bad_checksum() {
        let data = vec![0u8; 7];
        let mut sm = RxStateMachine::new();

        sm.feed(HEAD);
        sm.feed(DEV_ID_STM32);
        sm.feed(0x0A);
        sm.feed(0x0A);
        for &b in &data {
            sm.feed(b);
        }
        // 错误的校验和
        let result = sm.feed(0xFF);
        assert!(result.is_none(), "错误校验和应返回 None");
    }

    #[test]
    fn test_rx_state_machine_wrong_dev_id() {
        let mut sm = RxStateMachine::new();
        sm.feed(HEAD);
        // 下发帧设备 ID → 接收状态机应拒绝
        sm.feed(DEV_ID_HOST);
        // 下一个字节... 状态机已重置到 Head
        // 需要重新开始
        let result = sm.feed(HEAD); // 这会被正常处理
        assert!(result.is_none());
    }

    // ─── 数据解析测试 ─────────────────────────────────────

    #[test]
    fn test_parse_speed() {
        // vx = 1000 (1.0 m/s), vy = 0, vz = 0, battery = 74 (7.4V)
        let data = vec![0xE8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x4A];
        let (vx, vy, vz, battery) = parse_speed(&data).unwrap();
        assert!((vx - 1.0).abs() < 0.01, "vx 应为 1.0，实际 {vx}");
        assert_eq!(vy, 0.0);
        assert_eq!(vz, 0.0);
        assert!((battery - 7.4).abs() < 0.1, "电池应为 7.4V，实际 {battery}");
    }

    #[test]
    fn test_parse_speed_too_short() {
        assert!(parse_speed(&[0; 6]).is_none());
    }

    #[test]
    fn test_parse_imu_att() {
        // roll=500(0.05rad≈2.86°), pitch=0, yaw=-300(-0.03rad)
        let data = vec![0xF4, 0x01, 0x00, 0x00, 0xD4, 0xFE];
        let (roll, pitch, yaw) = parse_imu_att(&data).unwrap();
        assert!((roll - 0.05).abs() < 0.001);
        assert_eq!(pitch, 0.0);
        assert!((yaw - (-0.03)).abs() < 0.001);
    }

    #[test]
    fn test_parse_encoder() {
        let data = vec![
            0x10, 0x27, 0x00, 0x00,  // 10000
            0xE8, 0x03, 0x00, 0x00,  // 1000
            0x00, 0x00, 0x00, 0x00,  // 0
            0xC8, 0x00, 0x00, 0x00,  // 200
        ];
        let enc = parse_encoder(&data).unwrap();
        assert_eq!(enc, [10000, 1000, 0, 200]);
    }

    #[test]
    fn test_update_state() {
        let mut state = RobotState::default();
        // 喂入速度数据
        let speed_data = vec![0xE8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x50]; // vx=1.0, bat=8.0V
        update_state(&mut state, RPT_SPEED, &speed_data);
        assert!((state.vx - 1.0).abs() < 0.01);
        assert!((state.battery - 8.0).abs() < 0.1);

        // 喂入姿态数据
        let att_data = vec![0xF4, 0x01, 0x00, 0x00, 0x00, 0x00]; // roll=0.05
        update_state(&mut state, RPT_IMU_ATT, &att_data);
        assert!((state.attitude.roll - 0.05).abs() < 0.001);

        // 速度不应被覆盖
        assert!((state.vx - 1.0).abs() < 0.01);
    }

    // ─── 数据打包测试 ─────────────────────────────────────

    #[test]
    fn test_pack_car_run() {
        let data = pack_car_run(0x02, 0x01, 50);
        assert_eq!(data, vec![0x02, 0x01, 0x32, 0x00]);
    }

    #[test]
    fn test_pack_motion_clamp() {
        // X3_PLUS 最大 vx=0.7，传 2.0 应被 clamp
        let data = pack_motion(0x02, 2.0, 0.0, 0.0, (0.7, 0.7, 3.2));
        // 0.7 * 1000 = 700 = 0x02BC → [BC, 02]
        assert_eq!(data, vec![0x02, 0xBC, 0x02, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_pack_motor() {
        let data = pack_motor(50, 50, 50, 50);
        assert_eq!(data, vec![50, 50, 50, 50]);
    }

    #[test]
    fn test_pack_motor_keep() {
        // 127 = 保持当前值不变
        let data = pack_motor(50, 127, -30, 127);
        assert_eq!(data, vec![50, 127, 226, 127]); // -30 as u8 = 226
    }

    // ─── LEN 计算回归测试 ─────────────────────────────────

    #[test]
    fn test_len_calculation_various_funcs() {
        // FUNC_CAR_RUN: 4 字节 data → LEN = 7
        let data = pack_car_run(0x02, 0x01, 50);
        let frame = build_host_frame(FUNC_CAR_RUN, &data);
        assert_eq!(frame[2], 7);

        // FUNC_MOTION: 7 字节 data → LEN = 10
        let data = pack_motion(0x02, 0.5, 0.0, 0.0, (0.7, 0.7, 3.2));
        let frame = build_host_frame(FUNC_MOTION, &data);
        assert_eq!(frame[2], 10);

        // FUNC_BEEP: 2 字节 data → LEN = 5
        let data = pack_beep(50);
        let frame = build_host_frame(FUNC_BEEP, &data);
        assert_eq!(frame[2], 5);

        // FUNC_MOTOR: 4 字节 data → LEN = 7
        let data = pack_motor(50, 50, 50, 50);
        let frame = build_host_frame(FUNC_MOTOR, &data);
        assert_eq!(frame[2], 7);
    }

    // ─── 传感器解析往返 ───────────────────────────────────

    #[test]
    fn test_speed_roundtrip_negative() {
        // 负速度: vx = -500 = -0.5 m/s
        let data = vec![0x0C, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x4A]; // vx=-500
        let (vx, vy, vz, bat) = parse_speed(&data).unwrap();
        assert!((vx - (-0.5)).abs() < 0.01);
        assert_eq!(vy, 0.0);
        assert_eq!(vz, 0.0);
    }

    #[test]
    fn test_attitude_roundtrip_negative() {
        // roll = -0.05 rad
        let data = vec![0x0C, 0xFE, 0x00, 0x00, 0x00, 0x00]; // roll=-500
        let (roll, _pitch, _yaw) = parse_imu_att(&data).unwrap();
        assert!((roll - (-0.05)).abs() < 0.001);
    }
}
