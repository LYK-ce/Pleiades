//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-21

#![allow(non_snake_case)]

//! SIM（模拟车/机）配置：无硬件，只复用共享段 + 障碍膨胀半径 + 底盘速度绑定 + 世界链路
//!
//! `[sim_world]` 段（task_31 §四.3）与 ugv 的 `[lidar]` **不复用**：
//! 语义不同——这里写的是**世界零件目录名**（`diff_drive_ugv` / `quadrotor_uav` / `lidar_2d`），
//! 不是车体型号。整段缺失 = `None` = 不连世界（等价现状：纯决策链空跑）。
//!
//! `[chassis]` 段（task_31 §六.5.1）与 pleiades-ugv 的 `[chassis]` **同名同字段同默认**：
//! sim 只读 `enabled` / `forward_speed` / `turn_speed`（0~100 百分比，默认 30/10），
//! `port` / `baudrate` / `car_type` 等硬件键由 serde 忽略——同一份 TOML 被两个进程各解释一遍。
//! 整段缺失 = 不建底盘（§六.5.7：无 `[chassis]` → `chassis = None` → `on_tick` 保持 Idle）。
//!
//! `[control]` 段（task_31 §七.3，Step 5）是**测试夹具**：本地控制口，**默认开启**
//! （配置里完全没有该段也按 `enabled = true` / `127.0.0.1` / `9098` 起监听）；
//! `enabled = false` 才是关闭。只绑本机回环（非回环地址一律强制改回 `127.0.0.1`）。

use serde::Deserialize;

use pleiades_base::config::{config_file_path, BaseConfig};

use crate::world_link::config::WorldLinkConfig;

// ================================================================ [control] 默认值（Step 5）

/// 默认**开启**（§七.3）：测试夹具若默认关，端到端验收每次都得改配置
pub const DEFAULT_CONTROL_ENABLED: bool = true;
/// 只绑本机（§七.8-1：严禁 0.0.0.0）
pub const DEFAULT_CONTROL_BIND: &str = "127.0.0.1";
/// 默认端口（Step 7 的验收脚本按它连）
pub const DEFAULT_CONTROL_PORT: u16 = 9098;

/// 底盘速度绑定（task_31 §六.5.1）：与 pleiades-ugv 的 `[chassis]` 同名同字段、同默认值
/// （真机 `robot_handler.rs:99-100` 的 `forward_speed` / `turn_speed`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChassisConfig {
    /// 缺省 true（段存在即建底盘；§六.5.7 的「无段 → None」由 `Option` 表达）
    pub enabled: Option<bool>,
    /// 前进/后退速度档位（0~100 百分比，缺省 30）
    pub forward_speed: Option<i32>,
    /// 转向速度档位（0~100 百分比，缺省 10）；UAV 的 `up`/`down` 也复用它（G3 近似）
    pub turn_speed: Option<i32>,
}

/// 本地控制口配置（task_31 §七.3，Step 5）：**测试夹具**，默认开启、只绑本机、无鉴权
///
/// 整段缺失 → `SimConfig.control = None` → 调用方 `unwrap_or_default()` → 默认值（仍是开启）。
/// 三个字段全 `Option`：段存在但没写某个键 → 该键取默认。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ControlConfig {
    /// 缺省 true（段存在但没写也开；写 false 才关）
    pub enabled: Option<bool>,
    /// 缺省 "127.0.0.1"（严禁 0.0.0.0；非回环值由 [`ControlConfig::Resolved`] 强制改回）
    pub bind: Option<String>,
    /// 缺省 9098；允许 `0`（OS 分配，日志打真实端口，便于自动化测试避开端口冲突）
    pub port: Option<u16>,
}

/// 填完默认值的控制口配置（`control::Spawn` 的入参）
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedControl {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
}

