//Presented by KeJi
//Created Date ： 2026-04-10
//Modified Date ： 2026-06-15

//! Config模块 - 配置文件解析与节点身份管理
//!
//! 模组等级 Level 0 — 仅依赖 serde、toml、toml_edit 外部 crate 和 std::fs，
//! 不调用任何其他项目模块。
//!
//! 负责读取和解析 config.toml 配置文件，支持自动检测并生成默认配置，
//! 管理节点身份（密钥对持久化）。

pub mod config;
pub mod identity;

pub use config::{
    BaseConfig, Ensure_Config, Get_Node_Type, Get_Peer_Name, Identity_Config, Log_Config,
    Network_Config, NodeType, Read_Config, Set_Peer_Name, Storage_Config, Update_Config,
    CONFIG_DIR, config_file_path, kvcache_dir,
};
pub use identity::Ensure_Identity;
