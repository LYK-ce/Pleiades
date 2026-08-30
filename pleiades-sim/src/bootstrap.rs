//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! SIM 设备装配（无硬件）：Robot::new（共享）→ SimDeviceHandler → spawn 主循环

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::info;

use pleiades_base::config::Get_Peer_Name;
use pleiades_base::event_bus::EventBus;
use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};

use crate::config::SimConfig;
use crate::sim::sim_handler::SimDeviceHandler;

/// SIM bootstrap：读配置 → Robot::new（共享）→ 构造 SimDeviceHandler → spawn 主循环
pub async fn sim_bootstrap(
    config: &SimConfig,
    node_handle: Arc<NodeHandle>,
    robot_bus: Arc<EventBus>,
    robot_cmd_frame_rx: mpsc::Receiver<Vec<u8>>,
    origin: (f32, f32, f32),
) -> Result<Arc<Robot>, String> {
    let peer_name = Get_Peer_Name(&config.base);

    // 1. 创建共享状态 + 非设备 task（不含设备 spawn / 主循环）
    let (robot, cmd_rx) = Robot::new(
        origin,
        Some(node_handle.clone()),
        Some(robot_bus),
        Some(robot_cmd_frame_rx),
        peer_name,
    ).await?;
    let robot = Arc::new(robot);

    // 2. 构造模拟设备处理器（无硬件，只跑决策链 + 打印）
    let device: Arc<dyn DeviceHandler> = Arc::new(SimDeviceHandler::new(
        robot.clone(),
        config.clone(),
        node_handle.clone(),
        origin,
    ));

    // 3. spawn 主循环（后台跑；boot.run() 阻塞 core 主循环）
    let run_robot = robot.clone();
    tokio::spawn(async move {
        run_robot.run(device, cmd_rx).await;
    });
    info!("Robot 已启动（node_type=Sim）");

    Ok(robot)
}
