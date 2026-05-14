 //Presented by KeJi
 //Date ： 2026-05-13

 //! PeerManagement Capability 层
 //!
 //! 定义节点管理对外暴露的唯一 trait `Peer_Management_Capability`，
 //! 以及相关的错误类型 `Peer_Management_Error`。
 //!
 //! Orchestrator 和 Network 层通过此 trait 操作节点信息。
 //! 实现方为 PeerHandle（持有 Arc<PeerManager>）。

 use async_trait::async_trait;
 use libp2p::PeerId;
 use super::peer_info::{PeerInfo, PeerProfile, SupportedModel};

 /// 节点管理错误
 #[derive(Debug, thiserror::Error)]
 pub enum Peer_Management_Error {
     #[error("Peer not found: {0}")]
     PeerNotFound(String),
     #[error("Operation timeout")]
     Timeout,
     #[error("Internal error: {0}")]
     Internal(String),
 }

 /// 节点管理能力 trait
 ///
 /// Orchestrator 和 Network 层通过此 trait 操作节点信息。
 /// 实现方为 PeerHandle（持有 Arc<PeerManager>）。
 #[async_trait]
 pub trait Peer_Management_Capability: Send + Sync {
     // ─── 查询操作 ──────────────────────────────

     /// 获取远程节点列表（排除本地）
     async fn Get_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error>;

     /// 获取单个节点信息
     async fn Get_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;

     /// 检查节点是否存在
     async fn Contains_Peer(&self, peer_id: &PeerId) -> Result<bool, Peer_Management_Error>;

     /// 获取节点数量
     async fn Count(&self) -> Result<usize, Peer_Management_Error>;

     /// 检查节点列表是否为空
     async fn Is_Empty(&self) -> Result<bool, Peer_Management_Error>;

     // ─── 变更操作 ──────────────────────────────

     /// 添加或覆盖节点信息
     async fn Upsert_Peer(&self, peer_info: PeerInfo);

     /// 移除节点（保护本地节点）
     async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;

     /// 更新节点性能画像，字段为 None 时跳过
     async fn Update_Profile(&self, peer_id: &PeerId, profile: PeerProfile) -> Result<(), Peer_Management_Error>;

     /// 更新节点持有的模型列表
     async fn Update_Supported_Models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> Result<(), Peer_Management_Error>;

     /// 清理超时节点，返回清理数量
     async fn Cleanup_Timeout_Peers(&self, timeout_secs: u64) -> Result<usize, Peer_Management_Error>;

     /// 清空所有节点（保留本地节点）
     async fn Clear(&self) -> Result<(), Peer_Management_Error>;
 }
