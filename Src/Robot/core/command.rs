//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-07-28

//! Robot 统一命令定义
//!
//! 三层命令体系：
//! - Mode：模式切换（Manual ↔ Auto）
//! - Manual：手动遥控命令（仅 Manual 模式有效）
//! - Auto：自动任务命令（仅 Auto 模式有效）

/// 顶层命令
#[derive(Debug, Clone)]
pub enum Command {
    Mode(ModeCmd),
    Manual(ManualCmd),
    Auto(AutoCmd),
}

/// 模式控制
#[derive(Debug, Clone)]
pub enum ModeCmd {
    SwitchToManual,
    SwitchToAuto,
}

/// 手动遥控命令
#[derive(Debug, Clone)]
pub enum ManualCmd {
    Forward(i16),
    Backward(i16),
    SpinLeft(i16),
    SpinRight(i16),
    Stop,
    Beep(u16),
    StartLidarScan,
    StopLidarScan,
}

/// 自动任务命令
#[derive(Debug, Clone)]
pub enum AutoCmd {
    /// 追加任务到队列
    Push(Vec<Mission>),
    /// 清空队列
    Cancel,
}

/// 任务单元
#[derive(Debug, Clone)]
pub enum Mission {
    /// 前往目标点（世界坐标，米）
    Goto(f32, f32),
    // 未来: Patrol, Explore, ...
}
