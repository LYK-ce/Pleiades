//Presented by KeJi
//Date ： 2026-05-04

use libp2p::PeerId;
use crate::orchestrator::slot::{SlotId, SlotValue};
use crate::orchestrator::command::{NetworkProtocol, Serialize_Network_Command};
use crate::orchestrator::compiler::{SLOT_MODEL, SLOT_TENSOR_IO, SLOT_LAYER_START, SLOT_LAYER_END};
use crate::network::stream_protocol::{Write_File_Stream_Header, Read_File_Stream_Ack};
use crate::network::tensor_stream_protocol::Write_Tensor_Stream_Handshake;
use crate::network::DataType;
use crate::storage::{StorageCapability, ChecksumAlgorithm};
use crate::scheduler::Pipeline_Plan;
use super::task_engine::StepResult;

impl super::task_engine::TaskEngine {
    /// 处理 SendFile 指令：将本地文件通过 in-band header 方案发送到远端 Peer
    ///
    /// 单阶段 Stream 协议（与张量流同构）：
    ///   1. 解析 PeerId
    ///   2. 从 Storage 获取文件路径 + 读锁
    ///   3. 获取文件大小 + 计算 checksum
    ///   4. 打开文件流
    ///   5. 写入 in-band header（file_name、file_size、checksum）
    ///   6. 读取 ACK（ACCEPT / REJECT）
    ///   7. 发送文件原始数据（64KB 分块）
    ///   8. 释放读锁
    pub(super) async fn handle_send_file(&mut self, peer: SlotId, file: SlotId) -> StepResult {
        // 1. 解析 PeerId
        let peer_id_str = match self.slots.get_string(peer) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SendFile: peer slot error: {}", e)),
        };
        let peer_id: PeerId = match peer_id_str.parse() {
            Ok(id) => id,
            Err(e) => return StepResult::Abort(format!("SendFile: invalid PeerId '{}': {}", peer_id_str, e)),
        };