/// 是否本机回环：`IpAddr::is_loopback()` 或字面 `localhost`（大小写不敏感，容忍 `[::1]`）
pub fn Is_Loopback(host: &str) -> bool {
    let h = host.trim();
    if h.is_empty() {
        return false;
    }
    if h.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // IPv6 字面量可能写成 `[::1]`（`ToSocketAddrs` 两种都接受）
    let h = h.trim_start_matches('[').trim_end_matches(']');
    h.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

impl ControlConfig {
    /// `enabled` 缺省 = true（与 [`WorldLinkConfig::Enabled`] 语义完全一致）
    pub fn Enabled(&self) -> bool {
        self.enabled.unwrap_or(DEFAULT_CONTROL_ENABLED)
    }

    /// 填默认值；**永不 Err**（测试夹具配错不该拖死仿真）。
    ///
    /// `bind` 为空 → 默认；非回环 → **强制** `127.0.0.1`（§七.8-1「严禁 0.0.0.0」）。
    /// 强制这件事由 `control::Spawn` 比对原始值后打 `warn!`，本函数只保证值安全。
    pub fn Resolved(&self) -> ResolvedControl {
        let raw_bind = self.bind.as_deref().map(str::trim).unwrap_or("");
        let bind = if !raw_bind.is_empty() && Is_Loopback(raw_bind) {
            raw_bind.to_string()
        } else {
            DEFAULT_CONTROL_BIND.to_string()
        };
        ResolvedControl {
            enabled: self.Enabled(),
            bind,
            port: self.port.unwrap_or(DEFAULT_CONTROL_PORT),
        }
    }
}

/// SIM 完整配置 = 共享段（flatten）+ 障碍膨胀半径（寻路动态障碍用）+ 底盘 + 世界链路
#[derive(Debug, Clone, Deserialize)]
pub struct SimConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 他车障碍膨胀半径（米，缺省 0.2）
    pub obstacle_inflation_radius: Option<f32>,
    /// 底盘速度绑定（`[chassis]` 段）；无该段 = `None` = 不建底盘（车不会动）
    pub chassis: Option<ChassisConfig>,
    /// 世界链路（`[sim_world]` 段）；无该段 = `None` = 不连世界
    pub sim_world: Option<WorldLinkConfig>,
    /// 本地控制口（`[control]` 段）；无该段 = `None` = **仍按默认开启起**（§七.3）
    pub control: Option<ControlConfig>,
}

