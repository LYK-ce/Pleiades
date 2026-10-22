//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 世界侧 lidar 二进制帧解析（task_31 §五.4.2；世界契约见 `orion-simulation-world/COMPONENTS.md` §8.2）
//!
//! 帧格式（**全部小端**，偏移量只在本文件出现一处，别处一律引用这里的常量）：
//!
//! ```text
//! [0]      u8   type        = 1 (lidar_2d)；2 = camera（预留，未实现）
//! [1..3]   u16  slot_id     = 世界分配的 sensor_id
//! [3..7]   u32  seq
//! [7..15]  f64  world_time  秒
//! [15..]   f32  range[]     米；0.0 = 无回波（角度不上线：angle = TAU*i/n）
//! ```
//!
//! 360 线 = 15 + 4×360 = **1455 B**。
//!
//! 错误分支（文本要能直接定位问题）：短包 / 类型不符 / 空 payload → `Err`；
//! 尾字节不足 4 字节 → 丢弃尾字节 + 限流 warning（不算错，帧仍可用）。

use crate::sim::sensors::Warn_Throttled;

/// 帧头字节数：`1 + 2 + 4 + 8`
pub const FRAME_HEADER_SIZE: usize = 15;
/// 1 = lidar_2d；2 = camera（预留）
pub const FRAME_TYPE_LIDAR: u8 = 1;
/// 每个采样点 `f32`
pub const SAMPLE_BYTES: usize = 4;

/// 帧头（偏移量唯一来源）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameHeader {
    pub frame_type: u8,
    pub sensor_id: u16,
    pub seq: u32,
    pub world_time: f64,
}

/// 一帧 lidar 数据（已解出 `f32` 数组）
#[derive(Debug, Clone, PartialEq)]
pub struct LidarFrame {
    pub sensor_id: u16,
    pub seq: u32,
    pub world_time: f64,
    pub ranges: Vec<f32>,
}

/// 只解帧头；短包 → `Err`（供连接层做 `sensor_id` 校验，避免整帧解两遍）
pub fn Parse_Header(bytes: &[u8]) -> Result<FrameHeader, String> {
    if bytes.len() < FRAME_HEADER_SIZE {
        return Err(format!(
            "短包：{} B < {FRAME_HEADER_SIZE} B 帧头",
            bytes.len()
        ));
    }
    let frame_type = bytes[0];
    let sensor_id = u16::from_le_bytes([bytes[1], bytes[2]]);
    let seq = u32::from_le_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]);
    let world_time = f64::from_le_bytes([
        bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
    ]);
    Ok(FrameHeader { frame_type, sensor_id, seq, world_time })
}

/// 帧头的 `sensor_id`（偏移 1，u16 小端）；不足 3 B → `None`
pub fn Sensor_Id_Of(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < 3 {
        return None;
    }
    Some(u16::from_le_bytes([bytes[1], bytes[2]]))
}

/// 小端解析整帧。短包 / 类型不符 / 空 payload → `Err`。
pub fn Parse_Lidar_Frame(bytes: &[u8]) -> Result<LidarFrame, String> {
    let h = Parse_Header(bytes)?;
    if h.frame_type != FRAME_TYPE_LIDAR {
        return Err(format!(
            "帧类型 {} 不是 lidar（{FRAME_TYPE_LIDAR}）：相机帧 2 尚未实现，丢弃",
            h.frame_type
        ));
    }
    let payload = bytes.len() - FRAME_HEADER_SIZE;
    if payload == 0 {
        return Err(format!(
            "空 payload：只有 {FRAME_HEADER_SIZE} B 帧头没有采样点"
        ));
    }

    let n = payload / SAMPLE_BYTES;
    let tail = payload % SAMPLE_BYTES;
    if tail != 0 {
        // 尾字节丢弃：不是错，但必须能被看到（限流，避免 10 Hz 刷屏）
        Warn_Throttled("lidar.tail_bytes", 5000, || {
            format!("[lidar] 帧尾 {tail} B 不足 {SAMPLE_BYTES} B，丢弃尾字节（可用点数 {n}）")
        });
    }

    let mut ranges = Vec::with_capacity(n);
    for i in 0..n {
        let at = FRAME_HEADER_SIZE + i * SAMPLE_BYTES;
        ranges.push(f32::from_le_bytes([
            bytes[at],
            bytes[at + 1],
            bytes[at + 2],
            bytes[at + 3],
        ]));
    }

    Ok(LidarFrame {
        sensor_id: h.sensor_id,
        seq: h.seq,
        world_time: h.world_time,
        ranges,
    })
}

// ================================================================ 测试用帧构造（其它模块的单测复用）

