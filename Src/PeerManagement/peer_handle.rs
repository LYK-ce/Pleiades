 //Presented by KeJi
 //Date ： 2026-05-13

 //! 节点管理对外调用接口
 //!
 //! 该模块提供节点管理器的对外调用接口，封装内部实现细节，
 //! 通过实现 `Peer_Management_Capability` trait 提供线程安全、易于使用的API。

 use std::sync::Arc;

 use async_trait::async_trait;
 use libp2p::PeerId;

 use super::peer_info::{PeerInfo, PeerProfile, SupportedModel};
 use super::peer_manager::PeerManager;
 use super::capability::{Peer_Management_Capability, Peer_Management_Error};

 /// 节点管理句柄（impl Peer_Management_Capability）
 ///
 /// Thin wrapper，持有 Arc<PeerManager>。
 /// 支持 Clone，允许多个调用方各自持有 Box<dyn Peer_Management_Capability>
 /// 指向同一 Arc<PeerManager>。
 #[derive(Clone)]
 pub struct PeerHandle {
     inner: Arc<PeerManager>,
 }

 impl PeerHandle {
     /// 创建一个新的节点管理句柄
     pub fn new(manager: Arc<PeerManager>) -> Self {
         Self { inner: manager }
     }

     /// 获取内部管理器引用（用于高级操作）
     pub fn inner(&self) -> &Arc<PeerManager> {
         &self.inner
     }
 }

 #[async_trait]
 impl Peer_Management_Capability for PeerHandle {
     async fn Get_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> {
         Ok(self.inner.get_peers().await)
     }

     async fn Get_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error> {
         self.inner.get_peer(peer_id).await
             .ok_or_else(|| Peer_Management_Error::PeerNotFound(peer_id.to_string()))
     }

     async fn Contains_Peer(&self, peer_id: &PeerId) -> Result<bool, Peer_Management_Error> {
         Ok(self.inner.contains_peer(peer_id).await)
     }

     async fn Count(&self) -> Result<usize, Peer_Management_Error> {
         Ok(self.inner.count().await)
     }

     async fn Is_Empty(&self) -> Result<bool, Peer_Management_Error> {
         Ok(self.inner.is_empty().await)
     }

     async fn Upsert_Peer(&self, peer_info: PeerInfo) {
         self.inner.upsert_peer(peer_info).await;
     }

     async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error> {
         self.inner.remove_peer(peer_id).await
             .ok_or_else(|| Peer_Management_Error::PeerNotFound(peer_id.to_string()))
     }

     async fn Update_Profile(&self, peer_id: &PeerId, profile: PeerProfile) -> Result<(), Peer_Management_Error> {
         if self.inner.update_profile(peer_id, profile).await {
             Ok(())
         } else {
             Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
         }
     }

     async fn Update_Supported_Models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> Result<(), Peer_Management_Error> {
         if self.inner.update_supported_models(peer_id, models).await {
             Ok(())
         } else {
             Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
         }
     }

     async fn Cleanup_Timeout_Peers(&self, timeout_secs: u64) -> Result<usize, Peer_Management_Error> {
         Ok(self.inner.cleanup_timeout_peers(timeout_secs).await)
     }

     async fn Clear(&self) -> Result<(), Peer_Management_Error> {
         self.inner.clear().await;
         Ok(())
     }
 }

 impl std::fmt::Debug for PeerHandle {
     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
         f.debug_struct("PeerHandle")
             .field("inner", &"Arc<PeerManager>")
             .finish_non_exhaustive()
     }
 }
