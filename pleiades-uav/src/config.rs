//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-24

//! UAV（机）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）
//!
//! Task 23 §3.8：设备段由设备端自己负责。本文件除定义机独有字段外，
//! 还负责首次启动时把机端设备段补全到 config.toml。
//!
//! Task 34 Step 1：相机拆成**两段互不继承的配置**——
//! - `[camera_pc]`：开发机（电脑）摄像头，沿用 NV12 实现，**默认 `false`**；
//! - `[camera_uav]`：UAV 机载相机（SY011HD / UVC），MJPG 直通 + 格式协商，**默认 `true`**。
//!
//! 旧 `[camera]` 段（Task 28/29）**保留解析但一律不读取**：仍然派生
//! `Debug/Clone/Deserialize`，代码里只判 `is_some()`（bootstrap 打一行「已忽略」提示），
//! 免得老配置文件直接解析失败；同时 `fill_*` 不再生成它，避免自己刚补出来又自己 warn。

use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

use pleiades_base::config::{config_file_path, BaseConfig};

/// `[camera_uav].resolutions` 的默认值（缺失时写入，也作为运行时缺省）
pub const DEFAULT_UAV_RESOLUTIONS: [[u32; 2]; 2] = [[640, 480], [1280, 720]];
/// `[camera_uav].open_timeout_ms` 的默认值（毫秒）
pub const DEFAULT_UAV_OPEN_TIMEOUT_MS: u64 = 8_000;
/// `[camera_uav].capture_timeout_ms` 的默认值（毫秒）
pub const DEFAULT_UAV_CAPTURE_TIMEOUT_MS: u64 = 5_000;

/// UAV 完整配置 = 共享段（flatten）+ 机独有设备段
#[derive(Debug, Clone, Deserialize)]
pub struct UavConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 飞控设备配置（MAVLink）
    pub flight_ctrl: Option<FlightCtrlConfig>,
    /// 他车障碍膨胀半径（米，缺省 0.2；「天上小车」2D 寻路阶段使用）
    pub obstacle_inflation_radius: Option<f32>,
    /// **旧**摄像头段（Task 28/29）：保留解析，运行时**只判 `is_some()`** 打迁移提示，
    /// 任何字段都不读取（Task 34 Step 1）
    pub camera: Option<CameraConfig>,
    /// 开发机（电脑）摄像头 sensor（Task 34 Step 1）
    pub camera_pc: Option<CameraPcConfig>,
    /// UAV 机载相机 sensor（Task 34 Step 1）
    pub camera_uav: Option<CameraUavConfig>,
}

/// 飞控设备配置（Pixhawk / MAVLink）
#[derive(Debug, Clone, Deserialize)]
pub struct FlightCtrlConfig {
    pub enabled: Option<bool>,
    /// 连接方式（如串口 /dev/ttyS0 或 MAVLink UDP 地址）
    pub connection: Option<String>,
    /// 串口波特率（默认 921600）
    pub baudrate: Option<u32>,
    /// 前进/后退速度 (m/s，缺省 0.3，Task 22_5 D2：速度由设备层绑定)
    pub vel_fwd: Option<f32>,
    /// 转向角速度 (°/s，缺省 15)
    pub yaw_rate_deg: Option<f32>,
}

/// **旧**摄像头设备配置（USB UVC，Task 28/29）。
///
/// Task 34 Step 1 起只用于「检测到旧段 → 提示改用 `[camera_pc]`/`[camera_uav]`」，
/// 字段保留解析以兼容老配置文件，运行时**不读取**。
///
/// `#[allow(dead_code)]` 是必要的：`derive(Debug)` 不算「读字段」，但旧段只被
/// `is_some()` 判一下，否则会多出一条 `fields ... are never read` warning（本仓 warning 数要持平基线）。
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CameraConfig {
    pub enabled: Option<bool>,
    /// 设备路径：纯数字 = 索引（Windows 常用 "0"），否则 = 路径（Linux 常用 "/dev/video0"）
    pub path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub timeout_ms: Option<u64>,
}

/// 开发机（电脑）摄像头 sensor 配置（`[camera_pc]`，默认关）
#[derive(Debug, Clone, Deserialize)]
pub struct CameraPcConfig {
    pub enabled: Option<bool>,
    /// 设备路径：`"0"` / `"1"` 或 `"/dev/video0"`（**不接受**其它字符串路径）
    pub path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub timeout_ms: Option<u64>,
}

