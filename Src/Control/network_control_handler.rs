//Presented by KeJi
//Date ： 2026-04-17

//! 网络控制命令处理器模块
//!
//! 专门处理网络控制命令，包含 Handle_Inbound 和 Handle_Control_Command 函数，
//! 导出 NetworkCommandHandler 结构体。
//! 从 control.rs 中提取网络控制相关逻辑，减少代码耦合。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::{info, warn, error, debug};

use super::network_control_command::{Control_Command, Serialize_Command, Deserialize_Command};
use super::ui_message::Ui_Message;
use crate::network::data_protocol::DataType;
use crate::network::node_handle::{InboundRequest, NodeHandle};
use crate::network::tensor_stream_manager::Tensor_IO_Handle;
use crate::ml_engine::ml_inference_service::Create_Session;
use crate::ml_engine::ml_thread_engine_instruction::{Instruction, Inference_Input, Pipeline_Params};
use crate::llm_io::IoHandle;
use crate::ml_engine::ml_thread_register::TENSOR1;
use std::path::PathBuf;

// ============================================================
// NetworkCommandHandler 结构体
// ============================================================

/// 网络控制命令处理器
///
/// 封装所有网络控制命令的处理逻辑，提供统一的 Handle_Inbound 接口。
pub struct NetworkCommandHandler;

impl NetworkCommandHandler {
    /// 处理入站请求（其他节点发来的）
    ///
    /// 根据不同的 DataType 类型，调用相应的处理函数。
    /// 主要处理 Control_Command 命令，也处理文件传输、数据消息等。
    pub async fn Handle_Inbound(
        state: &mut super::control::Node_State,
        next_peer: &mut Option<PeerId>,
        pending_load: &mut Option<(String, usize, usize)>,
        node_handle: &NodeHandle,
        device: &str,
        req: InboundRequest,
        ui_tx: &mpsc::Sender<Ui_Message>,
    ) {
        info!(
            "Control: 收到入站请求 (id={}, peer={}, type={:?}, payload_len={}, state={:?})",
            req.request_id, req.peer, req.data_type, req.payload.len(), state
        );

        match req.data_type {
            // ===== 文件传输：始终接受 =====
            DataType::File => {
                Self::Send_Ui(ui_tx, Ui_Message::Log(format!(
                    "收到文件传输请求 (来自 {}), 自动接受", req.peer
                ))).await;
                if let Err(e) = node_handle
                    .Send_Response(req.request_id, DataType::Command, b"ACCEPT".to_vec())
                    .await
                {
                    error!("发送文件接受回复失败: {}", e);
                }
            }

            // ===== 命令处理 =====
            DataType::Command => {
                match Deserialize_Command(&req.payload) {
                    Ok(cmd) => {
                        Self::Handle_Control_Command(
                            state, next_peer, pending_load, node_handle, device,
                            req.request_id, req.peer, cmd, ui_tx,
                        ).await;
                    }
                    Err(e) => {
                        warn!("命令解析失败: {}", e);
                        if let Err(e) = node_handle
                            .Send_Response(req.request_id, DataType::Command, b"OK".to_vec())
                            .await
                        {
                            error!("发送回复失败: {}", e);
                        }
                    }
                }
            }

            // ===== 数据处理 =====
            DataType::Data => {
                debug!("收到 Data 消息 (来自 {}), 回复 OK", req.peer);
                let _ = node_handle
                    .Send_Response(req.request_id, DataType::Data, b"OK".to_vec())
                    .await;
            }

            // ===== Info 消息 =====
            DataType::Info => {
                debug!("收到 Info 消息 (来自 {}), 暂未处理", req.peer);
                let _ = node_handle
                    .Send_Response(req.request_id, DataType::Info, b"OK".to_vec())
                    .await;
            }

            // ===== 带宽测试消息 =====
            DataType::BandwidthTest => {
                debug!("收到带宽测试消息 (来自 {}), payload长度={}", req.peer, req.payload.len());
                
                // 带宽测试请求的payload包含数据包大小（小端字节序）
                if req.payload.len() >= 8 {
                    // 读取数据包大小
                    let size_bytes = u64::from_le_bytes([
                        req.payload[0], req.payload[1], req.payload[2], req.payload[3],
                        req.payload[4], req.payload[5], req.payload[6], req.payload[7],
                    ]);
                    
                    // 创建指定大小的响应数据包（填充零）
                    let response_payload = vec![0u8; size_bytes as usize];
                    
                    let _ = node_handle
                        .Send_Response(req.request_id, DataType::BandwidthTest, response_payload)
                        .await;
                } else {
                    warn!("带宽测试请求payload长度不足: {}", req.payload.len());
                    let _ = node_handle
                        .Send_Response(req.request_id, DataType::BandwidthTest, vec![0u8; 8])
                        .await;
                }
            }
        }
    }

