//Presented by KeJi
//Created Date ： 2026-08-10
//Modified Date ： 2026-08-13

//! 集群信息表（Task 13_1 / Task 15 表维护）
//!
//! 入站 POSE 帧解码后的其他车状态，键 = 完整 peer_id。
//! 表语义 = 当前在线车辆集合：consumer 写、规划层读（障碍注入）、
//! 维护 task 通过 `remove_stale` 周期剔除失联车。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    /// 本地接收时刻；超时清理与在线判断依据（Task 15）
    pub last_seen: Instant,
    /// 对方意图：D* 寻路下一格（网格坐标）；None = 无任务
    pub sub_target: Option<(i32, i32)>,
}

/// 集群信息表（Arc 共享，consumer 写 / 规划层读 / 维护 task 清理）
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

    /// 删除 `last_seen` 超过 `timeout` 的失联条目，返回删除数量（Task 15）
    ///
    /// 先读锁收集超时 peer_id：无超时直接返回（不碰写锁）；
    /// 有超时才拿写锁删除，删除时 double-check 仍超时（避免误删读锁释放后刚刷新的车）。
    pub async fn remove_stale(&self, timeout: Duration) -> usize {
        let now = Instant::now();
        let stale: Vec<Vec<u8>> = {
            let guard = self.inner.read().await;
            guard
                .iter()
                .filter(|(_, info)| now.saturating_duration_since(info.last_seen) > timeout)
                .map(|(k, _)| k.clone())
                .collect()
        };
        if stale.is_empty() {
            return 0;
        }
        let mut removed = 0;
        let mut guard = self.inner.write().await;
        for key in stale {
            if let Some(info) = guard.get(&key) {
                if now.saturating_duration_since(info.last_seen) > timeout {
                    guard.remove(&key);
                    removed += 1;
                }
            }
        }
        removed
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

    fn make_info_at(peer_id: &[u8], last_seen: Instant) -> ClusterInfo {
        ClusterInfo {
            peer_id: peer_id.to_vec(),
            x: 0.0,
            y: 0.0,
            yaw: 0.0,
            vx: 0.0,
            vy: 0.0,
            time_boot_ms: 1,
            last_seen,
            sub_target: None,
        }
    }

    #[tokio::test]
    async fn test_remove_stale_empty() {
        let table = ClusterInfoTable::new();
        assert_eq!(table.remove_stale(Duration::from_secs(2)).await, 0);
    }

    #[tokio::test]
    async fn test_remove_stale_none_stale() {
        let table = ClusterInfoTable::new();
        table.upsert(make_info_at(&[1], Instant::now())).await;
        assert_eq!(table.remove_stale(Duration::from_secs(2)).await, 0);
        assert_eq!(table.len().await, 1);
    }

    #[tokio::test]
    async fn test_remove_stale_one_stale() {
        let table = ClusterInfoTable::new();
        let old = Instant::now() - Duration::from_secs(5);
        table.upsert(make_info_at(&[1], old)).await;
        assert_eq!(table.remove_stale(Duration::from_secs(2)).await, 1);
        assert!(table.get(&[1]).await.is_none());
    }

    #[tokio::test]
    async fn test_remove_stale_mixed() {
        let table = ClusterInfoTable::new();
        let old = Instant::now() - Duration::from_secs(5);
        table.upsert(make_info_at(&[1], old)).await;
        table.upsert(make_info_at(&[2], Instant::now())).await;
        assert_eq!(table.remove_stale(Duration::from_secs(2)).await, 1);
        assert!(table.get(&[1]).await.is_none());
        assert!(table.get(&[2]).await.is_some());
    }
}
