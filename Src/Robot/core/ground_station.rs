//Presented by KeJi
//Created Date ： 2026-08-15
//Modified Date ： 2026-08-15

//! 地面站消费侧（Task 16）
//!
//! 无硬件的地面站节点组件：复用 `cluster_consumer`（robot_bus 遥测 → 表 + 地图）
//! 和 `cluster_table_cleaner`（失联清理），供桥读快照渲染。
//! 与 `Robot::launch` 的消费部分对称，但不含任何 STM32/LiDAR 硬件。

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::event_bus::EventBus;
use crate::robot::core::cluster::{cluster_consumer, cluster_table_cleaner, ClusterInfoTable};
use crate::robot::slam::OccupancyGrid;

/// 地面站消费侧组件（无硬件，桥渲染数据源）
pub struct GroundStation {
    /// 远端车辆状态表（pose 进表，consumer 写 / 桥读）
    pub table: Arc<ClusterInfoTable>,
    /// 合并占据栅格地图（consumer 写 / 桥读）
    pub grid: Arc<RwLock<OccupancyGrid>>,
    cancel: CancellationToken,
}

impl GroundStation {
    /// 启动地面站消费侧：创建 table/grid，spawn `cluster_consumer` + `cluster_table_cleaner`
    ///
    /// 须在 tokio runtime 上下文调用（内部 `tokio::spawn`）。
    pub fn launch(robot_bus: Arc<EventBus>, local_peer_id: Vec<u8>) -> Self {
        let table = Arc::new(ClusterInfoTable::new());
        let grid = Arc::new(RwLock::new(OccupancyGrid::new()));
        let cancel = CancellationToken::new();

        // 遥测消费（pose → 表 / map_delta → merged grid）
        {
            let (bus, t, g, c) = (robot_bus, table.clone(), grid.clone(), cancel.clone());
            tokio::spawn(async move {
                cluster_consumer(Some(bus), local_peer_id, t, g, c).await;
            });
        }

        // 失联清理（2s 周期 / 2s 超时）
        {
            let (t, c) = (table.clone(), cancel.clone());
            tokio::spawn(async move {
                cluster_table_cleaner(t, c).await;
            });
        }

        Self { table, grid, cancel }
    }

    /// 优雅停机：取消所有后台 task
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::Bus_Event;
    use crate::robot::core::protocol::{
        encode_frame, encode_pose, COMPID_ROBOT, MSGID_POSE, PoseData,
    };

    #[tokio::test]
    async fn test_ground_station_consumes_pose() {
        let bus = Arc::new(EventBus::New(64));
        let gs = GroundStation::launch(bus.clone(), vec![0x01]);
        // 等待 consumer 订阅（broadcast 无重放，先订阅再 publish）
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // 模拟远端车 pose 帧（sysid=0xAA ≠ local=0x01，不被 self-filter 过滤）
        let pose = PoseData {
            time_boot_ms: 1,
            x: 10.0,
            y: 20.0,
            vx: 0.0,
            vy: 0.0,
            yaw: 0.5,
            valid: false,
            sub_gx: 0,
            sub_gy: 0,
        };
        let frame = encode_frame(MSGID_POSE, &[0xAA], COMPID_ROBOT, &encode_pose(&pose));
        bus.Publish(Bus_Event::StreamRaw { payload: frame });

        // 等待 consumer 处理
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let info = gs.table.get(&[0xAA]).await.expect("远端车应已入表");
        assert_eq!(info.x, 10.0);
        assert_eq!(info.y, 20.0);

        gs.shutdown();
    }
}