    /// 处理 Control_Command（Worker 被动模式）
    pub(crate) async fn Handle_Control_Command(
        state: &mut super::control::Node_State,
        next_peer: &mut Option<PeerId>,
        pending_load: &mut Option<(String, usize, usize)>,
        node_handle: &NodeHandle,
        device: &str,
        request_id: u64,
        peer: PeerId,
        cmd: Control_Command,
        ui_tx: &mpsc::Sender<Ui_Message>,
    ) {
        match cmd {
            Control_Command::Work => {
                *state = super::control::Node_State::Busy;
                Self::Send_Ui(ui_tx, Ui_Message::Log(format!("WORK (来自 {}): 状态 → Busy", peer))).await;
                Self::Send_Ui(ui_tx, Ui_Message::State_Change("Busy".to_string())).await;
                let _ = node_handle
                    .Send_Response(request_id, DataType::Command, b"OK".to_vec())
                    .await;
            }

            Control_Command::Load { model_path, start, end } => {
                // 暂存 Load 参数，等 Pipeline_Flow 时再创建 Session
                Self::Send_Ui(ui_tx, Ui_Message::Log(format!("LOAD (来自 {}): {} 层 {}-{}", peer, model_path, start, end))).await;
                Self::Send_Ui(ui_tx, Ui_Message::Job_Inference {
                    model_name: model_path.clone(),
                    device_count: 1,
                    layer_range: format!("层{}-{}", start, end),
                    phase: "等待 Pipeline 配置".to_string(),
                }).await;

                *pending_load = Some((model_path, start, end));

                let _ = node_handle
                    .Send_Response(request_id, DataType::Command, b"OK".to_vec())
                    .await;
            }

            Control_Command::Prepare_Connection => {
                // 创建 Tensor Stream Manager（准备接收入站张量流）
                // 必须在 Pipeline_Flow 之前完成，避免上游节点打开出站流时本节点尚未就绪
                Self::Send_Ui(ui_tx, Ui_Message::Log(format!("PREPARE_CONNECTION (来自 {}): 创建 Tensor Stream Manager", peer))).await;
                if let Err(e) = node_handle.Create_Tensor_Stream().await {
                    error!("创建 Tensor Stream 失败: {}", e);
                    let _ = node_handle
                        .Send_Response(request_id, DataType::Command, format!("ERROR: {}", e).into_bytes())
                        .await;
                    return;
                }
                Self::Send_Ui(ui_tx, Ui_Message::Log("✓ Tensor Stream Manager 已创建".to_string())).await;
                let _ = node_handle
                    .Send_Response(request_id, DataType::Command, b"OK".to_vec())
                    .await;
            }

            Control_Command::Pipeline_Flow { next_peer: target } => {
                *next_peer = Some(target);
                Self::Send_Ui(ui_tx, Ui_Message::Log(format!("PIPELINE_FLOW (来自 {}): next → {}", peer, target))).await;

                // 取出 pending_load 参数
                let load_params = match pending_load.take() {
                    Some(params) => params,
                    None => {
                        error!("收到 PIPELINE_FLOW 但没有 pending_load 参数");
                        let _ = node_handle
                            .Send_Response(request_id, DataType::Command, b"ERROR: no pending load".to_vec())
                            .await;
                        return;
                    }
                };
                let (model_path_str, layer_start, layer_end) = load_params;

                // 1. 打开到 next_peer 的出站流（Manager 已在 Prepare_Connection 中创建）
                if let Err(e) = node_handle.Open_Tensor_Stream(&target).await {
                    error!("打开出站 Tensor Stream 失败: {}", e);
                    let _ = node_handle
                        .Send_Response(request_id, DataType::Command, format!("ERROR: {}", e).into_bytes())
                        .await;
                    return;
                }

                // 回复 OK（流设置完成，后续操作异步进行）
                let _ = node_handle
                    .Send_Response(request_id, DataType::Command, b"OK".to_vec())
                    .await;

                Self::Send_Ui(ui_tx, Ui_Message::Job_Inference {
                    model_name: model_path_str.clone(),
                    device_count: 1,
                    layer_range: format!("层{}-{}", layer_start, layer_end),
                    phase: "加载模型".to_string(),
                }).await;

                // 3. 等待 inbound stream 就绪（带重试）
                let mut take_result = None;
                for attempt in 0..20 {
                    match node_handle.Take_Tensor_Streams().await {
                        Ok(streams) => {
                            take_result = Some(streams);
                            break;
                        }
                        Err(_) => {
                            if attempt < 19 {
                                debug!("Worker: 等待 Tensor Stream 就绪... (尝试 {}/20)", attempt + 1);
                                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                            }
                        }
                    }
                }

                match take_result {
                    Some((inbound_stream, outbound_stream)) => {
                        let rt_handle = tokio::runtime::Handle::current();
                        let tensor_io = Tensor_IO_Handle::New(inbound_stream, outbound_stream, rt_handle);

                        // 4. 创建 Session 并启动 Relay（fire-and-forget）
                        let device_clone = device.to_string();
                        let ui_tx_clone = ui_tx.clone();
                        let node_handle_clone = node_handle.clone();
                        let model_path = PathBuf::from(&model_path_str);

                        tokio::spawn(async move {
                            // Worker 不需要前端文本通道，但 API 需要 IoHandle
                            let (_worker_input_tx, worker_input_rx) = mpsc::channel::<String>(1);
                            let (worker_output_tx, _worker_output_rx) = mpsc::channel::<String>(1);
                            let worker_io_handle = IoHandle { input_rx: worker_input_rx, output_tx: worker_output_tx };

                            // 创建 Session（加载模型 — 在 Session 的 OS 线程中阻塞）
                            let session_result = Create_Session(
                                "worker".to_string(),
                                &model_path,
                                layer_start,
                                layer_end,
                                device_clone,
                                Some(tensor_io),
                                worker_io_handle,
                            ).await;

                            match session_result {
                                Ok((session_handle, model_info)) => {
                                    Self::Send_Ui(&ui_tx_clone, Ui_Message::Log(format!(
                                        "✓ Worker 模型加载完成 (arch: {}, input: {}, output: {})",
                                        model_info.architecture, model_info.has_input_head, model_info.has_output_head
                                    ))).await;

                                    Self::Send_Ui(&ui_tx_clone, Ui_Message::Job_Inference {
                                        model_name: model_path_str.clone(),
                                        device_count: 1,
                                        layer_range: format!("层{}-{}", layer_start, layer_end),
                                        phase: "就绪，等待推理数据".to_string(),
                                    }).await;

                                    // 启动 Relay 程序
                                    let relay_program = Self::Build_Worker_Relay_Program();
                                    let cancel_flag = Arc::new(AtomicBool::new(false));

                                    info!("Worker: 启动 Relay 程序");
                                    let result = session_handle
                                        .Run_Program(
                                            relay_program,
                                            Pipeline_Params::default(),
                                            cancel_flag,
                                        )
                                        .await;

                                    match result {
                                        Ok(_) => {
                                            info!("Worker: Relay 程序正常结束");
                                        }
                                        Err(e) => {
                                            error!("Worker: Relay 程序执行失败: {}", e);
                                            Self::Send_Ui(&ui_tx_clone, Ui_Message::Log(format!(
                                                "Worker Relay 程序失败: {}", e
                                            ))).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Worker 创建 Session 失败: {}", e);
                                    Self::Send_Ui(&ui_tx_clone, Ui_Message::Log(format!(
                                        "Worker 模型加载失败: {}", e
                                    ))).await;
                                }
                            }
                        });
                    }
                    None => {
                        error!("Worker: 等待 Tensor Stream 超时");
                        Self::Send_Ui(ui_tx, Ui_Message::Log("Worker: 等待 Tensor Stream 超时".to_string())).await;
                    }
                }
            }

            Control_Command::Verify_File { file_name } => {
                // 文件传输阶段3：校验文件是否已成功接收
                // 当前为占位符，后续由 Orchestrator Core 层处理（查 Storage.exists）
                debug!("VERIFY_FILE (来自 {}): 校验文件 '{}'", peer, file_name);
                Self::Send_Ui(ui_tx, Ui_Message::Log(format!(
                    "收到文件校验请求 (来自 {}): {}", peer, file_name
                ))).await;
                // 占位符：默认回复 confirmed（后续接入 Storage 校验）
                let _ = node_handle
                    .Send_Response(request_id, DataType::Command, b"confirmed".to_vec())
                    .await;
            }
        }
    }

    /// 发送 UI 消息的辅助函数（async 版本）
    async fn Send_Ui(ui_tx: &mpsc::Sender<Ui_Message>, msg: Ui_Message) {
        let _ = ui_tx.send(msg).await;
    }

    /// 构建 Worker Relay 程序
    fn Build_Worker_Relay_Program() -> Vec<Instruction> {
        vec![
            Instruction::Loop {
                body: vec![
                    Instruction::Receive,
                    Instruction::BreakIf,
                    Instruction::Inference { input: Inference_Input::Tensor(TENSOR1) },
                    Instruction::Send,
                ],
            },
        ]
    }
}