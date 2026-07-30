//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-29

//! 机器人全局状态
//!
//! RobotState 由 STM32 写入，LidarState 由 LiDAR 写入，各设备独立。
//! 纯数据结构，不含业务逻辑。里程计计算见 slam/odometry.rs。

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
    /// 线速度 (m/s), vz 为角速度 (rad/s)
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

    /// 里程计累积位移 (m)，从 (0,0) 起步
    /// 由 slam/odometry.rs 在 RX 回调中实时累积
    pub odom_x: f32,
    pub odom_y: f32,
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
