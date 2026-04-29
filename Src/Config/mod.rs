//Presented by KeJi
//Date ： 2026-04-10

//! Config模块 - 配置文件解析与节点身份管理
//!
//! 负责读取和解析 config.toml 配置文件
//! 支持自动检测并生成默认配置
//! 管理节点身份（密钥对持久化）

pub mod config;
pub mod identity;

pub use config::{Pleiades_Config, Log_Config, Network_Config, Runtime_Config, Storage_Config, Read_Config, Ensure_Config, Update_Config};
pub use identity::Ensure_Identity;
