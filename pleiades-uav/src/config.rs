//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-29

//! UAV（机）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）
//!
//! Task 23 §3.8：设备段由设备端自己负责。本文件除定义机独有字段外，
//! 还负责首次启动时把机端设备段补全到 config.toml。
//!
//! Task 34 Step 1：相机拆成**两段互不继承的配置**——
//! - `[camera_pc]`：开发机（电脑）摄像头，沿用 NV12 实现，**默认 `false`**；
//! - `[camera_uav]`：UAV 机载相机（SY011HD / UVC），MJPG 直通 + 格式协商，**默认 `true`**。
//!
//! 旧 `[camera]` 段（Task 28/29）**保留解析但一律不读取**：仍然派生
//! `Debug/Clone/Deserialize`，代码里只判 `is_some()`（bootstrap 打一行「已忽略」提示），
//! 免得老配置文件直接解析失败；同时 `fill_*` 不再生成它，避免自己刚补出来又自己 warn。
//!
//! task_37 Step 2：新增 `[detector]` 段（`enabled` / `model` / `device` /
//! `conf_thres` / `nms_thres`，默认**关**），键名与 `pleiades-sim` 的 `[detector]` **逐字一致**。
//!
//! task_37 Step 3：新增 `[detector.camera]` 子表（内参 `fov_deg` + `fov_axis`、外参
//! `mount_x/y/z` / `mount_yaw_deg` / `mount_pitch_deg`、`ground_z` / `max_range_m`，
//! 共 9 个键，§四.4 逐字），供「像素 → 世界 (x,y)」的地面投影解析参数。

#![allow(non_snake_case)]

use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

use pleiades_base::config::{config_file_path, BaseConfig};

/// `[camera_uav].resolutions` 的默认值（缺失时写入，也作为运行时缺省）
pub const DEFAULT_UAV_RESOLUTIONS: [[u32; 2]; 2] = [[640, 480], [1280, 720]];
/// `[camera_uav].open_timeout_ms` 的默认值（毫秒）
pub const DEFAULT_UAV_OPEN_TIMEOUT_MS: u64 = 8_000;
/// `[camera_uav].capture_timeout_ms` 的默认值（毫秒）
pub const DEFAULT_UAV_CAPTURE_TIMEOUT_MS: u64 = 5_000;
/// `[camera_pc] / [camera_uav].stream_fps` 的默认值（常转帧率，Hz）
pub const DEFAULT_STREAM_FPS: u32 = 10;
/// `[camera_pc] / [camera_uav].stream_fps` 的上限（超过按上限算，避免把设备打到异常状态）
pub const MAX_STREAM_FPS: u32 = 60;

/// 顶层键 `plan_budget_ms` 的缺省值（毫秒）：D* 单拍规划时间预算（Task 39 §二）
pub const DEFAULT_PLAN_BUDGET_MS: u64 = 20;

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

/// UAV 完整配置 = 共享段（flatten）+ 机独有设备段
#[derive(Debug, Clone, Deserialize)]
pub struct UavConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 飞控设备配置（MAVLink）
    pub flight_ctrl: Option<FlightCtrlConfig>,
    /// 他车障碍膨胀半径（米，缺省 0.2；「天上小车」2D 寻路阶段使用）
    pub obstacle_inflation_radius: Option<f32>,
    /// 单拍规划时间预算（毫秒，缺省 20；Task 39 §二；**顶层键**，与
    /// `obstacle_inflation_radius` 同级 —— 机没有 `[chassis]` 段，故不进任何段）
    pub plan_budget_ms: Option<u64>,
    /// **旧**摄像头段（Task 28/29）：保留解析，运行时**只判 `is_some()`** 打迁移提示，
    /// 任何字段都不读取（Task 34 Step 1）
    pub camera: Option<CameraConfig>,
    /// 开发机（电脑）摄像头 sensor（Task 34 Step 1）
    pub camera_pc: Option<CameraPcConfig>,
    /// UAV 机载相机 sensor（Task 34 Step 1）
    pub camera_uav: Option<CameraUavConfig>,
    /// detector 设备（task_37 Step 2，`[detector]` 段）；无该段 = `None` = 不 spawn
    pub detector: Option<DetectorConfig>,
}

