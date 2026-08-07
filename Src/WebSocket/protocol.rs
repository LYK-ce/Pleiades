//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-07

//! WebSocket 协议编解码——ORION 协议帧解析
//!
//! 2026-08-07 协议统一：WS payload 统一为 ORION 帧（协议文档
//! `docs/design_doc/orion_protocol.md`）；旧 JSON 三层命令（mode/manual/auto）已移除。

use crate::robot::core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
use crate::robot::core::protocol::{
    decode_frame, decode_manual_control, decode_task_set, ACTION_BACKWARD, ACTION_BEEP,
    ACTION_FORWARD, ACTION_SPIN_LEFT, ACTION_SPIN_RIGHT, ACTION_START_LIDAR, ACTION_STOP,
    ACTION_STOP_LIDAR, ACTION_SWITCH_TO_AUTO, ACTION_SWITCH_TO_MANUAL, MISSION_GOTO,
    MSGID_MANUAL_CONTROL, MSGID_TASK_SET,
};

/// 解析 ORION 帧 → 内部命令（WS 入站：终端 → 车）
pub fn parse_orion_frame(bytes: &[u8]) -> Option<Command> {
    let frame = decode_frame(bytes)?;
    match frame.msgid {
        MSGID_MANUAL_CONTROL => {
            let mc = decode_manual_control(&frame.payload)?;
            manual_action_to_command(mc.action, mc.param)
        }
        MSGID_TASK_SET => {
            let missions = decode_task_set(&frame.payload)?;
            let list: Vec<Mission> = missions
                .into_iter()
                .filter_map(|m| match m.mission_type {
                    MISSION_GOTO => Some(Mission::Goto(m.x, m.y)),
                    _ => {
                        tracing::warn!("[WS] 未知 mission type: {}", m.mission_type);
                        None
                    }
                })
                .collect();
            Some(Command::Auto(AutoCmd::Set(list)))
        }
        other => {
            tracing::warn!("[WS] 未知 ORION msgid: {other}");
            None
        }
    }
}

/// action 枚举 → 内部命令（未知动作保守返回 None）
fn manual_action_to_command(action: u8, param: i16) -> Option<Command> {
    match action {
        ACTION_FORWARD => Some(Command::Manual(ManualCmd::Forward(param))),
        ACTION_BACKWARD => Some(Command::Manual(ManualCmd::Backward(param))),
        ACTION_SPIN_LEFT => Some(Command::Manual(ManualCmd::SpinLeft(param))),
        ACTION_SPIN_RIGHT => Some(Command::Manual(ManualCmd::SpinRight(param))),
        ACTION_STOP => Some(Command::Manual(ManualCmd::Stop)),
        ACTION_BEEP => Some(Command::Manual(ManualCmd::Beep(param as u16))),
        ACTION_START_LIDAR => Some(Command::Manual(ManualCmd::StartLidarScan)),
        ACTION_STOP_LIDAR => Some(Command::Manual(ManualCmd::StopLidarScan)),
        ACTION_SWITCH_TO_MANUAL => Some(Command::Mode(ModeCmd::SwitchToManual)),
        ACTION_SWITCH_TO_AUTO => Some(Command::Mode(ModeCmd::SwitchToAuto)),
        other => {
            tracing::warn!("[WS] 未知 manual action: {other}");
            None
        }
    }
}
