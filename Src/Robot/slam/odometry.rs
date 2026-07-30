//Presented by KeJi
//Created Date ： 2026-07-29
//Modified Date ： 2026-07-29

//! 里程计定位
//!
//! 基于 STM32 编码器线速度 (vx, vy) + IMU 航向角 (yaw) 累积位移。
//! 在 STM32Device RX 回调中收到 RPT_SPEED 帧后调用。

use crate::robot::core::state::RobotState;

/// 基于当前 vx/vy + yaw 累积里程计位移
///
/// dt 为距上一次 SPEED 帧的真实时间间隔（秒）。
/// 使用全向运动模型：vx（前进）和 vy（横移）都参与旋转到世界坐标系。
pub fn accumulate(state: &mut RobotState, dt: f32) {
    let yaw = state.attitude.yaw;
    let (sin, cos) = yaw.sin_cos();
    state.odom_x += (state.vx * cos - state.vy * sin) * dt;
    state.odom_y += (state.vx * sin + state.vy * cos) * dt;
}
