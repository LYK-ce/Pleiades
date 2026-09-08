//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-09-08

//! UAV（机）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）
//!
//! Task 23 §3.8：设备段由设备端自己负责。本文件除定义机独有字段外，
//! 还负责首次启动时把机端设备段（flight_ctrl/obstacle_inflation_radius）补全到 config.toml。

use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

use pleiades_base::config::{config_file_path, BaseConfig};

/// UAV 完整配置 = 共享段（flatten）+ 机独有设备段
#[derive(Debug, Clone, Deserialize)]
pub struct UavConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 飞控设备配置（MAVLink）
    pub flight_ctrl: Option<FlightCtrlConfig>,
    /// 他车障碍膨胀半径（米，缺省 0.2；「天上小车」2D 寻路阶段使用）
    pub obstacle_inflation_radius: Option<f32>,
    /// 摄像头设备配置（USB UVC，Task 28/29）
    pub camera: Option<CameraConfig>,
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

/// 摄像头设备配置（USB UVC）
#[derive(Debug, Clone, Deserialize)]
pub struct CameraConfig {
    pub enabled: Option<bool>,
    /// 设备路径：纯数字 = 索引（Windows 常用 "0"），否则 = 路径（Linux 常用 "/dev/video0"）
    pub path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub timeout_ms: Option<u64>,
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

/// 确保 config.toml 存在机端设备段（obstacle_inflation_radius / flight_ctrl），缺失才补默认值。
fn ensure_uav_section(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut doc: DocumentMut = content.parse()?;
    let mut changed = false;

    if doc.get("obstacle_inflation_radius").is_none() {
        doc["obstacle_inflation_radius"] = toml_edit::value(0.2_f64);
        changed = true;
    }
    changed |= fill_flight_ctrl(&mut doc);
    changed |= fill_camera(&mut doc);

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

/// 补全 [camera] 段（缺失才补，幂等）
fn fill_camera(doc: &mut DocumentMut) -> bool {
    if doc.get("camera").is_none() {
        doc["camera"] = Item::Table(toml_edit::Table::new());
    }
    let cam = doc["camera"].as_table_mut().expect("camera 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 5] = [
        ("enabled", toml_edit::value(false)),
        ("path", toml_edit::value("0")),
        ("width", toml_edit::value(1280_i64)),
        ("height", toml_edit::value(720_i64)),
        ("timeout_ms", toml_edit::value(3000_i64)),
    ];
    for (k, v) in defaults {
        if !cam.contains_key(k) {
            cam.insert(k, v);
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
    fn test_ensure_uav_section_fills_defaults() {
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
        let cam = parsed.camera.expect("camera 段应被补全");
        assert_eq!(cam.enabled, Some(false));
        assert_eq!(cam.path.as_deref(), Some("0"));
        assert_eq!(cam.width, Some(1280));
        assert_eq!(cam.height, Some(720));
        assert_eq!(cam.timeout_ms, Some(3000));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_uav_section_keeps_existing() {
        let dir = tmp_dir("uav_cfg_keep");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[flight_ctrl]\nenabled = true\nbaudrate = 115200\n").unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let f = parsed.flight_ctrl.unwrap();
        assert_eq!(f.enabled, Some(true));
        assert_eq!(f.baudrate, Some(115200));
        assert_eq!(f.vel_fwd, Some(0.3));
        assert_eq!(f.yaw_rate_deg, Some(15.0));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
