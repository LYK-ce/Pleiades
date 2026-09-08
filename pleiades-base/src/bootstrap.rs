//Presented by KeJi
//Created Date ： 2026-08-06
//Modified Date ： 2026-08-30

//! 启动组装层（Task 9_2，base）
//!
//! 抽取 main.rs 的 Phase 1~6 为 `core_bootstrap()`，返回 `CoreBootstrap`。
//! 各终端（纯推理节点 / 车 / 机 / 地面站）共用：
//! - `Pleiades` bin（PC 纯推理节点）：core_bootstrap + run
//! - `pleiades-ugv` / `pleiades-uav`：core_bootstrap + 各自设备端 bootstrap
//! - `pleiades-terminal`（地面站）：core_bootstrap + run_headless
//!
//! `robot_bootstrap`（车/机设备装配）已下沉到设备端 crate（Task 23 C0）。

use std::sync::{Arc, RwLock};

use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::info;

use crate::config::{BaseConfig, Ensure_Config, Ensure_Identity, Get_Node_Type, Get_Peer_Name, CONFIG_DIR};
use crate::event_bus::EventBus;
use crate::network::{NetworkConfig, Network_Service, NodeHandle};
use crate::orchestrator::command::UserCommand;
use crate::orchestrator::core::Core;
use crate::orchestrator::Capabilities;
use crate::peer_management::create_peer_management;
use crate::storage::StorageManager;
use crate::tui::TUI_Loop;

/// core_bootstrap 返回的组装产物
pub struct CoreBootstrap {
    pub config: BaseConfig,
    pub event_bus: Arc<EventBus>,
    pub robot_bus: Arc<EventBus>,
    /// 车端命令入站通道接收端（Task 16：core_bootstrap 创建，设备端 bootstrap 转交 Robot::new）
    pub robot_cmd_frame_rx: Option<mpsc::Receiver<Vec<u8>>>,
    pub node_handle: NodeHandle,
    pub network_service: Network_Service,
    pub core: Core,
    /// 组件能力容器（含 device_caps，设备端 bootstrap 把自己的设备能力注册进来，Task 29 阶段 1）
    pub capabilities: Arc<Capabilities>,
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
    /// 供 GDExtension 桥（pleiades-terminal）等无 TUI 场景使用（Task 16）。
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
    let node_type = Get_Node_Type(&config).as_str().to_string();

    let peer_manager_arc = create_peer_management(local_peer_id, peer_name, node_type);
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
            subscribe_topics: n.and_then(|n| n.subscribe_topics.clone()).unwrap_or_default(),
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
        device_caps: RwLock::new(Vec::new()),
    });

    let (user_cmd_tx, user_cmd_rx) = mpsc::channel::<UserCommand>(64);
    let core = Core::new(capabilities.clone(), user_cmd_rx, inbound_rx, net_event_rx);
    info!("Orchestrator Core 初始化完成");


    Ok(CoreBootstrap { config, event_bus, robot_bus, robot_cmd_frame_rx: Some(robot_cmd_frame_rx), node_handle, network_service, core, capabilities, user_cmd_tx, _log_guard: log_guard })
}
