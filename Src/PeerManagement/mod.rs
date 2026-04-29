//Presented by KeJi
//Date ： 2026-04-28

//! 节点管理模块
//!
//! 该模块提供Pleiades网络中节点信息的管理功能，包括节点状态跟踪、能力发现和并发安全访问。
//!
//! ## 模块结构
//! - `peer_info` - 节点信息数据结构定义
//! - `peer_manager` - 核心管理组件（读写锁实现）
//! - `peer_handle` - 对外调用接口（impl Peer_Management_Capability）
//! - `capability` - Peer_Management_Capability trait 定义

// 声明子模块
mod peer_info;
mod peer_manager;
mod peer_handle;
pub mod capability;

// 导出 peer_info 模块中的公共类型
pub use peer_info::{PeerInfo, PeerStatus, PeerCapability};
// 导出 peer_manager 模块中的公共类型
pub use peer_manager::PeerManager;
// 导出 peer_handle 模块中的公共类型
pub use peer_handle::{PeerHandle, PeerEvent};
// 导出 capability 模块中的公共类型
pub use capability::{Peer_Management_Capability, Peer_Management_Error};

/// 创建节点管理系统的工厂函数
///
/// 该函数返回节点管理器和对应的 Capability trait object，
/// 用于在系统中集成节点管理功能。
///
/// # 返回值
/// - `(Arc<PeerManager>, Box<dyn Peer_Management_Capability>)` - 管理器和 Capability 的元组
pub fn create_peer_management() -> (std::sync::Arc<PeerManager>, Box<dyn Peer_Management_Capability>) {
    let manager = std::sync::Arc::new(PeerManager::new());
    let handle = PeerHandle::new(manager.clone());
    (manager, Box::new(handle))
}
