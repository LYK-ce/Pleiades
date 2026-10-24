//Presented by KeJi
//Created Date ： 2026-08-30
//Modified Date ： 2026-10-24

//! UAV（机）设备装配（Task 23 C0：从 base 的 robot_bootstrap 拆出）
//!
//! Task 34 Step 1：两个相机 sensor **各自独立**尝试打开、各自 `warn` 失败、互不影响、
//! 不阻断进程（默认 `[camera_pc] enabled = false` / `[camera_uav] enabled = true`）。

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use pleiades_base::config::Get_Peer_Name;
use pleiades_base::event_bus::EventBus;
use pleiades_base::network::NodeHandle;
use pleiades_base::orchestrator::{Capabilities, DeviceCapability};
use pleiades_base::robot::core::robot::{DeviceHandler, Robot};

use crate::config::UavConfig;
use crate::device::camera_pc::Pc_Camera;
use crate::device::camera_uav::format::Format_Line;
use crate::device::camera_uav::Uav_Camera;
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

    // 1.4 旧 [camera] 段：保留解析但不再读取，检测到就打一行迁移提示
    if config.camera.is_some() {
        warn!("[UAV] 检测到旧 [camera] 配置段：已忽略，请改用 [camera_pc] / [camera_uav]");
    }

    // 1.5 sensor=camera_pc（开发机摄像头，默认关；失败只 warn，不拖垮进程）
    let mut pc_index: Option<u32> = None;
    if let Some(cfg) = config.camera_pc.as_ref().filter(|c| c.enabled.unwrap_or(false)) {
        let camera = Arc::new(Pc_Camera::new(cfg.clone()));
        let started = std::time::Instant::now();
        match camera.open() {
            Ok(()) => {
                let elapsed_ms = started.elapsed().as_millis();
                pc_index = camera.Device_Index().ok();
                let describe = camera.Describe();
                capabilities
                    .device_caps
                    .write()
                    .unwrap()
                    .push(camera as Arc<dyn DeviceCapability>);
                info!(
                    "[UAV] sensor=camera_pc 已打开（耗时 {elapsed_ms}ms）：{describe}；camera_pc.capture 已注册到 Lua"
                );
            }
            Err(e) => warn!(
                "[UAV] sensor=camera_pc 打开失败（耗时 {}ms，跳过，不影响 camera_uav）：{e}",
                started.elapsed().as_millis()
            ),
        }
    }

    // 1.6 sensor=camera_uav（机载相机，默认开；同 index 时后开者只 warn）
    //
    // 为什么要自己判同 index：nokhwa 的 Linux 绑定对「同一 index 再打开」会返回**同一个**
    // `Arc<Mutex<Device>>`（不报 busy），两个 sensor 会共享设备、互踩 open_stream/stop_stream，
    // 症状是偶发 "Stream Not Started"/花帧。这里改成确定性的「后开者跳过 + 一行 warn」。
    if let Some(cfg) = config.camera_uav.as_ref().filter(|c| c.enabled.unwrap_or(true)) {
        let camera = Arc::new(Uav_Camera::new(cfg.clone()));
        let same_device = matches!((pc_index, camera.Device_Index()), (Some(a), Ok(b)) if a == b);
        if same_device {
            warn!(
                "[UAV] sensor=camera_uav 跳过：path={} 已被 camera_pc 占用（同一 /dev/videoN 只能给一个 sensor）",
                cfg.path.as_deref().unwrap_or("0")
            );
        } else {
            let started = std::time::Instant::now();
            match camera.Open() {
                Ok(()) => {
                    let elapsed_ms = started.elapsed().as_millis();
                    match camera.Info() {
                        Some(device_info) => {
                            debug!(
                                "[UAV] sensor=camera_uav 枚举格式：\n{}",
                                device_info.format_lines.join("\n")
                            );
                            info!(
                                "[UAV] sensor=camera_uav 已打开（耗时 {elapsed_ms}ms）：设备「{}」（index {}）选定 {}，理由：{}；camera_uav.capture 已注册到 Lua",
                                device_info.name,
                                device_info.index,
                                Format_Line(device_info.format),
                                device_info.reason
                            );
                        }
                        None => info!(
                            "[UAV] sensor=camera_uav 已打开（耗时 {elapsed_ms}ms，未取到设备信息）；camera_uav.capture 已注册到 Lua"
                        ),
                    }
                    capabilities
                        .device_caps
                        .write()
                        .unwrap()
                        .push(camera as Arc<dyn DeviceCapability>);
                }
                Err(e) => warn!(
                    "[UAV] sensor=camera_uav 打开失败（耗时 {}ms，跳过，不影响 camera_pc）：{e}",
                    started.elapsed().as_millis()
                ),
            }
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
