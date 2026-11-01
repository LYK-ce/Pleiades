//Presented by KeJi
//Created Date ： 2026-10-29
//Modified Date ： 2026-11-01

//! 车栈连续速度执行器（task_44 §1.4）
//!
//! **与 `crate::sim::uav_sim::executor` 同形**：无状态、纯决策，`decide(next_cell, x, y, yaw)`
//! 只返回一拍的 `VelocityCmd { v, omega }`；**不写 `ExecuteState`、不发命令**
//! （`ExecuteState` 的写点仍由 `sim_handler` 单点负责）。与机栈的差别只有「档位数」与「阈值」：
//! 车五档（含身后直行）、机三档。
//!
//! ## 量纲：**物理量**
//!
//! - `v` 单位 **m/s**、`omega` 单位 **rad/s**（与真机同一口径，§1.2 #2/#11）；
//! - 两个值都来自构造参数（`[chassis] cruise_speed` / `cruise_yaw_rate`，缺省 0.45 / 0.5）；
//! - 执行器内**不做任何比例换算、不读百分比**（§1.2 #11）。世界侧 `DiffDrive.gd` 直接收
//!   这两个物理量，`max_speed` / `max_turn_rate` 只作 clamp 上限。
//!
//! ## 控制律（每拍 50 ms，判断顺序即表顺序）
//!
//! | 条件 | `v` | `omega` |
//! |---|---|---|
//! | `next_cell = None` | `0` | `0` |
//! | `d < PASS_THROUGH_M` 且 \|Δθ\| > 90° | `cruise_speed` | `0` |
//! | \|Δθ\| ≤ [`DEADBAND_DEG`]（8°） | `cruise_speed` | `0` |
//! | 8° < \|Δθ\| ≤ [`ARC_MAX_DEG`]（12°） | `cruise_speed` | ∓`cruise_yaw_rate` |
//! | \|Δθ\| > 12° | `0` | ∓`cruise_yaw_rate` |
//!
//! 三个阈值（[`DEADBAND_DEG`] / [`ARC_MAX_DEG`] / [`PASS_THROUGH_M`]）**保持不变**；
//! 量级是定值：不随位姿、距离、剩余路程变化。
//!
//! ## `omega` 的符号（本文件最容易写错的一处，证据链四步）
//!
//! 1. `Δθ = normalize(atan2(ty−y, tx−x) − yaw)`，顺时针为**正**（世界 yaw 约定）；
//! 2. `Δθ > 0` ⇒ 目标在右 ⇒ 世界 yaw 必须增大 ⇒ 世界 `rotation.y` 必须**减小**
//!    （`common/WorldTransform.gd`：`YawToGodot(yaw) = -yaw`，即 `rotation.y = -世界 yaw`）；
//! 3. 世界侧 `DiffDrive.gd` 的 `"velocity"` 分支**直接取 `omega` 的符号、不取反**
//!    （`_yaw_target = clampf(omega, ±max_turn_rate)`）；
//! 4. ⇒ **`Δθ > 0`（目标在右）时 `omega < 0`**。
//!
//! `omega > 0` = 左转（逆时针），与 §1.2 #2 一致；本条证据链由
//! [`tests::test_symbol_right_turn_is_negative`] 锁住。
//!
//! 到达不归执行器管：nav 判到达 → 叶子写 `None` → `v = omega = 0`。

use std::f32::consts::PI;

use pleiades_base::robot::core::grid::CELL_RESOLUTION;

/// 死区（度）：`|Δθ| ≤ 8°` 只直行，不纠偏（task_31 §七.3 / §九 #8）。
/// 过冲 ≈ ω × 滞后 ≈ 0.5 rad/s × 0.15 s ≈ 4.3° < 8°，余量约 2 倍。
pub const DEADBAND_DEG: f32 = 8.0;
/// 弧线档上界（度）：8°~12° 边走边转，> 12° 原地转（保留 4° 弧线带宽）
pub const ARC_MAX_DEG: f32 = 12.0;
/// 身后直行阈值（米，= 1 格）：目标格心在身后且比这更近 → 直行穿过，不掉头
pub const PASS_THROUGH_M: f32 = 0.5;
/// 二次兜底上限 = 世界 `DiffDrive.gd` 的 `max_speed` 缺省（1.5 m/s）
pub const MAX_CRUISE_SPEED: f32 = 1.5;
/// 二次兜底上限 = 世界 `DiffDrive.gd` 的 `max_turn_rate` 缺省（90 °/s = π/2 rad/s）
pub const MAX_CRUISE_YAW_RATE: f32 = std::f32::consts::FRAC_PI_2;

