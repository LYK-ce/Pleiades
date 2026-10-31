//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-31

//! UGV（车）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）
//!
//! Task 23 §3.8：设备段由设备端自己负责。本文件除定义车独有字段外，
//! 还负责首次启动时把车端设备段（chassis/lidar/obstacle_inflation_radius）补全到 config.toml。

use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

use pleiades_base::config::{config_file_path, BaseConfig};

/// 顶层键 `plan_budget_ms` 的缺省值（毫秒）：D* 单拍规划时间预算（Task 39 §二）
pub const DEFAULT_PLAN_BUDGET_MS: u64 = 20;

// ─── [camera_ugv] 缺省值（task_43 §三；设备路径与加密配置来自实机验证材料）───

/// 厂商 SDK 库的缺省绝对路径（Jetson 车上的安装位置；可用 `sdk_lib` 覆盖）
pub const DEFAULT_CAMERA_UGV_SDK_LIB: &str =
    "/home/jetson/yahboomcar_ros2_ws/software/library_ws/src/ascamera/libs/lib/aarch64-linux-gnu/libAngstrongCameraSdk.so";
/// 加密配置 json 的缺省绝对路径（`AS_SDK_OpenCamera` 只吃这个）
pub const DEFAULT_CAMERA_UGV_CONFIG_JSON: &str = "/home/jetson/yahboomcar_ros2_ws/software/library_ws/src/ascamera/configurationfiles/hp60c_v2_00_20230704_configEncrypt.json";
/// 请求的流宽（实测 640×480）
pub const DEFAULT_CAMERA_UGV_WIDTH: u32 = 640;
/// 请求的流高
pub const DEFAULT_CAMERA_UGV_HEIGHT: u32 = 480;
/// 请求的流帧率（实测约 20 fps 出帧）
pub const DEFAULT_CAMERA_UGV_STREAM_FPS: u32 = 25;
/// 打开后等首帧的诊断上限（毫秒）；**配 0 = 不等**（逃生开关，bootstrap 立刻返回）
pub const DEFAULT_CAMERA_UGV_TIMEOUT_MS: u64 = 3000;

/// UGV 完整配置 = 共享段（flatten）+ 车独有设备段
#[derive(Debug, Clone, Deserialize)]
pub struct UgvConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 底盘设备配置（STM32 轮式底盘）
    pub chassis: Option<ChassisConfig>,
    /// 雷达设备配置（YDLIDAR，含 SLAM 建图）
    pub lidar: Option<LidarConfig>,
    /// 他车障碍膨胀半径（米，缺省 0.2）
    pub obstacle_inflation_radius: Option<f32>,
    /// 单拍规划时间预算（毫秒，缺省 20；Task 39 §二；**顶层键**，与
    /// `obstacle_inflation_radius` 同级 —— 不放 `[chassis]`）
    pub plan_budget_ms: Option<u64>,
    /// RTK 流动站设备配置（LG290P）
    pub lg290p: Option<Lg290pConfig>,
    /// 车载相机设备配置（HP60C 厂商 SDK；**默认关**：相机独占 USB，与车上 ROS ascamera 互斥）
    pub camera_ugv: Option<CameraUgvConfig>,
}

/// 底盘设备配置（STM32 轮式底盘）
#[derive(Debug, Clone, Deserialize)]
pub struct ChassisConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    pub car_type: Option<String>,
    /// 前进/后退速度档位（缺省 30，Task 22_5 D2：速度由设备层绑定）
    pub forward_speed: Option<i16>,
    /// 原地转向速度档位（缺省 10）
    pub turn_speed: Option<i16>,
}

/// 雷达设备配置（YDLIDAR，含 SLAM 建图）
#[derive(Debug, Clone, Deserialize)]
pub struct LidarConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
}

/// RTK 流动站设备配置（LG290P）
#[derive(Debug, Clone, Deserialize)]
pub struct Lg290pConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
}

