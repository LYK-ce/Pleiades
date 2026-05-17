 //Presented by KeJi
 //Date ： 2026-05-13

 //! 节点管理模块
 //!
 //! 该模块提供 Pleiades 网络中节点资源信息的管理功能。
 //!
 //! ## 核心定义
 //! PeerManager = 相关节点资源目录。存在即在线，存在即相关。
 //!
 //! ## 模块结构
 //! - `peer_info` - 节点信息数据结构定义（PeerInfo, SupportedModel, PeerProfile）
 //! - `peer_manager` - 核心管理组件（读写锁实现）
 //! - `peer_handle` - 对外调用接口（impl Peer_Management_Capability）
 //! - `capability` - Peer_Management_Capability trait 定义

 // 声明子模块
 mod peer_info;
 mod peer_manager;
 mod peer_handle;
 pub mod capability;

 use libp2p::PeerId;

 // 导出 peer_info 模块中的公共类型
 pub use peer_info::{PeerInfo, PeerProfile, SupportedModel};
 // 导出 peer_manager 模块中的公共类型
 pub use peer_manager::PeerManager;
 // 导出 peer_handle 模块中的公共类型
 pub use peer_handle::PeerHandle;
 // 导出 capability 模块中的公共类型
pub use capability::{Peer_Management_Capability, Peer_Management_Error, PeerStatus, PeerCapability, PeerEvent};

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
 pub fn create_peer_management(local_peer_id: PeerId) -> (std::sync::Arc<PeerManager>, Box<dyn Peer_Management_Capability>) {
     let manager = std::sync::Arc::new(PeerManager::new(local_peer_id));
     let handle = PeerHandle::new(manager.clone());
     (manager, Box::new(handle))
 }
