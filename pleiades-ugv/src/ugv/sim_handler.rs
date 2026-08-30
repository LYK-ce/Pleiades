//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! 模拟车设备处理器（无硬件，用于测试决策链）
//!
//! 与 `CarDeviceHandler` 的区别：不 spawn stm32/lidar，`on_tick` 仍跑完整决策链
//! （`GoalService` 寻路 + `DecisionExecutor` 三状态机），但「执行动作」只打印、不驱动硬件。
//! 便于无硬件环境下验证「命令 → 任务 → 寻路 → 决策 → 动作」整条链。

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::info;

use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::UgvConfig;
use crate::ugv::executor::DecisionExecutor;
use crate::ugv::goal::{GoalService, ARRIVAL_THRESHOLD_M};

/// start() 后初始化的「设备组」（无硬件，只有决策组件）
struct SimInner {
    goal_service: Arc<Mutex<GoalService>>,
    executor: DecisionExecutor,
}

pub struct SimDeviceHandler {
    robot: Arc<Robot>,
    config: UgvConfig,
    node_handle: Arc<NodeHandle>,
    origin: (f32, f32, f32),
    inner: OnceLock<SimInner>,
}

impl SimDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: UgvConfig,
        node_handle: Arc<NodeHandle>,
        origin: (f32, f32, f32),
    ) -> Self {
        Self { robot, config, node_handle, origin, inner: OnceLock::new() }
    }
}

#[async_trait]
impl DeviceHandler for SimDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let obstacle_inflation_radius = self.config.obstacle_inflation_radius.unwrap_or(0.2);

        // 无硬件：只建决策链组件（寻路 + 决策器）
        let goal_service = Arc::new(Mutex::new(GoalService::new(
            self.robot.robot_state.clone(),
            self.robot.mission_queue.clone(),
            self.node_handle.Get_Local_Peer_Id().to_bytes(),
            self.robot.grid.clone(),
            self.robot.cluster_table.clone(),
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        )));

        self.inner.set(SimInner { goal_service, executor: DecisionExecutor::new() })
            .map_err(|_| "设备已启动".to_string())?;
        info!("[Sim] 模拟车设备启动（无硬件，只跑决策链 + 打印动作）");
        Ok(())
    }

    async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        info!("[Sim] 手动命令（不驱动硬件）: {cmd:?}");
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        d.goal_service.lock().await.reset();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        info!("[Sim] reset（清 goal + 清意图）");
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        // 完整决策链：寻路 → 决策（与真车一致）
        let next_cell = d.goal_service.lock().await.get_path().await;
        let current = execute_state.read().await.clone();
        let result = d.executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
        *execute_state.write().await = ExecuteState {
            state: result.state,
            sub_target: result.sub_target,
        };
        // 模拟「执行动作」：只打印，不驱动硬件
        if let Some(action) = result.action {
            info!("[Sim] 位置=({:.2},{:.2}) yaw={:.2}° 状态={:?} 动作={:?} sub_target={:?}",
                rs.x, rs.y, rs.attitude.yaw.to_degrees(), result.state, action, result.sub_target);
        }
    }

    fn stop(&self) {
        info!("[Sim] stop");
    }

    async fn shutdown(&self) {
        info!("[Sim] shutdown");
    }
}
