//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! 机器人全局状态
//!
//! 聚合所有设备上报的传感器数据，描述机器人整体状态。

/// 姿态角
#[derive(Debug, Clone, Default)]
pub struct Attitude {
    /// 弧度
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

/// 机器人传感器状态缓存
#[derive(Debug, Clone, Default)]
pub struct RobotState {
    /// 线速度 (m/s)
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
}
