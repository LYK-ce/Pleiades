//Presented by KeJi
//Created Date ： 2026-10-29
//Modified Date ： 2026-11-03

//! 机栈连续速度执行器（task_39 §四.2a，task_44 §1.3 起与车栈同形）
//!
//! **与车栈 `crate::sim::ugv_sim::executor` 同形**：同一个 `VelocityCmd { v, omega }`、
//! 同一套 `decide(target, x, y, yaw)` 签名；差异只在**档位数**（机三档 / 车五档）与**阈值**
//! （机 5°/20° / 车 8°/12°）。真机本来就是「车一套、机一套」，sim 侧保持两份独立文件。
//!
//! **口径与真机 UAV 一致**（task_39 §四 修订，2026-10-29）：输出的是**物理量**
//! —— `v` 单位 m/s、`omega` 单位 rad/s；两个值来自 `[flight_ctrl]` 的 `vel_fwd` /
//! `yaw_rate_deg`（与 `pleiades-uav` 同名同单位），不借用车的 `[chassis]`。
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
//! - 决策侧：`delta > 0` ⇒ 目标在右；
//! - 世界侧：`rotation.y = -世界 yaw`（`common/WorldTransform.gd`）⇒ 右转 = `rotation.y` 减小；
//! - 世界 `"velocity"` 分支直接 `clampf(omega, …)`，不再取反（`Quadrotor.gd`）；
//!
//! ⇒ Δθ > 0（右）必须让 `rotation.y` **减小** ⇒ `omega = -yaw_rate_rps`。
//! 车栈是同一套证据链（四步逐条写在 `ugv_sim/executor.rs` 文件头），两个底盘脚本各写一条符号单测。
//!
//! 到达不归执行器管：nav 判到达 → 叶子写 `None` → `v = omega = 0`。
//!
//! ## 目标类型（task_45 §一 #1/#2）
//!
//! 唯一入口 [`decide`] 收 `Option<`[`MotionTarget`]`>`：`Cell` → 取格心（Goto / 巡逻），
//! `Point` → **原值直接用**（Follow 给的人的世界坐标，不经格心、不量化）。进门消解一次，
//! 其后 `delta` / 三档判断 / `omega` 符号链**一行不改**。

use std::f32::consts::PI;

use pleiades_base::robot::core::behavior_tree::MotionTarget;
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

    /// 纯决策：读本拍目标 + 位姿 → 返回一拍的 `(v, omega)`（零副作用，不发命令不写状态）
    ///
    /// - `target`：叶子给的目标（格或点），`None` = 无任务/到达 → `v = omega = 0`
    /// - `x`/`y`/`yaw`：当前位置（世界坐标）与航向（弧度）
    pub fn decide(&self, target: Option<MotionTarget>, x: f32, y: f32, yaw: f32) -> VelocityCmd {
        // 进门只做一次类型消解：格 → 格心，点 → 原值
        let Some((tx, ty)) = target.map(|t| match t {
            MotionTarget::Cell((gx, gy)) => (cell_center(gx), cell_center(gy)),
            MotionTarget::Point((px, py)) => (px, py),
        }) else {
            // 无目标 = 停
            return VelocityCmd { v: 0.0, omega: 0.0 };
        };
        let delta = angle_to_target(tx, ty, x, y, yaw);
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

/// 目标方向角差（归一化到 [-π, π]）：入参是**世界坐标**（格已在上游消解成格心）
fn angle_to_target(tx: f32, ty: f32, x: f32, y: f32, yaw: f32) -> f32 {
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
        ex.decide(Some(MotionTarget::Cell((1, 0))), center(0), center(0), yaw_deg.to_radians())
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

    /// task_45 §一 #1/#2：`Point` 走**世界坐标原值**，不经格心（Follow 的核心诉求）
    ///
    /// 目标点 `(0.25, 1.25)` 取的是**格角**（不是任何格心）——若被当成格心处理，
    /// 目标会跑到 `(0.75, 0.75)`，下面两条断言必红。
    #[test]
    fn test_point_target_uses_raw_coords() {
        let ex = UavExecutor::new(0.8, 0.5236);
        let me = (0.25, 0.25);
        let pt = MotionTarget::Point((0.25, 1.25));
        // 目标在正前方（+y）且 yaw = +90° ⇒ Δθ = 0 ⇒ 死区直行
        assert_eq!(
            ex.decide(Some(pt), me.0, me.1, PI / 2.0),
            VelocityCmd { v: 0.8, omega: 0.0 },
            "点必须原值直接用：yaw 对齐目标方向就直行（格心化会让 Δθ 偏 45°）"
        );
        // 同一点、yaw = 0 ⇒ Δθ = +90° > SPIN_DEG（20°）⇒ 原地转，Δθ > 0 ⇒ omega 取负
        assert_eq!(
            ex.decide(Some(pt), me.0, me.1, 0.0),
            VelocityCmd { v: 0.0, omega: -0.5236 },
            "|Δθ| = 90° > 20° ⇒ 原地转，ω 符号见文件头证据链"
        );
    }
}
