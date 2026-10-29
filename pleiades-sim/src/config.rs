//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-29

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
//!
//! `[detector]` 段（task_37 §三.4，Step 2）与 `pleiades-uav` 的 `[detector]`
//! **键名、默认值、访问器逐字一致**；无该段 = `None` = 不 spawn detector（零线程）。
//!
//! `[detector.camera]` 子表（task_37 §四.4，Step 3）：内参 `fov_deg` + `fov_axis`、外参
//! `mount_x/y/z` / `mount_yaw_deg` / `mount_pitch_deg`、`ground_z` / `max_range_m` 共 9 个键，
//! 键名/默认值与 `pleiades-uav` **逐字一致**；sim 侧**只读**（无补全函数）。

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

// ================================================================ [detector]（task_37 Step 2）

/// `[detector].enabled` 的默认值：**默认关**（权重不一定在场，且 detector 会吃满一个 CPU 核）
pub const DEFAULT_DETECTOR_ENABLED: bool = false;
/// `[detector].model` 的默认值：只写**文件名**，权重放 `<workspace_dir>/` 下
pub const DEFAULT_DETECTOR_MODEL: &str = "yolov8n.safetensors";
/// `[detector].device` 的默认值（`"cpu"` / `"cuda"` / `"cuda:N"`）
pub const DEFAULT_DETECTOR_DEVICE: &str = "cpu";
/// `[detector].conf_thres` 的默认值（置信度阈值）
pub const DEFAULT_DETECTOR_CONF_THRES: f32 = 0.35;
/// `[detector].nms_thres` 的默认值（NMS IoU 阈值）
pub const DEFAULT_DETECTOR_NMS_THRES: f32 = 0.45;

// ============================================================ [detector.camera]（task_37 Step 3）

/// `[detector.camera].fov_deg` 的默认值（度，全视角；与 world 侧 `CameraSensor.DEFAULT_FOV_DEG` 同值）
pub const DEFAULT_DETECTOR_FOV_DEG: f32 = 70.0;
/// `[detector.camera].fov_axis` 的默认值：`fov_deg` 指的是**纵向**全视角
pub const DEFAULT_DETECTOR_FOV_AXIS: &str = "vertical";
/// `[detector.camera].mount_x` 的默认值（米，机体系 `x` 前）
pub const DEFAULT_DETECTOR_MOUNT_X: f32 = 0.20;
/// `[detector.camera].mount_y` 的默认值（米，机体系 `y` 右）
pub const DEFAULT_DETECTOR_MOUNT_Y: f32 = 0.0;
/// `[detector.camera].mount_z` 的默认值（米，机体系 `z` 高）
pub const DEFAULT_DETECTOR_MOUNT_Z: f32 = 0.045;
/// `[detector.camera].mount_yaw_deg` 的默认值（度；光轴 = 局部 −Z，**前视 = +90°**）
pub const DEFAULT_DETECTOR_MOUNT_YAW_DEG: f32 = 90.0;
/// `[detector.camera].mount_pitch_deg` 的默认值（度；**负 = 低头**）
pub const DEFAULT_DETECTOR_MOUNT_PITCH_DEG: f32 = -15.0;
/// `[detector.camera].ground_z` 的默认值（米，地面高度）
pub const DEFAULT_DETECTOR_GROUND_Z: f32 = 0.0;
/// `[detector.camera].max_range_m` 的默认值（米，超过就拒答）
pub const DEFAULT_DETECTOR_MAX_RANGE_M: f32 = 12.0;

/// detector 设备配置（`[detector]` 段，task_37 §三.4）；**uav 与 sim 键名一致**
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DetectorConfig {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub device: Option<String>,
    pub conf_thres: Option<f32>,
    pub nms_thres: Option<f32>,
    /// `[detector.camera]` 子表（task_37 Step 3，§四.4）；缺段 = None = 全用默认值
    pub camera: Option<DetectorCameraConfig>,
}

impl DetectorConfig {
    /// `enabled` 缺省 **false**（段存在但没写也按关处理）
    pub fn Enabled(&self) -> bool {
        self.enabled.unwrap_or(DEFAULT_DETECTOR_ENABLED)
    }

