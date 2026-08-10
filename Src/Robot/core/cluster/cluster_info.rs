//Presented by KeJi
//Created Date ： 2026-08-10
//Modified Date ： 2026-08-10

//! 集群信息表（Task 13_1）
//!
//! 入站 POSE 帧解码后的其他车状态，键 = 完整 peer_id。
//! 本阶段**不做超时处理**（人类决策 2026-08-10）：`last_seen` 仅记录，
//! 不删除/不标记/不淘汰；超时语义留未来消费端（如 P0 寻路障碍注入）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// 远端车辆状态（由 cluster_consumer 写入）
#[derive(Debug, Clone)]
pub struct ClusterInfo {
    /// 完整 peer_id（表键）
    pub peer_id: Vec<u8>,
    /// 位姿（全局世界坐标）
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
    /// 速度
    pub vx: f32,
    pub vy: f32,
    /// 发送方时间戳（仅数据标签，跨车不可比）
    pub time_boot_ms: u32,
    /// 本地接收时刻（本阶段仅记录，不消费）
    pub last_seen: Instant,
    /// 对方意图：D* 寻路下一格（网格坐标）；None = 无任务
    pub sub_target: Option<(i32, i32)>,
}

/// 集群信息表（Arc 共享，consumer 写 / 未来消费端读）
#[derive(Default)]
pub struct ClusterInfoTable {
    inner: RwLock<HashMap<Vec<u8>, ClusterInfo>>,
}

impl ClusterInfoTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 写入/覆盖一条远端车信息（收到 POSE 即调用；同 peer_id 覆盖）
    pub async fn upsert(&self, info: ClusterInfo) {
        self.inner.write().await.insert(info.peer_id.clone(), info);
    }

    /// 查询单辆车
    pub async fn get(&self, peer_id: &[u8]) -> Option<ClusterInfo> {
        self.inner.read().await.get(peer_id).cloned()
    }

    /// 全部远端车快照
    pub async fn snapshot(&self) -> Vec<ClusterInfo> {
        self.inner.read().await.values().cloned().collect()
    }

    /// 表内车辆数
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    /// 是否为空
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_info(peer_id: &[u8], x: f32, y: f32) -> ClusterInfo {
        ClusterInfo {
            peer_id: peer_id.to_vec(),
            x,
            y,
            yaw: 0.0,
            vx: 0.0,
            vy: 0.0,
            time_boot_ms: 1,
            last_seen: Instant::now(),
            sub_target: Some((5, 5)),
        }
    }

    #[tokio::test]
    async fn test_upsert_and_get() {
        let table = ClusterInfoTable::new();
        table.upsert(make_info(&[1, 2, 3], 10.0, 20.0)).await;
        let got = table.get(&[1, 2, 3]).await.unwrap();
        assert_eq!(got.x, 10.0);
        assert_eq!(got.y, 20.0);
        assert_eq!(got.sub_target, Some((5, 5)));
        assert!(table.get(&[9]).await.is_none());
    }

    #[tokio::test]
    async fn test_upsert_overwrite_same_key() {
        let table = ClusterInfoTable::new();
        table.upsert(make_info(&[1, 2, 3], 10.0, 20.0)).await;
        table.upsert(make_info(&[1, 2, 3], 30.0, 40.0)).await;
        assert_eq!(table.len().await, 1); // 同键覆盖不增条目
        let got = table.get(&[1, 2, 3]).await.unwrap();
        assert_eq!(got.y, 40.0);
    }

    #[tokio::test]
    async fn test_snapshot_multi() {
        let table = ClusterInfoTable::new();
        table.upsert(make_info(&[1], 1.0, 1.0)).await;
        table.upsert(make_info(&[2], 2.0, 2.0)).await;
        assert_eq!(table.snapshot().await.len(), 2);
        assert!(!table.is_empty().await);
    }
}
