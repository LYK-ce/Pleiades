//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-08-07

//! 5 条消息的 payload 编解码（协议文档 §3）
//!
//! 所有数值大端（BE）。坐标/计数为 i32，与内部类型一致。

use super::frame::MAGIC;

// ============================================================
// 数据结构
// ============================================================

/// ORION_POSE 载荷（msgid 1，33 字节，Task 13_1：意图广播扩展）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseData {
    pub time_boot_ms: u32,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub yaw: f32,
    /// 意图有效标志（Task 13_1）：false = 无当前任务/子目标
    pub valid: bool,
    /// 意图：D* 寻路下一格（网格坐标），valid=false 时忽略
    pub sub_gx: i32,
    pub sub_gy: i32,
}

/// ORION_MAP_DELTA 条目（9 字节）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapDeltaEntry {
    pub gx: i32,
    pub gy: i32,
    pub state: u8,
}

/// ORION_MANUAL_CONTROL 载荷（msgid 4，3 字节）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ManualControl {
    pub action: u8,
    pub param: i16,
}

/// ORION_TASK_SET 任务项（9 字节）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionItem {
    pub mission_type: u8,
    pub x: f32,
    pub y: f32,
}

// ============================================================
// ORION_POSE (msgid 1)
// ============================================================

/// 编码位姿：time_boot_ms + x/y/vx/vy/yaw + valid + sub_gx/sub_gy（33 字节，Task 13_1）
pub fn encode_pose(pose: &PoseData) -> Vec<u8> {
    let mut buf = Vec::with_capacity(33);
    buf.extend_from_slice(&pose.time_boot_ms.to_be_bytes());
    buf.extend_from_slice(&pose.x.to_be_bytes());
    buf.extend_from_slice(&pose.y.to_be_bytes());
    buf.extend_from_slice(&pose.vx.to_be_bytes());
    buf.extend_from_slice(&pose.vy.to_be_bytes());
    buf.extend_from_slice(&pose.yaw.to_be_bytes());
    buf.push(pose.valid as u8);
    buf.extend_from_slice(&pose.sub_gx.to_be_bytes());
    buf.extend_from_slice(&pose.sub_gy.to_be_bytes());
    buf
}

/// 解码位姿 payload（需恰好 33 字节，Task 13_1）
pub fn decode_pose(payload: &[u8]) -> Option<PoseData> {
    if payload.len() != 33 {
        return None;
    }
    Some(PoseData {
        time_boot_ms: u32::from_be_bytes(payload[0..4].try_into().ok()?),
        x: f32::from_be_bytes(payload[4..8].try_into().ok()?),
        y: f32::from_be_bytes(payload[8..12].try_into().ok()?),
        vx: f32::from_be_bytes(payload[12..16].try_into().ok()?),
        vy: f32::from_be_bytes(payload[16..20].try_into().ok()?),
        yaw: f32::from_be_bytes(payload[20..24].try_into().ok()?),
        valid: payload[24] != 0,
        sub_gx: i32::from_be_bytes(payload[25..29].try_into().ok()?),
        sub_gy: i32::from_be_bytes(payload[29..33].try_into().ok()?),
    })
}

// ============================================================
// ORION_MAP_FULL (msgid 2)
// ============================================================

/// 编码全量地图（协议文档 §3.2）
///
/// `data` 为 width×height 字节，状态编码 0/100/255（与内部一致，零映射）
pub fn encode_map_full(
    time_boot_ms: u32,
    origin_gx: i32,
    origin_gy: i32,
    width: u16,
    height: u16,
    resolution: f32,
    data: &[u8],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(20 + data.len());
    buf.extend_from_slice(&time_boot_ms.to_be_bytes());
    buf.extend_from_slice(&origin_gx.to_be_bytes());
    buf.extend_from_slice(&origin_gy.to_be_bytes());
    buf.extend_from_slice(&width.to_be_bytes());
    buf.extend_from_slice(&height.to_be_bytes());
    buf.extend_from_slice(&resolution.to_be_bytes());
    buf.extend_from_slice(data);
    buf
}

// ============================================================
// ORION_MAP_DELTA (msgid 3)
// ============================================================

