//Presented by KeJi
//Created Date ： 2026-08-08
//Modified Date ： 2026-08-08

//! GossipSub 业务状态广播模块
//!
//! 负责三个业务 topic 的常量定义与通用快照缓存：
//! - `pleiades/peer-info`  — 节点身份（name；未来扩展算力/内存等硬件信息）
//! - `pleiades/models`     — 模型能力（SupportedModel 列表 + 层位图）
//! - `pleiades/sessions`   — 会话状态（SessionSummary 列表）
//!
//! 分层职责（Task 20 重构后）：
//! - **payload 构建**：业务层（PeerManagement::Build_*_Payload）负责，Network 层不解析业务结构
//! - **发布唯一口**：`Network_Capability::publish_gossipsub(topic, payload)`（由调用方选 topic、构造 payload）
//! - **快照机制**：Network 层只缓存 bytes，对方订阅 topic 时按 topic 精准重放（类似 MQTT retained message）
//! - **接收侧**：Network 层消化（Handle_Gossipsub_Event），业务层无感

use std::collections::HashMap;

// ===== Topic 常量 =====

/// peer-info topic：节点身份（name；未来扩展算力/内存等硬件信息）
pub const TOPIC_PEER_INFO: &str = "pleiades/peer-info";
/// models topic：模型能力（SupportedModel 列表 + 层位图）
pub const TOPIC_MODELS: &str = "pleiades/models";
/// sessions topic：会话状态（SessionSummary 列表）
pub const TOPIC_SESSIONS: &str = "pleiades/sessions";
/// robot pose topic：ORION 位姿帧（10Hz 高频遥测，Task 12）
pub const TOPIC_ROBOT_POSE: &str = "pleiades/robot/pose";
/// robot map topic：ORION 地图帧（增量/全量，状态快照，Task 12）
pub const TOPIC_ROBOT_MAP: &str = "pleiades/robot/map";

// ===== 快照缓存（通用消息层机制，类似 MQTT retained message） =====

/// 快照缓存：缓存每个 topic 最近发布的 payload
///
/// - 发布（GossipsubPublish）时更新：无论是否有订阅者都更新——快照是"最近状态"，
///   与订阅者无关，新节点加入订阅时重放即可
/// - 收到对方 `Subscribed` 事件时按 topic 精准重放（gossipsub 只投递给已订阅者，
///   订阅完成后发布必达，因此连接建立时不重放）
pub struct SnapshotCache {
    snapshots: HashMap<String, Vec<u8>>,
}

impl SnapshotCache {
    /// 创建空快照缓存
    pub fn New() -> Self {
        Self {
            snapshots: HashMap::new(),
        }
    }

    /// 发布时更新快照（覆盖该 topic 最近一次 payload）
    pub fn Update(&mut self, topic: &str, payload: Vec<u8>) {
        self.snapshots.insert(topic.to_string(), payload);
    }

    /// 按 topic 取快照（精准重放用）
    pub fn Get(&self, topic: &str) -> Option<Vec<u8>> {
        self.snapshots.get(topic).cloned()
    }
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self::New()
    }
}
