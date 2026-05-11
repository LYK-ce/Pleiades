//Presented by KeJi
//Date ： 2026-05-09

//! 推理生命周期 handler。
//!
//! CreateSession / ShutdownSession / RunProgram / AnalyzeModel / SplitModel

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::collections::HashMap;
use crate::vm_base::{StepResult, SlotId};
use crate::ml_engine::capability::ML_Session_Config;
use crate::ml_engine::pipeline::Pipeline_Params;
use crate::orchestrator::command::{NetworkProtocol, Serialize_Network_Command};
use crate::orchestrator::program_selector::{
    ProgramSelector, SLOT_HIDDEN_DIM, SLOT_MODEL, SLOT_DEVICE,
};

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
            Ok(model_info) => {
                self.vm.slots.set(result, crate::vm_base::SlotValue::String(session_id));
                self.vm.slots.set(
                    SLOT_HIDDEN_DIM,
                    crate::vm_base::SlotValue::U64(model_info.embedding_length as u64),
                );
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
        let mode = self.vm.slots.get_string(crate::orchestrator::program_selector::SLOT_ML_PROGRAM_MODE)
            .map(|s| s.clone())
            .unwrap_or_else(|_| "run".to_string());
        let program = match mode.as_str() {
            "relay" => match ProgramSelector::load_ml_program_vm("relay", &params, &HashMap::new()) {
                Ok(p) => p,
                Err(e) => return StepResult::Abort(format!("load relay ml program failed: {}", e)),
            },
            "coordinator" => match ProgramSelector::load_ml_program_vm("coordinator", &params, &HashMap::new()) {
                Ok(p) => p,
                Err(e) => return StepResult::Abort(format!("load coordinator ml program failed: {}", e)),
            },
            _ => match ProgramSelector::load_ml_program_vm("run", &params, &HashMap::new()) {
                Ok(p) => p,
                Err(e) => return StepResult::Abort(format!("load run ml program failed: {}", e)),
            },
        };

        let cancel_flag = Arc::new(AtomicBool::new(false));

        match self.capabilities.ml_engine.Run_Program_VM(
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

    pub async fn handle_profile(&mut self, session: SlotId, result: SlotId) -> StepResult {
        let hidden_dim = match self.vm.slots.get_u64(SLOT_HIDDEN_DIM) {
            Ok(dim) => dim as usize,
            Err(e) => return StepResult::Abort(format!("Profile: hidden_dim 未设置: {}", e)),
        };

        let params = Pipeline_Params::default();
        let mut vars = HashMap::new();
        vars.insert("hidden_dim".to_string(), hidden_dim.to_string());
        let program = match ProgramSelector::load_ml_program_vm("profile", &params, &vars) {
            Ok(p) => p,
            Err(e) => return StepResult::Abort(format!("Profile: 加载程序失败: {}", e)),
        };

        let session_id = match self.vm.slots.get_string(session) {
            Ok(id) => id.clone(),
            Err(e) => return StepResult::Abort(format!("Profile: session_id 为空: {}", e)),
        };

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let local_duration = match self.capabilities.ml_engine.Run_Program_VM(
            &session_id,
            program,
            params,
            cancel_flag,
        ).await {
            Ok(pipeline_result) => {
                let d = pipeline_result.duration;
                self.vm.slots.set(
                    result,
                    crate::vm_base::SlotValue::F64(d.as_secs_f64()),
                );
                d
            }
            Err(e) => return StepResult::Abort(format!("Profile: 执行失败: {:?}", e)),
        };

        // 向所有 peer 广播 Profile_Request
        let model_id = self.vm.slots.get_string(SLOT_MODEL)
            .unwrap_or(&"unknown".to_string()).clone();
        let device = self.vm.slots.get_string(SLOT_DEVICE)
            .unwrap_or(&"cpu".to_string()).clone();
        let cmd = NetworkProtocol::Profile_Request {
            model_id: model_id.clone(),
            layer_count: 5,
            device: device.clone(),
        };
        let payload = Serialize_Network_Command(&cmd);

        match self.capabilities.peer_manager.List_Peers().await {
            Ok(peers) => {
                for peer in &peers {
                    match self.capabilities.network.send_data(
                        peer.peer_id,
                        crate::network::DataType::Command,
                        payload.clone(),
                    ).await {
                        Ok(response) => {
                            let text = String::from_utf8_lossy(&response.payload);
                            if text.starts_with("OK|") {
                                if let Ok(micros) = text[3..].trim().parse::<u64>() {
                                    let d = std::time::Duration::from_micros(micros);
                                    if let Ok(mut info) = self.capabilities.peer_manager.Get_Peer(&peer.peer_id).await {
                                        let mut cap = info.capability.take().unwrap_or_default();
                                        cap.set_layer_time(model_id.clone(), d);
                                        let _ = self.capabilities.peer_manager.Update_Capability(&peer.peer_id, Some(cap)).await;
                                    }
                                    tracing::info!("Profile: {} → {:?}", peer.peer_id, d);
                                }
                            } else {
                                tracing::warn!("Profile: {} replied FAIL: {}", peer.peer_id, text);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Profile: send to {} failed: {}", peer.peer_id, e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Profile: 获取 peer 列表失败: {}", e);
            }
        }

        StepResult::Continue
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
