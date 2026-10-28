//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-28

//! 世界链路 wire 协议（`orion-simulation-world/scripts/net/WireProtocol.gd` 的 Rust 镜像）
//!
//! 只做「协议事实」：消息名常量 + 出站 serde 结构 + 入站文本帧解析。不碰 socket、不打业务日志。
//!
//! 铁律：
//! - 出站一律 JSON **文本**帧（`Message::Text`）；世界侧收到二进制会 warning「二进制只下行」。
//! - 入站先读 `msg` 再分派；**任何缺失字段一律取默认、绝不 panic**
//!   （世界 `pose` 的 8 键在 Step 1 才补齐，`vx`/`vy` 缺失也必须能解析，见 task_31 §四.8-2）。
//! - 未知 `msg` → [`Inbound::Unknown`]（世界侧对未知 msg 是忽略，sim 侧对等处理，不断连）。
//! - 二进制帧（lidar）的帧头偏移量只有一处事实：`sim::sensors::lidar::frame`（Step 3）。
//!   本模块只管文本帧，不再记录第二份偏移量常量。

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ================================================================ 文本帧消息名（WireProtocol.gd 镜像）

pub const MSG_CHASSIS_HELLO: &str = "chassis_hello";
pub const MSG_CHASSIS_READY: &str = "chassis_ready";
pub const MSG_SENSOR_HELLO: &str = "sensor_hello";
pub const MSG_SENSOR_READY: &str = "sensor_ready";
pub const MSG_MOTION_CMD: &str = "motion_cmd";
pub const MSG_ASSEMBLY_READY: &str = "assembly_ready";
pub const MSG_POSE: &str = "pose";

// ================================================================ 出站结构

/// 出生点（世界坐标 Z-up、米、yaw 弧度绕 +z）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Spawn {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
}

/// 外参（世界 yaw 语义见 `Vehicle.gd`）；缺省全 0.0，不复制世界的数值默认
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MountOut {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    /// 俯仰（度，负 = 低头；世界 `Vehicle.AttachSensor` 的 `mount.pitch`）
    pub pitch: f32,
}

/// manifest 的一条：`{slot, sensor}`（世界 `_ReadManifest` 只读这两个键）
#[derive(Debug, Clone, Serialize)]
pub struct ManifestEntry<'a> {
    pub slot: &'a str,
    pub sensor: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChassisHello<'a> {
    pub msg: &'static str,
    pub peer_id: &'a str,
    pub chassis: &'a str,
    pub params: Value,
    pub manifest: Vec<ManifestEntry<'a>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorHello<'a> {
    pub msg: &'static str,
    pub peer_id: &'a str,
    pub slot: &'a str,
    pub sensor: &'a str,
    pub mount: MountOut,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct MotionCmd {
    pub msg: &'static str,
    pub seq: u64,
    pub action: &'static str,
    pub speed: u8,
}

// ================================================================ 入站

#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    ChassisReady {
        avatar_id: u16,
        spawn: Spawn,
        world_time: f64,
    },
    SensorReady {
        slot: String,
        sensor_id: u16,
    },
    AssemblyReady {
        world_time: f64,
    },
    Pose {
        world_time: f64,
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        vx: f32,
        vy: f32,
    },
    /// 只在 supervisor 里由 `LinkEvent::Binary` 转成，`Parse_Text` **不会**产生
    Binary {
        bytes: Vec<u8>,
    },
    Unknown {
        msg: String,
    },
}

// ---------------------------------------------------------------- 取值工具（缺字段 -> 默认，绝不 panic）