/// 编码增量地图：time_boot_ms + count + entries（每项 gx i32 + gy i32 + state i8）
///
/// `count` 字段为 u16，协议上限 **65535** 条目/帧（超出会截断导致接收端校验失败）
pub fn encode_map_delta(time_boot_ms: u32, entries: &[MapDeltaEntry]) -> Vec<u8> {
    debug_assert!(entries.len() <= u16::MAX as usize, "map_delta 条目数超出 u16 上限");
    let mut buf = Vec::with_capacity(6 + 9 * entries.len());
    buf.extend_from_slice(&time_boot_ms.to_be_bytes());
    buf.extend_from_slice(&(entries.len() as u16).to_be_bytes());
    for e in entries {
        buf.extend_from_slice(&e.gx.to_be_bytes());
        buf.extend_from_slice(&e.gy.to_be_bytes());
        buf.push(e.state);
    }
    buf
}

/// 解码增量地图 payload
pub fn decode_map_delta(payload: &[u8]) -> Option<(u32, Vec<MapDeltaEntry>)> {
    if payload.len() < 6 {
        return None;
    }
    let time_boot_ms = u32::from_be_bytes(payload[0..4].try_into().ok()?);
    let count = u16::from_be_bytes(payload[4..6].try_into().ok()?) as usize;
    let body = &payload[6..];
    if body.len() != 9 * count {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * 9;
        entries.push(MapDeltaEntry {
            gx: i32::from_be_bytes(body[off..off + 4].try_into().ok()?),
            gy: i32::from_be_bytes(body[off + 4..off + 8].try_into().ok()?),
            state: body[off + 8],
        });
    }
    Some((time_boot_ms, entries))
}

// ============================================================
// ORION_MANUAL_CONTROL (msgid 4)
// ============================================================

/// action 枚举（协议文档 §3.4）
pub const ACTION_FORWARD: u8 = 0;
pub const ACTION_BACKWARD: u8 = 1;
pub const ACTION_SPIN_LEFT: u8 = 2;
pub const ACTION_SPIN_RIGHT: u8 = 3;
pub const ACTION_STOP: u8 = 4;
pub const ACTION_BEEP: u8 = 5;
pub const ACTION_START_LIDAR: u8 = 6;
pub const ACTION_STOP_LIDAR: u8 = 7;
pub const ACTION_SWITCH_TO_MANUAL: u8 = 8;
pub const ACTION_SWITCH_TO_AUTO: u8 = 9;

/// 编码手动命令（3 字节）
pub fn encode_manual_control(action: u8, param: i16) -> Vec<u8> {
    let mut buf = Vec::with_capacity(3);
    buf.push(action);
    buf.extend_from_slice(&param.to_be_bytes());
    buf
}

/// 解码手动命令 payload（需恰好 3 字节）
pub fn decode_manual_control(payload: &[u8]) -> Option<ManualControl> {
    if payload.len() != 3 {
        return None;
    }
    Some(ManualControl {
        action: payload[0],
        param: i16::from_be_bytes([payload[1], payload[2]]),
    })
}

// ============================================================
// ORION_TASK_SET (msgid 5)
// ============================================================

/// mission type（协议文档 §3.5）
pub const MISSION_GOTO: u8 = 0;

/// 编码任务队列替换：count + missions（每项 type u8 + x f32 + y f32）
/// count = 0 表示取消全部任务（停车待命）
///
/// `count` 字段为 u8，协议上限 **255** 任务/帧（超出会截断导致接收端校验失败）
pub fn encode_task_set(missions: &[MissionItem]) -> Vec<u8> {
    debug_assert!(missions.len() <= u8::MAX as usize, "task_set 任务数超出 u8 上限");
    let mut buf = Vec::with_capacity(1 + 9 * missions.len());
    buf.push(missions.len() as u8);
    for m in missions {
        buf.push(m.mission_type);
        buf.extend_from_slice(&m.x.to_be_bytes());
        buf.extend_from_slice(&m.y.to_be_bytes());
    }
    buf
}