/// UAV 机载相机 sensor 配置（`[camera_uav]`，默认开）
#[derive(Debug, Clone, Deserialize)]
pub struct CameraUavConfig {
    pub enabled: Option<bool>,
    /// 设备路径：`"0"` / `"1"` 或 `"/dev/video0"`
    pub path: Option<String>,
    /// 优先像素格式：`"mjpg"`（默认）或 `"raw"`
    pub prefer: Option<String>,
    /// 首选分辨率（按顺序尝试；`[]` = 无分辨率偏好，直接走 Closest 兜底）
    pub resolutions: Option<Vec<[u32; 2]>>,
    /// 目标帧率：`0`（默认）= 取该组合枚举到的最高帧率
    pub fps: Option<u32>,
    pub open_timeout_ms: Option<u64>,
    pub capture_timeout_ms: Option<u64>,
}

/// 读取 UAV 配置（.config/config.toml）。
///
/// 首次启动时若 config.toml 缺失机端设备段，会先补全默认设备段再读取（幂等，不覆盖已有值）。
pub fn Ensure_Uav_Config() -> Result<UavConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    // 配置文件不存在则先由 base 生成共享段（防御性：正常流程 core_bootstrap 已生成）
    if !path.exists() {
        pleiades_base::config::Ensure_Config()?;
    }
    ensure_uav_section(&path)?;
    let content = fs::read_to_string(&path)?;
    let config: UavConfig = toml::from_str(&content)?;
    Ok(config)
}

/// 确保 config.toml 存在机端设备段（obstacle_inflation_radius / flight_ctrl /
/// camera_pc / camera_uav），缺失才补默认值。
///
/// **不再**补旧 `[camera]` 段：已存在的旧段原样留在文件里（由 bootstrap 打一行「已忽略」），
/// 新文件不会凭空生出旧段。
fn ensure_uav_section(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut doc: DocumentMut = content.parse()?;
    let mut changed = false;

    if doc.get("obstacle_inflation_radius").is_none() {
        doc["obstacle_inflation_radius"] = toml_edit::value(0.2_f64);
        changed = true;
    }
    changed |= fill_flight_ctrl(&mut doc);
    changed |= fill_camera_pc(&mut doc);
    changed |= fill_camera_uav(&mut doc);

    if changed {
        fs::write(path, doc.to_string())?;
    }
    Ok(())
}

