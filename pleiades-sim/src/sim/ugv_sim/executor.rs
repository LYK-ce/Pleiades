//Presented by KeJi
//Created Date ： 2026-10-29
//Modified Date ： 2026-10-30

//! 车栈连续档位执行器（task_40 §二）
//!
//! **与 `crate::sim::uav_sim::executor` 同构**：无状态、纯决策，`decide(next_cell, x, y, yaw)` 只返回
//! 一拍的档位；**不写 `ExecuteState`、不发命令**（`ExecuteState` 的写点仍由 `sim_handler` 单点负责）。
//!
//! 与车栈遗留离散三状态机（`crate::sim::executor::DecisionExecutor`）的关系：
//! 那份文件**一行不改**（与 `pleiades-ugv/src/ugv/executor.rs` 逐字相同），Step 1 起 sim 车栈
//! 改走本文件；离散路径保留为 A/B 回退点（见 `sim_handler::Actuator`）。
//!
//! ## 量纲：**档位，不是物理量**
//!
//! - `speed` = 前进档位，**0~100**，取值 = `[chassis] forward_speed`；
//! - `turn` = 带符号转向档位，**−100..+100**，绝对值 = `[chassis] turn_speed`；
//!
//! 百分比 → m/s / rad/s 的换算**只在世界侧** `orion-simulation-world/src/chassis/DiffDrive.gd`
//! 的 `"velocity"` 分支做（`max_speed = 1.5` / `max_turn_rate = 90`）。本文件与 sim 侧**不知道也不用知道**
//! 这两个数，**不引入 m/s / rad/s**（与机栈执行器输出物理量刻意不同 —— 真机本来就是车一套、机一套）。
//!
//! ## 控制律（每拍 50 ms）
//!
//! | 条件 | `speed` | `turn` | 怎么动 |
//! |---|---|---|---|
//! | `next_cell = None` | `0` | `0` | 停（到达不归执行器管） |
//! | `d < PASS_THROUGH_M` 且 \|Δθ\| > 90° | `forward_speed` | `0` | 目标格心在身后且很近 → 直行穿过，等 nav 换格 |
//! | \|Δθ\| ≤ [`DEADBAND_DEG`]（8°） | `forward_speed` | `0` | 直行 |
//! | \|Δθ\| ≤ [`ARC_MAX_DEG`]（12°） | `forward_speed` | ∓`turn_speed` | 边走边转（弧线） |
//! | 否则 | `0` | ∓`turn_speed` | 原地转 |
//!
//! 判定顺序即上表顺序（pass-through 在最前）；`d` 与 `Δθ` 都无副作用。
//!
//! ## `turn` 的符号（本文件最容易写错的一处，证据链四步）
//!
//! 1. `Δθ = normalize(atan2(ty−y, tx−x) − yaw)`，顺时针为**正**（世界 yaw 约定）；
//! 2. `Δθ > 0` ⇒ 目标在右 ⇒ 世界 yaw 必须增大 ⇒ 世界 `rotation.y` 必须**减小**
//!    （`common/WorldTransform.gd`：`YawToGodot(yaw) = -yaw`，即 `rotation.y = -世界 yaw`）；
//! 3. 世界侧 `DiffDrive.gd` 的 `"velocity"` 分支**直接取 `turn` 的符号、不取反**
//!    （`_yaw_target = max_turn_rate · turn%`）；
//! 4. ⇒ **`Δθ > 0`（目标在右）时 `turn < 0`**。
//!
//! 车/机是两份独立的世界脚本，故符号单测**各写一条**（本条证据链由 [`tests::test_symbol_right_turn_is_negative`] 锁住）。

use std::f32::consts::PI;

use pleiades_base::robot::core::grid::CELL_RESOLUTION;

