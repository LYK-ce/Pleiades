//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! 激光雷达数据类型（task_31 §五.4.1，**sim 自带定义**）
//!
//! 字段与真机 `LaserPoint` / `LaserScan` / `LidarState` 一致，便于以后对账；
//! 但**不 import 任何 ugv 类型**（`pleiades-sim` 禁止引用 `pleiades-ugv`，§五.2-1）。
//!
//! 语义（与真机同义）：
//! - `angle` 弧度、`range` 米（`0.0` = 无回波，**原样保留**）
//! - `intensity` 世界帧不上线 → 恒 `0.0`
//! - `stamp` 纳秒（帧头 `world_time * 1e9`）

/// 单个激光点（角度 弧度 / 距离 米 / 强度 0~1）
#[derive(Debug, Clone, Copy, Default)]
pub struct LaserPoint {
    pub angle: f32,
    pub range: f32,
    /// 信号强度：世界帧不上线（`FRAME_TYPE_LIDAR` 只有距离）→ 恒 `0.0`；
    /// 字段保留与真机 `LaserPoint` 对齐（以后要强度图需扩帧格式，另立 step）
    #[allow(dead_code)]
    pub intensity: f32,
}

/// 一圈扫描（字段与真机 `LaserScan` 一致，便于以后对账）
#[derive(Debug, Clone, Default)]
pub struct LaserScan {
    pub points: Vec<LaserPoint>,
    /// 纳秒（由帧头 `world_time * 1e9` 得来）；本 step 只记录，Step 4/6 与真机对账时消费
    #[allow(dead_code)]
    pub stamp: u64,
    /// 扫描频率 Hz（配置 `rate_hz`）
    pub scan_freq: f32,
    /// 一圈总时间 秒（`1 / rate_hz`）；本 step 只记录（与真机字段对齐）
    #[allow(dead_code)]
    pub scan_time: f32,
}

/// 激光雷达状态（建图 task / Step 6 急停读；device 自己的 state）
#[derive(Debug, Default)]
pub struct LidarState {
    pub scan: Option<LaserScan>,
}
