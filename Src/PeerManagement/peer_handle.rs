//Presented by KeJi
//Date : 2026-04-15

//! 节点管理对外调用接口
//!
//! 该模块提供节点管理器的对外调用接口，封装内部实现细节，
//! 提供线程安全、易于使用的API。

use std::sync::Arc;

use libp2p::PeerId;

use super::peer_info::{PeerInfo, PeerStatus, PeerCapability};
use super::peer_manager::PeerManager;

/// 节点管理句柄（类似C头文件接口）
#[derive(Clone)]
pub struct PeerHandle {
    inner: Arc<PeerManager>, // 内部管理器引用
}

/// 节点事件（用于通知）
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

/// 节点管理错误
#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    /// 节点未找到
    #[error("Peer not found: {0}")]
    PeerNotFound(PeerId),
    
    /// 操作超时
    #[error("Operation timeout")]
    Timeout,
    
    /// 内部错误
    #[error("Internal error: {0}")]
    Internal(String),
}

impl PeerHandle {
    /// 创建一个新的节点管理句柄
    pub fn new(manager: Arc<PeerManager>) -> Self {
        Self { inner: manager }
    }

    /// 添加节点
    pub async fn add_peer(&self, peer_info: PeerInfo) -> Result<(), PeerError> {
        self.inner.upsert_peer(peer_info).await;
        Ok(())
    }

    /// 移除节点
    pub async fn remove_peer(&self, peer_id: &PeerId) -> Result<PeerInfo, PeerError> {
        self.inner.remove_peer(peer_id).await
            .ok_or_else(|| PeerError::PeerNotFound(*peer_id))
    }

    /// 列出所有节点
    pub async fn list_peers(&self) -> Result<Vec<PeerInfo>, PeerError> {
        Ok(self.inner.get_all_peers().await)
    }

    /// 获取节点信息
    pub async fn get_peer(&self, peer_id: &PeerId) -> Result<PeerInfo, PeerError> {
        self.inner.get_peer(peer_id).await
            .ok_or_else(|| PeerError::PeerNotFound(*peer_id))
    }

    /// 更新节点状态
    pub async fn update_status(&self, peer_id: &PeerId, status: PeerStatus) -> Result<(), PeerError> {
        if self.inner.update_status(peer_id, status).await {
            Ok(())
        } else {
            Err(PeerError::PeerNotFound(*peer_id))
        }
    }

    /// 获取空闲节点
    pub async fn get_idle_peers(&self) -> Result<Vec<PeerInfo>, PeerError> {
        Ok(self.inner.get_idle_peers().await)
    }

    /// 获取节点数量
    pub async fn count(&self) -> Result<usize, PeerError> {
        Ok(self.inner.count().await)
    }

    /// 检查是否为空
    pub async fn is_empty(&self) -> Result<bool, PeerError> {
        Ok(self.inner.is_empty().await)
    }

    /// 更新节点心跳（包含延迟信息）
    pub async fn update_heartbeat(&self, peer_id: &PeerId, latency_ms: Option<u64>) -> Result<(), PeerError> {
        if self.inner.update_heartbeat(peer_id, latency_ms).await {
            Ok(())
        } else {
            Err(PeerError::PeerNotFound(*peer_id))
        }
    }

    /// 更新节点能力
    pub async fn update_capability(&self, peer_id: &PeerId, capability: Option<PeerCapability>) -> Result<(), PeerError> {
        if self.inner.update_capability(peer_id, capability).await {
            Ok(())
        } else {
            Err(PeerError::PeerNotFound(*peer_id))
        }
    }

    /// 获取忙碌节点列表
    pub async fn get_busy_peers(&self) -> Result<Vec<PeerInfo>, PeerError> {
        Ok(self.inner.get_busy_peers().await)
    }

    /// 清理超时节点
    pub async fn cleanup_timeout_peers(&self, timeout_secs: u64) -> Result<usize, PeerError> {
        Ok(self.inner.cleanup_timeout_peers(timeout_secs).await)
    }

    /// 获取所有节点ID
    pub async fn get_all_peer_ids(&self) -> Result<Vec<PeerId>, PeerError> {
        Ok(self.inner.get_all_peer_ids().await)
    }

    /// 检查节点是否存在
    pub async fn contains_peer(&self, peer_id: &PeerId) -> Result<bool, PeerError> {
        Ok(self.inner.contains_peer(peer_id).await)
    }

    /// 清空所有节点
    pub async fn clear(&self) -> Result<(), PeerError> {
        self.inner.clear().await;
        Ok(())
    }

    /// 获取内部管理器引用（用于高级操作）
    pub fn inner(&self) -> &Arc<PeerManager> {
        &self.inner
    }
}

impl std::fmt::Debug for PeerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerHandle")
            .field("inner", &"Arc<PeerManager>")
            .finish_non_exhaustive()
    }
}