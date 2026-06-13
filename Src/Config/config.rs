//Presented by KeJi
//Date : 2026-04-09

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use toml_edit::DocumentMut;

/// 编译时嵌入的默认配置文件内容
const DEFAULT_CONFIG: &str = include_str!("config.toml");

/// 默认配置目录名
pub const CONFIG_DIR: &str = ".config";

/// 默认配置文件名
const CONFIG_FILE: &str = "config.toml";

/// KV Cache offload 缓存目录（懒加载，只读一次 config.toml）
static KVCACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 获取 kvcache 目录（首次调用读配置，之后返回缓存）
pub fn kvcache_dir() -> &'static PathBuf {
    KVCACHE_DIR.get_or_init(|| {
        let config_path = Path::new(CONFIG_DIR).join(CONFIG_FILE);
        match Read_Config(&config_path) {
            Ok(config) => {
                config.Storage.as_ref()
                    .and_then(|s| s.kvcache_dir.as_deref())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(".kvcache"))
            }
            Err(_) => PathBuf::from(".kvcache"),
        }
    })
}

/// config.toml 根配置结构体
#[derive(Debug, Deserialize)]
pub struct Pleiades_Config {
    pub Log: Option<Log_Config>,
    pub Network: Option<Network_Config>,
    pub Runtime: Option<Runtime_Config>,
    pub Storage: Option<Storage_Config>,
    pub Session: Option<Session_Config>,
    pub Identity: Option<Identity_Config>,
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
    pub cleanup_interval: Option<u64>,   // 清理间隔（秒），默认300
    pub timeout_interval: Option<u64>,   // 超时间隔（秒），默认300
    pub heartbeat_interval: Option<u64>, // 心跳间隔（秒），默认60
    pub heartbeat_timeout: Option<u64>,  // 心跳超时（秒），默认10
    pub request_response_timeout: Option<u64>, // Request-Response 协议超时（秒），默认300
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

/// [Storage] 段配置
#[derive(Debug, Deserialize)]
pub struct Storage_Config {
    /// 工作目录（所有运行时数据的根目录），默认 "Pleiades_Workspace"
    pub workspace_dir: Option<String>,
    /// 存储配额（单位：GB），0 表示不限制
    pub quota_gb: Option<u64>,
    /// KV Cache offload 缓存目录，默认 ".kvcache"
    pub kvcache_dir: Option<String>,
}

/// [Session] 段配置
#[derive(Debug, Deserialize)]
pub struct Session_Config {
    /// 最大并发对话槽位数，默认 4
    pub max_slots: Option<usize>,
}

/// [Identity] 段配置
#[derive(Debug, Deserialize)]
pub struct Identity_Config {
    /// 节点名称（不含 #XXXX 后缀）
    pub peer_name: Option<String>,
}

/// 读取节点名称，默认 "new_peer"
pub fn Get_Peer_Name(config: &Pleiades_Config) -> String {
    config.Identity.as_ref()
        .and_then(|i| i.peer_name.clone())
        .unwrap_or_else(|| "new_peer".to_string())
}

/// 持久化节点名称到 config.toml
pub fn Set_Peer_Name(config_path: &Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    Update_Config(config_path, "Identity", "peer_name", name)
}

// ============================================================
// Pleiades_Config 便捷方法
// ============================================================

impl Pleiades_Config {
    /// 获取工作目录，默认 "Pleiades_Workspace"
    pub fn workspace_dir(&self) -> PathBuf {
        self.Storage.as_ref()
            .and_then(|s| s.workspace_dir.as_deref())
            .unwrap_or("Pleiades_Workspace")
            .into()
    }

    /// 获取日志目录，默认 <workspace_dir>/Log
    pub fn log_dir(&self, workspace_dir: &Path) -> PathBuf {
        self.Log.as_ref()
            .and_then(|l| l.log_file_path.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| workspace_dir.join("Log"))
    }

    /// 获取日志级别，默认 "info"
    pub fn log_level(&self) -> &str {
        self.Log.as_ref()
            .and_then(|l| l.level.as_deref())
            .unwrap_or("info")
    }
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
        eprintln!("[Info] 已生成默认配置文件: {}/{}", CONFIG_DIR, CONFIG_FILE);
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
/// ```ignore
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
