//Presented by KeJi
//Created Date ： 2026-08-25
//Modified Date ： 2026-08-25

//! MAVLink 帧解析 + 命令打包
//!
//! 从 UAV 仓库 `rust-mavlink-test` 的 `mavlink_device.rs` 迁移：
//! - RX：`parse_one_frame` + `handle_message`（帧提取 + 解析 → `Telemetry`）
//! - TX：`*_msg` 命令打包（构造 `MavMessage`，序列化由 `serialize_message` 完成）

use std::io::Cursor;

use mavlink::dialects::ardupilotmega::{
    MavAutopilot, MavCmd, MavFrame, MavMessage, MavModeFlag, MavState, MavType,
    PositionTargetTypemask, ATTITUDE_DATA, COMMAND_LONG_DATA, EKF_STATUS_REPORT_DATA,
    GLOBAL_POSITION_INT_DATA, GPS_RAW_INT_DATA, HEARTBEAT_DATA, LOCAL_POSITION_NED_DATA,
    SET_POSITION_TARGET_LOCAL_NED_DATA, SYS_STATUS_DATA,
};
use mavlink::peek_reader::PeekReader;
use mavlink::{MavHeader, MessageData, MAVLinkV2MessageRaw};
use tracing::info;

use super::constants::*;
use super::types::Telemetry;

// ============================================================
// RX：帧提取 + 解析
// ============================================================

const STX_V1: u8 = 0xFE;
const STX_V2: u8 = 0xFD;
const MAVLINK_IFLAG_SIGNED: u8 = 0x01;

/// 从字节缓冲里提取一个完整 MAVLink 帧并解析。无完整帧返回 None；坏帧自动丢弃并重同步。
pub fn parse_one_frame(buf: &mut Vec<u8>) -> Option<(MavHeader, MavMessage)> {
    loop {
        if buf.len() < 2 {
            return None;
        }
        match buf[0] {
            STX_V2 => {
                if buf.len() < 3 {
                    return None;
                }
                let payload_len = buf[1] as usize;
                let signed = (buf[2] & MAVLINK_IFLAG_SIGNED) != 0;
                // v2: STX(1)+len(1)+incompat(1)+compat(1)+seq(1)+sys(1)+comp(1)+msgid(3)+payload+crc(2)
                let frame_len = 12 + payload_len + if signed { 13 } else { 0 };
                if buf.len() < frame_len {
                    return None;
                }
                let frame: Vec<u8> = buf[..frame_len].to_vec();
                buf.drain(..frame_len);
                let mut r = PeekReader::new(Cursor::new(&frame));
                match mavlink::read_v2_msg::<MavMessage, _>(&mut r) {
                    Ok(v) => return Some(v),
                    Err(_) => continue, // 校验/解析失败，丢弃这一帧
                }
            }
            STX_V1 => {
                let payload_len = buf[1] as usize;
                // v1: STX(1)+len(1)+seq(1)+sys(1)+comp(1)+msgid(1)+payload+crc(2)
                let frame_len = 8 + payload_len;
                if buf.len() < frame_len {
                    return None;
                }
                let frame: Vec<u8> = buf[..frame_len].to_vec();
                buf.drain(..frame_len);
                let mut r = PeekReader::new(Cursor::new(&frame));
                match mavlink::read_v1_msg::<MavMessage, _>(&mut r) {
                    Ok(v) => return Some(v),
                    Err(_) => continue,
                }
            }
            _ => {
                buf.remove(0); // 噪声字节，重新同步
            }
        }
    }
}

/// 消息 → 遥测状态
pub fn handle_message(t: &mut Telemetry, header: MavHeader, msg: MavMessage) {
    t.system_id = header.system_id;
    t.component_id = header.component_id;

    match msg {
        MavMessage::HEARTBEAT(data) => {
            t.heartbeat_count += 1;
            let was_armed = t.armed;
            t.armed = data.base_mode.contains(MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED);
            t.custom_mode = data.custom_mode;
            if t.heartbeat_count == 1 || was_armed != t.armed {
                info!(
                    "HEARTBEAT: sys={} comp={} armed={} custom_mode={}",
                    header.system_id, header.component_id, t.armed, t.custom_mode
                );
            }
        }
        MavMessage::ATTITUDE(data) => {
            t.attitude_count += 1;
            t.roll = data.roll;
            t.pitch = data.pitch;
            t.yaw = data.yaw;
        }
        MavMessage::SYS_STATUS(data) => {
            t.sys_status_count += 1;
            t.battery_voltage = data.voltage_battery as f32 / 1000.0; // mV -> V
        }
        MavMessage::GLOBAL_POSITION_INT(data) => {
            t.global_pos_count += 1;
            t.relative_alt = data.relative_alt as f32 / 1000.0; // mm -> m
            t.lat = data.lat;
            t.lon = data.lon;
            t.vel_vx = data.vx as f32 / 100.0; // cm/s -> m/s
            t.vel_vy = data.vy as f32 / 100.0;
            t.vel_vz = data.vz as f32 / 100.0;
            t.hdg = data.hdg as f32 / 100.0; // cdeg -> deg
        }
        MavMessage::LOCAL_POSITION_NED(data) => {
            t.local_pos_count += 1;
            t.local_x = data.x;
            t.local_y = data.y;
            t.local_z = data.z;
            t.local_vx = data.vx;
            t.local_vy = data.vy;
            t.local_vz = data.vz;
        }
        MavMessage::EKF_STATUS_REPORT(data) => {
            t.ekf_count += 1;
            t.ekf_flags = data.flags.bits();
        }
        MavMessage::GPS_RAW_INT(data) => {
            t.gps_count += 1;
            t.fix_type = data.fix_type as u32;
            t.satellites_visible = data.satellites_visible;
        }
        MavMessage::STATUSTEXT(data) => {
            t.statustext_count += 1;
            info!("STATUSTEXT: {}", data.text.to_str().unwrap_or(""));
        }
        _ => {}
    }
}

