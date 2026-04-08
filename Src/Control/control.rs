//Presented by KeJi
//Date ： 2026-04-08

//! Control 核心模块 - 调度核心事件循环
//!
//! 通过 `tokio::select!` 同时处理三个事件源：
//! 1. cli_rx: CLI/TUI 发来的用户命令
//! 2. inbound_rx: 其他节点发来的网络请求 (InboundRequest)
//! 3. event_rx: 网络事件（连接/断开/文件到达等）
//!
//! 所有面向用户的输出通过 ui_tx 发送 Ui_Message 给 TUI 渲染，
//! 不再使用 println! 直接输出到终端。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::path::Path;
use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::{info, warn, error, debug};
use candle_transformers::generation::{LogitsProcessor, Sampling};

use super::cli::CLI_Command;
use super::command::{Control_Command, Serialize_Command, Deserialize_Command};
use super::ui_message::Ui_Message;
use crate::ml_engine::gguf_model::Token_Ids_To_Bytes;
use crate::ml_engine::ml_inference_service::ML_Service_Handle;
use crate::ml_engine::gguf_tensor::{
    GGUF_Tensor_Packet, GGUF_Tensor_Serialize, GGUF_Tensor_Deserialize, GGUF_Dtype,
};
use crate::network::data_protocol::DataType;
use crate::network::node::NetworkEvent;
use crate::network::node_handle::{InboundRequest, NodeHandle};

/// 自回归生成最大轮数
const MAX_GENERATION_ROUNDS: usize = 120;
/// 采样温度
const TEMPERATURE: f64 = 0.8;
/// 随机种子
const SEED: u64 = 299792458;

// ============================================================
// 节点状态
// ============================================================

/// 节点当前状态
#[derive(Debug, Clone, PartialEq)]
pub enum Node_State {
    /// 空闲状态，可以接受新任务
    Idle,
    /// 忙碌状态，正在执行推理任务
    Busy,
}

// ============================================================
// 辅助宏：发送 UI 消息（忽略发送失败）
// ============================================================

/// 发送 UI 消息的辅助函数（async 版本）
async fn Send_Ui(ui_tx: &mpsc::Sender<Ui_Message>, msg: Ui_Message) {
    let _ = ui_tx.send(msg).await;
}

// ============================================================
// Control 主事件循环
// ============================================================

/// Control 层主事件循环
pub async fn Control_Loop(
    mut cli_rx: mpsc::Receiver<CLI_Command>,
    mut inbound_rx: mpsc::Receiver<InboundRequest>,
    mut event_rx: mpsc::Receiver<NetworkEvent>,
    ml_service: ML_Service_Handle,
    node_handle: NodeHandle,
    device: String,
    ui_tx: mpsc::Sender<Ui_Message>,
) {
    let mut state = Node_State::Idle;
    let mut next_peer: Option<PeerId> = None;

    info!("Control 层事件循环已启动, 节点状态: {:?}, 设备: {}", state, device);
    Send_Ui(&ui_tx, Ui_Message::Log(format!(
        "节点启动, PeerId: {}", node_handle.Get_Local_Peer_Id()
    ))).await;

    loop {
        tokio::select! {
            // 1. CLI/TUI 命令
            cmd = cli_rx.recv() => {
                match cmd {
                    Some(CLI_Command::Run { model_path, prompt, reply }) => {
                        info!("Control: 收到 run 命令 (model: {}, prompt: {})",
                            model_path.display(), prompt);
                        let result = Handle_Run(
                            &mut state,
                            &mut next_peer,
                            &ml_service,
                            &node_handle,
                            &device,
                            &model_path,
                            &prompt,
                            &mut inbound_rx,
                            &ui_tx,
                        ).await;
                        // 任务完成后恢复 Idle 状态
                        state = Node_State::Idle;
                        next_peer = None;
                        Send_Ui(&ui_tx, Ui_Message::Job_Idle).await;
                        Send_Ui(&ui_tx, Ui_Message::State_Change("Idle".to_string())).await;
                        // 如果失败，把错误发送给 TUI 显示
                        if let Err(ref e) = result {
                            Send_Ui(&ui_tx, Ui_Message::Error(e.clone())).await;
                        }
                        let _ = reply.send(result);
                    }
                    Some(CLI_Command::Quit) => {
                        info!("Control: 收到退出命令，正在关闭...");
                        if let Err(e) = ml_service.Shutdown().await {
                            warn!("ML Service 关闭失败: {}", e);
                        }
                        if let Err(e) = node_handle.Stop().await {
                            warn!("网络节点关闭失败: {}", e);
                        }
                        break;
                    }
                    None => {
                        info!("Control: CLI 通道关闭，退出");
                        if let Err(e) = ml_service.Shutdown().await {
                            warn!("ML Service 关闭失败: {}", e);
                        }
                        if let Err(e) = node_handle.Stop().await {
                            warn!("网络节点关闭失败: {}", e);
                        }
                        break;
                    }
                }
            }

            // 2. 网络入站请求
            req = inbound_rx.recv() => {
                match req {
                    Some(inbound_req) => {
                        Handle_Inbound(
                            &mut state,
                            &mut next_peer,
                            &ml_service,
                            &node_handle,
                            &device,
                            inbound_req,
                            &ui_tx,
                        ).await;
                    }
                    None => {
                        info!("Control: 入站请求通道关闭");
                    }
                }
            }

            // 3. 网络事件
            evt = event_rx.recv() => {
                match evt {
                    Some(event) => {
                        Handle_Event(event, &ui_tx).await;
                    }
                    None => {
                        info!("Control: 网络事件通道关闭");
                    }
                }
            }
        }
    }

    info!("Control 层事件循环已退出");
}

