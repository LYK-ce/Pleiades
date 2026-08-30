//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! SIM（模拟车/机）配置：无硬件，只复用共享段 + 障碍膨胀半径

use serde::Deserialize;

use pleiades_base::config::{config_file_path, BaseConfig};

/// SIM 完整配置 = 共享段（flatten）+ 障碍膨胀半径（寻路动态障碍用）
#[derive(Debug, Clone, Deserialize)]
pub struct SimConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 他车障碍膨胀半径（米，缺省 0.2）
    pub obstacle_inflation_radius: Option<f32>,
}

/// 读取 SIM 配置（.config/config.toml；文件由 base 的 Ensure_Config 在 core_bootstrap 时创建）
pub fn Ensure_Sim_Config() -> Result<SimConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    let content = std::fs::read_to_string(&path)?;
    let config: SimConfig = toml::from_str(&content)?;
    Ok(config)
}
