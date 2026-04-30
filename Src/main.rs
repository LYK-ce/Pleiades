//Presented by KeJi
//Date ： 2026-04-30

//! Pleiades 入口点
//!
//! 启动引导流程：
//! 1. 读取配置 + 确保节点身份
//! 2. 创建工作目录 + 初始化 tracing 日志（文件输出）
//! 3. 创建基础组件（EventBus、PeerManager、Storage）
//! 4. 初始化 Network 服务
//! 5. 创建 ML Engine + IO Broker
//! 6. 组装 Capabilities + Orchestrator Core
//! 7. 启动 Network 事件循环 + TUI + Core 主循环
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

use tokio::sync::mpsc;
use tracing::info;

use pleiades::config::{Ensure_Config, Ensure_Identity};
use pleiades::event_bus::EventBus;
use pleiades::peer_management::{create_peer_management, PeerHandle};
use pleiades::network::{NetworkConfig, Network_Service};
use pleiades::storage::StorageManager;
use pleiades::ml_engine::ML_Engine_Service;
use pleiades::llm_io::LLM_IO_Broker;
use pleiades::tensor_io::Tensor_Port_Switch;
use pleiades::orchestrator::Capabilities;
use pleiades::orchestrator::core::Core;
use pleiades::orchestrator::compiler::Compiler;
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
    let (config, config_path) = Ensure_Config()?;

    // 2. 确保节点身份密钥对（持久化到 .config/keypair.bin）
    let keypair = Ensure_Identity(Path::new(CONFIG_DIR))?;

    // 3. 确定工作目录（从 config.Storage.workspace_dir 读取，默认 Pleiades_Workspace）
    let workspace_dir: PathBuf = config.Storage.as_ref()
        .and_then(|s| s.workspace_dir.as_deref())
        .unwrap_or(DEFAULT_WORKSPACE)
        .into();

    // 确保工作目录存在
    std::fs::create_dir_all(&workspace_dir)?;

    // ══════════════════════════════════════════════════════
    // Phase 2: 初始化 tracing 日志（文件输出，避免与 TUI 冲突）
    // ══════════════════════════════════════════════════════

    // 4. 日志目录：{workspace}/Log/（或从 config.Log.log_file_path 读取）
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

    // 使用 tracing-appender 写入日志文件（每日滚动）
    let file_appender = tracing_appender::rolling::daily(&log_dir, "pleiades.log");
    let (non_blocking, _log_guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_writer(non_blocking)
        .init();

    // _log_guard 必须存活到 main 函数结束，否则缓冲区中的日志会丢失

    info!("Pleiades 启动中...");
    info!("工作目录: {}", workspace_dir.display());
    info!("日志目录: {}", log_dir.display());

    // ══════════════════════════════════════════════════════
    // Phase 3: 创建基础组件
    // ══════════════════════════════════════════════════════

    // 5. EventBus — 全局事件总线
    let event_bus = Arc::new(EventBus::New(1024));

    // 6. PeerManagement — 节点管理
    //    PeerHandle 实现了 Clone，内部持有 Arc<PeerManager>，
    //    两个 Capability 实例共享同一个 PeerManager。
    let (peer_manager_arc, peer_capability_for_network) = create_peer_management();
    let peer_capability_for_caps = Box::new(PeerHandle::new(peer_manager_arc));

    // 7. Storage — 存储管理器（工作目录即为 Storage 根目录）
    //    Arc 共享给 ML_Engine_Service 和 Capabilities
    let quota_bytes = config.Storage.as_ref()
        .and_then(|s| s.quota_gb)
        .map(|gb| gb * 1024 * 1024 * 1024)
        .unwrap_or(0);
    let storage = Arc::new(
        StorageManager::New_With_Quota(&workspace_dir, quota_bytes).await?
    );

    info!("Storage 初始化完成: dir={}, quota={}",
        workspace_dir.display(),
        if quota_bytes == 0 { "unlimited".to_string() } else { format!("{}GB", quota_bytes / (1024 * 1024 * 1024)) }
    );

    // ══════════════════════════════════════════════════════
    // Phase 4: 创建服务组件
    // ══════════════════════════════════════════════════════

    // 8. 构建 NetworkConfig
    let net_cfg = {
        let n = config.Network.as_ref();
        NetworkConfig {
            lan_enabled:        n.and_then(|n| n.LAN).unwrap_or(true),
            wan_enabled:        n.and_then(|n| n.WAN).unwrap_or(false),
            transport_protocol: n.and_then(|n| n.Transport_Protocol.clone()).unwrap_or_else(|| "TCP".to_string()),
            listen_port:        0,  // 随机端口
            bootstrap_peers:    Vec::new(),
            cleanup_interval:   n.and_then(|n| n.cleanup_interval).unwrap_or(300),
            timeout_interval:   n.and_then(|n| n.timeout_interval).unwrap_or(300),
            heartbeat_interval: n.and_then(|n| n.heartbeat_interval).unwrap_or(60),
            heartbeat_timeout:  n.and_then(|n| n.heartbeat_timeout).unwrap_or(10),
        }
    };

    // 9. 初始化 Network 服务
    let (mut network_service, _node_handle, inbound_rx, net_capability, net_event_rx)
        = Network_Service::Init(
            net_cfg,
            keypair,
            peer_capability_for_network,
            event_bus.clone(),
        ).await?;

    info!("Network 服务初始化完成");

    // 10. ML Engine — 推理引擎（共享 Storage）
    let ml_engine = ML_Engine_Service::New(storage.clone());

    // 11. LLM_IO_Broker (Arc 共享给 Capabilities 和 TUI) + Tensor_IO_Broker
    let io_broker = Arc::new(LLM_IO_Broker::New());
    let tensor_switch = Arc::new(Tensor_Port_Switch::New());

    // ══════════════════════════════════════════════════════
    // Phase 5: 组装 Orchestrator
    // ══════════════════════════════════════════════════════

    // 12. 组装 Capabilities
    let capabilities = Arc::new(Capabilities {
        storage,
        ml_engine: Box::new(ml_engine),
        network: Box::new(net_capability),
        peer_manager: peer_capability_for_caps,
        event_bus: event_bus.clone(),
        io_broker: io_broker.clone(),
        tensor_switch,
    });

    // 13. 用户命令通道 (TUI → Core)
    let (user_cmd_tx, user_cmd_rx) = mpsc::channel::<UserCommand>(64);

    // 14. 创建 Compiler + Core
    let compiler = Arc::new(Compiler);
    let core = Core::new(
        compiler,
        capabilities,
        config_path,
        user_cmd_rx,
        inbound_rx,
        net_event_rx,
    );

    info!("Orchestrator Core 初始化完成");

    // ══════════════════════════════════════════════════════
    // Phase 6: 启动运行时
    // ══════════════════════════════════════════════════════

    // 15. 启动 Network 事件循环（独立 tokio task）
    tokio::spawn(async move {
        if let Err(e) = network_service.Start().await {
            tracing::error!("Network 事件循环异常退出: {}", e);
        }
    });

    // 16. 启动 TUI（spawn_blocking，因为 ratatui 是同步阻塞 API）
    let event_rx = event_bus.Subscribe();
    let io_broker_for_tui = io_broker.clone();
    tokio::task::spawn_blocking(move || {
        TUI_Loop(event_rx, user_cmd_tx, io_broker_for_tui);
    });

    // 17. Core 主循环（阻塞当前 task 直到 Quit）
    info!("进入 Orchestrator 主循环");
    core.run().await;

    info!("Pleiades 已退出");
    Ok(())
}
