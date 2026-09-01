//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! 车设备处理器（Task 23 阶段 C：实现 base 的 DeviceHandler）
//!
//! 统一契约：`start()` 里读配置 + spawn 车自己的设备（stm32 + lidar）。
//! `enabled=false` → 跳过该设备（统一跳过语义，节点照常跑；无运动设备时 on_tick 保持 Idle）。

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use pleiades_base::event_bus::EventBus;
use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::command::ManualCmd;
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};
use pleiades_base::robot::core::state::{DecisionState, ExecuteState, RobotState};

use crate::config::UgvConfig;
use crate::device::lg290p::Lg290pDevice;
use crate::ugv::emergency_stop::check_emergency_stop;
use crate::ugv::executor::DecisionExecutor;
use crate::ugv::goal::{GoalService, ARRIVAL_THRESHOLD_M};
use crate::ugv::lidar::LidarDevice;
use crate::ugv::slam::SlamContext;
use crate::ugv::stm32::STM32Device;
use crate::ugv::types::CarType;

/// start() 后初始化的设备组
struct CarInner {
    /// 底盘（enabled=false 时为 None）
    stm32: Option<Arc<STM32Device>>,
    lidar: Option<LidarDevice>,
    /// RTK 流动站（enabled=false 时为 None）
    lg290p: Option<Lg290pDevice>,
    goal_service: Arc<Mutex<GoalService>>,
    executor: DecisionExecutor,
}

pub struct CarDeviceHandler {
    robot: Arc<Robot>,
    config: UgvConfig,
    origin: (f32, f32, f32),
    node_handle: Arc<NodeHandle>,
    robot_bus: Arc<EventBus>,
    inner: OnceLock<CarInner>,
}

impl CarDeviceHandler {
    pub fn new(
        robot: Arc<Robot>,
        config: UgvConfig,
        node_handle: Arc<NodeHandle>,
        robot_bus: Arc<EventBus>,
        origin: (f32, f32, f32),
    ) -> Self {
        Self { robot, config, origin, node_handle, robot_bus, inner: OnceLock::new() }
    }
}

