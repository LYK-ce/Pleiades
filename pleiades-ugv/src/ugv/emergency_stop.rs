//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-11-01

//! 急停检查（Task 23 决策 D1：从 robot.rs 拆出，车端独有）
//!
//! 前方 LiDAR 扇形（±45°）最近点距离 < 阈值 → 立即停车 + 标记障碍到 D*。
//! 返回 true = 已触发急停（跳过本 tick 的决策）。
//!
//! 停车走 `stm32.stop()` = `0x12 FUNC_MOTION` 全零帧 = `apply_velocity(0, 0)`，
//! 但**保留 10×10 ms 有界重试**（TX 拥塞时安全关键帧不静默丢弃，task_44 §1.2 #1）。

use std::f32::consts::PI;
use std::sync::Arc;

use tracing::warn;

use pleiades_base::robot::core::grid::CELL_RESOLUTION;
use pleiades_base::robot::core::state::RobotState;
use crate::ugv::lidar::LidarDevice;
use crate::ugv::stm32::STM32Device;
use crate::ugv::nav::Navigator;

/// 急停距离阈值（米）
const OBSTACLE_THRESHOLD_M: f32 = 0.3;

/// 前方障碍急停检查：LiDAR 前方距离 < 阈值 → 立即停车 + 标记障碍。
pub async fn check_emergency_stop(
    lidar: &LidarDevice,
    robot_state: &RobotState,
    stm32: &STM32Device,
    nav: &Arc<tokio::sync::Mutex<Navigator>>,
) -> bool {
    let Some(scan) = lidar.get_scan().await else {
        return false;
    };
    let closest = scan
        .points
        .iter()
        .filter(|p| {
            let a = if p.angle < 0.0 { p.angle + 2.0 * PI } else { p.angle };
            p.range >= 0.1 && (a < PI / 4.0 || a >= 7.0 * PI / 4.0)
        })
        .min_by(|a, b| a.range.partial_cmp(&b.range).unwrap_or(std::cmp::Ordering::Equal));
    let Some(p) = closest else {
        return false;
    };

    if p.range < OBSTACLE_THRESHOLD_M {
        warn!("[Robot] 前方障碍 {:.2}m < {:.2}m，急停", p.range, OBSTACLE_THRESHOLD_M);
        // `stop()` = `0x12` 全零（等价 `apply_velocity(0, 0)`）+ 有界重试
        if let Err(e) = stm32.stop() {
            warn!("[Robot] 急停失败: {e}");
        }

        let (wx, wy) = (robot_state.x, robot_state.y);
        let yaw = robot_state.attitude.yaw;
        let ob_angle = yaw + p.angle;
        let ob_wx = wx + p.range * ob_angle.cos();
        let ob_wy = wy + p.range * ob_angle.sin();
        let ob_gx = (ob_wx / CELL_RESOLUTION).floor() as i32;
        let ob_gy = (ob_wy / CELL_RESOLUTION).floor() as i32;
        nav.lock().await.mark_obstacle((ob_gx, ob_gy)).await;
        return true;
    }
    false
}
