//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-24

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! `camera_uav` 的**纯函数**层：像素格式优先级 + 三级格式选择 + 枚举表渲染。
//!
//! 本模块零 IO、零设备、零全局状态，因此全部逻辑都能在无 `/dev/video*` 的机器上单测
//! （Task 34 Step 1 的验收要求：格式选择是纯函数 + 单测）。
//!
//! 选择策略（与 `rust_uav_camera_test/src/camera.rs` 在树莓派上验证过的路径等价，
//! 唯一差别是把写死的分辨率表换成配置驱动）：
//! 1. 按 `resolutions` 顺序 × `prefer` 指定的格式类（`mjpg` → MJPEG，`raw` → 非 MJPEG）；
//! 2. 按 `resolutions` 顺序 × 另一类格式（退化）；
//! 3. `Closest_Score` 在所有枚举结果里取最接近 `resolutions.last()` 者；
//!    枚举结果为空时给 `MJPEG {CLOSEST_WIDTH}x{CLOSEST_HEIGHT}@{CLOSEST_FPS}` 兜底值。
//!
//! **永不 panic**：空 slice、非法分辨率表、任意帧率目标都只走兜底分支。

use std::cmp::Reverse;

use nokhwa::utils::{CameraFormat, FrameFormat, Resolution};

/// Closest 兜底的默认目标宽度（`resolutions` 为空时使用）
pub const CLOSEST_WIDTH: u32 = 1280;
/// Closest 兜底的默认目标高度（`resolutions` 为空时使用）
pub const CLOSEST_HEIGHT: u32 = 720;
/// 枚举结果为空时的兜底帧率
pub const CLOSEST_FPS: u32 = 30;
/// 格式表最多打印多少行（stepwise 枚举可能产生上千项）
pub const MAX_FORMAT_LINES: usize = 2_000;

/// `[camera_uav].prefer`：先试哪一类像素格式。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Prefer_Format {
    /// 优先 MJPEG（原始 JPEG 字节直通，不重编码）
    Mjpg,
    /// 优先原始格式（YUYV/NV12/RAW… ，需解码 RGB 再重编码 JPEG）
    Raw,
}

impl Prefer_Format {
    /// 配置里对应的字面量（`"mjpg"` / `"raw"`），用于日志与理由文案
    pub fn As_Str(self) -> &'static str {
        match self {
            Prefer_Format::Mjpg => "mjpg",
            Prefer_Format::Raw => "raw",
        }
    }
}

/// 解析 `[camera_uav].prefer`。
///
/// 大小写不敏感、允许首尾空白、接受 `mjpeg` 别名（与 `FrameFormat::MJPEG` 的官方写法一致）。
/// 失败时 `Err` 的文案必须同时列出合法值 + 「已回退，请修改配置」提示（现场排障唯一依据）。
pub fn Parse_Prefer(raw: &str) -> Result<Prefer_Format, String> {
    match raw.trim().to_lowercase().as_str() {
        "mjpg" | "mjpeg" => Ok(Prefer_Format::Mjpg),
        "raw" => Ok(Prefer_Format::Raw),
        _ => Err(format!(
            "camera_uav.prefer=\"{raw}\" 非法：合法值只有 \"mjpg\"（默认，原始 JPEG 字节直通，不重编码）\
             与 \"raw\"（解码 RGB 再重编码 JPEG；别名 \"mjpeg\" 等同 \"mjpg\"）\n  \
             已回退为默认 prefer = \"mjpg\"，请修改配置或删除该键"
        )),
    }
}

/// 像素格式优先级：MJPEG=0（可直通）→ RAWRGB=1 → RAWBGR=2 → YUYV=3 → NV12=4 → GRAY=5。
pub fn Format_Rank(format: FrameFormat) -> u8 {
    match format {
        FrameFormat::MJPEG => 0,
        FrameFormat::RAWRGB => 1,
        FrameFormat::RAWBGR => 2,
        FrameFormat::YUYV => 3,
        FrameFormat::NV12 => 4,
        FrameFormat::GRAY => 5,
    }
}

