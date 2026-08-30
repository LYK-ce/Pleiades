//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! 机设备处理器（Task 23 阶段 C：实现 base 的 DeviceHandler）
//!
//! 统一契约：`start()` 里读配置 + spawn 机自己的设备（mavlink）。
//! 决策（飞行逻辑）/ 寻路（3D）/ 建图（3D 视觉）留待后续 task。

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::info;

use crate::config::Pleiades_Config;
use crate::robot::core::command::ManualCmd;
use crate::robot::core::robot::{DeviceHandler, Robot};
use crate::robot::core::state::{DecisionState, ExecuteState, RobotState};
use crate::robot::uav::mavlink::{MavlinkDevice, Telemetry};

pub struct UavDeviceHandler {
    robot: Arc<Robot>,
    config: Pleiades_Config,
    origin: (f32, f32, f32),
    inner: OnceLock<Arc<MavlinkDevice>>,
}

impl UavDeviceHandler {
    pub fn new(robot: Arc<Robot>, config: Pleiades_Config, origin: (f32, f32, f32)) -> Self {
        Self { robot, config, origin, inner: OnceLock::new() }
    }
}

#[async_trait]
impl DeviceHandler for UavDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let r = self.config.Robot.as_ref();
        let flight_ctrl = r.and_then(|r| r.flight_ctrl.as_ref());
        let flight_ctrl_enabled = flight_ctrl.and_then(|f| f.enabled).unwrap_or(false);
        let port = if !flight_ctrl_enabled { None } else {
            match flight_ctrl.and_then(|f| f.connection.clone()) {
                Some(s) if !s.trim().is_empty() => Some(s),
                _ => None,
            }
        };
        let Some(port) = port else {
            return Err("flight_ctrl.enabled=true 但 connection 未配置".to_string());
        };
        let baudrate = flight_ctrl.and_then(|f| f.baudrate).unwrap_or(921600);
        let vel_fwd = flight_ctrl.and_then(|f| f.vel_fwd).unwrap_or(0.3).max(0.0);
        let yaw_rate_deg = flight_ctrl.and_then(|f| f.yaw_rate_deg).unwrap_or(15.0).max(0.0);

        info!("启用飞控: port={port}, baud={baudrate}");
        let flight_state = Arc::new(RwLock::new(Telemetry::default()));
        let mavlink = Arc::new(MavlinkDevice::spawn(
            &port,
            baudrate,
            flight_state,
            Some(self.robot.robot_state.clone()),
            self.origin,
            vel_fwd,
            yaw_rate_deg,
        )?);
        self.inner.set(mavlink).map_err(|_| "设备已启动".to_string())?;
        Ok(())
    }

    async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        self.inner.get().expect("device not started").handle_manual_cmd(cmd);
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let m = self.inner.get().expect("device not started");
        let _ = m.stop();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    }

    async fn on_tick(&self, _rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
        // 机端飞行逻辑留待后续 task（先按 2D 小车跑通，后 3D 化）
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    }

    fn stop(&self) {
        if let Some(m) = self.inner.get() {
            let _ = m.stop();
        }
    }

    async fn shutdown(&self) {
        if let Some(m) = self.inner.get() {
            m.shutdown();
        }
    }
}
