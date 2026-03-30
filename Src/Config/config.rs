//Presented by KeJi
//Date : 2026-03-30

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use serde::Deserialize;
use std::fs;
use std::path::Path;

/// config.toml 根配置结构体
#[derive(Debug, Deserialize)]
pub struct Pleiades_Config {
    pub Log: Option<Log_Config>,
    pub Network: Option<Network_Config>,
    pub Runtime: Option<Runtime_Config>,
}

/// [Log] 段配置
#[derive(Debug, Deserialize)]
pub struct Log_Config {
    pub level: Option<String>,
    pub log_file_path: Option<String>,
}

/// [Network] 段配置
#[derive(Debug, Deserialize)]
pub struct Network_Config {
    pub LAN: Option<bool>,
    pub WAN: Option<bool>,
    pub Transport_Protocol: Option<String>,
}

/// [Runtime] 段配置
#[derive(Debug, Deserialize)]
pub struct Runtime_Config {
    pub device: Option<String>,
    pub model_path: Option<String>,
    pub max_token: Option<u32>,
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
}

/// 读取并解析 config.toml 配置文件
pub fn Read_Config(config_path: &Path) -> Result<Pleiades_Config, Box<dyn std::error::Error>> {
    let config_content = fs::read_to_string(config_path)?;
    let config: Pleiades_Config = toml::from_str(&config_content)?;
    Ok(config)
}