/// 读取 SIM 配置（.config/config.toml；文件由 base 的 Ensure_Config 在 core_bootstrap 时创建）
pub fn Ensure_Sim_Config() -> Result<SimConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    let content = std::fs::read_to_string(&path)?;
    let config: SimConfig = toml::from_str(&content)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sim_config_without_sim_world() {
        // 无 [sim_world] 段 -> None，且不报错（等价现状）
        let toml_s = r#"
[Log]
level = "info"

[Identity]
peer_name = "simA"
node_type = "car"
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("无 [sim_world] 段必须能解析");
        assert!(cfg.sim_world.is_none());
        assert_eq!(cfg.obstacle_inflation_radius, None);
        assert_eq!(cfg.base.Log.as_ref().and_then(|l| l.level.as_deref()), Some("info"));
    }

    #[test]
    fn test_sim_config_toml_full() {
        // 逐字对齐 task_31 §四.3 的段（段名取 [sim_world]）
        let toml_s = r#"
[Log]
level = "debug"

[sim_world]
enabled = true
url = "ws://127.0.0.1:9999"
chassis = "diff_drive_ugv"
reconnect_ms_min = 200
reconnect_ms_max = 3000
handshake_timeout_ms = 8000

[sim_world.chassis_params]
max_speed = 1.5
max_turn_rate = 90

[[sim_world.sensor]]
slot = "lidar_front"
sensor = "lidar_2d"

[sim_world.sensor.mount]
x = 0.1
y = 0.0
z = 0.2
yaw = 0.0

[sim_world.sensor.params]
range = 12.0
rays = 360
rate_hz = 10
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("整段 [sim_world] 必须能解析");
        let w = cfg.sim_world.as_ref().expect("[sim_world] 段应被解析出来");
        assert_eq!(w.Enabled(), true);
        assert_eq!(w.url.as_deref(), Some("ws://127.0.0.1:9999"));
        assert_eq!(w.chassis.as_deref(), Some("diff_drive_ugv"));
        assert_eq!(w.reconnect_ms_min, Some(200));
        assert_eq!(w.reconnect_ms_max, Some(3000));
        assert_eq!(w.handshake_timeout_ms, Some(8000));

        // TOML 表 -> serde_json::Value 的桥（R24 的守卫）
        let cp = w.chassis_params.as_ref().expect("chassis_params 应是 JSON 对象");
        assert_eq!(cp["max_speed"].as_f64(), Some(1.5));
        assert_eq!(cp["max_turn_rate"].as_f64(), Some(90.0));

        let sensors = w.sensor.as_ref().expect("sensor 数组");
        assert_eq!(sensors.len(), 1);
        let s = &sensors[0];
        assert_eq!(s.slot, "lidar_front");
        assert_eq!(s.sensor, "lidar_2d");
        let mount = s.mount.as_ref().expect("sensor.mount");
        assert_eq!(mount.x, Some(0.1));
        assert_eq!(mount.y, Some(0.0));
        assert_eq!(mount.z, Some(0.2));
        assert_eq!(mount.yaw, Some(0.0));
        let params = s.params.as_ref().expect("sensor.params");
        assert_eq!(params["range"].as_f64(), Some(12.0));
        assert_eq!(params["rays"].as_f64(), Some(360.0));
        assert_eq!(params["rate_hz"].as_f64(), Some(10.0));

        // 解析后的整段也必须能过校验
        let r = w.Resolved().expect("§四.3 的示例配置必须合法");
        assert_eq!(r.sensors.len(), 1);
        assert_eq!(r.sensors[0].mount.x, 0.1);
    }

    // ============================================================ [chassis]（Step 4）

    #[test]
    fn test_sim_config_chassis_section() {
        let toml_s = r#"
[chassis]
enabled = true
forward_speed = 55
turn_speed = 7
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("[chassis] 段必须能解析");
        let c = cfg.chassis.as_ref().expect("[chassis] 段应被解析出来");
        assert_eq!(c.enabled, Some(true));
        assert_eq!(c.forward_speed, Some(55));
        assert_eq!(c.turn_speed, Some(7));

        // 段存在但缺键 → 全部 None（默认值由 SpeedBinding::From_Config 填 30/10）
        let cfg: SimConfig = toml::from_str("[chassis]\n").expect("空 [chassis] 段必须能解析");
        let c = cfg.chassis.as_ref().expect("空段也是 Some");
        assert_eq!(c.enabled, None);
        assert_eq!(c.forward_speed, None);
        assert_eq!(c.turn_speed, None);
    }

    #[test]
    fn test_sim_config_chassis_ignores_ugv_keys() {
        // 同一段里带 ugv 的硬件键（port / baudrate / car_type）仍能解析：serde 忽略未知键
        let toml_s = r#"
[chassis]
enabled = true
port = "/dev/ttyUSB0"
baudrate = 115200
car_type = "X3Plus"
forward_speed = 30
turn_speed = 10
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("ugv 的硬件键必须被忽略而不是报错");
        let c = cfg.chassis.as_ref().expect("[chassis] 段应被解析出来");
        assert_eq!(c.forward_speed, Some(30));
        assert_eq!(c.turn_speed, Some(10));
    }

    #[test]
    fn test_sim_config_without_chassis() {
        // 无 [chassis] → None（= 不建底盘，§六.5.7），且整份配置不报错
        let toml_s = r#"
[Log]
level = "info"
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("无 [chassis] 段必须能解析");
        assert!(cfg.chassis.is_none());
        assert!(cfg.sim_world.is_none());
    }

    // ============================================================ [control]（Step 5）

    #[test]
    fn test_control_default_enabled_when_section_absent() {
        // 硬验收 3 的「默认开启」证据链：
        //   SimConfig.control = None → unwrap_or_default() → ControlConfig::default()
        //   → Enabled() == true（默认值 DEFAULT_CONTROL_ENABLED == true）
        let toml_s = r#"
[Log]
level = "info"

[Identity]
peer_name = "simA"
node_type = "car"
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("无 [control] 段必须能解析");
        assert!(cfg.control.is_none(), "无 [control] 段 → None");

        // start() 里的实际写法：`self.config.control.clone().unwrap_or_default()`
        let c = cfg.control.clone().unwrap_or_default();
        assert_eq!(DEFAULT_CONTROL_ENABLED, true);
        assert!(c.Enabled(), "无 [control] 段时必须默认开启");
        let r = c.Resolved();
        assert!(r.enabled);
        assert_eq!(r.bind, "127.0.0.1", "只绑本机");
        assert_eq!(r.port, 9098);
        assert_eq!(r.bind, DEFAULT_CONTROL_BIND);
        assert_eq!(r.port, DEFAULT_CONTROL_PORT);
    }

    #[test]
    fn test_control_disabled() {
        // 硬验收 3 的「可关闭」：enabled = false → Enabled() == false 且 Resolved().enabled == false
        let toml_s = r#"
[control]
enabled = false
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("[control] 段必须能解析");
        let c = cfg.control.as_ref().expect("[control] 段应被解析出来");
        assert_eq!(c.enabled, Some(false));
        assert!(!c.Enabled(), "enabled = false 必须关闭");
        let r = c.Resolved();
        assert!(!r.enabled);
        // 关闭时其余字段仍填默认值（日志/对账用）
        assert_eq!(r.bind, "127.0.0.1");
        assert_eq!(r.port, 9098);
    }

    #[test]
    fn test_control_section_parse() {
        // 段存在但只写部分键 → 其余取默认
        let cfg: SimConfig = toml::from_str("[control]\n").expect("空 [control] 段必须能解析");
        let c = cfg.control.as_ref().expect("空段也是 Some");
        assert_eq!(c.enabled, None);
        assert!(c.Enabled(), "段存在但没写 enabled → 默认 true");
        assert_eq!(c.Resolved(), ResolvedControl {
            enabled: true,
            bind: DEFAULT_CONTROL_BIND.to_string(),
            port: DEFAULT_CONTROL_PORT,
        });

        // 显式覆盖 bind / port
        let cfg: SimConfig = toml::from_str(
            r#"
[control]
enabled = true
bind = "127.0.0.1"
port = 9191
"#,
        )
        .expect("显式 [control] 必须能解析");
        let r = cfg.control.as_ref().expect("Some").Resolved();
        assert!(r.enabled);
        assert_eq!(r.bind, "127.0.0.1");
        assert_eq!(r.port, 9191, "port 原样生效");

        // port = 0（OS 分配）合法
        let cfg: SimConfig = toml::from_str("[control]\nport = 0\n").expect("port=0 必须能解析");
        assert_eq!(cfg.control.as_ref().expect("Some").Resolved().port, 0);
    }

    #[test]
    fn test_control_non_loopback_forced() {
        // §七.8-1：严禁 0.0.0.0 —— 非回环 bind 一律强制改回 127.0.0.1（不返回 Err）
        for bad in ["0.0.0.0", "192.168.1.2", "::", "example.com", " "] {
            let c = ControlConfig { enabled: None, bind: Some(bad.to_string()), port: None };
            let r = c.Resolved();
            assert_eq!(r.bind, "127.0.0.1", "bind='{bad}' 必须被强制改回本机回环");
            assert!(r.enabled, "强制 bind 不该影响 enabled");
        }
        // 合法回环原样保留
        for good in ["127.0.0.1", "127.0.0.5", "::1", "localhost", "LOCALHOST", "[::1]"] {
            let c = ControlConfig { enabled: None, bind: Some(good.to_string()), port: None };
            assert_eq!(c.Resolved().bind, good, "回环地址 '{good}' 必须原样保留");
        }
    }

    #[test]
    fn test_is_loopback() {
        for good in ["127.0.0.1", "127.0.0.5", "::1", "localhost", "  127.0.0.1  ", "[::1]"] {
            assert!(Is_Loopback(good), "'{good}' 是本机回环");
        }
        for bad in ["0.0.0.0", "192.168.1.2", "", "   ", "::", "example.com", "10.0.0.1"] {
            assert!(!Is_Loopback(bad), "'{bad}' 不是本机回环");
        }
    }
}
