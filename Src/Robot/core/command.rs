//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-07

//! Robot 统一命令定义
//!
//! 所有上层命令源（WS、Lua、LLM Agent）都通过此枚举向 Robot 发送命令。

/// 机器人命令
#[derive(Debug, Clone)]
pub enum Command {
    Forward(i16),
    Backward(i16),
    SpinLeft(i16),
    SpinRight(i16),
    Stop,
    Beep(u16),
    // LiDAR
    StartLidarScan,
    StopLidarScan,
    // 未来：
    // GoTo(f32, f32),
    // Patrol,
    // Explore,
}
