//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-31

//! UGV（车）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）
//!
//! Task 23 §3.8：设备段由设备端自己负责。本文件除定义车独有字段外，
//! 还负责首次启动时把车端设备段（chassis/lidar/obstacle_inflation_radius）补全到 config.toml。

use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

use pleiades_base::config::{config_file_path, BaseConfig};

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

/// 确保 config.toml 存在车端设备段（obstacle_inflation_radius / chassis / lidar），缺失才补默认值。
fn ensure_ugv_section(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut doc: DocumentMut = content.parse()?;
    let mut changed = false;

    if doc.get("obstacle_inflation_radius").is_none() {
        doc["obstacle_inflation_radius"] = toml_edit::value(0.2_f64);
        changed = true;
    }
    changed |= fill_chassis(&mut doc);
    changed |= fill_lidar(&mut doc);

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
}
