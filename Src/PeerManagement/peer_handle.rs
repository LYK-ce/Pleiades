//Presented by KeJi
//Date ： 2026-04-28

//! 节点管理对外调用接口
//!
//! 该模块提供节点管理器的对外调用接口，封装内部实现细节，
//! 通过实现 `Peer_Management_Capability` trait 提供线程安全、易于使用的API。

use std::sync::Arc;

use async_trait::async_trait;
use libp2p::PeerId;

use super::peer_info::{PeerInfo, PeerStatus, PeerCapability};
use super::peer_manager::PeerManager;
use super::capability::{Peer_Management_Capability, Peer_Management_Error};

/// 节点管理句柄（impl Peer_Management_Capability）
#[derive(Clone)]
pub struct PeerHandle {
    inner: Arc<PeerManager>, // 内部管理器引用
}

/// 节点事件（用于通知，与 Capability 正交）
#[derive(Debug, Clone)]
pub enum PeerEvent {
    /// 节点添加事件
    PeerAdded(PeerInfo),
    /// 节点移除事件
    PeerRemoved(PeerId),
    /// 节点状态变化事件
    PeerStatusChanged(PeerId, PeerStatus),
    /// 节点延迟更新事件
    PeerLatencyUpdated(PeerId, u64),
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
    async fn List_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> {
        Ok(self.inner.get_all_peers().await)
    }

    async fn Get_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error> {
        self.inner.get_peer(peer_id).await
            .ok_or_else(|| Peer_Management_Error::PeerNotFound(peer_id.to_string()))
    }

    async fn Contains_Peer(&self, peer_id: &PeerId) -> Result<bool, Peer_Management_Error> {
        Ok(self.inner.contains_peer(peer_id).await)
    }

    async fn Get_Idle_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> {
        Ok(self.inner.get_idle_peers().await)
    }

    async fn Count(&self) -> Result<usize, Peer_Management_Error> {
        Ok(self.inner.count().await)
    }

    async fn Get_All_Peer_Ids(&self) -> Result<Vec<PeerId>, Peer_Management_Error> {
        Ok(self.inner.get_all_peer_ids().await)
    }

    async fn Add_Peer(&self, peer_info: PeerInfo) -> Result<(), Peer_Management_Error> {
        self.inner.upsert_peer(peer_info).await;
        Ok(())
    }

    async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error> {
        self.inner.remove_peer(peer_id).await
            .ok_or_else(|| Peer_Management_Error::PeerNotFound(peer_id.to_string()))
    }

    async fn Update_Status(&self, peer_id: &PeerId, status: PeerStatus) -> Result<(), Peer_Management_Error> {
        if self.inner.update_status(peer_id, status).await {
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
        }
    }

    async fn Update_Heartbeat(&self, peer_id: &PeerId, latency_ms: Option<u64>) -> Result<(), Peer_Management_Error> {
        if self.inner.update_heartbeat(peer_id, latency_ms).await {
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
        }
    }

    async fn Update_Capability(&self, peer_id: &PeerId, capability: Option<PeerCapability>) -> Result<(), Peer_Management_Error> {
        if self.inner.update_capability(peer_id, capability).await {
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
        }
    }

    async fn Update_Bandwidth(&self, peer_id: &PeerId, bandwidth_mbps: Option<u64>) -> Result<(), Peer_Management_Error> {
        if self.inner.update_bandwidth(peer_id, bandwidth_mbps).await {
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
