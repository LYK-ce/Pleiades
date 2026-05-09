//Presented by KeJi
//Date ： 2026-05-09

//! 推理生命周期 handler。
//!
//! CreateSession / ShutdownSession / RunProgram / AnalyzeModel / SplitModel

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use crate::vm_base::{StepResult, SlotId};
use crate::ml_engine::capability::ML_Session_Config;
use crate::ml_engine::ml_thread_engine_instruction::Pipeline_Params;
use crate::orchestrator::compiler::Compiler;

use super::engine::Orchestrator_VM;

impl Orchestrator_VM {
    pub async fn handle_create_session(
        &mut self,
        model: SlotId,
        device: SlotId,
        start: SlotId,
        end: SlotId,
        io: SlotId,
        tensor_io: Option<SlotId>,
        result: SlotId,
    ) -> StepResult {
        let model_file_id = match self.vm.slots.get_string(model) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("CreateSession: model slot error: {}", e)),
        };

        let device_str = match self.vm.slots.get_string(device) {
            Ok(s) => s.clone(),
            Err(_) => "cpu".to_string(),
        };

        let layer_start = match self.vm.slots.get_u64(start) {
            Ok(v) => v as usize,
            Err(_) => 0,
        };

        let layer_end = match self.vm.slots.get_u64(end) {
            Ok(v) => v as usize,
            Err(_) => usize::MAX,
        };

        let io_handle = match self.slots.take_io_handle(io) {
            Some(h) => h,
            None => return StepResult::Abort(format!("CreateSession: io slot {} not found", io.0)),
        };

        let tensor_io_handle = match tensor_io {
            Some(slot) => match self.slots.take_tensor_io(slot) {
                Some(h) => Some(h),
                None => return StepResult::Abort(format!("CreateSession: tensor_io slot {} not found", slot.0)),
            },
            None => None,
        };

        let session_id = format!("job-{}", self.job_id.0);
        let config = ML_Session_Config {
            session_id: session_id.clone(),
            model_file_id,
            layer_start,
            layer_end,
            device: device_str,
            tensor_io: tensor_io_handle,
        };

        match self.capabilities.ml_engine.Create_Session(config, io_handle).await {
            Ok(_model_info) => {
                self.vm.slots.set(result, crate::vm_base::SlotValue::String(session_id));
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("CreateSession failed: {}", e)),
        }
    }

    pub async fn handle_shutdown_session(&mut self, session: SlotId) -> StepResult {
        let session_id = match self.vm.slots.get_string(session) {
            Ok(s) => s.clone(),
            Err(_) => return StepResult::Continue,
        };

        let _ = self.capabilities.ml_engine.Shutdown_Session(&session_id).await;
        StepResult::Continue
    }

    pub async fn handle_run_program(&mut self, session: SlotId, result: SlotId) -> StepResult {
        let session_id = match self.vm.slots.get_string(session) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("RunProgram: session slot error: {}", e)),
        };

        let params = Pipeline_Params::default();
        let mode = self.vm.slots.get_string(super::to_vm_slot(crate::orchestrator::compiler::SLOT_ML_PROGRAM_MODE))
            .map(|s| s.clone())
            .unwrap_or_else(|_| "run".to_string());
        let program = match mode.as_str() {
            "relay" => Compiler::build_relay_ml_program(),
            "coordinator" => Compiler::build_coordinator_ml_program(&params),
            _ => Compiler::build_run_ml_program(&params),
        };

        let cancel_flag = Arc::new(AtomicBool::new(false));

        match self.capabilities.ml_engine.Run_Program(
            &session_id,
            program,
            params,
            cancel_flag,
        ).await {
            Ok(_pipeline_result) => {
                self.vm.slots.set(result, crate::vm_base::SlotValue::String("done".to_string()));
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("RunProgram failed: {}", e)),
        }
    }

    pub async fn handle_analyze_model(&mut self, model: SlotId, result: SlotId) -> StepResult {
        let model_file_id = match self.vm.slots.get_string(model) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("AnalyzeModel: model slot error: {}", e)),
        };

        match self.capabilities.ml_engine.Analyze_Model(&model_file_id).await {
            Ok(model_info) => {
                self.slots.set(result, crate::orchestrator::orchestrator_vm::slots::OrchestratorSlotValue::ModelInfo(model_info));
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("AnalyzeModel failed: {}", e)),
        }
    }

    pub async fn handle_split_model(
        &mut self,
        source: SlotId,
        start: SlotId,
        end: SlotId,
        output: SlotId,
    ) -> StepResult {
        let source_file_id = match self.vm.slots.get_string(source) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SplitModel: source slot error: {}", e)),
        };

        let layer_start = match self.vm.slots.get_u64(start) {
            Ok(v) => v as usize,
            Err(e) => return StepResult::Abort(format!("SplitModel: start slot error: {}", e)),
        };

        let layer_end = match self.vm.slots.get_u64(end) {
            Ok(v) => v as usize,
            Err(e) => return StepResult::Abort(format!("SplitModel: end slot error: {}", e)),
        };

        let output_file_id = match self.vm.slots.get_string(output) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SplitModel: output slot error: {}", e)),
        };

        match self.capabilities.ml_engine.Split_Model(
            &source_file_id,
            layer_start,
            layer_end,
            &output_file_id,
        ).await {
            Ok(()) => StepResult::Continue,
            Err(e) => StepResult::Abort(format!("SplitModel failed: {}", e)),
        }
    }
}
