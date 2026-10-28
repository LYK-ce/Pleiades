//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-28

//! 零件声明 → `*_hello` 负载（底盘 / 传感器共用入口，task_31 §四.5.4 与 §四.2 差异 4）
//!
//! **JSON 形状只此一处**：hello 的键名/嵌套由 [`protocol::ChassisHello`] / [`protocol::SensorHello`]
//! 的结构体定义，本模块只负责「从 [`PartDecl`] 装出来」，避免第二份手写 JSON 与结构体漂移。
//!
//! `manifest` 的唯一来源是**过滤后的传感器清单**（同一份 `Vec<ResolvedSensor>` 既建连接也进 manifest），
//! 所以「连接数 = 1 + sensors.len()」和「manifest 条目数 = sensors.len()」天然一致（§四.2 差异 4）。

use tracing::error;

use crate::world_link::config::ResolvedSensor;
use crate::world_link::protocol::{self, ManifestEntry, MountOut};
use crate::world_link::protocol::{MSG_CHASSIS_HELLO, MSG_SENSOR_HELLO};

/// 一个零件的声明（底盘 / 传感器共用）
pub enum PartDecl<'a> {
    Chassis {
        peer_id: &'a str,
        chassis: &'a str,
        params: &'a serde_json::Value,
        manifest: Vec<ManifestEntry<'a>>,
    },
    Sensor {
        peer_id: &'a str,
        slot: &'a str,
        sensor: &'a str,
        mount: MountOut,
        params: &'a serde_json::Value,
    },
}

/// 声明 → hello 负载（可直接进 `LinkSender::Send_Json`）。
///
/// 序列化失败只可能是数值非法（NaN / infinity，来自手写配置）；此时回 `Value::Null`，
/// 调用方按**致命错误**处理（世界会以 1002 关闭，重连也不会成功）。
pub fn Build_Hello(decl: &PartDecl<'_>) -> serde_json::Value {
    let res = match decl {
        PartDecl::Chassis { peer_id, chassis, params, manifest } => {
            serde_json::to_value(protocol::ChassisHello {
                msg: MSG_CHASSIS_HELLO,
                peer_id,
                chassis,
                params: (*params).clone(),
                manifest: manifest.clone(),
            })
        }
        PartDecl::Sensor { peer_id, slot, sensor, mount, params } => {
            serde_json::to_value(protocol::SensorHello {
                msg: MSG_SENSOR_HELLO,
                peer_id,
                slot,
                sensor,
                mount: *mount,
                params: (*params).clone(),
            })
        }
    };
    match res {
        Ok(v) => v,
        Err(e) => {
            error!("[world] hello 序列化失败（params 里有 NaN / infinity？）：{e}");
            serde_json::Value::Null
        }
    }
}

/// manifest 由传感器清单推导（**唯一来源**）；`sensors` 为空 → 空数组
pub fn Build_Manifest(sensors: &[ResolvedSensor]) -> Vec<ManifestEntry<'_>> {
    sensors
        .iter()
        .map(|s| ManifestEntry { slot: s.slot.as_str(), sensor: s.sensor.as_str() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resolved(slot: &str, sensor: &str) -> ResolvedSensor {
        ResolvedSensor {
            slot: slot.into(),
            sensor: sensor.into(),
            mount: MountOut { x: 0.1, y: 0.0, z: 0.2, yaw: 0.0, pitch: 0.0 },
            params: json!({"range": 12.0}),
        }
    }

    #[test]
    fn test_build_chassis_hello() {
        let sensors = vec![resolved("lidar_front", "lidar_2d"), resolved("lidar_rear", "lidar_2d")];
        let manifest = Build_Manifest(&sensors);
        let params = json!({"max_speed": 1.5, "max_turn_rate": 90});
        let hello = Build_Hello(&PartDecl::Chassis {
            peer_id: "12D3KooW",
            chassis: "diff_drive_ugv",
            params: &params,
            manifest,
        });
        assert_eq!(hello["msg"], json!("chassis_hello"));
        assert_eq!(hello["peer_id"], json!("12D3KooW"));
        assert_eq!(hello["chassis"], json!("diff_drive_ugv"));
        assert_eq!(hello["params"], json!({"max_speed": 1.5, "max_turn_rate": 90}));
        let m = hello["manifest"].as_array().expect("manifest 应是数组");
        assert_eq!(m.len(), sensors.len(), "manifest 与传感器清单同长");
        assert_eq!(m[0], json!({"slot":"lidar_front","sensor":"lidar_2d"}));
        assert_eq!(m[1], json!({"slot":"lidar_rear","sensor":"lidar_2d"}));
    }

    #[test]
    fn test_build_sensor_hello() {
        let params = json!({"range": 12.0, "rays": 360, "rate_hz": 10});
        let hello = Build_Hello(&PartDecl::Sensor {
            peer_id: "12D3KooW",
            slot: "lidar_front",
            sensor: "lidar_2d",
            mount: MountOut { x: 0.1, y: 0.0, z: 0.2, yaw: 0.0, pitch: 0.0 },
            params: &params,
        });
        assert_eq!(hello["msg"], json!("sensor_hello"));
        assert_eq!(hello["peer_id"], json!("12D3KooW"));
        assert_eq!(hello["slot"], json!("lidar_front"));
        assert_eq!(hello["sensor"], json!("lidar_2d"));
        // mount 是 f32 → JSON 数字（f64 表示带精度尾巴），逐字段比
        let mount = hello["mount"].as_object().expect("mount 应是对象");
        let mut keys: Vec<&str> = mount.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["x", "y", "yaw", "z"], "mount 必须恰好 4 键");
        assert!((mount["x"].as_f64().unwrap() - 0.1).abs() < 1e-6);
        assert_eq!(mount["y"].as_f64(), Some(0.0));
        assert!((mount["z"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(mount["yaw"].as_f64(), Some(0.0));
        assert_eq!(hello["params"], json!({"range": 12.0, "rays": 360, "rate_hz": 10}));
    }

    #[test]
    fn test_build_manifest_from_resolved() {
        // 过滤后的清单 = manifest（顺序保持一致）
        let sensors = vec![resolved("lidar_front", "lidar_2d")];
        let m = Build_Manifest(&sensors);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].slot, "lidar_front");
        assert_eq!(m[0].sensor, "lidar_2d");

        // slots = 0 → 空数组（世界会立刻 assembly_ready，见 §四.2 差异 5）
        let empty = Build_Manifest(&[]);
        assert!(empty.is_empty());
        let hello = Build_Hello(&PartDecl::Chassis {
            peer_id: "p",
            chassis: "diff_drive_ugv",
            params: &json!({}),
            manifest: empty,
        });
        assert_eq!(hello["manifest"], json!([]));
    }
}
