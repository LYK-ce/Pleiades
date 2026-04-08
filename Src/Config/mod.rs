//Presented by KeJi
//Date : 2026-04-08

//! Config模块 - 配置文件解析
//!
//! 负责读取和解析 config.toml 配置文件
//! 支持自动检测并生成默认配置

pub mod config;

pub use config::{Pleiades_Config, Log_Config, Network_Config, Runtime_Config, Read_Config, Ensure_Config};