/// 精确 (像素格式 + 分辨率) 命中里挑帧率：
/// `fps == 0` → 枚举到的最高帧率；`fps > 0` → `|Δfps|` 最小，同分取更高帧率。
/// 无命中 → `None`。
pub fn Best_At(
    all: &[CameraFormat],
    wanted: FrameFormat,
    width: u32,
    height: u32,
    fps: u32,
) -> Option<CameraFormat> {
    let candidates: Vec<CameraFormat> = all
        .iter()
        .copied()
        .filter(|candidate| {
            candidate.format() == wanted
                && candidate.width() == width
                && candidate.height() == height
        })
        .collect();

    Best_Fps(&candidates, fps)
}

/// 非 MJPEG 的「原始格式」命中：先 [`Format_Rank`]，同秩再按帧率（规则同 [`Best_At`]）。
/// 无命中 → `None`。
pub fn Best_Raw_At(
    all: &[CameraFormat],
    width: u32,
    height: u32,
    fps: u32,
) -> Option<CameraFormat> {
    let candidates: Vec<CameraFormat> = all
        .iter()
        .copied()
        .filter(|candidate| {
            candidate.format() != FrameFormat::MJPEG
                && candidate.width() == width
                && candidate.height() == height
        })
        .collect();

    let best_rank = candidates
        .iter()
        .map(|candidate| Format_Rank(candidate.format()))
        .min()?;
    let same_rank: Vec<CameraFormat> = candidates
        .into_iter()
        .filter(|candidate| Format_Rank(candidate.format()) == best_rank)
        .collect();

    Best_Fps(&same_rank, fps)
}

/// Closest 兜底的打分（纯排序键，越小越好）：
/// `(到目标的曼哈顿距离, 格式优先级, Reverse(帧率))`。
pub fn Closest_Score(
    format: CameraFormat,
    target_width: u32,
    target_height: u32,
) -> (u64, u8, Reverse<u32>) {
    let distance = u64::from(format.width().abs_diff(target_width))
        + u64::from(format.height().abs_diff(target_height));
    (
        distance,
        Format_Rank(format.format()),
        Reverse(format.frame_rate()),
    )
}

/// 三级格式选择。返回 `(选中格式, 人类可读理由)`，**永不 panic**（空 slice 也给兜底值）。
pub fn Choose_Format(
    all: &[CameraFormat],
    prefer: Prefer_Format,
    resolutions: &[(u32, u32)],
    fps: u32,
) -> (CameraFormat, String) {
    let (closest_width, closest_height) = Closest_Target(resolutions);

    // 第 1、2 级：按 prefer 决定先试 MJPEG 还是先试原始格式。
    let mjpg_first = prefer == Prefer_Format::Mjpg;
    for pass in 0..2 {
        let try_mjpg = if pass == 0 { mjpg_first } else { !mjpg_first };
        if try_mjpg {
            if let Some(best) = First_At(all, FrameFormat::MJPEG, resolutions, fps) {
                let line = format!(
                    "{}x{}@{}FPS",
                    best.width(),
                    best.height(),
                    best.frame_rate()
                );
                let reason = if mjpg_first {
                    format!("MJPG 直通优先：{line}（原始 JPEG 字节直通，不重编码）")
                } else {
                    format!(
                        "按配置 prefer={} 但无可用的原始格式，退化到 MJPG 直通：{line}（原始 JPEG 字节直通，不重编码）",
                        prefer.As_Str()
                    )
                };
                return (best, reason);
            }
        } else if let Some(best) = First_Raw_At(all, resolutions, fps) {
            let line = format!(
                "{}x{}@{}FPS",
                best.width(),
                best.height(),
                best.frame_rate()
            );
            let reason = if mjpg_first {
                format!(
                    "MJPG 不可用，退化到原始格式 {}：{line}（需解码 RGB 再重编码 JPEG）",
                    best.format()
                )
            } else {
                format!(
                    "按配置 prefer={} 选中原始格式 {}：{line}（需解码 RGB 再重编码 JPEG）",
                    prefer.As_Str(),
                    best.format()
                )
            };
            return (best, reason);
        }
    }

    // 第 3 级：Closest 兜底。
    if all.is_empty() {
        let fallback = CameraFormat::new(
            Resolution::new(closest_width, closest_height),
            FrameFormat::MJPEG,
            CLOSEST_FPS,
        );
        return (
            fallback,
            format!(
                "无首选组合，Closest 兜底：{}（枚举结果为空，目标 {}x{}@{}FPS MJPEG）",
                Format_Line(fallback),
                closest_width,
                closest_height,
                CLOSEST_FPS
            ),
        );
    }

    let mut best = all[0];
    let mut best_score = Closest_Score(best, closest_width, closest_height);
    for candidate in all {
        let score = Closest_Score(*candidate, closest_width, closest_height);
        if score < best_score {
            best = *candidate;
            best_score = score;
        }
    }

    (
        best,
        format!(
            "无首选组合，Closest 兜底：{}（在枚举结果中取最接近 {}x{} 者，同分优先 MJPG、再取高帧率）",
            Format_Line(best),
            closest_width,
            closest_height
        ),
    )
}