// ============================================================
// Handle_Run — 完整的分布式推理流程
// ============================================================

/// 处理 run 命令 — 完整的分布式推理流程
///
/// Phase 1: 分析模型 → 切分 → 分发文件
/// Phase 2: 发送 WORK → LOAD → PIPELINE_FLOW 命令 → 加载本机模型
/// Phase 3: Encode → 自回归推理循环(120次) → Decode → 输出结果
async fn Handle_Run(
    state: &mut Node_State,
    next_peer: &mut Option<PeerId>,
    ml_service: &ML_Service_Handle,
    node_handle: &NodeHandle,
    device: &str,
    model_path: &Path,
    prompt: &str,
    inbound_rx: &mut mpsc::Receiver<InboundRequest>,
    ui_tx: &mpsc::Sender<Ui_Message>,
) -> Result<String, String> {
    let mut output = String::new();

    // ================================================================
    // Phase 1: 分析模型 → 切分 → 分发文件
    // ================================================================

    // Step 1: 查询网络中的节点
    let peers = node_handle
        .Get_Peers()
        .await
        .map_err(|e| format!("查询节点列表失败: {}", e))?;

    let total_devices = peers.len() + 1;
    Send_Ui(ui_tx, Ui_Message::Log(format!("发现 {} 台设备 (含自己)", total_devices))).await;
    Send_Ui(ui_tx, Ui_Message::Log(format!("本机: {}", node_handle.Get_Local_Peer_Id()))).await;
    for (i, peer) in peers.iter().enumerate() {
        Send_Ui(ui_tx, Ui_Message::Log(format!("节点 {}: {}", i + 1, peer))).await;
    }

    // Step 2: 分析模型
    let arch_info = ml_service
        .Analyze_Model(model_path)
        .await
        .map_err(|e| format!("模型分析失败: {}", e))?;

    let total_layers = arch_info.num_layers + 2;
    let eos_token_id = arch_info.eos_token_id;
    Send_Ui(ui_tx, Ui_Message::Log(format!(
        "模型: {}, 总层数: {}, EOS: {}",
        arch_info.architecture, total_layers, eos_token_id
    ))).await;

    // Step 3: 计算均分方案
    let layers_per_node = total_layers / total_devices;
    let remainder = total_layers % total_devices;

    let mut assignments: Vec<(usize, usize)> = Vec::with_capacity(total_devices);
    let mut current_layer = 0;
    for i in 0..total_devices {
        let count = layers_per_node + if i < remainder { 1 } else { 0 };
        let start = current_layer;
        let end = current_layer + count - 1;
        assignments.push((start, end));
        current_layer += count;
    }

    Send_Ui(ui_tx, Ui_Message::Log(format!("分配方案: 本机 层{}-{}", assignments[0].0, assignments[0].1))).await;
    for (i, peer) in peers.iter().enumerate() {
        let (s, e) = assignments[i + 1];
        Send_Ui(ui_tx, Ui_Message::Log(format!("  节点 {} ({}): 层 {}-{}", i + 1, peer, s, e))).await;
    }

    // 更新 Job 面板
    let model_stem = model_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    Send_Ui(ui_tx, Ui_Message::Job_Inference {
        model_name: model_stem.clone(),
        device_count: total_devices,
        layer_range: format!("层{}-{}", assignments[0].0, assignments[0].1),
        phase: "切分模型".to_string(),
    }).await;

    // Step 4: 切分模型
    let output_dir = Path::new("Pleiades_Workspace");
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    // 只为其他节点（peers）切分模型，协调者自己直接从原始模型加载
    if !peers.is_empty() {
        Send_Ui(ui_tx, Ui_Message::Log("正在为其他节点切分模型...".to_string())).await;
        for (i, (start, end)) in assignments[1..].iter().enumerate() {
            Send_Ui(ui_tx, Ui_Message::Log(format!("  切分第 {} 段: 层 {}-{}", i + 1, start, end))).await;
            ml_service
                .Split_Model(model_path, *start, *end, output_dir)
                .await
                .map_err(|e| format!("模型切分失败 (层 {}-{}): {}", start, end, e))?;
        }
        Send_Ui(ui_tx, Ui_Message::Log(format!("✓ 模型切分完成 ({} 个文件)", peers.len()))).await;
    }

    // Step 5: 分发文件给其他节点
    if !peers.is_empty() {
        Send_Ui(ui_tx, Ui_Message::Job_Inference {
            model_name: model_stem.clone(),
            device_count: total_devices,
            layer_range: format!("层{}-{}", assignments[0].0, assignments[0].1),
            phase: "分发文件".to_string(),
        }).await;

        for (i, peer) in peers.iter().enumerate() {
            let (start, end) = assignments[i + 1];
            let split_file_name = format!("{}_split_{}_{}.pgguf", model_stem, start, end);
            let split_file_path = output_dir.join(&split_file_name);

            Send_Ui(ui_tx, Ui_Message::Log(format!("发送 {} → 节点 {}", split_file_name, peer))).await;
            node_handle
                .Send_File(peer, split_file_path)
                .await
                .map_err(|e| format!("文件发送失败 (节点 {}): {}", peer, e))?;
            Send_Ui(ui_tx, Ui_Message::Log(format!("  ✓ 已发送 → {}", peer))).await;
        }
        Send_Ui(ui_tx, Ui_Message::Log("✓ 所有模型文件已发送".to_string())).await;
    }

    // ================================================================
    // Phase 2: 发送 WORK → LOAD → PIPELINE_FLOW → 加载本机模型
    // ================================================================

    if !peers.is_empty() {
        Send_Ui(ui_tx, Ui_Message::Job_Inference {
            model_name: model_stem.clone(),
            device_count: total_devices,
            layer_range: format!("层{}-{}", assignments[0].0, assignments[0].1),
            phase: "配置流水线".to_string(),
        }).await;

        // Step 6: 发送 WORK 命令
        Send_Ui(ui_tx, Ui_Message::Log("发送 WORK 命令给所有节点...".to_string())).await;
        for peer in &peers {
            let cmd_bytes = Serialize_Command(&Control_Command::Work);
            node_handle
                .Send_Bytes(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 WORK 失败 ({}): {}", peer, e))?;
        }
        Send_Ui(ui_tx, Ui_Message::Log("✓ 所有节点已进入 Busy 状态".to_string())).await;

        // Step 7: 发送 LOAD 命令
        Send_Ui(ui_tx, Ui_Message::Log("发送 LOAD 命令给所有节点...".to_string())).await;
        for (i, peer) in peers.iter().enumerate() {
            let (start, end) = assignments[i + 1];
            let split_file_name = format!("{}_split_{}_{}.pgguf", model_stem, start, end);
            let split_file_path = output_dir.join(&split_file_name);

            let cmd_bytes = Serialize_Command(&Control_Command::Load {
                model_path: split_file_path.to_string_lossy().to_string(),
                start,
                end,
            });
            node_handle
                .Send_Bytes(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 LOAD 失败 ({}): {}", peer, e))?;
            Send_Ui(ui_tx, Ui_Message::Log(format!("  ✓ 节点 {} 已加载模型", peer))).await;
        }
        Send_Ui(ui_tx, Ui_Message::Log("✓ 所有节点模型已加载".to_string())).await;

        // Step 8: 发送 PIPELINE_FLOW 命令（建立链条）
        Send_Ui(ui_tx, Ui_Message::Log("配置流水线...".to_string())).await;
        let local_peer = node_handle.Get_Local_Peer_Id();

        for (i, peer) in peers.iter().enumerate() {
            let target = if i + 1 < peers.len() {
                peers[i + 1]
            } else {
                local_peer
            };

            let cmd_bytes = Serialize_Command(&Control_Command::Pipeline_Flow {
                next_peer: target,
            });
            node_handle
                .Send_Bytes(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 PIPELINE_FLOW 失败 ({}): {}", peer, e))?;
            Send_Ui(ui_tx, Ui_Message::Log(format!("  节点 {} → next: {}", peer, target))).await;
        }

        *next_peer = Some(peers[0]);
        Send_Ui(ui_tx, Ui_Message::Log(format!("  本机 → next: {}", peers[0]))).await;
        Send_Ui(ui_tx, Ui_Message::Log("✓ 流水线配置完成".to_string())).await;
    }

    // Step 9: 加载本机模型
    let (my_start, my_end) = assignments[0];

    Send_Ui(ui_tx, Ui_Message::Job_Inference {
        model_name: model_stem.clone(),
        device_count: total_devices,
        layer_range: format!("层{}-{}", my_start, my_end),
        phase: "加载模型".to_string(),
    }).await;
    Send_Ui(ui_tx, Ui_Message::Log(format!("加载本机模型 (层 {}-{}, 从原始模型)...", my_start, my_end))).await;

    let my_load_info = ml_service
        .Load_Model(model_path, my_start, my_end, device)
        .await
        .map_err(|e| format!("本机模型加载失败: {}", e))?;
    Send_Ui(ui_tx, Ui_Message::Log(format!(
        "✓ 本机模型已加载 (input_head: {}, output_head: {}, tokenizer: {})",
        my_load_info.has_input_head, my_load_info.has_output_head, my_load_info.has_tokenizer
    ))).await;

    // 设置本机状态为 Busy
    *state = Node_State::Busy;
    Send_Ui(ui_tx, Ui_Message::State_Change("Busy".to_string())).await;

    // ================================================================
    // Phase 3: Encode → 自回归推理循环 → Decode
    // ================================================================

    // Step 10: Encode prompt
    Send_Ui(ui_tx, Ui_Message::Job_Inference {
        model_name: model_stem.clone(),
        device_count: total_devices,
        layer_range: format!("层{}-{}", my_start, my_end),
        phase: "Prefill".to_string(),
    }).await;
    Send_Ui(ui_tx, Ui_Message::Log(format!("编码 prompt: \"{}\"", prompt))).await;

    let token_ids = ml_service
        .Encode(prompt)
        .await
        .map_err(|e| format!("Encode 失败: {}", e))?;
    Send_Ui(ui_tx, Ui_Message::Log(format!("Token IDs: {} 个", token_ids.len()))).await;

    // Step 11: 自回归推理循环
    Send_Ui(ui_tx, Ui_Message::Log(format!("开始推理 (最多 {} 轮)...", MAX_GENERATION_ROUNDS))).await;
    let start_time = std::time::Instant::now();

    let sampling = Sampling::All { temperature: TEMPERATURE };
    let mut logits_processor = LogitsProcessor::from_sampling(SEED, sampling);
    let mut all_generated_tokens: Vec<u32> = Vec::new();

    let last_peer = if !peers.is_empty() {
        Some(*peers.last().unwrap())
    } else {
        None
    };

    // ---- Prefill: 处理完整 prompt ----
    let input_bytes = Token_Ids_To_Bytes(&token_ids);
    let prefill_start = std::time::Instant::now();

    let logits = if peers.is_empty() {
        ml_service.Inference(input_bytes, 0).await
            .map_err(|e| format!("Prefill 推理失败: {}", e))?
    } else {
        let local_result = ml_service.Inference(input_bytes, 0).await
            .map_err(|e| format!("本机 Prefill 推理失败: {}", e))?;

        let tensor_bytes = Serialize_Tensor_Output(&local_result)
            .map_err(|e| format!("Tensor 序列化失败: {}", e))?;

        let mut payload = Vec::with_capacity(8 + tensor_bytes.len());
        payload.extend_from_slice(&(0u64).to_le_bytes());
        payload.extend_from_slice(&tensor_bytes);

        node_handle
            .Send_Bytes(next_peer.as_ref().unwrap(), DataType::Data, payload)
            .await
            .map_err(|e| format!("Prefill 发送失败: {}", e))?;

        Wait_For_Pipeline_Result(node_handle, inbound_rx, last_peer.unwrap()).await
            .map_err(|e| format!("等待 Prefill 结果失败: {}", e))?
    };

    let prefill_time = prefill_start.elapsed();
    Send_Ui(ui_tx, Ui_Message::Log(format!("Prefill 完成 ({:.3}s)", prefill_time.as_secs_f64()))).await;

    // 更新 Job 为 Decode 阶段
    Send_Ui(ui_tx, Ui_Message::Job_Inference {
        model_name: model_stem.clone(),
        device_count: total_devices,
        layer_range: format!("层{}-{}", my_start, my_end),
        phase: "Decode".to_string(),
    }).await;

    // 采样第一个 token
    let logits = logits.squeeze(0)
        .map_err(|e| format!("Squeeze 失败: {}", e))?;
    let mut next_token = logits_processor.sample(&logits)
        .map_err(|e| format!("采样失败: {}", e))?;
    all_generated_tokens.push(next_token);

    // ---- Auto-regressive 生成 ----
    let decode_start = std::time::Instant::now();

    for round in 0..MAX_GENERATION_ROUNDS {
        if next_token == eos_token_id {
            Send_Ui(ui_tx, Ui_Message::Log(format!("在第 {} 轮遇到 EOS，停止生成", round))).await;
            break;
        }

        let offset = token_ids.len() + round;
        let step_bytes = Token_Ids_To_Bytes(&[next_token]);

        let logits = if peers.is_empty() {
            ml_service.Inference(step_bytes, offset).await
                .map_err(|e| format!("第 {} 轮推理失败: {}", round, e))?
        } else {
            let local_result = ml_service.Inference(step_bytes, offset).await
                .map_err(|e| format!("第 {} 轮本机推理失败: {}", round, e))?;

            let tensor_bytes = Serialize_Tensor_Output(&local_result)
                .map_err(|e| format!("Tensor 序列化失败: {}", e))?;

            let mut payload = Vec::with_capacity(8 + tensor_bytes.len());
            payload.extend_from_slice(&(offset as u64).to_le_bytes());
            payload.extend_from_slice(&tensor_bytes);

            node_handle
                .Send_Bytes(next_peer.as_ref().unwrap(), DataType::Data, payload)
                .await
                .map_err(|e| format!("第 {} 轮发送失败: {}", round, e))?;

            Wait_For_Pipeline_Result(node_handle, inbound_rx, last_peer.unwrap()).await
                .map_err(|e| format!("第 {} 轮等待结果失败: {}", round, e))?
        };

        let logits = logits.squeeze(0)
            .map_err(|e| format!("Squeeze 失败: {}", e))?;
        next_token = logits_processor.sample(&logits)
            .map_err(|e| format!("采样失败: {}", e))?;
        all_generated_tokens.push(next_token);
    }

    let decode_time = decode_start.elapsed();
    let total_time = start_time.elapsed();
    let gen_count = all_generated_tokens.len();
    let tok_per_sec = gen_count as f64 / decode_time.as_secs_f64();

    Send_Ui(ui_tx, Ui_Message::Log(format!(
        "生成完成: {} tokens, decode {:.3}s ({:.1} tok/s), total {:.3}s",
        gen_count, decode_time.as_secs_f64(), tok_per_sec, total_time.as_secs_f64()
    ))).await;

    // Step 12: Decode
    let all_tokens: Vec<u32> = token_ids
        .iter()
        .chain(all_generated_tokens.iter())
        .cloned()
        .collect();

    let result_text = ml_service
        .Decode(&all_tokens)
        .await
        .map_err(|e| format!("Decode 失败: {}", e))?;

    // 发送推理完成消息
    Send_Ui(ui_tx, Ui_Message::Inference_Complete {
        text: result_text.clone(),
        tokens: gen_count,
        tok_per_sec,
        total_secs: total_time.as_secs_f64(),
    }).await;

    Send_Ui(ui_tx, Ui_Message::State_Change("Idle".to_string())).await;

    output.push_str("[Pleiades] ✓ 推理完成\n");
    output.push_str(&result_text);

    Ok(output)
}

// ============================================================
// 辅助函数
// ============================================================

/// 将推理输出 Tensor 序列化为 GGUF_Tensor_Packet 字节流
fn Serialize_Tensor_Output(tensor: &candle_core::Tensor) -> Result<Vec<u8>, String> {
    let shape = tensor.dims().to_vec();
    let flat = tensor.flatten_all()
        .map_err(|e| format!("Tensor flatten 失败: {}", e))?;
    let f32_data: Vec<f32> = flat.to_vec1::<f32>()
        .map_err(|e| format!("Tensor 转 f32 失败: {}", e))?;
    let raw_bytes: Vec<u8> = f32_data.iter().flat_map(|f| f.to_le_bytes()).collect();

    let packet = GGUF_Tensor_Packet {
        name: "hidden_state".to_string(),
        shape,
        dtype: GGUF_Dtype::F32,
        data: raw_bytes,
    };

    GGUF_Tensor_Serialize(&packet)
        .map_err(|e| format!("Tensor 序列化失败: {}", e))
}

/// 等待流水线最后一个节点返回推理结果
async fn Wait_For_Pipeline_Result(
    node_handle: &NodeHandle,
    inbound_rx: &mut mpsc::Receiver<InboundRequest>,
    last_peer: PeerId,
) -> Result<candle_core::Tensor, String> {
    loop {
        let req = inbound_rx.recv().await
            .ok_or_else(|| "入站请求通道已关闭".to_string())?;

        if req.data_type == DataType::Data && req.peer == last_peer {
            if let Err(e) = node_handle
                .Send_Reply(req.request_id, DataType::Data, b"ACK".to_vec())
                .await
            {
                error!("ACK 失败: {}", e);
            }

            if req.payload.len() < 8 {
                return Err("Pipeline 结果 payload 过短".to_string());
            }
            let tensor_bytes = &req.payload[8..];

            let packet = GGUF_Tensor_Deserialize(tensor_bytes)
                .map_err(|e| format!("结果 Tensor 反序列化失败: {}", e))?;
            let f32_data: Vec<f32> = packet.data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            let tensor = candle_core::Tensor::new(&f32_data[..], &candle_core::Device::Cpu)
                .map_err(|e| format!("Tensor 重建失败: {}", e))?
                .reshape(&*packet.shape)
                .map_err(|e| format!("Tensor reshape 失败: {}", e))?;

            return Ok(tensor);
        }

        if req.data_type == DataType::File {
            let _ = node_handle
                .Send_Reply(req.request_id, DataType::Command, b"ACCEPT".to_vec())
                .await;
        } else {
            let _ = node_handle
                .Send_Reply(req.request_id, DataType::Command, b"OK".to_vec())
                .await;
        }
        debug!(
            "Wait_For_Pipeline_Result: 跳过非目标消息 (peer={}, type={:?})",
            req.peer, req.data_type
        );
    }
}

// ============================================================
// 入站请求处理（被动模式）
// ============================================================

/// 处理入站请求（其他节点发来的）
async fn Handle_Inbound(
    state: &mut Node_State,
    next_peer: &mut Option<PeerId>,
    ml_service: &ML_Service_Handle,
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
            Send_Ui(ui_tx, Ui_Message::Log(format!(
                "收到文件传输请求 (来自 {}), 自动接受", req.peer
            ))).await;
            if let Err(e) = node_handle
                .Send_Reply(req.request_id, DataType::Command, b"ACCEPT".to_vec())
                .await
            {
                error!("发送文件接受回复失败: {}", e);
            }
        }

        // ===== 命令处理 =====
        DataType::Command => {
            match Deserialize_Command(&req.payload) {
                Ok(cmd) => {
                    Handle_Control_Command(
                        state, next_peer, ml_service, node_handle, device,
                        req.request_id, req.peer, cmd, ui_tx,
                    ).await;
                }
                Err(e) => {
                    warn!("命令解析失败: {}", e);
                    if let Err(e) = node_handle
                        .Send_Reply(req.request_id, DataType::Command, b"OK".to_vec())
                        .await
                    {
                        error!("发送回复失败: {}", e);
                    }
                }
            }
        }

        // ===== 数据处理：Busy 状态下推理转发 =====
        DataType::Data => {
            if *state != Node_State::Busy {
                warn!("收到 Data 但当前不在 Busy 状态, 忽略");
                let _ = node_handle
                    .Send_Reply(req.request_id, DataType::Data, b"NOT_BUSY".to_vec())
                    .await;
                return;
            }

            if let Err(e) = node_handle
                .Send_Reply(req.request_id, DataType::Data, b"ACK".to_vec())
                .await
            {
                error!("发送 ACK 失败: {}", e);
                return;
            }

            if req.payload.len() < 8 {
                error!("Data payload 过短");
                return;
            }
            let offset_bytes: [u8; 8] = req.payload[0..8].try_into().unwrap();
            let offset = u64::from_le_bytes(offset_bytes) as usize;
            let tensor_bytes = &req.payload[8..];

            debug!("Busy: 收到 tensor (offset={}, {} bytes)", offset, tensor_bytes.len());

            let result_tensor = match ml_service.Inference(tensor_bytes.to_vec(), offset).await {
                Ok(tensor) => tensor,
                Err(e) => {
                    error!("推理失败: {}", e);
                    return;
                }
            };

            let serialized = match Serialize_Tensor_Output(&result_tensor) {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!("Tensor 序列化失败: {}", e);
                    return;
                }
            };

            let mut data_payload = Vec::with_capacity(8 + serialized.len());
            data_payload.extend_from_slice(&(offset as u64).to_le_bytes());
            data_payload.extend_from_slice(&serialized);

            match next_peer {
                Some(target) => {
                    debug!("Busy: 转发到 {} ({} bytes)", target, data_payload.len());
                    if let Err(e) = node_handle
                        .Send_Bytes(target, DataType::Data, data_payload)
                        .await
                    {
                        error!("转发失败: {}", e);
                    }
                }
                None => {
                    error!("Busy: next_peer 未设置");
                }
            }
        }
    }
}

