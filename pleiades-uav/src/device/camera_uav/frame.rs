//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-28

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! `camera_uav` 的帧类型，以及「nokhwa `Buffer` → JPEG 字节」的转换。
//!
//! 两条路径（与 `rust_uav_camera_test/src/camera.rs` 已验证的行为一致）：
//! - **MJPG 源**：`jpeg = Some(原始字节)`、`rgb = None` —— **绝不 `decode_image`**。
//!   本仓 `nokhwa` 未启用 `decoding`/`mjpeg` feature，MJPEG 解码恒返回
//!   `NotImplementedError`（`nokhwa-core-0.1.9/src/types.rs:1509`），解码必然失败且白白耗时。
//! - **其它原始格式**（YUYV / NV12 / RAWRGB…）：`decode_image::<RgbFormat>()` 后
//!   用 `image` 重编码 JPEG，`jpeg` 与 `rgb` 同时可用。
//!
//! 失败语义：一律 `Err(String)` + 排查提示，**不 panic / 不 unwrap / 不 expect**；
//! `Frame::Into_Jpeg()` 不会用空 `Vec` 冒充成功。

use std::io::Cursor;

use image::{ImageFormat, RgbImage};
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::FrameFormat;
use nokhwa::Buffer;

/// 抓到的一帧。
#[derive(Debug)]
pub struct Frame {
    /// 源像素格式（来自 `Buffer::source_frame_format`）
    pub format: FrameFormat,
    /// 帧宽（MJPG 取 `Buffer::resolution`，其它取解码后 RGB 宽度）
    pub width: u32,
    /// 帧高（同上）
    pub height: u32,
    /// JPEG 字节：MJPG 为原始字节，其它格式为重编码结果
    pub jpeg: Option<Vec<u8>>,
    /// 解码后的 RGB（MJPG 源为 `None`）。
    /// Step 1 只出 JPEG，此处保留与 `rust_uav_camera_test` 同形，供后续信息查询/调试复用。
    #[allow(dead_code)]
    pub rgb: Option<RgbImage>,
}

impl Frame {
    /// 取走 JPEG 字节（**唯一**的取字节出口；槽位要 `Arc<[u8]>` + 尺寸，不能只借 `&[u8]`）。
    ///
    /// 失败语义：`jpeg` 缺失或为空 → `Err`，**绝不返回空 `Vec` 充成功**
    /// （空 JPEG 落盘后在 TUI 侧表现为「文件存在但 `file` 认不出」，极难排查）。
    ///
    /// 只提供「移动」版：槽位存 `Arc<[u8]>`，抓帧线程抓完就交出去，没必要再拷一份。
    pub fn Into_Jpeg(self) -> Result<Vec<u8>, String> {
        let (width, height, format) = (self.width, self.height, self.format);
        match self.jpeg {
            Some(bytes) if !bytes.is_empty() => Ok(bytes),
            _ => Err(No_Jpeg(width, height, format)),
        }
    }
}

/// 「这一帧没有 JPEG 字节」的统一文案（唯一的取字节出口 `Into_Jpeg` 用它，免得文案漂移）。
fn No_Jpeg(width: u32, height: u32, format: FrameFormat) -> String {
    format!(
        "{width}x{height} {format} 帧没有 JPEG 字节（MJPG 直通与原始格式重编码都没有产出）\n  \
         排查建议：抓帧日志里找解码/编码失败原因；确认 [camera_uav].prefer 与设备实际格式匹配"
    )
}

/// `Buffer` → [`Frame`]（在 worker 线程内调用）。
///
/// MJPG 直通；其它格式解码 RGB 后重编码 JPEG。解码与编码失败分开写文案。
pub fn Capture_From_Buffer(buffer: Buffer) -> Result<Frame, String> {
    let format = buffer.source_frame_format();
    let resolution = buffer.resolution();

    if format == FrameFormat::MJPEG {
        // 未启用 mjpeg 解码：只能直通原始字节，绝不 decode_image。
        return Ok(Frame {
            format,
            width: resolution.width(),
            height: resolution.height(),
            jpeg: Some(buffer.buffer().to_vec()),
            rgb: None,
        });
    }

    let rgb = buffer.decode_image::<RgbFormat>().map_err(|error| {
        format!(
            "解码 {format} 帧失败（{}x{}）：{error}\n  \
             排查建议：确认设备真的在出 {format}（v4l2-ctl --list-formats-ext）；\
             若设备只出 MJPG，把 [camera_uav].prefer 改成 \"mjpg\" 走直通",
            resolution.width(),
            resolution.height()
        )
    })?;
    let jpeg = Encode_Jpeg(&rgb)?;

    Ok(Frame {
        format,
        width: rgb.width(),
        height: rgb.height(),
        jpeg: Some(jpeg),
        rgb: Some(rgb),
    })
}

