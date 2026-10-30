//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-30

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
    /// 跟随目标（Task 37 §五 / §六）：**只在 sim 的 uav 栈真跟**；`x/y` 预留（现在恒 0）
    ///
    /// 真机（`pleiades-uav` / `pleiades-ugv`）与两个 sim 车栈收到即建平凡树（一拍 `Success`）：
    /// 不进跟随逻辑，只让队列前进一条（**不得**返回 `Err`，那是「清队列 + 停车」）。
    /// 跟随的「下一格」怎么算在设备端：`uav_sim/mission/follow.rs` 不经 D*，朝目标方向给一格（§六.3）。
    Follow { x: f32, y: f32 },
    /// 封控（task_41 §3.2，`type=3`）：在轴对齐正方形内巡逻，识别到人且人在正方形内 → 跟随
    ///
    /// - `x` / `y`：正方形**中心**（世界坐标，米）
    /// - `half_side`：**半边长**（米，必须 `> 0`；非法值由 `decode_task_set` 整帧拒）
    /// - 正方形轴对齐世界坐标：`|px − x| ≤ half_side && |py − y| ≤ half_side`
    /// - 只有 **sim 机栈**真实现（`uav_sim/mission/containment.rs`）；真机 ugv/uav 与 sim ugv 只建
    ///   平凡树（一拍 `Success` + warn），**不得**返回 `Err`（那是「清队列 + 停车」）
    Containment { x: f32, y: f32, half_side: f32 },
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
