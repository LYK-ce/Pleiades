//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! 机设备处理器（Task 23 阶段 C：实现 base 的 DeviceHandler）
//!
//! 统一契约：`start()` 里读配置 + spawn 机自己的设备（mavlink + goal_service + 决策器）。
//! `enabled=false` → 跳过飞控（统一跳过语义，节点照常跑；无运动设备时 on_tick 保持 Idle）。
//! 决策/寻路当前复用车 2D 逻辑（「天上无人小车」）；3D 飞行逻辑留待后续 task。

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::info;

use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::UavConfig;
use crate::uav::executor::DecisionExecutor;
use crate::uav::goal::{GoalService, ARRIVAL_THRESHOLD_M};
use crate::uav::mavlink::{MavlinkDevice, Telemetry};

/// start() 后初始化的设备组
struct UavInner {
    /// 飞控（enabled=false 时为 None）
    mavlink: Option<Arc<MavlinkDevice>>,
    goal_service: Arc<Mutex<GoalService>>,
    executor: DecisionExecutor,
}

pub struct UavDeviceHandler {
    robot: Arc<Robot>,
    config: UavConfig,
    node_handle: Arc<NodeHandle>,
    origin: (f32, f32, f32),
    inner: OnceLock<UavInner>,
}

impl UavDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: UavConfig,
        node_handle: Arc<NodeHandle>,
        origin: (f32, f32, f32),
    ) -> Self {
        Self { robot, config, node_handle, origin, inner: OnceLock::new() }
    }
}

#[async_trait]
impl DeviceHandler for UavDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let flight_ctrl = self.config.flight_ctrl.as_ref();
        let flight_ctrl_enabled = flight_ctrl.and_then(|f| f.enabled).unwrap_or(false);
        let obstacle_inflation_radius = self.config.obstacle_inflation_radius.unwrap_or(0.2);

        // 飞控：enabled=false → 跳过（统一跳过语义）
        let mavlink = if flight_ctrl_enabled {
            let port = match flight_ctrl.and_then(|f| f.connection.clone()) {
                Some(s) if !s.trim().is_empty() => Some(s),
                _ => None,
            };
            let Some(port) = port else {
                return Err("flight_ctrl.enabled=true 但 connection 未配置".to_string());
            };
            let baudrate = flight_ctrl.and_then(|f| f.baudrate).unwrap_or(921600);
            let vel_fwd = flight_ctrl.and_then(|f| f.vel_fwd).unwrap_or(0.3).max(0.0);
            let yaw_rate_deg = flight_ctrl.and_then(|f| f.yaw_rate_deg).unwrap_or(15.0).max(0.0);

            info!("启用飞控: port={port}, baud={baudrate}, infl_r={obstacle_inflation_radius}");
            let flight_state = Arc::new(RwLock::new(Telemetry::default()));
            Some(Arc::new(MavlinkDevice::spawn(
                &port,
                baudrate,
                flight_state,
                Some(self.robot.robot_state.clone()),
                self.origin,
                vel_fwd,
                yaw_rate_deg,
            )?))
        } else {
            info!("飞控未启用（enabled=false），跳过");
            None
        };

        // 目标服务（无飞控时也创建，on_tick 无运动设备时保持 Idle，任务不消费）
        let goal_service = Arc::new(Mutex::new(GoalService::new(
            self.robot.robot_state.clone(),
            self.robot.mission_queue.clone(),
            self.node_handle.Get_Local_Peer_Id().to_bytes(),
            self.robot.grid.clone(),
            self.robot.cluster_table.clone(),
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        )));

        self.inner.set(UavInner { mavlink, goal_service, executor: DecisionExecutor::new() })
            .map_err(|_| "设备已启动".to_string())?;
        Ok(())
    }

    async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        let d = self.inner.get().expect("device not started");
        if let Some(m) = &d.mavlink {
            m.handle_manual_cmd(cmd);
        }
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        if let Some(m) = &d.mavlink { let _ = m.stop(); }
        d.goal_service.lock().await.reset();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        let Some(mavlink) = &d.mavlink else {
            // 无飞控：保持 Idle（节点照常广播位姿/遥测）
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
            return;
        };
        // 机端暂无 2D 扇形急停（无 LiDAR）；复用车的 2D 决策链：寻路 → 决策 → 发动作
        let next_cell = d.goal_service.lock().await.get_path().await;
        let current = execute_state.read().await.clone();
        let result = d.executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
        *execute_state.write().await = ExecuteState {
            state: result.state,
            sub_target: result.sub_target,
        };
        if let Some(action) = result.action {
            mavlink.apply_action(action);
        }
    }

    fn stop(&self) {
        if let Some(d) = self.inner.get() {
            if let Some(m) = &d.mavlink { let _ = m.stop(); }
        }
    }

    async fn shutdown(&self) {
        if let Some(d) = self.inner.get() {
            if let Some(m) = &d.mavlink { m.shutdown(); }
        }
    }
}
