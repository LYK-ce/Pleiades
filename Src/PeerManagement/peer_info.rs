//Presented by KeJi
//Date : 2026-05-13

//! 节点信息数据结构定义模块
//!
//! 包含节点信息、模型持有描述和动态性能画像等核心数据结构。
//!
//! ## 数据结构
//! - `SupportedModel` — 节点持有的模型描述（id + file_name + layer_bitmap）
//! - `PeerProfile` — 动态性能画像（延迟/带宽/内存/单层耗时）
//! - `PeerInfo` — 节点完整信息（身份元信息 + PeerProfile + SupportedModel 列表）

use libp2p::{Multiaddr, PeerId};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 节点持有的模型描述
///
/// 调度时依据此结构决定模型分配：
/// - `id` 用于跨节点匹配同一模型
/// - `layer_bitmap` 用于判断节点持有模型的哪些层
#[derive(Debug, Clone, PartialEq)]
pub struct SupportedModel {
    /// 模型唯一标识 — xxhash64(content) → u64
    pub id: u64,
    /// 存储文件名（Storage file_id）
    pub file_name: String,
    /// 256 位层位图，bit N = 1 表示持有第 N 层
    pub layer_bitmap: [u8; 32],
}

impl SupportedModel {
    /// 创建持有完整模型（全部层）的描述
    pub fn full(id: u64, file_name: String) -> Self {
        Self {
            id,
            file_name,
            layer_bitmap: [0xFF; 32],
        }
    }

    /// 创建持有模型分片的描述，仅指定层范围的位为 1
    pub fn shard(id: u64, file_name: String, layer_start: usize, layer_end: usize) -> Self {
        let mut bitmap = [0u8; 32];
        for layer in layer_start..layer_end {
            let byte_idx = layer / 8;
            let bit_idx = layer % 8;
            if byte_idx < 32 {
                bitmap[byte_idx] |= 1 << bit_idx;
            }
        }
        Self {
            id,
            file_name,
            layer_bitmap: bitmap,
        }
    }

    /// 检查是否持有指定层范围的全部层
    pub fn has_layer_range(&self, start: usize, end: usize) -> bool {
        for layer in start..end {
            let byte_idx = layer / 8;
            let bit_idx = layer % 8;
            if byte_idx >= 32 || (self.layer_bitmap[byte_idx] & (1 << bit_idx)) == 0 {
                return false;
            }
        }
        true
    }

    /// 返回位图中为 1 的层数
    pub fn layer_count(&self) -> usize {
        self.layer_bitmap
            .iter()
            .map(|b| b.count_ones() as usize)
            .sum()
    }
}

/// 节点动态性能画像
///
/// 运行时频繁更新。`Update_Profile` 方法接收此结构，
/// 字段为 `None` 时跳过不更新，`Some(v)` 时更新为 v。
#[derive(Debug, Clone, PartialEq)]
pub struct PeerProfile {
    /// 最后 ping 延迟（毫秒）
    pub latency_ms: Option<u64>,
    /// 带宽（Mbps）
    pub bandwidth_mbps: Option<u64>,
    /// 空闲内存（MB），随运行变化
    pub memory_mb: Option<u64>,
    /// 模型单层耗时（model_id → Duration）
    pub layer_time: Option<HashMap<String, Duration>>,
}

impl Default for PeerProfile {
    fn default() -> Self {
        Self {
            latency_ms: None,
            bandwidth_mbps: None,
            memory_mb: None,
            layer_time: None,
        }
    }
}

/// 节点详细信息
#[derive(Debug, Clone, PartialEq)]
pub struct PeerInfo {
    /// 节点 ID
    pub peer_id: PeerId,
    /// 地址列表
    pub addresses: Vec<Multiaddr>,
    /// 是否为本地节点
    pub local: bool,
    /// 连接建立时间
    pub connected_at: Instant,
    /// 最后活跃时间
    pub last_active: Instant,
    /// 动态性能画像
    pub profile: PeerProfile,
    /// 持有的模型列表
    pub supported_models: Vec<SupportedModel>,
}

impl PeerInfo {
    /// 创建一个新的远程节点信息
    pub fn new(peer_id: PeerId, addresses: Vec<Multiaddr>) -> Self {
        let now = Instant::now();
        Self {
            peer_id,
            addresses,
            local: false,
            connected_at: now,
            last_active: now,
            profile: PeerProfile::default(),
            supported_models: Vec::new(),
        }
    }

    /// 创建本地节点信息
    pub fn new_local(peer_id: PeerId) -> Self {
        let now = Instant::now();
        Self {
            peer_id,
            addresses: Vec::new(),
            local: true,
            connected_at: now,
            last_active: now,
            profile: PeerProfile::default(),
            supported_models: Vec::new(),
        }
    }

    /// Update Profile 更新性能画像，字段为 None 时跳过
    pub fn update_profile(&mut self, profile: PeerProfile) {
        if let Some(v) = profile.latency_ms {
            self.profile.latency_ms = Some(v);
        }
        if let Some(v) = profile.bandwidth_mbps {
            self.profile.bandwidth_mbps = Some(v);
        }
        if let Some(v) = profile.memory_mb {
            self.profile.memory_mb = Some(v);
        }
        if let Some(v) = profile.layer_time {
            self.profile.layer_time = Some(v);
        }
        self.last_active = Instant::now();
    }

    /// Update Supported Models 更新持有的模型列表
    pub fn update_supported_models(&mut self, models: Vec<SupportedModel>) {
        self.supported_models = models;
        self.last_active = Instant::now();
    }

    /// Is Timeout 检查节点是否超时
    pub fn is_timeout(&self, timeout_secs: u64) -> bool {
        let elapsed = self.last_active.elapsed().as_secs();
        elapsed >= timeout_secs
    }
}
