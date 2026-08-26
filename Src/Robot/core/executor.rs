//Presented by KeJi
//Created Date ： 2026-08-26
//Modified Date ： 2026-08-26

//! 决策执行器（Task 22_5 方案二：决策层回退 Rust）
//!
//! 三状态机：`Idle → Turning → Moving → Idle`（循环），纯决策、零副作用。
//! 逻辑等价于原 `car.lua` 的 `on_tick` 与 Godot-Library 的 `executor.rs` 三状态机。
//!
//! 输入：`next_cell`（GoalService 寻路结果）+ 当前位置/航向 + 当前 `ExecuteState`；
//! 输出：`DecisionResult`（新状态 + sub_target + 动作意图），由 main_loop 单点执行。

use std::f32::consts::PI;

use crate::robot::core::state::{DecisionResult, DecisionState, ExecuteState, MotionAction};
use crate::robot::slam::CELL_RESOLUTION;

/// 转向对齐阈值（弧度，5°，原 car.lua `TURN_ALIGN_RAD`）
const TURN_ALIGN_RAD: f32 = 5.0 * PI / 180.0;
/// 直行连续化阈值（弧度，10°，原 car.lua `STRAIGHT_ALIGN_RAD`）
const STRAIGHT_ALIGN_RAD: f32 = 10.0 * PI / 180.0;
/// 到达当前格判定（米，原 car.lua `SUB_TARGET_THRESHOLD_M`）
const SUB_TARGET_THRESHOLD_M: f32 = 0.2;

/// 决策执行器（无状态：决策所需的「当前状态」全部由 `ExecuteState` 传入）
pub struct DecisionExecutor;

impl DecisionExecutor {
    pub fn new() -> Self {
        Self
    }

    /// 纯决策：读状态 + next_cell → 返回 DecisionResult（零副作用，不发命令不写状态）
    ///
    /// - `next_cell`：GoalService 寻路的下一格（网格坐标），None = 无任务/到达/不可达
    /// - `x`/`y`/`yaw`：当前位置（世界坐标）与航向
    /// - `current`：当前执行器状态（state + sub_target），由 main_loop 维护
    pub fn decide(
        &self,
        next_cell: Option<(i32, i32)>,
        x: f32,
        y: f32,
        yaw: f32,
        current: &ExecuteState,
    ) -> DecisionResult {
        match current.state {
            DecisionState::Idle => self.decide_idle(next_cell, x, y, yaw),
            DecisionState::Turning => self.decide_turning(next_cell, x, y, yaw, current),
            DecisionState::Moving => self.decide_moving(next_cell, x, y, yaw, current),
        }
    }

    /// Idle：无任务保持停；有下一格 → 转向对齐 or 前进
    fn decide_idle(
        &self,
        next_cell: Option<(i32, i32)>,
        x: f32,
        y: f32,
        yaw: f32,
    ) -> DecisionResult {
        let Some((gx, gy)) = next_cell else {
            // 无任务/已到达：车已停，不发 stop（命令去重）
            return DecisionResult {
                state: DecisionState::Idle,
                sub_target: None,
                action: None,
            };
        };
        let delta = angle_to_target(gx, gy, x, y, yaw);
        if delta.abs() > TURN_ALIGN_RAD {
            DecisionResult {
                state: DecisionState::Turning,
                sub_target: Some((gx, gy)),
                action: Some(if delta > 0.0 { MotionAction::TurnRight } else { MotionAction::TurnLeft }),
            }
        } else {
            DecisionResult {
                state: DecisionState::Moving,
                sub_target: Some((gx, gy)),
                action: Some(MotionAction::MoveForward),
            }
        }
    }

    /// Turning：无任务停车；目标变了回 Idle；已对齐停车；未对齐保持（不重复发 turn）
    fn decide_turning(
        &self,
        next_cell: Option<(i32, i32)>,
        x: f32,
        y: f32,
        yaw: f32,
        current: &ExecuteState,
    ) -> DecisionResult {
        let Some((gx, gy)) = next_cell else {
            return DecisionResult {
                state: DecisionState::Idle,
                sub_target: None,
                action: Some(MotionAction::Stop),
            };
        };
        // 目标变了（任务/路径变化，含 sub_target 为 None）：停车回 Idle，下 tick 重新决策
        if current.sub_target != Some((gx, gy)) {
            return DecisionResult {
                state: DecisionState::Idle,
                sub_target: Some((gx, gy)),
                action: Some(MotionAction::Stop),
            };
        }
        // 此时 current.sub_target == Some((gx, gy))
        let delta = angle_to_target(gx, gy, x, y, yaw);
        if delta.abs() <= TURN_ALIGN_RAD {
            // 已对齐：停车（回 Idle）
            DecisionResult {
                state: DecisionState::Idle,
                sub_target: current.sub_target,
                action: Some(MotionAction::Stop),
            }
        } else {
            // 未对齐：保持（不重复发 turn）
            DecisionResult {
                state: DecisionState::Turning,
                sub_target: current.sub_target,
                action: None,
            }
        }
    }