/// 死区（度）：`|Δθ| ≤ 8°` 只直行，不纠偏（§七.3、§九 #8：Step 6 由 5° 上调。
/// 过冲 ≈ ω × 滞后 ≈ 27 °/s × 0.15 s ≈ 4.0°（尖峰 4.8°）< 8°，余量约 2 倍；
/// `turn_speed=30` ⇒ 每拍 2.7° < 8 ✓）
pub const DEADBAND_DEG: f32 = 8.0;
/// 弧线档上界（度）：8°~12° 边走边转，> 12° 原地转（§七.3、§九 #2：死区抬到 8° 后保住 4° 弧线带宽）
pub const ARC_MAX_DEG: f32 = 12.0;
/// 身后直行阈值（米，= 1 格）：目标格心在身后且比这更近 → 直行穿过，不掉头（§八 #3）
pub const PASS_THROUGH_M: f32 = 0.5;

/// 一拍的连续档位指令：`speed` 前进档位 0~100、`turn` 带符号转向档位 −100..+100（**档位**）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VelocityCmd {
    pub speed: u8,
    pub turn: i16,
}

/// 车栈连续档位执行器（无状态：两个档位来自构造参数，位姿由调用方逐拍传入）
pub struct UgvVelocityExecutor {
    /// `[chassis].forward_speed`（0~100 前进档位）
    forward_speed: u8,
    /// `[chassis].turn_speed`（0~100，恒为正；方向由 `decide` 定符号）
    turn_speed: u8,
}

impl UgvVelocityExecutor {
    /// 档位取自 `[chassis]`：`SpeedBinding` 已夹过一次，这里 `min(100)` 是二次兜底
    pub fn new(forward_speed: u8, turn_speed: u8) -> Self {
        Self { forward_speed: forward_speed.min(100), turn_speed: turn_speed.min(100) }
    }

