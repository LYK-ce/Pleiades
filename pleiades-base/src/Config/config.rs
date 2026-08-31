//Presented by KeJi
//Created Date ： 2026-04-09
//Modified Date ： 2026-08-30

#![allow(non_snake_case, non_camel_case_types)]

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use toml_edit::DocumentMut;

/// 默认配置内容（首次启动无 .config/config.toml 时写入，仅共享段）
///
/// 设备段（chassis / lidar / flight_ctrl / obstacle_inflation_radius）由各设备端 crate
/// 的 config.rs 负责；本文件只定义所有终端共享的段（Log / Network / Storage / Identity）。
const DEFAULT_CONFIG: &str = r#"# Pleiades 配置文件

[Log]
# 日志级别: trace, debug, info, warn, error
level = "info"
# 日志文件路径 (留空则输出到控制台)
log_file_path = "Log/"

[Network]
LAN = true
WAN = false
Transport_Protocol = "TCP" # TCP/QUIC
# p2p 监听端口，0 = 随机（种子节点建议固定端口）
listen_port = 0
# DHT 节点发现命名空间（同一集群的所有节点必须一致，区分大小写）
dht_namespace = "pleiades-nodes"
# DHT 种子节点列表（完整 Multiaddr，必须带 /p2p/<PeerId> 后缀）
# 留空 = 不启用 DHT bootstrap（仅靠 mDNS）
bootstrap_peers = []
# 节点管理相关配置
cleanup_interval = 300
timeout_interval = 300
heartbeat_interval = 60
heartbeat_timeout = 10
request_response_timeout = 300

[Storage]
workspace_dir = "Pleiades_Workspace"
quota_gb = 0
kvcache_dir = ".kvcache"

[Identity]
peer_name = "new_peer"
node_type = "car"  # car=车 / uav=机 / ground_station=地面站
"#;

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

/// config.toml 完整路径（.config/config.toml，供设备端 crate 读取自己的设备段）
pub fn config_file_path() -> PathBuf {
    Path::new(CONFIG_DIR).join(CONFIG_FILE)
}

/// config.toml 根配置结构体（共享段；设备段由设备端 crate 经 `#[serde(flatten)]` 组合）
#[derive(Debug, Clone, Deserialize)]
pub struct BaseConfig {
    pub Log: Option<Log_Config>,
    pub Network: Option<Network_Config>,
    pub Storage: Option<Storage_Config>,
    pub Identity: Option<Identity_Config>,
}

/// [Log] 段配置
#[derive(Debug, Clone, Deserialize)]
pub struct Log_Config {
    pub level: Option<String>,
    pub log_file_path: Option<String>,
}

/// [Network] 段配置
#[derive(Debug, Clone, Deserialize)]
pub struct Network_Config {
    pub LAN: Option<bool>,
    pub WAN: Option<bool>,
    pub Transport_Protocol: Option<String>,
    pub cleanup_interval: Option<u64>,
    pub timeout_interval: Option<u64>,
    pub heartbeat_interval: Option<u64>,
    pub heartbeat_timeout: Option<u64>,
    pub request_response_timeout: Option<u64>,
    pub listen_port: Option<u16>,               // p2p 监听端口，0=随机
    pub bootstrap_peers: Option<Vec<String>>,   // DHT 种子节点列表
    pub dht_namespace: Option<String>,          // DHT 节点发现命名空间
}

/// [Storage] 段配置
#[derive(Debug, Clone, Deserialize)]
pub struct Storage_Config {
    pub workspace_dir: Option<String>,
    pub kvcache_dir: Option<String>,
}

/// [Identity] 段配置
#[derive(Debug, Clone, Deserialize)]
pub struct Identity_Config {
    pub peer_name: Option<String>,
    /// 节点类型：car=车 / uav=机 / ground_station=地面站（Task 23 §3.5）
    pub node_type: Option<String>,
}

/// 节点类型（Task 23 §3.5：节点身份，非底盘物理状态）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeType {
    GroundStation = 0,
    Car = 1,
    Uav = 2,
}

impl NodeType {
    pub fn from_str(s: &str) -> Option<NodeType> {
        match s {
            "ground_station" => Some(NodeType::GroundStation),
            "car" => Some(NodeType::Car),
            "uav" => Some(NodeType::Uav),
            _ => None,
        }
    }
}

/// 读取节点类型，默认 Car（车）
pub fn Get_Node_Type(config: &BaseConfig) -> NodeType {
    config.Identity.as_ref()
        .and_then(|i| i.node_type.as_deref())
        .and_then(NodeType::from_str)
        .unwrap_or(NodeType::Car)
}

/// 读取节点名称，默认 "new_peer"
pub fn Get_Peer_Name(config: &BaseConfig) -> String {
    config.Identity.as_ref()
        .and_then(|i| i.peer_name.clone())
        .unwrap_or_else(|| "new_peer".to_string())
}

/// 持久化节点名称到 config.toml
pub fn Set_Peer_Name(config_path: &Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    Update_Config(config_path, "Identity", "peer_name", name)
}

// ============================================================
// BaseConfig 便捷方法
// ============================================================

impl BaseConfig {
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
pub fn Read_Config(config_path: &Path) -> Result<BaseConfig, Box<dyn std::error::Error>> {
    let config_content = fs::read_to_string(config_path)?;
    let config: BaseConfig = toml::from_str(&config_content)?;
    Ok(config)
}

/// 确保配置文件存在并读取配置
pub fn Ensure_Config() -> Result<(BaseConfig, PathBuf), Box<dyn std::error::Error>> {
    let config_dir = Path::new(CONFIG_DIR);
    let config_path = config_dir.join(CONFIG_FILE);

    if !config_path.exists() {
        if !config_dir.exists() {
            fs::create_dir_all(config_dir)?;
            eprintln!("[Info] 已创建配置目录: {}/", CONFIG_DIR);
        }
        fs::write(&config_path, DEFAULT_CONFIG)?;
        eprintln!("[Info] 已生成默认配置文件: {}/{}\n[Info] 设备段（chassis/lidar/flight_ctrl）将由对应设备端 crate 启动时自动补全", CONFIG_DIR, CONFIG_FILE);
    }

    let config = Read_Config(&config_path)?;
    Ok((config, config_path))
}

/// 通用配置修改函数
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
