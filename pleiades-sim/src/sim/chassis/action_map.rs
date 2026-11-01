//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-11-01

#![allow(non_snake_case)]

//! **速度绑定**：`[chassis]` 的配置 → 车栈执行器用的两个巡航物理量
//!
//! 本文件原名「动作翻译表」——`MotionAction`（真机语义）→ 世界 `motion_cmd` 的
//! `(action, speed)` 那张 4+1 行表**已随 `0x11 FUNC_CAR_RUN` 一起删除**（task_44 §1.2 #1），
//! 文件名保留只为 `git log --follow` 还能接上历史。
//!
//! 现在这里只剩一件事（task_44 §1.2 #11/#12）：
//!
//! | 配置键 | 字段 | 单位 | 缺省 | 上限 |
//! |---|---|---|---|---|
//! | `[chassis] cruise_speed` | [`SpeedBinding::cruise_speed`] | **m/s** | 0.45 | 1.5 |
//! | `[chassis] cruise_yaw_rate` | [`SpeedBinding::cruise_yaw_rate`] | **rad/s** | 0.5 | π/2 |
//!
//! 三条不可动的口径：
//!
//! 1. **不做比例换算**：配置里写 0.45 就是 0.45 m/s，不乘任何系数、不读百分比
//!    （百分比 → m/s 的换算在世界侧 `DiffDrive.gd` 早已删除）。
//! 2. **上限只作二次兜底**：`MAX_CRUISE_SPEED = 1.5` 与 `MAX_CRUISE_YAW_RATE = π/2` 取世界
//!    `DiffDrive.gd` 的两个缺省上限（`max_speed = 1.5` m/s、`max_turn_rate = 90 °/s`），
//!    只为「配置写坏也不让仿真飞出去」，正常值 0.45 / 0.5 离上限还有 3 倍余量。
//! 3. **负数 / 非有限一律回默认**（不是回 0）：与 `FlightCtrlConfig::Resolved`
//!    （`config.rs`）同风格；`0.0` 是**合法值**（原地不动），保留 0。

use crate::config::ChassisConfig;

/// 车栈速度绑定（两个**物理量**，不是百分比档位）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedBinding {
    /// 巡航线速度（**m/s**，缺省 0.45；0 = 不走）
    pub cruise_speed: f32,
    /// 巡航偏航角速度**绝对值**（**rad/s**，缺省 0.5，恒为正；方向由执行器定符号）
    pub cruise_yaw_rate: f32,
}

impl SpeedBinding {
    /// 缺省巡航线速度（task_44 §1.2 #11/#12 的定值）
    pub const DEFAULT_CRUISE_SPEED: f32 = 0.45;
    /// 缺省巡航角速度绝对值（task_44 §1.2 #11/#12 的定值）
    pub const DEFAULT_CRUISE_YAW_RATE: f32 = 0.5;
    /// 二次兜底上限 = 世界 `DiffDrive.gd` 的 `max_speed` 缺省（1.5 m/s）
    pub const MAX_CRUISE_SPEED: f32 = 1.5;
    /// 二次兜底上限 = 世界 `DiffDrive.gd` 的 `max_turn_rate` 缺省（90 °/s = π/2 rad/s）
    pub const MAX_CRUISE_YAW_RATE: f32 = std::f32::consts::FRAC_PI_2;

    /// `[chassis]` → 绑定：缺键取默认；非有限 / 负数回默认；超过上限的夹到上限。
    pub fn From_Config(cfg: Option<&ChassisConfig>) -> Self {
        let pick = |v: Option<f32>, default: f32, max: f32| -> f32 {
            match v {
                Some(x) if x.is_finite() && x >= 0.0 => x.min(max),
                _ => default,
            }
        };
        match cfg {
            Some(c) => Self {
                cruise_speed: pick(
                    c.cruise_speed,
                    Self::DEFAULT_CRUISE_SPEED,
                    Self::MAX_CRUISE_SPEED,
                ),
                cruise_yaw_rate: pick(
                    c.cruise_yaw_rate,
                    Self::DEFAULT_CRUISE_YAW_RATE,
                    Self::MAX_CRUISE_YAW_RATE,
                ),
            },
            None => Self {
                cruise_speed: Self::DEFAULT_CRUISE_SPEED,
                cruise_yaw_rate: Self::DEFAULT_CRUISE_YAW_RATE,
            },
        }
    }
}

impl Default for SpeedBinding {
    fn default() -> Self {
        Self {
            cruise_speed: Self::DEFAULT_CRUISE_SPEED,
            cruise_yaw_rate: Self::DEFAULT_CRUISE_YAW_RATE,
        }
    }
}

// ================================================================ 单测（锁 §1.2 #11/#12 的两个定值）

#[cfg(test)]
mod tests {
    use super::*;

    /// 缺省绑定（0.45 m/s / 0.5 rad/s）
    fn binding() -> SpeedBinding {
        SpeedBinding { cruise_speed: 0.45, cruise_yaw_rate: 0.5 }
    }

    #[test]
    fn test_speed_binding_defaults() {
        // 常量本身就是 §1.2 的定值（改常量必红）
        assert_eq!(SpeedBinding::DEFAULT_CRUISE_SPEED, 0.45);
        assert_eq!(SpeedBinding::DEFAULT_CRUISE_YAW_RATE, 0.5);
        // 无段 / 空段 / Default → 同一组定值
        assert_eq!(SpeedBinding::From_Config(None), binding());
        assert_eq!(SpeedBinding::From_Config(Some(&ChassisConfig::default())), binding());
        assert_eq!(SpeedBinding::default(), binding());
    }

    #[test]
    fn test_speed_binding_clamps() {
        let mk = |v: Option<f32>, w: Option<f32>| {
            SpeedBinding::From_Config(Some(&ChassisConfig {
                enabled: None,
                cruise_speed: v,
                cruise_yaw_rate: w,
            }))
        };
        // 负数 / 非有限 → 回**默认**（不是回 0：配置写坏不该让车不动）
        assert_eq!(mk(Some(-1.0), Some(-1.0)), binding());
        assert_eq!(mk(Some(f32::NAN), Some(f32::INFINITY)), binding());
        assert_eq!(mk(Some(f32::NEG_INFINITY), Some(f32::NAN)), binding());
        // 超过上限 → 夹到上限（1.5 m/s / π/2 rad/s）
        assert_eq!(
            mk(Some(99.0), Some(99.0)),
            SpeedBinding {
                cruise_speed: SpeedBinding::MAX_CRUISE_SPEED,
                cruise_yaw_rate: SpeedBinding::MAX_CRUISE_YAW_RATE,
            }
        );
        // 0.0 是合法值（原地不动），不许被当成"缺省"
        assert_eq!(mk(Some(0.0), Some(0.0)), SpeedBinding { cruise_speed: 0.0, cruise_yaw_rate: 0.0 });
        // 只写一个键：另一个仍取默认
        assert_eq!(mk(Some(0.55), None), SpeedBinding { cruise_speed: 0.55, cruise_yaw_rate: 0.5 });
    }

    #[test]
    fn test_speed_binding_no_percent() {
        // §1.2 #11：执行器内**不做任何比例换算** —— 配置值原样进结构体
        let c = ChassisConfig { enabled: Some(true), cruise_speed: Some(0.45), cruise_yaw_rate: Some(0.5) };
        let b = SpeedBinding::From_Config(Some(&c));
        assert_eq!(b.cruise_speed, 0.45, "0.45 传入后必须还是 0.45（不乘 100、不除 100）");
        assert_eq!(b.cruise_yaw_rate, 0.5, "0.5 传入后必须还是 0.5");
    }
}
