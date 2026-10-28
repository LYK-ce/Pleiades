//Presented by KeJi
//Created Date ： 2026-07-07
//Modified Date ： 2026-10-27

//! Robot 统一命令定义
//!
//! 三层命令体系：
//! - Mode：模式切换（Manual ↔ Auto）
//! - Manual：手动遥控命令（仅 Manual 模式有效）
//! - Auto：自动任务命令（仅 Auto 模式有效）

// Task 36 §12.1：`Mission` 已迁至 `mission_queue::element`。此处 re-export 仅为保持
// `command::Mission` 旧路径可用（设备端若干 `use …::command::Mission;` 本 step 不动）；
// 后续 step 统一到 `mission_queue::Mission` 后删除本行。
pub use super::mission_queue::Mission;

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
    Takeoff, // Task 22_4：起飞（固定 1.8m）
    Land,    // Task 22_4：降落
}

/// 自动任务命令
#[derive(Debug, Clone)]
pub enum AutoCmd {
    /// 整体替换任务队列（2026-08-07 协议统一：替代原 Push 追加语义）
    /// 空列表 = 取消全部任务（停车待命）；收到即丢弃旧队列（含正在执行的任务）
    Set(Vec<Mission>),
    /// 追加队尾（Task 36 §十一 #15）：当前任务**不受影响**，不打断、不重来
    ///
    /// ⚠️ 协议**不暴露**本变体（`TASK_SET` 只有 replace，§12.8.2 表注）：目前只有 sim 控制口
    /// 与端到端测试使用；将来地面站要用，需要新 msgid。`main_loop` 只动队列、**不碰设备**。
    Append(Vec<Mission>),
    /// 插入队头（Task 36 §十一 #15 / #17）：当前任务立即让位，**带树实例**退到第二位，
    /// 新任务出队后它回到队首**继续 tick 原实例**（可恢复、可嵌套）。
    ///
    /// 与 `Set` 的分界（§十一 #38）：`Set` → `device.reset()`（halt 树 + **清实例表**）；
    /// `Preempt` → `device.suspend()`（不停树 + **保留实例表**）。
    /// ⚠️ 协议不暴露（同上）。
    Preempt(Vec<Mission>),
}
