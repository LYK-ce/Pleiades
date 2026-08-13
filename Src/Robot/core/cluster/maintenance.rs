//Presented by KeJi
//Created Date ： 2026-08-13
//Modified Date ： 2026-08-13

//! 集群信息表维护 task（Task 15）
//!
//! 周期清理失联车辆，让 `ClusterInfoTable` 语义 = 当前在线车辆集合。
//! 障碍注入侧因此可以无脑读全量快照，不在注入层做在线判断。

use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::info;

use super::cluster_info::ClusterInfoTable;

/// 清理周期
const CLEAN_INTERVAL: Duration = Duration::from_secs(2);
/// 超时阈值：last_seen 超过 2s 视为离线
const STALE_TIMEOUT: Duration = Duration::from_secs(2);

/// 周期清理 task（launch 中 spawn）
pub async fn cluster_table_cleaner(table: Arc<ClusterInfoTable>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(CLEAN_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let removed = table.remove_stale(STALE_TIMEOUT).await;
                if removed > 0 {
                    info!("[Cluster] 清理失联车辆 {removed} 辆");
                }
            }
            _ = cancel.cancelled() => {
                info!("[Cluster] 表维护 task 退出");
                return;
            }
        }
    }
}
