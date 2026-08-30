//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-08-20

//! 5 条消息的 payload 编解码（协议文档 §3）
//!
//! 所有数值大端（BE）。坐标/计数为 i32，与内部类型一致。

use super::frame::MAGIC;
use crate::robot::core::grid::CHUNK_SIZE;

// ============================================================
// 数据结构
// ============================================================

/// ORION_POSE 载荷（msgid 1，37 字节，Task 22：加 z）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseData {
    pub time_boot_ms: u32,
    pub x: f32,
    pub y: f32,
    /// 垂直高度 (m)（车恒 0，机写飞控 EKF）
    pub z: f32,
    pub vx: f32,
    pub vy: f32,
    pub yaw: f32,
    /// 意图有效标志（Task 13_1）：false = 无当前任务/子目标
    pub valid: bool,
    /// 意图：D* 寻路下一格（网格坐标），valid=false 时忽略
    pub sub_gx: i32,
    pub sub_gy: i32,
}

/// ORION_MAP_DELTA 条目（9 字节；Task 13_2：state 三态 → delta 数值差分）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapDeltaEntry {
    pub gx: i32,
    pub gy: i32,
    pub delta: i8,
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

/// ORION_TASK_SET 载荷（Task 14：members 群发扩展）
///
/// 协议布局：mission_count u8 + member_count u8 + members[](len u8 + peer_id) + missions[]
/// - member_count == 0 → 取消全部任务（老 count=0 取消语义）
/// - member_count == 1 → 单车任务（missions 直接替换本地队列）
/// - member_count > 1  → 群发任务（每车自算散布位置，取第一个 Goto 为目标点）
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSetPayload {
    /// 参与车辆 peer_id 列表（帧序；排序由分配层负责）
    pub members: Vec<Vec<u8>>,
    /// 任务列表（MISSION_GOTO 等）
    pub missions: Vec<MissionItem>,
}

// ============================================================
// ORION_POSE (msgid 1)
// ============================================================

/// 编码位姿：time_boot_ms + x/y/z/vx/vy/yaw + valid + sub_gx/sub_gy（37 字节，Task 22）
pub fn encode_pose(pose: &PoseData) -> Vec<u8> {
    let mut buf = Vec::with_capacity(37);
    buf.extend_from_slice(&pose.time_boot_ms.to_be_bytes());
    buf.extend_from_slice(&pose.x.to_be_bytes());
    buf.extend_from_slice(&pose.y.to_be_bytes());
    buf.extend_from_slice(&pose.z.to_be_bytes());
    buf.extend_from_slice(&pose.vx.to_be_bytes());
    buf.extend_from_slice(&pose.vy.to_be_bytes());
    buf.extend_from_slice(&pose.yaw.to_be_bytes());
    buf.push(pose.valid as u8);
    buf.extend_from_slice(&pose.sub_gx.to_be_bytes());
    buf.extend_from_slice(&pose.sub_gy.to_be_bytes());
    buf
}

