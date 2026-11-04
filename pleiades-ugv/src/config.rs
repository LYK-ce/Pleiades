//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-11-04

//! UGV（车）设备配置（Task 23 C0：设备段从 base 拆出，`#[serde(flatten)]` 复用共享段）
//!
//! Task 23 §3.8：设备段由设备端自己负责。本文件除定义车独有字段外，
//! 还负责首次启动时把车端设备段（chassis/lidar/obstacle_inflation_radius）补全到 config.toml。

use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml_edit::{DocumentMut, Item};

use pleiades_base::config::{config_file_path, BaseConfig};

/// 顶层键 `plan_budget_ms` 的缺省值（毫秒）：D* 单拍规划时间预算（Task 39 §二）
pub const DEFAULT_PLAN_BUDGET_MS: u64 = 20;

/// `[chassis] cruise_speed` 缺省：巡航线速度（m/s，task_44 §1.2 #11 定值 0.45）
pub const DEFAULT_CRUISE_SPEED: f32 = 0.45;
/// `[chassis] cruise_yaw_rate` 缺省：巡航偏航角速度绝对值（rad/s，task_44 §1.2 #11 定值 0.5）
pub const DEFAULT_CRUISE_YAW_RATE: f32 = 0.5;

// ─── [camera_ugv] 缺省值（task_43 §三；设备路径与加密配置来自实机验证材料）───

/// 厂商 SDK 库的缺省绝对路径（Jetson 车上的安装位置；可用 `sdk_lib` 覆盖）
pub const DEFAULT_CAMERA_UGV_SDK_LIB: &str =
    "/home/jetson/yahboomcar_ros2_ws/software/library_ws/src/ascamera/libs/lib/aarch64-linux-gnu/libAngstrongCameraSdk.so";
/// 加密配置 json 的缺省绝对路径（`AS_SDK_OpenCamera` 只吃这个）
pub const DEFAULT_CAMERA_UGV_CONFIG_JSON: &str = "/home/jetson/yahboomcar_ros2_ws/software/library_ws/src/ascamera/configurationfiles/hp60c_v2_00_20230704_configEncrypt.json";
/// 请求的流宽（实测 640×480）
pub const DEFAULT_CAMERA_UGV_WIDTH: u32 = 640;
/// 请求的流高
pub const DEFAULT_CAMERA_UGV_HEIGHT: u32 = 480;
/// 请求的流帧率（实测约 20 fps 出帧）
pub const DEFAULT_CAMERA_UGV_STREAM_FPS: u32 = 25;
/// 打开后等首帧的诊断上限（毫秒）；**配 0 = 不等**（逃生开关，bootstrap 立刻返回）
pub const DEFAULT_CAMERA_UGV_TIMEOUT_MS: u64 = 3000;

// ================================================================ [detector]（task_44 Step 7）
//
// 键名与缺省值**逐字复制** `pleiades-uav/src/config.rs:49-211`（口径 #8：各 crate 自带一份，
// 不进 base）。这份 `DetectorCameraConfig` 的方法名/签名必须与 uav 完全一致 ——
// `device/detector/projection.rs` 是**逐字节**复制的，它 `use crate::config::DetectorCameraConfig`。

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

/// `[detector].stream` 的默认值：**默认关**（关 = 连 bind 都不做，零线程）
pub const DEFAULT_DETECTOR_STREAM: bool = false;
/// `[detector].stream_port` 的默认值（各节点自己配；0 = 由内核分配端口）
pub const DEFAULT_DETECTOR_STREAM_PORT: u16 = 9010;
/// `[detector].stream_bind` 的默认值（只本机可看；想让别的机器看写 `"0.0.0.0"`）
pub const DEFAULT_DETECTOR_STREAM_BIND: &str = "127.0.0.1";

/// `[detector].flip_vertical` 的默认值（false = 不翻转，行为与原版一致）
pub const DEFAULT_DETECTOR_FLIP_VERTICAL: bool = false;

