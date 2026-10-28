//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-28

//! 队列元素定义（Task 36 §12.1）
//!
//! - `Mission`：任务参数（纯协议值，不带队列/树状态），自 `core/command.rs` 逐字搬迁
//! - `TaskId`：入队时由 `MissionQueue` 分配，单调递增、永不复用
//! - `QueuedTask`：队列元素 = 编号 + 任务参数
//!
//! 元素**不携带树实例**（§十一 #18）：树实例由设备端按 `TaskId` 存在自己的表里（§十一 #34）。

/// 任务单元（Task 36 §十一 #27：协同上移指挥层，车端只接自己的目标点）
#[derive(Debug, Clone, PartialEq)]
pub enum Mission {
    /// 前往目标点（世界坐标，米）
    ///
    /// Task 36 起只有单车语义：`(x, y)` 就是本车目标；不再携带成员名单，
    /// 也不再在车端做分配（分配上移指挥层，地面站逐车发帧）。
    Goto { x: f32, y: f32 },
    /// 跟随目标（Task 37 §五）：**只在 sim 的 uav 栈实现**；`x/y` 预留（现在恒 0）
    ///
    /// 真机（`pleiades-uav` / `pleiades-ugv`）收到即建平凡树（一拍 `Success`）：
    /// 不进跟随逻辑，只让队列前进一条（**不得**返回 `Err`，那是「清队列 + 停车」）。
    Follow { x: f32, y: f32 },
    /// 直线跟随（Task 37 §六）：**只在 sim 的 uav 栈实现**；`x/y` 预留（现在恒 0）
    ///
    /// 与 `Follow` 的差别：不设导航目标、不寻路，`next_cell` 恒取「沿 `pose.yaw` 前进
    /// 一格」（0.5 m）——executor 的 `delta ≈ 0`，只发 `MoveForward`，不触发转向（§六.3）。
    ///
    /// **`TASK_SET` 的 `type = 3` 仅预留**：本步协议**不解码**它（`messages.rs` /
    /// `command_decode.rs` 一行不动），设备端收到 type=3 仍按未知 `type` `warn` 丢弃
    /// （整帧语义不变，不影响 Pictor）。入队路径只有 sim 控制口 `{"cmd":"follow_straight"}`。
    ///
    /// 真机（`pleiades-uav` / `pleiades-ugv`）与两个 sim **车**栈收到即建平凡树（一拍 `Success`）。
    FollowStraight { x: f32, y: f32 },
    // 未来: Patrol, Explore, ...
}

/// 任务编号（入队时分配，单调递增，永不复用）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);

/// 队列元素 = 编号 + 任务参数
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedTask {
    pub id: TaskId,
    pub mission: Mission,
}
