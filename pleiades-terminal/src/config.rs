//Presented by KeJi
//Created Date ： 2026-08-31
//Modified Date ： 2026-08-31

//! Terminal（地面站）设备配置（Task 24：RTK 基站 UM960）
//!
//! 参照 ugv/uav 的 config.rs：`#[serde(flatten)]` 复用共享段，设备段由设备端自己负责。
//! terminal 是 cdylib（无 main.rs），本文件只负责读 `[um960]` 段（含幂等补全默认值）。

use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

use pleiades_base::config::{config_file_path, BaseConfig};

/// Terminal 完整配置 = 共享段（flatten）+ 地面站独有设备段
#[derive(Debug, Clone, Deserialize)]
pub struct TerminalConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// RTK 基站设备配置（UM960）
    pub um960: Option<Um960Config>,
}

/// RTK 基站设备配置（UM960）
#[derive(Debug, Clone, Deserialize)]
pub struct Um960Config {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    pub survey_seconds: Option<u64>,
}

/// 读取 Terminal 配置（.config/config.toml）。
///
/// 首次启动时若 config.toml 缺失 [um960] 段，会先补全默认段再读取（幂等，不覆盖已有值）。
pub fn Ensure_Terminal_Config() -> Result<TerminalConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    // 配置文件不存在则先由 base 生成共享段（防御性：正常流程 core_bootstrap 已生成）
    if !path.exists() {
        pleiades_base::config::Ensure_Config()?;
    }
    ensure_terminal_section(&path)?;
    let content = fs::read_to_string(&path)?;
    let config: TerminalConfig = toml::from_str(&content)?;
    Ok(config)
}

/// 确保 config.toml 存在 [um960] 段，缺失才补默认值。
fn ensure_terminal_section(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut doc: DocumentMut = content.parse()?;
    let changed = fill_um960(&mut doc);
    if changed {
        fs::write(path, doc.to_string())?;
    }
    Ok(())
}

/// 补全 [um960] 段（缺失才补，幂等）
fn fill_um960(doc: &mut DocumentMut) -> bool {
    if doc.get("um960").is_none() {
        doc["um960"] = Item::Table(toml_edit::Table::new());
    }
    let um960 = doc["um960"].as_table_mut().expect("um960 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 4] = [
        ("enabled", toml_edit::value(false)),
        ("port", toml_edit::value("/dev/ttyUSB0")),
        ("baudrate", toml_edit::value(460800_i64)),
        ("survey_seconds", toml_edit::value(180_i64)),
    ];
    for (k, v) in defaults {
        if !um960.contains_key(k) {
            um960.insert(k, v);
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
    fn test_ensure_terminal_section_fills_defaults() {
        let dir = tmp_dir("term_cfg_fill");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[Identity]\npeer_name = \"t\"\nnode_type = \"ground_station\"\n",
        )
        .unwrap();

        ensure_terminal_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: TerminalConfig = toml::from_str(&content).unwrap();

        let u = parsed.um960.expect("um960 段应被补全");
        assert_eq!(u.enabled, Some(false));
        assert_eq!(u.port.as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(u.baudrate, Some(460800));
        assert_eq!(u.survey_seconds, Some(180));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_terminal_section_keeps_existing() {
        let dir = tmp_dir("term_cfg_keep");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[um960]\nport = \"/dev/ttyACM1\"\n").unwrap();

        ensure_terminal_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: TerminalConfig = toml::from_str(&content).unwrap();
        let u = parsed.um960.unwrap();
        assert_eq!(u.port.as_deref(), Some("/dev/ttyACM1"));
        assert_eq!(u.baudrate, Some(460800)); // 缺失的键补默认

        let _ = std::fs::remove_dir_all(&dir);
    }
}
