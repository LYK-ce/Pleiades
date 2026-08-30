//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! 车设备处理器（Task 23 阶段 C：实现 base 的 DeviceHandler）
//!
//! 统一契约：`start()` 里读配置 + spawn 车自己的设备（stm32 + lidar）。
//! 包住 stm32 + lidar + goal_service + 决策器，作为「车」的整体能力。

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::config::Pleiades_Config;
use crate::network::NodeHandle;
use crate::robot::core::command::ManualCmd;
use crate::robot::core::robot::{DeviceHandler, Robot};
use crate::robot::core::state::{DecisionState, ExecuteState, RobotState};
use crate::robot::ugv::emergency_stop::check_emergency_stop;
use crate::robot::ugv::executor::DecisionExecutor;
use crate::robot::ugv::goal::{GoalService, ARRIVAL_THRESHOLD_M};
use crate::robot::ugv::lidar::LidarDevice;
use crate::robot::ugv::slam::SlamContext;
use crate::robot::ugv::stm32::STM32Device;
use crate::robot::ugv::types::CarType;

/// start() 后初始化的设备组
struct CarInner {
    stm32: Arc<STM32Device>,
    lidar: Option<LidarDevice>,
    goal_service: Arc<Mutex<GoalService>>,
    executor: DecisionExecutor,
}

pub struct CarDeviceHandler {
    robot: Arc<Robot>,
    config: Pleiades_Config,
    origin: (f32, f32, f32),
    node_handle: Arc<NodeHandle>,
    inner: OnceLock<CarInner>,
}

impl CarDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: Pleiades_Config,
        node_handle: Arc<NodeHandle>,
        origin: (f32, f32, f32),
    ) -> Self {
        Self { robot, config, origin, node_handle, inner: OnceLock::new() }
    }
}

#[async_trait]
impl DeviceHandler for CarDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let r = self.config.Robot.as_ref();

        // 底盘配置
        let chassis = r.and_then(|r| r.chassis.as_ref());
        let chassis_enabled = chassis.and_then(|c| c.enabled).unwrap_or(true);
        let serial_port = chassis.and_then(|c| c.port.clone()).unwrap_or_else(|| "/dev/myserial".to_string());
        let baudrate = chassis.and_then(|c| c.baudrate).unwrap_or(115200);
        let car_type = match chassis.and_then(|c| c.car_type.as_deref()).map(CarType::from_str) {
            Some(Some(t)) => t,
            Some(None) => { warn!("car_type 解析失败，回退 X3Plus"); CarType::X3Plus }
            None => CarType::X3Plus,
        };
        let forward_speed = chassis.and_then(|c| c.forward_speed).unwrap_or(30).clamp(0, 100);
        let turn_speed = chassis.and_then(|c| c.turn_speed).unwrap_or(10).clamp(0, 100);

        // 雷达配置
        let lidar = r.and_then(|r| r.lidar.as_ref());
        let lidar_enabled = lidar.and_then(|l| l.enabled).unwrap_or(true);
        let lidar_port = if !lidar_enabled { None } else {
            match lidar.and_then(|l| l.port.clone()) {
                Some(s) if !s.trim().is_empty() => Some(s),
                Some(_) => None,
                None => Some("/dev/rplidar".to_string()),
            }
        };
        let lidar_baudrate = lidar.and_then(|l| l.baudrate).or(Some(230400));

        let obstacle_inflation_radius = r.and_then(|r| r.obstacle_inflation_radius).unwrap_or(0.2);

        if !chassis_enabled {
            return Err("chassis 未启用".to_string());
        }

        info!("Robot 配置(车): port={serial_port} baud={baudrate} car={car_type:?} lidar={lidar_enabled}({}) infl_r={obstacle_inflation_radius}",
            lidar_port.as_deref().unwrap_or("None"));

        // spawn 底盘
        let stm32 = Arc::new(STM32Device::spawn(&serial_port, baudrate, car_type, self.robot.robot_state.clone(), self.origin, forward_speed, turn_speed)?);

        // 目标服务
        let goal_service = Arc::new(Mutex::new(GoalService::new(
            self.robot.robot_state.clone(),
            self.robot.mission_queue.clone(),
            self.node_handle.Get_Local_Peer_Id().to_bytes(),
            self.robot.grid.clone(),
            self.robot.cluster_table.clone(),
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        )));

        // spawn 雷达（含 SLAM）
        let lidar = match (lidar_port, lidar_baudrate) {
            (Some(p), Some(b)) => {
                info!("启用 LiDAR: port={p}, baud={b}（含 SLAM 建图）");
                let slam_ctx = SlamContext {
                    grid: self.robot.grid.clone(),
                    robot_state: self.robot.robot_state.clone(),
                    cluster_table: self.robot.cluster_table.clone(),
                    map_tx: self.robot.map_tx.clone(),
                    node_handle: Some(self.node_handle.clone()),
                    local_peer_id: Some(self.node_handle.Get_Local_Peer_Id().to_bytes()),
                    obstacle_inflation_radius,
                };
                let d = LidarDevice::spawn(&p, b, Some(slam_ctx))?;
                if let Err(e) = d.start_scan().await {
                    d.shutdown();
                    stm32.shutdown();
                    return Err(e);
                }
                Some(d)
            }
            _ => { info!("LiDAR 未配置，跳过"); None }
        };

        self.inner.set(CarInner { stm32, lidar, goal_service, executor: DecisionExecutor::new() })
            .map_err(|_| "设备已启动".to_string())?;
        Ok(())
    }

    async fn handle_manual_cmd(&self, cmd: &ManualCmd) {
        let d = self.inner.get().expect("device not started");
        match cmd {
            ManualCmd::StartLidarScan => {
                if let Some(l) = &d.lidar {
                    if let Err(e) = l.start_scan().await { warn!("[Robot] LiDAR 启动失败: {e}"); }
                }
            }
            ManualCmd::StopLidarScan => {
                if let Some(l) = &d.lidar {
                    if let Err(e) = l.stop_scan().await { warn!("[Robot] LiDAR 停止失败: {e}"); }
                }
            }
            _ => d.stm32.handle_manual_cmd(cmd),
        }
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        let _ = d.stm32.stop();
        d.goal_service.lock().await.reset();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        let emergency = match &d.lidar {
            Some(l) => check_emergency_stop(l, rs, &d.stm32, &d.goal_service).await,
            None => false,
        };
        if emergency {
            // 急停：停车 + 清意图，保留 goal 供绕行
            let _ = d.stm32.stop();
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
        } else {
            let next_cell = d.goal_service.lock().await.get_path().await;
            let current = execute_state.read().await.clone();
            let result = d.executor.decide(next_cell, rs.x, rs.y, rs.attitude.yaw, &current);
            *execute_state.write().await = ExecuteState {
                state: result.state,
                sub_target: result.sub_target,
            };
            if let Some(action) = result.action {
                d.stm32.apply_action(action);
            }
        }
    }

    fn stop(&self) {
        if let Some(d) = self.inner.get() {
            let _ = d.stm32.stop();
        }
    }

    async fn shutdown(&self) {
        let Some(d) = self.inner.get() else { return; };
        if let Some(l) = &d.lidar {
            info!("正在停止 LiDAR...");
            let _ = l.stop_scan().await;
            l.shutdown();
        }
        d.stm32.shutdown();
    }
}