    /// 权重**文件名**（去空白；空串 → 默认 `yolov8n.safetensors`）
    pub fn Model(&self) -> &str {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_DETECTOR_MODEL)
    }

    /// 推理设备（`"cpu"` / `"cuda"` / `"cuda:N"`；空串 → 默认 `"cpu"`）
    pub fn Device(&self) -> &str {
        self.device
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_DETECTOR_DEVICE)
    }

    /// 置信度阈值（缺省 0.35；非法（<0 或 >1 或 NaN）→ 回默认，不 panic）
    pub fn Conf_Thres(&self) -> f32 {
        self.conf_thres
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .unwrap_or(DEFAULT_DETECTOR_CONF_THRES)
    }

    /// NMS IoU 阈值（缺省 0.45；非法 → 回默认，不 panic）
    pub fn Nms_Thres(&self) -> f32 {
        self.nms_thres
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .unwrap_or(DEFAULT_DETECTOR_NMS_THRES)
    }
}

/// 相机内参 + 外参（`[detector.camera]` 子表，task_37 §四.4）；**uav 与 sim 键名一致**
///
/// 单位一律写进键名：角度键带 `_deg`、距离键是米。**故意**不同于 world 侧
/// `Vehicle.AttachSensor`（那里 yaw 用弧度、pitch 用度混着写，见 `Vehicle.gd:183-184`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DetectorCameraConfig {
    pub fov_deg: Option<f32>,
    pub fov_axis: Option<String>,
    pub mount_x: Option<f32>,
    pub mount_y: Option<f32>,
    pub mount_z: Option<f32>,
    pub mount_yaw_deg: Option<f32>,
    pub mount_pitch_deg: Option<f32>,
    pub ground_z: Option<f32>,
    pub max_range_m: Option<f32>,
}

impl DetectorCameraConfig {
    /// 视场角（度；缺省 70；非有限 / <=1 / >=179 → 回默认，不 panic）
    ///
    /// `1 < v < 179` 与 world 侧 `CameraSensor.gd:66-67` 的 `MIN_FOV_DEG/MAX_FOV_DEG` 同界。
    pub fn Fov_Deg(&self) -> f32 {
        self.fov_deg
            .filter(|v| v.is_finite() && *v > 1.0 && *v < 179.0)
            .unwrap_or(DEFAULT_DETECTOR_FOV_DEG)
    }

    /// 视场角口径（`"vertical"` / `"horizontal"`；去空白，空 → 默认）
    pub fn Fov_Axis(&self) -> &str {
        self.fov_axis
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_DETECTOR_FOV_AXIS)
    }

    /// 光心在机体系的 `x`（米；非有限 → 回默认）
    pub fn Mount_X(&self) -> f32 {
        self.mount_x.filter(|v| v.is_finite()).unwrap_or(DEFAULT_DETECTOR_MOUNT_X)
    }

    /// 光心在机体系的 `y`（米；非有限 → 回默认）
    pub fn Mount_Y(&self) -> f32 {
        self.mount_y.filter(|v| v.is_finite()).unwrap_or(DEFAULT_DETECTOR_MOUNT_Y)
    }

    /// 光心在机体系的 `z`（米；非有限 → 回默认）
    pub fn Mount_Z(&self) -> f32 {
        self.mount_z.filter(|v| v.is_finite()).unwrap_or(DEFAULT_DETECTOR_MOUNT_Z)
    }

    /// 光轴偏航（度；前视 = +90；非有限 → 回默认）
    pub fn Mount_Yaw_Deg(&self) -> f32 {
        self.mount_yaw_deg.filter(|v| v.is_finite()).unwrap_or(DEFAULT_DETECTOR_MOUNT_YAW_DEG)
    }

    /// 光轴俯仰（度；负 = 低头；非有限 → 回默认）
    pub fn Mount_Pitch_Deg(&self) -> f32 {
        self.mount_pitch_deg
            .filter(|v| v.is_finite())
            .unwrap_or(DEFAULT_DETECTOR_MOUNT_PITCH_DEG)
    }

    /// 地面高度（米，世界 `z`；非有限 → 回默认）
    pub fn Ground_Z(&self) -> f32 {
        self.ground_z.filter(|v| v.is_finite()).unwrap_or(DEFAULT_DETECTOR_GROUND_Z)
    }

    /// 最大距离（米；缺省 12；非有限 / <= 0 → 回默认）
    pub fn Max_Range_M(&self) -> f32 {
        self.max_range_m
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(DEFAULT_DETECTOR_MAX_RANGE_M)
    }
}

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