/// 解码任务队列 payload
pub fn decode_task_set(payload: &[u8]) -> Option<Vec<MissionItem>> {
    if payload.is_empty() {
        return None;
    }
    let count = payload[0] as usize;
    let body = &payload[1..];
    if body.len() != 9 * count {
        return None;
    }
    let mut missions = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * 9;
        missions.push(MissionItem {
            mission_type: body[off],
            x: f32::from_be_bytes(body[off + 1..off + 5].try_into().ok()?),
            y: f32::from_be_bytes(body[off + 5..off + 9].try_into().ok()?),
        });
    }
    Some(missions)
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pose_roundtrip() {
        let p = PoseData {
            time_boot_ms: 123456,
            x: 65.5,
            y: 63.25,
            vx: 0.3,
            vy: 0.0,
            yaw: 1.5708,
            valid: true,
            sub_gx: 130,
            sub_gy: 128,
        };
        let encoded = encode_pose(&p);
        assert_eq!(encoded.len(), 33);
        assert_eq!(decode_pose(&encoded).unwrap(), p);
        // valid=false（无意图）时 sub 坐标仍往返
        let p2 = PoseData { valid: false, sub_gx: 0, sub_gy: 0, ..p };
        assert_eq!(decode_pose(&encode_pose(&p2)).unwrap(), p2);
        // 长度不符（旧版 24B 帧）拒绝
        assert!(decode_pose(&encoded[..24]).is_none());
    }

    #[test]
    fn test_map_delta_roundtrip() {
        let entries = vec![
            MapDeltaEntry { gx: 130, gy: 128, state: 100 },
            MapDeltaEntry { gx: -5, gy: 300, state: 0 },
            MapDeltaEntry { gx: 0, gy: 0, state: 255 },
        ];
        let encoded = encode_map_delta(42, &entries);
        let (ts, decoded) = decode_map_delta(&encoded).unwrap();
        assert_eq!(ts, 42);
        assert_eq!(decoded, entries);
        // 空 entries 合法（count=0）
        let empty = encode_map_delta(1, &[]);
        assert_eq!(decode_map_delta(&empty).unwrap().1.len(), 0);
    }

    #[test]
    fn test_manual_control_roundtrip() {
        let encoded = encode_manual_control(ACTION_FORWARD, 30);
        assert_eq!(encoded.len(), 3);
        let mc = decode_manual_control(&encoded).unwrap();
        assert_eq!(mc.action, ACTION_FORWARD);
        assert_eq!(mc.param, 30);
        assert!(decode_manual_control(&encoded[..2]).is_none());
    }

    #[test]
    fn test_task_set_roundtrip() {
        let missions = vec![
            MissionItem { mission_type: MISSION_GOTO, x: 66.0, y: 64.0 },
            MissionItem { mission_type: MISSION_GOTO, x: 70.5, y: 60.0 },
        ];
        let encoded = encode_task_set(&missions);
        let decoded = decode_task_set(&encoded).unwrap();
        assert_eq!(decoded, missions);
        // 空任务 = 取消（count=0）
        let empty = encode_task_set(&[]);
        assert_eq!(empty.len(), 1);
        assert!(decode_task_set(&empty).unwrap().is_empty());
    }

    #[test]
    fn test_frame_with_message() {
        // 帧 + 消息组合：编码一条 POSE 消息并解码
        let p = PoseData { time_boot_ms: 1, x: 64.0, y: 64.0, vx: 0.0, vy: 0.0, yaw: 0.0, valid: false, sub_gx: 0, sub_gy: 0 };
        let payload = encode_pose(&p);
        // Task 13 阶段一：sysid 为变长字节（测试用单字节身份）
        let frame = super::super::frame::encode_frame(
            super::super::MSGID_POSE,
            &[7],
            super::super::COMPID_ROBOT,
            &payload,
        );
        assert_eq!(frame[0], MAGIC);
        let decoded = super::super::frame::decode_frame(&frame).unwrap();
        assert_eq!(decoded.msgid, super::super::MSGID_POSE);
        assert_eq!(decoded.sysid, vec![7]);
        assert_eq!(decode_pose(&decoded.payload).unwrap(), p);
    }
}
