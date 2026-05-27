//Presented by KeJi
//Date ： 2026-05-17

//! Pleiades 入口点
//!
//! 启动引导流程：
//! 1. 读取配置 + 确保节点身份
//! 2. 创建工作目录 + 初始化 tracing 日志（文件输出）
//! 3. 创建基础组件（EventBus、PeerManager、Storage、SessionManager）
//! 4. 初始化 Network 服务
//! 5. 组装 Capabilities + Orchestrator Core
//! 6. 启动 Network 事件循环 + TUI + Core 主循环
//!
//! ## 目录结构
//! ```text
//! .config/                  ← 配置目录（固定）
//!   config.toml
//!   keypair.bin
//! Pleiades_Workspace/       ← 工作目录（可配置，默认 Pleiades_Workspace）
//!   Log/                    ← 日志文件
//!     pleiades.log.YYYY-MM-DD
//!   (files...)              ← Storage 管理的文件
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::info;

use pleiades::config::{Ensure_Config, Ensure_Identity};
use pleiades::event_bus::EventBus;
use pleiades::peer_management::{create_peer_management, PeerHandle, SupportedModel};
use pleiades::network::{NetworkConfig, Network_Service};
use pleiades::storage::StorageManager;
use pleiades::orchestrator::Capabilities;
use pleiades::orchestrator::core::Core;
use pleiades::orchestrator::command::UserCommand;
use pleiades::tui::TUI_Loop;

/// 配置目录路径（固定）
const CONFIG_DIR: &str = ".config";

/// 默认工作目录名
const DEFAULT_WORKSPACE: &str = "Pleiades_Workspace";