// ------------------------------------------------------------ [detector.camera]

/// `[detector.camera].fov_deg` 的默认值（度，全视角）
///
/// **真车实测值**（2026-11-02，地上 1/2/4 m 三点拟合，残差 1.63 px）：
/// 厂商标称的 70° 是**对角线**视角，纵向只有 47.8°。
pub const DEFAULT_DETECTOR_FOV_DEG: f32 = 47.8;
/// `[detector.camera].fov_axis` 的默认值：`fov_deg` 指的是**纵向**全视角
pub const DEFAULT_DETECTOR_FOV_AXIS: &str = "vertical";
/// `[detector.camera].mount_x` 的默认值（米，机体系 `x` 前）
pub const DEFAULT_DETECTOR_MOUNT_X: f32 = 0.10;
/// `[detector.camera].mount_y` 的默认值（米，机体系 `y` 右）
pub const DEFAULT_DETECTOR_MOUNT_Y: f32 = 0.0;
/// `[detector.camera].mount_z` 的默认值（米，机体系 `z` 高）
pub const DEFAULT_DETECTOR_MOUNT_Z: f32 = 0.135;
/// `[detector.camera].mount_yaw_deg` 的默认值（度；光轴 = 局部 −Z，**前视 = +90°**）
pub const DEFAULT_DETECTOR_MOUNT_YAW_DEG: f32 = 90.0;
/// `[detector.camera].mount_pitch_deg` 的默认值（度；**负 = 低头，正 = 抬头**）
///
/// 真车实测为 **+1.7°（略微抬头）**；相机装反时配合 `[detector].flip_vertical` 使用。
pub const DEFAULT_DETECTOR_MOUNT_PITCH_DEG: f32 = 1.7;
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
    /// 检测流：true = detector 起来后自动开 WS（false = 连 bind 都不做）
    pub stream: Option<bool>,
    /// 检测流端口（各节点自己配；0 = 内核分配）
    pub stream_port: Option<u16>,
    /// 检测流绑定地址（默认只本机；想让别的机器看写 "0.0.0.0"）
    pub stream_bind: Option<String>,
    /// 画面上下颠倒时置 true：推理前把图像垂直翻转一次（相机装反用）
    pub flip_vertical: Option<bool>,
    /// `[detector.camera]` 子表（task_37 Step 3，§四.4）；缺段 = None = 全用默认值
    pub camera: Option<DetectorCameraConfig>,
}

// ugv 的 `config.rs` 没有 uav 那份文件级 `#![allow(non_snake_case)]`（本文件既有
// `Ensure_Ugv_Config` 就带着该告警），故在这两个 impl 上按项放行，保持「新增代码 0 新告警」。
#[allow(non_snake_case)]
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

    /// `flip_vertical` 缺省 **false**（不翻转）
    pub fn Flip_Vertical(&self) -> bool {
        self.flip_vertical.unwrap_or(DEFAULT_DETECTOR_FLIP_VERTICAL)
    }

    /// 检测流开关（缺省 **false**：不 bind、零线程）
    pub fn Stream(&self) -> bool {
        self.stream.unwrap_or(DEFAULT_DETECTOR_STREAM)
    }

    /// 检测流端口（缺省 9010；0 = 让内核分配，日志打回填后的真实端口）
    pub fn Stream_Port(&self) -> u16 {
        self.stream_port.unwrap_or(DEFAULT_DETECTOR_STREAM_PORT)
    }

    /// 检测流绑定地址（去空白；空串 → 默认 `127.0.0.1`；非法 IP 由 detector 侧 `warn` + 回退）
    pub fn Stream_Bind(&self) -> &str {
        self.stream_bind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_DETECTOR_STREAM_BIND)
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

#[allow(non_snake_case)]
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

