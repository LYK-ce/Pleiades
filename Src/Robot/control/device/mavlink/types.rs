//Presented by KeJi
//Created Date ： 2026-08-25
//Modified Date ： 2026-08-25

//! MAVLink 飞控遥测快照
//!
//! 从 UAV 仓库 `rust-mavlink-test` 的 `Telemetry` 迁移，字段与 UAV 侧一致。

/// 飞控遥测状态快照（关键字段，供状态展示与坐标对齐用）
#[derive(Debug, Clone, Default)]
pub struct Telemetry {
    pub system_id: u8,
    pub component_id: u8,
    pub armed: bool,
    pub custom_mode: u32,
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub battery_voltage: f32, // V
    pub relative_alt: f32,    // m
    pub lat: i32,             // deg * 1e7
    pub lon: i32,             // deg * 1e7
    /// 全局速度（GLOBAL_POSITION_INT.vx/vy/vz，cm/s → m/s）
    pub vel_vx: f32,
    pub vel_vy: f32,
    pub vel_vz: f32,
    /// 航向（GLOBAL_POSITION_INT.hdg，cdeg → deg）
    pub hdg: f32,
    /// 本地位置（LOCAL_POSITION_NED，米，相对 home）
    pub local_x: f32,
    pub local_y: f32,
    pub local_z: f32,
    /// 本地速度（LOCAL_POSITION_NED，m/s）
    pub local_vx: f32,
    pub local_vy: f32,
    pub local_vz: f32,
    /// EKF 状态 flags（EKF_STATUS_REPORT.flags.bits()）
    pub ekf_flags: u16,
    /// GPS fix type（3 = 3D fix）
    pub fix_type: u32,
    pub satellites_visible: u8,
    // 各消息计数（供状态新鲜度判定用）
    pub heartbeat_count: u32,
    pub attitude_count: u32,
    pub sys_status_count: u32,
    pub gps_count: u32,
    pub global_pos_count: u32,
    pub local_pos_count: u32,
    pub ekf_count: u32,
    pub statustext_count: u32,
}
