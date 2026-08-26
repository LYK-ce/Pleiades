//Presented by KeJi
//Created Date ： 2026-08-06
//Modified Date ： 2026-08-26

//! 启动组装层（Task 9_2）
//!
//! 抽取 main.rs 的 Phase 1~6 为 `core_bootstrap()`，返回 `CoreBootstrap`；
//! `robot_bootstrap()` 组装 Jetson 车载完整节点（Robot + 网络数据面）。
//!
//! 两个入口共用：
//! - `main.rs`（Pleiades，PC 纯推理节点）：core_bootstrap + run
//! - `main_robot.rs`（orion-robot，车载完整节点）：core_bootstrap + robot_bootstrap + run

use std::sync::Arc;

use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::{Ensure_Config, Ensure_Identity, Get_Peer_Name, Pleiades_Config, CONFIG_DIR};
use crate::event_bus::EventBus;
use crate::network::{NetworkConfig, Network_Service, NodeHandle};
use crate::orchestrator::command::UserCommand;
use crate::orchestrator::core::Core;
use crate::orchestrator::Capabilities;
use crate::peer_management::create_peer_management;
use crate::robot::control::types::CarType;
use crate::robot::Robot;
use crate::storage::StorageManager;
use crate::tui::TUI_Loop;

/// core_bootstrap 返回的组装产物
pub struct CoreBootstrap {
    pub config: Pleiades_Config,
    pub event_bus: Arc<EventBus>,
    pub robot_bus: Arc<EventBus>,
    /// 车端命令入站通道接收端（Task 16：core_bootstrap 创建，robot_bootstrap 转交 Robot::launch）
    pub robot_cmd_frame_rx: Option<mpsc::Receiver<Vec<u8>>>,
    pub node_handle: NodeHandle,
    pub network_service: Network_Service,
    pub core: Core,
    pub user_cmd_tx: mpsc::Sender<UserCommand>,
    /// 日志 worker 保活（drop 即关闭日志线程——必须存活到进程退出，Task 9_2 实测修复）
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
}

impl CoreBootstrap {
    /// 启动运行时（network 事件循环 + TUI + Core 主循环），阻塞直到退出
    pub async fn run(self) {
        self.run_internal(true).await;
    }

    /// 启动运行时（无头：network 事件循环 + Core 主循环，不启动 TUI），阻塞直到退出
    ///
    /// 供 GDExtension 桥（pictor-kernel）等无 TUI 场景使用（Task 16）。
    pub async fn run_headless(self) {
        self.run_internal(false).await;
    }

    /// 内部实现：`with_tui` 决定是否 spawn TUI
    async fn run_internal(self, with_tui: bool) {
        // network 事件循环（Task 9_2 决策 #2：内部 spawn，调用方不用管）
        let mut network_service = self.network_service;
        tokio::spawn(async move {
            if let Err(e) = network_service.Start().await {
                tracing::error!("Network 事件循环异常退出: {}", e);
            }
        });

        if with_tui {
            // TUI 模式（两个入口一致；CLI 模式已去除——`cli` 参数与 orion-robot 位置参数冲突，人类 2026-08-06 决策）
            let event_rx = self.event_bus.Subscribe();
            tokio::task::spawn_blocking(move || {
                TUI_Loop(event_rx, self.user_cmd_tx);
            });
        }
        info!("进入 Orchestrator 主循环（{}）", if with_tui { "TUI" } else { "headless" });

        // 先订阅再 flush：本机 peer_info_updated 一次性事件必须被订阅者收到（broadcast 无重放，2026-08-09 与 ML_review 同步修复）
        self.core.spawn_initial_flush();

        self.core.run().await;
        info!("Pleiades 已退出");
    }
}

