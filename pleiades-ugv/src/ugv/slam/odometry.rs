//Presented by KeJi
//Created Date ： 2026-07-29
//Modified Date ： 2026-08-06

//! 里程计定位
//!
//! 基于 STM32 编码器线速度 (vx, vy) + IMU 航向角 (yaw) 在世界坐标上积分。
//! 在 STM32Device RX 回调中收到 RPT_SPEED 帧后调用。
//!
//! x/y 初值 = origin，由 `Robot::new` 注入共享 RobotState（Task 24：从 local_state 上移）。
//! 本函数必须在 STM32 RX 回调已持有写锁的共享 RobotState guard 上调用
//! （`accumulate(&mut *guard, dt)`），与 LG290P RTK 覆盖共写 x/y。

use pleiades_base::robot::core::state::RobotState;

/// 基于当前 vx/vy + yaw 在世界坐标上累积当前位置
///
pub fn accumulate(state: &mut RobotState, dt: f32) {
    let yaw = state.attitude.yaw;
    let (sin, cos) = yaw.sin_cos();
    state.x += (state.vx * cos - state.vy * sin) * dt;
    state.y += (state.vx * sin + state.vy * cos) * dt;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pleiades_base::robot::core::state::Attitude;

    /// 世界坐标积分：x/y 以 origin 为起点，位移叠加在原点之上（Task 9）
    #[test]
    fn test_accumulate_world_coords() {
        // 初始位置 = origin (66.5, 63.25)，朝 +x
        let mut s = RobotState::default();
        s.x = 66.5;
        s.y = 63.25;
        s.vx = 1.0;
        s.vy = 0.0;
        s.attitude = Attitude { roll: 0.0, pitch: 0.0, yaw: 0.0 };

        accumulate(&mut s, 2.0); // dt=2s, vx=1 → 世界位移 (2, 0)
        assert!((s.x - 68.5).abs() < 1e-4, "x 应在 origin 基础上积分: {}", s.x);
        assert!((s.y - 63.25).abs() < 1e-4, "y 应保持不变: {}", s.y);

        // 旋转 90°（朝 +y），机体 vy=1 → 世界系 -x 方向
        s.attitude.yaw = std::f32::consts::FRAC_PI_2;
        s.vx = 0.0;
        s.vy = 1.0;
        accumulate(&mut s, 1.0); // 世界位移 (-1, 0)
        assert!((s.x - 67.5).abs() < 1e-4, "yaw=90° 时 vy 应旋转到世界 -x: {}", s.x);
        assert!((s.y - 63.25).abs() < 1e-4, "y 应保持不变: {}", s.y);
    }
}
