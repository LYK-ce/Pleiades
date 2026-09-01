//Presented by KeJi
//Created Date ： 2026-05-13
//Modified Date ： 2026-06-15

//! 节点管理模块
//!
//! 模组等级 Level 0 — 仅依赖 libp2p::PeerId、tokio::sync::RwLock，不调用任何其他项目模块。
//!
//! PeerManager = 集群节点内存目录。HashMap 中有记录即视为在线，超时未活跃则被清理。
//!
//! ## 模块结构
//! - `peer_info` — 数据结构 (PeerInfo, SupportedModel, PeerProfile, SessionSummary)
//! - `peer_manager` — 核心组件 + impl Peer_Management_Capability
//! - `capability` — trait 定义 ("头文件")

mod peer_info;
mod peer_manager;
pub mod capability;

use libp2p::PeerId;

pub use peer_info::{PeerInfo, PeerProfile, SessionSummary, SupportedModel};
pub use peer_manager::PeerManager;
pub use capability::{Peer_Management_Capability, Peer_Management_Error};

/// 创建节点管理系统
///
/// 返回 Arc<PeerManager>（可直接作为 trait object 使用）和
/// Box<dyn Peer_Management_Capability>（供常规消费）。
/// 本地节点自动创建并注册。
pub fn create_peer_management(local_peer_id: PeerId, name: String, node_type: String) -> std::sync::Arc<PeerManager> {
    std::sync::Arc::new(PeerManager::new(local_peer_id, name, node_type))
}