/// 车载相机设备配置（HP60C / Angstrong 厂商 SDK，task_43 §三）
///
/// 所有键都可缺省：`enabled = false`（默认关）、`sdk_lib` / `config_json` 缺省取
/// `DEFAULT_CAMERA_UGV_*` 的实机路径，分辨率/帧率缺省 640×480@25。
#[derive(Debug, Clone, Deserialize)]
pub struct CameraUgvConfig {
    /// 是否启用（**缺省 false**：相机独占 USB，与车上 ROS ascamera 节点互斥）
    pub enabled: Option<bool>,
    /// 厂商 SDK 库路径（缺省 `DEFAULT_CAMERA_UGV_SDK_LIB`；运行期 dlopen）
    pub sdk_lib: Option<String>,
    /// 加密配置 json 的绝对路径（缺省 `DEFAULT_CAMERA_UGV_CONFIG_JSON`）
    pub config_json: Option<String>,
    /// 请求的流宽（缺省 640）
    pub width: Option<u32>,
    /// 请求的流高（缺省 480）
    pub height: Option<u32>,
    /// 请求的流帧率（缺省 25）
    pub stream_fps: Option<u32>,
    /// 打开后等首帧的诊断上限（毫秒，缺省 3000）；**配 0 = 不等**（逃生开关）
    pub timeout_ms: Option<u64>,
}

/// 读取 UGV 配置（.config/config.toml）。
///
/// 首次启动时若 config.toml 缺失车端设备段，会先补全默认设备段再读取（幂等，不覆盖已有值）。
pub fn Ensure_Ugv_Config() -> Result<UgvConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    // 配置文件不存在则先由 base 生成共享段（防御性：正常流程 core_bootstrap 已生成）
    if !path.exists() {
        pleiades_base::config::Ensure_Config()?;
    }
    ensure_ugv_section(&path)?;
    let content = fs::read_to_string(&path)?;
    let config: UgvConfig = toml::from_str(&content)?;
    Ok(config)
}

/// 确保 config.toml 存在车端设备段（obstacle_inflation_radius / plan_budget_ms / chassis / lidar /
/// lg290p / camera_ugv），缺失才补默认值。
fn ensure_ugv_section(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut doc: DocumentMut = content.parse()?;
    let mut changed = false;

    if doc.get("obstacle_inflation_radius").is_none() {
        doc["obstacle_inflation_radius"] = toml_edit::value(0.2_f64);
        changed = true;
    }
    // Task 39 §二：规划时间预算（顶层键，缺键补 20 ms；已有值不覆盖）
    if doc.get("plan_budget_ms").is_none() {
        doc["plan_budget_ms"] = toml_edit::value(DEFAULT_PLAN_BUDGET_MS as i64);
        changed = true;
    }
    changed |= fill_chassis(&mut doc);
    changed |= fill_lidar(&mut doc);
    changed |= fill_lg290p(&mut doc);
    changed |= fill_camera_ugv(&mut doc);

    if changed {
        fs::write(path, doc.to_string())?;
    }
    Ok(())
}

