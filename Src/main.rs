//Presented by KeJi
//Date ： 2026-04-07

#![allow(non_snake_case, non_camel_case_types)]

use std::path::Path;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber;

use pleiades::{
    Read_Config, NetworkConfig, Node, ML_Service_Handle,
    Control_Loop,
};
use pleiades::control::cli::{CLI_Command, CLI_Loop};

#[tokio::main]
async fn main() {
    // ============================================================
    // 1. 读取配置文件
    // ============================================================
    let config_path = Path::new("config.toml");
    let config = match Read_Config(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[Error] 配置文件读取失败: {}", e);
            std::process::exit(1);
        }
    };

    // ============================================================
    // 2. 初始化日志系统
    // ============================================================
    let log_level = config
        .Log
        .as_ref()
        .and_then(|l| l.level.as_deref())
        .unwrap_or("info");

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
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
        }
    };

    let (event_tx, event_rx) = mpsc::channel(100);
    let (mut node, node_handle, inbound_rx) = match Node::Init(network_cfg, event_tx).await {
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
    println!(
        "[Pleiades] 节点启动, PeerId: {}",
        node_handle.Get_Local_Peer_Id()
    );

    // 启动 Node 事件循环（后台任务）
    tokio::spawn(async move {
        if let Err(e) = node.Start().await {
            eprintln!("[Error] 网络节点运行错误: {}", e);
        }
    });

    // ============================================================
    // 5. 初始化 ML Service
    // ============================================================
    let ml_service = ML_Service_Handle::Init();
    info!("ML Service 已初始化");

    // 读取设备配置
    let device = config
        .Runtime
        .as_ref()
        .and_then(|r| r.device.clone())
        .unwrap_or_else(|| "cpu".to_string());
    info!("推理设备: {}", device);

    // ============================================================
    // 6. 启动 CLI（独立阻塞线程）
    // ============================================================
    let (cli_tx, cli_rx) = mpsc::channel::<CLI_Command>(32);

    tokio::task::spawn_blocking(move || {
        CLI_Loop(cli_tx);
    });

    info!("CLI 已启动");

    // ============================================================
    // 7. 启动 Control 事件循环（阻塞主线程直到退出）
    // ============================================================
    Control_Loop(cli_rx, inbound_rx, event_rx, ml_service, node_handle, device).await;

    info!("Pleiades 已退出");
}

