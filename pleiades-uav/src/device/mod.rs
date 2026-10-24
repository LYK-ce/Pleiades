//Presented by KeJi
//Created Date ： 2026-09-08
//Modified Date ： 2026-10-24

//! 机设备端子设备（与 mavlink/lg290p 同级，Task 28/29）
//!
//! Task 34 Step 1：相机拆成两个**彼此独立**的 sensor，互不引用、不共享抽象：
//! - [`camera_pc`]：开发机（电脑）摄像头，NV12 迁移实现，默认关；
//! - [`camera_uav`]：UAV 机载相机（SY011HD / UVC），MJPG 直通 + 格式协商，默认开。

pub mod camera_pc;
pub mod camera_uav;
