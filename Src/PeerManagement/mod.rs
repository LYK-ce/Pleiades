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
//! - `peer_manager` — 核心组件 (RwLock<HashMap<PeerId, PeerInfo>>)
//! - `peer_handle` — impl Peer_Management_Capability (薄委托)
//! - `capability` — trait 定义 ("头文件")

 // 声明子模块
 mod peer_info;
 mod peer_manager;
 mod peer_handle;
 pub mod capability;

 use libp2p::PeerId;

 // 导出 peer_info 模块中的公共类型
 pub use peer_info::{PeerInfo, PeerProfile, SessionSummary, SupportedModel};
 // 导出 peer_manager 模块中的公共类型
 pub use peer_manager::PeerManager;
 // 导出 peer_handle 模块中的公共类型
 pub use peer_handle::PeerHandle;
 // 导出 capability 模块中的公共类型
pub use capability::{Peer_Management_Capability, Peer_Management_Error};

 /// 创建节点管理系统的工厂函数
 ///
 /// 该函数返回节点管理器和对应的 Capability trait object，
 /// 用于在系统中集成节点管理功能。
 ///
 /// 本地节点在构造时自动创建并插入 map，无需手动注册。
 ///
 /// # 参数
 /// - `local_peer_id` — 本地节点 ID，由调用方从 Network keypair 生成
 ///
 /// # 返回值
 /// - `(Arc<PeerManager>, Box<dyn Peer_Management_Capability>)` — 管理器和 Capability 的元组
 pub fn create_peer_management(local_peer_id: PeerId, name: String) -> (std::sync::Arc<PeerManager>, Box<dyn Peer_Management_Capability>) {
     let manager = std::sync::Arc::new(PeerManager::new(local_peer_id, name));
     let handle = PeerHandle::new(manager.clone());
     (manager, Box::new(handle))
 }