/// UGV 完整配置 = 共享段（flatten）+ 车独有设备段
#[derive(Debug, Clone, Deserialize)]
pub struct UgvConfig {
    #[serde(flatten)]
    pub base: BaseConfig,
    /// 底盘设备配置（STM32 轮式底盘）
    pub chassis: Option<ChassisConfig>,
    /// 雷达设备配置（YDLIDAR，含 SLAM 建图）
    pub lidar: Option<LidarConfig>,
    /// 他车障碍膨胀半径（米，缺省 0.2）
    pub obstacle_inflation_radius: Option<f32>,
    /// 单拍规划时间预算（毫秒，缺省 20；Task 39 §二；**顶层键**，与
    /// `obstacle_inflation_radius` 同级 —— 不放 `[chassis]`）
    pub plan_budget_ms: Option<u64>,
    /// RTK 流动站设备配置（LG290P）
    pub lg290p: Option<Lg290pConfig>,
    /// 车载相机设备配置（HP60C 厂商 SDK；**默认关**：相机独占 USB，与车上 ROS ascamera 互斥）
    pub camera_ugv: Option<CameraUgvConfig>,
    /// detector 设备（task_44 Step 7，`[detector]` 段）；无该段 = `None` = 不 spawn
    pub detector: Option<DetectorConfig>,
}

/// 底盘设备配置（STM32 轮式底盘）
#[derive(Debug, Clone, Deserialize)]
pub struct ChassisConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    /// 车型（缺省 `M1`，task_44 §1.2 #9：本车是麦克纳姆 M1）
    pub car_type: Option<String>,
    /// 巡航线速度（m/s，缺省 `DEFAULT_CRUISE_SPEED` = 0.45；task_44 §1.2 #11/#12）
    pub cruise_speed: Option<f32>,
    /// 巡航偏航角速度绝对值（rad/s，缺省 `DEFAULT_CRUISE_YAW_RATE` = 0.5；方向由执行器定符号）
    pub cruise_yaw_rate: Option<f32>,
}

/// 雷达设备配置（YDLIDAR，含 SLAM 建图）
#[derive(Debug, Clone, Deserialize)]
pub struct LidarConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
}

/// RTK 流动站设备配置（LG290P）
#[derive(Debug, Clone, Deserialize)]
pub struct Lg290pConfig {
    pub enabled: Option<bool>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
}

/// 车载相机设备配置（HP60C / Angstrong 厂商 SDK，task_43 §三）
///
/// 所有键都可缺省：`enabled = false`（默认关）、`sdk_lib` / `config_json` 缺省取
/// `DEFAULT_CAMERA_UGV_*` 的实机路径，分辨率/帧率缺省 640×480@25。
#[derive(Debug, Clone, Deserialize)]
pub struct CameraUgvConfig {
    /// 是否启用（**缺省 false**：相机独占 USB，与车上 ROS ascamera 节点互斥）
    pub enabled: Option<bool>,
    /// 厂商 SDK 库路径（缺省 `DEFAULT_CAMERA_UGV_SDK_LIB`；运行期 dlopen）
    pub sdk_lib: Option<String>,
    /// 加密配置 json 的绝对路径（缺省 `DEFAULT_CAMERA_UGV_CONFIG_JSON`）
    pub config_json: Option<String>,
    /// 请求的流宽（缺省 640）
    pub width: Option<u32>,
    /// 请求的流高（缺省 480）
    pub height: Option<u32>,
    /// 请求的流帧率（缺省 25）
    pub stream_fps: Option<u32>,
    /// 打开后等首帧的诊断上限（毫秒，缺省 3000）；**配 0 = 不等**（逃生开关）
    pub timeout_ms: Option<u64>,
}

/// 读取 UGV 配置（.config/config.toml）。
///
/// 首次启动时若 config.toml 缺失车端设备段，会先补全默认设备段再读取（幂等，不覆盖已有值）。
pub fn Ensure_Ugv_Config() -> Result<UgvConfig, Box<dyn std::error::Error>> {
    let path = config_file_path();
    // 配置文件不存在则先由 base 生成共享段（防御性：正常流程 core_bootstrap 已生成）
    if !path.exists() {
        pleiades_base::config::Ensure_Config()?;
    }
    ensure_ugv_section(&path)?;
    let content = fs::read_to_string(&path)?;
    let config: UgvConfig = toml::from_str(&content)?;
    Ok(config)
}

