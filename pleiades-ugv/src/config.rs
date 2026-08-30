//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! UGV（车）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）

use serde::Deserialize;

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

/// 读取 UGV 配置（.config/config.toml；文件由 base 的 Ensure_Config 在 core_bootstrap 时创建）
pub fn Ensure_Ugv_Config() -> Result<UgvConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    let content = std::fs::read_to_string(&path)?;
    let config: UgvConfig = toml::from_str(&content)?;
    Ok(config)
}