fn f32_of(obj: &serde_json::Map<String, Value>, key: &str) -> f32 {
    obj.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn f64_of(obj: &serde_json::Map<String, Value>, key: &str) -> f64 {
    obj.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn u16_of(obj: &serde_json::Map<String, Value>, key: &str) -> u16 {
    obj.get(key)
        .and_then(Value::as_u64)
        .filter(|n| *n <= u16::MAX as u64)
        .unwrap_or(0) as u16
}

/// 解析一条文本帧。`None` = 非法 JSON / 顶层非对象 / 缺 `msg` / `msg` 非字符串（调用方 debug 后丢弃）。
pub fn Parse_Text(s: &str) -> Option<Inbound> {
    let v: Value = serde_json::from_str(s).ok()?;
    let obj = v.as_object()?;
    let msg = obj.get("msg")?.as_str()?.to_string();

    Some(match msg.as_str() {
        MSG_CHASSIS_READY => {
            let spawn = match obj.get("spawn").and_then(Value::as_object) {
                Some(sp) => Spawn {
                    x: f32_of(sp, "x"),
                    y: f32_of(sp, "y"),
                    z: f32_of(sp, "z"),
                    yaw: f32_of(sp, "yaw"),
                },
                None => Spawn::default(),
            };
            Inbound::ChassisReady {
                avatar_id: u16_of(obj, "avatar_id"),
                spawn,
                world_time: f64_of(obj, "world_time"),
            }
        }
        MSG_SENSOR_READY => {
            // slot 是匹配就绪的**唯一**依据：缺失/非字符串/空 -> 这条消息无法使用（不 panic）
            let slot = obj.get("slot")?.as_str()?.trim().to_string();
            if slot.is_empty() {
                return None;
            }
            Inbound::SensorReady {
                slot,
                sensor_id: u16_of(obj, "sensor_id"),
            }
        }
        MSG_ASSEMBLY_READY => Inbound::AssemblyReady {
            world_time: f64_of(obj, "world_time"),
        },
        MSG_POSE => Inbound::Pose {
            world_time: f64_of(obj, "world_time"),
            x: f32_of(obj, "x"),
            y: f32_of(obj, "y"),
            z: f32_of(obj, "z"),
            yaw: f32_of(obj, "yaw"),
            // vx / vy 缺失必须仍能解析（取 0.0）
            vx: f32_of(obj, "vx"),
            vy: f32_of(obj, "vy"),
        },
        other => Inbound::Unknown {
            msg: other.to_string(),
        },
    })
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_chassis_ready() {
        let s = r#"{"msg":"chassis_ready","peer_id":"abc","avatar_id":3,
                    "spawn":{"x":64.0,"y":65.5,"z":0.25,"yaw":1.5},"world_time":12.5}"#;
        match Parse_Text(s) {
            Some(Inbound::ChassisReady { avatar_id, spawn, world_time }) => {
                assert_eq!(avatar_id, 3);
                assert!((spawn.x - 64.0).abs() < 1e-6, "spawn.x={}", spawn.x);
                assert!((spawn.y - 65.5).abs() < 1e-6, "spawn.y={}", spawn.y);
                assert!((spawn.z - 0.25).abs() < 1e-6, "spawn.z={}", spawn.z);
                assert!((spawn.yaw - 1.5).abs() < 1e-6, "spawn.yaw={}", spawn.yaw);
                assert!((world_time - 12.5).abs() < 1e-9);
            }
            other => panic!("期望 ChassisReady，实得 {other:?}"),
        }
    }

    #[test]
    fn test_parse_chassis_ready_missing_spawn() {
        // 缺 spawn -> Spawn::default()，不 panic
        match Parse_Text(r#"{"msg":"chassis_ready","avatar_id":7}"#) {
            Some(Inbound::ChassisReady { avatar_id, spawn, world_time }) => {
                assert_eq!(avatar_id, 7);
                assert_eq!(spawn, Spawn::default());
                assert_eq!(world_time, 0.0);
            }
            other => panic!("期望 ChassisReady，实得 {other:?}"),
        }
    }

    #[test]
    fn test_parse_sensor_ready() {
        match Parse_Text(r#"{"msg":"sensor_ready","slot":"lidar_front","sensor_id":42}"#) {
            Some(Inbound::SensorReady { slot, sensor_id }) => {
                assert_eq!(slot, "lidar_front");
                assert_eq!(sensor_id, 42);
            }
            other => panic!("期望 SensorReady，实得 {other:?}"),
        }
        // slot 缺失 -> None（无法用于匹配，但不 panic）
        assert_eq!(Parse_Text(r#"{"msg":"sensor_ready","sensor_id":42}"#), None);
    }

    #[test]
    fn test_parse_assembly_ready() {
        assert_eq!(
            Parse_Text(r#"{"msg":"assembly_ready","peer_id":"abc","world_time":7.25}"#),
            Some(Inbound::AssemblyReady { world_time: 7.25 })
        );
    }

    #[test]
    fn test_parse_pose_full() {
        let s = r#"{"msg":"pose","world_time":3.5,"x":1.0,"y":2.0,"z":0.3,
                    "yaw":-1.25,"vx":0.5,"vy":-0.25}"#;
        match Parse_Text(s) {
            Some(Inbound::Pose { world_time, x, y, z, yaw, vx, vy }) => {
                assert!((world_time - 3.5).abs() < 1e-9);
                assert!((x - 1.0).abs() < 1e-6);
                assert!((y - 2.0).abs() < 1e-6);
                assert!((z - 0.3).abs() < 1e-6);
                assert!((yaw + 1.25).abs() < 1e-6);
                assert!((vx - 0.5).abs() < 1e-6);
                assert!((vy + 0.25).abs() < 1e-6);
            }
            other => panic!("期望 Pose，实得 {other:?}"),
        }
    }

    #[test]
    fn test_parse_pose_missing_vx_vy() {
        // 硬验收 3：缺 vx/vy 仍可解析，取 0.0
        let s = r#"{"msg":"pose","world_time":1.0,"x":1.0,"y":2.0,"z":0.0,"yaw":0.5}"#;
        match Parse_Text(s) {
            Some(Inbound::Pose { vx, vy, x, yaw, .. }) => {
                assert_eq!(vx, 0.0);
                assert_eq!(vy, 0.0);
                assert!((x - 1.0).abs() < 1e-6);
                assert!((yaw - 0.5).abs() < 1e-6);
            }
            other => panic!("期望 Pose（vx/vy 缺省 0.0），实得 {other:?}"),
        }
    }

    #[test]
    fn test_parse_unknown_msg() {
        assert_eq!(
            Parse_Text(r#"{"msg":"imu","x":1}"#),
            Some(Inbound::Unknown { msg: "imu".to_string() })
        );
    }

    #[test]
    fn test_parse_invalid() {
        assert_eq!(Parse_Text(""), None);
        assert_eq!(Parse_Text("[]"), None);
        assert_eq!(Parse_Text("{}"), None);
        assert_eq!(Parse_Text(r#"{"msg":1}"#), None);
        assert_eq!(Parse_Text("not json"), None);
    }

    #[test]
    fn test_serialize_chassis_hello_keys() {
        let manifest = vec![ManifestEntry { slot: "lidar_front", sensor: "lidar_2d" }];
        let hello = ChassisHello {
            msg: MSG_CHASSIS_HELLO,
            peer_id: "12D3Koo",
            chassis: "diff_drive_ugv",
            params: json!({"max_speed": 1.5}),
            manifest,
        };
        let v = serde_json::to_value(&hello).expect("serialize");
        let obj = v.as_object().expect("object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["chassis", "manifest", "msg", "params", "peer_id"]);
        assert_eq!(obj["msg"], json!("chassis_hello"));
        assert_eq!(obj["peer_id"], json!("12D3Koo"));
        assert_eq!(obj["chassis"], json!("diff_drive_ugv"));
        assert_eq!(obj["params"], json!({"max_speed": 1.5}));
        let m0 = obj["manifest"][0].as_object().expect("manifest[0]");
        let mut mkeys: Vec<&str> = m0.keys().map(String::as_str).collect();
        mkeys.sort_unstable();
        assert_eq!(mkeys, vec!["sensor", "slot"]);
        assert_eq!(m0["slot"], json!("lidar_front"));
        assert_eq!(m0["sensor"], json!("lidar_2d"));
    }

    #[test]
    fn test_serialize_sensor_hello_mount() {
        let hello = SensorHello {
            msg: MSG_SENSOR_HELLO,
            peer_id: "12D3Koo",
            slot: "lidar_front",
            sensor: "lidar_2d",
            mount: MountOut { x: 0.1, y: 0.0, z: 0.2, yaw: 0.0, pitch: 0.0 },
            params: json!({"range": 12.0}),
        };
        let v = serde_json::to_value(&hello).expect("serialize");
        let mount = v["mount"].as_object().expect("mount object");
        let mut keys: Vec<&str> = mount.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["x", "y", "yaw", "z"]);
        assert_eq!(v["msg"], json!("sensor_hello"));
        assert_eq!(v["slot"], json!("lidar_front"));
        assert_eq!(v["sensor"], json!("lidar_2d"));
    }

    #[test]
    fn test_serialize_motion_cmd() {
        let cmd = MotionCmd { msg: MSG_MOTION_CMD, seq: 7, action: "forward", speed: 50 };
        let v = serde_json::to_value(&cmd).expect("serialize");
        assert_eq!(v, json!({"msg":"motion_cmd","seq":7,"action":"forward","speed":50}));
    }
}
