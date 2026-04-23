// Presented by KeJi
// Date ： 2026-04-23

use crate::orchestrator::slot::SlotId;
use super::task_engine::StepResult;

impl super::TaskEngine {
    /// 处理 CreateSession 指令：从 `model` 槽位读取模型路径，从 `device` 槽位 **take** 设备租约，
    /// 从 `io` 槽位 **take** IoHandle，创建 ML Session，句柄写入 `result` 槽位
    pub(super) fn handle_create_session(&mut self, model: SlotId, device: SlotId, io: SlotId, result: SlotId) -> StepResult {
        // 占位符：实际应读取 model 槽位的路径，take device 槽位的租约，take io 槽位的 IoHandle，
        // 调用 Inference Capability 创建 Session，结果写入 result 槽位
        StepResult::Continue
    }

    /// 处理 ShutdownSession 指令：从 `session` 槽位 **take** Session 句柄，调用 Inference Capability 关闭
    pub(super) fn handle_shutdown_session(&mut self, session: SlotId) -> StepResult {
        // 占位符：实际应 take session 槽位的句柄，调用 Inference Capability 关闭
        StepResult::Continue
    }
}