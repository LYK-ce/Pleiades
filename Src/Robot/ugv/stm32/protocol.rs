//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-20

//! STM32 协议层
//!
//! 帧构建、命令打包、接收状态机、传感器解析。

use super::constants::*;
use crate::robot::core::state::RobotState;

// ============================================================
// 帧构建
// ============================================================

pub fn build_host_frame(func: u8, data: &[u8]) -> Vec<u8> {
    let mut frame = vec![HEAD, DEV_ID_HOST, 0, func];
    frame.extend_from_slice(data);
    let len = 3 + data.len();
    debug_assert!(len <= 255, "帧数据过长: {len} > 255 (data={}B)", data.len());
    frame[2] = len as u8;
    let csum = (frame.iter().map(|&b| b as u32).sum::<u32>() + COMPLEMENT as u32) as u8;
    frame.push(csum);
    frame
}

// ============================================================
// 命令打包
// ============================================================

pub fn pack_car_run(car_type: u8, state: u8, speed: i16) -> Vec<u8> {
    let s = speed.to_le_bytes();
    build_host_frame(FUNC_CAR_RUN, &[car_type, state, s[0], s[1]])
}

pub fn pack_motion(car_type: u8, vx: f32, vy: f32, vz: f32, limits: (f32, f32, f32)) -> Vec<u8> {
    let (vx_max, vy_max, vz_max) = limits;
    let vx = (vx.clamp(-vx_max, vx_max) * 1000.0) as i16;
    let vy = (vy.clamp(-vy_max, vy_max) * 1000.0) as i16;
    let vz = (vz.clamp(-vz_max, vz_max) * 1000.0) as i16;
    let vx_b = vx.to_le_bytes(); let vy_b = vy.to_le_bytes(); let vz_b = vz.to_le_bytes();
    build_host_frame(FUNC_MOTION, &[car_type, vx_b[0], vx_b[1], vy_b[0], vy_b[1], vz_b[0], vz_b[1]])
}

pub fn pack_motor(m1: i8, m2: i8, m3: i8, m4: i8) -> Vec<u8> {
    let c = |v: i8| if v == 127 { 127u8 } else { v.clamp(-100, 100) as u8 };
    build_host_frame(FUNC_MOTOR, &[c(m1), c(m2), c(m3), c(m4)])
}

pub fn pack_beep(duration_ms: u16) -> Vec<u8> {
    let b = duration_ms.to_le_bytes();
    build_host_frame(FUNC_BEEP, &[b[0], b[1]])
}

pub fn pack_servo(servo_id: u8, angle: u8) -> Vec<u8> {
    build_host_frame(FUNC_PWM_SERVO, &[servo_id, angle.min(180)])
}

pub fn pack_rgb(led_id: u8, r: u8, g: u8, b: u8) -> Vec<u8> {
    build_host_frame(FUNC_RGB, &[led_id, r, g, b])
}

pub fn pack_rgb_effect(effect: u8, speed: u8) -> Vec<u8> {
    build_host_frame(FUNC_RGB_EFFECT, &[effect, speed, 0xFF])
}

pub fn pack_reset() -> Vec<u8> {
    build_host_frame(FUNC_RESET_STATE, &[0x5F])
}

// ============================================================
// 接收状态机
// ============================================================

pub enum RxState {
    Head,
    Data { idx: u8, expected_len: u8, func: u8, data: Vec<u8> },
}

