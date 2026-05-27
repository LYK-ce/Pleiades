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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SupportedModel {
    /// 模型唯一标识 — xxhash32(content) → u32
    pub id: u32,
    /// 存储文件名（Storage file_id）
    pub file_name: String,
    /// 256 位层位图，bit N = 1 表示持有第 N 层
    #[serde(serialize_with = "serialize_bitmap_hex", deserialize_with = "deserialize_bitmap_hex")]
    pub layer_bitmap: [u8; 32],
}

fn serialize_bitmap_hex<S: serde::Serializer>(bitmap: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
    let hex: String = bitmap.iter().map(|b| format!("{:02x}", b)).collect();
    s.serialize_str(&hex)
}

fn deserialize_bitmap_hex<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
    let hex: String = serde::Deserialize::deserialize(d)?;
    if hex.len() != 64 {
        return Err(serde::de::Error::custom("layer_bitmap hex must be 64 chars"));
    }
    let mut bitmap = [0u8; 32];
    for i in 0..32 {
        bitmap[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|e| serde::de::Error::custom(format!("invalid hex: {}", e)))?;
    }
    Ok(bitmap)
}

impl SupportedModel {
    /// 创建持有完整模型（全部层）的描述
    pub fn full(id: u32, file_name: String) -> Self {
        Self {
            id,
            file_name,
            layer_bitmap: [0xFF; 32],
        }
    }

    /// 创建持有模型分片的描述，仅指定层范围的位为 1
    pub fn shard(id: u32, file_name: String, layer_start: usize, layer_end: usize) -> Self {
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

    /// 将 layer_bitmap 解码为层范围字符串（如 "0-31" 或 "0-3,5,7-9"）
    pub fn layer_range(&self) -> String {
        let mut layers: Vec<usize> = Vec::new();
        for i in 0..256 {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if self.layer_bitmap[byte_idx] & (1 << bit_idx) != 0 {
                layers.push(i);
            }
        }
        if layers.is_empty() {
            return "none".to_string();
        }
        let mut ranges = Vec::new();
        let mut start = layers[0];
        let mut end = layers[0];
        for &l in &layers[1..] {
            if l == end + 1 {
                end = l;
            } else {
                ranges.push(if start == end {
                    format!("{}", start)
                } else {
                    format!("{}-{}", start, end)
                });
                start = l;
                end = l;
            }
        }
        ranges.push(if start == end {
            format!("{}", start)
        } else {
            format!("{}-{}", start, end)
        });
        ranges.join(",")
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

/// 推理会话摘要（用于 PeerInfo，`#[serde(skip)]` 仅本地使用）
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionSummary {
    pub session_id: u64,
    pub model_id: String,
    pub occupied_slots: usize,
    pub total_slots: usize,
}

/// 节点详细信息
#[derive(Debug, Clone, PartialEq)]
pub struct PeerInfo {
    /// 节点 ID
    pub peer_id: PeerId,
    /// 节点名称（不含 #XXXX 后缀，空字符串表示未知）
    pub name: String,
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
    /// 本节点的活跃会话（仅 local=true 有意义）
    pub sessions: Vec<SessionSummary>,
}

impl PeerInfo {
    /// 创建一个新的远程节点信息（名称未知）
    pub fn new(peer_id: PeerId, addresses: Vec<Multiaddr>) -> Self {
        let now = Instant::now();
        Self {
            peer_id,
            name: String::new(),
            addresses,
            local: false,
            connected_at: now,
            last_active: now,
            profile: PeerProfile::default(),
            supported_models: Vec::new(),
            sessions: Vec::new(),
        }
    }

    /// 创建本地节点信息
    pub fn new_local(peer_id: PeerId, name: String) -> Self {
        let now = Instant::now();
        Self {
            peer_id,
            name,
            addresses: Vec::new(),
            local: true,
            connected_at: now,
            last_active: now,
            profile: PeerProfile::default(),
            supported_models: Vec::new(),
            sessions: Vec::new(),
        }
    }

    /// 显示名称：有 name → "name#XXXX"（XXXX 为 peer_id 后 4 位），否则 "unknown"
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            "unknown".to_string()
        } else {
            let peer_str = self.peer_id.to_string();
            let suffix = if peer_str.len() >= 4 { &peer_str[peer_str.len()-4..] } else { &peer_str };
            format!("{}#{}", self.name, suffix)
        }
    }

    /// 设置节点名称
    pub fn set_name(&mut self, name: String) {
        self.name = name;
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
