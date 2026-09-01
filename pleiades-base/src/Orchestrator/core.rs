// Presented by KeJi
// Created Date ： 2026-05-16
// Modified Date ： 2026-08-09

mod branch_user;
mod branch_command;
mod branch_stream;
mod branch_lifecycle;
mod job_executor;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use super::job::{JobId, JobKind, LifecycleEvent};
use super::command::UserCommand;
use super::Capabilities;
use crate::network::{Network_Inbound_Event, InboundRequest};
use crate::vm::registry::ProgramRegistry;

const LIFECYCLE_CHANNEL_BUFFER: usize = 64;

// ============================================================
// Portal 错误类型
// ============================================================

#[derive(Debug, thiserror::Error)]
pub enum PortalError {
    #[error("路由错误: {0}")]
    Routing(String),
    #[error("内部错误: {0}")]
    Internal(String),
}

// ============================================================
// Job 句柄
// ============================================================

/// Core 内部使用的 Job 句柄
struct JobHandle {
    kind: JobKind,
    cancel: CancellationToken,
    inference_id: Option<u64>,
}

// ============================================================
// Core
// ============================================================

/// Orchestrator Core 结构体。
///
/// ## select! 分支总览
/// | # | 通道 | 来源 | 处理方法 |
/// |---|------|------|---------|
/// | B1 | `user_cmd_rx` | TUI / CLI | `route_user()` |
/// | B2 | `inbound_rx` | Network Request-Response | `route_inbound()` |
/// | B3 | `network_inbound_rx` | Network Stream | `route_stream()` |
/// | B4 | `lifecycle_rx` | JobExecutor | `route_lifecycle()` |
pub struct Core {
    // --- 内核状态 ---
    registry: HashMap<JobId, JobHandle>,
    shutting_down: bool,

    // --- 组件能力 ---
    capabilities: Arc<Capabilities>,

    // --- B1: 用户命令 ---
    user_cmd_rx: mpsc::Receiver<UserCommand>,

    // --- B2: Request-Response 入站 ---
    inbound_rx: mpsc::Receiver<InboundRequest>,

    // --- B3: Stream 入站 ---
    network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,

    // --- B4: 生命周期 ---
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    lifecycle_rx: mpsc::Receiver<LifecycleEvent>,

    // --- Lua 脚本引擎 ---
    program_registry: ProgramRegistry,

    // --- Session Manager ---
    pub(crate) session_mgr: std::sync::Arc<std::sync::Mutex<crate::session::SessionManager>>,


    // --- 偏好设置 ---
    device_preference: String,
}

impl Core {
    /// 创建新的 Core 实例。
    pub fn new(
        capabilities: Arc<Capabilities>,
        user_cmd_rx: mpsc::Receiver<UserCommand>,
        inbound_rx: mpsc::Receiver<InboundRequest>,
        network_inbound_rx: mpsc::Receiver<Network_Inbound_Event>,
    ) -> Self {
        let (lifecycle_tx, lifecycle_rx) = mpsc::channel(LIFECYCLE_CHANNEL_BUFFER);
        let program_registry = ProgramRegistry::new()
            .unwrap_or_else(|e| {
                tracing::warn!("ProgramRegistry 初始化失败: {}，使用空注册表", e);
                ProgramRegistry::default()
            });
        let session_mgr = crate::session::SessionManager::new(
            4, capabilities.local_stream_hub.clone(), capabilities.event_bus.clone(),
            capabilities.storage.clone(),
        );
        Core {
            registry: HashMap::new(),
            shutting_down: false,
            capabilities,
            user_cmd_rx,
            inbound_rx,
            network_inbound_rx,
            lifecycle_tx,
            lifecycle_rx,
            program_registry,
            session_mgr,
            device_preference: String::new(),
        }
    }

