//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-08-07

//! Orion 统一通信协议（MAVLink 风格帧 + 自定义消息）
//!
//! 设计文档：`docs/design_doc/orion_protocol.md`
//!
//! - `frame.rs`   帧编解码（magic/len/seq/sysid/compid/msgid/payload/checksum）
//! - `messages.rs` 5 条消息的 payload 编解码

pub mod frame;
pub mod messages;

pub use frame::{decode_frame, encode_frame, Frame};
pub use messages::{
    decode_manual_control, decode_map_delta, decode_map_full, decode_pose, decode_task_set,
    encode_manual_control, encode_map_delta, encode_map_full, encode_pose, encode_task_set,
    ACTION_BACKWARD, ACTION_BEEP, ACTION_FORWARD, ACTION_SPIN_LEFT, ACTION_SPIN_RIGHT,
    ACTION_START_LIDAR, ACTION_STOP, ACTION_STOP_LIDAR, ACTION_SWITCH_TO_AUTO,
    ACTION_SWITCH_TO_MANUAL, MISSION_GOTO, ManualControl, MapDeltaEntry, MissionItem, PoseData,
};

/// 消息 ID 常量（与协议文档 §3 一致）
pub const MSGID_POSE: u16 = 1;
pub const MSGID_MAP_FULL: u16 = 2;
pub const MSGID_MAP_DELTA: u16 = 3;
pub const MSGID_MANUAL_CONTROL: u16 = 4;
pub const MSGID_TASK_SET: u16 = 5;

/// compid 约定（协议文档 §4.3）
pub const COMPID_ROBOT: u8 = 1;
pub const COMPID_GROUND_STATION: u8 = 200;

/// 开机起算毫秒（协议时间戳 time_boot_ms 来源，2026-08-07 决策）
///
/// 本机单调时间（`Instant`），仅作数据时间标签；分布式系统无全局时钟，不做跨车比较。
pub fn now_boot_ms() -> u32 {
    static BOOT: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    BOOT.get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u32
}
