// Presented by KeJi
// Date ： 2026-05-05

//! B2 分支：网络入站 Request-Response 命令处理
//!
//! 处理来自 Network 的 `InboundRequest`（仅 Command 类型），
//! 解析 `NetworkProtocol` 并分发到对应处理逻辑。

use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{Core, generate_id, JobHandle};
use crate::orchestrator::job::{JobId, JobKind};
use crate::orchestrator::command::{NetworkProtocol, Parse_Network_Command};
use crate::orchestrator::core::job_executor::JobExecutor;
use crate::orchestrator::program_selector::ProgramSelector;
use crate::orchestrator::program_selector::SLOT_TENSOR_IO;
use crate::orchestrator::orchestrator_vm::OrchestratorSlotValue;
use crate::network::{InboundRequest, DataType};
use crate::network::tensor_stream_protocol::Write_Tensor_Stream_Handshake;
use crate::event_bus::Bus_Event;
use crate::llm_io::LLM_IO_Capability;

impl Core {
    /// 处理 B2 入站请求（Request-Response 协议，仅 Command）
    ///
    /// Network_Service 按 DataType 预筛选后，仅将 Command 类型转发到此方法。
    /// 文件传输已改为 in-band header + stream 方案，不再使用 Request-Response。
    /// 每个请求携带 request_id，处理完后必须通过 network.send_response(request_id, ...) 回复。
    pub(super) async fn handle_inbound_request(&mut self, req: InboundRequest) {
        match req.data_type {
            DataType::Command => {
                self.handle_network_command(req).await;
            }
            _ => {
                // File / BandwidthTest / Data / Info 不应到达此处
                warn!("B2: 收到非预期的 DataType {:?} from {}, request_id={}", req.data_type, req.peer, req.request_id);
            }
        }
    }

