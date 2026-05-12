//Presented by KeJi
//Date ： 2026-05-09

//! Pipeline 规划 handler。
//!
//! PlanPipeline

use crate::vm_base::{StepResult, SlotId};
use crate::scheduler::{Scheduler_Input, Scheduler_Strategy};
use super::engine::Orchestrator_VM;
use super::slots::OrchestratorSlotValue;
use crate::orchestrator::program_selector::SLOT_MODEL;

impl Orchestrator_VM {
    pub async fn handle_plan_pipeline(
        &mut self,
        model_info_slot: SlotId,
        inference_id_slot: SlotId,
        strategy_slot: SlotId,
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

        let model_id = self.vm.slots.get_string(SLOT_MODEL)
            .unwrap_or(&"unknown".to_string())
            .clone();

        let coordinator_info = match self.capabilities.peer_manager.Get_Peer(&local_peer_id).await {
            Ok(info) => info,
            Err(e) => return StepResult::Abort(format!("PlanPipeline: get coordinator info error: {}", e)),
        };

        let strategy = self.vm.slots.get_string(strategy_slot)
            .map(|s| match s.as_str() {
                "weighted" => Scheduler_Strategy::Weighted,
                _ => Scheduler_Strategy::Uniform,
            })
            .unwrap_or(Scheduler_Strategy::Uniform);

        let input = Scheduler_Input {
            strategy,
            model_info,
            model_id,
            available_peers,
            coordinator_info,
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