    /// 主事件循环。
    ///
    /// 5 个 select! 分支，每轮只处理一个事件。
    /// B5 为 shutdown 超时兜底，仅在 shutting_down 时激活。
    pub async fn run(mut self) {
        const SHUTDOWN_GRACE_SECS: u64 = 30;
        loop {
            if self.shutting_down && self.registry.is_empty() {
                break;
            }
            tokio::select! {
                // B1: 用户命令
                Some(cmd) = self.user_cmd_rx.recv(), if !self.shutting_down => {
                    self.route_user(cmd).await;
                }
                // B2: Request-Response 入站请求
                Some(req) = self.inbound_rx.recv(), if !self.shutting_down => {
                    self.route_inbound(req).await;
                }
                // B3: Stream 入站事件
                Some(event) = self.network_inbound_rx.recv(), if !self.shutting_down => {
                    self.route_stream(event).await;
                }
                // B4: 生命周期事件（始终活跃，shutdown 期间仍处理完成事件）
                Some(event) = self.lifecycle_rx.recv() => {
                    self.route_lifecycle(event);
                }
                // B5: shutdown 超时兜底 — 防止 Job 卡死导致进程永不退出
                _ = tokio::time::sleep(std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS)),
                    if self.shutting_down => {
                    tracing::warn!(
                        "shutdown grace period ({SHUTDOWN_GRACE_SECS}s) expired, {} jobs still running, forcing exit",
                        self.registry.len()
                    );
                    break;
                }
            }
        }
    }

    // ─── Job 管理 ───────────────────────────────────────────

    fn register_job(&mut self, job_id: JobId, kind: JobKind) {
        self.registry.insert(job_id, JobHandle {
            kind,
            cancel: CancellationToken::new(),
            inference_id: None,
        });
    }

    fn cancel_job(&mut self, job_id: JobId) {
        if let Some(handle) = self.registry.get(&job_id) {
            handle.cancel.cancel();
        }
    }

    fn shutdown(&mut self) {
        self.shutting_down = true;
        for handle in self.registry.values() {
            handle.cancel.cancel();
        }
    }

    // ─── Flush 管理 ────────────────────────────────────────

    /// 执行 flush + 广播本地节点信息（GossipSub peer-info/models/sessions topic），返回结果文本
    pub async fn do_flush(caps: &Capabilities) -> String {
        match caps.storage.Flush().await {
            Ok((added, removed)) => {
                // 模型文件变更 → 业务层构造 payload + publish_gossipsub 广播 3 topic
                if let Ok(local) = caps.peer_manager.Get_Local_Peer().await {
                    let _ = caps.network.publish_gossipsub(
                        crate::network::TOPIC_PEER_INFO,
                        crate::peer_management::PeerManager::Build_Peer_Info_Payload(&local),
                    ).await;
                    let _ = caps.network.publish_gossipsub(
                        crate::network::TOPIC_MODELS,
                        crate::peer_management::PeerManager::Build_Models_Payload(&local),
                    ).await;
                    let _ = caps.network.publish_gossipsub(
                        crate::network::TOPIC_SESSIONS,
                        crate::peer_management::PeerManager::Build_Sessions_Payload(&local),
                    ).await;

                    // 本地 TUI 刷新：models 事件已由 Storage::Flush() 内部 Sync_Models_To_Peer_Manager 发；
                    // name/sessions 事件原由 publish_* 内部发，此处显式补发（gossipsub 不回流本机）
                    let peer_id_str = local.peer_id.to_string();
                    caps.event_bus.Publish(crate::event_bus::Bus_Event::State {
                        payload: serde_json::json!({
                            "type": "peer_info_updated",
                            "peer_id": peer_id_str.clone(),
                            "peer_name": local.name,
                            "node_type": local.node_type,
                            "is_local": true,
                            "models": [],
                            "sessions": [],
                        }).to_string(),
                    });
                    let sessions_display: Vec<serde_json::Value> = local.sessions.iter().map(|s| {
                        serde_json::json!({
                            "session_id": s.session_id,
                            "model_id": s.model_id,
                            "occupied_slots": s.occupied_slots,
                            "total_slots": s.total_slots,
                        })
                    }).collect();
                    caps.event_bus.Publish(crate::event_bus::Bus_Event::State {
                        payload: serde_json::json!({
                            "type": "peer_info_updated",
                            "peer_id": peer_id_str,
                            "peer_name": "",
                            "is_local": true,
                            "models": [],
                            "sessions": sessions_display,
                        }).to_string(),
                    });
                }
                format!("flush 完成: 新增 {} 个, 移除 {} 个", added, removed)
            }
            Err(e) => format!("flush 失败: {}", e),
        }
    }

    /// 后台触发初始 flush（fire-and-forget）
    ///
    /// 必须在 EventBus 订阅者就位后调用：本机 peer_info_updated 是一次性事件，
    /// broadcast 无历史重放，先订阅再 flush 才能保证 TUI/CLI 收到（2026-08-09 修复）。
    pub fn spawn_initial_flush(&self) {
        let caps = self.capabilities.clone();
        tokio::spawn(async move {
            let text = Self::do_flush(&caps).await;
            tracing::info!("初始 {}", text);
            caps.event_bus.Publish(crate::event_bus::Bus_Event::Output {
                payload: cmd_output(text, true),
            });
        });
    }
}

fn cmd_output(text: impl Into<String>, completed: bool) -> String {
    serde_json::json!({"type":"cmd_result","text":text.into(),"completed":completed}).to_string()
}

/// 生成唯一 ID
static JOB_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) fn generate_id() -> u64 {
    JOB_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}
