//Presented by KeJi
//Date ： 2026-06-17

//! 机器人控制相关的数据类型

/// 车型码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CarType {
    X3 = 0x01,
    X3Plus = 0x02,
    X1 = 0x04,
    R2 = 0x05,
}

impl CarType {
    /// 从 u8 构造，未知值返回 None
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::X3),
            0x02 => Some(Self::X3Plus),
            0x04 => Some(Self::X1),
            0x05 => Some(Self::R2),
            _ => None,
        }
    }

    /// 运动参数限制 (vx_max, vy_max, vz_max)
    pub fn motion_limits(&self) -> (f32, f32, f32) {
        match self {
            Self::X3 => (1.0, 1.0, 5.0),
            Self::X3Plus => (0.7, 0.7, 3.2),
            Self::X1 => (1.0, 1.0, 5.0),
            Self::R2 => (1.8, 0.045, 3.0),
        }
    }
}

/// 运动状态码（FUNC_CAR_RUN）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MotionState {
    Stop = 0,
    Forward = 1,
    Backward = 2,
    Left = 3,
    Right = 4,
    SpinLeft = 5,
    SpinRight = 6,
    Park = 7,
}

impl MotionState {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Stop),
            1 => Some(Self::Forward),
            2 => Some(Self::Backward),
            3 => Some(Self::Left),
            4 => Some(Self::Right),
            5 => Some(Self::SpinLeft),
            6 => Some(Self::SpinRight),
            7 => Some(Self::Park),
            _ => None,
        }
    }
}

/// 陀螺仪原始数据
#[derive(Debug, Clone, Default)]
pub struct GyroData {
    /// rad/s
    pub gx: f32,
    pub gy: f32,
    pub gz: f32,
}

/// 加速度计原始数据
#[derive(Debug, Clone, Default)]
pub struct AccelData {
    /// m/s²
    pub ax: f32,
    pub ay: f32,
    pub az: f32,
}

/// 磁力计原始数据
#[derive(Debug, Clone, Default)]
pub struct MagData {
    pub mx: f32,
    pub my: f32,
    pub mz: f32,
}

/// 姿态角
#[derive(Debug, Clone, Default)]
pub struct Attitude {
    /// 弧度
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
}

/// 机器人传感器状态缓存
///
/// 由后台接收任务持续更新，外部通过 RobotControl 的 get_state() 读取。
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
