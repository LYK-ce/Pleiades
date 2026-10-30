//Presented by KeJi
//Created Date ： 2026-08-07
//Modified Date ： 2026-10-30

//! 5 条消息的 payload 编解码（协议文档 §3）
//!
//! 所有数值大端（BE）。坐标/计数为 i32，与内部类型一致。

use super::frame::MAGIC;
use crate::robot::core::grid::CHUNK_SIZE;

// ============================================================
// 数据结构
// ============================================================

/// ORION_POSE 载荷（msgid 1，38 字节，Task 24：加 rtk_fixed）
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
    /// RTK 固定解标志（Task 24）：FIXED true / 失锁 false；未启用 RTK 恒 false
    pub rtk_fixed: bool,
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

/// ORION_TASK_SET 任务项（按 type 变长：`0/2` = 9 字节，`3` = 13 字节；task_41 §3.2）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionItem {
    pub mission_type: u8,
    pub x: f32,
    pub y: f32,
    /// 半边长（米）：**仅 type=3 Containment 编码**，`0` / `2` 恒 `0.0`
    pub half_side: f32,
}

/// ORION_TASK_SET 载荷（Task 36 §十二 Step 3：删除群发名单段；task_41 §3.2：记录变长）
///
/// 协议布局：mission_count u8 + missions[]（总长 `1 + Σlen`，`len` 见 [`mission_wire_len`]）
/// - mission_count == 0 → 取消全部任务（唯一取消入口）
/// - mission_count > 0  → 逐条入队（未知 type 由 `decode_task_set` 整帧拒）
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSetPayload {
    /// 任务列表（MISSION_GOTO / MISSION_FOLLOW / MISSION_CONTAINMENT）
    pub missions: Vec<MissionItem>,
}

// ============================================================
// ORION_POSE (msgid 1)
// ============================================================

/// 编码位姿：time_boot_ms + x/y/z/vx/vy/yaw + valid + sub_gx/sub_gy + rtk_fixed（38 字节，Task 24）
pub fn encode_pose(pose: &PoseData) -> Vec<u8> {
    let mut buf = Vec::with_capacity(38);
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
    buf.push(pose.rtk_fixed as u8);
    buf
}

