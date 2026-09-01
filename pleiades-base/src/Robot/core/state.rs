//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-26

//! 机器人位姿状态（base 共享 schema）
//!
//! RobotState 由设备端 RX 回调（stm32/mavlink）单一写入，纯数据结构不含业务逻辑。
//! 设备独有状态（encoders、LidarState）已摘出到设备端（Task 23 阶段 B）。
//!
//! 位置语义（Task 9）：`x/y` 为全局世界坐标 (m)，初值 = origin（默认 64,64），
//! 由 `Robot::new` 注入共享 `RobotState`（Task 24：从 local_state 上移）。
//! 写者（Task 24 写者改造）：传感器字段由设备端 RX 回调（stm32/mavlink）写；
//! x/y 由 STM32 里程计（读-改-写）+ LG290P RTK（FIXED 覆盖）共同写；
//! `rtk_fixed` 由 LG290P 独写（FIXED true / 失锁 false）。


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


    /// 全局世界坐标 (m)，启动时 = origin（默认 64,64,0）
    /// 由 STM32 里程计积分 + LG290P RTK_FIXED 覆盖（Task 24）
    pub x: f32,
    pub y: f32,
    /// 垂直高度 (m)。写入者 = 设备自己：车恒写 0，机写飞控 EKF；odometry 不积分 z；LG290P 忽略 U 分量（D11）
    pub z: f32,

    /// RTK 固定解标志（Task 24）：LG290P 实时写——FIXED true、失锁 false；未启用 RTK 恒 false
    pub rtk_fixed: bool,
}


// ============================================================
// 决策状态机 / 动作 / 决策结果（Task 22_3 单写者收敛）
// ============================================================

/// 状态机状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecisionState {
    #[default]
    Idle,
    Turning,
    Moving,
}

/// 动作（纯意图，无速度载荷——速度由设备层绑定，Task 22_5 D2）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionAction {
    MoveForward,
    MoveBackward,
    TurnLeft,
    TurnRight,
    Stop,
}

/// 决策器 decide 的返回值（纯决策结果）
#[derive(Debug, Clone)]
pub struct DecisionResult {
    pub state: DecisionState,
    pub sub_target: Option<(i32, i32)>,
    /// None = 保持、不发命令（命令去重）
    pub action: Option<MotionAction>,
}

// ============================================================
// ExecuteState — 执行器状态（Task 22_3 单写者：main_loop 唯一写）
// ============================================================

/// 执行器状态（决策状态机 + 意图广播）
///
/// 写者 = main_loop（应用决策结果时写；急停/任务替换/模式切换统一写 Idle+None）；
/// 读者 = state_notifier（100ms 组帧）。
#[derive(Debug, Clone, Default)]
pub struct ExecuteState {
    /// 状态机状态
    pub state: DecisionState,
    /// D* 寻路当前子目标（网格坐标）；None = 无任务/空闲
    pub sub_target: Option<(i32, i32)>,
}
