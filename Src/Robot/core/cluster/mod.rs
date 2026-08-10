//Presented by KeJi
//Created Date ： 2026-08-10
//Modified Date ： 2026-08-10

//! 集群（多车）数据面（Task 13_1）
//!
//! - `cluster_info.rs`  远端车信息表（ClusterInfo / ClusterInfoTable）
//! - `consumer.rs`      入站 POSE 消费 task（robot_bus → 表）
//!
//! 职责：入站数据接入（gossipsub → robot_bus → 表），为后续"寻路把车当障碍注入"（P0）铺数据基础。
//! 边界：本模块不依赖 Network/EventBus 的实现细节（仅消费 robot_bus 订阅）。

pub mod cluster_info;
pub mod consumer;

pub use cluster_info::{ClusterInfo, ClusterInfoTable};
pub use consumer::cluster_consumer;
