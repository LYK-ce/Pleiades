// Presented by KeJi
// Date ： 2026-05-16

//! B4: 生命周期事件路由。
//!
//! 处理来自 JobExecutor 的 LifecycleEvent（仅 Done）。

use super::Core;
use super::job::{JobResult, LifecycleEvent};
use crate::event_bus::Bus_Event;

impl Core {
    /// 路由生命周期事件 (B4)。
    ///
    /// Done 事件处理完成后从 registry 移除对应 Job。
    pub fn route_lifecycle(&mut self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::Done { job_id, result } => {
                let job_id_val = job_id.0;
                self.capabilities.event_bus.Publish(Bus_Event::Job_Completed {
                    job_id: job_id_val,
                    result: format!("{:?}", result),
                });
                self.registry.remove(&job_id);
            }
        }
    }
}
