//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! 地图增量消息（Task 13_2：state 三态 → delta 数值差分）
//!
//! Task 23 阶段 C：从 ugv/slam 移回 base，因为 base 的 map_tx 广播通道
//! 与 cluster_consumer 均消费此类型（共享协议类型，非设备端独有）。

/// 地图增量广播消息
#[derive(Debug, Clone)]
pub struct MapDelta {
    pub gx: i32,
    pub gy: i32,
    pub delta: i8,
}