/// 工作目录下的日志子目录
const LOG_SUBDIR: &str = "Log";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ══════════════════════════════════════════════════════
    // Phase 1: 配置 & 身份
    // ══════════════════════════════════════════════════════

    // 1. 读取/创建配置文件
    let (config, _config_path) = Ensure_Config()?;

    // 2. 确保节点身份密钥对（持久化到 .config/keypair.bin）
    let keypair = Ensure_Identity(Path::new(CONFIG_DIR))?;

    // 3. 确定工作目录
    let workspace_dir: PathBuf = config.Storage.as_ref()
        .and_then(|s| s.workspace_dir.as_deref())
        .unwrap_or(DEFAULT_WORKSPACE)
        .into();

    std::fs::create_dir_all(&workspace_dir)?;

    // ══════════════════════════════════════════════════════
    // Phase 2: 初始化 tracing 日志
    // ══════════════════════════════════════════════════════

    let log_dir: PathBuf = config.Log.as_ref()
        .and_then(|l| l.log_file_path.as_deref())
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_dir.join(LOG_SUBDIR));

    std::fs::create_dir_all(&log_dir)?;

    let log_level = config.Log.as_ref()
        .and_then(|l| l.level.as_deref())
        .unwrap_or("info");

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    let file_appender = tracing_appender::rolling::daily(&log_dir, "pleiades.log");
    let (non_blocking, _log_guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_writer(non_blocking)
        .init();

    info!("Pleiades 启动中...");
    info!("工作目录: {}", workspace_dir.display());
    info!("日志目录: {}", log_dir.display());

    // ══════════════════════════════════════════════════════
    // Phase 3: 创建基础组件
    // ══════════════════════════════════════════════════════

    // 5. EventBus
    let event_bus = Arc::new(EventBus::New(1024));

    // 6. PeerManagement
    let local_peer_id = PeerId::from(keypair.public());
    let peer_name = pleiades::config::Get_Peer_Name(&config);
    let (peer_manager_arc, peer_capability_for_network) = create_peer_management(local_peer_id, peer_name);
    let peer_capability_for_core = Box::new(PeerHandle::new(peer_manager_arc.clone()));

    // 7. Storage
    let storage: Arc<dyn pleiades::storage::StorageCapability> = Arc::new(
        StorageManager::New(&workspace_dir).await?
    );

    info!("Storage 初始化完成: dir={}", workspace_dir.display());

    // ══════════════════════════════════════════════════════
    // Phase 4: 创建服务组件
    // ══════════════════════════════════════════════════════

    // 9. 构建 NetworkConfig
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

    // 10. 初始化 Network 服务
    let (mut network_service, _node_handle, inbound_rx, net_capability, net_event_rx)
        = Network_Service::Init(
            net_cfg,
            keypair,
            peer_capability_for_network,
            event_bus.clone(),
        ).await?;

    info!("Network 服务初始化完成");

    // ══════════════════════════════════════════════════════
    // Phase 5: 组装 Orchestrator
    // ══════════════════════════════════════════════════════

    // 11. 组装 Capabilities
    let capabilities = Arc::new(Capabilities {
        storage,
        network: Box::new(net_capability),
        peer_manager: peer_capability_for_core,
        event_bus: event_bus.clone(),
        local_stream_hub: Arc::new(pleiades::orchestrator::local_tensor_stream::LocalStreamHub::new()),
    });

    // 12. 用户命令通道 (TUI → Core)
    let (user_cmd_tx, user_cmd_rx) = mpsc::channel::<UserCommand>(64);

    // 12.5 Prompt 广播通道（TUI 创建，Core 中的 chat task 订阅）
    let (prompt_tx, _prompt_rx) = tokio::sync::broadcast::channel::<String>(16);

    // 13. 创建 Core
    let core = Core::new(
        capabilities.clone(),
        user_cmd_rx,
        inbound_rx,
        net_event_rx,
        prompt_tx.clone(),
    );

    info!("Orchestrator Core 初始化完成");

    // ══════════════════════════════════════════════════════
    // Phase 5.5: 后台刷新 Storage 索引 + 同步到 PeerManager
    // ══════════════════════════════════════════════════════
    {
        let storage = capabilities.storage.clone();
        let peer_manager = peer_manager_arc.clone();
        let event_bus_5_5 = event_bus.clone();
        tokio::spawn(async move {
            if let Ok((added, _removed)) = storage.flush().await {
                info!("启动后台 flush 完成: 新增 {} 个文件", added);
                // 同步 supported_models 到 PeerManager
                if let Ok(entries) = storage.list().await {
                    let models: Vec<_> = entries.iter().filter_map(|e| {
                        Some(SupportedModel {
                            id: e.model_id?,
                            file_name: e.file_name.clone(),
                            layer_bitmap: e.layer_bitmap?,
                        })
                    }).collect();
                    if let Some(local) = peer_manager.get_local_peer().await {
                        peer_manager.update_supported_models(&local.peer_id, models.clone()).await;
                        // 通知 TUI 显示本地节点
                        let models_display: Vec<serde_json::Value> = models.iter().map(|m| {
                            serde_json::json!({"file_name": m.file_name, "layer_range": m.layer_range()})
                        }).collect();
                        event_bus_5_5.Publish(pleiades::event_bus::Bus_Event::State {
                            payload: serde_json::json!({
                                "type": "peer_info_updated",
                                "peer_id": local.peer_id.to_string(),
                                "peer_name": local.name,
                                "is_local": true,
                                "models": models_display,
                                "sessions": serde_json::json!([]),
                            }).to_string(),
                        });
                    }
                }
            }
        });
    }

    // ══════════════════════════════════════════════════════
    // Phase 6: 启动运行时
    // ══════════════════════════════════════════════════════

    // 14. 启动 Network 事件循环
    tokio::spawn(async move {
        if let Err(e) = network_service.Start().await {
            tracing::error!("Network 事件循环异常退出: {}", e);
        }
    });

    // 16. 启动 TUI
    let event_rx = event_bus.Subscribe();
    tokio::task::spawn_blocking(move || {
        TUI_Loop(event_rx, user_cmd_tx, prompt_tx);
    });

    // 17. Core 主循环
    info!("进入 Orchestrator 主循环");
    core.run().await;

    info!("Pleiades 已退出");
    Ok(())
}
