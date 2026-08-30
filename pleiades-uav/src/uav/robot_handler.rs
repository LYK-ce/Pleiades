//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! 机设备处理器（Task 23 阶段 C：实现 base 的 DeviceHandler）
//!
//! 统一契约：`start()` 里读配置 + spawn 机自己的设备（mavlink + goal_service + 决策器）。
//! 当前（2026-08-30）把无人机当作「飞在天上的无人小车」：决策/寻路复用车的 2D 三状态机，
//! 高度 z 暂不纳入决策（后续 task 3D 化）。

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
    mavlink: Arc<MavlinkDevice>,
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
        let obstacle_inflation_radius = self.config.obstacle_inflation_radius.unwrap_or(0.2);

        info!("启用飞控: port={port}, baud={baudrate}, infl_r={obstacle_inflation_radius}");

        // spawn 飞控
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

        // 目标服务（复用车的 2D 寻路 + 群发任务分配 + 动态障碍注入）
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
        self.inner.get().expect("device not started").mavlink.handle_manual_cmd(cmd);
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        let _ = d.mavlink.stop();
        d.goal_service.lock().await.reset();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        // 机端暂无 2D 扇形急停（无 LiDAR）；复用车的 2D 决策链：寻路 → 决策 → 发动作
        let next_cell = d.goal_service.lock().await.get_path().await;
        let current = execute_state.read().await.clone();
        let result = d.executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
        *execute_state.write().await = ExecuteState {
            state: result.state,
            sub_target: result.sub_target,
        };
        if let Some(action) = result.action {
            d.mavlink.apply_action(action);
        }
    }

    fn stop(&self) {
        if let Some(d) = self.inner.get() {
            let _ = d.mavlink.stop();
        }
    }

    async fn shutdown(&self) {
        if let Some(d) = self.inner.get() {
            d.mavlink.shutdown();
        }
    }
}
