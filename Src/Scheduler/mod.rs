//Presented by KeJi
//Date ： 2026-05-04

//! Scheduler 模块 — 分布式流水线拓扑规划
//!
//! 提供 `Scheduler_Capability` trait 及两种调度策略：
//! - `Scheduler_Strategy::Uniform` — 均匀分配
//! - `Scheduler_Strategy::Weighted` — 综合成本加权分配
//!
//! 策略通过 `Scheduler_Input.strategy` 指定，`Scheduler_Service::Plan_Pipeline` 内部分发。
//!
//! ## 模块结构
//! - `capability.rs`: trait 定义 + 数据结构 + 策略枚举
//! - `service.rs`: Scheduler_Service（统一入口，策略内部分发）
//! - `weighted.rs`: Scheduler_Weighted（便捷包装，固定 Weighted 策略）

pub mod capability;
pub mod service;
pub mod weighted;

// 聚合导出
pub use capability::{
    Scheduler_Capability, Scheduler_Strategy,
    Scheduler_Error, Scheduler_Input,
    Pipeline_Plan, Worker_Assignment,
};
pub use service::Scheduler_Service;
pub use weighted::Scheduler_Weighted;