/// 补全 [chassis] 段（缺失才补，幂等）
fn fill_chassis(doc: &mut DocumentMut) -> bool {
    if doc.get("chassis").is_none() {
        doc["chassis"] = Item::Table(toml_edit::Table::new());
    }
    let chassis = doc["chassis"].as_table_mut().expect("chassis 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 6] = [
        ("enabled", toml_edit::value(true)),
        ("port", toml_edit::value("/dev/ttyUSB0")),
        ("baudrate", toml_edit::value(115200_i64)),
        ("car_type", toml_edit::value("X3Plus")),
        ("forward_speed", toml_edit::value(30_i64)),
        ("turn_speed", toml_edit::value(10_i64)),
    ];
    for (k, v) in defaults {
        if !chassis.contains_key(k) {
            chassis.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [lidar] 段（缺失才补，幂等）
fn fill_lidar(doc: &mut DocumentMut) -> bool {
    if doc.get("lidar").is_none() {
        doc["lidar"] = Item::Table(toml_edit::Table::new());
    }
    let lidar = doc["lidar"].as_table_mut().expect("lidar 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 3] = [
        ("enabled", toml_edit::value(true)),
        ("port", toml_edit::value("/dev/ttyUSB1")),
        ("baudrate", toml_edit::value(230400_i64)),
    ];
    for (k, v) in defaults {
        if !lidar.contains_key(k) {
            lidar.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [lg290p] 段（缺失才补，幂等）
fn fill_lg290p(doc: &mut DocumentMut) -> bool {
    if doc.get("lg290p").is_none() {
        doc["lg290p"] = Item::Table(toml_edit::Table::new());
    }
    let lg290p = doc["lg290p"].as_table_mut().expect("lg290p 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 3] = [
        ("enabled", toml_edit::value(false)),
        ("port", toml_edit::value("/dev/ttyUSB2")),
        ("baudrate", toml_edit::value(460800_i64)),
    ];
    for (k, v) in defaults {
        if !lg290p.contains_key(k) {
            lg290p.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [camera_ugv] 段（缺失才补，幂等；task_43 §三）
///
/// `enabled` 缺省 **false**（相机独占 USB，与车上 ROS ascamera 节点互斥，按需开）；
/// `timeout_ms` 缺省 3000，配 0 表示"打开后不等首帧"。
fn fill_camera_ugv(doc: &mut DocumentMut) -> bool {
    if doc.get("camera_ugv").is_none() {
        doc["camera_ugv"] = Item::Table(toml_edit::Table::new());
    }
    let camera_ugv = doc["camera_ugv"].as_table_mut().expect("camera_ugv 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 7] = [
        ("enabled", toml_edit::value(false)),
        ("sdk_lib", toml_edit::value(DEFAULT_CAMERA_UGV_SDK_LIB)),
        ("config_json", toml_edit::value(DEFAULT_CAMERA_UGV_CONFIG_JSON)),
        ("width", toml_edit::value(DEFAULT_CAMERA_UGV_WIDTH as i64)),
        ("height", toml_edit::value(DEFAULT_CAMERA_UGV_HEIGHT as i64)),
        ("stream_fps", toml_edit::value(DEFAULT_CAMERA_UGV_STREAM_FPS as i64)),
        ("timeout_ms", toml_edit::value(DEFAULT_CAMERA_UGV_TIMEOUT_MS as i64)),
    ];
    for (k, v) in defaults {
        if !camera_ugv.contains_key(k) {
            camera_ugv.insert(k, v);
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
    fn test_ensure_ugv_section_fills_defaults() {
        let dir = tmp_dir("ugv_cfg_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();

        assert_eq!(parsed.obstacle_inflation_radius, Some(0.2));
        let c = parsed.chassis.expect("chassis 段应被补全");
        assert_eq!(c.enabled, Some(true));
        assert_eq!(c.port.as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(c.baudrate, Some(115200));
        assert_eq!(c.car_type.as_deref(), Some("X3Plus"));
        assert_eq!(c.forward_speed, Some(30));
        assert_eq!(c.turn_speed, Some(10));
        let l = parsed.lidar.expect("lidar 段应被补全");
        assert_eq!(l.enabled, Some(true));
        assert_eq!(l.port.as_deref(), Some("/dev/ttyUSB1"));
        assert_eq!(l.baudrate, Some(230400));
        let g = parsed.lg290p.expect("lg290p 段应被补全");
        assert_eq!(g.enabled, Some(false));
        assert_eq!(g.port.as_deref(), Some("/dev/ttyUSB2"));
        assert_eq!(g.baudrate, Some(460800));
        let cam = parsed.camera_ugv.expect("camera_ugv 段应被补全");
        assert_eq!(cam.enabled, Some(false), "相机默认必须关（独占 USB）");
        assert_eq!(cam.sdk_lib.as_deref(), Some(DEFAULT_CAMERA_UGV_SDK_LIB));
        assert_eq!(cam.config_json.as_deref(), Some(DEFAULT_CAMERA_UGV_CONFIG_JSON));
        assert_eq!((cam.width, cam.height), (Some(640), Some(480)));
        assert_eq!(cam.stream_fps, Some(25));
        assert_eq!(cam.timeout_ms, Some(3000));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_ugv_section_keeps_existing() {
        let dir = tmp_dir("ugv_cfg_keep");
        let path = dir.join("config.toml");
        std::fs::write(&path, "obstacle_inflation_radius = 0.5\n[chassis]\nport = \"/dev/ttyACM0\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.obstacle_inflation_radius, Some(0.5));
        let c = parsed.chassis.unwrap();
        assert_eq!(c.port.as_deref(), Some("/dev/ttyACM0"));
        assert_eq!(c.baudrate, Some(115200));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── camera_ugv（task_43 §三：缺键补全、默认关、幂等、已有值不覆盖）───

    #[test]
    fn test_ensure_ugv_section_fills_camera_ugv() {
        let dir = tmp_dir("ugv_cfg_camera_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[camera_ugv]"), "必须独立成段：{content}");
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        let cam = parsed.camera_ugv.expect("camera_ugv 段应被补全");
        assert_eq!(cam.enabled, Some(false), "enabled 缺省必须 false");
        assert_eq!(cam.timeout_ms, Some(DEFAULT_CAMERA_UGV_TIMEOUT_MS));
        assert_eq!(cam.stream_fps, Some(DEFAULT_CAMERA_UGV_STREAM_FPS));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_camera_ugv_existing_value_kept() {
        let dir = tmp_dir("ugv_cfg_camera_keep");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[camera_ugv]\nenabled = true\nwidth = 1280\ntimeout_ms = 0\n",
        )
        .unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        let cam = parsed.camera_ugv.expect("camera_ugv 段应存在");
        assert_eq!(cam.enabled, Some(true), "不得覆盖用户写的 enabled");
        assert_eq!(cam.width, Some(1280), "不得覆盖用户写的 width");
        assert_eq!(cam.timeout_ms, Some(0), "0 是合法的逃生开关，不得被改成默认");
        // 缺的键仍然补齐
        assert_eq!(cam.height, Some(480));
        assert_eq!(cam.sdk_lib.as_deref(), Some(DEFAULT_CAMERA_UGV_SDK_LIB));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_camera_ugv_idempotent() {
        let dir = tmp_dir("ugv_cfg_camera_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[camera_ugv]\nenabled = false\n").unwrap();

        ensure_ugv_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_ugv_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        let parsed: UgvConfig = toml::from_str(&second).unwrap();
        assert_eq!(parsed.camera_ugv.expect("camera_ugv 段应存在").enabled, Some(false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── plan_budget_ms（Task 39 §二：顶层键，缺键补 20、幂等、已有值不覆盖）───

    #[test]
    fn test_ensure_ugv_section_fills_plan_budget() {
        let dir = tmp_dir("ugv_cfg_plan_budget_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(DEFAULT_PLAN_BUDGET_MS));
        assert_eq!(parsed.plan_budget_ms, Some(20), "缺键补 20 ms");
        // 必须是**顶层键**，不能掉进 [chassis]
        assert!(content.contains("plan_budget_ms = 20"), "{content}");
        assert!(!content.contains("[chassis]\nplan_budget_ms"), "{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plan_budget_existing_value_kept() {
        let dir = tmp_dir("ugv_cfg_plan_budget_keep");
        let path = dir.join("config.toml");
        std::fs::write(&path, "plan_budget_ms = 50\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(50), "不得覆盖用户写的值");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_ugv_section_plan_budget_idempotent() {
        let dir = tmp_dir("ugv_cfg_plan_budget_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "plan_budget_ms = 50\n").unwrap();

        ensure_ugv_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_ugv_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        let parsed: UgvConfig = toml::from_str(&second).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(50));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
