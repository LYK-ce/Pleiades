//Presented by KeJi
//Created Date ： 2026-10-29
//Modified Date ： 2026-10-29

//! 机栈连续速度执行器（task_39 §四.2a）
//!
//! **与车栈 `crate::sim::executor::DecisionExecutor` 刻意分成两份**：真机本来就是
//! 「车一套、机一套」，sim 只是历史原因共用。车栈继续跑离散三状态机，**车栈一个字都不碰**。
//!
//! **口径与真机 UAV 一致**（task_39 §四 修订，2026-10-29）：输出的是**物理量**
//! —— `v` 单位 m/s、`omega` 单位 rad/s；两个值来自 `[flight_ctrl]` 的 `vel_fwd` /
//! `yaw_rate_deg`（与 `pleiades-uav` 同名同单位），不再借用车的 `[chassis]` 百分比档位。
//!
//! 控制律（三档全有全无 —— 没有比例项、没有中间值）：
//!
//! | 偏角 \|Δθ\| | `v` | `omega` | 怎么动 |
//! |---|---|---|---|
//! | ≤ [`DEADBAND_DEG`]（5°） | `vel_fwd` | `0` | 直行 |
//! | 5° ~ [`SPIN_DEG`]（20°） | `vel_fwd` | `±yaw_rate_rps` | 边走边转（弧线） |
//! | > [`SPIN_DEG`]（20°） | `0` | `±yaw_rate_rps` | 原地转 |
//!
//! `omega` 的符号**取负**（本文件最容易写错的一处，证据链）：
//! - 决策侧：`delta > 0` → `MotionAction::TurnRight`（`sim/executor.rs`）；
//! - 翻译表：`TurnRight → action="right"`（`sim/chassis/action_map.rs`）；
//! - 世界侧：`"right" → _yaw_target = -_max_yaw_rate·pct`（`Quadrotor.gd` 的 `"right"` 分支）；
//! - 坐标：`rotation.y = -世界 yaw`（`common/WorldTransform.gd`）⇒ 世界 `right` = `rotation.y` 减小；
//!
//! ⇒ Δθ > 0（右）必须让 `rotation.y` **减小** ⇒ `omega = -yaw_rate_rps`
//! （世界侧 `"velocity"` 分支直接 `clampf(omega, …)`，不再取反）。
//!
//! 到达不归执行器管：nav 判到达 → 叶子写 `None` → `v = omega = 0`。

use std::f32::consts::PI;

use pleiades_base::robot::core::grid::CELL_RESOLUTION;

/// 死区（度）：`|Δθ| ≤ 5°` 只直行，不纠偏。
pub const DEADBAND_DEG: f32 = 5.0;
/// 原地转阈值（度）：`|Δθ| > 20°` 只转不走，两者之间「边走边转」。
pub const SPIN_DEG: f32 = 20.0;

/// 一拍的连续速度指令：`v` 单位 m/s、`omega` 单位 rad/s（**物理量**，与真机 UAV 同口径）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VelocityCmd {
    pub v: f32,
    pub omega: f32,
}

/// 机栈连续速度执行器（无状态：两个速度来自构造参数，位姿由调用方逐拍传入）
pub struct UavExecutor {
    /// `[flight_ctrl].vel_fwd`（m/s）
    vel_fwd: f32,
    /// `[flight_ctrl].yaw_rate_deg` 换算后的角速度（rad/s，恒为正；方向由 `decide` 定符号）
    yaw_rate_rps: f32,
}

impl UavExecutor {
    /// `vel_fwd` 单位 m/s；`yaw_rate_rps` 单位 rad/s（调用方从 `yaw_rate_deg` 换算后传入）
    pub fn new(vel_fwd: f32, yaw_rate_rps: f32) -> Self {
        Self { vel_fwd, yaw_rate_rps }
    }

    /// 纯决策：读下一格 + 位姿 → 返回一拍的 `(v, omega)`（零副作用，不发命令不写状态）
    ///
    /// - `next_cell`：叶子给的下一格（网格坐标），`None` = 无任务/到达 → `v = omega = 0`
    /// - `x`/`y`/`yaw`：当前位置（世界坐标）与航向（弧度）
    pub fn decide(&self, next_cell: Option<(i32, i32)>, x: f32, y: f32, yaw: f32) -> VelocityCmd {
        let Some((gx, gy)) = next_cell else {
            // 无目标 = 停
            return VelocityCmd { v: 0.0, omega: 0.0 };
        };
        let delta = angle_to_target(gx, gy, x, y, yaw);
        let deg = delta.to_degrees().abs();
        if deg <= DEADBAND_DEG {
            // 直行
            VelocityCmd { v: self.vel_fwd, omega: 0.0 }
        } else {
            // 负号见文件头：Δθ > 0（右转）⇒ rotation.y 减小 ⇒ omega < 0
            let omega = if delta > 0.0 { -self.yaw_rate_rps } else { self.yaw_rate_rps };
            if deg <= SPIN_DEG {
                // 边走边转（弧线）
                VelocityCmd { v: self.vel_fwd, omega }
            } else {
                // 原地转
                VelocityCmd { v: 0.0, omega }
            }
        }
    }
}

