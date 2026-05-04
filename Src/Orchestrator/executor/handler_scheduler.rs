//Presented by KeJi
//Date ： 2026-05-04

use crate::orchestrator::slot::{SlotId, SlotValue};
use crate::scheduler::Scheduler_Input;
use super::task_engine::StepResult;

impl super::TaskEngine {
    /// 处理 PlanPipeline 指令：规划 Pipeline 拓扑
    ///
    /// 1. 从 `model_info` 槽位读取 Model_Info
    /// 2. 从 `inference_id` 槽位读取 inference_id
    /// 3. 查询 PeerManager 获取可用节点快照
    /// 4. 获取本地 PeerId
    /// 5. 调用 Scheduler Capability 计算拓扑
    /// 6. 将 Pipeline_Plan 写入 `result` 槽位
    pub(super) async fn handle_plan_pipeline(
        &mut self,
        model_info_slot: SlotId,
        inference_id_slot: SlotId,
        result: SlotId,
    ) -> StepResult {
        // 1. 从槽位读取 Model_Info（clone，因为需要所有权传入 Scheduler）
        let model_info = match self.slots.get_model_info(model_info_slot) {
            Ok(info) => info.clone(),
            Err(e) => return StepResult::Abort(format!("PlanPipeline: model_info error: {}", e)),
        };

        // 2. 从槽位读取 inference_id
        let inference_id = match self.slots.get_u64(inference_id_slot) {
            Ok(id) => id,
            Err(e) => return StepResult::Abort(format!("PlanPipeline: inference_id error: {}", e)),
        };

        // 3. 查询可用节点（空闲 + 已连接）
        let available_peers = match self.capabilities.peer_manager.Get_Idle_Peers().await {
            Ok(peers) => peers,
            Err(e) => return StepResult::Abort(format!("PlanPipeline: get idle peers error: {}", e)),
        };

        // 4. 获取本地 PeerId
        let local_peer_id = self.capabilities.network.get_local_peer_id();

        // 5. 调用 Scheduler Capability
        let input = Scheduler_Input {
            model_info,
            available_peers,
            local_peer_id,
            inference_id,
        };
        let plan = match self.capabilities.scheduler.Plan_Pipeline(input).await {
            Ok(p) => p,
            Err(e) => return StepResult::Abort(format!("PlanPipeline: scheduler error: {}", e)),
        };

        // 6. 写入结果槽位
        self.slots.set(result, SlotValue::PipelinePlan(plan));
        StepResult::Continue
    }
}
