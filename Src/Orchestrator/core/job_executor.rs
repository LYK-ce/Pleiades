// Presented by KeJi
// Date ： 2026-05-16

//! JobExecutor — 最小 stub。
//!
//! 旧版本的 JobExecutor 负责 Orchestrator_VM 执行循环。
//! 重构后业务逻辑由 Core 直接编排或未来 Lua 脚本处理，JobExecutor 简化为最小占位。

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use crate::orchestrator::job::{JobId, JobKind, JobResult, LifecycleEvent};
use crate::orchestrator::Capabilities;

/// 最小 Job 执行器 stub。
///
/// 当前仅用于 ReceiveFile 场景（Network Stream → Storage 写入）。
/// 其余 Job 类型 (Generic) 的业务由 Core 或未来 Lua 线程处理。
pub struct JobExecutor {
    job_id: JobId,
    _kind: JobKind,
    _capabilities: Arc<Capabilities>,
    cancel: CancellationToken,
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
}

impl JobExecutor {
    pub fn new(
        job_id: JobId,
        kind: JobKind,
        capabilities: Arc<Capabilities>,
        cancel: CancellationToken,
        lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    ) -> Self {
        Self {
            job_id,
            _kind: kind,
            _capabilities: capabilities,
            cancel,
            lifecycle_tx,
        }
    }

    /// 执行入口。
    ///
    /// 当前为 stub：立即发送 Done/Success。
    /// 未来 ReceiveFile Job 在此处实现完整的文件接收逻辑。
    pub async fn run(self) {
        // stub: 立即完成
        let _ = self.cancel; // 保留 cancel 以便未来检查
        let _ = self.lifecycle_tx.send(LifecycleEvent::Done {
            job_id: self.job_id,
            result: JobResult::Success,
        }).await;
    }
}