/// 处理 Control_Command
async fn Handle_Control_Command(
    state: &mut Node_State,
    next_peer: &mut Option<PeerId>,
    ml_service: &ML_Service_Handle,
    node_handle: &NodeHandle,
    device: &str,
    request_id: u64,
    peer: PeerId,
    cmd: Control_Command,
    ui_tx: &mpsc::Sender<Ui_Message>,
) {
    match cmd {
        Control_Command::Work => {
            *state = Node_State::Busy;
            Send_Ui(ui_tx, Ui_Message::Log(format!("WORK (来自 {}): 状态 → Busy", peer))).await;
            Send_Ui(ui_tx, Ui_Message::State_Change("Busy".to_string())).await;
            let _ = node_handle
                .Send_Reply(request_id, DataType::Command, b"OK".to_vec())
                .await;
        }

        Control_Command::Load { model_path, start, end } => {
            Send_Ui(ui_tx, Ui_Message::Log(format!("LOAD (来自 {}): {} 层 {}-{}", peer, model_path, start, end))).await;
            Send_Ui(ui_tx, Ui_Message::Job_Inference {
                model_name: model_path.clone(),
                device_count: 1,
                layer_range: format!("层{}-{}", start, end),
                phase: "加载模型".to_string(),
            }).await;

            let full_path = std::path::Path::new(&model_path);
            match ml_service.Load_Model(full_path, start, end, device).await {
                Ok(info) => {
                    Send_Ui(ui_tx, Ui_Message::Log(format!(
                        "✓ 模型加载完成 (arch: {}, input: {}, output: {})",
                        info.architecture, info.has_input_head, info.has_output_head
                    ))).await;
                    let _ = node_handle
                        .Send_Reply(request_id, DataType::Command, b"OK".to_vec())
                        .await;
                }
                Err(e) => {
                    error!("模型加载失败: {}", e);
                    Send_Ui(ui_tx, Ui_Message::Error(format!("模型加载失败: {}", e))).await;
                    let _ = node_handle
                        .Send_Reply(request_id, DataType::Command, format!("ERROR: {}", e).into_bytes())
                        .await;
                }
            }
        }

        Control_Command::Pipeline_Flow { next_peer: target } => {
            *next_peer = Some(target);
            Send_Ui(ui_tx, Ui_Message::Log(format!("PIPELINE_FLOW (来自 {}): next → {}", peer, target))).await;
            Send_Ui(ui_tx, Ui_Message::Job_Inference {
                model_name: String::new(),
                device_count: 1,
                layer_range: String::new(),
                phase: "就绪，等待推理数据".to_string(),
            }).await;
            let _ = node_handle
                .Send_Reply(request_id, DataType::Command, b"OK".to_vec())
                .await;
        }
    }
}

