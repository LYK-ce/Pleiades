//Presented by KeJi
//Date ： 2026-05-17

//! Pleiades 入口点

use std::sync::Arc;

use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::info;

use pleiades::config::{Ensure_Config, Ensure_Identity};
use pleiades::event_bus::EventBus;
use pleiades::peer_management::create_peer_management;
use pleiades::network::{NetworkConfig, Network_Service};
use pleiades::storage::StorageManager;
use pleiades::orchestrator::Capabilities;
use pleiades::orchestrator::core::Core;
use pleiades::orchestrator::command::UserCommand;

use pleiades::tui::TUI_Loop;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ══════════════════════════════════════════════════════
    // Phase 1: 配置 & 身份
    // ══════════════════════════════════════════════════════

    let (config, _config_path) = Ensure_Config()?;
    let keypair = Ensure_Identity(
        std::path::Path::new(pleiades::config::CONFIG_DIR)
    )?;
    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir)?;
    std::fs::create_dir_all(pleiades::config::kvcache_dir())?;

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
    let (non_blocking, _log_guard) = tracing_appender::non_blocking(log_file);
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_writer(non_blocking)
        .init();

    info!("Pleiades 启动中...");
    info!("工作目录: {}", workspace_dir.display());
    info!("日志目录: {}", log_dir.display());
    info!("日志文件: {}", log_path.display());

    let cli_mode = std::env::args().nth(1).map_or(false, |a| a == "cli");

    // ══════════════════════════════════════════════════════
    // Phase 3: 创建基础组件
    // ══════════════════════════════════════════════════════

    let event_bus = Arc::new(EventBus::New(1024));
    let robot_bus = Arc::new(EventBus::New(1024));

    let local_peer_id = PeerId::from(keypair.public());
    let peer_name = pleiades::config::Get_Peer_Name(&config);

    let peer_manager_arc = create_peer_management(local_peer_id, peer_name);
    let peer_capability_for_network: Arc<dyn pleiades::peer_management::Peer_Management_Capability> = peer_manager_arc.clone();
    let peer_capability_for_core = peer_manager_arc.clone();

    // Storage 持有 peer_manager + event_bus，flush 时自动同步
    let storage_peer_handle: Arc<dyn pleiades::peer_management::Peer_Management_Capability> =
        peer_manager_arc.clone();
    let storage: Arc<dyn pleiades::storage::StorageCapability> = Arc::new(
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
            listen_port:        0,
            bootstrap_peers:    Vec::new(),
            cleanup_interval:   n.and_then(|n| n.cleanup_interval).unwrap_or(300),
            timeout_interval:   n.and_then(|n| n.timeout_interval).unwrap_or(300),
            heartbeat_interval: n.and_then(|n| n.heartbeat_interval).unwrap_or(60),
            heartbeat_timeout:  n.and_then(|n| n.heartbeat_timeout).unwrap_or(10),
            request_response_timeout: n.and_then(|n| n.request_response_timeout).unwrap_or(300),
        }
    };

    let (mut network_service, _node_handle, inbound_rx, net_capability, net_event_rx)
        = Network_Service::Init(net_cfg, keypair, peer_capability_for_network, event_bus.clone(), robot_bus.clone()).await?;
    info!("Network 服务初始化完成");

    // ══════════════════════════════════════════════════════
    // Phase 5: 组装 Orchestrator
    // ══════════════════════════════════════════════════════

    let capabilities = Arc::new(Capabilities {
        storage,
        network: Box::new(net_capability),
        peer_manager: peer_capability_for_core,
        event_bus: event_bus.clone(),
        local_stream_hub: Arc::new(pleiades::orchestrator::local_tensor_stream::LocalStreamHub::new()),
    });

    let (user_cmd_tx, user_cmd_rx) = mpsc::channel::<UserCommand>(64);
    let core = Core::new(capabilities.clone(), user_cmd_rx, inbound_rx, net_event_rx);
    info!("Orchestrator Core 初始化完成");

    // 启动时后台 flush
    Core::spawn_initial_flush(capabilities.clone());



    // ══════════════════════════════════════════════════════
    // Phase 6: 启动运行时
    // ══════════════════════════════════════════════════════

    tokio::spawn(async move {
        if let Err(e) = network_service.Start().await {
            tracing::error!("Network 事件循环异常退出: {}", e);
        }
    });

    if cli_mode {
        pleiades::cli::spawn_stdout_subscriber(&event_bus);
        pleiades::cli::spawn_stdin_repl(user_cmd_tx.clone());
        info!("进入 Orchestrator 主循环 (CLI 模式)");
        core.run().await;
    } else {
        let event_rx = event_bus.Subscribe();
        tokio::task::spawn_blocking(move || {
            TUI_Loop(event_rx, user_cmd_tx);
        });
        info!("进入 Orchestrator 主循环");
        core.run().await;
    }


    info!("Pleiades 已退出");
    Ok(())
}
