//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! UAV（机）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）

use serde::Deserialize;

use pleiades_base::config::{config_file_path, BaseConfig};

/// UAV 完整配置 = 共享段（flatten）+ 机独有设备段
#[derive(Debug, Clone, Deserialize)]
pub struct UavConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 飞控设备配置（MAVLink）
    pub flight_ctrl: Option<FlightCtrlConfig>,
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

/// 读取 UAV 配置（.config/config.toml；文件由 base 的 Ensure_Config 在 core_bootstrap 时创建）
pub fn Ensure_Uav_Config() -> Result<UavConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    let content = std::fs::read_to_string(&path)?;
    let config: UavConfig = toml::from_str(&content)?;
    Ok(config)
}