/// 确保 config.toml 存在车端设备段（obstacle_inflation_radius / plan_budget_ms / chassis / lidar /
/// lg290p / camera_ugv），缺失才补默认值。
fn ensure_ugv_section(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
    changed |= fill_chassis(&mut doc);
    changed |= fill_lidar(&mut doc);
    changed |= fill_lg290p(&mut doc);
    changed |= fill_camera_ugv(&mut doc);
    changed |= fill_detector(&mut doc);

    if changed {
        fs::write(path, doc.to_string())?;
    }
    Ok(())
}

/// 把 `f32` 定值写成 toml 里的十进制字面量。
///
/// 不能直接 `DEFAULT_CRUISE_SPEED as f64`：`0.45f32` 的 f64 展开是
/// `0.44999998807907104`，写进 config.toml 会变成一串尾巴（人读 / 手改都不友好）。
/// `f32::to_string()` 给的是最短往返十进制（`0.45`），再解析回 f64 即是干净字面量。
fn toml_f32(v: f32) -> Item {
    toml_edit::value(v.to_string().parse::<f64>().expect("f32 定值应为合法十进制"))
}

/// 补全 [chassis] 段（缺失才补，幂等）
fn fill_chassis(doc: &mut DocumentMut) -> bool {
    if doc.get("chassis").is_none() {
        doc["chassis"] = Item::Table(toml_edit::Table::new());
    }
    let chassis = doc["chassis"].as_table_mut().expect("chassis 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 6] = [
        ("enabled", toml_edit::value(true)),
        ("port", toml_edit::value("/dev/ttyUSB0")),
        ("baudrate", toml_edit::value(115200_i64)),
        ("car_type", toml_edit::value("M1")),
        ("cruise_speed", toml_f32(DEFAULT_CRUISE_SPEED)),
        ("cruise_yaw_rate", toml_f32(DEFAULT_CRUISE_YAW_RATE)),
    ];
    for (k, v) in defaults {
        if !chassis.contains_key(k) {
            chassis.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [lidar] 段（缺失才补，幂等）
fn fill_lidar(doc: &mut DocumentMut) -> bool {
    if doc.get("lidar").is_none() {
        doc["lidar"] = Item::Table(toml_edit::Table::new());
    }
    let lidar = doc["lidar"].as_table_mut().expect("lidar 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 3] = [
        ("enabled", toml_edit::value(true)),
        ("port", toml_edit::value("/dev/ttyUSB1")),
        ("baudrate", toml_edit::value(230400_i64)),
    ];
    for (k, v) in defaults {
        if !lidar.contains_key(k) {
            lidar.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [lg290p] 段（缺失才补，幂等）
fn fill_lg290p(doc: &mut DocumentMut) -> bool {
    if doc.get("lg290p").is_none() {
        doc["lg290p"] = Item::Table(toml_edit::Table::new());
    }
    let lg290p = doc["lg290p"].as_table_mut().expect("lg290p 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 3] = [
        ("enabled", toml_edit::value(false)),
        ("port", toml_edit::value("/dev/ttyUSB2")),
        ("baudrate", toml_edit::value(460800_i64)),
    ];
    for (k, v) in defaults {
        if !lg290p.contains_key(k) {
            lg290p.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [camera_ugv] 段（缺失才补，幂等；task_43 §三）
///
/// `enabled` 缺省 **false**（相机独占 USB，与车上 ROS ascamera 节点互斥，按需开）；
/// `timeout_ms` 缺省 3000，配 0 表示"打开后不等首帧"。
fn fill_camera_ugv(doc: &mut DocumentMut) -> bool {
    if doc.get("camera_ugv").is_none() {
        doc["camera_ugv"] = Item::Table(toml_edit::Table::new());
    }
    let camera_ugv = doc["camera_ugv"].as_table_mut().expect("camera_ugv 应为 table");
    let mut changed = false;
    let defaults: [(&str, Item); 7] = [
        ("enabled", toml_edit::value(false)),
        ("sdk_lib", toml_edit::value(DEFAULT_CAMERA_UGV_SDK_LIB)),
        ("config_json", toml_edit::value(DEFAULT_CAMERA_UGV_CONFIG_JSON)),
        ("width", toml_edit::value(DEFAULT_CAMERA_UGV_WIDTH as i64)),
        ("height", toml_edit::value(DEFAULT_CAMERA_UGV_HEIGHT as i64)),
        ("stream_fps", toml_edit::value(DEFAULT_CAMERA_UGV_STREAM_FPS as i64)),
        ("timeout_ms", toml_edit::value(DEFAULT_CAMERA_UGV_TIMEOUT_MS as i64)),
    ];
    for (k, v) in defaults {
        if !camera_ugv.contains_key(k) {
            camera_ugv.insert(k, v);
            changed = true;
        }
    }
    changed
}

/// 补全 [detector] 段（task_44 Step 7；缺失才补，幂等；**不覆盖**用户已写的值）
///
/// 键名与缺省值逐字对齐 `pleiades-uav/src/config.rs:463-487`（口径 #8）。
fn fill_detector(doc: &mut DocumentMut) -> bool {
    if doc.get("detector").is_none() {
        doc["detector"] = Item::Table(toml_edit::Table::new());
    }
    let Some(table) = doc["detector"].as_table_mut() else {
        return false;
    };
    let mut changed = false;
    let defaults: [(&str, Item); 9] = [
        // 默认关：权重不一定在场，且 detector 会吃满一个 CPU 核
        ("enabled", toml_edit::value(DEFAULT_DETECTOR_ENABLED)),
        ("model", toml_edit::value(DEFAULT_DETECTOR_MODEL)),
        ("device", toml_edit::value(DEFAULT_DETECTOR_DEVICE)),
        ("conf_thres", toml_edit::value(f64::from(DEFAULT_DETECTOR_CONF_THRES))),
        ("nms_thres", toml_edit::value(f64::from(DEFAULT_DETECTOR_NMS_THRES))),
        ("flip_vertical", toml_edit::value(DEFAULT_DETECTOR_FLIP_VERTICAL)),
        // task_47 Step 3：检测流三键（缺失才补，幂等，不覆盖用户已写的值）
        ("stream", toml_edit::value(DEFAULT_DETECTOR_STREAM)),
        ("stream_port", toml_edit::value(DEFAULT_DETECTOR_STREAM_PORT as i64)),
        ("stream_bind", toml_edit::value(DEFAULT_DETECTOR_STREAM_BIND)),
    ];
    for (k, v) in defaults {
        if !table.contains_key(k) {
            table.insert(k, v);
            changed = true;
        }
    }
    // `[detector.camera]` 子表（缺失才补、幂等、不覆盖用户值）
    changed |= fill_detector_camera(table);
    changed
}

/// 补全 `[detector.camera]` 子表（task_44 Step 7；缺失才补，幂等；**不覆盖**用户已写的值）
///
/// 数值一律用 **f64 字面量**：`f64::from(0.045_f32)` 会渲染成 `0.04500000178813934`
/// （f32→f64 的尾数），落盘难看；解析回 f32 后两种写法是同一个数。
fn fill_detector_camera(table: &mut toml_edit::Table) -> bool {
    if !table.contains_key("camera") {
        table.insert("camera", Item::Table(toml_edit::Table::new()));
    }
    let Some(cam) = table.get_mut("camera").and_then(|i| i.as_table_mut()) else {
        return false;
    };
    let defaults: [(&str, Item); 9] = [
        ("fov_deg", toml_edit::value(47.8_f64)),
        ("fov_axis", toml_edit::value(DEFAULT_DETECTOR_FOV_AXIS)),
        ("mount_x", toml_edit::value(0.10_f64)),
        ("mount_y", toml_edit::value(0.0_f64)),
        ("mount_z", toml_edit::value(0.135_f64)),
        ("mount_yaw_deg", toml_edit::value(90.0_f64)),
        ("mount_pitch_deg", toml_edit::value(1.7_f64)),
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
    fn test_ensure_ugv_section_fills_defaults() {
        let dir = tmp_dir("ugv_cfg_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();

        assert_eq!(parsed.obstacle_inflation_radius, Some(0.2));
        let c = parsed.chassis.expect("chassis 段应被补全");
        assert_eq!(c.enabled, Some(true));
        assert_eq!(c.port.as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(c.baudrate, Some(115200));
        assert_eq!(c.car_type.as_deref(), Some("M1"));
        assert_eq!(c.cruise_speed, Some(0.45));
        assert_eq!(c.cruise_yaw_rate, Some(0.5));
        let l = parsed.lidar.expect("lidar 段应被补全");
        assert_eq!(l.enabled, Some(true));
        assert_eq!(l.port.as_deref(), Some("/dev/ttyUSB1"));
        assert_eq!(l.baudrate, Some(230400));
        let g = parsed.lg290p.expect("lg290p 段应被补全");
        assert_eq!(g.enabled, Some(false));
        assert_eq!(g.port.as_deref(), Some("/dev/ttyUSB2"));
        assert_eq!(g.baudrate, Some(460800));
        let cam = parsed.camera_ugv.expect("camera_ugv 段应被补全");
        assert_eq!(cam.enabled, Some(false), "相机默认必须关（独占 USB）");
        assert_eq!(cam.sdk_lib.as_deref(), Some(DEFAULT_CAMERA_UGV_SDK_LIB));
        assert_eq!(cam.config_json.as_deref(), Some(DEFAULT_CAMERA_UGV_CONFIG_JSON));
        assert_eq!((cam.width, cam.height), (Some(640), Some(480)));
        assert_eq!(cam.stream_fps, Some(25));
        assert_eq!(cam.timeout_ms, Some(3000));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── [chassis] 巡航量（task_44 §1.2 #11/#12：0.45 m/s / 0.5 rad/s，百分比档位删除）───

    #[test]
    fn test_chassis_cruise_defaults_are_spec_values() {
        // §1.2 #11 的定值只在这两个常量里出现一次
        assert_eq!(DEFAULT_CRUISE_SPEED, 0.45);
        assert_eq!(DEFAULT_CRUISE_YAW_RATE, 0.5);

        let dir = tmp_dir("ugv_cfg_cruise");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();
        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        // 定值以干净十进制落盘（不是 `0.44999998807907104` 这种 f32→f64 尾巴）
        assert!(content.contains("cruise_speed = 0.45"), "{content}");
        assert!(content.contains("cruise_yaw_rate = 0.5"), "{content}");
        // §1.2 #12：百分比档位键必须整体消失 —— 比对 [chassis] 的键集合（不多也不少）
        let doc: DocumentMut = content.parse().unwrap();
        let keys: Vec<&str> = doc["chassis"]
            .as_table()
            .expect("chassis 应为 table")
            .iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            keys,
            ["enabled", "port", "baudrate", "car_type", "cruise_speed", "cruise_yaw_rate"],
            "百分比档位键（{content}）必须换成两项巡航量"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_ugv_section_keeps_existing() {
        let dir = tmp_dir("ugv_cfg_keep");
        let path = dir.join("config.toml");
        std::fs::write(&path, "obstacle_inflation_radius = 0.5\n[chassis]\nport = \"/dev/ttyACM0\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.obstacle_inflation_radius, Some(0.5));
        let c = parsed.chassis.unwrap();
        assert_eq!(c.port.as_deref(), Some("/dev/ttyACM0"));
        assert_eq!(c.baudrate, Some(115200));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── camera_ugv（task_43 §三：缺键补全、默认关、幂等、已有值不覆盖）───

    #[test]
    fn test_ensure_ugv_section_fills_camera_ugv() {
        let dir = tmp_dir("ugv_cfg_camera_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[camera_ugv]"), "必须独立成段：{content}");
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        let cam = parsed.camera_ugv.expect("camera_ugv 段应被补全");
        assert_eq!(cam.enabled, Some(false), "enabled 缺省必须 false");
        assert_eq!(cam.timeout_ms, Some(DEFAULT_CAMERA_UGV_TIMEOUT_MS));
        assert_eq!(cam.stream_fps, Some(DEFAULT_CAMERA_UGV_STREAM_FPS));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_camera_ugv_existing_value_kept() {
        let dir = tmp_dir("ugv_cfg_camera_keep");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "[camera_ugv]\nenabled = true\nwidth = 1280\ntimeout_ms = 0\n",
        )
        .unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        let cam = parsed.camera_ugv.expect("camera_ugv 段应存在");
        assert_eq!(cam.enabled, Some(true), "不得覆盖用户写的 enabled");
        assert_eq!(cam.width, Some(1280), "不得覆盖用户写的 width");
        assert_eq!(cam.timeout_ms, Some(0), "0 是合法的逃生开关，不得被改成默认");
        // 缺的键仍然补齐
        assert_eq!(cam.height, Some(480));
        assert_eq!(cam.sdk_lib.as_deref(), Some(DEFAULT_CAMERA_UGV_SDK_LIB));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_camera_ugv_idempotent() {
        let dir = tmp_dir("ugv_cfg_camera_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[camera_ugv]\nenabled = false\n").unwrap();

        ensure_ugv_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_ugv_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        let parsed: UgvConfig = toml::from_str(&second).unwrap();
        assert_eq!(parsed.camera_ugv.expect("camera_ugv 段应存在").enabled, Some(false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ─── plan_budget_ms（Task 39 §二：顶层键，缺键补 20、幂等、已有值不覆盖）───

    #[test]
    fn test_ensure_ugv_section_fills_plan_budget() {
        let dir = tmp_dir("ugv_cfg_plan_budget_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(DEFAULT_PLAN_BUDGET_MS));
        assert_eq!(parsed.plan_budget_ms, Some(20), "缺键补 20 ms");
        // 必须是**顶层键**，不能掉进 [chassis]
        assert!(content.contains("plan_budget_ms = 20"), "{content}");
        assert!(!content.contains("[chassis]\nplan_budget_ms"), "{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plan_budget_existing_value_kept() {
        let dir = tmp_dir("ugv_cfg_plan_budget_keep");
        let path = dir.join("config.toml");
        std::fs::write(&path, "plan_budget_ms = 50\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(50), "不得覆盖用户写的值");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ensure_ugv_section_plan_budget_idempotent() {
        let dir = tmp_dir("ugv_cfg_plan_budget_idem");
        let path = dir.join("config.toml");
        std::fs::write(&path, "plan_budget_ms = 50\n").unwrap();

        ensure_ugv_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_ugv_section(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "第二次补全不得改动文件内容");
        let parsed: UgvConfig = toml::from_str(&second).unwrap();
        assert_eq!(parsed.plan_budget_ms, Some(50));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============================================================ [detector]（task_44 Step 7）

    /// `[detector]` 段被补成五个默认值、`enabled=false`、落盘只有一段；用户值不覆盖
    #[test]
    fn test_ensure_ugv_section_fills_detector() {
        let dir = tmp_dir("ugv_cfg_detector_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        let d = parsed.detector.expect("[detector] 段应被补全");
        assert_eq!(d.enabled, Some(false), "enabled 缺省必须 false");
        assert_eq!(d.model.as_deref(), Some("yolov8n.safetensors"));
        assert_eq!(d.device.as_deref(), Some("cpu"));
        assert_eq!(d.conf_thres, Some(0.35));
        assert_eq!(d.nms_thres, Some(0.45));
        assert_eq!(d.flip_vertical, Some(false));
        assert!(content.contains("[detector]"), "{content}");
        // 补段幂等：不得重复插入
        assert_eq!(content.matches("[detector]").count(), 1, "{content}");

        // 用户已写的值不被覆盖（`enabled=true` / 越界 `conf_thres=2.0` 原样保留）
        let dir2 = tmp_dir("ugv_cfg_detector_keep");
        let path2 = dir2.join("config.toml");
        std::fs::write(&path2, "[detector]\nenabled = true\nconf_thres = 2.0\n").unwrap();
        ensure_ugv_section(&path2).unwrap();
        let content2 = std::fs::read_to_string(&path2).unwrap();
        let parsed2: UgvConfig = toml::from_str(&content2).unwrap();
        let d2 = parsed2.detector.expect("[detector] 段应仍在");
        assert_eq!(d2.enabled, Some(true), "不得覆盖用户写的 enabled");
        assert!(content2.contains("conf_thres = 2.0"), "不得覆盖用户写的值：\n{content2}");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    /// `[detector.camera]` 子表被补成 9 个默认值，落盘是干净十进制、子表只 1 次
    #[test]
    fn test_ensure_ugv_section_fills_detector_camera() {
        let dir = tmp_dir("ugv_cfg_detector_camera_fill");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();

        ensure_ugv_section(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: UgvConfig = toml::from_str(&content).unwrap();
        let cam = parsed
            .detector
            .expect("[detector] 段应被补全")
            .camera
            .expect("[detector.camera] 子表应被补全");
        assert_eq!(cam.fov_deg, Some(47.8));
        assert_eq!(cam.fov_axis.as_deref(), Some("vertical"));
        assert_eq!(cam.mount_x, Some(0.10));
        assert_eq!(cam.mount_y, Some(0.0));
        assert_eq!(cam.mount_z, Some(0.135));
        assert_eq!(cam.mount_yaw_deg, Some(90.0));
        assert_eq!(cam.mount_pitch_deg, Some(1.7));
        assert_eq!(cam.ground_z, Some(0.0));
        assert_eq!(cam.max_range_m, Some(12.0));

        // 落盘文本：子表头在、是干净的十进制（不是 f32 尾数长串）、不重复
        assert!(content.contains("[detector.camera]"), "{content}");
        assert!(content.contains("mount_z = 0.045"), "{content}");
        assert_eq!(content.matches("[detector.camera]").count(), 1, "{content}");
        assert_eq!(content.matches("[detector]").count(), 1, "{content}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============================================================ [detector] 检测流（task_47 Step 3）

    /// `[detector]` 被补成 9 个默认值（task_47 Step 3 补 stream 三键），其余行不变、幂等
    #[test]
    fn test_detector_section_fills_stream_defaults() {
        let dir = tmp_dir("ugv_cfg_detector_stream");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[Identity]\npeer_name = \"t\"\nnode_type = \"car\"\n").unwrap();
        ensure_ugv_section(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        let d = toml::from_str::<UgvConfig>(&first)
            .unwrap()
            .detector
            .expect("[detector] 段应被补全");
        assert_eq!(d.stream, Some(DEFAULT_DETECTOR_STREAM), "stream 默认 false 且要写进文件");
        assert_eq!(d.stream_port, Some(DEFAULT_DETECTOR_STREAM_PORT));
        assert_eq!(d.stream_bind.as_deref(), Some(DEFAULT_DETECTOR_STREAM_BIND));
        assert_eq!(d.Stream(), DEFAULT_DETECTOR_STREAM);
        assert_eq!(d.Stream_Port(), 9010);
        assert_eq!(d.Stream_Bind(), "127.0.0.1");
        // 幂等：再跑一次，落盘文本一字不变（其余行也不动）
        ensure_ugv_section(&path).unwrap();
        assert_eq!(first, std::fs::read_to_string(&path).unwrap(), "补段必须幂等");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
