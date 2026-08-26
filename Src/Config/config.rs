//Presented by KeJi
//Created Date : 2026-04-09
//Modified Date ： 2026-08-26

#![allow(non_snake_case, non_camel_case_types)]

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use toml_edit::DocumentMut;

/// 默认配置内容（首次启动无 .config/config.toml 时写入）
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

[Robot]
obstacle_inflation_radius = 0.2 # 他车障碍膨胀半径（米，20cm）

[Robot.chassis]
enabled = true # 底盘开关（车默认开）
port = "/dev/myserial"
baudrate = 115200
car_type = "X3Plus" # X3 / X3Plus / X1 / R2
forward_speed = 30 # 前进/后退档位（缺省 30）
turn_speed = 10    # 原地转向档位（缺省 10）

[Robot.lidar]
enabled = true # 雷达开关（含 SLAM 建图）
port = "/dev/rplidar"
baudrate = 230400

[Robot.flight_ctrl]
enabled = false # 飞控开关（机，未来）
# connection = "/dev/ttyS0" # 飞控连接方式（MAVLink 串口/UDP，未来 task）
# baudrate = 921600 # 飞控串口波特率
vel_fwd = 0.3      # 前进/后退速度 m/s（缺省 0.3）
yaw_rate_deg = 15  # 转向角速度 °/s（缺省 15）
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

/// config.toml 根配置结构体
#[derive(Debug, Deserialize)]
pub struct Pleiades_Config {
    pub Log: Option<Log_Config>,
    pub Network: Option<Network_Config>,
    pub Storage: Option<Storage_Config>,
    pub Identity: Option<Identity_Config>,
    pub Robot: Option<Robot_Config>,
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
#[derive(Debug, Deserialize)]
pub struct Storage_Config {
    pub workspace_dir: Option<String>,
    pub kvcache_dir: Option<String>,
}

/// [Identity] 段配置
#[derive(Debug, Deserialize)]
pub struct Identity_Config {
    pub peer_name: Option<String>,
}

/// [Robot] 段配置（Task 22：设备开关 + 遍历 spawn）
#[derive(Debug, Deserialize)]
pub struct Robot_Config {
    pub chassis: Option<ChassisConfig>,
    pub lidar: Option<LidarConfig>,
    pub flight_ctrl: Option<FlightCtrlConfig>,
    pub obstacle_inflation_radius: Option<f32>,
}

/// 底盘设备配置（STM32 轮式底盘）
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
pub struct LidarConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
}

/// 飞控设备配置（Pixhawk，未来）
#[derive(Debug, Deserialize)]
pub struct FlightCtrlConfig {
    pub enabled: Option<bool>,
    /// 连接方式（如串口 /dev/ttyS0 或 MAVLink UDP 地址），未来 task 用
    pub connection: Option<String>,
    /// 串口波特率（默认 921600）
    pub baudrate: Option<u32>,
    /// 前进/后退速度 (m/s，缺省 0.3，Task 22_5 D2：速度由设备层绑定)
    pub vel_fwd: Option<f32>,
    /// 转向角速度 (°/s，缺省 15)
    pub yaw_rate_deg: Option<f32>,
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
pub fn Ensure_Config() -> Result<(Pleiades_Config, PathBuf), Box<dyn std::error::Error>> {
    let config_dir = Path::new(CONFIG_DIR);
    let config_path = config_dir.join(CONFIG_FILE);

    if !config_path.exists() {
        if !config_dir.exists() {
            fs::create_dir_all(config_dir)?;
            eprintln!("[Info] 已创建配置目录: {}/", CONFIG_DIR);
        }
        fs::write(&config_path, DEFAULT_CONFIG)?;
        eprintln!("[Info] 已生成默认配置文件: {}/{}", CONFIG_DIR, CONFIG_FILE);
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
