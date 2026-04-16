//Presented by KeJi
//Date ： 2026-04-13

#![allow(non_snake_case, non_camel_case_types)]

use candle_core;
use tokio::sync::mpsc;
use tracing::info;
use libp2p;

use pleiades::{
    Ensure_Config, Ensure_Identity, NetworkConfig, Network_Service,
    Control_Loop, Ui_Message, TUI_Loop,
};
use pleiades::control::cli::CLI_Command;
use pleiades::peer_management;

#[tokio::main]
async fn main() {
    // ============================================================
    // 1. 确保配置文件存在并读取
    // ============================================================
    let (config, config_path) = match Ensure_Config() {
        Ok(result) => result,
        Err(e) => {
            eprintln!("[Error] 配置文件读取失败: {}", e);
            std::process::exit(1);
        }
    };
    eprintln!("[Info] 配置文件路径: {}", config_path.display());

    // ============================================================
    // 2. 初始化日志系统（写入文件，不输出到终端，避免干扰 TUI）
    // ============================================================
    let log_level = config
        .Log
        .as_ref()
        .and_then(|l| l.level.as_deref())
        .unwrap_or("info");

    let log_dir = config
        .Log
        .as_ref()
        .and_then(|l| l.log_file_path.as_deref())
        .unwrap_or("Log/");

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    let file_appender = tracing_appender::rolling::daily(log_dir, "pleiades.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    info!("Pleiades 启动中...");

    // ============================================================
    // 3. 创建工作目录
    // ============================================================
    let workspace_dir = std::path::Path::new("Pleiades_Workspace");
    if !workspace_dir.exists() {
        std::fs::create_dir_all(workspace_dir).expect("创建 Pleiades_Workspace 目录失败");
        info!("已创建工作目录: Pleiades_Workspace/");
    }

    // ============================================================
    // 4. 初始化网络层
    // ============================================================
    let network_cfg = {
        let net = config.Network.as_ref();
        NetworkConfig {
            lan_enabled: net.and_then(|n| n.LAN).unwrap_or(true),
            wan_enabled: net.and_then(|n| n.WAN).unwrap_or(false),
            transport_protocol: net
                .and_then(|n| n.Transport_Protocol.clone())
                .unwrap_or_else(|| "TCP".to_string()),
            listen_port: 0,
            bootstrap_peers: Vec::new(),
            cleanup_interval: net.and_then(|n| n.cleanup_interval).unwrap_or(300),
            timeout_interval: net.and_then(|n| n.timeout_interval).unwrap_or(300),
        }
    };

    // 加载或生成节点身份密钥对
    let config_dir = config_path.parent().unwrap_or(std::path::Path::new(".config"));
    let keypair = match Ensure_Identity(config_dir) {
        Ok(kp) => kp,
        Err(e) => {
            eprintln!("[Error] 节点身份初始化失败: {}", e);
            std::process::exit(1);
        }
    };
    info!("节点 PeerId: {}", libp2p::PeerId::from(keypair.public()));

    let (event_tx, event_rx) = mpsc::channel(100);
    
    // ============================================================
    // 5. 创建 PeerManager（任务4要求：集成到 NetworkService）
    // ============================================================
    let (_peer_manager, peer_handle) = peer_management::create_peer_management();
    info!("PeerManager 已创建，准备集成到 NetworkService");

    let (mut network_service, node_handle, inbound_rx) = match Network_Service::Init(network_cfg, keypair, event_tx, peer_handle).await {
        Ok(result) => result,
        Err(e) => {
            eprintln!("[Error] 网络初始化失败: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        "网络节点已初始化, PeerId: {}",
        node_handle.Get_Local_Peer_Id()
    );

    // 启动 Network_Service 事件循环（后台任务）
    tokio::spawn(async move {
        if let Err(e) = network_service.Start().await {
            eprintln!("[Error] 网络服务运行错误: {}", e);
        }
    });

    // ============================================================
    // 6. 读取设备配置，并进行 CUDA fallback 检查
    // ============================================================

    // 读取设备配置，并进行 CUDA fallback 检查
    let config_device = config
        .Runtime
        .as_ref()
        .and_then(|r| r.device.clone())
        .unwrap_or_else(|| "cpu".to_string());

    let device = if config_device.to_lowercase() == "cuda" {
        // 检测 CUDA 是否可用
        match candle_core::Device::new_cuda(0) {
            Ok(_) => {
                info!("CUDA 设备可用，使用 cuda");
                "cuda".to_string()
            }
            Err(e) => {
                eprintln!("[Warn] CUDA 不可用 ({}), 回退到 CPU", e);
                info!("CUDA 不可用 ({}), 回退到 CPU", e);
                "cpu".to_string()
            }
        }
    } else {
        config_device.to_lowercase()
    };
    info!("推理设备: {}", device);

    // ============================================================
    // 7. 创建通信通道并启动 TUI
    // ============================================================
    let (cli_tx, cli_rx) = mpsc::channel::<CLI_Command>(32);
    let (ui_tx, ui_rx) = mpsc::channel::<Ui_Message>(256);

    // 启动 TUI（独立阻塞线程，替代原 CLI_Loop）
    tokio::task::spawn_blocking(move || {
        TUI_Loop(ui_rx, cli_tx);
    });

    info!("TUI 已启动");

    // ============================================================
    // 8. 启动 Control 事件循环（阻塞主线程直到退出）
    // ============================================================
    Control_Loop(cli_rx, inbound_rx, event_rx, node_handle, device, config_path, ui_tx).await;

    info!("Pleiades 已退出");
}
