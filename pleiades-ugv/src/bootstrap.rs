//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-08-30

//! UGV（车）设备装配（Task 23 C0：从 base 的 robot_bootstrap 拆出）

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::info;

use pleiades_base::config::Get_Peer_Name;
use pleiades_base::event_bus::EventBus;
use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};

use crate::config::UgvConfig;
use crate::ugv::robot_handler::CarDeviceHandler;
use crate::ugv::sim_handler::SimDeviceHandler;

/// UGV bootstrap：读配置 → Robot::new（共享）→ 构造 CarDeviceHandler → spawn 主循环
pub async fn ugv_bootstrap(
    config: &UgvConfig,
    node_handle: Arc<NodeHandle>,
    robot_bus: Arc<EventBus>,
    robot_cmd_frame_rx: mpsc::Receiver<Vec<u8>>,
    origin: (f32, f32, f32),
) -> Result<Arc<Robot>, String> {
    let peer_name = Get_Peer_Name(&config.base);

    // 1. 创建共享状态 + 非设备 task（不含设备 spawn / goal_service / 主循环）
    let (robot, cmd_rx) = Robot::new(
        origin,
        Some(node_handle.clone()),
        Some(robot_bus),
        Some(robot_cmd_frame_rx),
        peer_name,
    ).await?;
    let robot = Arc::new(robot);

    // 2. 构造车设备处理器（设备自管理启动；simulated=true 用模拟车，无硬件）
    let device: Arc<dyn DeviceHandler> = if config.simulated.unwrap_or(false) {
        Arc::new(SimDeviceHandler::new(robot.clone(), config.clone(), node_handle.clone(), origin))
    } else {
        Arc::new(CarDeviceHandler::new(robot.clone(), config.clone(), node_handle.clone(), origin))
    };

    // 3. spawn 主循环（后台跑；boot.run() 阻塞 core 主循环）
    let run_robot = robot.clone();
    tokio::spawn(async move {
        run_robot.run(device, cmd_rx).await;
    });
    info!("Robot 已启动（node_type=Car）");

    Ok(robot)
}
