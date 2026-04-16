//Presented by KeJi
//Date : 2026-04-09

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

/// 编译时嵌入的默认配置文件内容
const DEFAULT_CONFIG: &str = include_str!("config.toml");

/// 默认配置目录名
const CONFIG_DIR: &str = ".config";

/// 默认配置文件名
const CONFIG_FILE: &str = "config.toml";

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
    pub cleanup_interval: Option<u64>,    // 清理间隔（秒），默认300
    pub timeout_interval: Option<u64>,    // 超时间隔（秒），默认300
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

/// 确保配置文件存在并读取配置
///
/// 检查当前目录下 .config/config.toml 是否存在：
/// - 若不存在，则创建 .config/ 目录并写入默认配置
/// - 若已存在，则直接读取
///
/// 返回解析后的配置和配置文件路径
pub fn Ensure_Config() -> Result<(Pleiades_Config, PathBuf), Box<dyn std::error::Error>> {
    let config_dir = Path::new(CONFIG_DIR);
    let config_path = config_dir.join(CONFIG_FILE);

    if !config_path.exists() {
        // 创建 .config/ 目录（若不存在）
        if !config_dir.exists() {
            fs::create_dir_all(config_dir)?;
            eprintln!("[Info] 已创建配置目录: {}/", CONFIG_DIR);
        }

        // 写入默认配置文件
        fs::write(&config_path, DEFAULT_CONFIG)?;
        eprintln!(
            "[Info] 已生成默认配置文件: {}/{}",
            CONFIG_DIR, CONFIG_FILE
        );
    }

    let config = Read_Config(&config_path)?;
    Ok((config, config_path))
}

/// 通用配置修改函数
///
/// 修改 config.toml 中指定 section 下 key 的值（字符串类型）。
/// 使用 `toml_edit` 解析为可编辑 AST，修改后保留注释和格式写回。
///
/// # 参数
/// - `config_path`: 配置文件路径
/// - `section`: TOML 段名（如 "Runtime", "Network", "Log"）
/// - `key`: 字段名（如 "device", "level"）
/// - `value`: 新值（字符串）
///
/// # 示例
/// ```rust
/// Update_Config(path, "Runtime", "device", "cuda");
/// Update_Config(path, "Network", "Transport_Protocol", "QUIC");
/// Update_Config(path, "Log", "level", "debug");
/// ```
pub fn Update_Config(
    config_path: &Path,
    section: &str,
    key: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(config_path)?;
    let mut doc = content.parse::<DocumentMut>()?;
    doc[section][key] = toml_edit::value(value);
    fs::write(config_path, doc.to_string())?;
    Ok(())
}