/// 飞控设备配置（Pixhawk / MAVLink）
#[derive(Debug, Clone, Deserialize)]
pub struct FlightCtrlConfig {
    pub enabled: Option<bool>,
    /// 连接方式（如串口 /dev/ttyS0 或 MAVLink UDP 地址）
    pub connection: Option<String>,
    /// 串口波特率（默认 921600）
    pub baudrate: Option<u32>,
    /// 前进/后退速度 (m/s，缺省 0.3，Task 22_5 D2：速度由设备层绑定)
    pub vel_fwd: Option<f32>,
    /// 转向角速度 (°/s，缺省 15)
    pub yaw_rate_deg: Option<f32>,
}

/// **旧**摄像头设备配置（USB UVC，Task 28/29）。
///
/// Task 34 Step 1 起只用于「检测到旧段 → 提示改用 `[camera_pc]`/`[camera_uav]`」，
/// 字段保留解析以兼容老配置文件，运行时**不读取**。
///
/// `#[allow(dead_code)]` 是必要的：`derive(Debug)` 不算「读字段」，但旧段只被
/// `is_some()` 判一下，否则会多出一条 `fields ... are never read` warning（本仓 warning 数要持平基线）。
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CameraConfig {
    pub enabled: Option<bool>,
    /// 设备路径：纯数字 = 索引（Windows 常用 "0"），否则 = 路径（Linux 常用 "/dev/video0"）
    pub path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub timeout_ms: Option<u64>,
}

/// 开发机（电脑）摄像头 sensor 配置（`[camera_pc]`，默认关）
#[derive(Debug, Clone, Deserialize)]
pub struct CameraPcConfig {
    pub enabled: Option<bool>,
    /// 设备路径：`"0"` / `"1"` 或 `"/dev/video0"`（**不接受**其它字符串路径）
    pub path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub timeout_ms: Option<u64>,
    /// 常转开关：`true`（默认）= worker 按 `stream_fps` 一直抓帧写槽位；`false` = 不抓、不占 CPU
    pub stream: Option<bool>,
    /// 常转帧率（Hz，缺省 10，`0` 视为缺省，上限 60）；**与 `fps` 无关**（后者是格式协商请求帧率）
    pub stream_fps: Option<u32>,
}

/// UAV 机载相机 sensor 配置（`[camera_uav]`，默认开）
#[derive(Debug, Clone, Deserialize)]
pub struct CameraUavConfig {
    pub enabled: Option<bool>,
    /// 设备路径：`"0"` / `"1"` 或 `"/dev/video0"`
    pub path: Option<String>,
    /// 优先像素格式：`"mjpg"`（默认）或 `"raw"`
    pub prefer: Option<String>,
    /// 首选分辨率（按顺序尝试；`[]` = 无分辨率偏好，直接走 Closest 兜底）
    pub resolutions: Option<Vec<[u32; 2]>>,
    /// 目标帧率：`0`（默认）= 取该组合枚举到的最高帧率
    pub fps: Option<u32>,
    pub open_timeout_ms: Option<u64>,
    pub capture_timeout_ms: Option<u64>,
    /// 常转开关（同 `CameraPcConfig::stream`）
    pub stream: Option<bool>,
    /// 常转帧率（Hz，缺省 10，`0` 视为缺省，上限 60）；**不复用 `fps`**
    pub stream_fps: Option<u32>,
}

/// 读取 UAV 配置（.config/config.toml）。
///
/// 首次启动时若 config.toml 缺失机端设备段，会先补全默认设备段再读取（幂等，不覆盖已有值）。
pub fn Ensure_Uav_Config() -> Result<UavConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    // 配置文件不存在则先由 base 生成共享段（防御性：正常流程 core_bootstrap 已生成）
    if !path.exists() {
        pleiades_base::config::Ensure_Config()?;
    }
    ensure_uav_section(&path)?;
    let content = fs::read_to_string(&path)?;
    let config: UavConfig = toml::from_str(&content)?;
    Ok(config)
}