#[async_trait]
impl DeviceHandler for CarDeviceHandler {
    async fn start(&self) -> Result<(), String> {
        if self.inner.get().is_some() {
            return Ok(()); // 幂等
        }
        let r = &self.config;

        // 底盘配置
        let chassis = r.chassis.as_ref();
        let chassis_enabled = chassis.and_then(|c| c.enabled).unwrap_or(true);

        // 雷达配置
        let lidar = r.lidar.as_ref();
        let lidar_enabled = lidar.and_then(|l| l.enabled).unwrap_or(true);
        let lidar_port = if !lidar_enabled { None } else {
            match lidar.and_then(|l| l.port.clone()) {
                Some(s) if !s.trim().is_empty() => Some(s),
                Some(_) => None,
                None => Some("/dev/rplidar".to_string()),
            }
        };
        let lidar_baudrate = lidar.and_then(|l| l.baudrate).or(Some(230400));

        let obstacle_inflation_radius = r.obstacle_inflation_radius.unwrap_or(0.2);

        // 底盘：enabled=false → 跳过（统一跳过语义）
        let stm32 = if chassis_enabled {
            let serial_port = chassis.and_then(|c| c.port.clone()).unwrap_or_else(|| "/dev/myserial".to_string());
            let baudrate = chassis.and_then(|c| c.baudrate).unwrap_or(115200);
            let car_type = match chassis.and_then(|c| c.car_type.as_deref()).map(CarType::from_str) {
                Some(Some(t)) => t,
                Some(None) => { warn!("car_type 解析失败，回退 X3Plus"); CarType::X3Plus }
                None => CarType::X3Plus,
            };
            let forward_speed = chassis.and_then(|c| c.forward_speed).unwrap_or(30).clamp(0, 100);
            let turn_speed = chassis.and_then(|c| c.turn_speed).unwrap_or(10).clamp(0, 100);

            info!("启用底盘: port={serial_port} baud={baudrate} car={car_type:?}");
            Some(Arc::new(STM32Device::spawn(&serial_port, baudrate, car_type, self.robot.robot_state.clone(), self.origin, forward_speed, turn_speed)?))
        } else {
            info!("底盘未启用（enabled=false），跳过");
            None
        };

        // 目标服务（无底盘时也创建，on_tick 无运动设备时保持 Idle，任务不消费）
        let goal_service = Arc::new(Mutex::new(GoalService::new(
            self.robot.robot_state.clone(),
            self.robot.mission_queue.clone(),
            self.node_handle.Get_Local_Peer_Id().to_bytes(),
            self.robot.grid.clone(),
            self.robot.cluster_table.clone(),
            obstacle_inflation_radius,
            ARRIVAL_THRESHOLD_M,
        )));

        // 雷达（含 SLAM）：enabled=false / 未配置 → 跳过
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
                let d = match LidarDevice::spawn(&p, b, Some(slam_ctx)) {
                    Ok(d) => d,
                    Err(e) => {
                        // 雷达 spawn 失败：清理已启动的底盘，避免后台 task 泄漏（Task 23 review 修复）
                        if let Some(s) = &stm32 { s.shutdown(); }
                        return Err(e);
                    }
                };
                if let Err(e) = d.start_scan().await {
                    d.shutdown();
                    if let Some(s) = &stm32 { s.shutdown(); }
                    return Err(e);
                }
                Some(d)
            }
            _ => { info!("LiDAR 未配置，跳过"); None }
        };

        // RTK 流动站（LG290P）：enabled=false → 跳过（Task 24）
        let lg290p_cfg = r.lg290p.as_ref();
        let lg290p_enabled = lg290p_cfg.and_then(|l| l.enabled).unwrap_or(false);
        let lg290p = if lg290p_enabled {
            let port = lg290p_cfg
                .and_then(|l| l.port.clone())
                .unwrap_or_else(|| "/dev/ttyUSB2".to_string());
            let baudrate = lg290p_cfg.and_then(|l| l.baudrate).unwrap_or(460800);
            info!("启用 LG290P: port={port} baud={baudrate}");
            match Lg290pDevice::spawn(
                &port,
                baudrate,
                self.robot_bus.clone(),
                self.robot.robot_state.clone(),
                self.origin,
            ) {
                Ok(d) => Some(d),
                Err(e) => {
                    // spawn 失败：清理已启动的 stm32/lidar，避免后台 task 泄漏
                    if let Some(s) = &stm32 {
                        s.shutdown();
                    }
                    if let Some(l) = &lidar {
                        l.shutdown();
                    }
                    return Err(e);
                }
            }
        } else {
            info!("LG290P 未启用（enabled=false），跳过");
            None
        };

        self.inner.set(CarInner { stm32, lidar, lg290p, goal_service, executor: DecisionExecutor::new() })
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
            _ => {
                if let Some(s) = &d.stm32 {
                    s.handle_manual_cmd(cmd);
                }
            }
        }
    }

    async fn reset(&self, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        if let Some(s) = &d.stm32 { let _ = s.stop(); }
        d.goal_service.lock().await.reset();
        *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
    }

    async fn on_tick(&self, rs: &RobotState, execute_state: &Arc<RwLock<ExecuteState>>) {
        let d = self.inner.get().expect("device not started");
        let Some(stm32) = &d.stm32 else {
            // 无底盘：无运动设备，保持 Idle（节点照常广播位姿/地图）
            *execute_state.write().await = ExecuteState { state: DecisionState::Idle, sub_target: None };
            return;
        };
        let emergency = match &d.lidar {
            Some(l) => check_emergency_stop(l, rs, stm32, &d.goal_service).await,
            None => false,
        };
        if emergency {
            // 急停：停车 + 清意图，保留 goal 供绕行
            let _ = stm32.stop();
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
                stm32.apply_action(action);
            }
        }
    }

    fn stop(&self) {
        if let Some(d) = self.inner.get() {
            if let Some(s) = &d.stm32 { let _ = s.stop(); }
        }
    }

    async fn shutdown(&self) {
        let Some(d) = self.inner.get() else { return; };
        if let Some(l) = &d.lidar {
            info!("正在停止 LiDAR...");
            let _ = l.stop_scan().await;
            l.shutdown();
        }
        if let Some(s) = &d.stm32 { s.shutdown(); }
        if let Some(g) = &d.lg290p { g.shutdown(); }
    }
}
