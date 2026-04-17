//Presented by KeJi
//Date : 2026-04-15

//! 节点管理器核心组件
//!
//! 该模块提供并发安全的节点信息管理，使用读写锁（RwLock）保护节点数据，
//! 支持异步上下文中的高并发访问。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use libp2p::PeerId;
use tokio::sync::RwLock;
use tokio::time;

use super::peer_info::{PeerInfo, PeerStatus, PeerCapability};

/// 节点管理器（使用读写锁保护）
pub struct PeerManager {
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>, // 核心存储
}

impl PeerManager {
    /// 创建一个新的节点管理器
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 添加或更新节点信息
    pub async fn upsert_peer(&self, peer_info: PeerInfo) {
        let mut peers = self.peers.write().await;
        peers.insert(peer_info.peer_id, peer_info);
    }

    /// 移除节点
    pub async fn remove_peer(&self, peer_id: &PeerId) -> Option<PeerInfo> {
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

    /// 更新节点状态
    pub async fn update_status(&self, peer_id: &PeerId, status: PeerStatus) -> bool {
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

    /// 获取空闲节点列表（状态为Connected）
    pub async fn get_idle_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values()
            .filter(|p| p.query_status() == PeerStatus::Connected)
            .cloned()
            .collect()
    }

    /// 获取忙碌节点列表（状态为Busy）
    pub async fn get_busy_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values()
            .filter(|p| p.query_status() == PeerStatus::Busy)
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

    /// 清理超时节点（内部实现）
    async fn cleanup_timeout_peers_internal(peers: &Arc<RwLock<HashMap<PeerId, PeerInfo>>>, timeout_secs: u64) {
        let mut peers_write = peers.write().await;
        peers_write.retain(|_, peer_info| !peer_info.is_timeout(timeout_secs));
    }

    /// 清理超时节点（公开接口）
    pub async fn cleanup_timeout_peers(&self, timeout_secs: u64) -> usize {
        let mut peers = self.peers.write().await;
        let before_count = peers.len();
        peers.retain(|_, peer_info| !peer_info.is_timeout(timeout_secs));
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

    /// 清空所有节点
    pub async fn clear(&self) {
        let mut peers = self.peers.write().await;
        peers.clear();
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new()
    }
}