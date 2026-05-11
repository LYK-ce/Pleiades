// Presented by KeJi
// Date ： 2026-05-05

//! B3 分支：网络入站 Stream 事件处理
//!
//! 处理来自 Network 的 `Network_Inbound_Event`：
//! - FileStreamArrived: 读取 in-band header → 检查空间 → ACK → compile 接收作业 → spawn Job
//! - TensorStreamArrived: 读取 handshake → Register_Inbound 到 Pipeline entries

use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{Core, generate_id, JobHandle};
use crate::orchestrator::job::{JobId, JobKind};
use crate::orchestrator::core::job_executor::JobExecutor;
use crate::orchestrator::program_selector::ProgramSelector;
use crate::orchestrator::program_selector::SLOT_RECEIVE_STREAM;
use crate::orchestrator::orchestrator_vm::OrchestratorSlotValue;
use crate::network::Network_Inbound_Event;
use crate::network::tensor_stream_protocol::Read_Tensor_Stream_Handshake;
use crate::network::stream_protocol::{Read_File_Stream_Header, Write_File_Stream_Ack};
use crate::event_bus::Bus_Event;
use crate::storage::StorageCapability;

impl Core {
    /// 处理 Network 转发的入站事件
    ///
    /// - FileStreamArrived: 读取 in-band header → 检查空间 → ACK → compile 接收作业 → spawn Job
    /// - TensorStreamArrived: 读取 handshake → Register_Inbound 到 Pipeline entries
    pub(super) async fn handle_network_inbound(&mut self, event: Network_Inbound_Event) {
        match event {
            Network_Inbound_Event::FileStreamArrived { peer, mut stream } => {
                // 1. 读取 in-band header（file_name, file_size, checksum）
                let (file_name, file_size, checksum) = match Read_File_Stream_Header(&mut stream).await {
                    Ok(h) => h,
                    Err(e) => {
                        warn!("B3/File: 读取文件流 header 失败 from {}: {}", peer, e);
                        return;
                    }
                };

                // 2. 检查存储空间
                let quota = self.capabilities.storage.quota_info().await;
                if quota.total > 0 && file_size > quota.available {
                    warn!(
                        "B3/File: 存储空间不足 from {}: 请求 {} bytes, 可用 {} bytes",
                        peer, file_size, quota.available
                    );
                    let _ = Write_File_Stream_Ack(&mut stream, false).await;
                    return;
                }

                // 3. 回复 ACCEPT
                if let Err(e) = Write_File_Stream_Ack(&mut stream, true).await {
                    warn!("B3/File: 写入 ACK 失败 from {}: {}", peer, e);
                    return;
                }

                info!(
                    "B3/File: 接受文件流 from {}: name={}, size={}, checksum={}",
                    peer, file_name, file_size, checksum
                );

                // 4. compile ReceiveFile Job
                let job_id = JobId(generate_id());
                let mut vars = HashMap::new();
                vars.insert("file_name".to_string(), file_name.clone());
                vars.insert("file_size".to_string(), file_size.to_string());
                vars.insert("checksum".to_string(), checksum.clone());
                let program = match ProgramSelector::select(JobKind::Receive, job_id, vars) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!("B3/File: 编译 ReceiveFile 失败 from {}: {:?}", peer, e);
                        return;
                    }
                };

                // 5. 创建 Executor + 注入 stream 到 SlotFile
                // ReceiveFile Job 不使用 ML 推理，无需 IO 通道
                let cancel = CancellationToken::new();
                let mut executor = JobExecutor::new(
                    job_id,
                    JobKind::Receive,
                    program,
                    cancel.clone(),
                    self.capabilities.clone(),
                    None,
                    self.lifecycle_tx.clone(),
                );
                executor.inject_slot(
                    SLOT_RECEIVE_STREAM,
                    OrchestratorSlotValue::Stream(std::sync::Mutex::new(Some(stream))),
                );

                // 7. 发布 Job 创建事件 + spawn + 注册
                self.capabilities.event_bus.Publish(Bus_Event::Job_Created {
                    job_id: job_id.0,
                    kind: "Receive".to_string(),
                    model_name: file_name,
                });
                tokio::spawn(executor.run());
                self.registry.insert(job_id, JobHandle { kind: JobKind::Receive, cancel, inference_id: None });

                info!("B3/File: ReceiveFile Job {:?} 已 spawn", job_id);
            }
            Network_Inbound_Event::TensorStreamArrived { peer, mut stream } => {
                // 从 stream 读取 handshake 帧（inference_id）
                match Read_Tensor_Stream_Handshake(&mut stream).await {
                    Ok(inference_id) => {
                        info!("B3/Tensor: 收到入站张量流 from {}, inference_id={}", peer, inference_id);

                        // 注册为 Pipeline 入站流（供 Join_Pipeline / Create_Endpoint 使用）
                        self.capabilities.tensor_switch.Register_Inbound(inference_id, stream).await;
                        info!(
                            "B3/Tensor: inference_id={} 入站流已注册到 Pipeline entries (from {})",
                            inference_id, peer
                        );
                    }
                    Err(e) => {
                        warn!("B3/Tensor: 读取张量流 handshake 失败 from {}: {}", peer, e);
                    }
                }
            }
        }
    }
}
