//Presented by KeJi
//Created Date ： 2026-07-28
//Modified Date ： 2026-08-07

//! 运行模式

/// 机器人运行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMode {
    /// 手动遥控模式：接收 WS/Lua 发来的 ManualCmd
    Manual,
    /// 自动任务模式：Executor 消费 Mission 队列自主决策
    Auto,
}

impl Default for OpMode {
    fn default() -> Self {
        // 2026-08-07 决策：默认 Auto（开机即进入自动任务模式，任务下发无需先切模式）
        OpMode::Auto
    }
}