/// 解码位姿 payload（需恰好 38 字节，Task 24）
pub fn decode_pose(payload: &[u8]) -> Option<PoseData> {
    if payload.len() != 38 {
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
        rtk_fixed: payload[37] != 0,
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
    // 显式截断（release 下 debug_assert 无效，避免静默截断导致接收端校验失败）
    let n = entries.len().min(u16::MAX as usize);
    if entries.len() > n {
        tracing::warn!("encode_map_delta: {} 条目超出 u16 上限，截断为 {n}", entries.len());
    }
    let mut buf = Vec::with_capacity(6 + 9 * n);
    buf.extend_from_slice(&time_boot_ms.to_be_bytes());
    buf.extend_from_slice(&(n as u16).to_be_bytes());
    for e in &entries[..n] {
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
/// Follow（Task 37 §五.2）：`type=2 | x(f32) | y(f32)`，`x/y` 预留（填 0）
///
/// ⚠️ **必须用 2，不能用 1**：`1` 是 Task 36 已删除的旧 `Circle`，
/// 地面站/旧工具链里可能仍有该值，复用会让旧 Circle 帧被误认成 Follow。
pub const MISSION_FOLLOW: u8 = 2;
/// Containment（task_41 §3.2）：`type=3 | cx(f32) | cy(f32) | half(f32)`（13 B）
///
/// `1` 仍是 Task 36 已删除的旧 `Circle`，**不得复用**；`3` 才是封控。
pub const MISSION_CONTAINMENT: u8 = 3;

/// 记录长度表（含 type 字节；task_41 §3.2）。`None` = 未知 type → 整帧拒。
fn mission_wire_len(t: u8) -> Option<usize> {
    match t {
        MISSION_GOTO | MISSION_FOLLOW => Some(9),
        MISSION_CONTAINMENT => Some(13),
        _ => None,
    }
}

/// 编码任务队列：mission_count(1) + missions[]（记录变长，总长 `1 + Σlen`）
///
/// `mission_count` 为 u8，协议上限 255（超出会截断导致接收端校验失败）。
/// 未知 type 记 warn 后**跳过该条**（生产编码链路只有 Pictor，Pictor 对未知 type 直接不发帧）。
pub fn encode_task_set(payload: &TaskSetPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 13 * payload.missions.len());
    buf.push(0); // count 占位，最后回填（未知 type 会被跳过，条数不等于输入长度）
    let mut n = 0usize;
    for m in &payload.missions {
        let Some(len) = mission_wire_len(m.mission_type) else {
            tracing::warn!("encode_task_set: 未知 mission type {}，跳过该条", m.mission_type);
            continue;
        };
        // 显式截断（release 下 debug_assert 无效）
        if n >= u8::MAX as usize {
            tracing::warn!("encode_task_set: missions 超出 u8 上限，截断");
            break;
        }
        buf.push(m.mission_type);
        buf.extend_from_slice(&m.x.to_be_bytes());
        buf.extend_from_slice(&m.y.to_be_bytes());
        if len == 13 {
            buf.extend_from_slice(&m.half_side.to_be_bytes());
        }
        n += 1;
    }
    buf[0] = n as u8;
    buf
}

/// 解码任务队列 payload（task_41 §3.2：按 type 变长，**严格**校验）
///
/// 任何不符 → `None`（调用方 warn，队列不动）：
/// - 空 payload / 记录截断 / 循环后偏移 ≠ `payload.len()`（多 1 B、`count` 与条数不符）
/// - 未知 type（含退役的 `1`）→ 无法知道长度 → 整帧拒
/// - `half_side` 非有限或 `<= 0.0` → 整帧拒
pub fn decode_task_set(payload: &[u8]) -> Option<TaskSetPayload> {
    let mission_count = *payload.first()? as usize;
    let mut off = 1usize;
    let mut missions = Vec::with_capacity(mission_count);
    for _ in 0..mission_count {
        let t = *payload.get(off)?; // 越界 → None
        let len = mission_wire_len(t)?; // 未知 type（含 1）→ 整帧 None
        let end = off.checked_add(len)?;
        if end > payload.len() {
            return None;
        }
        let x = f32::from_be_bytes(payload[off + 1..off + 5].try_into().ok()?);
        let y = f32::from_be_bytes(payload[off + 5..off + 9].try_into().ok()?);
        let half_side = if t == MISSION_CONTAINMENT {
            let h = f32::from_be_bytes(payload[off + 9..off + 13].try_into().ok()?);
            if !h.is_finite() || h <= 0.0 {
                return None;
            }
            h
        } else {
            0.0
        };
        missions.push(MissionItem { mission_type: t, x, y, half_side });
        off = end;
    }
    if off != payload.len() {
        return None; // 多 1 B / count 与条数不符 → None
    }
    Some(TaskSetPayload { missions })
}

// ============================================================
// ORION_ASSIST_REQUEST (msgid 7)
// ============================================================

/// 协助请求载荷（msgid 7，**严格 8 字节**）：`x f32 BE | y f32 BE`（task_41 §5.3）
///
/// 语义：sim 机栈封控树的 `InsideSquare` 条件里，首次发现入侵者、以及目标相对上次发送点
/// 移动 > 1 m 时发出；`x/y` = 目标（人）的世界坐标（米，x 东 / y 南）。
/// 单向：只有地面站订阅 `TOPIC_ROBOT_ASSIST_REQUEST`，车 / 机不订阅、不回复。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AssistRequest {
    pub x: f32,
    pub y: f32,
}

/// 编码协助请求（恒 8 B；**不做有限性校验**——生产调用方（树）已先判有限，
/// 非有限值由解码侧拒，见 [`decode_assist_request`]）
pub fn encode_assist_request(x: f32, y: f32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    buf.extend_from_slice(&x.to_be_bytes());
    buf.extend_from_slice(&y.to_be_bytes());
    buf
}

/// 解码协助请求 payload（必须**恰好 8 字节**；`x`/`y` 非有限 → `None`）
pub fn decode_assist_request(payload: &[u8]) -> Option<AssistRequest> {
    if payload.len() != 8 {
        return None;
    }
    let x = f32::from_be_bytes(payload[0..4].try_into().ok()?);
    let y = f32::from_be_bytes(payload[4..8].try_into().ok()?);
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(AssistRequest { x, y })
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
            rtk_fixed: true,
        };
        let encoded = encode_pose(&p);
        assert_eq!(encoded.len(), 38);
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
    fn test_task_set_roundtrip_new_layout() {
        // Task 36 §12.3.4：mission_count(1) + 9×mission_count（type 0/2 每条 9 B）
        for missions in [
            vec![MissionItem { mission_type: MISSION_GOTO, x: 66.0, y: 64.0, half_side: 0.0 }],
            vec![
                MissionItem { mission_type: MISSION_GOTO, x: 66.0, y: 64.0, half_side: 0.0 },
                MissionItem { mission_type: MISSION_GOTO, x: 70.5, y: 60.0, half_side: 0.0 },
                MissionItem { mission_type: MISSION_FOLLOW, x: -1.5, y: 128.0, half_side: 0.0 },
            ],
        ] {
            let payload = TaskSetPayload { missions: missions.clone() };
            let encoded = encode_task_set(&payload);
            assert_eq!(encoded.len(), 1 + 9 * missions.len(), "全 9B type 帧长必须是 1 + 9×count");
            assert_eq!(encoded[0] as usize, missions.len(), "count 在首字节");
            assert_eq!(decode_task_set(&encoded).unwrap(), payload);
        }
    }

    #[test]
    fn test_task_set_follow_type_roundtrip() {
        // Task 37 §五.2：type=2 = Follow，`x/y` 预留（填 0）；布局仍是 9B/项
        assert_eq!(MISSION_FOLLOW, 2);
        assert_ne!(MISSION_FOLLOW, 1, "1 是已删除的旧 Circle，不得复用");
        let missions = vec![
            MissionItem { mission_type: MISSION_FOLLOW, x: 0.0, y: 0.0, half_side: 0.0 },
            MissionItem { mission_type: MISSION_GOTO, x: 66.0, y: 64.0, half_side: 0.0 },
        ];
        let payload = TaskSetPayload { missions: missions.clone() };
        let encoded = encode_task_set(&payload);
        assert_eq!(encoded.len(), 1 + 9 * missions.len(), "帧长必须是 1 + 9×count");
        assert_eq!(encoded[1], MISSION_FOLLOW, "首条 type 必须是 2");
        assert_eq!(decode_task_set(&encoded).unwrap(), payload);
        // 单条 Follow(0,0) 金标逐字不变（10 B）
        let single = TaskSetPayload {
            missions: vec![MissionItem { mission_type: MISSION_FOLLOW, x: 0.0, y: 0.0, half_side: 0.0 }],
        };
        assert_eq!(encode_task_set(&single), hex("01020000000000000000"));
    }

    #[test]
    fn test_task_set_cancel() {
        // count == 0 = 取消（唯一取消入口）
        let payload = TaskSetPayload { missions: vec![] };
        let encoded = encode_task_set(&payload);
        assert_eq!(encoded, vec![0]);
        assert!(decode_task_set(&encoded).unwrap().missions.is_empty());
    }

    #[test]
    fn test_task_set_rejects_old_layout_with_member_segment() {
        // 旧布局（Task 14）：count=1 + member_count=1 + len=1 + peer=7 + 9B mission = 13 字节
        // 现在首条 type=1 是退役的旧 Circle（长度表里没有）→ **整帧 None**
        let mut old = vec![1u8, 1, 1, 7];
        old.push(MISSION_GOTO);
        old.extend_from_slice(&64.0f32.to_be_bytes());
        old.extend_from_slice(&64.0f32.to_be_bytes());
        assert_eq!(old.len(), 13);
        assert!(decode_task_set(&old).is_none());
        // 老 count=0 取消帧（1 字节）在新布局下**合法**（总长 1 + Σ0 = 1）
        assert!(decode_task_set(&[0]).is_some());
    }

    #[test]
    fn test_task_set_rejects_bad_length() {
        let mut ok = vec![1u8];
        ok.extend_from_slice(&[0u8; 9]);
        assert!(decode_task_set(&ok).is_some(), "count=1 + 9B 必须合法");
        assert!(decode_task_set(&ok[..9]).is_none(), "少 1 字节必须拒");
        let mut extra = ok.clone();
        extra.push(0);
        assert!(decode_task_set(&extra).is_none(), "多 1 字节必须拒");
        assert!(decode_task_set(&[]).is_none(), "空 payload 必须拒");
    }

    #[test]
    fn test_task_set_max_count() {
        // 上限 255 条：1 + 9×255 = 2296 字节，编解码一致、不 panic
        let payload = TaskSetPayload {
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 1.0, y: 2.0, half_side: 0.0 };
                255
            ],
        };
        let encoded = encode_task_set(&payload);
        assert_eq!(encoded.len(), 1 + 9 * 255);
        assert_eq!(decode_task_set(&encoded).unwrap().missions.len(), 255);
        // 256 条越 u8 上限 → 截断为 255（warn 已打，不 panic）
        let over = TaskSetPayload {
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 0.0, y: 0.0, half_side: 0.0 };
                256
            ],
        };
        assert_eq!(encode_task_set(&over).len(), 1 + 9 * 255);
    }

    // ---------------------------------------------------- task_41 §3.2：变长 TASK_SET

    /// hex 字面量 → 字节（金标硬编码，与 Pictor / Python 用同一串）
    fn hex(s: &str) -> Vec<u8> {
        assert!(s.len() % 2 == 0, "hex 长度必须是偶数");
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("合法 hex"))
            .collect()
    }

    /// type=3 帧：`type u8 | cx | cy | half`（13 B）
    fn containment_item(x: f32, y: f32, half_side: f32) -> MissionItem {
        MissionItem { mission_type: MISSION_CONTAINMENT, x, y, half_side }
    }

    /// 长度表：0/2 → 9、3 → 13、其余（含退役的 1）→ None
    #[test]
    fn test_mission_wire_len_table() {
        assert_eq!(mission_wire_len(MISSION_GOTO), Some(9));
        assert_eq!(mission_wire_len(MISSION_FOLLOW), Some(9));
        assert_eq!(mission_wire_len(MISSION_CONTAINMENT), Some(13));
        assert_eq!(mission_wire_len(1), None, "1 是退役的旧 Circle，不得复用");
        assert_eq!(mission_wire_len(9), None, "未知 type 无长度 → 整帧拒");
        assert_eq!(MISSION_CONTAINMENT, 3);
    }

    /// 单条 Containment(64,64,8) = 14 B，金标逐字节；roundtrip 含 `half_side`
    #[test]
    fn test_task_set_containment_golden() {
        let payload = TaskSetPayload { missions: vec![containment_item(64.0, 64.0, 8.0)] };
        let encoded = encode_task_set(&payload);
        assert_eq!(encoded.len(), 14, "单条 Containment = 1 + 13");
        assert_eq!(encoded, hex("0103428000004280000041000000"));
        assert_eq!(decode_task_set(&encoded).unwrap(), payload);
        // Goto 金标逐字不变（10 B）
        let goto = TaskSetPayload {
            missions: vec![MissionItem {
                mission_type: MISSION_GOTO,
                x: 64.0,
                y: 64.0,
                half_side: 0.0,
            }],
        };
        assert_eq!(encode_task_set(&goto), hex("01004280000042800000"));
    }

    /// 混合 [Goto(64,64), Containment(64,64,8)] = 23 B，偏移与顺序正确
    #[test]
    fn test_task_set_mixed_goto_containment() {
        let payload = TaskSetPayload {
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 64.0, y: 64.0, half_side: 0.0 },
                containment_item(64.0, 64.0, 8.0),
            ],
        };
        let encoded = encode_task_set(&payload);
        assert_eq!(encoded.len(), 23, "[Goto, Containment] = 1 + 9 + 13");
        assert_eq!(encoded, hex("0200428000004280000003428000004280000041000000"));
        assert_eq!(encoded[0], 2, "count 在首字节");
        assert_eq!(encoded[1], MISSION_GOTO, "第 0 条 type");
        assert_eq!(encoded[10], MISSION_CONTAINMENT, "第 1 条 type 在 1+9 处");
        assert_eq!(&encoded[19..23], &[0x41, 0x00, 0x00, 0x00], "half_side = 8.0 大端");
        let back = decode_task_set(&encoded).unwrap();
        assert_eq!(back, payload, "顺序与值往返");
        assert_eq!(back.missions[0].half_side, 0.0, "非 3 的 half_side 恒 0.0");
        assert_eq!(back.missions[1].half_side, 8.0);
    }

    /// 未知 type（如 9）→ 整帧 None（不知道长度，不能跳过）
    #[test]
    fn test_task_set_rejects_unknown_type() {
        let raw = vec![1u8, 9, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(decode_task_set(&raw).is_none(), "未知 type → 整帧拒");
        // 混合帧里出现未知 type 也整帧拒（不是滤掉那一条）
        let mut mixed = vec![2u8, MISSION_GOTO];
        mixed.extend_from_slice(&64.0f32.to_be_bytes());
        mixed.extend_from_slice(&64.0f32.to_be_bytes());
        mixed.extend_from_slice(&raw);
        assert!(decode_task_set(&mixed).is_none(), "[Goto, 未知 9] → 整帧拒");
    }

    /// `half_side` 非有限或 `<= 0` → 整帧 None
    #[test]
    fn test_task_set_rejects_bad_half_side() {
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            // 编码端不做校验（Pictor 才是生产编码链路，那边整帧不发），此处直接用非法值造帧
            let raw = encode_task_set(&TaskSetPayload {
                missions: vec![containment_item(64.0, 64.0, bad)],
            });
            assert_eq!(raw.len(), 14);
            assert!(decode_task_set(&raw).is_none(), "half_side = {bad} 必须整帧拒");
        }
        // 合法边界：极小正数照收
        let ok = encode_task_set(&TaskSetPayload {
            missions: vec![containment_item(64.0, 64.0, 0.5)],
        });
        assert_eq!(decode_task_set(&ok).unwrap().missions[0].half_side, 0.5);
    }

    /// 旧接收端语义：含 13 B 记录的帧总长不符 → 整帧拒；截断 / 多 1 B 同理
    #[test]
    fn test_task_set_old_receiver_rejects_containment_frame() {
        // 旧端按 `1 + 9n` 校验：count=1 + 13 B = 14 B ≠ 1 + 9×1 → 拒
        // （本端也拒：count=1 只吃 9 B，剩 4 B → 偏移 ≠ 长度）
        let mut old = vec![1u8, MISSION_GOTO];
        old.extend_from_slice(&64.0f32.to_be_bytes());
        old.extend_from_slice(&64.0f32.to_be_bytes());
        old.extend_from_slice(&8.0f32.to_be_bytes());
        assert_eq!(old.len(), 14);
        assert!(decode_task_set(&old).is_none());
        // 混合帧截掉最后 1 B（22 B）→ 拒
        let mixed = encode_task_set(&TaskSetPayload {
            missions: vec![
                MissionItem { mission_type: MISSION_GOTO, x: 64.0, y: 64.0, half_side: 0.0 },
                containment_item(64.0, 64.0, 8.0),
            ],
        });
        assert!(decode_task_set(&mixed[..22]).is_none(), "截断 1 B → 拒");
        let mut extra = mixed.clone();
        extra.push(0);
        assert!(decode_task_set(&extra).is_none(), "多 1 B → 拒");
    }

    /// `count` 与实际条数不符（count=2 但只有 1 条 13 B）→ 拒
    #[test]
    fn test_task_set_rejects_count_mismatch_containment() {
        let mut raw = encode_task_set(&TaskSetPayload {
            missions: vec![containment_item(64.0, 64.0, 8.0)],
        });
        raw[0] = 2;
        assert!(decode_task_set(&raw).is_none(), "count=2 但只有 1 条 → 循环越界读 type → 拒");
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
        let p = PoseData { time_boot_ms: 1, x: 64.0, y: 64.0, z: 0.0, vx: 0.0, vy: 0.0, yaw: 0.0, valid: false, sub_gx: 0, sub_gy: 0, rtk_fixed: false };
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

    // ---------------------------------------------------- task_41 §5.3：ASSIST_REQUEST（msgid 7）

    /// 8 B 金标：`[x=1.0, y=2.0]` → `3f80000040000000`（与 Pictor 单测同一串）
    #[test]
    fn test_assist_request_golden() {
        let p = encode_assist_request(1.0, 2.0);
        assert_eq!(p.len(), 8, "payload 恒 8 B");
        assert_eq!(p, hex("3f80000040000000"));
        // 负值 / 0 的 BE 字节
        assert_eq!(encode_assist_request(0.0, 0.0), hex("0000000000000000"));
        assert_eq!(encode_assist_request(-1.5, 64.25), hex("bfc0000042808000"));
    }

    /// roundtrip + 长度严格（7 / 9 / 空 → None）
    #[test]
    fn test_assist_request_roundtrip_and_reject() {
        for (x, y) in [(1.0f32, 2.0f32), (-0.5, 128.0), (0.0, -0.0), (63.75, 71.5)] {
            let p = encode_assist_request(x, y);
            assert_eq!(p.len(), 8);
            let back = decode_assist_request(&p).expect("8 B 必须解出");
            assert_eq!(back.x.to_bits(), x.to_bits(), "x 位模式往返");
            assert_eq!(back.y.to_bits(), y.to_bits(), "y 位模式往返");
        }
        let p = encode_assist_request(1.0, 2.0);
        assert!(decode_assist_request(&p[..7]).is_none(), "7 B → None");
        let mut nine = p.clone();
        nine.push(0);
        assert!(decode_assist_request(&nine).is_none(), "9 B → None");
        assert!(decode_assist_request(&[]).is_none(), "空 → None");
    }

    /// 非有限值拒（NaN / ±inf；解码侧是唯一门控）
    #[test]
    fn test_assist_request_rejects_non_finite() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                decode_assist_request(&encode_assist_request(bad, 0.0)).is_none(),
                "x = {bad} 必须拒"
            );
            assert!(
                decode_assist_request(&encode_assist_request(0.0, bad)).is_none(),
                "y = {bad} 必须拒"
            );
        }
    }

    /// 帧级契约：`encode_frame(7, uav_peer_id, COMPID_ROBOT, 8 B)` → msgid/compid/sysid/len 全对
    #[test]
    fn test_assist_request_frame_contract() {
        let sysid = vec![0x11u8; 34]; // 真实 libp2p PeerId 是 38 B multihash，这里用 34 B 便于数长
        let payload = encode_assist_request(1.0, 2.0);
        let frame = super::super::frame::encode_frame(
            super::super::MSGID_ASSIST_REQUEST,
            &sysid,
            super::super::COMPID_ROBOT,
            &payload,
        );
        // 固定头 10 + sysid 34 + payload 8 + checksum 2 = 54
        assert_eq!(frame.len(), 10 + 34 + 8 + 2);
        let dec = super::super::frame::decode_frame(&frame).expect("帧可解");
        assert_eq!(dec.msgid, 7);
        assert_eq!(dec.compid, super::super::COMPID_ROBOT);
        assert_eq!(dec.sysid, sysid);
        assert_eq!(
            decode_assist_request(&dec.payload).unwrap(),
            AssistRequest { x: 1.0, y: 2.0 }
        );
    }
}