/// 一拍的连续速度指令：`v` 单位 m/s、`omega` 单位 rad/s（**物理量**）
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VelocityCmd {
    pub v: f32,
    pub omega: f32,
}

/// 车栈连续速度执行器（无状态：两个速度来自构造参数，位姿由调用方逐拍传入）
pub struct UgvVelocityExecutor {
    /// 巡航线速度（m/s，来自 `[chassis].cruise_speed`，已夹过上限）
    cruise_speed: f32,
    /// 巡航偏航角速度**绝对值**（rad/s，恒为正；方向由 `decide` 定符号）
    cruise_yaw_rate: f32,
}

impl UgvVelocityExecutor {
    /// 两个值取 `[chassis]`（`SpeedBinding` 已夹过一次，这里 `clamp` 是二次兜底）；
    /// **只夹，不换算、不读百分比**（§1.2 #11）。
    pub fn new(cruise_speed: f32, cruise_yaw_rate: f32) -> Self {
        Self {
            cruise_speed: cruise_speed.clamp(0.0, MAX_CRUISE_SPEED),
            cruise_yaw_rate: cruise_yaw_rate.clamp(0.0, MAX_CRUISE_YAW_RATE),
        }
    }

    /// 纯决策：读下一格 + 位姿 → 返回一拍的 `(v, omega)`（零副作用，不发命令不写状态）
    ///
    /// - `next_cell`：叶子给的下一格（网格坐标），`None` = 无任务/到达 → `v = omega = 0`
    /// - `x`/`y`/`yaw`：当前位置（世界坐标）与航向（弧度）
    pub fn decide(&self, next_cell: Option<(i32, i32)>, x: f32, y: f32, yaw: f32) -> VelocityCmd {
        let Some((gx, gy)) = next_cell else {
            // 无目标 = 停（`v`/`omega` 同时 0 即停车，不必另发 `stop`）
            return VelocityCmd { v: 0.0, omega: 0.0 };
        };
        let tx = cell_center(gx);
        let ty = cell_center(gy);
        let d = ((tx - x).powi(2) + (ty - y).powi(2)).sqrt();
        let delta = normalize_angle((ty - y).atan2(tx - x) - yaw);
        let deg = delta.to_degrees().abs();
        // 车版独有的一档：目标格心在身后且很近 → 直行穿过，等 nav 换格（不掉头 180°）
        if d < PASS_THROUGH_M && deg > 90.0 {
            return VelocityCmd { v: self.cruise_speed, omega: 0.0 };
        }
        if deg <= DEADBAND_DEG {
            // 直行
            return VelocityCmd { v: self.cruise_speed, omega: 0.0 };
        }
        // 负号见文件头：Δθ > 0（右转）⇒ rotation.y 减小 ⇒ omega < 0
        let omega = if delta > 0.0 { -self.cruise_yaw_rate } else { self.cruise_yaw_rate };
        if deg <= ARC_MAX_DEG {
            // 边走边转（弧线）
            VelocityCmd { v: self.cruise_speed, omega }
        } else {
            // 原地转
            VelocityCmd { v: 0.0, omega }
        }
    }
}

/// 网格坐标 → 世界坐标（格中心，与 `world_to_grid` 的 floor 语义一致）
fn cell_center(coord: i32) -> f32 {
    (coord as f32 + 0.5) * CELL_RESOLUTION
}

/// 角度归一化到 [-π, π]（等价 Lua `(a + pi) % (2pi) - pi`）
fn normalize_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(2.0 * PI) - PI
}

// ================================================================ 单测（常量 + 五档 + 身后直行 + 符号 + ±π）

#[cfg(test)]
mod tests {
    use super::*;

    /// 格心（`CELL_RESOLUTION = 0.5`）：不许写死 0.25 / 0.75 这类坐标
    fn center(c: i32) -> f32 {
        (c as f32 + 0.5) * CELL_RESOLUTION
    }

    /// 默认执行器：§1.4 的定值 0.45 m/s / 0.5 rad/s
    fn ex() -> UgvVelocityExecutor {
        UgvVelocityExecutor::new(0.45, 0.5)
    }

    /// 位姿取 (0,0) 格心、目标格 (1,0) ⇒ 目标方位恰为 +x ⇒ Δθ = −yaw
    fn decide_at_yaw(ex: &UgvVelocityExecutor, yaw_deg: f32) -> VelocityCmd {
        ex.decide(Some((1, 0)), center(0), center(0), yaw_deg.to_radians())
    }

