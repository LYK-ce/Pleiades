//Presented by KeJi
//Date ： 2026-05-04

//! Scheduler 模块 — 分布式流水线拓扑规划
//!
//! 提供 `Scheduler_Capability` trait 接口及默认的均匀分配策略实现 `Scheduler_Service`。
//!
//! ## 模块结构
//! - `capability.rs`: trait 定义 + 数据结构（Pipeline_Plan, Worker_Assignment 等）
//! - `service.rs`: Scheduler_Service（均匀分配策略实现）

pub mod capability;
pub mod service;

// 聚合导出
pub use capability::{
    Scheduler_Capability,
    Scheduler_Error,
    Scheduler_Input,
    Pipeline_Plan,
    Worker_Assignment,
};
pub use service::Scheduler_Service;