    /// 处理 DataType::Command 入站请求（命令分发）
    ///
    /// 通过 `Parse_Network_Command` 解析为 `NetworkProtocol` 枚举，
    /// 再按变体分发到对应处理方法。
    async fn handle_network_command(&mut self, req: InboundRequest) {
        let protocol = match Parse_Network_Command(&req.payload) {
            Ok(p) => p,
            Err(e) => {
                warn!("B2/Command: 协议解析失败 from {}, request_id={}: {}", req.peer, req.request_id, e);
                // 回复解析错误
                if let Err(send_err) = self.capabilities.network.send_response(
                    req.request_id, DataType::Command, format!("REJECT|{}", e).into_bytes()
                ).await {
                    warn!("send_response 失败: {}", send_err);
                }
                return;
            }
        };

        match protocol {
            NetworkProtocol::Establish_Tensor_Stream {
                inference_id,
                target_peer_id,
            } => {
                info!(
                    "B2/Command: ESTABLISH_TENSOR_STREAM from {}, inference_id={}, target={}",
                    req.peer, inference_id, target_peer_id
                );

                // 1. 解析 target_peer_id
                let target = match target_peer_id.parse::<libp2p::PeerId>() {
                    Ok(p) => p,
                    Err(e) => {
                        let response = format!("FAIL|invalid target_peer_id: {}", e);
                        if let Err(send_err) = self.capabilities.network.send_response(
                            req.request_id, DataType::Command, response.into_bytes()
                        ).await {
                            warn!("send_response 失败: {}", send_err);
                        }
                        return;
                    }
                };

                // 2. 打开到 target 的出站张量流
                let mut outbound_stream = match self.capabilities.network.open_tensor_stream(target).await {
                    Ok(s) => s,
                    Err(e) => {
                        let response = format!("FAIL|open_tensor_stream failed: {}", e);
                        if let Err(send_err) = self.capabilities.network.send_response(
                            req.request_id, DataType::Command, response.into_bytes()
                        ).await {
                            warn!("send_response 失败: {}", send_err);
                        }
                        return;
                    }
                };

                // 3. 写入 handshake（inference_id 作为标识）
                if let Err(e) = Write_Tensor_Stream_Handshake(&mut outbound_stream, inference_id).await {
                    let response = format!("FAIL|handshake failed: {}", e);
                    if let Err(send_err) = self.capabilities.network.send_response(
                        req.request_id, DataType::Command, response.into_bytes()
                    ).await {
                        warn!("send_response 失败: {}", send_err);
                    }
                    return;
                }

                // 4. 注册出站到 tensor_switch
                self.capabilities.tensor_switch.Register_Outbound(inference_id, outbound_stream).await;

                // 5. 回复 OK
                info!("ESTABLISH_TENSOR_STREAM 成功: inference_id={}, target={}", inference_id, target);
                let response = "OK".to_string();
                if let Err(e) = self.capabilities.network.send_response(
                    req.request_id, DataType::Command, response.into_bytes()
                ).await {
                    warn!("send_response 失败: {}", e);
                }
            }
            NetworkProtocol::Join_Pipeline {
                inference_id,
                model_file_id,
                device,
                layer_start,
                layer_end,
            } => {
                info!(
                    "B2/Command: JOIN_PIPELINE from {}, inference_id={}, model={}, device={}, layers={}-{}",
                    req.peer, inference_id, model_file_id, device, layer_start, layer_end
                );

                let job_id = JobId(generate_id());

                // 1. 等待 Create_Endpoint 就绪（短暂重试，最多 3s）
                let rt = tokio::runtime::Handle::current();
                let mut endpoint_result = None;
                for _ in 0..6 {
                    match self.capabilities.tensor_switch.Create_Endpoint(
                        inference_id, rt.clone(), job_id,
                    ).await {
                        Ok(ep) => {
                            endpoint_result = Some(ep);
                            break;
                        }
                        Err(_) => {
                            // inbound/outbound 尚未就绪，等待 500ms 后重试
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        }
                    }
                }
                let (endpoint, _failure_rx) = match endpoint_result {
                    Some(ep) => ep,
                    None => {
                        let response = "FAIL|tensor streams not ready after 3s".to_string();
                        if let Err(e) = self.capabilities.network.send_response(
                            req.request_id, DataType::Command, response.into_bytes()
                        ).await {
                            warn!("send_response 失败: {}", e);
                        }
                        return;
                    }
                };

                // 2. 编译 Relay 程序
                let mut vars = HashMap::new();
                vars.insert("model_path".to_string(), model_file_id.clone());
                vars.insert("device".to_string(), device.clone());
                vars.insert("layer_start".to_string(), layer_start.to_string());
                vars.insert("layer_end".to_string(), layer_end.to_string());
                let program = match ProgramSelector::select(JobKind::Relay, job_id, vars) {
                    Ok(p) => p,
                    Err(e) => {
                        self.capabilities.tensor_switch.Deregister_Pipeline(inference_id).await;
                        let response = format!("FAIL|compile error: {:?}", e);
                        if let Err(send_err) = self.capabilities.network.send_response(
                            req.request_id, DataType::Command, response.into_bytes()
                        ).await {
                            warn!("send_response 失败: {}", send_err);
                        }
                        return;
                    }
                };

                // 3. 分配 IO 通道
                if let Err(e) = self.capabilities.io_broker.Allocate(job_id).await {
                    self.capabilities.tensor_switch.Deregister_Pipeline(inference_id).await;
                    let response = format!("FAIL|IO allocate error: {}", e);
                    if let Err(send_err) = self.capabilities.network.send_response(
                        req.request_id, DataType::Command, response.into_bytes()
                    ).await {
                        warn!("send_response 失败: {}", send_err);
                    }
                    return;
                }
                let io = match self.capabilities.io_broker.Take_ML_Side(job_id).await {
                    Ok(handle) => handle,
                    Err(e) => {
                        self.capabilities.tensor_switch.Deregister_Pipeline(inference_id).await;
                        let response = format!("FAIL|IO Take_ML_Side error: {}", e);
                        if let Err(send_err) = self.capabilities.network.send_response(
                            req.request_id, DataType::Command, response.into_bytes()
                        ).await {
                            warn!("send_response 失败: {}", send_err);
                        }
                        return;
                    }
                };

                // 4. 创建 Executor + 注入 Tensor_IO_Endpoint + spawn
                self.capabilities.event_bus.Publish(Bus_Event::Job_Created {
                    job_id: job_id.0,
                    kind: format!("{:?}", JobKind::Relay),
                    model_name: String::new(),
                });

                let cancel = CancellationToken::new();
                let mut executor = JobExecutor::new(
                    job_id,
                    JobKind::Relay,
                    program,
                    cancel.clone(),
                    self.capabilities.clone(),
                    Some(io),
                    self.lifecycle_tx.clone(),
                );
                // 预注入 Tensor_IO_Endpoint 到约定槽位
                executor.inject_slot(
                    SLOT_TENSOR_IO,
                    OrchestratorSlotValue::TensorIO(endpoint),
                );
                tokio::spawn(executor.run());
                self.registry.insert(job_id, JobHandle { kind: JobKind::Relay, cancel, inference_id: Some(inference_id) });

                info!("JOIN_PIPELINE 成功: inference_id={}, job_id={:?}", inference_id, job_id);

                // 5. 回复 OK
                let response = "OK".to_string();
                if let Err(e) = self.capabilities.network.send_response(
                    req.request_id, DataType::Command, response.into_bytes()
                ).await {
                    warn!("send_response 失败: {}", e);
                }
            }
        }
    }
}