/// 网格坐标 → 世界坐标（格中心，与 `world_to_grid` 的 floor 语义一致）
fn cell_center(coord: i32) -> f32 {
    (coord as f32 + 0.5) * CELL_RESOLUTION
}

/// 目标方向角差（归一化到 [-π, π]）
fn angle_to_target(gx: i32, gy: i32, x: f32, y: f32, yaw: f32) -> f32 {
    let tx = cell_center(gx);
    let ty = cell_center(gy);
    normalize_angle((ty - y).atan2(tx - x) - yaw)
}

/// 角度归一化到 [-π, π]（等价 Lua `(a + pi) % (2pi) - pi`）
fn normalize_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

// ================================================================ 单测（三档 + 无目标 + 常量 + 物理量透传）

#[cfg(test)]
mod tests {
    use super::*;

    /// 格心（`CELL_RESOLUTION = 0.5`）：不许写死 0.25 / 0.75 这类坐标
    fn center(c: i32) -> f32 {
        (c as f32 + 0.5) * CELL_RESOLUTION
    }

    /// 位姿取 (0,0) 格心、目标格 (1,0) ⇒ 目标方位恰为 +x ⇒ Δθ = −yaw
    fn decide_at_yaw(ex: &UavExecutor, yaw_deg: f32) -> VelocityCmd {
        ex.decide(Some((1, 0)), center(0), center(0), yaw_deg.to_radians())
    }

    #[test]
    fn test_constants_are_spec() {
        // 两个角度是写死的常量（不在 5°/20° 边界上抽签，改由本条单独锁）
        assert_eq!(DEADBAND_DEG, 5.0);
        assert_eq!(SPIN_DEG, 20.0);
    }

    #[test]
    fn test_velocity_passthrough_is_physical() {
        // §四 修订：v 就是 vel_fwd 本身（m/s），omega 就是 yaw_rate 本身（rad/s），不再乘档位
        let ex = UavExecutor::new(0.8, 0.5236);
        let c = decide_at_yaw(&ex, 0.0);
        assert_eq!(c.v, 0.8, "v 必须等于 vel_fwd（m/s）");
        assert_eq!(c.omega, 0.0);
        let arc = decide_at_yaw(&ex, -10.0);
        assert_eq!(arc.v, 0.8);
        assert_eq!(arc.omega, -0.5236, "右转 ⇒ omega 取负，且就是 yaw_rate 原值");
    }

    #[test]
    fn test_deadband_goes_straight() {
        let ex = UavExecutor::new(0.8, 0.5236);
        for yaw in [0.0_f32, 4.0, -4.0] {
            assert_eq!(
                decide_at_yaw(&ex, yaw),
                VelocityCmd { v: 0.8, omega: 0.0 },
                "yaw={yaw}° 应落在死区内直行"
            );
        }
    }

    #[test]
    fn test_arc_right_when_10deg() {
        // Δθ = +10°（右）⇒ v 与 omega 同时非零，且 omega 取负（文件头证据链）
        let ex = UavExecutor::new(0.8, 0.5236);
        assert_eq!(decide_at_yaw(&ex, -10.0), VelocityCmd { v: 0.8, omega: -0.5236 });
    }

    #[test]
    fn test_arc_left_is_positive_omega() {
        // Δθ = −10°（左）⇒ omega 为正（两侧都断言，写反必红）
        let ex = UavExecutor::new(0.8, 0.5236);
        assert_eq!(decide_at_yaw(&ex, 10.0), VelocityCmd { v: 0.8, omega: 0.5236 });
    }

    #[test]
    fn test_spin_in_place_over_20deg() {
        // 两档边界一次锁死：19° 仍「边走边转」，21° 变「原地转」
        let ex = UavExecutor::new(0.8, 0.5236);
        assert_eq!(decide_at_yaw(&ex, -19.0), VelocityCmd { v: 0.8, omega: -0.5236 });
        assert_eq!(decide_at_yaw(&ex, -21.0), VelocityCmd { v: 0.0, omega: -0.5236 });
        // 左转侧同样锁一次（21° 左 = 只转不走、omega 为正）
        assert_eq!(decide_at_yaw(&ex, 21.0), VelocityCmd { v: 0.0, omega: 0.5236 });
    }

    #[test]
    fn test_no_target_is_zero() {
        // 无目标 = 停（本轮不引入任何"保持上一条"的语义）
        let ex = UavExecutor::new(0.8, 0.5236);
        assert_eq!(ex.decide(None, center(0), center(0), 0.0), VelocityCmd { v: 0.0, omega: 0.0 });
        assert_eq!(
            ex.decide(None, center(0), center(0), 1.234_f32.to_radians()),
            VelocityCmd { v: 0.0, omega: 0.0 }
        );
    }
}
