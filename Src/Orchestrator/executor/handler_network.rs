//Presented by KeJi
//Date ： 2026-04-28

use std::sync::Mutex;
use libp2p::PeerId;
use crate::orchestrator::slot::{SlotId, SlotValue};
use crate::network::DataType;
use crate::network::tensor_stream_protocol::{Tensor_IO_Handle, Write_Tensor_Stream_Handshake};
use crate::storage::{StorageCapability, ChecksumAlgorithm};
use super::task_engine::StepResult;

impl super::TaskEngine {
    /// 处理 SendFile 指令：将本地文件通过三阶段协议发送到远端 Peer
    ///
    /// 阶段1：元数据协商（Request-Response，DataType::File）
    ///   1. 解析 PeerId
    ///   2. 从 Storage 获取文件路径 + 读锁
    ///   3. 获取文件大小 + 计算 checksum
    ///   4. 发送元数据，等待对方 accept/reject
    ///
    /// 阶段2：文件数据传输（Stream 协议）
    ///   5. 打开文件流
    ///   6. 发送文件原始数据（64KB 分块）
    ///   7. 释放读锁
    ///
    /// 阶段3：发送方主动验证（Request-Response，DataType::Command + Verify_File）
    ///   8. 发送 Verify_File 命令，等待 confirmed/failed
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

        // ═══ 阶段1：元数据协商 ═══

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

        // 6. 序列化元数据 payload（简单文本格式：file_name|file_size|checksum）
        let metadata_payload = format!("{}|{}|{}", file_id, file_size, checksum).into_bytes();

        // 7. 发送元数据，等待对方响应
        let response = match self.capabilities.network.send_data(peer_id, DataType::File, metadata_payload).await {
            Ok(r) => r,
            Err(e) => return StepResult::Abort(format!("SendFile: metadata send failed: {}", e)),
        };

        // 8. 解析响应：reject → Abort
        let response_str = String::from_utf8_lossy(&response.payload);
        if !response_str.contains("ACCEPT") && !response_str.contains("accept") {
            return StepResult::Abort(format!("SendFile: peer rejected file transfer: {}", response_str));
        }

        // ═══ 阶段2：文件数据传输 ═══

