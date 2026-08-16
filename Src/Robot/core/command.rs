//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-08-12

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
    /// 整体替换任务队列（2026-08-07 协议统一：替代原 Push 追加语义）
    /// 空列表 = 取消全部任务（停车待命）；收到即丢弃旧队列（含正在执行的任务）
    Set(Vec<Mission>),
}

/// 任务单元
#[derive(Debug, Clone, PartialEq)]
pub enum Mission {
    /// 前往目标点（世界坐标，米）
    ///
    /// Task 14：`members` 非空 = 群发任务——executor 执行时经 `assignment` 自算散布位置；
    /// `members` 空 = 老单车语义（直接以 (x, y) 为目标）。
    Goto { x: f32, y: f32, members: Vec<Vec<u8>> },
    /// 围圈（Task 18）：`x/y` = 圆心（世界坐标，米），`members` 非空 = 群发。
    ///
    /// 每车按 peer_id 排序序号在环上均匀铺开（半径写死 0.5m = 与圆心隔 1 格，见 assignment）。
    /// 第一版到达即停、不朝圆心。
    Circle { x: f32, y: f32, members: Vec<Vec<u8>> },
    // 未来: Patrol, Explore, ...
}
