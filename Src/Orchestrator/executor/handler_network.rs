//Presented by KeJi
//Date ： 2026-04-30

use std::sync::Mutex;
use libp2p::PeerId;
use crate::orchestrator::slot::{SlotId, SlotValue};
use crate::network::DataType;
use crate::network::tensor_stream_protocol::Write_Tensor_Stream_Handshake;
use crate::network::stream_protocol::{Write_File_Stream_Header, Read_File_Stream_Ack};
use crate::storage::{StorageCapability, ChecksumAlgorithm};
use super::task_engine::StepResult;

impl super::TaskEngine {
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

    /// 处理 RequestPipeline 指令：请求远端 Peer 加入 Pipeline 并获取 Relay Job ID
    ///
    /// 1. 解析 PeerId
    /// 2. 从槽位读取 model_file_id、device、layer_start、layer_end
    /// 3. 构造 REQUEST_PIPELINE payload（携带 coordinator_job_id + 上述参数）
    /// 4. 通过 send_data 发送，等待远端回复
    /// 5. 解析 relay_job_id 存入 result 槽位
    pub(super) async fn handle_request_pipeline(
        &mut self,
        peer: SlotId,
        model: SlotId,
        device: SlotId,
        start: SlotId,
        end: SlotId,
        result: SlotId,
    ) -> StepResult {
        // 1. 解析 PeerId
        let peer_id_str = match self.slots.get_string(peer) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("RequestPipeline: peer slot error: {}", e)),
        };
        let peer_id: PeerId = match peer_id_str.parse() {
            Ok(id) => id,
            Err(e) => return StepResult::Abort(format!("RequestPipeline: invalid PeerId '{}': {}", peer_id_str, e)),
        };

        // 2. 读取模型/设备/层范围参数
        let model_file_id = match self.slots.get_string(model) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("RequestPipeline: model slot error: {}", e)),
        };
        let device_str = match self.slots.get_string(device) {
            Ok(s) => s.clone(),
            Err(_) => "cpu".to_string(),
        };
        let layer_start = match self.slots.get_u64(start) {
            Ok(v) => v,
            Err(_) => 0,
        };
        let layer_end = match self.slots.get_u64(end) {
            Ok(v) => v,
            Err(_) => u64::MAX,
        };

        // 3. 构造 payload：REQUEST_PIPELINE|coordinator_job_id|model_file_id|device|layer_start|layer_end
        let coordinator_job_id = self.job_id.0;
        let payload = format!(
            "REQUEST_PIPELINE|{}|{}|{}|{}|{}",
            coordinator_job_id, model_file_id, device_str, layer_start, layer_end
        ).into_bytes();

        // 4. 发送请求，等待响应
        let response = match self.capabilities.network.send_data(peer_id, DataType::Command, payload).await {
            Ok(r) => r,
            Err(e) => return StepResult::Abort(format!("RequestPipeline: send_data failed: {}", e)),
        };

        // 5. 解析响应：期望 "OK|{relay_job_id}" 或 "REJECT|{reason}"
        let response_str = String::from_utf8_lossy(&response.payload);
        if response_str.starts_with("OK|") {
            let relay_job_id_str = &response_str[3..];
            match relay_job_id_str.parse::<u64>() {
                Ok(relay_job_id) => {
                    self.slots.set(result, SlotValue::U64(relay_job_id));
                    StepResult::Continue
                }
                Err(e) => StepResult::Abort(format!(
                    "RequestPipeline: invalid relay_job_id in response '{}': {}",
                    response_str, e
                )),
            }
        } else {
            StepResult::Abort(format!("RequestPipeline: peer rejected: {}", response_str))
        }
    }

    // NOTE: handle_open_tensor_stream, handle_take_inbound_stream,
    // handle_take_outbound_stream, handle_build_tensor_io 已在 Phase 3 移除。
    // 张量流连接现由 Core 在 spawn Job 之前完成（"先连接后启动"模式）。
}
