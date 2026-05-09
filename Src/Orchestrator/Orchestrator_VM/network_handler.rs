//Presented by KeJi
//Date ： 2026-05-09

//! 网络 + Pipeline 编排 handler。
//!
//! SendFile / ReceiveFile / EstablishStreams / JoinWorkers

use libp2p::PeerId;
use crate::vm_base::{StepResult, SlotId};
use crate::orchestrator::command::{NetworkProtocol, Serialize_Network_Command};
use crate::orchestrator::compiler::{SLOT_MODEL, SLOT_TENSOR_IO, SLOT_LAYER_START, SLOT_LAYER_END};
use crate::network::stream_protocol::{Write_File_Stream_Header, Read_File_Stream_Ack};
use crate::network::tensor_stream_protocol::Write_Tensor_Stream_Handshake;
use crate::network::DataType;
use crate::storage::{StorageCapability, ChecksumAlgorithm};
use crate::scheduler::Pipeline_Plan;

use super::engine::Orchestrator_VM;
use super::slots::OrchestratorSlotValue;
use super::to_vm_slot;

impl Orchestrator_VM {
    pub async fn handle_send_file(&mut self, peer: SlotId, file: SlotId) -> StepResult {
        let peer_id_str = match self.vm.slots.get_string(peer) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SendFile: peer slot error: {}", e)),
        };
        let peer_id: PeerId = match peer_id_str.parse() {
            Ok(id) => id,
            Err(e) => return StepResult::Abort(format!("SendFile: invalid PeerId '{}': {}", peer_id_str, e)),
        };

