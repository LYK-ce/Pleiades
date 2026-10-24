//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-24

#![allow(non_snake_case)]

//! 世界侧 camera 二进制帧解析（task_34 §二 Step 4；世界契约见
//! `orion-simulation-world/src/net/WireProtocol.gd` 的 `BuildCameraFrame`）
//!
//! 帧格式（**全部小端**；15 B 帧头与 lidar **完全相同**，只是 payload 换成原始 JPEG）：
//!
//! ```text
//! [0]      u8   type        = 2 (camera)
//! [1..3]   u16  slot_id     = 世界分配的 sensor_id
//! [3..7]   u32  seq
//! [7..15]  f64  world_time  秒
//! [15..]   u8   jpeg[]      原始 JPEG（世界侧已编码；sim 不重新编码、不重采样、不加字段）
//! ```
//!
//! **帧头偏移量不在这里复制第二份**：[`crate::sim::sensors::lidar::frame`] 是 15 B 帧头的唯一实现
//! （那个文件自己写着"偏移量只在本文件出现一处，别处一律引用这里的常量"），相机只做类型判定 + payload 解析。
//!
//! 宽高从 JPEG 的 SOF 段读（**不信任配置值**：世界可能按自己的 `sensor.params` 出图）。
//!
//! 失败语义：短包 / `type != 2` / 空 payload / payload 不是 JPEG / 段结构损坏 /
//! 找不到 SOF / SOF 宽高为 0 —— 一律 `Err(String)`，调用方（[`super::CameraSensor::On_Binary`]）
//! 限流告警后丢弃，不 panic、不 unwrap。

use crate::sim::sensors::lidar::frame::{Parse_Header, FRAME_HEADER_SIZE};

/// 帧类型：2 = camera（与 `WireProtocol.FRAME_TYPE_CAMERA` 一致）
pub const FRAME_TYPE_CAMERA: u8 = 2;

/// JPEG 起始标记（SOI + 紧跟的第一个 marker 前缀）
pub const JPEG_SOI: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// 一帧 camera 数据（JPEG 原样保留）
#[derive(Debug, Clone, PartialEq)]
pub struct CameraFrame {
    pub sensor_id: u16,
    pub seq: u32,
    pub world_time: f64,
    /// 从 JPEG SOF 段读出的**真实**宽（不是配置值）
    pub width: u32,
    /// 从 JPEG SOF 段读出的**真实**高
    pub height: u32,
    /// payload 原样（世界侧已编码，sim 不重新编码）
    pub jpeg: Vec<u8>,
}

/// 小端解析整帧。**所有**失败都是 `Err(String)`（调用方限流，不 panic、不 unwrap）
pub fn Parse_Camera_Frame(bytes: &[u8]) -> Result<CameraFrame, String> {
    // 帧头复用 lidar 的唯一实现（短包错误文案也来自那里）
    let h = Parse_Header(bytes)?;
    if h.frame_type != FRAME_TYPE_CAMERA {
        return Err(format!(
            "帧类型 {} 不是 camera（{FRAME_TYPE_CAMERA}）：lidar 帧 1 请由 lidar 传感器接收，丢弃",
            h.frame_type
        ));
    }
    let payload = &bytes[FRAME_HEADER_SIZE..];
    if payload.is_empty() {
        return Err(format!(
            "空 payload：只有 {FRAME_HEADER_SIZE} B 帧头没有 JPEG"
        ));
    }
    let (width, height) = Jpeg_Size(payload)?;
    Ok(CameraFrame {
        sensor_id: h.sensor_id,
        seq: h.seq,
        world_time: h.world_time,
        width,
        height,
        jpeg: payload.to_vec(),
    })
}