/// 构造一帧合法 lidar 帧（小端）。仅测试用。
#[cfg(test)]
pub(crate) fn Test_Frame(
    sensor_id: u16,
    seq: u32,
    world_time: f64,
    ranges: &[f32],
) -> Vec<u8> {
    let mut b = Vec::with_capacity(FRAME_HEADER_SIZE + SAMPLE_BYTES * ranges.len());
    b.push(FRAME_TYPE_LIDAR);
    b.extend_from_slice(&sensor_id.to_le_bytes());
    b.extend_from_slice(&seq.to_le_bytes());
    b.extend_from_slice(&world_time.to_le_bytes());
    for r in ranges {
        b.extend_from_slice(&r.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_360_frame_is_1455_bytes() {
        // 硬验收 2：360 点 = 1455 B，头字段与点数全对
        let ranges: Vec<f32> = (0..360).map(|i| i as f32 * 0.01).collect();
        let bytes = Test_Frame(7, 42, 12.5, &ranges);
        assert_eq!(bytes.len(), 1455, "360 点帧必须是 1455 B");

        let f = Parse_Lidar_Frame(&bytes).expect("合法帧必须解析成功");
        assert_eq!(f.sensor_id, 7);
        assert_eq!(f.seq, 42);
        assert_eq!(f.world_time, 12.5);
        assert_eq!(f.ranges.len(), 360);
        assert!((f.ranges[0] - 0.0).abs() < 1e-6);
        assert!((f.ranges[359] - 3.59).abs() < 1e-4);
    }

    #[test]
    fn test_parse_header() {
        let bytes = Test_Frame(9, 3, 1.5, &[1.0]);
        let h = Parse_Header(&bytes).expect("帧头应能单独解出");
        assert_eq!(
            h,
            FrameHeader { frame_type: FRAME_TYPE_LIDAR, sensor_id: 9, seq: 3, world_time: 1.5 }
        );
    }

    #[test]
    fn test_parse_rejects_short_packet() {
        assert!(Parse_Lidar_Frame(&[]).is_err());
        assert!(Parse_Lidar_Frame(&[0u8; 14]).is_err());
        // 错误信息要能定位（含字节数）
        let e = Parse_Lidar_Frame(&[0u8; 14]).unwrap_err();
        assert!(e.contains("14"), "错误信息缺实际字节数：{e}");
        assert!(e.contains("15"), "错误信息缺帧头长度：{e}");
    }

    #[test]
    fn test_parse_rejects_non_lidar_type() {
        let mut bytes = Test_Frame(1, 1, 0.0, &[1.0, 2.0]);
        bytes[0] = 2; // 相机
        let e = Parse_Lidar_Frame(&bytes).unwrap_err();
        assert!(e.contains('2'), "错误信息缺帧类型：{e}");
        assert!(e.contains("lidar"), "错误信息缺期望类型：{e}");
    }

    #[test]
    fn test_parse_rejects_empty_payload() {
        // 只有 15 B 帧头
        let bytes = Test_Frame(1, 1, 0.0, &[]);
        assert_eq!(bytes.len(), FRAME_HEADER_SIZE);
        let e = Parse_Lidar_Frame(&bytes).unwrap_err();
        assert!(e.contains("空 payload"), "错误信息应说明空 payload：{e}");
    }

    #[test]
    fn test_parse_tail_bytes_dropped() {
        // 15 + 4*3 + 2 → Ok，且 ranges.len() == 3
        let mut bytes = Test_Frame(5, 6, 2.0, &[1.0, 2.0, 3.0]);
        bytes.extend_from_slice(&[0xAA, 0xBB]);
        assert_eq!(bytes.len(), 15 + 12 + 2);
        let f = Parse_Lidar_Frame(&bytes).expect("尾字节不整除仍是合法帧");
        assert_eq!(f.ranges.len(), 3);
        assert!((f.ranges[2] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_sensor_id_of() {
        let bytes = Test_Frame(513, 0, 0.0, &[]);
        assert_eq!(Sensor_Id_Of(&bytes), Some(513));
        // 不足 3 B 取不到
        assert_eq!(Sensor_Id_Of(&[]), None);
        assert_eq!(Sensor_Id_Of(&[1, 2]), None);
        assert_eq!(Sensor_Id_Of(&[1, 2, 3]), Some(0x0302));
    }

    #[test]
    fn test_parse_empty_payload_only_header_type_check() {
        // 类型不符优先于空 payload 报错（相机帧即便空也要说"不是 lidar"）
        let mut bytes = vec![2u8];
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0f64.to_le_bytes());
        assert_eq!(bytes.len(), FRAME_HEADER_SIZE);
        let e = Parse_Lidar_Frame(&bytes).unwrap_err();
        assert!(e.contains("帧类型"), "应先报类型不符：{e}");
    }
}