        // 9. 打开文件流
        let mut stream = match self.capabilities.network.open_file_stream(peer_id).await {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("SendFile: open_file_stream failed: {}", e)),
        };

        // 10. 发送文件原始数据
        if let Err(e) = self.capabilities.network.send_file_data(&mut stream, &path).await {
            return StepResult::Abort(format!("SendFile: send_file_data failed: {}", e));
        }

        // 11. 释放读锁
        drop(read_guard);

        // ═══ 阶段3：发送方主动验证 ═══

        // 12. 发送 Verify_File 命令（"VERIFY_FILE|<file_name>" 文本协议格式）
        let verify_payload = format!("VERIFY_FILE|{}", file_id).into_bytes();
        let verify_response = match self.capabilities.network.send_data(peer_id, DataType::Command, verify_payload).await {
            Ok(r) => r,
            Err(e) => return StepResult::Abort(format!("SendFile: verify request failed: {}", e)),
        };

        // 13. 解析验证响应
        let verify_str = String::from_utf8_lossy(&verify_response.payload);
        if verify_str.contains("confirmed") {
            StepResult::Continue
        } else {
            StepResult::Abort(format!("SendFile: receiver verification failed for '{}': {}", file_id, verify_str))
        }
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

    /// 处理 OpenTensorStream 指令：向下游 Peer 打开出站张量流并存入 Broker
    ///
    /// 1. 从 `peer` 槽位读取下游 PeerId
    /// 2. 从 `target_job` 槽位读取远端 Job ID（用于 handshake 帧）
    /// 3. 调用 `network.open_tensor_stream(peer)` 打开出站流
    /// 4. 在流上写入 handshake 帧（target_job_id）
    /// 5. 将 outbound 存入 `Tensor_IO_Broker.Store_Outbound(job_id, stream)`
    pub(super) async fn handle_open_tensor_stream(&mut self, peer: SlotId, target_job: SlotId) -> StepResult {
        // 1. 解析 PeerId
        let peer_id_str = match self.slots.get_string(peer) {
            Ok(s) => s.clone(),
            Err(e) => return StepResult::Abort(format!("OpenTensorStream: peer slot error: {}", e)),
        };
        let peer_id: PeerId = match peer_id_str.parse() {
            Ok(id) => id,
            Err(e) => return StepResult::Abort(format!("OpenTensorStream: invalid PeerId '{}': {}", peer_id_str, e)),
        };

        // 2. 读取 target_job_id
        let target_job_id = match self.slots.get_u64(target_job) {
            Ok(v) => v,
            Err(e) => return StepResult::Abort(format!("OpenTensorStream: target_job slot error: {}", e)),
        };

        // 3. 打开出站流
        let mut stream = match self.capabilities.network.open_tensor_stream(peer_id).await {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("OpenTensorStream: open_tensor_stream failed: {}", e)),
        };

        // 4. 写入 handshake 帧（target_job_id）
        if let Err(e) = Write_Tensor_Stream_Handshake(&mut stream, target_job_id).await {
            return StepResult::Abort(format!("OpenTensorStream: handshake write failed: {}", e));
        }

        // 5. 存入 Broker
        if let Err(e) = self.capabilities.tensor_io_broker.Store_Outbound(self.job_id, stream).await {
            return StepResult::Abort(format!("OpenTensorStream: broker Store_Outbound failed: {}", e));
        }

        StepResult::Continue
    }

    /// 处理 TakeInboundStream 指令：从 Tensor_IO_Broker 取出入站张量流
    ///
    /// 调用 `broker.Take_Inbound(job_id).await`（阻塞直到 inbound 到达）
    /// raw `libp2p::Stream` 写入 `result` 槽位
    pub(super) async fn handle_take_inbound_stream(&mut self, result: SlotId) -> StepResult {
        match self.capabilities.tensor_io_broker.Take_Inbound(self.job_id).await {
            Ok(stream) => {
                self.slots.set(result, SlotValue::Stream(Mutex::new(Some(stream))));
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("TakeInboundStream: broker Take_Inbound failed: {}", e)),
        }
    }

    /// 处理 TakeOutboundStream 指令：从 Tensor_IO_Broker 取出出站张量流
    ///
    /// 调用 `broker.Take_Outbound(job_id).await`（阻塞直到 outbound 就绪）
    /// raw `libp2p::Stream` 写入 `result` 槽位
    pub(super) async fn handle_take_outbound_stream(&mut self, result: SlotId) -> StepResult {
        match self.capabilities.tensor_io_broker.Take_Outbound(self.job_id).await {
            Ok(stream) => {
                self.slots.set(result, SlotValue::Stream(Mutex::new(Some(stream))));
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("TakeOutboundStream: broker Take_Outbound failed: {}", e)),
        }
    }

    /// 处理 BuildTensorIo 指令：组装 Tensor_IO_Handle
    ///
    /// 从 `inbound`/`outbound` 槽位 take 两条 raw stream，
    /// 调用 `Tensor_IO_Handle::New(inbound, outbound, rt)` 组装，
    /// 结果写入 `result` 槽位（SlotValue::TensorIo）
    pub(super) async fn handle_build_tensor_io(
        &mut self,
        inbound: SlotId,
        outbound: SlotId,
        result: SlotId,
    ) -> StepResult {
        // 1. 取出 inbound stream
        let inbound_stream = match self.slots.take_stream(inbound) {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("BuildTensorIo: inbound slot error: {}", e)),
        };

        // 2. 取出 outbound stream
        let outbound_stream = match self.slots.take_stream(outbound) {
            Ok(s) => s,
            Err(e) => return StepResult::Abort(format!("BuildTensorIo: outbound slot error: {}", e)),
        };

        // 3. 组装 Tensor_IO_Handle
        let rt = tokio::runtime::Handle::current();
        let tensor_io = Tensor_IO_Handle::New(inbound_stream, outbound_stream, rt);

        // 4. 存入 result 槽位
        self.slots.set(result, SlotValue::TensorIo(Mutex::new(Some(tensor_io))));
        StepResult::Continue
    }
}