        let file_id = match self.vm.slots.get_string(file) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SendFile: file slot error: {}", e)),
        };

        let (path, read_guard) = match self.capabilities.storage.acquire_read(&file_id).await {
            Ok(r) => r,
            Err(e) => return StepResult::Abort(format!("SendFile: acquire_read failed: {}", e)),
        };

        let file_size = match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) => return StepResult::Abort(format!("SendFile: metadata failed: {}", e)),
        };

        let checksum = match self.capabilities.storage.checksum(&file_id, Some(ChecksumAlgorithm::Blake3)).await {
            Ok(c) => c,
            Err(e) => return StepResult::Abort(format!("SendFile: checksum failed: {}", e)),
        };

        let mut stream = match self.capabilities.network.open_file_stream(peer_id).await {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("SendFile: open_file_stream failed: {}", e)),
        };

        if let Err(e) = Write_File_Stream_Header(&mut stream, &file_id, file_size, &checksum).await {
            return StepResult::Abort(format!("SendFile: write header failed: {}", e));
        }

        match Read_File_Stream_Ack(&mut stream).await {
            Ok(true) => {}
            Ok(false) => return StepResult::Abort("SendFile: peer rejected file transfer".to_string()),
            Err(e) => return StepResult::Abort(format!("SendFile: read ACK failed: {}", e)),
        }

        if let Err(e) = self.capabilities.network.send_file_data(&mut stream, &path).await {
            return StepResult::Abort(format!("SendFile: send_file_data failed: {}", e));
        }

        drop(read_guard);
        StepResult::Continue
    }

    pub async fn handle_receive_file(
        &mut self,
        stream_slot: SlotId,
        file_name_slot: SlotId,
        file_size_slot: SlotId,
        checksum_slot: SlotId,
        result: SlotId,
    ) -> StepResult {
        let mut stream = match self.slots.take_stream(stream_slot) {
            Some(s) => s,
            None => return StepResult::Abort(format!("ReceiveFile: stream slot {} not found", stream_slot.0)),
        };

        let file_name = match self.vm.slots.get_string(file_name_slot) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("ReceiveFile: file_name slot error: {}", e)),
        };

        let file_size = match self.vm.slots.get_u64(file_size_slot) {
            Ok(v) => v,
            Err(e) => return StepResult::Abort(format!("ReceiveFile: file_size slot error: {}", e)),
        };

        let expected_checksum = match self.vm.slots.get_string(checksum_slot) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("ReceiveFile: checksum slot error: {}", e)),
        };

        let (dest_path, write_guard) = match self.capabilities.storage.acquire_write(&file_name).await {
            Ok(r) => r,
            Err(e) => return StepResult::Abort(format!("ReceiveFile: acquire_write failed: {}", e)),
        };

        if let Err(e) = self.capabilities.network.receive_file_data(&mut stream, &dest_path, file_size).await {
            return StepResult::Abort(format!("ReceiveFile: receive_file_data failed: {}", e));
        }

        drop(write_guard);

        let actual_checksum = match self.capabilities.storage.checksum(&file_name, Some(ChecksumAlgorithm::Blake3)).await {
            Ok(c) => c,
            Err(e) => return StepResult::Abort(format!("ReceiveFile: checksum calculation failed: {}", e)),
        };

        if actual_checksum == expected_checksum {
            self.vm.slots.set(result, crate::vm_base::SlotValue::String(file_name));
            StepResult::Continue
        } else {
            let _ = self.capabilities.storage.remove(&file_name).await;
            StepResult::Abort(format!(
                "ReceiveFile: checksum mismatch (expected: {}, actual: {})",
                expected_checksum, actual_checksum
            ))
        }
    }

    pub async fn handle_establish_streams(&mut self, plan_slot: SlotId, result: SlotId) -> StepResult {
        let plan: Pipeline_Plan = match self.slots.take_pipeline_plan(plan_slot) {
            Some(p) => p.clone(),
            None => return StepResult::Abort(format!("EstablishStreams: plan slot {} not found", plan_slot.0)),
        };

        // 放回，供 JoinWorkers 读取
        self.slots.set(plan_slot, OrchestratorSlotValue::PipelinePlan(plan.clone()));

        let inference_id = plan.inference_id;

        if plan.workers.is_empty() {
            self.vm.slots.set(to_vm_slot(SLOT_LAYER_START), crate::vm_base::SlotValue::U64(plan.coord_layer_start as u64));
            self.vm.slots.set(to_vm_slot(SLOT_LAYER_END), crate::vm_base::SlotValue::U64(plan.coord_layer_end as u64));
            self.vm.slots.set(result, crate::vm_base::SlotValue::String("ok_single_node".to_string()));
            return StepResult::Continue;
        }

        let outbound_target = match plan.coord_outbound_target {
            Some(target) => target,
            None => return StepResult::Abort("EstablishStreams: coord_outbound_target is None but workers exist".to_string()),
        };

        let mut outbound_stream = match self.capabilities.network.open_tensor_stream(outbound_target).await {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("EstablishStreams: open_tensor_stream failed: {}", e)),
        };

        if let Err(e) = Write_Tensor_Stream_Handshake(&mut outbound_stream, inference_id).await {
            return StepResult::Abort(format!("EstablishStreams: handshake write failed: {}", e));
        }

        self.capabilities.tensor_switch.Register_Outbound(inference_id, outbound_stream).await;

        for worker in &plan.workers {
            let cmd = NetworkProtocol::Establish_Tensor_Stream {
                inference_id,
                target_peer_id: worker.outbound_target.to_string(),
            };
            let payload = Serialize_Network_Command(&cmd);

            let response = match self.capabilities.network.send_data(
                worker.peer_id,
                DataType::Command,
                payload,
            ).await {
                Ok(r) => r,
                Err(e) => return StepResult::Abort(format!(
                    "EstablishStreams: send to worker {} failed: {}", worker.peer_id, e
                )),
            };

            let reply_text = String::from_utf8_lossy(&response.payload);
            if !reply_text.starts_with("OK") {
                return StepResult::Abort(format!(
                    "EstablishStreams: worker {} replied: {}", worker.peer_id, reply_text
                ));
            }
        }

        let rt_handle = tokio::runtime::Handle::current();
        let job_id = self.job_id;

        let max_attempts = 60;
        let mut endpoint = None;

        for _ in 0..max_attempts {
            match self.capabilities.tensor_switch.Create_Endpoint(inference_id, rt_handle.clone(), job_id).await {
                Ok((ep, _failure_rx)) => {
                    endpoint = Some(ep);
                    break;
                }
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }

        let ep = match endpoint {
            Some(ep) => ep,
            None => return StepResult::Abort(format!(
                "EstablishStreams: timeout waiting for inbound stream (inference_id={})",
                inference_id
            )),
        };

        self.slots.set(
            to_vm_slot(SLOT_TENSOR_IO),
            OrchestratorSlotValue::TensorIO(ep),
        );

        self.vm.slots.set(to_vm_slot(SLOT_LAYER_START), crate::vm_base::SlotValue::U64(plan.coord_layer_start as u64));
        self.vm.slots.set(to_vm_slot(SLOT_LAYER_END), crate::vm_base::SlotValue::U64(plan.coord_layer_end as u64));
        self.vm.slots.set(result, crate::vm_base::SlotValue::String("ok".to_string()));
        StepResult::Continue
    }

    pub async fn handle_join_workers(&mut self, plan_slot: SlotId, result: SlotId) -> StepResult {
        let plan: Pipeline_Plan = match self.slots.take_pipeline_plan(plan_slot) {
            Some(p) => p.clone(),
            None => return StepResult::Abort(format!("JoinWorkers: plan slot {} not found", plan_slot.0)),
        };

        if plan.workers.is_empty() {
            self.vm.slots.set(result, crate::vm_base::SlotValue::String("ok_single_node".to_string()));
            return StepResult::Continue;
        }

        let model_file_id = match self.vm.slots.get_string(to_vm_slot(SLOT_MODEL)) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("JoinWorkers: model slot error: {}", e)),
        };

        for worker in &plan.workers {
            let cmd = NetworkProtocol::Join_Pipeline {
                inference_id: plan.inference_id,
                model_file_id: model_file_id.clone(),
                device: worker.device.clone(),
                layer_start: worker.layer_start,
                layer_end: worker.layer_end,
            };
            let payload = Serialize_Network_Command(&cmd);

            let response = match self.capabilities.network.send_data(
                worker.peer_id,
                DataType::Command,
                payload,
            ).await {
                Ok(r) => r,
                Err(e) => return StepResult::Abort(format!(
                    "JoinWorkers: send to worker {} failed: {}", worker.peer_id, e
                )),
            };

            let reply_text = String::from_utf8_lossy(&response.payload);
            if !reply_text.starts_with("OK") {
                return StepResult::Abort(format!(
                    "JoinWorkers: worker {} replied: {}", worker.peer_id, reply_text
                ));
            }
        }

        self.vm.slots.set(result, crate::vm_base::SlotValue::String("ok".to_string()));
        StepResult::Continue
    }
}