/// 飞控速度配置（task_39 §四 修订）：与 `pleiades-uav` 的 `[flight_ctrl]` **同名同单位**
///
/// sim 机栈的连续执行器只读这两个键；车栈仍用 `[chassis]` 的 `forward_speed/turn_speed`。
/// 整段缺失 / 缺键 → 取真机同款默认（`vel_fwd = 0.3 m/s`、`yaw_rate_deg = 15°/s`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FlightCtrlConfig {
    /// 前进速度（**米/秒**，缺省 0.3）
    pub vel_fwd: Option<f32>,
    /// 偏航角速度（**度/秒**，缺省 15）
    pub yaw_rate_deg: Option<f32>,
}

impl FlightCtrlConfig {
    /// 填默认值并夹到合法域：`vel_fwd ∈ [0, 5]` m/s、`yaw_rate ∈ [1, 180]` °/s。
    ///
    /// 非有限值或 ≤ 0 → 回落默认（配置写坏不该让仿真飞出去）。
    pub fn Resolved(&self) -> (f32, f32) {
        let v = self
            .vel_fwd
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(DEFAULT_VEL_FWD)
            .clamp(0.0, 5.0);
        let w = self
            .yaw_rate_deg
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(DEFAULT_YAW_RATE_DEG)
            .clamp(1.0, 180.0);
        (v, w)
    }
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

// ============================================================ 顶层键：规划预算（Task 39）

/// 顶层键 `plan_budget_ms` 的缺省值（毫秒）：D* 单拍规划时间预算
///
/// sim **不补全** config（`Ensure_Sim_Config` 只读文件，没有 `fill_*`），缺键 = `None` →
/// 调用方 `unwrap_or(DEFAULT_PLAN_BUDGET_MS)`（与 `obstacle_inflation_radius` 的先例一致）。
pub const DEFAULT_PLAN_BUDGET_MS: u64 = 20;

/// `[flight_ctrl].vel_fwd` 的缺省值（m/s）：与 `pleiades-uav` 的默认值一致（Task 22_5 D2）
pub const DEFAULT_VEL_FWD: f32 = 0.3;

/// `[flight_ctrl].yaw_rate_deg` 的缺省值（°/s）：与 `pleiades-uav` 的默认值一致
pub const DEFAULT_YAW_RATE_DEG: f32 = 15.0;

/// SIM 完整配置 = 共享段（flatten）+ 障碍膨胀半径（寻路动态障碍用）+ 底盘 + 世界链路
#[derive(Debug, Clone, Deserialize)]
pub struct SimConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 他车障碍膨胀半径（米，缺省 0.2）
    pub obstacle_inflation_radius: Option<f32>,
    /// 单拍规划时间预算（毫秒，缺省 20；Task 39 §二；**顶层键**，与
    /// `obstacle_inflation_radius` 同级 —— 不放 `[chassis]`，UAV 没有该段）
    pub plan_budget_ms: Option<u64>,
    /// 底盘速度绑定（`[chassis]` 段）；无该段 = `None` = 不建底盘（车不会动）
    pub chassis: Option<ChassisConfig>,
    /// 飞控速度（`[flight_ctrl]` 段，task_39 §四 修订）；无该段 = 机栈执行器用默认 0.3 / 15
    pub flight_ctrl: Option<FlightCtrlConfig>,
    /// 世界链路（`[sim_world]` 段）；无该段 = `None` = 不连世界
    pub sim_world: Option<WorldLinkConfig>,
    /// 本地控制口（`[control]` 段）；无该段 = `None` = **仍按默认开启起**（§七.3）
    pub control: Option<ControlConfig>,
    /// detector 设备（task_37 Step 2，`[detector]` 段）；无该段 = `None` = 不 spawn
    pub detector: Option<DetectorConfig>,
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
    fn test_sim_config_flight_ctrl_section() {
        // task_39 §四 修订：机栈速度与真机 UAV 同名同单位（m/s、°/s）
        let toml_s = r#"
[flight_ctrl]
vel_fwd = 0.8
yaw_rate_deg = 30.0
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("[flight_ctrl] 段必须能解析");
        let f = cfg.flight_ctrl.as_ref().expect("[flight_ctrl] 应被解析出来");
        assert_eq!(f.Resolved(), (0.8, 30.0));
    }