// ============================================================
// 网络事件处理
// ============================================================

async fn Handle_Event(event: NetworkEvent, ui_tx: &mpsc::Sender<Ui_Message>) {
    match event {
        NetworkEvent::PeerDiscovered(peer) => {
            Send_Ui(ui_tx, Ui_Message::Peer_Discovered(peer.to_string())).await;
        }
        NetworkEvent::PeerLeft(peer) => {
            Send_Ui(ui_tx, Ui_Message::Peer_Left(peer.to_string())).await;
        }
        NetworkEvent::ConnectionEstablished(peer) => {
            Send_Ui(ui_tx, Ui_Message::Connection_Established(peer.to_string())).await;
        }
        NetworkEvent::ConnectionClosed(peer) => {
            Send_Ui(ui_tx, Ui_Message::Connection_Closed(peer.to_string())).await;
        }
        NetworkEvent::FileStreamReceived { peer, file_path } => {
            Send_Ui(ui_tx, Ui_Message::Log(format!(
                "收到文件: {} (来自 {})", file_path.display(), peer
            ))).await;
        }
        NetworkEvent::FileStreamError { peer, error } => {
            Send_Ui(ui_tx, Ui_Message::Error(format!(
                "文件传输错误: {} (节点 {})", error, peer
            ))).await;
        }
        NetworkEvent::RecordFound { key, value } => {
            info!("DHT 记录查询成功: key={} bytes, value={} bytes", key.len(), value.len());
        }
        NetworkEvent::RecordNotFound { key } => {
            info!("DHT 记录未找到: key={} bytes", key.len());
        }
    }
}
