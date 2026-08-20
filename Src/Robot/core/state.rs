//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-20

//! 机器人全局状态
//!
//! RobotState 由 STM32 写入，LidarState 由 LiDAR 写入，各设备独立。
//! 纯数据结构，不含业务逻辑。里程计计算见 slam/odometry.rs。
//!
//! 位置语义（Task 9）：`x/y` 为全局世界坐标 (m)，初值 = origin（默认 64,64），
//! 由 `STM32Device::spawn` 注入 local_state。
//! 单一写入者不变式：共享 RobotState 只被 STM32 RX 回调全量覆盖写，
//! 任何其他路径不得直接修改共享态字段。

use crate::robot::control::device::lidar::LaserScan;

/// 姿态角（弧度）
#[derive(Debug, Clone, Default)]
pub struct Attitude {
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
}

/// 陀螺仪数据 (rad/s)
#[derive(Debug, Clone, Default)]
pub struct GyroData {
    pub gx: f32,
    pub gy: f32,
    pub gz: f32,
}

/// 加速度计数据 (m/s²)
#[derive(Debug, Clone, Default)]
pub struct AccelData {
    pub ax: f32,
    pub ay: f32,
    pub az: f32,
}

/// 磁力计数据
#[derive(Debug, Clone, Default)]
pub struct MagData {
    pub mx: f32,
    pub my: f32,
    pub mz: f32,
}

// ============================================================
// RobotState — STM32 传感器上报
// ============================================================

/// 机器人底盘传感器状态（由 STM32 维护）
#[derive(Debug, Clone, Default)]
pub struct RobotState {
    /// 线速度 (m/s), vz 为垂直速度 (m/s)（车恒 0，机写飞控垂速）
    pub vx: f32,
    pub vy: f32,
    pub vz: f32,

    /// 电池电压 (V)
    pub battery: f32,

    /// 姿态角 (弧度)
    pub attitude: Attitude,

    /// 陀螺仪 (rad/s)
    pub gyro: GyroData,

    /// 加速度计 (m/s²)
    pub accel: AccelData,

    /// 磁力计
    pub mag: MagData,

    /// 四轮编码器计数
    pub encoders: [i32; 4],

    /// 全局世界坐标 (m)，启动时 = origin（默认 64,64,0）
    /// 由 STM32 RX 回调中的 odometry::accumulate 直接积分维护
    pub x: f32,
    pub y: f32,
    /// 垂直高度 (m)。写入者 = 设备自己：车恒写 0，机写飞控 EKF；odometry 不积分 z
    pub z: f32,
}

// ============================================================
// LidarState — LiDAR 点云
// ============================================================

/// 激光雷达扫描状态（由 LiDAR 维护）
#[derive(Debug, Clone, Default)]
pub struct LidarState {
    /// 最新扫描结果
    pub scan: Option<LaserScan>,
}

// ============================================================
// ExecuteState — 执行器意图（Task 13_1）
// ============================================================

/// 执行器意图状态（由 main_loop 写入，state_notifier 读取用于意图广播）
///
/// 写者 = main_loop（每 auto_tick 同步 GoalService 的 sub_target）；
/// 读者 = state_notifier（100ms 组帧）。单一写入者不变式与 RobotState 同源。
#[derive(Debug, Clone, Default)]
pub struct ExecuteState {
    /// D* 寻路当前子目标（网格坐标）；None = 无任务/空闲
    pub sub_target: Option<(i32, i32)>,
}