/// 从 JPEG 里读宽高（SOI → 逐段扫 marker → 命中 SOF 段）
///
/// 扫描规则（写清楚，免得被当成魔法数字）：
/// - 起点 `i = 2`（跳过 SOI）；遇到 `0xFF` 后跳过连续 `0xFF` 填充，取 `marker`
/// - **无长度段**：`0xD8`(SOI) / `0x01`(TEM) / `0xD0..=0xD7`(RSTn) → `i = m + 1` 继续；
///   `0xD9`(EOI) → 结束并 `Err`（到 EOI 都没见到 SOF）
/// - **有长度段**：`len = u16 大端`（含自身 2 B），段尾 `seg_end = m + 1 + len`；
///   `seg_end > payload.len()` → `Err`（段被截断）
/// - **SOF 判定**：`0xC0`(SOF0，主判据) 兼容 `0xC1`(SOF1) / `0xC2`(SOF2，progressive)：
///   段内 `[m+4..m+6]` = 高、`[m+6..m+8]` = 宽（大端）
/// - 其它段（APPn / DQT / DHT / COM …）按 `seg_end` 整体跳过
///
/// 为什么要兼容 C1/C2：只认 `0xC0` 会让 progressive JPEG 被**永久**判成坏帧，
/// 现场只看到一句"坏 JPEG"，几乎无法定位（见 plan §9-R7）。
pub fn Jpeg_Size(jpeg: &[u8]) -> Result<(u32, u32), String> {
    if jpeg.len() < JPEG_SOI.len()
        || jpeg[0] != JPEG_SOI[0]
        || jpeg[1] != JPEG_SOI[1]
        || jpeg[2] != JPEG_SOI[2]
    {
        let head: Vec<String> = jpeg
            .iter()
            .take(JPEG_SOI.len())
            .map(|b| format!("{b:02X}"))
            .collect();
        return Err(format!(
            "payload 不是 JPEG（缺 SOI FF D8 FF）：前 {} 字节 [{}]",
            head.len(),
            head.join(" ")
        ));
    }

    let mut i = 2usize;
    let mut markers = 0usize;
    while i < jpeg.len() {
        if jpeg[i] != 0xFF {
            return Err(format!(
                "JPEG 结构损坏：偏移 {i} 处应为 marker 前缀 0xFF，实为 0x{:02X}",
                jpeg[i]
            ));
        }
        // 跳过连续 0xFF 填充
        let mut m = i;
        while m < jpeg.len() && jpeg[m] == 0xFF {
            m += 1;
        }
        if m >= jpeg.len() {
            return Err(format!("JPEG 结构损坏：偏移 {i} 起的 marker 被截断"));
        }
        let marker = jpeg[m];
        markers += 1;

        match marker {
            // 无长度段：SOI / TEM / RSTn → 前进 1 字节继续扫
            0xD8 | 0x01 | 0xD0..=0xD7 => {
                i = m + 1;
            }
            // EOI：到结尾都没见到 SOF
            0xD9 => {
                return Err(format!(
                    "JPEG 里找不到 SOF0（宽高不可知，共扫描 {markers} 个 marker）"
                ));
            }
            // SOF0 / SOF1 / SOF2：段内 [m+3]=精度 [m+4..m+6]=高 [m+6..m+8]=宽
            0xC0..=0xC2 => {
                if m + 8 > jpeg.len() {
                    return Err(format!(
                        "JPEG 结构损坏：SOF 段头被截断（marker 偏移 {m}，剩余 {} B）",
                        jpeg.len() - m
                    ));
                }
                let len = u16::from_be_bytes([jpeg[m + 1], jpeg[m + 2]]);
                if len < 2 {
                    return Err(format!("JPEG 段长度非法（{len}）"));
                }
                let seg_end = m + 1 + len as usize;
                if seg_end > jpeg.len() {
                    return Err(format!(
                        "JPEG 结构损坏：SOF 段声明 {len} B，超出剩余 {} B",
                        jpeg.len() - (m + 1)
                    ));
                }
                let height = u16::from_be_bytes([jpeg[m + 4], jpeg[m + 5]]);
                let width = u16::from_be_bytes([jpeg[m + 6], jpeg[m + 7]]);
                if width == 0 || height == 0 {
                    return Err(format!("JPEG SOF0 宽高为 0（w={width} h={height}）"));
                }
                return Ok((width as u32, height as u32));
            }
            // 其它有长度段：按 seg_end 整体跳过
            _ => {
                if m + 3 > jpeg.len() {
                    return Err(format!(
                        "JPEG 结构损坏：marker 0x{marker:02X} 的长度字段被截断（偏移 {m}）"
                    ));
                }
                let len = u16::from_be_bytes([jpeg[m + 1], jpeg[m + 2]]);
                if len < 2 {
                    return Err(format!("JPEG 段长度非法（{len}）"));
                }
                let seg_end = m + 1 + len as usize;
                if seg_end > jpeg.len() {
                    return Err(format!(
                        "JPEG 结构损坏：marker 0x{marker:02X} 声明 {len} B，超出剩余 {} B",
                        jpeg.len() - (m + 1)
                    ));
                }
                i = seg_end;
            }
        }
    }

    Err(format!(
        "JPEG 里找不到 SOF0（宽高不可知，共扫描 {markers} 个 marker）"
    ))
}

// ================================================================ 测试夹具（供本文件 / camera/mod.rs / device_cap.rs 复用）

/// 造一张**结构合法**的最小 JPEG（SOI + APP0 + SOF0 + EOI），只用于单测
///
/// 布局（长度字段必须自洽，否则会被 [`Jpeg_Size`] 判成"段被截断"）：
/// `FF D8` + `FF E0 00 10` + 14×`00`（APP0，len=16）
/// + `FF C0 00 11 08` + `h(2 BE)` + `w(2 BE)` + `03` + 9×`00`（3 个分量描述）+ `FF D9`
#[cfg(test)]
pub(crate) fn Test_Jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut j = Vec::with_capacity(41);
    // SOI
    j.extend_from_slice(&[0xFF, 0xD8]);
    // APP0：marker + len(16) + 14 B 内容
    j.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
    j.extend_from_slice(&[0u8; 14]);
    // SOF0：marker + len(17) + 精度(8) + 高 + 宽 + 分量数(3) + 3×(id,采样,量化表)
    j.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
    j.extend_from_slice(&height.to_be_bytes());
    j.extend_from_slice(&width.to_be_bytes());
    j.push(0x03);
    j.extend_from_slice(&[0u8; 9]);
    // EOI
    j.extend_from_slice(&[0xFF, 0xD9]);
    j
}

