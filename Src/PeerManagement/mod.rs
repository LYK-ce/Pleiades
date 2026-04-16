//Presented by KeJi
//Date ： 2026-04-15

//! 节点管理模块
//!
//! 该模块提供Pleiades网络中节点信息的管理功能，包括节点状态跟踪、能力发现和并发安全访问。
//!
//! ## 模块结构
//! - `peer_info` - 节点信息数据结构定义
//! - `peer_manager` - 核心管理组件（读写锁实现）
//! - `peer_handle` - 对外调用接口

// 导出peer_info模块中的公共类型
pub use peer_info::{PeerInfo, PeerStatus, PeerCapability};
// 导出peer_manager模块中的公共类型
pub use peer_manager::PeerManager;
// 导出peer_handle模块中的公共类型
pub use peer_handle::{PeerHandle, PeerEvent, PeerError};

// 声明子模块
mod peer_info;
mod peer_manager;
mod peer_handle;

/// 创建节点管理系统的工厂函数
///
/// 该函数返回节点管理器和对应的句柄，用于在系统中集成节点管理功能。
///
/// # 返回值
/// - `(Arc<PeerManager>, PeerHandle)` - 管理器和句柄的元组
pub fn create_peer_management() -> (std::sync::Arc<PeerManager>, PeerHandle) {
    let manager = std::sync::Arc::new(PeerManager::new());
    let handle = PeerHandle::new(manager.clone());
    (manager, handle)
}