//Presented by KeJi
//Date ： 2026-04-28

//! PeerManagement Capability 层
//!
//! 定义节点管理对外暴露的唯一 trait `Peer_Management_Capability`，
//! 以及相关的错误类型 `Peer_Management_Error`。
//!
//! Orchestrator 和 Network 层通过此 trait 操作节点信息。
//! 实现方为 PeerHandle（持有 Arc<PeerManager>）。

use async_trait::async_trait;
use libp2p::PeerId;
use super::peer_info::{PeerInfo, PeerStatus, PeerCapability};

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

    /// 获取所有节点信息
    async fn List_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error>;

    /// 获取单个节点信息
    async fn Get_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;

    /// 检查节点是否存在
    async fn Contains_Peer(&self, peer_id: &PeerId) -> Result<bool, Peer_Management_Error>;

    /// 获取空闲节点列表（状态为 Connected）
    async fn Get_Idle_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error>;

    /// 获取节点数量
    async fn Count(&self) -> Result<usize, Peer_Management_Error>;

    /// 获取所有节点 ID
    async fn Get_All_Peer_Ids(&self) -> Result<Vec<PeerId>, Peer_Management_Error>;

    // ─── 变更操作 ──────────────────────────────

    /// 添加或更新节点信息
    async fn Add_Peer(&self, peer_info: PeerInfo) -> Result<(), Peer_Management_Error>;

    /// 移除节点
    async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;

    /// 更新节点状态
    async fn Update_Status(&self, peer_id: &PeerId, status: PeerStatus) -> Result<(), Peer_Management_Error>;

    /// 更新节点心跳（包含延迟信息）
    async fn Update_Heartbeat(&self, peer_id: &PeerId, latency_ms: Option<u64>) -> Result<(), Peer_Management_Error>;

    /// 更新节点能力
    async fn Update_Capability(&self, peer_id: &PeerId, capability: Option<PeerCapability>) -> Result<(), Peer_Management_Error>;

    /// 更新节点带宽信息
    async fn Update_Bandwidth(&self, peer_id: &PeerId, bandwidth_mbps: Option<u64>) -> Result<(), Peer_Management_Error>;

    /// 清理超时节点，返回清理数量
    async fn Cleanup_Timeout_Peers(&self, timeout_secs: u64) -> Result<usize, Peer_Management_Error>;

    /// 清空所有节点
    async fn Clear(&self) -> Result<(), Peer_Management_Error>;
}