/// 主枝 bootstrap：配置 → 日志 → 基础组件 → Network → Core（原 main.rs Phase 1~6）
pub async fn core_bootstrap() -> Result<CoreBootstrap, Box<dyn std::error::Error>> {
    // ══════════════════════════════════════════════════════
    // Phase 1: 配置 & 身份
    // ══════════════════════════════════════════════════════

    let (config, _config_path) = Ensure_Config()?;
    let keypair = Ensure_Identity(std::path::Path::new(CONFIG_DIR))?;
    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir)?;
    std::fs::create_dir_all(crate::config::kvcache_dir())?;

    // ══════════════════════════════════════════════════════
    // Phase 2: 初始化 tracing 日志
    // ══════════════════════════════════════════════════════

    let log_dir = config.log_dir(&workspace_dir);
    std::fs::create_dir_all(&log_dir)?;
    let log_level = config.log_level();
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));
    let timestamp = chrono::Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();
    let log_path = log_dir.join(format!("pleiades.log.{timestamp}"));
    let log_file = std::fs::File::create(&log_path)?;
    let (non_blocking, log_guard) = tracing_appender::non_blocking(log_file);
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_writer(non_blocking)
        .init();

    info!("Pleiades 启动中...");
    info!("工作目录: {}", workspace_dir.display());
    info!("日志目录: {}", log_dir.display());
    info!("日志文件: {}", log_path.display());

    // ══════════════════════════════════════════════════════
    // Phase 3: 创建基础组件
    // ══════════════════════════════════════════════════════

    let event_bus = Arc::new(EventBus::New(1024));
    let robot_bus = Arc::new(EventBus::New(1024));
    // 车端命令入站通道（request-response DataType::Robot 命令帧，Task 16）
    let (robot_cmd_frame_tx, robot_cmd_frame_rx) = mpsc::channel::<Vec<u8>>(64);

    let local_peer_id = PeerId::from(keypair.public());
    let peer_name = Get_Peer_Name(&config);

    let peer_manager_arc = create_peer_management(local_peer_id, peer_name);
    let peer_capability_for_network: Arc<dyn crate::peer_management::Peer_Management_Capability> = peer_manager_arc.clone();
    let peer_capability_for_core = peer_manager_arc.clone();

    // Storage 持有 peer_manager + event_bus，flush 时自动同步
    let storage_peer_handle: Arc<dyn crate::peer_management::Peer_Management_Capability> =
        peer_manager_arc.clone();
    let storage: Arc<dyn crate::storage::StorageCapability> = Arc::new(
        StorageManager::New(
            &workspace_dir,
            storage_peer_handle,
            event_bus.clone(),
        ).await?
    );
    info!("Storage 初始化完成: dir={}", workspace_dir.display());

    // ══════════════════════════════════════════════════════
    // Phase 4: 创建服务组件
    // ══════════════════════════════════════════════════════

    let net_cfg = {
        let n = config.Network.as_ref();
        NetworkConfig {
            lan_enabled:        n.and_then(|n| n.LAN).unwrap_or(true),
            wan_enabled:        n.and_then(|n| n.WAN).unwrap_or(false),
            transport_protocol: n.and_then(|n| n.Transport_Protocol.clone()).unwrap_or_else(|| "TCP".to_string()),
            listen_port:        n.and_then(|n| n.listen_port).unwrap_or(0),
            dht_namespace:       n.and_then(|n| n.dht_namespace.clone())
                                    .unwrap_or_else(|| crate::network::DHT::DEFAULT_NODE_NAMESPACE.to_string()),
            bootstrap_peers:     n.and_then(|n| n.bootstrap_peers.clone()).unwrap_or_default(),
            cleanup_interval:   n.and_then(|n| n.cleanup_interval).unwrap_or(300),
            timeout_interval:   n.and_then(|n| n.timeout_interval).unwrap_or(300),
            heartbeat_interval: n.and_then(|n| n.heartbeat_interval).unwrap_or(60),
            heartbeat_timeout:  n.and_then(|n| n.heartbeat_timeout).unwrap_or(10),
            request_response_timeout: n.and_then(|n| n.request_response_timeout).unwrap_or(300),
        }
    };

    let (network_service, node_handle, inbound_rx, net_capability, net_event_rx)
        = Network_Service::Init(net_cfg, keypair, peer_capability_for_network, event_bus.clone(), robot_bus.clone(), robot_cmd_frame_tx).await?;
    info!("Network 服务初始化完成");

    // ══════════════════════════════════════════════════════
    // Phase 5: 组装 Orchestrator
    // ══════════════════════════════════════════════════════

    let capabilities = Arc::new(Capabilities {
        storage,
        network: Box::new(net_capability),
        peer_manager: peer_capability_for_core,
        event_bus: event_bus.clone(),
        local_stream_hub: Arc::new(crate::orchestrator::local_tensor_stream::LocalStreamHub::new()),
    });

    let (user_cmd_tx, user_cmd_rx) = mpsc::channel::<UserCommand>(64);
    let core = Core::new(capabilities.clone(), user_cmd_rx, inbound_rx, net_event_rx);
    info!("Orchestrator Core 初始化完成");


    Ok(CoreBootstrap { config, event_bus, robot_bus, robot_cmd_frame_rx: Some(robot_cmd_frame_rx), node_handle, network_service, core, user_cmd_tx, _log_guard: log_guard })
}

