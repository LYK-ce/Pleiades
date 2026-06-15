//Presented by KeJi
//Created Date ： 2026-05-13
//Modified Date ： 2026-06-15

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
    pub fn new(manager: Arc<PeerManager>) -> Self {
        Self { inner: manager }
    }
}

#[async_trait]
impl Peer_Management_Capability for PeerHandle {
    async fn Get_All_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> {
        Ok(self.inner.get_all_peers().await)
    }

    async fn Get_Local_Peer(&self) -> Result<PeerInfo, Peer_Management_Error> {
        self.inner.get_local_peer().await
            .ok_or_else(|| Peer_Management_Error::PeerNotFound("local".to_string()))
    }

    async fn Get_Peer_By_Name(&self, name: &str) -> Result<PeerInfo, Peer_Management_Error> {
        self.inner.get_peer_by_name(name).await
            .ok_or_else(|| Peer_Management_Error::PeerNotFound(name.to_string()))
    }

    async fn Upsert_Peer(&self, peer_info: PeerInfo) -> Result<bool, Peer_Management_Error> {
        Ok(self.inner.upsert_peer(peer_info).await)
    }

    async fn Set_Local_Name(&self, name: &str) -> Result<(), Peer_Management_Error> {
        if self.inner.set_local_name(name.to_string()).await {
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound("local".to_string()))
        }
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

    async fn Update_Sessions(&self, peer_id: &PeerId, sessions: Vec<crate::peer_management::SessionSummary>) -> Result<(), Peer_Management_Error> {
        if self.inner.update_sessions(peer_id, sessions).await {
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
        }
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
