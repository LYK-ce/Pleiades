// Presented by KeJi
// Date ： 2026-04-23

pub mod core;
pub mod job;
pub mod compiler;
pub mod executor;
pub mod command;
pub mod slot;
pub mod instruction;

use crate::storage::StorageManager;
use crate::llm_io::LLM_IO_Broker;

// 网络能力占位符
pub struct NetworkCapability;

impl NetworkCapability {
    /// 列出所有已连接的节点（占位符）
    pub fn list_peers(&self) -> Vec<String> {
        todo!("实现 list_peers")
    }
}

// 用户界面能力占位符
pub struct UiCapability;

impl UiCapability {
    /// 显示节点列表（占位符）
    pub fn display(&self, _peers: Vec<String>) {
        todo!("实现 display")
    }
}

// 重新导出 executor 中定义的能力 trait
pub use executor::{ComputeCapability, InferenceCapability};

// 统一的能力结构体，供整个 Orchestrator 层使用
pub struct Capabilities {
    pub storage: StorageManager,
    pub compute: Box<dyn ComputeCapability>,
    pub inference: Box<dyn InferenceCapability>,
    pub network: NetworkCapability,
    pub ui: UiCapability,
    pub io_broker: LLM_IO_Broker,
}

impl Capabilities {
    /// 设置计算设备偏好（占位符）
    pub fn set_compute_preference(&self, _device: String) {
        todo!("实现 set_compute_preference")
    }
}