/// Robot bootstrap：读取 [Robot] 段配置 → Robot::launch（注入 node_handle/robot_bus）→ WS 遥控
///
/// - 缺省值沿用原硬编码（/dev/myserial、115200、X3Plus、/dev/rplidar、230400）
/// - car_type 解析失败 warn 回退 X3Plus（Task 9_2 决策 #8）
/// - vehicle_id = peer_name = 车名，全链路统一（Task 9_2 决策 #6）
pub async fn robot_bootstrap(
    config: &Pleiades_Config,
    node_handle: Arc<NodeHandle>,
    robot_bus: Arc<EventBus>,
    robot_cmd_frame_rx: mpsc::Receiver<Vec<u8>>,
    origin: (f32, f32, f32),
) -> Result<Robot, String> {
    let r = config.Robot.as_ref();

    // 设备开关（Task 22）：chassis/lidar 缺省启用，flight_ctrl 缺省禁用
    let chassis_enabled = r.and_then(|r| r.chassis.as_ref())
        .and_then(|c| c.enabled).unwrap_or(true);
    let lidar_enabled = r.and_then(|r| r.lidar.as_ref())
        .and_then(|l| l.enabled).unwrap_or(true);
    let flight_ctrl_enabled = r.and_then(|r| r.flight_ctrl.as_ref())
        .and_then(|f| f.enabled).unwrap_or(false);

    // 底盘配置
    let chassis = r.and_then(|r| r.chassis.as_ref());
    let serial_port = chassis.and_then(|c| c.port.clone())
        .unwrap_or_else(|| "/dev/myserial".to_string());
    let baudrate = chassis.and_then(|c| c.baudrate).unwrap_or(115200);
    let car_type = match chassis.and_then(|c| c.car_type.as_deref()).map(CarType::from_str) {
        Some(Some(t)) => t,
        Some(None) => {
            warn!("car_type 解析失败，回退 X3Plus");
            CarType::X3Plus
        }
        None => CarType::X3Plus,
    };

    // 底盘速度（Task 22_5 D2：速度由设备层绑定，config 可调）
    let forward_speed = chassis.and_then(|c| c.forward_speed).unwrap_or(30).clamp(0, 100);
    let turn_speed = chassis.and_then(|c| c.turn_speed).unwrap_or(10).clamp(0, 100);

    // 雷达配置：enabled=false 或 port 空串 → 禁用；字段缺失 → 缺省 /dev/rplidar（决策 #8）
    let lidar = r.and_then(|r| r.lidar.as_ref());
    let lidar_port = if !lidar_enabled {
        None
    } else {
        match lidar.and_then(|l| l.port.clone()) {
            Some(s) if !s.trim().is_empty() => Some(s),       // 显式配置
            Some(_) => None,                                  // 留空 = 禁用
            None => Some("/dev/rplidar".to_string()),         // 缺省
        }
    };
    let lidar_baudrate = lidar.and_then(|l| l.baudrate).or(Some(230400));

    // 飞控配置（Task 22_4）：enabled=false 或 connection 空 → 禁用；字段缺失 → None（纯车）
    let flight_ctrl = r.and_then(|r| r.flight_ctrl.as_ref());
    let flight_ctrl_port = if !flight_ctrl_enabled {
        None
    } else {
        match flight_ctrl.and_then(|f| f.connection.clone()) {
            Some(s) if !s.trim().is_empty() => Some(s),
            _ => None,
        }
    };
    let flight_ctrl_baudrate = flight_ctrl.and_then(|f| f.baudrate).or(Some(921600));

    // 飞控速度（Task 22_5 D2：速度由设备层绑定，config 可调）
    let vel_fwd = flight_ctrl.and_then(|f| f.vel_fwd).unwrap_or(0.3).max(0.0);
    let yaw_rate_deg = flight_ctrl.and_then(|f| f.yaw_rate_deg).unwrap_or(15.0).max(0.0);
    if flight_ctrl_enabled && flight_ctrl_port.is_none() {
        warn!("[Robot] flight_ctrl.enabled=true 但 connection 未配置，忽略飞控（退化为纯车）");
    }

    // 车机互斥
    let chassis_enabled = if flight_ctrl_port.is_some() {
        if chassis_enabled {
            warn!("[Robot] flight_ctrl 已启用，强制禁用 chassis（车机互斥）");
        }
        false
    } else {
        chassis_enabled
    };
    let obstacle_inflation_radius = r.and_then(|r| r.obstacle_inflation_radius).unwrap_or(0.2);

    let peer_name = Get_Peer_Name(config);

    info!(
        "Robot 配置: chassis={chassis_enabled}(port={serial_port} baud={baudrate} car={car_type:?}) lidar={lidar_enabled}({}) flight_ctrl={} infl_r={obstacle_inflation_radius} peer_name={peer_name}",
        lidar_port.as_deref().unwrap_or("None"),
        flight_ctrl_port.as_deref().unwrap_or("None")
    );

    let robot = Robot::launch(
        chassis_enabled, &serial_port, baudrate, car_type,
        lidar_enabled, lidar_port.as_deref(), lidar_baudrate,
        flight_ctrl_port.as_deref(), flight_ctrl_baudrate,
        origin,
        Some(node_handle),
        Some(robot_bus),
        Some(robot_cmd_frame_rx),
        obstacle_inflation_radius,
        forward_speed,
        turn_speed,
        vel_fwd,
        yaw_rate_deg,
        peer_name.clone(),
    ).await?;
    info!("Robot 已启动");

    Ok(robot)
}
