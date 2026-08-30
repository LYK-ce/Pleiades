//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-15

//! ORION 命令帧 → 内部 Command 解码
//!
//! Task 16：从 `Src/WebSocket/protocol.rs` 迁入，供车端 `command_consumer` 复用
//! （WebSocket 退役后此函数仍需存在，不能留在 WS 目录）。

use crate::robot::core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
use super::frame::Frame;
use super::{MSGID_MANUAL_CONTROL, MSGID_TASK_SET};
use super::messages::{
    decode_manual_control, decode_task_set, ACTION_BACKWARD, ACTION_BEEP, ACTION_FORWARD,
    ACTION_SPIN_LEFT, ACTION_SPIN_RIGHT, ACTION_START_LIDAR, ACTION_STOP, ACTION_STOP_LIDAR,
    ACTION_SWITCH_TO_AUTO, ACTION_SWITCH_TO_MANUAL, ACTION_TAKEOFF, ACTION_LAND,
    MISSION_CIRCLE, MISSION_GOTO,
};

/// 解析 ORION 帧 → 内部命令（入站：终端/地面站 → 车）
pub fn parse_orion_frame(frame: &Frame) -> Option<Command> {
    match frame.msgid {
        MSGID_MANUAL_CONTROL => {
            let mc = decode_manual_control(&frame.payload)?;
            manual_action_to_command(mc.action, mc.param)
        }
        MSGID_TASK_SET => {
            let ts = decode_task_set(&frame.payload)?;
            // Task 14 三分支（member_count）：0 取消 / 1 单车 / >1 群发
            match ts.members.len() {
                0 => Some(Command::Auto(AutoCmd::Set(vec![]))),
                1 => {
                    let list: Vec<Mission> = ts
                        .missions
                        .into_iter()
                        .filter_map(|m| match m.mission_type {
                            MISSION_GOTO => Some(Mission::Goto { x: m.x, y: m.y, members: vec![] }),
                            MISSION_CIRCLE => Some(Mission::Circle { x: m.x, y: m.y, members: vec![] }),
                            _ => {
                                tracing::warn!("[Cmd] 未知 mission type: {}", m.mission_type);
                                None
                            }
                        })
                        .collect();
                    Some(Command::Auto(AutoCmd::Set(list)))
                }
                _ => match ts
                    .missions
                    .into_iter()
                    .find(|m| m.mission_type == MISSION_GOTO || m.mission_type == MISSION_CIRCLE)
                {
                    Some(m) if m.mission_type == MISSION_GOTO => {
                        Some(Command::Auto(AutoCmd::Set(vec![Mission::Goto {
                            x: m.x,
                            y: m.y,
                            members: ts.members,
                        }])))
                    }
                    Some(m) => Some(Command::Auto(AutoCmd::Set(vec![Mission::Circle {
                        x: m.x,
                        y: m.y,
                        members: ts.members,
                    }]))),
                    None => {
                        tracing::warn!("[Cmd] 群发任务无 Goto/Circle 目标");
                        None
                    }
                },
            }
        }
        other => {
            tracing::warn!("[Cmd] 未知 ORION msgid: {other}");
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
        ACTION_TAKEOFF => Some(Command::Manual(ManualCmd::Takeoff)),
        ACTION_LAND => Some(Command::Manual(ManualCmd::Land)),
        other => {
            tracing::warn!("[Cmd] 未知 manual action: {other}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::robot::core::command::{AutoCmd, Command, Mission};
    use crate::robot::core::protocol::command_decode::parse_orion_frame;
    use crate::robot::core::protocol::frame::Frame;
    use crate::robot::core::protocol::MSGID_TASK_SET;
    use crate::robot::core::protocol::messages::{
        encode_task_set, MissionItem, MISSION_CIRCLE, MISSION_GOTO, TaskSetPayload,
    };

    fn task_set_frame(payload: &TaskSetPayload) -> Frame {
        Frame {
            msgid: MSGID_TASK_SET,
            sysid: vec![],
            compid: 200,
            payload: encode_task_set(payload),
        }
    }

    fn assert_set(cmd: Option<Command>, expected: Vec<Mission>) {
        match cmd {
            Some(Command::Auto(AutoCmd::Set(list))) => assert_eq!(list, expected),
            other => panic!("预期 AutoCmd::Set，实际 {other:?}"),
        }
    }

    #[test]
    fn test_task_set_cancel() {
        let f = task_set_frame(&TaskSetPayload { members: vec![], missions: vec![] });
        assert_set(parse_orion_frame(&f), vec![]);
    }

    #[test]
    fn test_task_set_single() {
        let f = task_set_frame(&TaskSetPayload {
            members: vec![vec![7]],
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 66.0, y: 64.0 },
                MissionItem { mission_type: MISSION_GOTO, x: 70.5, y: 60.0 },
            ],
        });
        assert_set(
            parse_orion_frame(&f),
            vec![
                Mission::Goto { x: 66.0, y: 64.0, members: vec![] },
                Mission::Goto { x: 70.5, y: 60.0, members: vec![] },
            ],
        );
    }

    #[test]
    fn test_task_set_group() {
        let members = vec![vec![1u8; 38], vec![2u8; 38]];
        let f = task_set_frame(&TaskSetPayload {
            members: members.clone(),
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 64.0, y: 63.0 },
                MissionItem { mission_type: MISSION_GOTO, x: 10.0, y: 10.0 },
            ],
        });
        assert_set(
            parse_orion_frame(&f),
            vec![Mission::Goto { x: 64.0, y: 63.0, members }],
        );
    }

    #[test]
    fn test_task_set_group_no_goto() {
        let f = task_set_frame(&TaskSetPayload {
            members: vec![vec![1u8; 38], vec![2u8; 38]],
            missions: vec![MissionItem { mission_type: 99, x: 1.0, y: 2.0 }],
        });
        assert!(parse_orion_frame(&f).is_none());
    }

    #[test]
    fn test_task_set_single_unknown_type_filtered() {
        let f = task_set_frame(&TaskSetPayload {
            members: vec![vec![7]],
            missions: vec![
                MissionItem { mission_type: 99, x: 1.0, y: 2.0 },
                MissionItem { mission_type: MISSION_GOTO, x: 8.0, y: 9.0 },
            ],
        });
        assert_set(
            parse_orion_frame(&f),
            vec![Mission::Goto { x: 8.0, y: 9.0, members: vec![] }],
        );
    }

    #[test]
    fn test_task_set_single_circle() {
        let f = task_set_frame(&TaskSetPayload {
            members: vec![vec![7]],
            missions: vec![MissionItem { mission_type: MISSION_CIRCLE, x: 64.0, y: 64.0 }],
        });
        assert_set(
            parse_orion_frame(&f),
            vec![Mission::Circle { x: 64.0, y: 64.0, members: vec![] }],
        );
    }

    #[test]
    fn test_task_set_group_circle() {
        let members = vec![vec![1u8; 38], vec![2u8; 38]];
        let f = task_set_frame(&TaskSetPayload {
            members: members.clone(),
            missions: vec![MissionItem { mission_type: MISSION_CIRCLE, x: 64.0, y: 64.0 }],
        });
        assert_set(
            parse_orion_frame(&f),
            vec![Mission::Circle { x: 64.0, y: 64.0, members }],
        );
    }
}
