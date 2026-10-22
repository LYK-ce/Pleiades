//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! **唯一翻译入口**：`MotionAction`（真机语义）→ 世界 `motion_cmd` 的 `(action, speed)`
//!
//! 本文件就是 task_31 §六.2.1 的对照表的**全部内容**，一行都不许自行发挥：
//!
//! | `MotionAction` | 真机（STM32 `apply_action`） | 世界 `action` | 世界 `speed` |
//! |---|---|---|---|
//! | `MoveForward`  | `forward(forward_speed)`  / `Forward=1`  | `"forward"`  | `forward_speed` |
//! | `MoveBackward` | `backward(forward_speed)` / `Backward=2` | `"backward"` | `forward_speed` |
//! | `TurnLeft`     | `spin_left(turn_speed)`   / `SpinLeft=5` | `"left"`     | `turn_speed` |
//! | `TurnRight`    | `spin_right(turn_speed)`  / `SpinRight=6`| `"right"`    | `turn_speed` |
//! | `Stop`         | `stop()`                  / `Stop=0`     | `"stop"`     | 恒 `0` |
//!
//! 两条不可动的结论（§六.2.2 / §六.2.3）：
//!
//! 1. **符号不取反**：真机转向是 `SpinLeft`/`SpinRight`（原地偏航），世界的 `left`/`right`
//!    在 `DiffDrive.gd` 里也是「`speed=0` + 纯 `yaw_target`」= 原地偏航；三方推导
//!    （世界 `right` → `rotation.y` 减小 → 世界 `yaw` 增大 → 俯视顺时针 = 右转）与决策侧
//!    `delta > TURN_ALIGN_RAD → TurnRight` 一致，所以 `TurnLeft ↔ "left"`、`TurnRight ↔ "right"`。
//! 2. **单位直接透传**：真机 `speed` 是 `clamp(0,100)` 的 i16 百分比，世界 `speed` 是
//!    0~100 百分比（世界侧 `clamp(0,100)/100 × max_speed`），同一量纲，**不做 m/s 换算**。

use pleiades_base::robot::core::state::MotionAction;

use crate::config::ChassisConfig;

/// `clamp(0,100)` 之后的速度绑定（`forward` ≈ 真机 `forward_speed`，`turn` ≈ 真机 `turn_speed`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedBinding {
    /// 前进/后退档位（0~100）
    pub forward: u8,
    /// 转向档位（0~100）；UAV 的 `up`/`down` 也用它（§六.2.6 G3 的近似）
    pub turn: u8,
}

impl SpeedBinding {
    /// 缺省前进速度（与 pleiades-ugv 的 `[chassis].forward_speed` 默认一致）
    pub const DEFAULT_FORWARD: u8 = 30;
    /// 缺省转向速度（与 pleiades-ugv 的 `[chassis].turn_speed` 默认一致）
    pub const DEFAULT_TURN: u8 = 10;

    /// 缺省 30/10；负值 → 0、>100 → 100（与真机 speed 的 `clamp(0,100)` 一致）
    pub fn From_Config(cfg: Option<&ChassisConfig>) -> Self {
        let pick = |v: Option<i32>, default: u8| -> u8 {
            match v {
                Some(x) => x.clamp(0, 100) as u8,
                None => default,
            }
        };
        match cfg {
            Some(c) => Self {
                forward: pick(c.forward_speed, Self::DEFAULT_FORWARD),
                turn: pick(c.turn_speed, Self::DEFAULT_TURN),
            },
            None => Self { forward: Self::DEFAULT_FORWARD, turn: Self::DEFAULT_TURN },
        }
    }
}

impl Default for SpeedBinding {
    fn default() -> Self {
        Self { forward: Self::DEFAULT_FORWARD, turn: Self::DEFAULT_TURN }
    }
}

/// 世界 wire 上的一个动作（`action` 只能取自本表的 4+1 个字面量，故是 `&'static str`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionWire {
    pub action: &'static str,
    pub speed: u8,
}

/// 真机动作 → 世界 `motion_cmd` 的**唯一**翻译点（§六.2.1 的表格就是本函数的全部内容）
pub fn Action_To_Wire(action: MotionAction, speed: &SpeedBinding) -> MotionWire {
    match action {
        MotionAction::MoveForward => MotionWire { action: "forward", speed: speed.forward },
        MotionAction::MoveBackward => MotionWire { action: "backward", speed: speed.forward },
        MotionAction::TurnLeft => MotionWire { action: "left", speed: speed.turn },
        MotionAction::TurnRight => MotionWire { action: "right", speed: speed.turn },
        // 停车恒 0：世界 `stop` 本身就是「speed=0 + yaw=0」，带档位没有意义
        MotionAction::Stop => MotionWire { action: "stop", speed: 0 },
    }
}