/// 解码位姿 payload（需恰好 37 字节，Task 22）
pub fn decode_pose(payload: &[u8]) -> Option<PoseData> {
    if payload.len() != 37 {
        return None;
    }
    Some(PoseData {
        time_boot_ms: u32::from_be_bytes(payload[0..4].try_into().ok()?),
        x: f32::from_be_bytes(payload[4..8].try_into().ok()?),
        y: f32::from_be_bytes(payload[8..12].try_into().ok()?),
        z: f32::from_be_bytes(payload[12..16].try_into().ok()?),
        vx: f32::from_be_bytes(payload[16..20].try_into().ok()?),
        vy: f32::from_be_bytes(payload[20..24].try_into().ok()?),
        yaw: f32::from_be_bytes(payload[24..28].try_into().ok()?),
        valid: payload[28] != 0,
        sub_gx: i32::from_be_bytes(payload[29..33].try_into().ok()?),
        sub_gy: i32::from_be_bytes(payload[33..37].try_into().ok()?),
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

/// 解码全量地图 payload（与 `encode_map_full` 对称，Task 13_2）
///
/// 布局：`time_boot_ms(4) | origin_gx(4) | origin_gy(4) | width(2) | height(2) | resolution(4) | data(65536)`
/// data 为 i8 log-odds 的 u8 位模式（与编码零转换，逐字节 `as i8`）。
///
/// 宽松校验（2026-08-12 决策）：仅做结构性长度检查（20 + 65536）；
/// origin/width/height/resolution 读出但不校验——当前全系统 origin 恒 (0,0)、256×256、0.5m，
/// 多 chunk / 偏移 origin 场景留后续完善。
pub fn decode_map_full(
    payload: &[u8],
) -> Option<(u32, i32, i32, u16, u16, f32, Box<[i8; CHUNK_SIZE * CHUNK_SIZE]>)> {
    if payload.len() != 20 + CHUNK_SIZE * CHUNK_SIZE {
        return None;
    }
    let time_boot_ms = u32::from_be_bytes(payload[0..4].try_into().ok()?);
    let origin_gx = i32::from_be_bytes(payload[4..8].try_into().ok()?);
    let origin_gy = i32::from_be_bytes(payload[8..12].try_into().ok()?);
    let width = u16::from_be_bytes(payload[12..14].try_into().ok()?);
    let height = u16::from_be_bytes(payload[14..16].try_into().ok()?);
    let resolution = f32::from_be_bytes(payload[16..20].try_into().ok()?);
    let mut data = Box::new([0i8; CHUNK_SIZE * CHUNK_SIZE]);
    for (i, &b) in payload[20..].iter().enumerate() {
        data[i] = b as i8; // u8 位模式 → i8，数值语义在接收方（阈值 ±6 派生三态）
    }
    Some((time_boot_ms, origin_gx, origin_gy, width, height, resolution, data))
}

// ============================================================
// ORION_MAP_DELTA (msgid 3)
// ============================================================

/// 编码增量地图：time_boot_ms + count + entries（每项 gx i32 + gy i32 + delta i8）
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
        buf.push(e.delta as u8); // i8 位模式直传（与 log-odds 字节一致）
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
            delta: body[off + 8] as i8,
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
pub const ACTION_TAKEOFF: u8 = 10; // Task 22_4：起飞
pub const ACTION_LAND: u8 = 11;    // Task 22_4：降落

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
/// 围圈（Task 18）：x/y 为圆心，车在环上均匀铺开
pub const MISSION_CIRCLE: u8 = 1;

/// 编码任务队列：mission_count + member_count + members[] + missions[]
///
/// member_count == 0 表示取消全部任务（停车待命）。
/// `mission_count` / `member_count` 为 u8，协议上限 255（超出会截断导致接收端校验失败）。
pub fn encode_task_set(payload: &TaskSetPayload) -> Vec<u8> {
    debug_assert!(payload.missions.len() <= u8::MAX as usize, "task_set 任务数超出 u8 上限");
    debug_assert!(payload.members.len() <= u8::MAX as usize, "task_set 成员数超出 u8 上限");
    let mut buf = Vec::with_capacity(2 + 9 * payload.missions.len());
    buf.push(payload.missions.len() as u8);
    buf.push(payload.members.len() as u8);
    for m in &payload.members {
        debug_assert!(m.len() <= u8::MAX as usize, "peer_id 超出 u8 上限");
        buf.push(m.len() as u8);
        buf.extend_from_slice(m);
    }
    for m in &payload.missions {
        buf.push(m.mission_type);
        buf.extend_from_slice(&m.x.to_be_bytes());
        buf.extend_from_slice(&m.y.to_be_bytes());
    }
    buf
}

/// 解码任务队列 payload（Task 14 布局）
///
/// 校验：长度 ≥ 2；members 每项 `off+len` 越界 → None（防恶意帧）；missions 剩余严格 `9×mission_count`。
pub fn decode_task_set(payload: &[u8]) -> Option<TaskSetPayload> {
    if payload.len() < 2 {
        return None;
    }
    let mission_count = payload[0] as usize;
    let member_count = payload[1] as usize;
    let mut off = 2usize;

    let mut members = Vec::with_capacity(member_count);
    for _ in 0..member_count {
        if off >= payload.len() {
            return None;
        }
        let len = payload[off] as usize;
        off += 1;
        if off + len > payload.len() {
            return None;
        }
        members.push(payload[off..off + len].to_vec());
        off += len;
    }

    let body = &payload[off..];
    if body.len() != 9 * mission_count {
        return None;
    }
    let mut missions = Vec::with_capacity(mission_count);
    for i in 0..mission_count {
        let o = i * 9;
        missions.push(MissionItem {
            mission_type: body[o],
            x: f32::from_be_bytes(body[o + 1..o + 5].try_into().ok()?),
            y: f32::from_be_bytes(body[o + 5..o + 9].try_into().ok()?),
        });
    }
    Some(TaskSetPayload { members, missions })
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
            z: 1.5,
            vx: 0.3,
            vy: 0.0,
            yaw: 1.5708,
            valid: true,
            sub_gx: 130,
            sub_gy: 128,
        };
        let encoded = encode_pose(&p);
        assert_eq!(encoded.len(), 37);
        assert_eq!(decode_pose(&encoded).unwrap(), p);
        // valid=false（无意图）时 sub 坐标仍往返
        let p2 = PoseData { valid: false, sub_gx: 0, sub_gy: 0, ..p };
        assert_eq!(decode_pose(&encode_pose(&p2)).unwrap(), p2);
        // 长度不符（旧版 33B 帧）拒绝
        assert!(decode_pose(&encoded[..33]).is_none());
    }

    #[test]
    fn test_map_delta_roundtrip() {
        // Task 13_2：delta i8 差分语义（含负值、clamp 边界值）
        let entries = vec![
            MapDeltaEntry { gx: 130, gy: 128, delta: 3 },
            MapDeltaEntry { gx: -5, gy: 300, delta: -1 },
            MapDeltaEntry { gx: 0, gy: 0, delta: 8 },
            MapDeltaEntry { gx: 7, gy: 9, delta: -8 },
            MapDeltaEntry { gx: 11, gy: 12, delta: 2 }, // clamp 边界 6→8 的 Δ=+2
        ];
        let encoded = encode_map_delta(42, &entries);
        let (ts, decoded) = decode_map_delta(&encoded).unwrap();
        assert_eq!(ts, 42);
        assert_eq!(decoded, entries);
        // 9B/项：布局不变
        assert_eq!(encoded.len(), 6 + 9 * entries.len());
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
    fn test_task_set_roundtrip_single() {
        // 单车（member_count=1）：missions 多个任务
        let payload = TaskSetPayload {
            members: vec![vec![7]],
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 66.0, y: 64.0 },
                MissionItem { mission_type: MISSION_GOTO, x: 70.5, y: 60.0 },
            ],
        };
        let encoded = encode_task_set(&payload);
        // 布局：mission_count(1) + member_count(1) + members(1+1) + missions(9*2) = 2 + 2 + 18
        assert_eq!(encoded.len(), 2 + 2 + 18);
        let decoded = decode_task_set(&encoded).unwrap();
        assert_eq!(decoded.members, payload.members);
        assert_eq!(decoded.missions, payload.missions);
    }

    #[test]
    fn test_task_set_roundtrip_group() {
        // 群发（member_count=3）：变长 peer_id（38B multihash 风格）
        let members: Vec<Vec<u8>> = vec![vec![7u8; 38], vec![1, 2, 3], vec![9u8; 2]];
        let payload = TaskSetPayload {
            members: members.clone(),
            missions: vec![MissionItem { mission_type: MISSION_GOTO, x: 64.0, y: 64.0 }],
        };
        let encoded = encode_task_set(&payload);
        let decoded = decode_task_set(&encoded).unwrap();
        assert_eq!(decoded.members, members);
        assert_eq!(decoded.missions, payload.missions);
    }

    #[test]
    fn test_task_set_cancel() {
        // member_count=0 = 取消全部任务（mission_count=0）
        let payload = TaskSetPayload { members: vec![], missions: vec![] };
        let encoded = encode_task_set(&payload);
        assert_eq!(encoded, vec![0, 0]);
        let decoded = decode_task_set(&encoded).unwrap();
        assert!(decoded.members.is_empty() && decoded.missions.is_empty());
    }

    #[test]
    fn test_task_set_decode_reject_malformed() {
        // 长度 < 2（含旧格式 [0] 取消帧）→ None
        assert!(decode_task_set(&[]).is_none());
        assert!(decode_task_set(&[0]).is_none());
        // members 内 len 越界 → None
        assert!(decode_task_set(&[0, 1, 200]).is_none());
        assert!(decode_task_set(&[0, 1, 3, 1, 2]).is_none());
        // missions 字节不足 → None
        assert!(decode_task_set(&[1, 0, 0, 0]).is_none());
        // 旧格式 [count, missions...]（无 member_count 段）的语义安全性：
        // [1, 0, 9B...] / [2, 0, 18B...] 在新解析下 mission_count=n、member_count=0 →
        // 结构合法（长度恰好匹配时），但 WS 层按 member_count==0 → 取消处理 → 不会执行旧帧任务，语义安全
        let old_two = [2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let parsed = decode_task_set(&old_two).unwrap();
        assert!(parsed.members.is_empty());
        assert_eq!(parsed.missions.len(), 2, "长度恰好匹配的旧帧结构可解析，语义由上层判（取消）");
    }

    #[test]
    fn test_task_set_members_empty_is_single() {
        // member_count=1 且 members 恰好 1 条时,mission_count>0 正常解析
        let payload = TaskSetPayload {
            members: vec![vec![5]],
            missions: vec![MissionItem { mission_type: MISSION_GOTO, x: 1.0, y: 2.0 }],
        };
        let encoded = encode_task_set(&payload);
        let decoded = decode_task_set(&encoded).unwrap();
        assert_eq!(decoded.members.len(), 1);
        assert_eq!(decoded.missions.len(), 1);
    }

    #[test]
    fn test_map_full_roundtrip() {
        use crate::robot::core::grid::CHUNK_SIZE;
        // 构造带非零值的整表，验证位模式往返
        let mut data = Box::new([0i8; CHUNK_SIZE * CHUNK_SIZE]);
        data[0] = 3;
        data[100] = -8;
        data[65535] = 8;
        let data_u8: Vec<u8> = data.iter().map(|&v| v as u8).collect();
        let payload = encode_map_full(42, 0, 0, 256, 256, 0.5, &data_u8);
        assert_eq!(payload.len(), 20 + CHUNK_SIZE * CHUNK_SIZE);

        let (ts, ogx, ogy, w, h, res, decoded) = decode_map_full(&payload).unwrap();
        assert_eq!(ts, 42);
        assert_eq!((ogx, ogy), (0, 0));
        assert_eq!((w, h), (256, 256));
        assert_eq!(res, 0.5);
        assert_eq!(decoded[0], 3);
        assert_eq!(decoded[100], -8);
        assert_eq!(decoded[65535], 8);

        // 长度非法（过短 / 空 / 截断）拒绝
        assert!(decode_map_full(&payload[..100]).is_none());
        assert!(decode_map_full(&[]).is_none());
        assert!(decode_map_full(&payload[..payload.len() - 1]).is_none());
    }

    #[test]
    fn test_frame_with_message() {
        // 帧 + 消息组合：编码一条 POSE 消息并解码
        let p = PoseData { time_boot_ms: 1, x: 64.0, y: 64.0, z: 0.0, vx: 0.0, vy: 0.0, yaw: 0.0, valid: false, sub_gx: 0, sub_gy: 0 };
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
