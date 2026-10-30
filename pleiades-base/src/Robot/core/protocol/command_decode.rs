//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-10-30

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
    MISSION_CONTAINMENT, MISSION_FOLLOW, MISSION_GOTO,
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
            // Task 36 §12.3.4 单分支：`mission_count == 0` **才是**取消（唯一取消入口）；
            // `> 0` 时逐条入队，未知 type 该条 warn 丢弃；**全部被丢 → 整帧不生效（不动队列）**
            if ts.missions.is_empty() {
                return Some(Command::Auto(AutoCmd::Set(vec![])));
            }
            let list: Vec<Mission> = ts
                .missions
                .into_iter()
                .filter_map(|m| match m.mission_type {
                    MISSION_GOTO => Some(Mission::Goto { x: m.x, y: m.y }),
                    // Task 37 §五.2：type=2 = Follow（`x/y` 预留，原样透传）
                    MISSION_FOLLOW => Some(Mission::Follow { x: m.x, y: m.y }),
                    // task_41 §3.2：type=3 = Containment（`half_side` 已在 decode 侧校验）
                    MISSION_CONTAINMENT => Some(Mission::Containment {
                        x: m.x,
                        y: m.y,
                        half_side: m.half_side,
                    }),
                    other => {
                        // `decode_task_set` 已对未知 type 整帧拒 → 此处理论不可达
                        tracing::warn!(
                            "[Cmd] 未知 mission type: {other}（decode_task_set 已整帧拒，理论不可达）"
                        );
                        None
                    }
                })
                .collect();
            if list.is_empty() {
                tracing::warn!("[Cmd] TASK_SET 全部任务类型未知，整帧不生效（队列不动）");
                return None;
            }
            Some(Command::Auto(AutoCmd::Set(list)))
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
    use crate::robot::core::protocol::messages::{
        encode_task_set, MissionItem, MISSION_CONTAINMENT, MISSION_FOLLOW, MISSION_GOTO,
        TaskSetPayload,
    };
    use crate::robot::core::protocol::MSGID_TASK_SET;

    /// 用 encoder 造帧（**只给合法 type**：未知 type 必须手写 raw，见 `raw_task_set_frame`）
    fn task_set_frame(payload: &TaskSetPayload) -> Frame {
        Frame {
            msgid: MSGID_TASK_SET,
            sysid: vec![],
            compid: 200,
            payload: encode_task_set(payload),
        }
    }

    /// 直接给定 payload 字节（构造旧布局 / 非法长度 / 未知 type 帧）
    fn raw_task_set_frame(payload: Vec<u8>) -> Frame {
        Frame { msgid: MSGID_TASK_SET, sysid: vec![], compid: 200, payload }
    }

    /// 一条 9 B 记录（Goto / Follow）：`type | x | y`
    fn rec9(t: u8, x: f32, y: f32) -> Vec<u8> {
        let mut v = vec![t];
        v.extend_from_slice(&x.to_be_bytes());
        v.extend_from_slice(&y.to_be_bytes());
        v
    }

    /// 一条 13 B 记录（Containment）：`type | cx | cy | half`
    fn rec13(x: f32, y: f32, half_side: f32) -> Vec<u8> {
        let mut v = vec![MISSION_CONTAINMENT];
        v.extend_from_slice(&x.to_be_bytes());
        v.extend_from_slice(&y.to_be_bytes());
        v.extend_from_slice(&half_side.to_be_bytes());
        v
    }

    fn assert_set(cmd: Option<Command>, expected: Vec<Mission>) {
        match cmd {
            Some(Command::Auto(AutoCmd::Set(list))) => assert_eq!(list, expected),
            other => panic!("预期 AutoCmd::Set，实际 {other:?}"),
        }
    }

    #[test]
    fn test_task_set_cancel() {
        // count == 0 → 取消（唯一入口）
        let f = task_set_frame(&TaskSetPayload { missions: vec![] });
        assert_set(parse_orion_frame(&f), vec![]);
    }

    #[test]
    fn test_task_set_all_goto_queued() {
        // 逐条入队（不再"群发只取第一条"）
        let f = task_set_frame(&TaskSetPayload {
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 66.0, y: 64.0, half_side: 0.0 },
                MissionItem { mission_type: MISSION_GOTO, x: 70.5, y: 60.0, half_side: 0.0 },
            ],
        });
        assert_set(
            parse_orion_frame(&f),
            vec![
                Mission::Goto { x: 66.0, y: 64.0 },
                Mission::Goto { x: 70.5, y: 60.0 },
            ],
        );
    }

    #[test]
    fn test_task_set_old_layout_frame_rejected() {
        // 旧布局（含 member 段）13 字节 → 首条 type=1（退役 Circle）未知 → 整帧 None
        let mut old = vec![1u8, 1, 1, 7];
        old.push(MISSION_GOTO);
        old.extend_from_slice(&64.0f32.to_be_bytes());
        old.extend_from_slice(&64.0f32.to_be_bytes());
        assert!(parse_orion_frame(&raw_task_set_frame(old)).is_none());
    }

    #[test]
    fn test_task_set_bad_length_rejected() {
        // count=1 要求 10 字节（1 + 9）
        let mut short = vec![1u8];
        short.extend_from_slice(&[0u8; 8]);
        assert!(parse_orion_frame(&raw_task_set_frame(short)).is_none(), "少 1 字节 → None");
        let mut long = vec![1u8];
        long.extend_from_slice(&[0u8; 10]);
        assert!(parse_orion_frame(&raw_task_set_frame(long)).is_none(), "多 1 字节 → None");
    }

    #[test]
    fn test_task_set_follow_decoded() {
        // Task 37 §五.2：type=2 → Mission::Follow（x/y 预留，原样透传）
        let f = task_set_frame(&TaskSetPayload {
            missions: vec![MissionItem { mission_type: MISSION_FOLLOW, x: 0.0, y: 0.0, half_side: 0.0 }],
        });
        assert_set(parse_orion_frame(&f), vec![Mission::Follow { x: 0.0, y: 0.0 }]);
    }

    #[test]
    fn test_task_set_containment_decoded() {
        // task_41 §3.2：type=3 → Mission::Containment（x/y 中心 + half_side）
        let f = task_set_frame(&TaskSetPayload {
            missions: vec![MissionItem {
                mission_type: MISSION_CONTAINMENT,
                x: 64.0,
                y: 64.0,
                half_side: 8.0,
            }],
        });
        assert_set(
            parse_orion_frame(&f),
            vec![Mission::Containment { x: 64.0, y: 64.0, half_side: 8.0 }],
        );
    }

    #[test]
    fn test_task_set_mixed_three_types() {
        // 三种 type 同帧：顺序与值都不变（9/13/9 → 总长 33）
        let f = task_set_frame(&TaskSetPayload {
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 10.0, y: 11.0, half_side: 0.0 },
                MissionItem {
                    mission_type: MISSION_CONTAINMENT,
                    x: 64.0,
                    y: 64.0,
                    half_side: 8.0,
                },
                MissionItem { mission_type: MISSION_FOLLOW, x: 0.0, y: 0.0, half_side: 0.0 },
            ],
        });
        assert_eq!(f.payload.len(), 1 + 9 + 13 + 9);
        assert_set(
            parse_orion_frame(&f),
            vec![
                Mission::Goto { x: 10.0, y: 11.0 },
                Mission::Containment { x: 64.0, y: 64.0, half_side: 8.0 },
                Mission::Follow { x: 0.0, y: 0.0 },
            ],
        );
    }

    #[test]
    fn test_task_set_old_circle_type_rejected() {
        // type=1 是 Task 36 已删除的旧 Circle：长度表里没有 → **整帧拒**（队列不动）
        let mut raw = vec![1u8];
        raw.extend_from_slice(&rec9(1, 1.0, 2.0));
        assert_eq!(raw.len(), 10);
        assert!(parse_orion_frame(&raw_task_set_frame(raw)).is_none(), "旧 type=1 → 整帧拒");
    }

    #[test]
    fn test_task_set_mixed_old_circle_rejected() {
        // 混合帧：旧 type=1 无法知道长度 → **从"滤掉"变成"整帧拒"**（task_41 §3.2）
        let mut raw = vec![2u8];
        raw.extend_from_slice(&rec9(1, 1.0, 2.0));
        raw.extend_from_slice(&rec9(MISSION_GOTO, 8.0, 9.0));
        assert_eq!(raw.len(), 19);
        assert!(parse_orion_frame(&raw_task_set_frame(raw)).is_none(), "[1, Goto] → 整帧拒");
    }

    #[test]
    fn test_task_set_unknown_type_rejected() {
        // 未知 type=99：单独的帧与混合帧都整帧拒（不再逐条丢）
        let mut single = vec![1u8];
        single.extend_from_slice(&rec9(99, 1.0, 2.0));
        assert!(parse_orion_frame(&raw_task_set_frame(single)).is_none(), "未知 type 单条 → 拒");

        let mut mixed = vec![2u8];
        mixed.extend_from_slice(&rec9(99, 1.0, 2.0));
        mixed.extend_from_slice(&rec9(MISSION_GOTO, 8.0, 9.0));
        assert_eq!(mixed.len(), 19);
        assert!(parse_orion_frame(&raw_task_set_frame(mixed)).is_none(), "[99, Goto] → 整帧拒");
    }

    #[test]
    fn test_task_set_bad_half_side_rejected() {
        // half_side 非有限 / <= 0 → decode 整帧拒（`Command` 根本不产生）
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            let mut raw = vec![1u8];
            raw.extend_from_slice(&rec13(64.0, 64.0, bad));
            assert_eq!(raw.len(), 14);
            assert!(parse_orion_frame(&raw_task_set_frame(raw)).is_none(), "half_side={bad} → 拒");
        }
    }

    #[test]
    fn test_task_set_max_count() {
        // 255 条上限：编解码一致、不 panic
        let missions = vec![
            MissionItem { mission_type: MISSION_GOTO, x: 1.0, y: 2.0, half_side: 0.0 };
            255
        ];
        let f = task_set_frame(&TaskSetPayload { missions });
        match parse_orion_frame(&f) {
            Some(Command::Auto(AutoCmd::Set(list))) => assert_eq!(list.len(), 255),
            other => panic!("预期 255 条 Goto，实际 {other:?}"),
        }
    }
}
