//Presented by KeJi
//Created Date ： 2026-10-21
//Modified Date ： 2026-10-28

//! `[sim_world]` 段配置：TOML 声明 → 填默认值 + 校验 → [`ResolvedWorld`]（task_31 §四.5.2）
//!
//! 设计取舍：
//! - `params` / `chassis_params` 用 `serde_json::Value` 直通车（TOML 表 → JSON 对象），
//!   不做字段白名单：世界对未知参数容忍，缺省走世界默认。
//! - **不复制世界的数值默认**（缺省 mount 全 0.0、params 空对象），默认值唯一来源是世界侧。
//! - `slot` 只允许 `[A-Za-z0-9_-]`：世界 `_SanitizeSlot()` 走 Godot `validate_node_name()`，
//!   会把 `. : @ / " %` 静默改写，导致 `sensor_ready` 回的 slot 与本地声明对不上、永远等不到就绪。
//! - `handshake_timeout_ms` 强制 >= 5000：世界从 accept 起算 5 s 未握手就 1002 关闭。
//! - 底盘**目录名**校验不在这里（归 `catalog.rs`，由 `start()` 调），这里只保证非空。

use serde::Deserialize;

use crate::world_link::protocol::MountOut;

// ================================================================ 默认值（本 sim 侧的网络默认）

pub const DEFAULT_URL: &str = "ws://127.0.0.1:9999";
pub const DEFAULT_RECONNECT_MS_MIN: u64 = 200;
pub const DEFAULT_RECONNECT_MS_MAX: u64 = 3000;
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 8000;
/// 世界侧 `WebSocketManager.handshake_timeout_ms = 5000`：配得比它短必然先被关
pub const MIN_HANDSHAKE_TIMEOUT_MS: u64 = 5000;

/// `[sim_world.sensor.mount]`：外参（世界 yaw 语义见 `Vehicle.gd`）
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Mount {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
    pub yaw: Option<f32>,
    /// 俯仰（**度**，负 = 低头；task_33 D10）。世界侧 `Vehicle.AttachSensor` 读同一个键，缺省 0。
    /// 此前这里漏了该字段 → 相机永远 0°，与 `[detector.camera].mount_pitch_deg` 不一致。
    pub pitch: Option<f32>,
}

/// 一条传感器声明 = 一条 WS 连接 = manifest 的一条
#[derive(Debug, Clone, Deserialize)]
pub struct SensorDecl {
    pub slot: String,
    pub sensor: String,
    pub mount: Option<Mount>,
    pub params: Option<serde_json::Value>,
}

/// `.config/config.toml` 的 `[sim_world]` 段（整段缺失由 `SimConfig.sim_world: Option<..>` 表达）
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorldLinkConfig {
    /// 缺省 true（段存在即开启）
    pub enabled: Option<bool>,
    pub url: Option<String>,
    /// 世界零件名；速度绑定见 `[chassis]` 段（Step 4）
    pub chassis: Option<String>,
    pub reconnect_ms_min: Option<u64>,
    pub reconnect_ms_max: Option<u64>,
    pub handshake_timeout_ms: Option<u64>,
    pub chassis_params: Option<serde_json::Value>,
    pub sensor: Option<Vec<SensorDecl>>,
}

/// 填完默认值、校验通过的运行期配置（`WorldLink::Spawn` 的入参）
#[derive(Debug, Clone)]
pub struct ResolvedWorld {
    pub url: String,
    pub chassis: String,
    pub reconnect_ms_min: u64,
    pub reconnect_ms_max: u64,
    pub handshake_timeout_ms: u64,
    pub chassis_params: serde_json::Value,
    pub sensors: Vec<ResolvedSensor>,
}

/// 一条通过校验的传感器声明
#[derive(Debug, Clone)]
pub struct ResolvedSensor {
    pub slot: String,
    pub sensor: String,
    pub mount: MountOut,
    pub params: serde_json::Value,
}

fn mount_out(m: Option<&Mount>) -> MountOut {
    MountOut {
        x: m.and_then(|m| m.x).unwrap_or(0.0),
        y: m.and_then(|m| m.y).unwrap_or(0.0),
        z: m.and_then(|m| m.z).unwrap_or(0.0),
        yaw: m.and_then(|m| m.yaw).unwrap_or(0.0),
        pitch: m.and_then(|m| m.pitch).unwrap_or(0.0),
    }
}

