//Presented by KeJi
//Date : 2026-04-15

//! 节点管理器核心组件
//!
//! 该模块提供并发安全的节点信息管理，使用读写锁（RwLock）保护节点数据，
//! 支持异步上下文中的高并发访问。

use std::collections::HashMap;
use std::sync::Arc;

use libp2p::PeerId;
use tokio::sync::RwLock;

use super::peer_info::{PeerInfo, PeerStatus, PeerCapability};

/// 节点管理器（使用读写锁保护）
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    local_peer_id: PeerId,
}

impl PeerManager {
    /// 创建一个新的节点管理器
    pub fn new(local_peer_id: PeerId) -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            local_peer_id,
        }
    }

    /// 添加或更新节点信息
    pub async fn upsert_peer(&self, peer_info: PeerInfo) {
        let mut peers = self.peers.write().await;
        peers.insert(peer_info.peer_id, peer_info);
    }

    /// 移除节点（自动保护本地节点）
    pub async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        if *peer_id == self.local_peer_id {
            return None;
        }
        let mut peers = self.peers.write().await;
        peers.remove(peer_id)
    }

    /// 获取单个节点信息
    pub async fn get_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
        let peers = self.peers.read().await;
        peers.get(peer_id).cloned()
    }

    /// 获取所有节点信息
    pub async fn get_all_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().cloned().collect()
    }

    /// 更新节点状态（本地节点仅允许 Local/Connected/Busy）
    pub async fn update_status(&self, peer_id: &PeerId, status: PeerStatus) -> bool {
        if *peer_id == self.local_peer_id {
            match status {
                PeerStatus::Local | PeerStatus::Connected | PeerStatus::Busy => {}
                _ => return false,
            }
        }
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_status(status);
            true
        } else {
            false
        }
    }

    /// 更新节点心跳（包含延迟信息）
    pub async fn update_heartbeat(&self, peer_id: &PeerId, latency_ms: Option<u64>) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_heartbeat(latency_ms);
            true
        } else {
            false
        }
    }

    /// 更新节点能力
    pub async fn update_capability(&self, peer_id: &PeerId, capability: Option<PeerCapability>) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_capability(capability);
            true
        } else {
            false
        }
    }

    /// 更新节点带宽信息
    pub async fn update_bandwidth(&self, peer_id: &PeerId, bandwidth_mbps: Option<u64>) -> bool {
        let mut peers = self.peers.write().await;
        if let Some(peer_info) = peers.get_mut(peer_id) {
            peer_info.update_bandwidth(bandwidth_mbps);
            true
        } else {
            false
        }
    }

    /// 获取空闲节点列表（仅含远程 Connected 节点，排除 Local）
    pub async fn get_idle_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values()
            .filter(|p| p.status == PeerStatus::Connected)
            .cloned()
            .collect()
    }

    /// 获取忙碌节点列表（仅含远程 Busy 节点，排除 Local）
    pub async fn get_busy_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values()
            .filter(|p| p.status == PeerStatus::Busy)
            .cloned()
            .collect()
    }

    /// 获取节点数量
    pub async fn count(&self) -> usize {
        let peers = self.peers.read().await;
        peers.len()
    }

    /// 检查是否为空
    pub async fn is_empty(&self) -> bool {
        let peers = self.peers.read().await;
        peers.is_empty()
    }

    /// 清理超时节点（自动保护本地节点）
    pub async fn cleanup_timeout_peers(&self, timeout_secs: u64) -> usize {
        let mut peers = self.peers.write().await;
        let before_count = peers.len();
        let local_id = self.local_peer_id;
        peers.retain(|id, peer_info| *id == local_id || !peer_info.is_timeout(timeout_secs));
        before_count - peers.len()
    }

    /// 获取所有节点ID
    pub async fn get_all_peer_ids(&self) -> Vec<PeerId> {
        let peers = self.peers.read().await;
        peers.keys().cloned().collect()
    }

    /// 检查节点是否存在
    pub async fn contains_peer(&self, peer_id: &PeerId) -> bool {
        let peers = self.peers.read().await;
        peers.contains_key(peer_id)
    }

    /// 清空所有节点（自动保留本地节点）
    pub async fn clear(&self) {
        let mut peers = self.peers.write().await;
        let local_id = self.local_peer_id;
        peers.retain(|id, _| *id == local_id);
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new(PeerId::random())
    }
}