    /// Moving：无任务停车；到达当前格前瞻下一格（直行连续化）；未到保持（命令去重）
    fn decide_moving(
        &self,
        next_cell: Option<(i32, i32)>,
        x: f32,
        y: f32,
        yaw: f32,
        current: &ExecuteState,
    ) -> DecisionResult {
        let Some((gx, gy)) = next_cell else {
            return DecisionResult {
                state: DecisionState::Idle,
                sub_target: None,
                action: Some(MotionAction::Stop),
            };
        };
        let Some((sgx, sgy)) = current.sub_target else {
            return DecisionResult {
                state: DecisionState::Idle,
                sub_target: Some((gx, gy)),
                action: Some(MotionAction::Stop),
            };
        };
        let tx = cell_center(sgx);
        let ty = cell_center(sgy);
        let dist = ((x - tx).powi(2) + (y - ty).powi(2)).sqrt();
        if dist < SUB_TARGET_THRESHOLD_M {
            // 到达当前格：前瞻下一格
            let nd = angle_to_target(gx, gy, x, y, yaw);
            if nd.abs() < STRAIGHT_ALIGN_RAD {
                // 直行连续化：不停车，换目标继续走（车在动，不重发 move_forward）
                DecisionResult {
                    state: DecisionState::Moving,
                    sub_target: Some((gx, gy)),
                    action: None,
                }
            } else {
                // 需转向：停车回 Idle（下 tick 转向，天然隔 50ms）
                DecisionResult {
                    state: DecisionState::Idle,
                    sub_target: Some((gx, gy)),
                    action: Some(MotionAction::Stop),
                }
            }
        } else {
            // 未到 0.2m：保持（命令去重）
            DecisionResult {
                state: DecisionState::Moving,
                sub_target: current.sub_target,
                action: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> ExecuteState {
        ExecuteState { state: DecisionState::Idle, sub_target: None }
    }

    #[test]
    fn test_idle_no_task_holds() {
        let ex = DecisionExecutor::new();
        let r = ex.decide(None, 0.25, 0.25, 0.0, &idle());
        assert_eq!(r.state, DecisionState::Idle);
        assert_eq!(r.sub_target, None);
        assert!(r.action.is_none());
    }

    #[test]
    fn test_idle_target_ahead_moves_forward() {
        let ex = DecisionExecutor::new();
        let r = ex.decide(Some((1, 0)), 0.25, 0.25, 0.0, &idle());
        assert_eq!(r.state, DecisionState::Moving);
        assert_eq!(r.sub_target, Some((1, 0)));
        assert_eq!(r.action, Some(MotionAction::MoveForward));
    }

    #[test]
    fn test_idle_target_right_turns_right() {
        let ex = DecisionExecutor::new();
        let r = ex.decide(Some((0, 1)), 0.25, 0.25, 0.0, &idle());
        assert_eq!(r.state, DecisionState::Turning);
        assert_eq!(r.sub_target, Some((0, 1)));
        assert_eq!(r.action, Some(MotionAction::TurnRight));
    }

    #[test]
    fn test_turning_aligned_stops() {
        let ex = DecisionExecutor::new();
        let cur = ExecuteState { state: DecisionState::Turning, sub_target: Some((1, 0)) };
        let r = ex.decide(Some((1, 0)), 0.25, 0.25, 0.0, &cur);
        assert_eq!(r.state, DecisionState::Idle);
        assert_eq!(r.action, Some(MotionAction::Stop));
    }

    #[test]
    fn test_turning_target_changed_stops() {
        let ex = DecisionExecutor::new();
        let cur = ExecuteState { state: DecisionState::Turning, sub_target: Some((1, 0)) };
        let r = ex.decide(Some((2, 0)), 0.25, 0.25, 0.0, &cur);
        assert_eq!(r.state, DecisionState::Idle);
        assert_eq!(r.sub_target, Some((2, 0)));
        assert_eq!(r.action, Some(MotionAction::Stop));
    }

    #[test]
    fn test_moving_reach_straight_continues() {
        let ex = DecisionExecutor::new();
        let cur = ExecuteState { state: DecisionState::Moving, sub_target: Some((1, 0)) };
        // 位置 (0.7, 0.25) 距格 (1,0) 中心 0.05m < 0.2m，已到达；下一格 (2,0) 直行
        let r = ex.decide(Some((2, 0)), 0.7, 0.25, 0.0, &cur);
        assert_eq!(r.state, DecisionState::Moving);
        assert_eq!(r.sub_target, Some((2, 0)));
        assert!(r.action.is_none());
    }

    #[test]
    fn test_moving_reach_turn_stops() {
        let ex = DecisionExecutor::new();
        let cur = ExecuteState { state: DecisionState::Moving, sub_target: Some((1, 0)) };
        // 已到达当前格，下一格 (1,1) 需转向（角度差 > 10°）
        let r = ex.decide(Some((1, 1)), 0.7, 0.25, 0.0, &cur);
        assert_eq!(r.state, DecisionState::Idle);
        assert_eq!(r.sub_target, Some((1, 1)));
        assert_eq!(r.action, Some(MotionAction::Stop));
    }

    #[test]
    fn test_moving_not_reached_holds() {
        let ex = DecisionExecutor::new();
        let cur = ExecuteState { state: DecisionState::Moving, sub_target: Some((1, 0)) };
        // 位置 (0.3, 0.25) 距格 (1,0) 中心 0.45m > 0.2m，未到达
        let r = ex.decide(Some((1, 0)), 0.3, 0.25, 0.0, &cur);
        assert_eq!(r.state, DecisionState::Moving);
        assert_eq!(r.sub_target, Some((1, 0)));
        assert!(r.action.is_none());
    }
}