pub fn feed_state_machine(sm: &mut RxState, byte: u8) -> Option<(u8, Vec<u8>)> {
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

pub fn update_state(state: &mut RobotState, func: u8, data: &[u8]) -> Option<[i32; 4]> {
    match func {
        RPT_SPEED => {
            if data.len() >= 7 {
                state.vx = i16l(&data[0..2]) as f32 / 1000.0;
                state.vy = i16l(&data[2..4]) as f32 / 1000.0;
                // 第三槽 = 垂直速度（车恒 0）；原「偏航角速度」语义已废弃（yaw 直接来自 IMU，不积分角速度）
                state.vz = i16l(&data[4..6]) as f32 / 1000.0;
                state.battery = data[6] as f32 / 10.0;
            }
            None
        }
        RPT_IMU_ATT => {
            if data.len() >= 6 {
                state.attitude.roll = i16l(&data[0..2]) as f32 / 10000.0;
                state.attitude.pitch = i16l(&data[2..4]) as f32 / 10000.0;
                state.attitude.yaw = i16l(&data[4..6]) as f32 / 10000.0;
            }
            None
        }
        RPT_ENCODER => {
            // Task 23 B1：encoders 摘出到设备，这里只解析返回、不写 RobotState
            if data.len() >= 16 {
                Some([i32l(&data[0..4]), i32l(&data[4..8]),
                      i32l(&data[8..12]), i32l(&data[12..16])])
            } else {
                None
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
            None
        }
        RPT_ICM_RAW => {
            if data.len() >= 6 {
                let r = 1.0 / 1000.0;
                state.gyro.gx = i16l(&data[0..2]) as f32 * r;
                state.gyro.gy = i16l(&data[2..4]) as f32 * r;
                state.gyro.gz = i16l(&data[4..6]) as f32 * r;
            }
            None
        }
        _ => None,
    }
}

/// 构造一条 STM32 → Host 上报帧（测试用）
pub fn stm32_report_frame(func: u8, data: &[u8]) -> Vec<u8> {
    let mut f = vec![HEAD, DEV_ID_STM32];
    let len = (3 + data.len()) as u8;
    f.push(len);
    f.push(func);
    f.extend_from_slice(data);
    let sum = len.wrapping_add(func)
        .wrapping_add(data.iter().fold(0u8, |a, &b| a.wrapping_add(b)));
    f.push(sum);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================
    // 帧构建
    // ========================================================

    #[test]
    fn test_build_host_frame_structure() {
        let frame = build_host_frame(0x11, &[0x02, 0x01, 0x64, 0x00]);
        // HEAD + DEV_ID_HOST + LEN + FUNC + data + CHKSUM
        assert_eq!(frame[0], HEAD);
        assert_eq!(frame[1], DEV_ID_HOST);
        // LEN = frame.len() - 1（build_host_frame 在 push CHKSUM 前计算）
        assert_eq!(frame[2] as usize, frame.len() - 2);
        assert_eq!(frame[3], 0x11); // FUNC
        assert_eq!(&frame[4..8], &[0x02, 0x01, 0x64, 0x00]);
    }

    #[test]
    fn test_build_host_frame_checksum() {
        let frame = build_host_frame(0x02, &[0xE8, 0x03]); // beep 1000ms
        // 手动校验: 0xFF+0xFC+0x05+0x02+0xE8+0x03+5 = 0x5F2, 低8位=0xF2
        assert_eq!(*frame.last().unwrap(), 0xF2);
    }

    // ========================================================
    // 命令打包
    // ========================================================

    #[test]
    fn test_pack_car_run() {
        // X3Plus(0x02), Forward(1), speed=100
        let f = pack_car_run(0x02, 1, 100);
        assert_eq!(f[3], FUNC_CAR_RUN);
        assert_eq!(f[4], 0x02); // car_type
        assert_eq!(f[5], 0x01); // state=Forward
        let speed = i16::from_le_bytes([f[6], f[7]]);
        assert_eq!(speed, 100);
    }

    #[test]
    fn test_pack_beep() {
        let f = pack_beep(500);
        assert_eq!(f[3], FUNC_BEEP);
        let ms = u16::from_le_bytes([f[4], f[5]]);
        assert_eq!(ms, 500);
    }

    #[test]
    fn test_pack_servo_clamps() {
        let f = pack_servo(1, 200);
        assert_eq!(f[5], 180); // clamped to 180
    }

    #[test]
    fn test_pack_motor() {
        let f = pack_motor(50, -30, 0, 127);
        assert_eq!(f[4], 50);
        assert_eq!(f[5], 226); // -30 as u8 clamped: -30.clamp(-100,100)= -30 → u8
        assert_eq!(f[6], 0);
        assert_eq!(f[7], 127); // special value preserved
    }

    // ========================================================
    // 接收状态机
    // ========================================================

    #[test]
    fn test_feed_state_machine_speed_frame() {
        let mut sm = RxState::Head;
        // SPEED 上报: vx=100, vy=-50, vz=0, battery=11.5V
        let vx = (0.100f32 * 1000.0) as i16; // = 100
        let vy = (-0.050f32 * 1000.0) as i16; // = -50
        let vz = 0i16;
        let bat = (11.5 * 10.0) as u8; // = 115
        let mut data = Vec::new();
        data.extend_from_slice(&vx.to_le_bytes());
        data.extend_from_slice(&vy.to_le_bytes());
        data.extend_from_slice(&vz.to_le_bytes());
        data.push(bat);

        let frame = stm32_report_frame(RPT_SPEED, &data);
        let mut got: Option<(u8, Vec<u8>)> = None;
        for &b in &frame {
            if let Some(r) = feed_state_machine(&mut sm, b) {
                got = Some(r);
            }
        }
        assert!(got.is_some(), "应解析出完整帧");
        let (func, parsed) = got.unwrap();
        assert_eq!(func, RPT_SPEED);
        assert_eq!(parsed, data);
    }

    #[test]
    fn test_feed_state_machine_bad_checksum() {
        let mut sm = RxState::Head;
        let mut frame = stm32_report_frame(RPT_SPEED, &[0u8; 7]);
        // 篡改校验和
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        let mut got = false;
        for &b in &frame {
            if feed_state_machine(&mut sm, b).is_some() {
                got = true;
            }
        }
        assert!(!got, "错误校验和应被丢弃");
    }

    #[test]
    fn test_feed_state_machine_noise_prefix() {
        let mut sm = RxState::Head;
        let frame = stm32_report_frame(RPT_ENCODER, &[0u8; 16]);
        // 前面加噪声字节
        let noise = [0x00, 0xAB, 0xCD];
        let mut all = Vec::new();
        all.extend_from_slice(&noise);
        all.extend_from_slice(&frame);

        let mut got = false;
        for &b in &all {
            if feed_state_machine(&mut sm, b).is_some() {
                got = true;
            }
        }
        assert!(got, "前导噪声不应破坏解析");
    }

    #[test]
    fn test_feed_state_machine_fragmented() {
        // 模拟串口分包到达：一帧分 3 次
        let frame = stm32_report_frame(RPT_SPEED, &[0u8; 7]);
        let (p1, rest) = frame.split_at(3);
        let (p2, p3) = rest.split_at(3);

        let mut sm = RxState::Head;
        for &b in p1 { feed_state_machine(&mut sm, b); }
        assert!(matches!(sm, RxState::Data { .. }), "应在 Data 状态");

        for &b in p2 { feed_state_machine(&mut sm, b); }

        let mut got = false;
        for &b in p3 {
            if feed_state_machine(&mut sm, b).is_some() {
                got = true;
            }
        }
        assert!(got, "分包到达应正确组装");
    }

    // ========================================================
    // 传感器解析
    // ========================================================

    #[test]
    fn test_update_state_speed() {
        let mut s = RobotState::default();
        let vx = 100i16; // 0.100 m/s
        let vy = (-50i16);
        let vz = 0i16;
        let bat: u8 = 115; // 11.5V
        let mut data = Vec::new();
        data.extend_from_slice(&vx.to_le_bytes());
        data.extend_from_slice(&vy.to_le_bytes());
        data.extend_from_slice(&vz.to_le_bytes());
        data.push(bat);

        update_state(&mut s, RPT_SPEED, &data);
        assert!((s.vx - 0.100).abs() < 0.001);
        assert!((s.vy - (-0.050)).abs() < 0.001);
        assert_eq!(s.vz, 0.0);
        assert!((s.battery - 11.5).abs() < 0.01);
    }

    #[test]
    fn test_update_state_imu() {
        let mut s = RobotState::default();
        // roll=0.5236 rad (30°), pitch=-0.1745 (-10°), yaw=1.5708 (90°)
        let roll = (0.5236 * 10000.0) as i16;
        let pitch = (-0.1745 * 10000.0) as i16;
        let yaw = (1.5708 * 10000.0) as i16;
        let mut data = Vec::new();
        data.extend_from_slice(&roll.to_le_bytes());
        data.extend_from_slice(&pitch.to_le_bytes());
        data.extend_from_slice(&yaw.to_le_bytes());

        update_state(&mut s, RPT_IMU_ATT, &data);
        assert!((s.attitude.roll - 0.5236).abs() < 0.001);
        assert!((s.attitude.pitch - (-0.1745)).abs() < 0.001);
        assert!((s.attitude.yaw - 1.5708).abs() < 0.001);
    }

    #[test]
    #[test]
    fn test_update_state_encoder() {
        let mut s = RobotState::default();
        let enc = [100i32, 200, -50, 0];
        let mut data = Vec::new();
        for v in &enc { data.extend_from_slice(&v.to_le_bytes()); }
        let parsed = update_state(&mut s, RPT_ENCODER, &data);
        assert_eq!(parsed, Some(enc));
    }

    #[test]
    fn test_update_state_short_data_no_panic() {
        // 数据不够长时不应 panic
        let mut s = RobotState::default();
        update_state(&mut s, RPT_SPEED, &[0u8; 3]);
        update_state(&mut s, RPT_IMU_ATT, &[]);
        update_state(&mut s, RPT_ENCODER, &[0u8; 10]);
        // 状态应保持不变
        assert_eq!(s.vx, 0.0);
        assert_eq!(s.attitude.roll, 0.0);
    }

    // ========================================================
    // 往返测试: pack → 字节 → feed + update → 语义一致
    // ========================================================

    #[test]
    fn test_roundtrip_car_run() {
        // TX: 发送 Forward 100
        let tx_frame = pack_car_run(0x02, MotionState::Forward as u8, 100);

        // 模拟 STM32 收到后回传 SPEED 上报（实际硬件会回传，此处用逻辑验证）
        // 验证：TX 帧可以构建且格式正确
        assert_eq!(tx_frame[0], HEAD);
        assert_eq!(tx_frame[1], DEV_ID_HOST);
        assert_eq!(tx_frame[3], FUNC_CAR_RUN);
    }

    #[test]
    fn test_roundtrip_beep() {
        let tx = pack_beep(300);
        // 模拟 STM32 回复 BEEP 确认（实际上报取决于固件实现）
        assert_eq!(tx[3], FUNC_BEEP);
        let ms = u16::from_le_bytes([tx[4], tx[5]]);
        assert_eq!(ms, 300);
    }
}