        // 2. 获取 file_id
        let file_id = match self.slots.get_string(file) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("SendFile: file slot error: {}", e)),
        };

        // 3. 从 Storage 获取文件路径 + 读锁
        let (path, read_guard) = match self.capabilities.storage.acquire_read(&file_id).await {
            Ok(r) => r,
            Err(e) => return StepResult::Abort(format!("SendFile: acquire_read failed: {}", e)),
        };

        // 4. 获取文件大小
        let file_size = match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) => return StepResult::Abort(format!("SendFile: metadata failed: {}", e)),
        };

        // 5. 计算校验和
        let checksum = match self.capabilities.storage.checksum(&file_id, Some(ChecksumAlgorithm::Blake3)).await {
            Ok(c) => c,
            Err(e) => return StepResult::Abort(format!("SendFile: checksum failed: {}", e)),
        };

        // 6. 打开文件流
        let mut stream = match self.capabilities.network.open_file_stream(peer_id).await {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("SendFile: open_file_stream failed: {}", e)),
        };

        // 7. 写入 in-band header（file_name、file_size、checksum）
        if let Err(e) = Write_File_Stream_Header(&mut stream, &file_id, file_size, &checksum).await {
            return StepResult::Abort(format!("SendFile: write header failed: {}", e));
        }

        // 8. 读取 ACK
        match Read_File_Stream_Ack(&mut stream).await {
            Ok(true) => { /* ACCEPT, 继续 */ }
            Ok(false) => {
                return StepResult::Abort("SendFile: peer rejected file transfer".to_string());
            }
            Err(e) => {
                return StepResult::Abort(format!("SendFile: read ACK failed: {}", e));
            }
        }

        // 9. 发送文件原始数据
        if let Err(e) = self.capabilities.network.send_file_data(&mut stream, &path).await {
            return StepResult::Abort(format!("SendFile: send_file_data failed: {}", e));
        }

        // 10. 释放读锁
        drop(read_guard);

        StepResult::Continue
    }

    /// 处理 ReceiveFile 指令：从入站 Stream 接收文件数据并存入 Storage
    ///
    /// 所有输入参数由 Core 在 spawn Job 时注入到 SlotFile。
    /// 本 handler 完成阶段2 的接收侧工作：
    ///   1. 从槽位取出 stream / file_name / file_size / checksum
    ///   2. 从 Storage 获取写锁 + 目标路径
    ///   3. 通过 stream 接收文件数据
    ///   4. 释放写锁
    ///   5. 本地校验 checksum，不匹配则删除文件
    pub(super) async fn handle_receive_file(
        &mut self,
        stream_slot: SlotId,
        file_name_slot: SlotId,
        file_size_slot: SlotId,
        checksum_slot: SlotId,
        result: SlotId,
    ) -> StepResult {
        // 1. 从槽位读取所有输入
        let mut stream = match self.slots.take_stream(stream_slot) {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("ReceiveFile: stream slot error: {}", e)),
        };

        let file_name = match self.slots.get_string(file_name_slot) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("ReceiveFile: file_name slot error: {}", e)),
        };

        let file_size = match self.slots.get_u64(file_size_slot) {
            Ok(v) => v,
            Err(e) => return StepResult::Abort(format!("ReceiveFile: file_size slot error: {}", e)),
        };

        let expected_checksum = match self.slots.get_string(checksum_slot) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("ReceiveFile: checksum slot error: {}", e)),
        };

        // 2. 从 Storage 获取写锁 + 目标路径
        let (dest_path, write_guard) = match self.capabilities.storage.acquire_write(&file_name).await {
            Ok(r) => r,
            Err(e) => return StepResult::Abort(format!("ReceiveFile: acquire_write failed: {}", e)),
        };

        // 3. 通过 stream 接收文件数据
        if let Err(e) = self.capabilities.network.receive_file_data(&mut stream, &dest_path, file_size).await {
            return StepResult::Abort(format!("ReceiveFile: receive_file_data failed: {}", e));
        }

        // 4. 释放写锁（文件注册到 Storage 索引）
        drop(write_guard);

        // 5. 本地校验 checksum
        let actual_checksum = match self.capabilities.storage.checksum(&file_name, Some(ChecksumAlgorithm::Blake3)).await {
            Ok(c) => c,
            Err(e) => return StepResult::Abort(format!("ReceiveFile: checksum calculation failed: {}", e)),
        };

        if actual_checksum == expected_checksum {
            // 校验通过：写入 result 槽位
            self.slots.set(result, SlotValue::String(file_name));
            StepResult::Continue
        } else {
            // 校验失败：删除损坏文件
            let _ = self.capabilities.storage.remove(&file_name).await;
            StepResult::Abort(format!(
                "ReceiveFile: checksum mismatch (expected: {}, actual: {})",
                expected_checksum, actual_checksum
            ))
        }
    }

    // ─── Pipeline 编排网络指令 ──────────────────────────────────

    /// 处理 EstablishStreams 指令：Phase 1 — 建立所有张量流连接
    ///
    /// 1. 从 `plan` 槽位读取拓扑规划信息
    /// 2. 若无 Worker（单机退化），跳过网络操作
    /// 3. Coordinator 自身打开到第一个 Worker 的出站流 + handshake
    /// 4. 注册出站到 tensor_switch
    /// 5. 向所有 Worker 并发发送 `ESTABLISH_TENSOR_STREAM` 命令
    /// 6. 等待所有 Worker 回复 OK
    /// 7. 等待入站流就绪，创建 Tensor_IO_Endpoint
    /// 8. 注入 SLOT_TENSOR_IO + 写入 Coordinator 层范围
    /// 9. 将结果写入 `result` 槽位
    pub(super) async fn handle_establish_streams(&mut self, plan_slot: SlotId, result: SlotId) -> StepResult {
        // 1. 读取 Pipeline_Plan（clone — JoinWorkers 还需要读）
        let plan: Pipeline_Plan = match self.slots.get_pipeline_plan(plan_slot) {
            Ok(p) => p.clone(),
            Err(e) => return StepResult::Abort(format!("EstablishStreams: plan slot error: {}", e)),
        };

        let inference_id = plan.inference_id;

        // 2. 单机退化：无 Worker，不需要张量流
        if plan.workers.is_empty() {
            // 写入 Coordinator 层范围
            self.slots.set(SLOT_LAYER_START, SlotValue::U64(plan.coord_layer_start as u64));
            self.slots.set(SLOT_LAYER_END, SlotValue::U64(plan.coord_layer_end as u64));
            self.slots.set(result, SlotValue::String("ok_single_node".to_string()));
            return StepResult::Continue;
        }

        // 3. Coordinator 打开到第一个 Worker 的出站流 + handshake
        let outbound_target = match plan.coord_outbound_target {
            Some(target) => target,
            None => return StepResult::Abort("EstablishStreams: coord_outbound_target is None but workers exist".to_string()),
        };

        let mut outbound_stream = match self.capabilities.network.open_tensor_stream(outbound_target).await {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("EstablishStreams: open_tensor_stream failed: {}", e)),
        };

        // 写入 handshake（inference_id）
        if let Err(e) = Write_Tensor_Stream_Handshake(&mut outbound_stream, inference_id).await {
            return StepResult::Abort(format!("EstablishStreams: handshake write failed: {}", e));
        }

        // 4. 注册出站到 tensor_switch
        self.capabilities.tensor_switch.Register_Outbound(inference_id, outbound_stream).await;

        // 5. 向所有 Worker 并发发送 ESTABLISH_TENSOR_STREAM 命令
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
                    "EstablishStreams: send to worker {} failed: {}",
                    worker.peer_id, e
                )),
            };

            // 检查回复
            let reply_text = String::from_utf8_lossy(&response.payload);
            if !reply_text.starts_with("OK") {
                return StepResult::Abort(format!(
                    "EstablishStreams: worker {} replied: {}",
                    worker.peer_id, reply_text
                ));
            }
        }

        // 6. 等待入站流就绪，创建 Tensor_IO_Endpoint
        //    入站流由最后一个 Worker (或唯一 Worker) 打开回 Coordinator，
        //    Core 的 B3 分支 TensorStreamArrived 会调用 Register_Inbound。
        //    这里轮询等待 Create_Endpoint 成功。
        let rt_handle = tokio::runtime::Handle::current();
        let job_id = self.job_id;

        let max_attempts = 60; // 60 * 500ms = 30s 超时
        let mut endpoint = None;

        for _ in 0..max_attempts {
            match self.capabilities.tensor_switch.Create_Endpoint(inference_id, rt_handle.clone(), job_id).await {
                Ok((ep, _failure_rx)) => {
                    endpoint = Some(ep);
                    break;
                }
                Err(_) => {
                    // 入站流尚未到达，等待 500ms 后重试
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

        // 7. 注入 SLOT_TENSOR_IO
        self.slots.set(SLOT_TENSOR_IO, SlotValue::TensorIo(std::sync::Mutex::new(Some(ep))));

        // 8. 写入 Coordinator 层范围
        self.slots.set(SLOT_LAYER_START, SlotValue::U64(plan.coord_layer_start as u64));
        self.slots.set(SLOT_LAYER_END, SlotValue::U64(plan.coord_layer_end as u64));

        // 9. 写结果
        self.slots.set(result, SlotValue::String("ok".to_string()));
        StepResult::Continue
    }

    /// 处理 JoinWorkers 指令：Phase 2 — 通知所有 Worker 加入流水线
    ///
    /// 1. 从 `plan` 槽位读取拓扑规划信息
    /// 2. 从 SLOT_MODEL 读取模型文件 ID（假设所有节点持有完整模型）
    /// 3. 向所有 Worker 并发发送 `JOIN_PIPELINE` 命令
    /// 4. 等待所有 Worker 回复 OK（超时由 Network 层 request_response_timeout 控制）
    /// 5. 将结果写入 `result` 槽位
    pub(super) async fn handle_join_workers(&mut self, plan_slot: SlotId, result: SlotId) -> StepResult {
        // 1. 读取 Pipeline_Plan
        let plan: Pipeline_Plan = match self.slots.get_pipeline_plan(plan_slot) {
            Ok(p) => p.clone(),
            Err(e) => return StepResult::Abort(format!("JoinWorkers: plan slot error: {}", e)),
        };

        // 2. 单机退化：无 Worker
        if plan.workers.is_empty() {
            self.slots.set(result, SlotValue::String("ok_single_node".to_string()));
            return StepResult::Continue;
        }

        // 3. 读取模型文件 ID（所有节点同一份完整模型）
        let model_file_id = match self.slots.get_string(SLOT_MODEL) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("JoinWorkers: model slot error: {}", e)),
        };

        // 4. 向所有 Worker 发送 JOIN_PIPELINE 命令
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
                    "JoinWorkers: send to worker {} failed: {}",
                    worker.peer_id, e
                )),
            };

            // 检查回复
            let reply_text = String::from_utf8_lossy(&response.payload);
            if !reply_text.starts_with("OK") {
                return StepResult::Abort(format!(
                    "JoinWorkers: worker {} replied: {}",
                    worker.peer_id, reply_text
                ));
            }
        }

        // 5. 全部成功
        self.slots.set(result, SlotValue::String("ok".to_string()));
        StepResult::Continue
    }

    /// 处理 TeardownPipeline 指令：补偿 — 清理流水线资源
    ///
    /// 1. 尝试从 `plan` 槽位读取拓扑规划信息（包含 inference_id）
    ///    补偿执行时 plan 可能尚未写入（如果 PlanPipeline 就失败了），此时直接跳过
    /// 2. 调用 `tensor_switch.Deregister_Pipeline(inference_id)` 清理张量流
    /// 3. 返回 Continue（补偿指令永不 Abort）
    pub(super) async fn handle_teardown_pipeline(&mut self, plan_slot: SlotId) -> StepResult {
        // 尝试读取 plan — 补偿时可能尚未写入
        if let Ok(plan) = self.slots.get_pipeline_plan(plan_slot) {
            let inference_id = plan.inference_id;
            // 清理 tensor_switch 中注册的张量流
            self.capabilities.tensor_switch.Deregister_Pipeline(inference_id).await;
        }
        // 补偿指令永不 Abort，尽力清理即可
        StepResult::Continue
    }
}
