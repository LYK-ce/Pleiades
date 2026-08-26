//Presented by KeJi
//Created Date ： 2026-08-25
//Modified Date ： 2026-08-25

//! MavlinkDevice 常量

/// 飞行模式（ArduCopter custom_mode）
pub const MODE_GUIDED: u32 = 4;

/// takeoff 固定起飞高度 (m)
pub const TARGET_ALT: f32 = 1.8;

/// 前进/后退速度 (m/s，机体系 vx)
pub const VEL_FWD: f32 = 0.3;

/// 转向角速度 (°/s)
pub const YAW_RATE_DEG: f32 = 15.0;

/// 强制上锁魔法数字（保留备用；实飞由遥控器解锁，程序不主动 arm/disarm）
pub const FORCE_DISARM_MAGIC: f32 = 21196.0;

/// 起飞到位判定比例（1.8 × 0.9 = 1.62m；本 task 纯触发不轮询，保留供参考）
pub const TAKEOFF_ARRIVE_RATIO: f32 = 0.9;

/// 飞控目标身份（硬编码，与 UAV 侧一致）
pub const FC_SYSTEM_ID: u8 = 1;
pub const FC_COMPONENT_ID: u8 = 1;

/// 本机（地面站/机载计算机）身份
pub const GCS_SYSTEM_ID: u8 = 255;
pub const GCS_COMPONENT_ID: u8 = 0;