    #[test]
    fn test_constants_are_spec() {
        // 三个阈值 §1.4 明确「保持不变」；缺省量级由构造参数给（本文件不写死 0.45/0.5）
        assert_eq!(DEADBAND_DEG, 8.0);
        assert_eq!(ARC_MAX_DEG, 12.0);
        assert_eq!(PASS_THROUGH_M, 0.5);
    }

    #[test]
    fn test_no_target_is_zero() {
        // 无目标 = 停（不引入任何"保持上一条"的语义）
        let ex = ex();
        assert_eq!(ex.decide(None, center(0), center(0), 0.0), VelocityCmd { v: 0.0, omega: 0.0 });
        assert_eq!(
            ex.decide(None, center(0), center(0), 1.234_f32.to_radians()),
            VelocityCmd { v: 0.0, omega: 0.0 }
        );
    }

    #[test]
    fn test_deadband_goes_straight() {
        let ex = ex();
        for yaw in [0.0_f32, 4.0, -4.0, 7.0, -7.0] {
            assert_eq!(
                decide_at_yaw(&ex, yaw),
                VelocityCmd { v: 0.45, omega: 0.0 },
                "yaw={yaw}° 应落在死区内直行"
            );
        }
    }

    /// `7°` 直行、`8.5°` 已跨出死区进弧线档（8° 是含边界的，单测里用 8.5° 避开 f32 压线）
    #[test]
    fn test_deadband_boundary_7_vs_8_5() {
        let ex = ex();
        // 7° < 8°：两侧都直行，omega 必须为 0
        assert_eq!(decide_at_yaw(&ex, 7.0), VelocityCmd { v: 0.45, omega: 0.0 });
        assert_eq!(decide_at_yaw(&ex, -7.0), VelocityCmd { v: 0.45, omega: 0.0 });
        // ±8.5° > 8°：进弧线档 —— 既在走（v=0.45）又在转（omega≠0）
        let right = decide_at_yaw(&ex, -8.5);
        assert_eq!(right.v, 0.45, "8.5° 弧线档仍要前进");
        assert!(right.omega != 0.0, "8.5° 已跨出死区，必须带转向");
        assert!(right.omega < 0.0, "Δθ = +8.5°（右）⇒ omega 必须为负，实际 {}", right.omega);
        let left = decide_at_yaw(&ex, 8.5);
        assert_eq!(left.v, 0.45, "8.5° 弧线档仍要前进");
        assert!(left.omega != 0.0, "8.5° 已跨出死区，必须带转向");
        assert!(left.omega > 0.0, "Δθ = −8.5°（左）⇒ omega 必须为正，实际 {}", left.omega);
    }

    #[test]
    fn test_arc_right_when_10deg() {
        // Δθ = +10°（右）⇒ 速度与转向同时非零，且 omega 取负（文件头证据链）
        assert_eq!(decide_at_yaw(&ex(), -10.0), VelocityCmd { v: 0.45, omega: -0.5 });
    }

    #[test]
    fn test_arc_left_is_positive_omega() {
        // Δθ = −10°（左）⇒ omega 为正（两侧都断言，写反必红）
        assert_eq!(decide_at_yaw(&ex(), 10.0), VelocityCmd { v: 0.45, omega: 0.5 });
    }

    #[test]
    fn test_spin_right_over_12deg() {
        assert_eq!(decide_at_yaw(&ex(), -30.0), VelocityCmd { v: 0.0, omega: -0.5 });
    }

    #[test]
    fn test_spin_left_over_12deg() {
        assert_eq!(decide_at_yaw(&ex(), 30.0), VelocityCmd { v: 0.0, omega: 0.5 });
    }

    #[test]
    fn test_arc_boundary_12_vs_12_5() {
        // 两档边界一次锁死：12° 仍「边走边转」（含边界），12.5° 变「原地转」
        let ex = ex();
        assert_eq!(decide_at_yaw(&ex, -12.0), VelocityCmd { v: 0.45, omega: -0.5 });
        assert_eq!(decide_at_yaw(&ex, 12.0), VelocityCmd { v: 0.45, omega: 0.5 });
        assert_eq!(decide_at_yaw(&ex, -12.5), VelocityCmd { v: 0.0, omega: -0.5 });
        assert_eq!(decide_at_yaw(&ex, 12.5), VelocityCmd { v: 0.0, omega: 0.5 });
    }

