//Presented by KeJi
//Created Date ： 2026-04-24
//Modified Date ： 2026-06-15

//! EventBus 模块 — 全局事件总线
//!
//! 模组等级 Level 0 — 不调用任何其他项目模块，仅依赖第三方 crate（tokio）。
//!
//! 独立于任何组件之外的事件广播系统，用于在系统内部传递通知型事件。
//! 基于 `tokio::sync::broadcast` 实现多生产者多消费者。
//!
//! ## 子模块
//! - event: Bus_Event 事件类型定义
//! - event_bus: EventBus 结构体实现
//!
//! ## 使用方式
//! ```ignore
//! use std::sync::Arc;
//! use pleiades::event_bus::{EventBus, Bus_Event};
//!
//! let bus = Arc::new(EventBus::New(1024));
//!
//! // 消费者订阅
//! let mut rx = bus.Subscribe();
//!
//! // 生产者发布
//! bus.Publish(Bus_Event::Notify { level: NotifyLevel::Info, message: "hello".to_string() });
//! ```

pub mod event;
pub mod event_bus;

pub use event::{Bus_Event, NotifyLevel};
pub use event_bus::EventBus;