/// slot 字符集：世界 `validate_node_name()` 的安全子集
fn slot_charset_ok(slot: &str) -> bool {
    slot.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl WorldLinkConfig {
    /// `enabled` 缺省 = true；整段缺失由 `SimConfig.sim_world: Option<..>` 表达
    pub fn Enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// 填默认值 + 校验。任何不合规都返回**可操作**的中文错误（`start()` 直接 `Err`）。
    /// 校验顺序即报错优先级：url → chassis → 退避 → 握手 → slot（空/字符集/重复）。
    pub fn Resolved(&self) -> Result<ResolvedWorld, String> {
        // 1) url
        let url = self.url.as_deref().unwrap_or(DEFAULT_URL).trim().to_string();
        if !url.starts_with("ws://") {
            return Err(format!(
                "url 必须以 ws:// 开头（tokio-tungstenite 默认特性不含 TLS），当前 '{url}'"
            ));
        }

        // 2) chassis（目录名合法性由 catalog::Validate_Chassis 在 start() 里查）
        let chassis = self.chassis.as_deref().unwrap_or("").trim().to_string();
        if chassis.is_empty() {
            return Err(
                "[sim_world].chassis 未设置（须为世界底盘目录名：diff_drive_ugv / quadrotor_uav）"
                    .to_string(),
            );
        }

        // 3) 退避
        let min = self.reconnect_ms_min.unwrap_or(DEFAULT_RECONNECT_MS_MIN);
        let max = self.reconnect_ms_max.unwrap_or(DEFAULT_RECONNECT_MS_MAX);
        if min == 0 {
            return Err("reconnect_ms_min 必须 > 0（=0 会忙等重连，打爆日志与世界侧）".to_string());
        }
        if min > max {
            return Err(format!(
                "reconnect_ms_min={min} 不能大于 reconnect_ms_max={max}（退避应逐次增大）"
            ));
        }

        // 4) 握手超时
        let hs = self.handshake_timeout_ms.unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT_MS);
        if hs < MIN_HANDSHAKE_TIMEOUT_MS {
            return Err(format!(
                "handshake_timeout_ms={hs} 必须 >= {MIN_HANDSHAKE_TIMEOUT_MS}（世界侧 5 s 未握手会以 1002 关闭；建议用默认 8000）"
            ));
        }

        // 5~7) 传感器清单：slot 非空 / 字符集 / 唯一；mount・params 缺省
        let mut sensors: Vec<ResolvedSensor> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for (i, d) in self.sensor.iter().flatten().enumerate() {
            let slot = d.slot.trim().to_string();
            if slot.is_empty() {
                return Err(format!("[[sim_world.sensor]] 第 {i} 条的 slot 不能为空"));
            }
            if !slot_charset_ok(&slot) {
                return Err(format!(
                    "slot '{slot}' 含非法字符：只允许 [A-Za-z0-9_-]（世界侧 validate_node_name() 会静默改写 . : @ / \" %，导致 sensor_ready 对不上）"
                ));
            }
            if seen.contains(&slot) {
                return Err(format!(
                    "slot '{slot}' 重复；世界侧 slot 是 Vehicle/Sensors 下的节点名，重复会互相覆盖"
                ));
            }
            seen.push(slot.clone());

            let sensor = d.sensor.trim().to_string();
            if sensor.is_empty() {
                return Err(format!("[[sim_world.sensor]] slot='{slot}' 的 sensor 不能为空"));
            }
            sensors.push(ResolvedSensor {
                slot,
                sensor,
                mount: mount_out(d.mount.as_ref()),
                params: d.params.clone().unwrap_or_else(|| serde_json::json!({})),
            });
        }

        Ok(ResolvedWorld {
            url,
            chassis,
            reconnect_ms_min: min,
            reconnect_ms_max: max,
            handshake_timeout_ms: hs,
            chassis_params: self.chassis_params.clone().unwrap_or_else(|| serde_json::json!({})),
            sensors,
        })
    }
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;

    /// 只给 chassis 的最小配置
    fn minimal() -> WorldLinkConfig {
        WorldLinkConfig { chassis: Some("diff_drive_ugv".into()), ..Default::default() }
    }

    fn sensor(slot: &str, sensor: &str) -> SensorDecl {
        SensorDecl { slot: slot.into(), sensor: sensor.into(), mount: None, params: None }
    }

    #[test]
    fn test_enabled_default_true() {
        assert!(WorldLinkConfig::default().Enabled());
        assert!(WorldLinkConfig { enabled: Some(true), ..Default::default() }.Enabled());
        assert!(!WorldLinkConfig { enabled: Some(false), ..Default::default() }.Enabled());
    }

    #[test]
    fn test_resolved_defaults() {
        let r = minimal().Resolved().expect("只有 chassis 也应该能解析");
        assert_eq!(r.url, DEFAULT_URL);
        assert_eq!(r.url, "ws://127.0.0.1:9999");
        assert_eq!(r.chassis, "diff_drive_ugv");
        assert_eq!(r.reconnect_ms_min, 200);
        assert_eq!(r.reconnect_ms_max, 3000);
        assert_eq!(r.handshake_timeout_ms, 8000);
        assert_eq!(r.chassis_params, serde_json::json!({}));
        assert!(r.sensors.is_empty());
    }

    #[test]
    fn test_resolved_sensor_mount_and_params_defaults() {
        let cfg = WorldLinkConfig {
            sensor: Some(vec![SensorDecl {
                slot: "lidar_front".into(),
                sensor: "lidar_2d".into(),
                mount: Some(Mount { x: Some(0.1), y: None, z: Some(0.2), yaw: None, pitch: None }),
                params: None,
            }]),
            ..minimal()
        };
        let r = cfg.Resolved().expect("应通过校验");
        assert_eq!(r.sensors.len(), 1);
        let s = &r.sensors[0];
        assert_eq!(s.slot, "lidar_front");
        assert_eq!(s.sensor, "lidar_2d");
        // 缺省 0.0：不复制世界的数值默认
        assert_eq!(s.mount, MountOut { x: 0.1, y: 0.0, z: 0.2, yaw: 0.0, pitch: 0.0 });
        assert_eq!(s.params, serde_json::json!({}));
    }

    #[test]
    fn test_resolved_missing_chassis() {
        let e = WorldLinkConfig::default().Resolved().unwrap_err();
        assert!(e.contains("chassis"), "错误信息应指向 chassis：{e}");
        // 全空白等于没填
        assert!(WorldLinkConfig { chassis: Some("   ".into()), ..Default::default() }
            .Resolved()
            .is_err());
    }

    #[test]
    fn test_resolved_bad_url() {
        for bad in ["http://127.0.0.1:9999", "wss://127.0.0.1:9999", "127.0.0.1:9999"] {
            let cfg = WorldLinkConfig { url: Some(bad.into()), ..minimal() };
            let e = cfg.Resolved().unwrap_err();
            assert!(e.contains("ws://"), "错误信息应提示 ws://：{e}（url={bad}）");
        }
        // ws:// 前缀正确就放行
        assert!(WorldLinkConfig { url: Some("ws://example:1".into()), ..minimal() }.Resolved().is_ok());
    }

    #[test]
    fn test_resolved_handshake_below_min() {
        let cfg = WorldLinkConfig { handshake_timeout_ms: Some(4999), ..minimal() };
        let e = cfg.Resolved().unwrap_err();
        assert!(e.contains("5000"), "错误信息应含下限 5000：{e}");

        for ok in [5000u64, 8000] {
            let cfg = WorldLinkConfig { handshake_timeout_ms: Some(ok), ..minimal() };
            let r = cfg.Resolved().expect(">= 5000 应通过");
            assert_eq!(r.handshake_timeout_ms, ok);
        }
    }

    #[test]
    fn test_resolved_duplicate_slot() {
        let cfg = WorldLinkConfig {
            sensor: Some(vec![sensor("lidar_front", "lidar_2d"), sensor("lidar_front", "lidar_2d")]),
            ..minimal()
        };
        let e = cfg.Resolved().unwrap_err();
        assert!(e.contains("lidar_front"), "错误信息应含重复的 slot 名：{e}");
        assert!(e.contains("重复"), "错误信息应说明重复：{e}");
    }

    #[test]
    fn test_resolved_empty_slot() {
        let cfg = WorldLinkConfig { sensor: Some(vec![sensor("   ", "lidar_2d")]), ..minimal() };
        let e = cfg.Resolved().unwrap_err();
        assert!(e.contains("slot"), "错误信息应指向 slot：{e}");
    }

    #[test]
    fn test_resolved_slot_charset() {
        for bad in ["lidar.front", "lidar:front", "lidar front", "lidar@1"] {
            let cfg = WorldLinkConfig { sensor: Some(vec![sensor(bad, "lidar_2d")]), ..minimal() };
            let e = cfg.Resolved().unwrap_err();
            assert!(e.contains(bad.trim()), "错误信息应含非法 slot：{e}");
        }
        // 合法字符集
        let cfg = WorldLinkConfig {
            sensor: Some(vec![sensor("lidar_front-1", "lidar_2d")]),
            ..minimal()
        };
        assert!(cfg.Resolved().is_ok());
    }

    #[test]
    fn test_resolved_backoff_bad() {
        let e = WorldLinkConfig { reconnect_ms_min: Some(0), ..minimal() }.Resolved().unwrap_err();
        assert!(e.contains("reconnect_ms_min"), "错误信息应指向 reconnect_ms_min：{e}");

        let e = WorldLinkConfig { reconnect_ms_min: Some(5000), reconnect_ms_max: Some(3000), ..minimal() }
            .Resolved()
            .unwrap_err();
        assert!(e.contains("reconnect_ms_min"), "错误信息应指向 min>max：{e}");
    }
}