/// 单行格式描述，例如 `1920x1080@30FPS  MJPEG`。
pub fn Format_Line(format: CameraFormat) -> String {
    format!(
        "{}x{}@{}FPS  {}",
        format.width(),
        format.height(),
        format.frame_rate(),
        format.format()
    )
}

/// 枚举表（去重 + 按格式优先级/分辨率降序/帧率降序 + 截断），用于失败信息与 debug 日志。
pub fn Format_Table(all: &[CameraFormat], warning: Option<&str>) -> Vec<String> {
    let mut formats = all.to_vec();
    formats.sort_by(|left, right| {
        Format_Rank(left.format())
            .cmp(&Format_Rank(right.format()))
            // 分辨率降序：不使用 Resolution 自带的 Ord（它是反向的）。
            .then_with(|| right.width().cmp(&left.width()))
            .then_with(|| right.height().cmp(&left.height()))
            .then_with(|| right.frame_rate().cmp(&left.frame_rate()))
    });
    formats.dedup();

    let total = formats.len();
    let mut lines: Vec<String> = Vec::new();
    if let Some(warning) = warning {
        lines.push(format!("  [警告] {warning}"));
    }
    lines.push(format!(
        "  共 {total} 项（像素格式 / 分辨率 / 帧率，MJPG 优先排列）："
    ));

    if total == 0 {
        lines.push("  （枚举结果为空）".to_string());
        return lines;
    }

    for (position, format) in formats.iter().take(MAX_FORMAT_LINES).enumerate() {
        lines.push(format!("  #{:<4} {}", position + 1, Format_Line(*format)));
    }
    if total > MAX_FORMAT_LINES {
        lines.push(format!(
            "  ... 另有 {} 项未打印（上限 MAX_FORMAT_LINES = {MAX_FORMAT_LINES}）",
            total - MAX_FORMAT_LINES
        ));
    }

    lines
}

/// Closest 目标分辨率 = `resolutions.last()`；`resolutions` 为空时退
/// [`CLOSEST_WIDTH`] / [`CLOSEST_HEIGHT`]。
fn Closest_Target(resolutions: &[(u32, u32)]) -> (u32, u32) {
    resolutions
        .last()
        .copied()
        .unwrap_or((CLOSEST_WIDTH, CLOSEST_HEIGHT))
}

/// 按 `resolutions` 顺序找第一个命中的指定像素格式。
fn First_At(
    all: &[CameraFormat],
    wanted: FrameFormat,
    resolutions: &[(u32, u32)],
    fps: u32,
) -> Option<CameraFormat> {
    resolutions
        .iter()
        .find_map(|(width, height)| Best_At(all, wanted, *width, *height, fps))
}

/// 按 `resolutions` 顺序找第一个命中的原始格式（非 MJPEG）。
fn First_Raw_At(
    all: &[CameraFormat],
    resolutions: &[(u32, u32)],
    fps: u32,
) -> Option<CameraFormat> {
    resolutions
        .iter()
        .find_map(|(width, height)| Best_Raw_At(all, *width, *height, fps))
}