    #[test]
    fn test_pass_through_behind_cell() {
        // 目标格 (0,0) 在正后方（yaw=0），阈值两侧各一条
        let ex = ex();
        let y = center(0);
        // d = 0.45 < PASS_THROUGH_M ⇒ 直行穿过（不掉头）
        let near = ex.decide(Some((0, 0)), center(0) + (PASS_THROUGH_M - 0.05), y, 0.0);
        assert_eq!(near, VelocityCmd { v: 0.45, omega: 0.0 }, "身后且很近 ⇒ 直行穿过");
        // d = 0.80 > PASS_THROUGH_M ⇒ 不再直行，按「原地转」处理
        let far = ex.decide(Some((0, 0)), center(0) + (PASS_THROUGH_M + 0.30), y, 0.0);
        assert_eq!(far, VelocityCmd { v: 0.0, omega: 0.5 }, "身后但够远 ⇒ 正常掉头（原地转）");
    }

    #[test]
    fn test_delta_pi_is_normalized() {
        // 目标恰在正后方 ⇒ atan2(0, −0.60) = π，`normalize_angle` 必须归一成 **−π**
        // （不是 +π、不是 0）：|Δθ| = 180 > 90 且 d > PASS_THROUGH_M ⇒ 原地转，omega 取正
        let c = ex().decide(Some((0, 0)), center(0) + 0.60, center(0), 0.0);
        assert_eq!(c, VelocityCmd { v: 0.0, omega: 0.5 });
        assert_eq!(normalize_angle(PI), -PI, "±π 归一化必须落到 −π");
        assert!(c.omega.abs() <= MAX_CRUISE_YAW_RATE, "归一化后角速度必须有限（非 NaN）");
    }

    #[test]
    fn test_spec_values_are_fixed() {
        // §1.4 钉死：直行档 v = 0.45；弧线 / 原地档 |omega| = 0.5（两个轴都是定值，
        // 不随位姿、距离、剩余路程变化）
        let ex = ex();
        for yaw in [0.0_f32, 10.0, -10.0, 30.0, -30.0] {
            let c = decide_at_yaw(&ex, yaw);
            if c.v != 0.0 {
                assert_eq!(c.v, 0.45, "yaw={yaw}° 的 v 必须是定值 0.45 m/s");
            }
            if c.omega != 0.0 {
                assert_eq!(c.omega.abs(), 0.5, "yaw={yaw}° 的 |omega| 必须是定值 0.5 rad/s");
            }
        }
    }

    #[test]
    fn test_cruise_comes_from_constructor() {
        // 两个值原值透传，不乘任何系数、不做百分比换算（§1.2 #11）
        let ex = UgvVelocityExecutor::new(0.6, 0.7);
        assert_eq!(decide_at_yaw(&ex, -10.0), VelocityCmd { v: 0.6, omega: -0.7 });
        assert_eq!(decide_at_yaw(&ex, -30.0), VelocityCmd { v: 0.0, omega: -0.7 });
        // 超过上限的值在构造时夹到上限（`SpeedBinding` 已夹一次，这是二次兜底）
        let capped = UgvVelocityExecutor::new(99.0, 99.0);
        assert_eq!(
            decide_at_yaw(&capped, -10.0),
            VelocityCmd { v: MAX_CRUISE_SPEED, omega: -MAX_CRUISE_YAW_RATE }
        );
    }

    #[test]
    fn test_zero_cruise_yaw_rate_cannot_steer() {
        // cruise_yaw_rate = 0 ⇒ 原地转档退化为停，不产生「转但不动」的歧义
        let ex = UgvVelocityExecutor::new(0.45, 0.0);
        assert_eq!(decide_at_yaw(&ex, -30.0), VelocityCmd { v: 0.0, omega: 0.0 });
    }

    #[test]
    fn test_symbol_right_turn_is_negative() {
        // 符号单测（文件头证据链）：Δθ > 0（目标在右）⇒ omega < 0；
        // 因为 `common/WorldTransform.gd` 的 `YawToGodot = −world_yaw` ⇒ 世界 `rotation.y` 减小 ⇒
        // 世界 `"velocity"` 分支直接取 omega 符号、不取反。
        let ex = ex();
        let right = decide_at_yaw(&ex, -20.0);
        assert!(right.omega < 0.0, "Δθ > 0（右）⇒ omega 必须为负，实际 {}", right.omega);
        let left = decide_at_yaw(&ex, 20.0);
        assert!(left.omega > 0.0, "Δθ < 0（左）⇒ omega 必须为正，实际 {}", left.omega);
    }
}
