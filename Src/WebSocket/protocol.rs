//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-12

//! WebSocket 协议编解码——ORION 协议帧解析
//!
//! 2026-08-07 协议统一：WS payload 统一为 ORION 帧（协议文档
//! `docs/design_doc/orion_protocol.md`）；旧 JSON 三层命令（mode/manual/auto）已移除。

use crate::robot::core::command::{AutoCmd, Command, ManualCmd, Mission, ModeCmd};
use crate::robot::core::protocol::{
    decode_manual_control, decode_task_set, Frame, ACTION_BACKWARD, ACTION_BEEP,
    ACTION_FORWARD, ACTION_SPIN_LEFT, ACTION_SPIN_RIGHT, ACTION_START_LIDAR, ACTION_STOP,
    ACTION_STOP_LIDAR, ACTION_SWITCH_TO_AUTO, ACTION_SWITCH_TO_MANUAL, MISSION_GOTO,
    MSGID_MANUAL_CONTROL, MSGID_TASK_SET,
};

/// 解析 ORION 帧 → 内部命令（WS 入站：终端 → 车）
///
/// Task 13_2：改收 `&Frame`（调用方已 decode，避免 65KB MAP_FULL 帧二次解析）
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
                0 => {
                    // 取消全部任务（replace 空队列；mission_count 忽略）
                    Some(Command::Auto(AutoCmd::Set(vec![])))
                }
                1 => {
                    // 单车：missions 过滤未知 type → 直接替换队列（老行为）
                    let list: Vec<Mission> = ts
                        .missions
                        .into_iter()
                        .filter_map(|m| match m.mission_type {
                            MISSION_GOTO => Some(Mission::Goto { x: m.x, y: m.y, members: vec![] }),
                            _ => {
                                tracing::warn!("[WS] 未知 mission type: {}", m.mission_type);
                                None
                            }
                        })
                        .collect();
                    Some(Command::Auto(AutoCmd::Set(list)))
                }
                _ => {
                    // 群发：遍历找第一个 MISSION_GOTO 为目标点，members 原样透传
                    match ts.missions.into_iter().find(|m| m.mission_type == MISSION_GOTO) {
                        Some(m) => Some(Command::Auto(AutoCmd::Set(vec![Mission::Goto {
                            x: m.x,
                            y: m.y,
                            members: ts.members,
                        }]))),
                        None => {
                            tracing::warn!("[WS] 群发任务无 Goto 目标");
                            None
                        }
                    }
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::core::protocol::{encode_task_set, Frame, MissionItem, TaskSetPayload};

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
        // member_count=0 → Set(vec![]) 取消
        let f = task_set_frame(&TaskSetPayload { members: vec![], missions: vec![] });
        assert_set(parse_orion_frame(&f), vec![]);
    }

    #[test]
    fn test_task_set_single() {
        // member_count=1 → 老行为：missions 直接替换，members 不进入 Mission
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
        // member_count>1 → 取第一个 Goto 为目标点，members 透传
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
        // 群发但 missions 无 Goto → None
        let f = task_set_frame(&TaskSetPayload {
            members: vec![vec![1u8; 38], vec![2u8; 38]],
            missions: vec![MissionItem { mission_type: 99, x: 1.0, y: 2.0 }],
        });
        assert!(parse_orion_frame(&f).is_none());
    }

    #[test]
    fn test_task_set_single_unknown_type_filtered() {
        // 单车分支过滤未知 mission type
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
}