/// 帧率挑选规则：`fps == 0` → 最高帧率；`fps > 0` → `|Δfps|` 最小，同分取更高帧率。
fn Best_Fps(candidates: &[CameraFormat], fps: u32) -> Option<CameraFormat> {
    if candidates.is_empty() {
        return None;
    }
    if fps == 0 {
        return candidates
            .iter()
            .copied()
            .max_by_key(|candidate| candidate.frame_rate());
    }
    candidates.iter().copied().min_by_key(|candidate| {
        (
            candidate.frame_rate().abs_diff(fps),
            Reverse(candidate.frame_rate()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个枚举项（格式 + 分辨率 + 帧率）
    fn Cam(format: FrameFormat, width: u32, height: u32, fps: u32) -> CameraFormat {
        CameraFormat::new(Resolution::new(width, height), format, fps)
    }

    const DEFAULT_RESOLUTIONS: [(u32, u32); 2] = [(640, 480), (1280, 720)];

    #[test]
    fn format_rank_order() {
        assert_eq!(Format_Rank(FrameFormat::MJPEG), 0);
        assert_eq!(Format_Rank(FrameFormat::RAWRGB), 1);
        assert_eq!(Format_Rank(FrameFormat::RAWBGR), 2);
        assert_eq!(Format_Rank(FrameFormat::YUYV), 3);
        assert_eq!(Format_Rank(FrameFormat::NV12), 4);
        assert_eq!(Format_Rank(FrameFormat::GRAY), 5);
        assert!(
            Format_Rank(FrameFormat::MJPEG) < Format_Rank(FrameFormat::NV12),
            "MJPEG 必须最优先（可直通）"
        );
    }

    #[test]
    fn parse_prefer_ok() {
        assert_eq!(Parse_Prefer("mjpg"), Ok(Prefer_Format::Mjpg));
        assert_eq!(Parse_Prefer("MJPG"), Ok(Prefer_Format::Mjpg));
        assert_eq!(Parse_Prefer("  mjpeg  "), Ok(Prefer_Format::Mjpg));
        assert_eq!(Parse_Prefer("raw"), Ok(Prefer_Format::Raw));
        assert_eq!(Parse_Prefer(" RAW "), Ok(Prefer_Format::Raw));
        assert_eq!(Prefer_Format::Mjpg.As_Str(), "mjpg");
        assert_eq!(Prefer_Format::Raw.As_Str(), "raw");
    }

    #[test]
    fn parse_prefer_rejects_unknown() {
        for bad in ["", "   ", "jpeg", "yuv", "mjpg2", "rbg"] {
            let error = Parse_Prefer(bad).expect_err(&format!("应拒绝 {bad:?}"));
            assert!(error.contains("mjpg"), "文案应列出合法值：{error}");
            assert!(error.contains("raw"), "文案应列出合法值：{error}");
            assert!(error.contains("已回退"), "文案应说明已回退：{error}");
        }
    }

    #[test]
    fn best_at_picks_highest_fps() {
        let all = [
            Cam(FrameFormat::MJPEG, 640, 480, 15),
            Cam(FrameFormat::MJPEG, 640, 480, 30),
            Cam(FrameFormat::MJPEG, 1280, 720, 30),
        ];
        // fps = 0 → 该组合枚举到的最高帧率
        assert_eq!(
            Best_At(&all, FrameFormat::MJPEG, 640, 480, 0),
            Some(Cam(FrameFormat::MJPEG, 640, 480, 30))
        );
        // 分辨率不命中 → None
        assert_eq!(Best_At(&all, FrameFormat::MJPEG, 320, 240, 0), None);
        // 像素格式不命中 → None
        assert_eq!(Best_At(&all, FrameFormat::YUYV, 640, 480, 0), None);
    }

    #[test]
    fn best_at_honours_fps_target() {
        let all = [
            Cam(FrameFormat::MJPEG, 1280, 720, 15),
            Cam(FrameFormat::MJPEG, 1280, 720, 30),
        ];
        // 目标 20：|15-20|=5 < |30-20|=10 → 取 15
        assert_eq!(
            Best_At(&all, FrameFormat::MJPEG, 1280, 720, 20),
            Some(Cam(FrameFormat::MJPEG, 1280, 720, 15))
        );
        // 目标 24：|30-24|=6 < |15-24|=9 → 取 30
        assert_eq!(
            Best_At(&all, FrameFormat::MJPEG, 1280, 720, 24),
            Some(Cam(FrameFormat::MJPEG, 1280, 720, 30))
        );
        // 等距：目标 25 距 15/30 各 10 → 同分取更高帧率
        assert_eq!(
            Best_At(&all, FrameFormat::MJPEG, 1280, 720, 25),
            Some(Cam(FrameFormat::MJPEG, 1280, 720, 30))
        );
    }

    #[test]
    fn best_raw_at_prefers_rgb_over_yuyv() {
        let all = [
            Cam(FrameFormat::YUYV, 640, 480, 30),
            Cam(FrameFormat::RAWRGB, 640, 480, 15),
            Cam(FrameFormat::MJPEG, 640, 480, 30),
        ];
        // RAWRGB(1) 优先于 YUYV(3)，与帧率无关；MJPEG 不属于原始格式
        assert_eq!(
            Best_Raw_At(&all, 640, 480, 0),
            Some(Cam(FrameFormat::RAWRGB, 640, 480, 15))
        );
        // 只有 MJPEG 时无原始格式可用
        let mjpg_only = [Cam(FrameFormat::MJPEG, 640, 480, 30)];
        assert_eq!(Best_Raw_At(&mjpg_only, 640, 480, 0), None);
    }

    #[test]
    fn choose_prefers_mjpg_first_resolution() {
        let all = [
            Cam(FrameFormat::MJPEG, 640, 480, 30),
            Cam(FrameFormat::MJPEG, 1280, 720, 30),
            Cam(FrameFormat::YUYV, 640, 480, 30),
        ];
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Mjpg, &DEFAULT_RESOLUTIONS, 0);
        // 按 resolutions 顺序：640x480 在前，优先于更高分辨率
        assert_eq!(chosen, Cam(FrameFormat::MJPEG, 640, 480, 30));
        assert!(reason.contains("MJPG 直通优先"), "{reason}");
        assert!(reason.contains("640x480"), "{reason}");
        assert!(reason.contains("不重编码"), "{reason}");
    }

    #[test]
    fn choose_falls_back_to_second_resolution() {
        let all = [
            Cam(FrameFormat::MJPEG, 1280, 720, 30),
            Cam(FrameFormat::YUYV, 640, 480, 30),
        ];
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Mjpg, &DEFAULT_RESOLUTIONS, 0);
        // 640x480 没有 MJPEG，但 1280x720 有 → 第 1 级第 2 个分辨率命中
        assert_eq!(chosen, Cam(FrameFormat::MJPEG, 1280, 720, 30));
        assert!(reason.contains("MJPG 直通优先"), "{reason}");
        assert!(reason.contains("1280x720"), "{reason}");
    }

    #[test]
    fn choose_prefer_raw_orders_raw_first() {
        let all = [
            Cam(FrameFormat::MJPEG, 640, 480, 30),
            Cam(FrameFormat::YUYV, 640, 480, 30),
        ];
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Raw, &DEFAULT_RESOLUTIONS, 0);
        // prefer=raw：即使 MJPEG 可用，也先选原始格式
        assert_eq!(chosen, Cam(FrameFormat::YUYV, 640, 480, 30));
        assert!(reason.contains("prefer=raw"), "{reason}");
        assert!(reason.contains("YUYV"), "{reason}");
    }

    #[test]
    fn choose_raw_when_no_mjpg() {
        let all = [Cam(FrameFormat::YUYV, 640, 480, 30)];
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Mjpg, &DEFAULT_RESOLUTIONS, 0);
        // MJPG 缺失 → 退化到原始格式（第 2 级）
        assert_eq!(chosen, Cam(FrameFormat::YUYV, 640, 480, 30));
        assert!(reason.contains("MJPG 不可用，退化到原始格式"), "{reason}");
        assert!(reason.contains("YUYV"), "{reason}");
    }

    #[test]
    fn choose_closest_when_no_preferred() {
        let all = [
            Cam(FrameFormat::MJPEG, 1920, 1080, 30),
            Cam(FrameFormat::YUYV, 1920, 1080, 15),
        ];
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Mjpg, &DEFAULT_RESOLUTIONS, 0);
        // 两个首选分辨率都没有命中 → Closest 兜底（目标 = resolutions.last() = 1280x720）
        assert_eq!(chosen, Cam(FrameFormat::MJPEG, 1920, 1080, 30));
        assert!(reason.contains("无首选组合，Closest 兜底"), "{reason}");
        assert!(reason.contains("1280x720"), "{reason}");
    }

    #[test]
    fn choose_empty_enumerations_returns_fallback() {
        let (chosen, reason) = Choose_Format(&[], Prefer_Format::Mjpg, &DEFAULT_RESOLUTIONS, 0);
        assert_eq!(
            chosen,
            Cam(
                FrameFormat::MJPEG,
                CLOSEST_WIDTH,
                CLOSEST_HEIGHT,
                CLOSEST_FPS
            )
        );
        assert!(reason.contains("Closest 兜底"), "{reason}");
        assert!(reason.contains("枚举结果为空"), "{reason}");
    }

    #[test]
    fn choose_empty_resolutions_falls_to_closest_target() {
        // resolutions = [] → 无偏好，Closest 目标退回 CLOSEST_WIDTH/HEIGHT
        let all = [Cam(FrameFormat::MJPEG, CLOSEST_WIDTH, CLOSEST_HEIGHT, 30)];
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Mjpg, &[], 0);
        assert_eq!(
            chosen,
            Cam(FrameFormat::MJPEG, CLOSEST_WIDTH, CLOSEST_HEIGHT, 30)
        );
        assert!(reason.contains("Closest 兜底"), "{reason}");
    }

    #[test]
    fn closest_score_prefers_nearer_and_higher_fps() {
        let near = Cam(FrameFormat::MJPEG, 1280, 720, 30);
        let far = Cam(FrameFormat::MJPEG, 1920, 1080, 30);
        assert!(
            Closest_Score(near, CLOSEST_WIDTH, CLOSEST_HEIGHT)
                < Closest_Score(far, CLOSEST_WIDTH, CLOSEST_HEIGHT)
        );
        // 同距离同格式 → 帧率高的更小
        let low = Cam(FrameFormat::MJPEG, 1280, 720, 15);
        assert!(
            Closest_Score(near, CLOSEST_WIDTH, CLOSEST_HEIGHT)
                < Closest_Score(low, CLOSEST_WIDTH, CLOSEST_HEIGHT)
        );
    }

    #[test]
    fn format_line_and_table() {
        let all = [
            Cam(FrameFormat::YUYV, 640, 480, 30),
            Cam(FrameFormat::MJPEG, 1280, 720, 30),
            Cam(FrameFormat::MJPEG, 1280, 720, 30),
        ];
        assert_eq!(
            Format_Line(Cam(FrameFormat::MJPEG, 1920, 1080, 30)),
            "1920x1080@30FPS  MJPEG"
        );

        let lines = Format_Table(&all, Some("枚举返回空列表"));
        assert!(lines[0].contains("[警告] 枚举返回空列表"), "{:?}", lines);
        // 去重后共 2 项，MJPEG 排在第一
        assert!(lines[1].contains("共 2 项"), "{:?}", lines);
        assert!(lines[2].contains("MJPEG"), "{:?}", lines);
        assert!(lines[3].contains("YUYV"), "{:?}", lines);

        let empty = Format_Table(&[], None);
        assert!(
            empty.iter().any(|line| line.contains("枚举结果为空")),
            "{empty:?}"
        );
    }
}
