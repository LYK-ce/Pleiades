//Presented by KeJi
//Date : 2026-04-15

//! 节点信息数据结构定义模块
//!
//! 包含节点状态、能力描述和节点信息等核心数据结构。

use libp2p::{Multiaddr, PeerId};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 节点状态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeerStatus {
    Local,        // 本地协调节点
    Connected,    // 已连接，空闲
    Busy,         // 已连接，忙碌（执行推理任务）
    Connecting,   // 连接建立中
    Disconnected, // 已断开连接
}

/// 节点能力描述
#[derive(Debug, Clone, PartialEq)]
pub struct PeerCapability {
    pub has_gpu: bool,                         // 是否有GPU
    pub memory_mb: u64,                        // 内存大小（MB）
    pub compute_score: f32,                    // 计算能力评分
    pub supported_models: Vec<String>,         // 支持的模型类型
    pub layer_time: HashMap<String, Duration>, // 模型单层耗时
}

impl Default for PeerCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerCapability {
    /// 创建一个默认的节点能力描述
    pub fn new() -> Self {
        Self {
            has_gpu: false,
            memory_mb: 0,
            compute_score: 0.0,
            supported_models: Vec::new(),
            layer_time: HashMap::new(),
        }
    }

    /// 创建一个具有GPU能力的节点能力描述
    pub fn with_gpu(memory_mb: u64, compute_score: f32) -> Self {
        Self {
            has_gpu: true,
            memory_mb,
            compute_score,
            supported_models: Vec::new(),
            layer_time: HashMap::new(),
        }
    }

    /// 更新指定模型的单层耗时
    pub fn set_layer_time(&mut self, model_id: String, duration: Duration) {
        self.layer_time.insert(model_id, duration);
    }
}

/// 节点详细信息
#[derive(Debug, Clone, PartialEq)]
pub struct PeerInfo {
    pub peer_id: PeerId,                    // 节点ID
    pub addresses: Vec<Multiaddr>,          // 地址列表
    pub latency_ms: Option<u64>,            // 最后一次ping延迟
    pub bandwidth_mbps: Option<u64>,        // 带宽（Mbps），可选
    pub connected_at: Instant,              // 连接建立时间
    pub last_active: Instant,               // 最后活跃时间
    pub status: PeerStatus,                 // 节点状态
    pub capability: Option<PeerCapability>, // 节点能力（可选）
}

impl PeerInfo {
    /// 创建一个新的节点信息
    pub fn new(peer_id: PeerId, addresses: Vec<Multiaddr>) -> Self {
        let now = Instant::now();
        Self {
            peer_id,
            addresses,
            latency_ms: None,
            bandwidth_mbps: None,
            connected_at: now,
            last_active: now,
            status: PeerStatus::Connected,
            capability: None,
        }
    }

    /// Update Status 更新节点状态，仅在节点的状态发生变化的时候才会发出状态变化通知，然后调用此方法更改状态
    pub fn update_status(&mut self, status: PeerStatus) {
        self.status = status;
        self.last_active = Instant::now();
    }

    /// Update Heartbeat 心跳更新，更新内容包括最后活跃事件和延迟信息
    pub fn update_heartbeat(&mut self, latency_ms: Option<u64>) {
        self.last_active = Instant::now();
        if let Some(latency) = latency_ms {
            self.latency_ms = Some(latency);
        }
    }

    /// Update Capability 更新节点能力，参数为新的能力描述
    pub fn update_capability(&mut self, capability: Option<PeerCapability>) {
        self.capability = capability;
        self.last_active = Instant::now();
    }

    /// Update Bandwidth 更新节点带宽信息
    pub fn update_bandwidth(&mut self, bandwidth_mbps: Option<u64>) {
        self.bandwidth_mbps = bandwidth_mbps;
        self.last_active = Instant::now();
    }

    /// Query Status 查询节点状态，仅返回status
    pub fn query_status(&self) -> PeerStatus {
        self.status
    }

    /// Query Profile 查询节点能力、延迟和带宽，返回能力描述、延迟信息和带宽信息，因为这三者作为节点分配依据，往往需要一起查询
    pub fn query_profile(&self) -> (Option<&PeerCapability>, Option<u64>, Option<u64>) {
        (
            self.capability.as_ref(),
            self.latency_ms,
            self.bandwidth_mbps,
        )
    }

    /// Is_Timeout 检查节点是否超时，参数为超时时间（秒），返回布尔值
    pub fn is_timeout(&self, timeout_secs: u64) -> bool {
        let elapsed = self.last_active.elapsed().as_secs();
        elapsed >= timeout_secs
    }
}
