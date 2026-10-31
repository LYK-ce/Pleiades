//Presented by KeJi
//Created Date ： 2026-08-31
//Modified Date ： 2026-10-31

//! ugv 设备驱动模块（Task 24）
//!
//! - `lg290p`：RTK 流动站（LG290P，串口）
//! - `camera_ugv`：HP60C 厂商 SDK（dlopen），默认关（task_43）

pub mod lg290p;
pub mod camera_ugv;