    #[test]
    fn test_sim_config_flight_ctrl_defaults_and_clamp() {
        // 无该段 → None（机栈执行器回落到常量默认 0.3 / 15）
        let cfg: SimConfig = toml::from_str("[chassis]\n").expect("仅 [chassis] 必须能解析");
        assert!(cfg.flight_ctrl.is_none());
        // 缺键 → Resolved 补默认
        assert_eq!(
            FlightCtrlConfig::default().Resolved(),
            (DEFAULT_VEL_FWD, DEFAULT_YAW_RATE_DEG)
        );
        // 非法值（负 / 非有限）→ 回落默认；超大 → 夹到上界
        let bad = FlightCtrlConfig { vel_fwd: Some(-1.0), yaw_rate_deg: Some(f32::INFINITY) };
        assert_eq!(bad.Resolved(), (DEFAULT_VEL_FWD, DEFAULT_YAW_RATE_DEG));
        let huge = FlightCtrlConfig { vel_fwd: Some(99.0), yaw_rate_deg: Some(999.0) };
        assert_eq!(huge.Resolved(), (5.0, 180.0));
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

    // ============================================================ [detector]（task_37 Step 2）

    #[test]
    fn test_detector_section_parses() {
        // 整段显式配置 → 五个访问器逐项生效（键名与 uav 的 [detector] 完全一致）
        let toml_s = r#"
[detector]
enabled = true
model = "yolov8s.safetensors"
device = "cuda:1"
conf_thres = 0.5
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("[detector] 段必须能解析");
        let d = cfg.detector.as_ref().expect("[detector] 段应被解析出来");
        assert!(d.Enabled(), "enabled = true");
        assert_eq!(d.Model(), "yolov8s.safetensors");
        assert_eq!(d.Device(), "cuda:1");
        assert_eq!(d.Conf_Thres(), 0.5);
        assert_eq!(d.Nms_Thres(), DEFAULT_DETECTOR_NMS_THRES, "缺失的 nms_thres 回默认");

        // 常量与 §三.4 的示例值一致
        assert_eq!(DEFAULT_DETECTOR_MODEL, "yolov8n.safetensors");
        assert_eq!(DEFAULT_DETECTOR_DEVICE, "cpu");
        assert_eq!(DEFAULT_DETECTOR_CONF_THRES, 0.35);
        assert_eq!(DEFAULT_DETECTOR_NMS_THRES, 0.45);
        assert!(!DEFAULT_DETECTOR_ENABLED, "detector 默认关");

        // 无 [detector] 段 → None（= 不 spawn，零线程）
        let cfg: SimConfig = toml::from_str("").expect("空配置必须能解析");
        assert!(cfg.detector.is_none(), "无 [detector] 段 → None");

        // 段存在但缺 enabled → None（访问器按 false 处理，不 panic）
        let cfg: SimConfig = toml::from_str("[detector]\n").expect("空 [detector] 段必须能解析");
        let d = cfg.detector.as_ref().expect("空段也是 Some");
        assert_eq!(d.enabled, None);
        assert!(!d.Enabled(), "段存在但没写 enabled → 默认 false");
        assert_eq!(d.Model(), DEFAULT_DETECTOR_MODEL);
        assert_eq!(d.Device(), DEFAULT_DETECTOR_DEVICE);

        // 非法值（空白 model / 空 device / 越界阈值 / NaN）→ 回默认，不 panic
        let bad = DetectorConfig {
            enabled: Some(true),
            model: Some("   ".to_string()),
            device: Some(String::new()),
            conf_thres: Some(2.0),
            nms_thres: Some(f32::NAN),
            camera: None, // task_37 Step 3：新增字段（穷举字面量必须补）
        };
        assert_eq!(bad.Model(), DEFAULT_DETECTOR_MODEL, "空白 model 回默认");
        assert_eq!(bad.Device(), DEFAULT_DETECTOR_DEVICE, "空 device 回默认");
        assert_eq!(bad.Conf_Thres(), DEFAULT_DETECTOR_CONF_THRES, "越界 conf 回默认");
        assert_eq!(bad.Nms_Thres(), DEFAULT_DETECTOR_NMS_THRES, "NaN nms 回默认");
    }

    // ==================================================== [detector.camera]（task_37 Step 3）

    #[test]
    fn test_detector_camera_section_parses() {
        // 整段显式配置 → 9 个访问器逐项生效（键名与 uav 的 [detector.camera] 完全一致）
        let toml_s = r#"
[detector]
enabled = true

[detector.camera]
fov_deg = 90.0
fov_axis = "horizontal"
mount_x = 0.5
mount_y = -0.1
mount_z = 0.3
mount_yaw_deg = 45.0
mount_pitch_deg = -30.0
ground_z = 1.5
max_range_m = 20.0
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("[detector.camera] 必须能解析");
        let cam = cfg.detector.expect("Some").camera.expect("[detector.camera] 应解析出来");
        assert_eq!(cam.Fov_Deg(), 90.0);
        assert_eq!(cam.Fov_Axis(), "horizontal");
        assert_eq!(cam.Mount_X(), 0.5);
        assert_eq!(cam.Mount_Y(), -0.1);
        assert_eq!(cam.Mount_Z(), 0.3);
        assert_eq!(cam.Mount_Yaw_Deg(), 45.0);
        assert_eq!(cam.Mount_Pitch_Deg(), -30.0);
        assert_eq!(cam.Ground_Z(), 1.5);
        assert_eq!(cam.Max_Range_M(), 20.0);

        // 子表缺失 → None（= 全默认值）；空子表 → None 且访问器回默认
        let cfg: SimConfig = toml::from_str("[detector]\nenabled = true\n").expect("缺子表必须能解析");
        assert!(cfg.detector.expect("Some").camera.is_none(), "缺 [detector.camera] → None");
        let cfg: SimConfig = toml::from_str("[detector.camera]\n").expect("空子表必须能解析");
        let cam = cfg.detector.expect("Some").camera.expect("空子表也是 Some");
        assert_eq!(cam.Fov_Deg(), DEFAULT_DETECTOR_FOV_DEG);
        assert_eq!(cam.Mount_Z(), DEFAULT_DETECTOR_MOUNT_Z);
        assert_eq!(cam.Max_Range_M(), DEFAULT_DETECTOR_MAX_RANGE_M);

        // 常量与 §四.4 的示例值一致
        assert_eq!(DEFAULT_DETECTOR_FOV_DEG, 70.0);
        assert_eq!(DEFAULT_DETECTOR_FOV_AXIS, "vertical");
        assert_eq!(DEFAULT_DETECTOR_MOUNT_X, 0.20);
        assert_eq!(DEFAULT_DETECTOR_MOUNT_Y, 0.0);
        assert_eq!(DEFAULT_DETECTOR_MOUNT_Z, 0.045);
        assert_eq!(DEFAULT_DETECTOR_MOUNT_YAW_DEG, 90.0);
        assert_eq!(DEFAULT_DETECTOR_MOUNT_PITCH_DEG, -15.0);
        assert_eq!(DEFAULT_DETECTOR_GROUND_Z, 0.0);
        assert_eq!(DEFAULT_DETECTOR_MAX_RANGE_M, 12.0);
    }

    // ==================================================== plan_budget_ms（Task 39 §二）

    #[test]
    fn test_sim_config_plan_budget_absent_defaults_20() {
        // 缺键 → None（sim 没有 fill_*，不写回文件）；调用方 unwrap_or 得 20
        let toml_s = r#"
[Log]
level = "info"

[Identity]
peer_name = "simA"
node_type = "car"
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("无 plan_budget_ms 必须能解析");
        assert_eq!(cfg.plan_budget_ms, None, "缺键 → None（sim 不补全 config）");
        assert_eq!(DEFAULT_PLAN_BUDGET_MS, 20);
        assert_eq!(
            cfg.plan_budget_ms.unwrap_or(DEFAULT_PLAN_BUDGET_MS),
            20,
            "缺键按缺省 20 ms 用"
        );
    }

    #[test]
    fn test_sim_config_plan_budget_parses() {
        // 顶层键（不在任何段里）→ Some(50)；与 [chassis] 同级
        let cfg: SimConfig = toml::from_str("plan_budget_ms = 50\n").expect("顶层键必须能解析");
        assert_eq!(cfg.plan_budget_ms, Some(50));

        let toml_s = r#"
plan_budget_ms = 35

[chassis]
enabled = true
"#;
        let cfg: SimConfig = toml::from_str(toml_s).expect("顶层键 + [chassis] 必须能解析");
        assert_eq!(cfg.plan_budget_ms, Some(35));
        assert!(cfg.chassis.is_some());
    }
}