// ================================================================ 单测（硬验收 2：锁表）

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺省绑定（30/10）
    fn binding() -> SpeedBinding {
        SpeedBinding { forward: 30, turn: 10 }
    }

    #[test]
    fn test_action_to_wire_table() {
        // §六.2.1 的五行逐项断言（动作 → (action, speed)）
        let b = binding();
        let expect = [
            (MotionAction::MoveForward, MotionWire { action: "forward", speed: 30 }),
            (MotionAction::MoveBackward, MotionWire { action: "backward", speed: 30 }),
            (MotionAction::TurnLeft, MotionWire { action: "left", speed: 10 }),
            (MotionAction::TurnRight, MotionWire { action: "right", speed: 10 }),
            (MotionAction::Stop, MotionWire { action: "stop", speed: 0 }),
        ];
        for (action, want) in expect {
            let got = Action_To_Wire(action, &b);
            assert_eq!(got.action, want.action, "{action:?} 的 action 错");
            assert_eq!(got.speed, want.speed, "{action:?} 的 speed 错");
            assert_eq!(got, want);
        }
    }

    #[test]
    fn test_action_to_wire_uses_binding() {
        // 速度必须来自绑定（配置改了就走配置值），不是写死 30/10
        let b = SpeedBinding { forward: 77, turn: 5 };
        assert_eq!(Action_To_Wire(MotionAction::MoveForward, &b), MotionWire { action: "forward", speed: 77 });
        assert_eq!(Action_To_Wire(MotionAction::MoveBackward, &b), MotionWire { action: "backward", speed: 77 });
        assert_eq!(Action_To_Wire(MotionAction::TurnLeft, &b), MotionWire { action: "left", speed: 5 });
        assert_eq!(Action_To_Wire(MotionAction::TurnRight, &b), MotionWire { action: "right", speed: 5 });
    }

    #[test]
    fn test_action_to_wire_stop_speed_always_zero() {
        // Stop 的 speed 与绑定无关：恒 0（任何组合）
        let combos = [
            SpeedBinding { forward: 0, turn: 0 },
            SpeedBinding { forward: 100, turn: 100 },
            SpeedBinding { forward: 1, turn: 99 },
            SpeedBinding { forward: 30, turn: 10 },
        ];
        for b in combos {
            let w = Action_To_Wire(MotionAction::Stop, &b);
            assert_eq!(w.action, "stop");
            assert_eq!(w.speed, 0, "Stop 的 speed 必须恒 0（binding={b:?}）");
        }
    }

    #[test]
    fn test_speed_binding_from_config_defaults() {
        // 无段 / 空段 → 30/10（与 pleiades-ugv 的 [chassis] 默认一致）
        assert_eq!(SpeedBinding::From_Config(None), binding());
        assert_eq!(SpeedBinding::From_Config(Some(&ChassisConfig::default())), binding());
        assert_eq!(SpeedBinding::default(), binding());
        // 只写一个键：另一个仍取默认
        let one = ChassisConfig { enabled: None, forward_speed: Some(55), turn_speed: None };
        assert_eq!(SpeedBinding::From_Config(Some(&one)), SpeedBinding { forward: 55, turn: 10 });
    }

    #[test]
    fn test_speed_binding_clamps() {
        let mk = |f: Option<i32>, t: Option<i32>| {
            SpeedBinding::From_Config(Some(&ChassisConfig { enabled: None, forward_speed: f, turn_speed: t }))
        };
        // 负值 → 0；>100 → 100；i32 两端也不 panic/溢出
        assert_eq!(mk(Some(-1), Some(-1)), SpeedBinding { forward: 0, turn: 0 });
        assert_eq!(mk(Some(101), Some(101)), SpeedBinding { forward: 100, turn: 100 });
        assert_eq!(mk(Some(i32::MIN), Some(i32::MAX)), SpeedBinding { forward: 0, turn: 100 });
        // 合法值原样保留（30/10 不能被 clamp 改掉）
        assert_eq!(mk(Some(30), Some(10)), binding());
        assert_eq!(mk(Some(0), Some(100)), SpeedBinding { forward: 0, turn: 100 });
    }
}