/// 造一帧合法的 camera 帧（15 B 小端帧头 + JPEG payload），只用于单测
#[cfg(test)]
pub(crate) fn Test_Frame(sensor_id: u16, seq: u32, world_time: f64, jpeg: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(FRAME_HEADER_SIZE + jpeg.len());
    b.push(FRAME_TYPE_CAMERA);
    b.extend_from_slice(&sensor_id.to_le_bytes());
    b.extend_from_slice(&seq.to_le_bytes());
    b.extend_from_slice(&world_time.to_le_bytes());
    b.extend_from_slice(jpeg);
    b
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_camera_frame_ok() {
        let jpeg = Test_Jpeg(640, 480);
        let bytes = Test_Frame(7, 42, 12.5, &jpeg);
        let f = match Parse_Camera_Frame(&bytes) {
            Ok(f) => f,
            Err(e) => {
                assert!(false, "合法 camera 帧必须解析成功：{e}");
                return;
            }
        };
        assert_eq!(f.sensor_id, 7);
        assert_eq!(f.seq, 42);
        assert_eq!(f.world_time, 12.5);
        assert_eq!(f.width, 640);
        assert_eq!(f.height, 480);
        // payload 原样：长度 = 帧长 − 15，且以 SOI 开头
        assert_eq!(f.jpeg.len(), bytes.len() - FRAME_HEADER_SIZE);
        assert_eq!(&f.jpeg[..JPEG_SOI.len()], &JPEG_SOI[..]);
    }

    #[test]
    fn test_parse_rejects_short_packet() {
        assert!(Parse_Camera_Frame(&[]).is_err());
        let e = Parse_Camera_Frame(&[0u8; 14]).unwrap_err();
        // 文案要能直接定位（含实际字节数与帧头长度）
        assert!(e.contains("14"), "错误信息缺实际字节数：{e}");
        assert!(e.contains("15"), "错误信息缺帧头长度：{e}");
    }

    #[test]
    fn test_parse_rejects_non_camera_type() {
        let jpeg = Test_Jpeg(320, 240);
        let mut bytes = Test_Frame(1, 1, 0.0, &jpeg);
        bytes[0] = 1; // lidar 帧
        let e = Parse_Camera_Frame(&bytes).unwrap_err();
        assert!(e.contains('2'), "错误信息缺期望类型：{e}");
        assert!(e.contains("camera"), "错误信息缺传感器名：{e}");
    }

    #[test]
    fn test_parse_rejects_empty_payload() {
        let bytes = Test_Frame(3, 1, 1.0, &[]);
        assert_eq!(bytes.len(), FRAME_HEADER_SIZE);
        let e = Parse_Camera_Frame(&bytes).unwrap_err();
        assert!(e.contains("空 payload"), "错误信息应说明空 payload：{e}");
    }

    #[test]
    fn test_parse_rejects_bad_jpeg() {
        // 完全不是 JPEG
        let e = Parse_Camera_Frame(&Test_Frame(3, 1, 0.0, &[0, 1, 2, 3])).unwrap_err();
        assert!(e.contains("JPEG"), "错误信息应说明不是 JPEG：{e}");
        // 是 JPEG 但没有 SOF（只有 SOI + EOI）
        let e = Parse_Camera_Frame(&Test_Frame(3, 2, 0.0, &[0xFF, 0xD8, 0xFF, 0xD9])).unwrap_err();
        assert!(e.contains("SOF"), "错误信息应说明找不到 SOF：{e}");
    }

    #[test]
    fn test_jpeg_size_sof0() {
        assert_eq!(Jpeg_Size(&Test_Jpeg(1280, 720)), Ok((1280, 720)));
        // SOF 段被截断（整段少 4 B）→ Err，不能静默给 (0,0)
        let mut truncated = Test_Jpeg(640, 480);
        let cut = truncated.len() - 4;
        truncated.truncate(cut);
        let e = Jpeg_Size(&truncated).unwrap_err();
        assert!(e.contains("损坏") || e.contains("截断") || e.contains("超出"), "错误信息应说明结构问题：{e}");
        // 宽为 0 → Err
        let zero = Test_Jpeg(0, 480);
        let e = Jpeg_Size(&zero).unwrap_err();
        assert!(e.contains("0"), "错误信息应说明宽高为 0：{e}");
        // 空 payload / 过短
        assert!(Jpeg_Size(&[]).is_err());
        assert!(Jpeg_Size(&[0xFF, 0xD8]).is_err());
    }

    #[test]
    fn test_jpeg_size_tolerates_sof2() {
        // 把 SOF0 的 marker 换成 SOF2（progressive）：仍能读出宽高（plan §9-R7）
        let mut j = Test_Jpeg(800, 600);
        // SOI(2) + APP0 段(18) → SOF marker 在偏移 20
        assert_eq!(j[20], 0xFF);
        assert_eq!(j[21], 0xC0);
        j[21] = 0xC2;
        assert_eq!(Jpeg_Size(&j), Ok((800, 600)));
    }
}
