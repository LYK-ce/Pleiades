//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-09-08

//! UAV（机）设备装配（Task 23 C0：从 base 的 robot_bootstrap 拆出）

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{info, warn};

use pleiades_base::config::Get_Peer_Name;
use pleiades_base::event_bus::EventBus;
use pleiades_base::network::NodeHandle;
use pleiades_base::orchestrator::{Capabilities, DeviceCapability};
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};

use crate::config::UavConfig;
use crate::device::camera::CameraDevice;
use crate::uav::robot_handler::UavDeviceHandler;

/// UAV bootstrap：读配置 → Robot::new（共享）→ 构造 UavDeviceHandler → spawn 主循环
pub async fn uav_bootstrap(
    config: &UavConfig,
    node_handle: Arc<NodeHandle>,
    robot_bus: Arc<EventBus>,
    robot_cmd_frame_rx: mpsc::Receiver<Vec<u8>>,
    origin: (f32, f32, f32),
    capabilities: Arc<Capabilities>,
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

    // 1.5 摄像头设备（enabled 才 open + 注册进 device_caps，失败告警不拖垮，Task 29 阶段 1）
    if let Some(cfg) = config.camera.as_ref().filter(|c| c.enabled.unwrap_or(false)) {
        let camera = Arc::new(CameraDevice::new(cfg.clone()));
        match camera.open() {
            Ok(()) => {
                capabilities.device_caps.write().unwrap().push(camera as Arc<dyn DeviceCapability>);
                info!("[UAV] 摄像头已打开，camera.capture 已注册到 Lua");
            }
            Err(e) => warn!("[UAV] 摄像头打开失败（跳过注册）: {e}"),
        }
    }

    // 2. 构造机设备处理器
    let device: Arc<dyn DeviceHandler> = Arc::new(UavDeviceHandler::new(
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
    info!("Robot 已启动（node_type=Uav）");

    Ok(robot)
}
