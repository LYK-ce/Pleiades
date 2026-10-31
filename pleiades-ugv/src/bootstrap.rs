//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-31

//! UGV（车）设备装配（Task 23 C0：从 base 的 robot_bootstrap 拆出）
//!
//! task_43 Step 1：多一个 `sensor=camera_ugv`（HP60C 厂商 SDK，**默认关**）——
//! `enabled = true` 才构造并 `Open`，失败只 `warn` + 跳过，不影响 stm32 / lidar / 主循环。

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{info, warn};

use pleiades_base::config::Get_Peer_Name;
use pleiades_base::event_bus::EventBus;
use pleiades_base::network::NodeHandle;
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};

use crate::config::UgvConfig;
use crate::device::camera_ugv::Ugv_Camera;
use crate::ugv::robot_handler::CarDeviceHandler;

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
        Some(robot_bus.clone()),
        Some(robot_cmd_frame_rx),
        peer_name,
    ).await?;
    let robot = Arc::new(robot);

    // 1.5 sensor=camera_ugv（HP60C 厂商 SDK，**默认关**：相机独占 USB，与车上 ROS ascamera 互斥）
    //     失败只 warn + 跳过，不影响 stm32 / lidar / 主循环
    let mut camera: Option<Arc<Ugv_Camera>> = None;
    if let Some(cfg) = config.camera_ugv.as_ref().filter(|c| c.enabled.unwrap_or(false)) {
        let cam = Arc::new(Ugv_Camera::new(cfg.clone()));
        let started = std::time::Instant::now();
        match cam.Open() {
            Ok(()) => {
                let elapsed_ms = started.elapsed().as_millis();
                match cam.Info() {
                    Some(info) => info!(
                        "[UGV] sensor=camera_ugv 已打开（耗时 {elapsed_ms}ms）：{}",
                        info.Describe()
                    ),
                    None => info!("[UGV] sensor=camera_ugv 已打开（耗时 {elapsed_ms}ms，未取到设备信息）"),
                }
                camera = Some(cam);
            }
            Err(e) => warn!(
                "[UGV] sensor=camera_ugv 打开失败（耗时 {}ms，跳过，不影响其它设备）：{e}",
                started.elapsed().as_millis()
            ),
        }
    } else {
        info!("[UGV] camera_ugv 未启用（enabled=false），跳过");
    }

    // 2. 构造车设备处理器（设备自管理启动；task_43：多传一份相机 `Arc`，本 step 不消费）
    let device: Arc<dyn DeviceHandler> = Arc::new(CarDeviceHandler::new(
        robot.clone(),
        config.clone(),
        node_handle.clone(),
        robot_bus.clone(),
        origin,
        camera,
    ));

    // 3. spawn 主循环（后台跑；boot.run() 阻塞 core 主循环）
    let run_robot = robot.clone();
    tokio::spawn(async move {
        run_robot.run(device, cmd_rx).await;
    });
    info!("Robot 已启动（node_type=Car）");

    Ok(robot)
}
