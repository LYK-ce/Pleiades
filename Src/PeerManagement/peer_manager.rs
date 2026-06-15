//Presented by KeJi
//Created Date : 2026-05-13
//Modified Date ： 2026-06-15

//! 节点管理器核心组件
//!
//! 该模块提供并发安全的节点信息管理，使用读写锁（RwLock）保护节点数据，
//! 支持异步上下文中的高并发访问。

use std::collections::HashMap;

use libp2p::PeerId;
use tokio::sync::RwLock;

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

    /// 添加或覆盖节点信息
    /// 若节点已存在，保留已有字段：connected_at / last_active / profile / supported_models / local，
    /// name 仅在传入为空时保留旧值。
    /// 返回 true=更新已有节点，false=新增节点。
    pub async fn upsert_peer(&self, mut peer_info: PeerInfo) -> bool {
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
    pub async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        let mut peers = self.peers.write().await;
        if let Some(info) = peers.get(peer_id) {
            if info.local {
                return None;
            }
        }
        peers.remove(peer_id)
    }

    /// 获取所有节点信息
    pub async fn get_all_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().cloned().collect()
    }

    /// 获取本地节点信息
    pub async fn get_local_peer(&self) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().find(|p| p.local).cloned()
    }

    /// 按显示名称精确匹配节点（匹配 display_name，即 name#XXXX）
    pub async fn get_peer_by_name(&self, name: &str) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().find(|p| p.display_name() == name).cloned()
    }

    /// 设置本地节点名称，返回是否成功
    pub async fn set_local_name(&self, name: String) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(info) = peers.values_mut().find(|p| p.local) {
            info.name = name;
            true
        } else {
            false
        }
    }

    /// 更新节点性能画像
    pub async fn update_profile(&self, peer_id: &PeerId, profile: PeerProfile) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_profile(profile);
            true
        } else {
            false
        }
    }

    /// 更新节点持有的模型列表
    pub async fn update_supported_models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_supported_models(models);
            true
        } else {
            false
        }
    }

    /// 更新节点的 sessions 列表
    pub async fn update_sessions(&self, peer_id: &PeerId, sessions: Vec<crate::peer_management::SessionSummary>) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_sessions(sessions);
            true
        } else {
            false
        }
    }

    /// 清空所有节点（保留本地节点）
    pub async fn clear(&self) {
        let mut peers = self.peers.write().await;
        peers.retain(|_, info| info.local);
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new(PeerId::random(), String::new())
    }
}