/// 确保 config.toml 存在机端设备段（obstacle_inflation_radius / plan_budget_ms / flight_ctrl /
/// camera_pc / camera_uav），缺失才补默认值。
///
/// **不再**补旧 `[camera]` 段：已存在的旧段原样留在文件里（由 bootstrap 打一行「已忽略」），
/// 新文件不会凭空生出旧段。
fn ensure_uav_section(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut doc: DocumentMut = content.parse()?;
    let mut changed = false;

    if doc.get("obstacle_inflation_radius").is_none() {
        doc["obstacle_inflation_radius"] = toml_edit::value(0.2_f64);
        changed = true;
    }
    // Task 39 §二：规划时间预算（顶层键，缺键补 20 ms；已有值不覆盖）
    if doc.get("plan_budget_ms").is_none() {
        doc["plan_budget_ms"] = toml_edit::value(DEFAULT_PLAN_BUDGET_MS as i64);
        changed = true;
    }
    changed |= fill_flight_ctrl(&mut doc);
    changed |= fill_camera_pc(&mut doc);
    changed |= fill_camera_uav(&mut doc);
    changed |= fill_detector(&mut doc);

    if changed {
        fs::write(path, doc.to_string())?;
    }
    Ok(())
}

/// 补全 [flight_ctrl] 段（缺失才补，幂等）
fn fill_flight_ctrl(doc: &mut DocumentMut) -> bool {
    if doc.get("flight_ctrl").is_none() {
        doc["flight_ctrl"] = Item::Table(toml_edit::Table::new());
    }
    let fc = doc["flight_ctrl"].as_table_mut().expect("flight_ctrl 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 4] = [
        ("enabled", toml_edit::value(false)),
        ("baudrate", toml_edit::value(921600_i64)),
        ("vel_fwd", toml_edit::value(0.3_f64)),
        ("yaw_rate_deg", toml_edit::value(15.0_f64)),
    ];
    for (k, v) in defaults {
        if !fc.contains_key(k) {
            fc.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [camera_pc] 段（缺失才补，幂等；**不覆盖**用户已写的值）
fn fill_camera_pc(doc: &mut DocumentMut) -> bool {
    if doc.get("camera_pc").is_none() {
        doc["camera_pc"] = Item::Table(toml_edit::Table::new());
    }
    let Some(table) = doc["camera_pc"].as_table_mut() else {
        return false;
    };
    let mut changed = false;
    let defaults: [(&str, Item); 7] = [
        // 开发机摄像头默认关：真机（树莓派）上通常没有 /dev/video0
        ("enabled", toml_edit::value(false)),
        ("path", toml_edit::value("0")),
        ("width", toml_edit::value(1280_i64)),
        ("height", toml_edit::value(720_i64)),
        ("timeout_ms", toml_edit::value(3000_i64)),
        // 常转：默认开，10 Hz（`stream_fps` 是新增键，与上面无关的 `fps` 语义不同）
        ("stream", toml_edit::value(true)),
        (
            "stream_fps",
            toml_edit::value(i64::from(DEFAULT_STREAM_FPS)),
        ),
    ];
    for (k, v) in defaults {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [camera_uav] 段（缺失才补，幂等；**不覆盖**用户已写的值）
fn fill_camera_uav(doc: &mut DocumentMut) -> bool {
    if doc.get("camera_uav").is_none() {
        doc["camera_uav"] = Item::Table(toml_edit::Table::new());
    }
    let Some(table) = doc["camera_uav"].as_table_mut() else {
        return false;
    };
    let mut changed = false;

    let defaults: [(&str, Item); 6] = [
        // 机载相机默认开：树莓派上这是主力相机
        ("enabled", toml_edit::value(true)),
        ("path", toml_edit::value("0")),
        ("prefer", toml_edit::value("mjpg")),
        ("fps", toml_edit::value(0_i64)),
        // 常转：默认开，10 Hz（`fps` 仍是「格式协商请求帧率」，两者互不影响）
        ("stream", toml_edit::value(true)),
        (
            "stream_fps",
            toml_edit::value(i64::from(DEFAULT_STREAM_FPS)),
        ),
    ];
    for (k, v) in defaults {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }

    if !table.contains_key("resolutions") {
        // 嵌套数组：外层 Array 的每个元素是内层 Array → 渲染为 [[640, 480], [1280, 720]]
        let mut outer = toml_edit::Array::new();
        for pair in DEFAULT_UAV_RESOLUTIONS {
            let mut inner = toml_edit::Array::new();
            inner.push(i64::from(pair[0]));
            inner.push(i64::from(pair[1]));
            outer.push(inner);
        }
        table.insert("resolutions", toml_edit::value(outer));
        changed = true;
    }

    let timeouts: [(&str, Item); 2] = [
        (
            "open_timeout_ms",
            toml_edit::value(DEFAULT_UAV_OPEN_TIMEOUT_MS as i64),
        ),
        (
            "capture_timeout_ms",
            toml_edit::value(DEFAULT_UAV_CAPTURE_TIMEOUT_MS as i64),
        ),
    ];
    for (k, v) in timeouts {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }

    changed
}

/// 补全 [detector] 段（task_37 Step 2；缺失才补，幂等；**不覆盖**用户已写的值）
fn fill_detector(doc: &mut DocumentMut) -> bool {
    if doc.get("detector").is_none() {
        doc["detector"] = Item::Table(toml_edit::Table::new());
    }
    let Some(table) = doc["detector"].as_table_mut() else {
        return false;
    };
    let mut changed = false;
    let defaults: [(&str, Item); 5] = [
        // 默认关：权重不一定在场，且 detector 会吃满一个 CPU 核
        ("enabled", toml_edit::value(DEFAULT_DETECTOR_ENABLED)),
        ("model", toml_edit::value(DEFAULT_DETECTOR_MODEL)),
        ("device", toml_edit::value(DEFAULT_DETECTOR_DEVICE)),
        ("conf_thres", toml_edit::value(f64::from(DEFAULT_DETECTOR_CONF_THRES))),
        ("nms_thres", toml_edit::value(f64::from(DEFAULT_DETECTOR_NMS_THRES))),
    ];
    for (k, v) in defaults {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }
    // task_37 Step 3：`[detector.camera]` 子表（缺失才补、幂等、不覆盖用户值）
    changed |= fill_detector_camera(table);
    changed
}

/// 补全 `[detector.camera]` 子表（task_37 Step 3；缺失才补，幂等；**不覆盖**用户已写的值）
///
/// 数值一律用 **f64 字面量**：`f64::from(0.045_f32)` 会渲染成 `0.04500000178813934`
/// （f32→f64 的尾数），落盘难看；解析回 f32 后两种写法是同一个数。
/// 上一步 5 个键的渲染问题属于 Step 2 已 PASS 的落盘文本，本 step **不改**。
fn fill_detector_camera(table: &mut toml_edit::Table) -> bool {
    if !table.contains_key("camera") {
        table.insert("camera", Item::Table(toml_edit::Table::new()));
    }
    let Some(cam) = table.get_mut("camera").and_then(|i| i.as_table_mut()) else {
        return false;
    };
    let defaults: [(&str, Item); 9] = [
        ("fov_deg", toml_edit::value(70.0_f64)),
        ("fov_axis", toml_edit::value(DEFAULT_DETECTOR_FOV_AXIS)),
        ("mount_x", toml_edit::value(0.20_f64)),
        ("mount_y", toml_edit::value(0.0_f64)),
        ("mount_z", toml_edit::value(0.045_f64)),
        ("mount_yaw_deg", toml_edit::value(90.0_f64)),
        ("mount_pitch_deg", toml_edit::value(-15.0_f64)),
        ("ground_z", toml_edit::value(0.0_f64)),
        ("max_range_m", toml_edit::value(12.0_f64)),
    ];
    let mut changed = false;
    for (k, v) in defaults {
        if !cam.contains_key(k) {
            cam.insert(k, v);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}", tag, nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_uav_section_fills_defaults() {
        let dir = tmp_dir("uav_cfg_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();

        assert_eq!(parsed.obstacle_inflation_radius, Some(0.2));
        let f = parsed.flight_ctrl.expect("flight_ctrl 段应被补全");
        assert_eq!(f.enabled, Some(false));
        assert_eq!(f.baudrate, Some(921600));
        assert_eq!(f.vel_fwd, Some(0.3));
        assert_eq!(f.yaw_rate_deg, Some(15.0));

        // 旧段不再被凭空补出来（否则会立刻被自己的「已忽略」提示打脸）
        assert!(parsed.camera.is_none(), "不应自动生成旧 [camera] 段");

        let pc = parsed.camera_pc.expect("camera_pc 段应被补全");
        assert_eq!(pc.enabled, Some(false), "开发机摄像头默认关");
        assert_eq!(pc.path.as_deref(), Some("0"));
        assert_eq!(pc.width, Some(1280));
        assert_eq!(pc.height, Some(720));
        assert_eq!(pc.timeout_ms, Some(3000));
        assert_eq!(pc.stream, Some(true), "常转默认开");
        assert_eq!(pc.stream_fps, Some(DEFAULT_STREAM_FPS));

        let uav = parsed.camera_uav.expect("camera_uav 段应被补全");
        assert_eq!(uav.enabled, Some(true), "机载相机默认开");
        assert_eq!(uav.path.as_deref(), Some("0"));
        assert_eq!(uav.prefer.as_deref(), Some("mjpg"));
        assert_eq!(
            uav.resolutions,
            Some(vec![[640, 480], [1280, 720]]),
            "resolutions 默认应为 640x480 在前、1280x720 在后"
        );
        assert_eq!(uav.fps, Some(0));
        // `fps`（格式协商请求帧率）与 `stream_fps`（常转帧率）是**两个独立键**，同存不冲突
        assert_eq!(uav.stream, Some(true), "常转默认开");
        assert_eq!(uav.stream_fps, Some(10), "常转帧率默认 10 Hz");
        assert_ne!(
            uav.stream_fps,
            uav.fps,
            "stream_fps 不得复用 fps（后者保持「格式协商请求帧率」原义）"
        );
        assert_eq!(uav.open_timeout_ms, Some(8000));
        assert_eq!(uav.capture_timeout_ms, Some(5000));

        // 落盘的文本要是标准 TOML 的两段独立表
        assert!(content.contains("[camera_pc]"), "{content}");
        assert!(content.contains("[camera_uav]"), "{content}");
        assert!(content.contains("[[640, 480], [1280, 720]]"), "{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_uav_section_keeps_existing() {
        let dir = tmp_dir("uav_cfg_keep");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[flight_ctrl]\nenabled = true\nbaudrate = 115200\n\n\
             [camera_uav]\nenabled = false\nprefer = \"raw\"\nresolutions = [[1920, 1080]]\n\
             stream = false\nstream_fps = 25\n",
        )
        .unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let f = parsed.flight_ctrl.unwrap();
        assert_eq!(f.enabled, Some(true));
        assert_eq!(f.baudrate, Some(115200));
        assert_eq!(f.vel_fwd, Some(0.3));
        assert_eq!(f.yaw_rate_deg, Some(15.0));

        // 用户写的 camera_uav 值一律不覆盖，只补缺的键
        let uav = parsed.camera_uav.unwrap();
        assert_eq!(uav.enabled, Some(false), "不得覆盖用户写的 enabled");
        assert_eq!(uav.prefer.as_deref(), Some("raw"), "不得覆盖用户写的 prefer");
        assert_eq!(
            uav.resolutions,
            Some(vec![[1920, 1080]]),
            "不得覆盖用户写的 resolutions"
        );
        assert_eq!(uav.fps, Some(0), "缺失的 fps 应被补成 0");
        assert_eq!(uav.open_timeout_ms, Some(8000), "缺失的 open_timeout_ms 应被补");
        assert_eq!(
            uav.capture_timeout_ms,
            Some(5000),
            "缺失的 capture_timeout_ms 应被补"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_uav_section_is_idempotent() {
        let dir = tmp_dir("uav_cfg_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_uav_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        assert_eq!(
            first.matches("[camera_pc]").count(),
            1,
            "不得重复插入 [camera_pc]：\n{first}"
        );
        assert_eq!(
            first.matches("[camera_uav]").count(),
            1,
            "不得重复插入 [camera_uav]：\n{first}"
        );
        assert_eq!(
            first.matches("resolutions").count(),
            1,
            "不得重复插入 resolutions：\n{first}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_camera_section_still_parses() {
        let dir = tmp_dir("uav_cfg_legacy");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[camera]\nenabled = true\npath = \"/dev/video2\"\nwidth = 640\nheight = 480\ntimeout_ms = 1500\n",
        )
        .unwrap();

        // 补段不能因为旧段而失败，也不能删/改旧段
        ensure_uav_section(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[camera]"), "旧段应原样保留：\n{content}");

        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let legacy = parsed.camera.expect("旧 [camera] 段仍应能解析");
        assert_eq!(legacy.enabled, Some(true));
        assert_eq!(legacy.path.as_deref(), Some("/dev/video2"));
        assert_eq!(legacy.width, Some(640));
        assert_eq!(legacy.height, Some(480));
        assert_eq!(legacy.timeout_ms, Some(1500));

        // 同时新段也被补全（新旧共存不冲突）
        assert!(parsed.camera_pc.is_some());
        assert!(parsed.camera_uav.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uav_defaults_when_fields_missing() {
        // 只写 enabled：其余键缺失时读成 None，由各 sensor 自己 unwrap_or 兜底
        let config: UavConfig = toml::from_str("[camera_uav]\nenabled = true\n").unwrap();
        let uav = config.camera_uav.expect("camera_uav 应解析出来");
        assert_eq!(uav.enabled, Some(true));
        assert_eq!(uav.path, None);
        assert_eq!(uav.prefer, None);
        assert_eq!(uav.resolutions, None);
        assert_eq!(uav.fps, None);
        assert_eq!(uav.open_timeout_ms, None);
        assert_eq!(uav.capture_timeout_ms, None);

        // 「缺省即默认开」的判断在 bootstrap：enabled 缺失 → unwrap_or(true)
        let empty: UavConfig = toml::from_str("[camera_uav]\n").unwrap();
        let uav = empty.camera_uav.expect("camera_uav 应解析出来");
        assert_eq!(uav.enabled.unwrap_or(true), true);

        // 两段都缺 → 都是 None（不影响解析）
        let bare: UavConfig = toml::from_str("[camera_pc]\n").unwrap();
        assert!(bare.camera_uav.is_none());
        assert!(bare.camera_pc.is_some());
    }

    #[test]
    fn empty_resolutions_means_no_preference() {
        use crate::device::camera_uav::format::{Choose_Format, Prefer_Format};
        use nokhwa::utils::{CameraFormat, FrameFormat, Resolution};

        let config: UavConfig = toml::from_str("[camera_uav]\nresolutions = []\n").unwrap();
        let uav = config.camera_uav.expect("camera_uav 应解析出来");
        assert_eq!(
            uav.resolutions,
            Some(Vec::new()),
            "显式空列表应读成空 Vec（= 无分辨率偏好）"
        );

        // 空分辨率表 → 直接走 Closest 兜底（不 panic、不取默认分辨率）
        let all = [CameraFormat::new(
            Resolution::new(1280, 720),
            FrameFormat::MJPEG,
            30,
        )];
        let resolutions: Vec<(u32, u32)> = uav
            .resolutions
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|pair| (pair[0], pair[1]))
            .collect();
        let (chosen, reason) = Choose_Format(&all, Prefer_Format::Mjpg, &resolutions, 0);
        assert_eq!(chosen, all[0]);
        assert!(reason.contains("Closest 兜底"), "{reason}");

        // 缺省（键缺失）时由 camera_uav 用 DEFAULT_UAV_RESOLUTIONS 兜底
        assert_eq!(DEFAULT_UAV_RESOLUTIONS, [[640, 480], [1280, 720]]);
    }

    // ============================================================ [detector]（task_37 Step 2）

    /// `[detector]` 段被补全成 §三.4 的五个默认值，且落盘文本里有该段
    #[test]
    fn detector_section_fills_defaults() {
        let dir = tmp_dir("uav_cfg_detector_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let d = parsed.detector.expect("[detector] 段应被补全");

        assert_eq!(d.enabled, Some(DEFAULT_DETECTOR_ENABLED), "默认关");
        assert_eq!(d.enabled, Some(false));
        assert_eq!(d.model.as_deref(), Some("yolov8n.safetensors"));
        assert_eq!(d.device.as_deref(), Some("cpu"));
        assert_eq!(d.conf_thres, Some(0.35));
        assert_eq!(d.nms_thres, Some(0.45));

        // 访问器与写进文件的默认值一致
        assert!(!d.Enabled(), "enabled 缺省 false");
        assert_eq!(d.Model(), "yolov8n.safetensors");
        assert_eq!(d.Device(), "cpu");
        assert_eq!(d.Conf_Thres(), DEFAULT_DETECTOR_CONF_THRES);
        assert_eq!(d.Nms_Thres(), DEFAULT_DETECTOR_NMS_THRES);

        assert!(content.contains("[detector]"), "{content}");
        // 补段幂等：不得重复插入
        assert_eq!(content.matches("[detector]").count(), 1, "{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 用户已写的值不覆盖；非法值（model 全空白 / conf_thres 越界）回默认且不 panic
    #[test]
    fn detector_section_keeps_existing_and_tolerates_illegal() {
        let dir = tmp_dir("uav_cfg_detector_keep");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[detector]\nenabled = true\nmodel = \"  \"\nconf_thres = 2.0\n",
        )
        .unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let d = parsed.detector.expect("[detector] 段应仍在");

        assert_eq!(d.enabled, Some(true), "不得覆盖用户写的 enabled");
        assert!(d.Enabled());
        assert_eq!(d.Model(), "yolov8n.safetensors", "空白 model 回默认");
        assert_eq!(d.Device(), "cpu", "缺失的 device 被补成默认");
        assert_eq!(d.Conf_Thres(), DEFAULT_DETECTOR_CONF_THRES, "越界阈值回默认");
        assert_eq!(d.Nms_Thres(), DEFAULT_DETECTOR_NMS_THRES);
        assert!(content.contains("conf_thres = 2.0"), "不得覆盖用户写的值：\n{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================================================== [detector.camera]（task_37 Step 3）

    /// `[detector.camera]` 被补成 §四.4 的 9 个默认值，落盘文本是干净的十进制
    #[test]
    fn detector_camera_section_fills_defaults() {
        let dir = tmp_dir("uav_cfg_detector_camera_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let d = parsed.detector.expect("[detector] 段应被补全");
        let cam = d.camera.expect("[detector.camera] 子表应被补全");

        assert_eq!(cam.fov_deg, Some(70.0));
        assert_eq!(cam.fov_axis.as_deref(), Some("vertical"));
        assert_eq!(cam.mount_x, Some(0.20));
        assert_eq!(cam.mount_y, Some(0.0));
        assert_eq!(cam.mount_z, Some(0.045));
        assert_eq!(cam.mount_yaw_deg, Some(90.0));
        assert_eq!(cam.mount_pitch_deg, Some(-15.0));
        assert_eq!(cam.ground_z, Some(0.0));
        assert_eq!(cam.max_range_m, Some(12.0));

        // 访问器与落盘默认值一致
        assert_eq!(cam.Fov_Deg(), 70.0);
        assert_eq!(cam.Fov_Axis(), "vertical");
        assert_eq!(cam.Mount_X(), 0.20);
        assert_eq!(cam.Mount_Y(), 0.0);
        assert_eq!(cam.Mount_Z(), 0.045);
        assert_eq!(cam.Mount_Yaw_Deg(), 90.0);
        assert_eq!(cam.Mount_Pitch_Deg(), -15.0);
        assert_eq!(cam.Ground_Z(), 0.0);
        assert_eq!(cam.Max_Range_M(), 12.0);

        // 落盘文本：有子表头、是干净的十进制（不是 f32 尾数长串）、不重复
        assert!(content.contains("[detector.camera]"), "{content}");
        assert!(content.contains("mount_z = 0.045"), "{content}");
        assert!(content.contains("mount_pitch_deg = -15.0"), "{content}");
        assert_eq!(content.matches("[detector.camera]").count(), 1, "{content}");
        assert_eq!(content.matches("[detector]").count(), 1, "既有断言不得回归：{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 用户已写的 camera 键不被覆盖；非法值访问器回默认且不 panic
    #[test]
    fn detector_camera_section_keeps_existing_and_tolerates_illegal() {
        let dir = tmp_dir("uav_cfg_detector_camera_keep");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[detector]\nenabled = true\n\n[detector.camera]\nfov_deg = 90.0\nfov_axis = \"horizontal\"\n\
             mount_pitch_deg = -25.0\nmax_range_m = 0.0\n",
        )
        .unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        let cam = parsed.detector.expect("[detector] 段应仍在").camera.expect("[detector.camera] 应仍在");

        assert_eq!(cam.fov_deg, Some(90.0), "不得覆盖用户写的 fov_deg");
        assert_eq!(cam.fov_axis.as_deref(), Some("horizontal"));
        assert_eq!(cam.mount_pitch_deg, Some(-25.0), "不得覆盖用户写的 mount_pitch_deg");
        // 缺失的键被补成默认
        assert_eq!(cam.mount_x, Some(0.20));
        assert_eq!(cam.mount_yaw_deg, Some(90.0));
        // 非法值：访问器回默认，但落盘文本原样保留用户写的值
        assert_eq!(cam.Fov_Deg(), 90.0);
        assert_eq!(cam.Max_Range_M(), DEFAULT_DETECTOR_MAX_RANGE_M, "0 非法 → 回默认");
        assert!(content.contains("max_range_m = 0.0"), "不得覆盖用户写的值：\n{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 补 `[detector.camera]` 也幂等
    #[test]
    fn detector_camera_section_is_idempotent() {
        let dir = tmp_dir("uav_cfg_detector_camera_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_uav_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        assert_eq!(first.matches("[detector.camera]").count(), 1, "{first}");
        assert_eq!(first.matches("mount_yaw_deg").count(), 1, "{first}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── plan_budget_ms（Task 39 §二：顶层键，缺键补 20、幂等、已有值不覆盖）───

    /// 缺键 → 补 20 ms（且是**顶层键**，不掉进任何段）
    #[test]
    fn ensure_uav_section_fills_plan_budget() {
        let dir = tmp_dir("uav_cfg_plan_budget_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"uav\"\n").unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(DEFAULT_PLAN_BUDGET_MS));
        assert_eq!(parsed.plan_budget_ms, Some(20), "缺键补 20 ms");
        assert!(content.contains("plan_budget_ms = 20"), "{content}");
        // 顶层键：不在任何段头之后（`[flight_ctrl]` 之前那一行）
        let top_pos = content.find("plan_budget_ms").expect("应落盘");
        let first_table = content.find("\n[").expect("应有段头");
        assert!(top_pos < first_table, "plan_budget_ms 必须是顶层键：\n{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 已有值不被覆盖
    #[test]
    fn plan_budget_existing_value_kept() {
        let dir = tmp_dir("uav_cfg_plan_budget_keep");
        let path = dir.join("config.toml");
        std::fs::write(&path, "plan_budget_ms = 50\n").unwrap();

        ensure_uav_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UavConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(50), "不得覆盖用户写的值");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 幂等：连补两次文件字节不变
    #[test]
    fn ensure_uav_section_plan_budget_idempotent() {
        let dir = tmp_dir("uav_cfg_plan_budget_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "plan_budget_ms = 50\n").unwrap();

        ensure_uav_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_uav_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        let parsed: UavConfig = toml::from_str(&second).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(50));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
