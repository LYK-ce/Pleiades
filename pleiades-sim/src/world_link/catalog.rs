//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-21

//! 世界零件目录的**本地镜像** + 名字校验（task_31 §四.5.3）
//!
//! 为什么必须本地校验：世界对未知 `sensor` 是「忽略该 slot、不回 ready、**保持连接**」，
//! 也就是一条永远等不到 `sensor_ready` 的半死连接（task_31 §2.4-1）；对未知 `chassis`
//! 则直接 1002 关闭。两种失败都只能在 sim 侧提前拦下，绝不能让世界踩到。
//!
//! 镜像来源（**名字**唯一来源，不复制任何数值默认——缺省一律交给世界
//! `ApplyChassis` / `AttachSensor` 的 `_merge_params` / `mount.get(...)`）：
//! - `orion-simulation-world/scripts/chassis/ChassisCatalog.gd` 的 `SCENES` 键
//! - `orion-simulation-world/scripts/sensors/SensorCatalog.gd` 的 `SCENES` 键

/// 世界底盘目录名（`ChassisCatalog.SCENES` 的键）
pub const CHASSIS: [&str; 2] = ["diff_drive_ugv", "quadrotor_uav"];

/// 世界传感器目录名（`SensorCatalog.SCENES` 的键；v1 只有 lidar_2d）
pub const SENSORS: [&str; 1] = ["lidar_2d"];

/// 底盘名校验：非法 → `start()` 直接 `Err`（不发起任何连接）。
pub fn Validate_Chassis(name: &str) -> Result<(), String> {
    let n = name.trim();
    if CHASSIS.contains(&n) {
        Ok(())
    } else {
        Err(format!("未知底盘 '{n}'；世界目录只有 {CHASSIS:?}"))
    }
}

/// 传感器名校验：非法 → 跳过该 slot + `warn!`，不进 manifest（其余照连）。
pub fn Validate_Sensor(name: &str) -> Result<(), String> {
    let n = name.trim();
    if SENSORS.contains(&n) {
        Ok(())
    } else {
        Err(format!("未知传感器 '{n}'；世界目录只有 {SENSORS:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_chassis_ok() {
        assert!(Validate_Chassis("diff_drive_ugv").is_ok());
        assert!(Validate_Chassis("quadrotor_uav").is_ok());
        // 前后空格被 trim（TOML 里手抖多打的空白不该判死）
        assert!(Validate_Chassis(" diff_drive_ugv ").is_ok());
    }

    #[test]
    fn test_validate_chassis_reject() {
        for bad in ["tank", "X3Plus", "", "  "] {
            assert!(Validate_Chassis(bad).is_err(), "'{bad}' 应该被拒");
        }
        // 错误信息要能直接指路
        let e = Validate_Chassis("tank").unwrap_err();
        assert!(e.contains("tank"), "错误信息缺名字：{e}");
        assert!(e.contains("diff_drive_ugv"), "错误信息缺目录：{e}");
    }

    #[test]
    fn test_validate_sensor_ok() {
        assert!(Validate_Sensor("lidar_2d").is_ok());
    }

    #[test]
    fn test_validate_sensor_reject() {
        for bad in ["imu", "gnss", "camera", ""] {
            assert!(Validate_Sensor(bad).is_err(), "'{bad}' 应该被拒");
        }
        let e = Validate_Sensor("imu").unwrap_err();
        assert!(e.contains("imu"), "错误信息缺名字：{e}");
        assert!(e.contains("lidar_2d"), "错误信息缺目录：{e}");
    }
}