// ============================================================
// TX：命令打包（构造 MavMessage，不含发送）
// ============================================================

/// 把一条 MAVLink 消息序列化为 MAVLink2 字节。
pub fn serialize_message(msg: &MavMessage, system_id: u8, component_id: u8) -> Vec<u8> {
    let mut header = MavHeader::default();
    header.system_id = system_id;
    header.component_id = component_id;

    let mut raw = MAVLinkV2MessageRaw::new();
    raw.serialize_message(header, msg);
    raw.raw_bytes().to_vec()
}

/// HEARTBEAT（标识本机为 GCS）
pub fn heartbeat_msg() -> MavMessage {
    MavMessage::HEARTBEAT(HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: MavType::MAV_TYPE_GCS,
        autopilot: MavAutopilot::MAV_AUTOPILOT_INVALID,
        base_mode: MavModeFlag::empty(),
        system_status: MavState::MAV_STATE_ACTIVE,
        mavlink_version: 3,
    })
}

/// 请求关键遥测流：姿态 10Hz / 位置 5Hz / 状态与 GPS 2Hz / EKF 2Hz
pub fn telemetry_stream_msgs(target_system: u8, target_component: u8) -> Vec<MavMessage> {
    let streams = [
        (ATTITUDE_DATA::ID, 10u32),
        (GLOBAL_POSITION_INT_DATA::ID, 5u32),
        (LOCAL_POSITION_NED_DATA::ID, 5u32),
        (SYS_STATUS_DATA::ID, 2u32),
        (GPS_RAW_INT_DATA::ID, 2u32),
        (EKF_STATUS_REPORT_DATA::ID, 2u32),
    ];
    streams
        .iter()
        .map(|(id, hz)| {
            MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
                param1: *id as f32,
                param2: (1_000_000 / hz) as f32,
                param3: 0.0,
                param4: 0.0,
                param5: 0.0,
                param6: 0.0,
                param7: 0.0,
                command: MavCmd::MAV_CMD_SET_MESSAGE_INTERVAL,
                target_system,
                target_component,
                confirmation: 0,
            })
        })
        .collect()
}

/// 切换模式命令（param1=1 = MAV_MODE_FLAG_CUSTOM_MODE_ENABLED，param2=模式号）
pub fn set_mode_msg(target_system: u8, target_component: u8, mode: u32) -> MavMessage {
    MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        param1: 1.0,
        param2: mode as f32,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
        command: MavCmd::MAV_CMD_DO_SET_MODE,
        target_system,
        target_component,
        confirmation: 0,
    })
}

/// 解锁/上锁命令（arm=true 解锁，false 上锁；上锁用强制上锁魔法数字）
pub fn arm_disarm_msg(target_system: u8, target_component: u8, arm: bool) -> MavMessage {
    MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        param1: if arm { 1.0 } else { 0.0 },
        param2: if arm { 0.0 } else { FORCE_DISARM_MAGIC },
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
        command: MavCmd::MAV_CMD_COMPONENT_ARM_DISARM,
        target_system,
        target_component,
        confirmation: 0,
    })
}

/// 起飞命令（固定 TARGET_ALT）
pub fn takeoff_msg(target_system: u8, target_component: u8) -> MavMessage {
    MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        param1: 0.0,
        param2: 0.0,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: TARGET_ALT,
        command: MavCmd::MAV_CMD_NAV_TAKEOFF,
        target_system,
        target_component,
        confirmation: 0,
    })
}

/// 降落命令（原地降落，飞控自主触地 + 上锁）
pub fn land_msg(target_system: u8, target_component: u8) -> MavMessage {
    MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        param1: 0.0,
        param2: 0.0,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
        command: MavCmd::MAV_CMD_NAV_LAND,
        target_system,
        target_component,
        confirmation: 0,
    })
}

/// 速度目标指令（机体系 BODY_NED，type_mask 只保留速度 + yaw_rate）
///
/// ⚠️ 速度指令会超时（约 2~3s 无新指令回落到悬停），保持 loop 里持续下发（~10Hz）。
#[allow(deprecated)]
pub fn velocity_msg(
    target_system: u8,
    target_component: u8,
    vx: f32,
    vy: f32,
    vz: f32,
    yaw_rate: f32,
) -> MavMessage {
    let type_mask = PositionTargetTypemask::POSITION_TARGET_TYPEMASK_X_IGNORE
        | PositionTargetTypemask::POSITION_TARGET_TYPEMASK_Y_IGNORE
        | PositionTargetTypemask::POSITION_TARGET_TYPEMASK_Z_IGNORE
        | PositionTargetTypemask::POSITION_TARGET_TYPEMASK_AX_IGNORE
        | PositionTargetTypemask::POSITION_TARGET_TYPEMASK_AY_IGNORE
        | PositionTargetTypemask::POSITION_TARGET_TYPEMASK_AZ_IGNORE
        | PositionTargetTypemask::POSITION_TARGET_TYPEMASK_VZ_IGNORE
        | PositionTargetTypemask::POSITION_TARGET_TYPEMASK_YAW_IGNORE;
    MavMessage::SET_POSITION_TARGET_LOCAL_NED(SET_POSITION_TARGET_LOCAL_NED_DATA {
        time_boot_ms: 0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        vx,
        vy,
        vz,
        afx: 0.0,
        afy: 0.0,
        afz: 0.0,
        yaw: 0.0,
        yaw_rate,
        type_mask,
        target_system,
        target_component,
        coordinate_frame: MavFrame::MAV_FRAME_BODY_NED,
    })
}
