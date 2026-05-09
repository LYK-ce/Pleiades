//Presented by KeJi
//Date ： 2026-05-09

//! Pipeline 规划 handler。
//!
//! PlanPipeline

use crate::vm_base::{StepResult, SlotId};
use crate::scheduler::Scheduler_Input;
use super::engine::Orchestrator_VM;
use super::slots::OrchestratorSlotValue;

impl Orchestrator_VM {
    pub async fn handle_plan_pipeline(
        &mut self,
        model_info_slot: SlotId,
        inference_id_slot: SlotId,
        result: SlotId,
    ) -> StepResult {
        let model_info = match self.slots.get_model_info(model_info_slot) {
            Some(info) => info.clone(),
            None => return StepResult::Abort(format!("PlanPipeline: model_info slot {} not found", model_info_slot.0)),
        };

        let inference_id = match self.vm.slots.get_u64(inference_id_slot) {
            Ok(id) => id,
            Err(e) => return StepResult::Abort(format!("PlanPipeline: inference_id error: {}", e)),
        };

        let available_peers = match self.capabilities.peer_manager.Get_Idle_Peers().await {
            Ok(peers) => peers,
            Err(e) => return StepResult::Abort(format!("PlanPipeline: get idle peers error: {}", e)),
        };

        let local_peer_id = self.capabilities.network.get_local_peer_id();

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

        self.slots.set(result, OrchestratorSlotValue::PipelinePlan(plan));
        StepResult::Continue
    }
}
