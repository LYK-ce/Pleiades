//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-31

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! 设备能力占位（sim 侧）：**Lua 表 `camera_sim` 已移除**（task_43 §二 #5）。
//!
//! 原实现（`capture()` / `info()` 两个成员 + `Sensor_Info` 三级回退 + 帧龄过期告警）
//! 全部只服务于 Lua，随 `camera_uav` / `camera_pc` / `camera_sim` 三张表一起删除；
//! 业务取帧改走 `CameraSensor::Latest()`（Rust 侧，detector 用 `FrameSource`）。
//!
//! 保留本文件与 `CameraSimCap` 是因为装配点在 `sim_handler.rs`（不在 task_43 Step 1
//! 的允许路径），`New(Arc<CameraSensor>)` 签名与 `Arc<dyn DeviceCapability>` 注入契约
//! 必须原地不变。

use std::sync::Arc;

use pleiades_base::orchestrator::DeviceCapability;

use crate::sim::sensors::camera::CameraSensor;

/// 设备能力占位：注册行为为空（口径 §二 #5：sim 不再有 `camera_sim` 表）
pub struct CameraSimCap;

impl CameraSimCap {
    /// 参数只为保持装配点签名不变；本占位不持有传感器
    pub fn New(_sensor: Arc<CameraSensor>) -> Self {
        Self
    }
}

impl DeviceCapability for CameraSimCap {
    fn register_lua_caps(&self, _lua: &mlua::Lua) -> mlua::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_sim_table_is_gone() {
        let lua = mlua::Lua::new();
        assert!(CameraSimCap.register_lua_caps(&lua).is_ok());

        assert!(
            lua.globals().get::<mlua::Table>("camera_sim").is_err(),
            "camera_sim 表必须已移除（task_43 §二 #5）"
        );
        // 三张相机表是一起删的：确认这里没"换个名字"偷偷注册
        assert!(lua.globals().get::<mlua::Table>("camera_uav").is_err());
        assert!(lua.globals().get::<mlua::Table>("camera_pc").is_err());
    }
}
