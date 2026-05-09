// Presented by KeJi
// Date ： 2026-05-05

//! B4 分支：生命周期事件处理
//!
//! 处理来自 JobExecutor 的 `LifecycleEvent`，
//! 更新 registry 状态并发布 Bus 事件。

use super::Core;
use crate::orchestrator::job::{LifecycleEvent, JobResult};
use crate::event_bus::Bus_Event;

impl Core {
    /// 处理生命周期事件
    ///
    /// 当 Job 完成时，从 registry 移除并发布完成事件。
    /// 如果 Job 关联了 inference_id，异步清理 Tensor_Port_Switch Pipeline 条目。
    pub(super) fn handle_lifecycle_event(&mut self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::Done { job_id, result } => {
                // 发布 Job 完成事件
                let result_str = match &result {
                    JobResult::Success => "Success".to_string(),
                    JobResult::Cancelled => "Cancelled".to_string(),
                    JobResult::Failed(e) => format!("Failed: {}", e),
                };
                self.capabilities.event_bus.Publish(Bus_Event::Job_Completed {
                    job_id: job_id.0,
                    result: result_str,
                });

                // 先获取 inference_id（在 remove 之前），用于清理 Pipeline 条目
                let inference_id = self.registry.get(&job_id)
                    .and_then(|handle| handle.inference_id);

                // 清理 Tensor_Port_Switch Pipeline 条目（仅 Pipeline 相关 Job）
                if let Some(inf_id) = inference_id {
                    let caps = self.capabilities.clone();
                    // 使用 tokio::spawn 异步清理，避免在同步方法中 .await
                    tokio::spawn(async move {
                        caps.tensor_switch.Deregister_Pipeline(inf_id).await;
                    });
                }

                self.registry.remove(&job_id);  // 同步 HashMap 操作
            }
        }
    }
}
