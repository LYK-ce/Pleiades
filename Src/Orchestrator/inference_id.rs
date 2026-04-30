//Presented by KeJi
//Date ： 2026-04-30

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, Ordering};

static LOCAL_COUNTER: AtomicU32 = AtomicU32::new(1);

/// 生成分布式系统内全局唯一的 inference_id
///
/// 算法：PeerId 哈希高 32 位 | 本地原子计数器低 32 位
pub fn Generate_Inference_Id(local_peer_id: &libp2p::PeerId) -> u64 {
    let mut hasher = DefaultHasher::new();
    local_peer_id.hash(&mut hasher);
    let peer_hash = hasher.finish();

    let high = (peer_hash >> 32) as u32;
    let low = LOCAL_COUNTER.fetch_add(1, Ordering::Relaxed);

    ((high as u64) << 32) | (low as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_inference_id_uniqueness() {
        let peer_id = libp2p::PeerId::random();
        let id1 = Generate_Inference_Id(&peer_id);
        let id2 = Generate_Inference_Id(&peer_id);
        assert_ne!(id1, id2, "连续生成的 inference_id 应不同");
    }

    #[test]
    fn test_generate_inference_id_high_bits_consistent() {
        let peer_id = libp2p::PeerId::random();
        let id1 = Generate_Inference_Id(&peer_id);
        let id2 = Generate_Inference_Id(&peer_id);
        // 高 32 位应相同（来自同一 PeerId）
        assert_eq!(id1 >> 32, id2 >> 32);
    }

    #[test]
    fn test_generate_inference_id_low_bits_increment() {
        let peer_id = libp2p::PeerId::random();
        let id1 = Generate_Inference_Id(&peer_id);
        let id2 = Generate_Inference_Id(&peer_id);
        // 低 32 位递增
        let low1 = (id1 & 0xFFFF_FFFF) as u32;
        let low2 = (id2 & 0xFFFF_FFFF) as u32;
        assert_eq!(low2, low1 + 1);
    }
}
