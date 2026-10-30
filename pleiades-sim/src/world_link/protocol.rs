//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-29

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
/// 连续档位 / 速度指令的 `action`（task_39 §四.4、task_40 §二.6）：与
/// `forward/backward/left/right/up/down/stop` 七个离散 action **并存**。
///
/// 同一 `action` 按**底盘**解释（两个底盘脚本不共享解析代码，故同一字面量不冲突）：
/// - 机栈 `Quadrotor.gd` 读 `v` / `omega`（物理量，m/s / rad/s，task_39）；
/// - 车栈 `DiffDrive.gd` 读 `speed` / `turn`（百分比档位，task_40）。
pub const ACTION_VELOCITY: &str = "velocity";
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

/// 一条动作帧（世界 `motion_cmd`）
///
/// task_39 §四.4 在**同一个结构体**上加 `v` / `omega` 两个可选字段，task_40 §二.6 再加 `turn`，
/// 而不是换结构体 / 换 enum：本类型是动作通道的元素类型（`mpsc::Receiver<protocol::MotionCmd>`），
/// 且 `sim::chassis::ugv.rs` 以 `m.action` / `m.speed` 字段访问它 —— 换类型会让差速底盘编不过。
/// 三个可选字段全 `None` 时 ⇒ 序列化**逐字不变**（`skip_serializing_if` 整键省略，由单测锁住）。
#[derive(Debug, Clone, Serialize)]
pub struct MotionCmd {
    pub msg: &'static str,
    pub seq: u64,
    pub action: &'static str,
    /// 前进档位（车栈连续档 / 旧离散动作共用同一个字段；机栈速度帧填 0）
    pub speed: u8,
    /// 连续速度的水平速度（仅 `action == ACTION_VELOCITY` 且机栈时 `Some`）：**m/s**
    /// （物理量，与真机 UAV 的 `vel_fwd` 同口径；task_39 §四 修订）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v: Option<f32>,
    /// 连续速度的偏航角速度（同上）：**rad/s**（与真机 UAV 的 `yaw_rate_deg` 同口径；
    /// 符号见 `sim::uav_sim::executor` 的文件头推导）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omega: Option<f32>,
    /// 车栈连续档位的**转向档位**（仅 `action == ACTION_VELOCITY` 且车栈时 `Some`）：**−100..+100**
    /// 带符号百分比（`Δθ > 0`（目标在右）⇒ `turn < 0`；符号证据链见 `sim::ugv_sim::executor` 文件头）。
    /// `skip_serializing_if` ⇒ 旧帧（离散动作 / 机栈速度帧）JSON **逐字不变**。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<i16>,
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
        // 三个可选字段全 None ⇒ JSON **逐字**等于 task_39 之前的旧帧（task_40 §二.6 单测 ①）
        let cmd = MotionCmd {
            msg: MSG_MOTION_CMD,
            seq: 7,
            action: "forward",
            speed: 50,
            v: None,
            omega: None,
            turn: None,
        };
        let v = serde_json::to_value(&cmd).expect("serialize");
        assert_eq!(v, json!({"msg":"motion_cmd","seq":7,"action":"forward","speed":50}));
    }

    #[test]
    fn test_serialize_motion_velocity_cmd() {
        // task_39 §四.4：速度帧带 v / omega；`speed` 仍恒序列化（世界 `"velocity"` 分支不读它）
        let cmd = MotionCmd {
            msg: MSG_MOTION_CMD,
            seq: 12,
            action: ACTION_VELOCITY,
            speed: 0,
            v: Some(0.5),
            omega: Some(-0.25),
            turn: None,
        };
        let v = serde_json::to_value(&cmd).expect("serialize");
        assert_eq!(
            v,
            json!({"msg":"motion_cmd","seq":12,"action":"velocity","speed":0,"v":0.5,"omega":-0.25})
        );
        let obj = v.as_object().expect("object");
        assert!(!obj.contains_key("turn"), "机栈速度帧不该出现 turn 键");
    }

    #[test]
    fn test_serialize_motion_cmd_without_velocity_keys() {
        // 双锁「None 时三个键**整键省略**」（旧帧 JSON 逐字不变，R4）
        let cmd = MotionCmd {
            msg: MSG_MOTION_CMD,
            seq: 1,
            action: "stop",
            speed: 0,
            v: None,
            omega: None,
            turn: None,
        };
        let v = serde_json::to_value(&cmd).expect("serialize");
        assert_eq!(v, json!({"msg":"motion_cmd","seq":1,"action":"stop","speed":0}));
        let obj = v.as_object().expect("object");
        assert!(!obj.contains_key("v"), "None 不该出现 v 键");
        assert!(!obj.contains_key("omega"), "None 不该出现 omega 键");
        assert!(!obj.contains_key("turn"), "None 不该出现 turn 键");
    }

    #[test]
    fn test_serialize_motion_cmd_turn_negative() {
        // task_40 §二.6：车栈帧 = action="velocity" + speed 前进档 + turn 带符号转向档，
        // `v` / `omega` 整键省略（世界 `DiffDrive.gd` 的 `"velocity"` 分支读 speed/turn）
        let cmd = MotionCmd {
            msg: MSG_MOTION_CMD,
            seq: 13,
            action: ACTION_VELOCITY,
            speed: 30,
            v: None,
            omega: None,
            turn: Some(-50),
        };
        let v = serde_json::to_value(&cmd).expect("serialize");
        assert_eq!(
            v,
            json!({"msg":"motion_cmd","seq":13,"action":"velocity","speed":30,"turn":-50})
        );
        let obj = v.as_object().expect("object");
        assert!(!obj.contains_key("v"), "车栈帧不该出现 v 键");
        assert!(!obj.contains_key("omega"), "车栈帧不该出现 omega 键");
    }

    #[test]
    fn test_serialize_motion_cmd_turn_zero_is_present() {
        // 0 ≠ None：`turn = 0`（直行）必须**留在** JSON 里，否则世界侧会当成"没带 turn"
        let cmd = MotionCmd {
            msg: MSG_MOTION_CMD,
            seq: 14,
            action: ACTION_VELOCITY,
            speed: 30,
            v: None,
            omega: None,
            turn: Some(0),
        };
        let v = serde_json::to_value(&cmd).expect("serialize");
        assert_eq!(
            v,
            json!({"msg":"motion_cmd","seq":14,"action":"velocity","speed":30,"turn":0})
        );
    }

    #[test]
    fn test_serialize_motion_cmd_turn_positive() {
        // 正 turn（目标在左）⇒ 世界 `rotation.y` 增大（左转）
        let cmd = MotionCmd {
            msg: MSG_MOTION_CMD,
            seq: 15,
            action: ACTION_VELOCITY,
            speed: 30,
            v: None,
            omega: None,
            turn: Some(45),
        };
        let v = serde_json::to_value(&cmd).expect("serialize");
        assert_eq!(
            v,
            json!({"msg":"motion_cmd","seq":15,"action":"velocity","speed":30,"turn":45})
        );
    }
}
