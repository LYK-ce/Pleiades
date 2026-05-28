// Presented by KeJi
// Date ： 2026-05-16

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
}

/// 生成唯一 ID
static JOB_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) fn generate_id() -> u64 {
    JOB_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}