/// 把 RGB 图编码成 JPEG 字节（内存内，不落盘）。
pub fn Encode_Jpeg(rgb: &RgbImage) -> Result<Vec<u8>, String> {
    let mut buffer = Cursor::new(Vec::new());
    rgb.write_to(&mut buffer, ImageFormat::Jpeg)
        .map_err(|error| {
            format!(
            "JPEG 编码失败（{}x{}）：{error}\n  排查建议：确认图像尺寸合法、内存充足（free -h）",
            rgb.width(),
            rgb.height()
        )
        })?;
    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nokhwa::utils::Resolution;

    /// 最小的「像 JPEG」的字节串：`FF D8 FF E0` 开头、`FF D9` 结尾
    fn fake_jpeg() -> Vec<u8> {
        vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0xFF, 0xD9,
        ]
    }

    #[test]
    fn encode_jpeg_has_jpeg_magic() {
        let rgb = RgbImage::from_pixel(4, 3, image::Rgb([10, 20, 30]));
        let jpeg = Encode_Jpeg(&rgb).expect("JPEG 编码应成功");
        assert!(jpeg.len() > 4, "JPEG 字节太少：{}", jpeg.len());
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "JPEG 应以 FF D8 FF 开头");
        assert_eq!(
            &jpeg[jpeg.len() - 2..],
            &[0xFF, 0xD9],
            "JPEG 应以 FF D9 结尾"
        );
    }

    #[test]
    fn mjpg_passthrough_keeps_raw_bytes() {
        let raw = fake_jpeg();
        let buffer = Buffer::new(Resolution::new(64, 48), &raw, FrameFormat::MJPEG);
        let frame = Capture_From_Buffer(buffer).expect("MJPG 直通不该失败");

        // 直通：字节原样保留（证明没走 decode_image —— 未启用 mjpeg feature 时必返 NotImplementedError）
        assert_eq!(frame.format, FrameFormat::MJPEG);
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert!(frame.rgb.is_none(), "MJPG 不该解码出 RGB");
        assert_eq!(frame.Into_Jpeg().expect("应有 JPEG 字节"), raw);
    }

    #[test]
    fn yuyv_buffer_is_decoded_and_reencoded() {
        // 2x2 YUYV = 8 字节（4:2:2，每 4 字节出 2 像素）
        let raw = vec![0x80u8; 8];
        let buffer = Buffer::new(Resolution::new(2, 2), &raw, FrameFormat::YUYV);
        let frame = Capture_From_Buffer(buffer).expect("YUYV 解码应成功");

        assert_eq!(frame.format, FrameFormat::YUYV);
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert!(frame.rgb.is_some(), "原始格式应解出 RGB");
        let jpeg = frame.Into_Jpeg().expect("应重编码出 JPEG 字节");
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "重编码结果应是 JPEG");
    }

    #[test]
    fn frame_without_jpeg_is_err() {
        let frame = Frame {
            format: FrameFormat::MJPEG,
            width: 640,
            height: 480,
            jpeg: None,
            rgb: None,
        };
        let error = frame.Into_Jpeg().expect_err("没有 JPEG 字节时必须 Err");
        assert!(error.contains("640x480"), "文案应带尺寸：{error}");
        assert!(
            error.contains("没有 JPEG 字节"),
            "文案应说明缺 JPEG：{error}"
        );

        // 空 Vec 也不能冒充成功
        let empty = Frame {
            format: FrameFormat::MJPEG,
            width: 1,
            height: 1,
            jpeg: Some(Vec::new()),
            rgb: None,
        };
        assert!(empty.Into_Jpeg().is_err(), "空 JPEG 字节必须当成失败");
    }

    #[test]
    fn into_jpeg_moves_bytes() {
        // 有字节：**原样移动**出来（槽位直接拿这份 owned 字节，不再拷一遍）
        let raw = fake_jpeg();
        let frame = Frame {
            format: FrameFormat::MJPEG,
            width: 64,
            height: 48,
            jpeg: Some(raw.clone()),
            rgb: None,
        };
        assert_eq!(frame.Into_Jpeg().expect("有字节时必须 Ok"), raw);

        // 没字节：与有字节路径共用 `No_Jpeg` 文案（尺寸 + 「没有 JPEG 字节」）
        let missing = Frame {
            format: FrameFormat::MJPEG,
            width: 640,
            height: 480,
            jpeg: None,
            rgb: None,
        };
        let error = missing.Into_Jpeg().expect_err("没有 JPEG 字节时必须 Err");
        assert!(error.contains("640x480"), "文案应带尺寸：{error}");
        assert!(error.contains("没有 JPEG 字节"), "文案应说明缺 JPEG：{error}");

        // 空 Vec 同样不能冒充成功
        let empty = Frame {
            format: FrameFormat::MJPEG,
            width: 2,
            height: 2,
            jpeg: Some(Vec::new()),
            rgb: None,
        };
        assert!(empty.Into_Jpeg().is_err(), "空 JPEG 字节必须当成失败");
    }
}
