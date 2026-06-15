//Presented by KeJi
//Created Date ： 2026-05-13
//Modified Date ： 2026-06-15

//! PeerManagement Capability 层
//!
//! 定义节点管理对外暴露的唯一 trait `Peer_Management_Capability`，
//! 以及相关的错误类型 `Peer_Management_Error`。
//!
//! Orchestrator 和 Network 层通过此 trait 操作节点信息。
//! 实现方为 PeerHandle（持有 Arc<PeerManager>）。

use async_trait::async_trait;
use libp2p::PeerId;
use super::peer_info::{PeerInfo, PeerProfile, SessionSummary, SupportedModel};

/// 节点管理错误
#[derive(Debug, thiserror::Error)]
pub enum Peer_Management_Error {
    #[error("Peer not found: {0}")]
    PeerNotFound(String),
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

    /// 获取所有节点列表（含本地）
    async fn Get_All_Peers(&self) -> Result<Vec<PeerInfo>, Peer_Management_Error>;

     /// 获取本地节点信息
     async fn Get_Local_Peer(&self) -> Result<PeerInfo, Peer_Management_Error>;

    /// 按显示名称精确匹配节点（匹配 name#XXXX）
    async fn Get_Peer_By_Name(&self, name: &str) -> Result<PeerInfo, Peer_Management_Error>;

    // ─── 变更操作 ──────────────────────────────

    /// 添加或覆盖节点信息，返回 true=更新已有节点，false=新增节点
    async fn Upsert_Peer(&self, peer_info: PeerInfo) -> Result<bool, Peer_Management_Error>;

    /// 设置本地节点名称
    async fn Set_Local_Name(&self, name: &str) -> Result<(), Peer_Management_Error>;

     /// 移除节点（保护本地节点）
     async fn Remove_Peer(&self, peer_id: &PeerId) -> Result<PeerInfo, Peer_Management_Error>;

     /// 更新节点性能画像，字段为 None 时跳过
     async fn Update_Profile(&self, peer_id: &PeerId, profile: PeerProfile) -> Result<(), Peer_Management_Error>;

    /// 更新节点持有的模型列表
    async fn Update_Supported_Models(&self, peer_id: &PeerId, models: Vec<SupportedModel>) -> Result<(), Peer_Management_Error>;

    /// 更新节点的 sessions 列表
    async fn Update_Sessions(&self, peer_id: &PeerId, sessions: Vec<SessionSummary>) -> Result<(), Peer_Management_Error>;

    /// 清空所有节点（保留本地节点）
    async fn Clear(&self) -> Result<(), Peer_Management_Error>;
}
