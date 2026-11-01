//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-11-01

//! Orion 统一通信协议（MAVLink 风格帧 + 自定义消息）
//!
//! 设计文档：`docs/design_doc/orion_protocol.md`
//!
//! - `frame.rs`   帧编解码（magic/len/seq/sysid/compid/msgid/payload/checksum）
//! - `messages.rs` 6 条消息的 payload 编解码

pub mod frame;
pub mod messages;
pub mod command_decode;

pub use frame::{decode_frame, encode_frame, Frame};
pub use command_decode::parse_orion_frame;
pub use messages::{
    decode_assist_request, decode_manual_control, decode_map_delta, decode_map_full, decode_mode,
    decode_pose, decode_task_set, encode_assist_request, encode_manual_event, encode_manual_motion,
    encode_map_delta, encode_map_full, encode_mode, encode_pose, encode_task_set,
    ACTION_BEEP, ACTION_LAND, ACTION_START_LIDAR, ACTION_STOP_LIDAR, ACTION_TAKEOFF,
    MANUAL_EVENT_LEN, MANUAL_MOTION_LEN, MODE_AUTO, MODE_LEN, MODE_MANUAL, AssistRequest,
    MISSION_FOLLOW, MISSION_GOTO, ManualPayload, MapDeltaEntry, MissionItem, PoseData,
    TaskSetPayload,
};

/// 消息 ID 常量（与协议文档 §3 一致）
pub const MSGID_POSE: u16 = 1;
pub const MSGID_MAP_FULL: u16 = 2;
pub const MSGID_MAP_DELTA: u16 = 3;
pub const MSGID_MANUAL_CONTROL: u16 = 4;
pub const MSGID_TASK_SET: u16 = 5;
pub const MSGID_RTCM: u16 = 6;
/// 协助请求（UAV → 地面站；**非下行命令**，地面站不回复，task_41 §5.3）
pub const MSGID_ASSIST_REQUEST: u16 = 7;
/// 模式切换（终端 → 车，payload 恒 1 B；task_44 §1.2 #4）
///
/// ⚠️ 解码必须映射到 `Command::Mode`，**不得**映射成 `Command::Manual`——
/// 它要绕过 `main_loop` 的手动模式门闩（否则「从手动切回自动」会被自己拦住）。
pub const MSGID_MODE: u16 = 8;

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
