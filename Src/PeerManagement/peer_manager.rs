//Presented by KeJi
//Created Date : 2026-05-13
//Modified Date ： 2026-06-15

//! 节点管理器核心组件
//!
//! 该模块提供并发安全的节点信息管理，使用读写锁（RwLock）保护节点数据，
//! 支持异步上下文中的高并发访问。
//! 直接实现 `Peer_Management_Capability` trait，无中间 handle 层。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use libp2p::PeerId;
use tokio::sync::RwLock;

use super::capability::{Peer_Management_Capability, Peer_Management_Error};
use super::peer_info::{PeerInfo, PeerProfile, SupportedModel};

/// 节点管理器（使用读写锁保护）
///
/// 本地节点在构造时自动创建并插入 map，通过 `PeerInfo.local` 标识。
pub struct PeerManager {
    peers: RwLock<HashMap<PeerId, PeerInfo>>,
}

impl PeerManager {
    pub fn new(local_peer_id: PeerId, name: String) -> Self {
        let mut peers = HashMap::new();
        peers.insert(local_peer_id, PeerInfo::new_local(local_peer_id, name));
        Self {
            peers: RwLock::new(peers),
        }
    }

    /// 添加或覆盖节点信息，返回 true=更新，false=新增
    pub(crate) async fn upsert_peer(&self, mut peer_info: PeerInfo) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(old) = peers.get(&peer_info.peer_id) {
            peer_info.connected_at = old.connected_at;
            peer_info.last_active = old.last_active;
            peer_info.profile = old.profile.clone();
            peer_info.supported_models = old.supported_models.clone();
            peer_info.local = old.local;
            if peer_info.name.is_empty() {
                peer_info.name = old.name.clone();
            }
        }
        let existed = peers.contains_key(&peer_info.peer_id);
        peers.insert(peer_info.peer_id, peer_info);
        existed
    }

    /// 移除节点（保护本地节点）
    pub(crate) async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        let mut peers = self.peers.write().await;
        if let Some(info) = peers.get(peer_id) {
            if info.local {
                return None;
            }
        }
        peers.remove(peer_id)
    }

    /// 获取所有节点信息
    pub(crate) async fn get_all_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().cloned().collect()
    }

    /// 获取本地节点信息
    pub(crate) async fn get_local_peer(&self) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().find(|p| p.local).cloned()
    }

    /// 按显示名称精确匹配节点
    pub(crate) async fn get_peer_by_name(&self, name: &str) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().find(|p| p.display_name() == name).cloned()
    }

    /// 设置本地节点名称
    pub(crate) async fn set_local_name(&self, name: &str) -> Result<(), Peer_Management_Error> {
        let mut peers = self.peers.write().await;
        if let Some(info) = peers.values_mut().find(|p| p.local) {
            info.name = name.to_string();
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound("local".to_string()))
        }
    }

    /// 更新节点性能画像
    pub(crate) async fn update_profile(&self, peer_id: &PeerId, profile: PeerProfile) -> Result<(), Peer_Management_Error> {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_profile(profile);
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
        }
    }

    /// 更新节点持有的模型列表
    pub(crate) async fn update_supported_models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> Result<(), Peer_Management_Error> {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_supported_models(models);
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
        }
    }

    /// 更新节点的 sessions 列表
    pub(crate) async fn update_sessions(&self, peer_id: &PeerId, sessions: Vec<crate::peer_management::SessionSummary>) -> Result<(), Peer_Management_Error> {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_sessions(sessions);
            Ok(())
        } else {
            Err(Peer_Management_Error::PeerNotFound(peer_id.to_string()))
        }
    }

    /// 清空所有节点（保留本地节点）
    pub(crate) async fn clear(&self) {
        let mut peers = self.peers.write().await;
        peers.retain(|_, info| info.local);
    }
}

#[async_trait]
impl Peer_Management_Capability for PeerManager {
    async fn Get_All_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> {
        Ok(self.get_all_peers().await)
    }
    async fn Get_Local_Peer(&self) -> Result<PeerInfo, Peer_Management_Error> {
        self.get_local_peer().await
            .ok_or_else(|| Peer_Management_Error::PeerNotFound("local".to_string()))
    }
    async fn Get_Peer_By_Name(&self, name: &str) -> Result<PeerInfo, Peer_Management_Error> {
        self.get_peer_by_name(name).await
            .ok_or_else(|| Peer_Management_Error::PeerNotFound(name.to_string()))
    }
    async fn Upsert_Peer(&self, peer_info: PeerInfo) -> Result<bool, Peer_Management_Error> {
        Ok(self.upsert_peer(peer_info).await)
    }
    async fn Set_Local_Name(&self, name: &str) -> Result<(), Peer_Management_Error> {
        self.set_local_name(name).await
    }
    async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error> {
        self.remove_peer(peer_id).await
            .ok_or_else(|| Peer_Management_Error::PeerNotFound(peer_id.to_string()))
    }
    async fn Update_Profile(&self, peer_id: &PeerId, profile: PeerProfile) -> Result<(), Peer_Management_Error> {
        self.update_profile(peer_id, profile).await
    }
    async fn Update_Supported_Models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> Result<(), Peer_Management_Error> {
        self.update_supported_models(peer_id, models).await
    }
    async fn Update_Sessions(&self, peer_id: &PeerId, sessions: Vec<crate::peer_management::SessionSummary>) -> Result<(), Peer_Management_Error> {
        self.update_sessions(peer_id, sessions).await
    }
    async fn Clear(&self) -> Result<(), Peer_Management_Error> {
        self.clear().await;
        Ok(())
    }
}

#[async_trait]
impl Peer_Management_Capability for Arc<PeerManager> {
    async fn Get_All_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error> {
        self.as_ref().Get_All_Peers().await
    }
    async fn Get_Local_Peer(&self) -> Result<PeerInfo, Peer_Management_Error> {
        self.as_ref().Get_Local_Peer().await
    }
    async fn Get_Peer_By_Name(&self, name: &str) -> Result<PeerInfo, Peer_Management_Error> {
        self.as_ref().Get_Peer_By_Name(name).await
    }
    async fn Upsert_Peer(&self, peer_info: PeerInfo) -> Result<bool, Peer_Management_Error> {
        self.as_ref().Upsert_Peer(peer_info).await
    }
    async fn Set_Local_Name(&self, name: &str) -> Result<(), Peer_Management_Error> {
        self.as_ref().Set_Local_Name(name).await
    }
    async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error> {
        self.as_ref().Remove_Peer(peer_id).await
    }
    async fn Update_Profile(&self, peer_id: &PeerId, profile: PeerProfile) -> Result<(), Peer_Management_Error> {
        self.as_ref().Update_Profile(peer_id, profile).await
    }
    async fn Update_Supported_Models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> Result<(), Peer_Management_Error> {
        self.as_ref().Update_Supported_Models(peer_id, models).await
    }
    async fn Update_Sessions(&self, peer_id: &PeerId, sessions: Vec<crate::peer_management::SessionSummary>) -> Result<(), Peer_Management_Error> {
        self.as_ref().Update_Sessions(peer_id, sessions).await
    }
    async fn Clear(&self) -> Result<(), Peer_Management_Error> {
        self.as_ref().Clear().await
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new(PeerId::random(), String::new())
    }
}