    /// 纯决策：读下一格 + 位姿 → 返回一拍的 `(speed, turn)`（零副作用，不发命令不写状态）
    ///
    /// - `next_cell`：叶子给的下一格（网格坐标），`None` = 无任务/到达 → `(0, 0)`
    /// - `x`/`y`/`yaw`：当前位置（世界坐标）与航向（弧度）
    pub fn decide(&self, next_cell: Option<(i32, i32)>, x: f32, y: f32, yaw: f32) -> VelocityCmd {
        let Some((gx, gy)) = next_cell else {
            // 无目标 = 停（`speed`/`turn` 同时 0 即停车，不必另发 `stop`）
            return VelocityCmd { speed: 0, turn: 0 };
        };
        let tx = cell_center(gx);
        let ty = cell_center(gy);
        let d = ((tx - x).powi(2) + (ty - y).powi(2)).sqrt();
        let delta = normalize_angle((ty - y).atan2(tx - x) - yaw);
        let deg = delta.to_degrees().abs();
        // 车版独有的一档（§4.3）：目标格心在身后且很近 → 直行穿过，等 nav 换格（不掉头 180°）
        if d < PASS_THROUGH_M && deg > 90.0 {
            return VelocityCmd { speed: self.forward_speed, turn: 0 };
        }
        if deg <= DEADBAND_DEG {
            // 直行
            return VelocityCmd { speed: self.forward_speed, turn: 0 };
        }
        // 负号见文件头：Δθ > 0（右转）⇒ rotation.y 减小 ⇒ turn < 0
        let turn = if delta > 0.0 { -(self.turn_speed as i16) } else { self.turn_speed as i16 };
        if deg <= ARC_MAX_DEG {
            // 边走边转（弧线）
            VelocityCmd { speed: self.forward_speed, turn }
        } else {
            // 原地转
            VelocityCmd { speed: 0, turn }
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

// ================================================================ 单测（常量 + 三档 + 身后直行 + 符号 + ±π）

#[cfg(test)]
mod tests {
    use super::*;

    /// 格心（`CELL_RESOLUTION = 0.5`）：不许写死 0.25 / 0.75 这类坐标
    fn center(c: i32) -> f32 {
        (c as f32 + 0.5) * CELL_RESOLUTION
    }

    /// 默认执行器：`[chassis]` 的 30 / 30（§八 #1）
    fn ex() -> UgvVelocityExecutor {
        UgvVelocityExecutor::new(30, 30)
    }

    /// 位姿取 (0,0) 格心、目标格 (1,0) ⇒ 目标方位恰为 +x ⇒ Δθ = −yaw
    fn decide_at_yaw(ex: &UgvVelocityExecutor, yaw_deg: f32) -> VelocityCmd {
        ex.decide(Some((1, 0)), center(0), center(0), yaw_deg.to_radians())
    }

    #[test]
    fn test_constants_are_spec() {
        // §七.3 / §九 #2/#8：Step 6 由 5°/10° 上调，此后锁死
        assert_eq!(DEADBAND_DEG, 8.0);
        assert_eq!(ARC_MAX_DEG, 12.0);
        assert_eq!(PASS_THROUGH_M, 0.5);
    }

    #[test]
    fn test_no_target_is_zero() {
        // 无目标 = 停（不引入任何"保持上一条"的语义）
        let ex = ex();
        assert_eq!(ex.decide(None, center(0), center(0), 0.0), VelocityCmd { speed: 0, turn: 0 });
        assert_eq!(
            ex.decide(None, center(0), center(0), 1.234_f32.to_radians()),
            VelocityCmd { speed: 0, turn: 0 }
        );
    }

    #[test]
    fn test_deadband_goes_straight() {
        let ex = ex();
        for yaw in [0.0_f32, 4.0, -4.0, 7.0, -7.0] {
            assert_eq!(
                decide_at_yaw(&ex, yaw),
                VelocityCmd { speed: 30, turn: 0 },
                "yaw={yaw}° 应落在死区内直行"
            );
        }
    }

    /// §七.6：`7°` 直行、`8.5°` 已跨出死区进弧线档（8° 是含边界的，单测里用 8.5° 避开 f32 压线）
    #[test]
    fn test_deadband_boundary_7_vs_8_5() {
        let ex = ex();
        // 7° < 8°：两侧都直行，turn 必须为 0
        assert_eq!(decide_at_yaw(&ex, 7.0), VelocityCmd { speed: 30, turn: 0 });
        assert_eq!(decide_at_yaw(&ex, -7.0), VelocityCmd { speed: 30, turn: 0 });
        // ±8.5° > 8°：进弧线档 —— 既在走（speed=30）又在转（turn≠0）
        let right = decide_at_yaw(&ex, -8.5);
        assert_eq!(right.speed, 30, "8.5° 弧线档仍要前进");
        assert!(right.turn != 0, "8.5° 已跨出死区，必须带转向");
        assert!(right.turn < 0, "Δθ = +8.5°（右）⇒ turn 必须为负，实际 {}", right.turn);
        let left = decide_at_yaw(&ex, 8.5);
        assert_eq!(left.speed, 30, "8.5° 弧线档仍要前进");
        assert!(left.turn != 0, "8.5° 已跨出死区，必须带转向");
        assert!(left.turn > 0, "Δθ = −8.5°（左）⇒ turn 必须为正，实际 {}", left.turn);
    }

    #[test]
    fn test_arc_right_when_10deg() {
        // Δθ = +10°（右）⇒ 速度与转向同时非零，且 turn 取负（文件头证据链）
        // Step 6：10° 落在 8°~12° 弧线档（旧版落在 5°~10° 弧线档，断言不变）
        assert_eq!(decide_at_yaw(&ex(), -10.0), VelocityCmd { speed: 30, turn: -30 });
    }

    #[test]
    fn test_arc_left_is_positive_turn() {
        // Δθ = −10°（左）⇒ turn 为正（两侧都断言，写反必红）
        assert_eq!(decide_at_yaw(&ex(), 10.0), VelocityCmd { speed: 30, turn: 30 });
    }

    #[test]
    fn test_spin_right_over_12deg() {
        assert_eq!(decide_at_yaw(&ex(), -30.0), VelocityCmd { speed: 0, turn: -30 });
    }

    #[test]
    fn test_spin_left_over_12deg() {
        assert_eq!(decide_at_yaw(&ex(), 30.0), VelocityCmd { speed: 0, turn: 30 });
    }

    #[test]
    fn test_arc_boundary_12_vs_12_5() {
        // 两档边界一次锁死：12° 仍「边走边转」（含边界），12.5° 变「原地转」
        let ex = ex();
        assert_eq!(decide_at_yaw(&ex, -12.0), VelocityCmd { speed: 30, turn: -30 });
        assert_eq!(decide_at_yaw(&ex, 12.0), VelocityCmd { speed: 30, turn: 30 });
        assert_eq!(decide_at_yaw(&ex, -12.5), VelocityCmd { speed: 0, turn: -30 });
        assert_eq!(decide_at_yaw(&ex, 12.5), VelocityCmd { speed: 0, turn: 30 });
    }

    #[test]
    fn test_pass_through_behind_cell() {
        // 目标格 (0,0) 在正后方（yaw=0），阈值两侧各一条
        let ex = ex();
        let y = center(0);
        // d = 0.45 < PASS_THROUGH_M ⇒ 直行穿过（不掉头）
        let near = ex.decide(Some((0, 0)), center(0) + (PASS_THROUGH_M - 0.05), y, 0.0);
        assert_eq!(near, VelocityCmd { speed: 30, turn: 0 }, "身后且很近 ⇒ 直行穿过");
        // d = 0.80 > PASS_THROUGH_M ⇒ 不再直行，按「原地转」处理
        let far = ex.decide(Some((0, 0)), center(0) + (PASS_THROUGH_M + 0.30), y, 0.0);
        assert_eq!(far, VelocityCmd { speed: 0, turn: 30 }, "身后但够远 ⇒ 正常掉头（原地转）");
    }

    #[test]
    fn test_delta_pi_is_normalized() {
        // 目标恰在正后方 ⇒ atan2(0, −0.60) = π，`normalize_angle` 必须归一成 **−π**
        // （不是 +π、不是 0）：|Δθ| = 180 > 90 且 d > PASS_THROUGH_M ⇒ 原地转，turn 取正
        let c = ex().decide(Some((0, 0)), center(0) + 0.60, center(0), 0.0);
        assert_eq!(c, VelocityCmd { speed: 0, turn: 30 });
        assert_eq!(normalize_angle(PI), -PI, "±π 归一化必须落到 −π");
        assert!(c.turn.abs() <= 100, "归一化后档位必须有限（非 NaN/非溢出）");
    }

    #[test]
    fn test_speed_turn_come_from_constructor() {
        // 档位原值透传，不乘任何系数（换算只在世界侧）
        let ex = UgvVelocityExecutor::new(45, 60);
        assert_eq!(decide_at_yaw(&ex, -10.0), VelocityCmd { speed: 45, turn: -60 });
        assert_eq!(decide_at_yaw(&ex, -30.0), VelocityCmd { speed: 0, turn: -60 });
        // 超过 100 的档位在构造时夹到 100（`SpeedBinding` 已夹一次，这是二次兜底）
        let capped = UgvVelocityExecutor::new(250, 250);
        assert_eq!(decide_at_yaw(&capped, -30.0), VelocityCmd { speed: 0, turn: -100 });
    }

    #[test]
    fn test_zero_turn_speed_cannot_steer() {
        // turn_speed = 0 ⇒ 原地转档退化为停，不产生「转但不动」的歧义
        let ex = UgvVelocityExecutor::new(30, 0);
        assert_eq!(decide_at_yaw(&ex, -30.0), VelocityCmd { speed: 0, turn: 0 });
    }

    #[test]
    fn test_symbol_right_turn_is_negative() {
        // 符号单测（§二.3 证据链）：Δθ > 0（目标在右）⇒ turn < 0；
        // 因为 `common/WorldTransform.gd` 的 `YawToGodot = −world_yaw` ⇒ 世界 `rotation.y` 减小 ⇒
        // 世界 `"velocity"` 分支直接取 turn 符号、不取反。
        let ex = ex();
        let right = decide_at_yaw(&ex, -20.0);
        assert!(right.turn < 0, "Δθ > 0（右）⇒ turn 必须为负，实际 {}", right.turn);
        let left = decide_at_yaw(&ex, 20.0);
        assert!(left.turn > 0, "Δθ < 0（左）⇒ turn 必须为正，实际 {}", left.turn);
    }
}
