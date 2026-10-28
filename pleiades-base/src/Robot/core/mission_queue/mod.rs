//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 任务队列（Task 36 §12.1）：元素定义 + 队列操作
//!
//! - `element`：`Mission`（任务参数）/ `TaskId`（入队编号）/ `QueuedTask`（队列元素）
//! - `queue`：`MissionQueue`（replace / append / preempt / clear / head / pop_next / len / is_empty）
//!
//! 目录化理由（§十一 #18）：元素定义与全部操作收在一处，设备端只依赖 `MissionQueue` 接口。

pub mod element;
pub mod queue;

pub use element::{Mission, QueuedTask, TaskId};
pub use queue::MissionQueue;