/// 补全 [flight_ctrl] 段（缺失才补，幂等）
fn fill_flight_ctrl(doc: &mut DocumentMut) -> bool {
    if doc.get("flight_ctrl").is_none() {
        doc["flight_ctrl"] = Item::Table(toml_edit::Table::new());
    }
    let fc = doc["flight_ctrl"].as_table_mut().expect("flight_ctrl 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 4] = [
        ("enabled", toml_edit::value(false)),
        ("baudrate", toml_edit::value(921600_i64)),
        ("vel_fwd", toml_edit::value(0.3_f64)),
        ("yaw_rate_deg", toml_edit::value(15.0_f64)),
    ];
    for (k, v) in defaults {
        if !fc.contains_key(k) {
            fc.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [camera_pc] 段（缺失才补，幂等；**不覆盖**用户已写的值）
fn fill_camera_pc(doc: &mut DocumentMut) -> bool {
    if doc.get("camera_pc").is_none() {
        doc["camera_pc"] = Item::Table(toml_edit::Table::new());
    }
    let Some(table) = doc["camera_pc"].as_table_mut() else {
        return false;
    };
    let mut changed = false;
    let defaults: [(&str, Item); 5] = [
        // 开发机摄像头默认关：真机（树莓派）上通常没有 /dev/video0
        ("enabled", toml_edit::value(false)),
        ("path", toml_edit::value("0")),
        ("width", toml_edit::value(1280_i64)),
        ("height", toml_edit::value(720_i64)),
        ("timeout_ms", toml_edit::value(3000_i64)),
    ];
    for (k, v) in defaults {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [camera_uav] 段（缺失才补，幂等；**不覆盖**用户已写的值）
fn fill_camera_uav(doc: &mut DocumentMut) -> bool {
    if doc.get("camera_uav").is_none() {
        doc["camera_uav"] = Item::Table(toml_edit::Table::new());
    }
    let Some(table) = doc["camera_uav"].as_table_mut() else {
        return false;
    };
    let mut changed = false;

    let defaults: [(&str, Item); 4] = [
        // 机载相机默认开：树莓派上这是主力相机
        ("enabled", toml_edit::value(true)),
        ("path", toml_edit::value("0")),
        ("prefer", toml_edit::value("mjpg")),
        ("fps", toml_edit::value(0_i64)),
    ];
    for (k, v) in defaults {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }

    if !table.contains_key("resolutions") {
        // 嵌套数组：外层 Array 的每个元素是内层 Array → 渲染为 [[640, 480], [1280, 720]]
        let mut outer = toml_edit::Array::new();
        for pair in DEFAULT_UAV_RESOLUTIONS {
            let mut inner = toml_edit::Array::new();
            inner.push(i64::from(pair[0]));
            inner.push(i64::from(pair[1]));
            outer.push(inner);
        }
        table.insert("resolutions", toml_edit::value(outer));
        changed = true;
    }

    let timeouts: [(&str, Item); 2] = [
        (
            "open_timeout_ms",
            toml_edit::value(DEFAULT_UAV_OPEN_TIMEOUT_MS as i64),
        ),
        (
            "capture_timeout_ms",
            toml_edit::value(DEFAULT_UAV_CAPTURE_TIMEOUT_MS as i64),
        ),
    ];
    for (k, v) in timeouts {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}", tag, nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_uav_section_fills_defaults() {
        let dir = tmp_dir("uav_cfg_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();

        assert_eq!(parsed.obstacle_inflation_radius, Some(0.2));
        let f = parsed.flight_ctrl.expect("flight_ctrl 段应被补全");
        assert_eq!(f.enabled, Some(false));
        assert_eq!(f.baudrate, Some(921600));
        assert_eq!(f.vel_fwd, Some(0.3));
        assert_eq!(f.yaw_rate_deg, Some(15.0));

        // 旧段不再被凭空补出来（否则会立刻被自己的「已忽略」提示打脸）
        assert!(parsed.camera.is_none(), "不应自动生成旧 [camera] 段");

        let pc = parsed.camera_pc.expect("camera_pc 段应被补全");
        assert_eq!(pc.enabled, Some(false), "开发机摄像头默认关");
        assert_eq!(pc.path.as_deref(), Some("0"));
        assert_eq!(pc.width, Some(1280));
        assert_eq!(pc.height, Some(720));
        assert_eq!(pc.timeout_ms, Some(3000));

        let uav = parsed.camera_uav.expect("camera_uav 段应被补全");
        assert_eq!(uav.enabled, Some(true), "机载相机默认开");
        assert_eq!(uav.path.as_deref(), Some("0"));
        assert_eq!(uav.prefer.as_deref(), Some("mjpg"));
        assert_eq!(
            uav.resolutions,
            Some(vec![[640, 480], [1280, 720]]),
            "resolutions 默认应为 640x480 在前、1280x720 在后"
        );
        assert_eq!(uav.fps, Some(0));
        assert_eq!(uav.open_timeout_ms, Some(8000));
        assert_eq!(uav.capture_timeout_ms, Some(5000));

        // 落盘的文本要是标准 TOML 的两段独立表
        assert!(content.contains("[camera_pc]"), "{content}");
        assert!(content.contains("[camera_uav]"), "{content}");
        assert!(content.contains("[[640, 480], [1280, 720]]"), "{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_uav_section_keeps_existing() {
        let dir = tmp_dir("uav_cfg_keep");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[flight_ctrl]\nenabled = true\nbaudrate = 115200\n\n\
             [camera_uav]\nenabled = false\nprefer = \"raw\"\nresolutions = [[1920, 1080]]\n",
        )
        .unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let f = parsed.flight_ctrl.unwrap();
        assert_eq!(f.enabled, Some(true));
        assert_eq!(f.baudrate, Some(115200));
        assert_eq!(f.vel_fwd, Some(0.3));
        assert_eq!(f.yaw_rate_deg, Some(15.0));

        // 用户写的 camera_uav 值一律不覆盖，只补缺的键
        let uav = parsed.camera_uav.unwrap();
        assert_eq!(uav.enabled, Some(false), "不得覆盖用户写的 enabled");
        assert_eq!(uav.prefer.as_deref(), Some("raw"), "不得覆盖用户写的 prefer");
        assert_eq!(
            uav.resolutions,
            Some(vec![[1920, 1080]]),
            "不得覆盖用户写的 resolutions"
        );
        assert_eq!(uav.fps, Some(0), "缺失的 fps 应被补成 0");
        assert_eq!(uav.open_timeout_ms, Some(8000), "缺失的 open_timeout_ms 应被补");
        assert_eq!(
            uav.capture_timeout_ms,
            Some(5000),
            "缺失的 capture_timeout_ms 应被补"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_uav_section_is_idempotent() {
        let dir = tmp_dir("uav_cfg_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_uav_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        assert_eq!(
            first.matches("[camera_pc]").count(),
            1,
            "不得重复插入 [camera_pc]：\n{first}"
        );
        assert_eq!(
            first.matches("[camera_uav]").count(),
            1,
            "不得重复插入 [camera_uav]：\n{first}"
        );
        assert_eq!(
            first.matches("resolutions").count(),
            1,
            "不得重复插入 resolutions：\n{first}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_camera_section_still_parses() {
        let dir = tmp_dir("uav_cfg_legacy");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[camera]\nenabled = true\npath = \"/dev/video2\"\nwidth = 640\nheight = 480\ntimeout_ms = 1500\n",
        )
        .unwrap();

        // 补段不能因为旧段而失败，也不能删/改旧段
        ensure_uav_section(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[camera]"), "旧段应原样保留：\n{content}");

        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let legacy = parsed.camera.expect("旧 [camera] 段仍应能解析");
        assert_eq!(legacy.enabled, Some(true));
        assert_eq!(legacy.path.as_deref(), Some("/dev/video2"));
        assert_eq!(legacy.width, Some(640));
        assert_eq!(legacy.height, Some(480));
        assert_eq!(legacy.timeout_ms, Some(1500));

        // 同时新段也被补全（新旧共存不冲突）
        assert!(parsed.camera_pc.is_some());
        assert!(parsed.camera_uav.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uav_defaults_when_fields_missing() {
        // 只写 enabled：其余键缺失时读成 None，由各 sensor 自己 unwrap_or 兜底
        let config: UavConfig = toml::from_str("[camera_uav]\nenabled = true\n").unwrap();
        let uav = config.camera_uav.expect("camera_uav 应解析出来");
        assert_eq!(uav.enabled, Some(true));
        assert_eq!(uav.path, None);
        assert_eq!(uav.prefer, None);
        assert_eq!(uav.resolutions, None);
        assert_eq!(uav.fps, None);
        assert_eq!(uav.open_timeout_ms, None);
        assert_eq!(uav.capture_timeout_ms, None);

        // 「缺省即默认开」的判断在 bootstrap：enabled 缺失 → unwrap_or(true)
        let empty: UavConfig = toml::from_str("[camera_uav]\n").unwrap();
        let uav = empty.camera_uav.expect("camera_uav 应解析出来");
        assert_eq!(uav.enabled.unwrap_or(true), true);

        // 两段都缺 → 都是 None（不影响解析）
        let bare: UavConfig = toml::from_str("[camera_pc]\n").unwrap();
        assert!(bare.camera_uav.is_none());
        assert!(bare.camera_pc.is_some());
    }

    #[test]
    fn empty_resolutions_means_no_preference() {
        use crate::device::camera_uav::format::{Choose_Format, Prefer_Format};
        use nokhwa::utils::{CameraFormat, FrameFormat, Resolution};

        let config: UavConfig = toml::from_str("[camera_uav]\nresolutions = []\n").unwrap();
        let uav = config.camera_uav.expect("camera_uav 应解析出来");
        assert_eq!(
            uav.resolutions,
            Some(Vec::new()),
            "显式空列表应读成空 Vec（= 无分辨率偏好）"
        );

        // 空分辨率表 → 直接走 Closest 兜底（不 panic、不取默认分辨率）
        let all = [CameraFormat::new(
            Resolution::new(1280, 720),
            FrameFormat::MJPEG,
            30,
        )];
        let resolutions: Vec<(u32, u32)> = uav
            .resolutions
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|pair| (pair[0], pair[1]))
            .collect();
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Mjpg, &resolutions, 0);
        assert_eq!(chosen, all[0]);
        assert!(reason.contains("Closest 兜底"), "{reason}");

        // 缺省（键缺失）时由 camera_uav 用 DEFAULT_UAV_RESOLUTIONS 兜底
        assert_eq!(DEFAULT_UAV_RESOLUTIONS, [[640, 480], [1280, 720]]);
    }
}
