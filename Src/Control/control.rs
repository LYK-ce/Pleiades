//Presented by KeJi
//Date ： 2026-04-07

//! Control 核心模块 - 调度核心事件循环
//!
//! 通过 `tokio::select!` 同时处理三个事件源：
//! 1. cli_rx: CLI 发来的用户命令
//! 2. inbound_rx: 其他节点发来的网络请求 (InboundRequest)
//! 3. event_rx: 网络事件（连接/断开/文件到达等）
//!
//! ## 功能
//! - 处理 run 命令: 切分→分发→WORK→LOAD→PIPELINE_FLOW→自回归推理→输出
//! - Busy 状态: 处理 Work/Load/Pipeline_Flow 命令，收到 tensor 自动推理转发
//! - 被动接收: 接受文件传输请求，文件保存到 Pleiades_Workspace/

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::path::Path;
use libp2p::PeerId;
use tokio::sync::mpsc;
use tracing::{info, warn, error, debug};
use candle_transformers::generation::{LogitsProcessor, Sampling};

use super::cli::CLI_Command;
use super::command::{Control_Command, Serialize_Command, Deserialize_Command};
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
) {
    let mut state = Node_State::Idle;
    let mut next_peer: Option<PeerId> = None;

    info!("Control 层事件循环已启动, 节点状态: {:?}, 设备: {}", state, device);

    loop {
        tokio::select! {
            // 1. CLI 命令
            cmd = cli_rx.recv() => {
                match cmd {
                    Some(CLI_Command::Run { model_path, prompt, reply }) => {
                        info!("Control: 收到 run 命令 (model: {}, prompt: {})",
                            model_path.display(), prompt);
                        // Handle_Run 需要 inbound_rx 来接收流水线中返回的 logits
                        let result = Handle_Run(
                            &mut state,
                            &mut next_peer,
                            &ml_service,
                            &node_handle,
                            &device,
                            &model_path,
                            &prompt,
                            &mut inbound_rx,
                        ).await;
                        // 任务完成后恢复 Idle 状态
                        state = Node_State::Idle;
                        next_peer = None;
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
                        Handle_Event(event);
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
    println!("[Pleiades] 发现 {} 台设备 (含自己)", total_devices);
    println!("[Pleiades] 本机: {}", node_handle.Get_Local_Peer_Id());
    for (i, peer) in peers.iter().enumerate() {
        println!("[Pleiades] 节点 {}: {}", i + 1, peer);
    }

    // Step 2: 分析模型
    let arch_info = ml_service
        .Analyze_Model(model_path)
        .await
        .map_err(|e| format!("模型分析失败: {}", e))?;

    let total_layers = arch_info.num_layers + 2;
    let eos_token_id = arch_info.eos_token_id;
    println!(
        "[Pleiades] 模型: {}, 总层数: {}, EOS: {}",
        arch_info.architecture, total_layers, eos_token_id
    );

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

    println!("[Pleiades] 分配方案:");
    println!("  本机: 层 {}-{}", assignments[0].0, assignments[0].1);
    for (i, peer) in peers.iter().enumerate() {
        let (s, e) = assignments[i + 1];
        println!("  节点 {} ({}): 层 {}-{}", i + 1, peer, s, e);
    }

    // Step 4: 切分模型
    let output_dir = Path::new("Pleiades_Workspace");
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    let model_stem = model_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();

    // 只为其他节点（peers）切分模型，协调者自己直接从原始模型加载
    if !peers.is_empty() {
        println!("[Pleiades] 正在为其他节点切分模型...");
        for (i, (start, end)) in assignments[1..].iter().enumerate() {
            println!("  切分第 {} 段: 层 {}-{}", i + 1, start, end);
            ml_service
                .Split_Model(model_path, *start, *end, output_dir)
                .await
                .map_err(|e| format!("模型切分失败 (层 {}-{}): {}", start, end, e))?;
        }
        println!("[Pleiades] ✓ 模型切分完成 ({} 个文件)", peers.len());
    }

    // Step 5: 分发文件给其他节点
    if !peers.is_empty() {
        println!("[Pleiades] 正在向其他节点发送模型文件...");
        for (i, peer) in peers.iter().enumerate() {
            let (start, end) = assignments[i + 1];
            let split_file_name = format!("{}_split_{}_{}.pgguf", model_stem, start, end);
            let split_file_path = output_dir.join(&split_file_name);

            println!("  发送 {} → 节点 {}", split_file_name, peer);
            node_handle
                .Send_File(peer, split_file_path)
                .await
                .map_err(|e| format!("文件发送失败 (节点 {}): {}", peer, e))?;
            println!("  ✓ 已发送");
        }
        println!("[Pleiades] ✓ 所有模型文件已发送");
    }

    // ================================================================
    // Phase 2: 发送 WORK → LOAD → PIPELINE_FLOW → 加载本机模型
    // ================================================================

    if !peers.is_empty() {
        // Step 6: 发送 WORK 命令
        println!("[Pleiades] 发送 WORK 命令给所有节点...");
        for peer in &peers {
            let cmd_bytes = Serialize_Command(&Control_Command::Work);
            node_handle
                .Send_Bytes(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 WORK 失败 ({}): {}", peer, e))?;
        }
        println!("[Pleiades] ✓ 所有节点已进入 Busy 状态");

        // Step 7: 发送 LOAD 命令
        println!("[Pleiades] 发送 LOAD 命令给所有节点...");
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
            println!("  ✓ 节点 {} 已加载模型", peer);
        }
        println!("[Pleiades] ✓ 所有节点模型已加载");

        // Step 8: 发送 PIPELINE_FLOW 命令（建立链条）
        // 链条: self → peers[0] → peers[1] → ... → peers[N-1] → self
        println!("[Pleiades] 配置流水线...");
        let local_peer = node_handle.Get_Local_Peer_Id();

        for (i, peer) in peers.iter().enumerate() {
            let target = if i + 1 < peers.len() {
                // 中间节点：发给下一个 peer
                peers[i + 1]
            } else {
                // 最后一个 peer：发回给协调者
                local_peer
            };

            let cmd_bytes = Serialize_Command(&Control_Command::Pipeline_Flow {
                next_peer: target,
            });
            node_handle
                .Send_Bytes(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 PIPELINE_FLOW 失败 ({}): {}", peer, e))?;
            println!("  节点 {} → next: {}", peer, target);
        }

        // 协调者自己的 next_peer 是第一个 peer
        *next_peer = Some(peers[0]);
        println!("  本机 → next: {}", peers[0]);
        println!("[Pleiades] ✓ 流水线配置完成");
    }

    // Step 9: 加载本机模型（直接从原始完整模型加载，保留 tokenizer）
    let (my_start, my_end) = assignments[0];

    println!("[Pleiades] 加载本机模型 (层 {}-{}, 从原始模型)...", my_start, my_end);
    let my_load_info = ml_service
        .Load_Model(model_path, my_start, my_end, device)
        .await
        .map_err(|e| format!("本机模型加载失败: {}", e))?;
    println!(
        "[Pleiades] ✓ 本机模型已加载 (input_head: {}, output_head: {}, tokenizer: {})",
        my_load_info.has_input_head, my_load_info.has_output_head, my_load_info.has_tokenizer
    );

    // 设置本机状态为 Busy
    *state = Node_State::Busy;

    // ================================================================
    // Phase 3: Encode → 自回归推理循环 → Decode
    // ================================================================

    // Step 10: Encode prompt
    println!("[Pleiades] 编码 prompt: \"{}\"", prompt);
    let token_ids = ml_service
        .Encode(prompt)
        .await
        .map_err(|e| format!("Encode 失败: {}", e))?;
    println!("[Pleiades] Token IDs: {} 个", token_ids.len());

    // Step 11: 自回归推理循环
    println!("[Pleiades] 开始推理 (最多 {} 轮)...", MAX_GENERATION_ROUNDS);
    let start_time = std::time::Instant::now();

    let sampling = Sampling::All { temperature: TEMPERATURE };
    let mut logits_processor = LogitsProcessor::from_sampling(SEED, sampling);
    let mut all_generated_tokens: Vec<u32> = Vec::new();

    // 确定最后一个节点（流水线末尾节点，发 logits 回来的那个）
    let last_peer = if !peers.is_empty() {
        Some(*peers.last().unwrap())
    } else {
        None
    };

    // ---- Prefill: 处理完整 prompt ----
    let input_bytes = Token_Ids_To_Bytes(&token_ids);
    let prefill_start = std::time::Instant::now();

    let logits = if peers.is_empty() {
        // 单机模式：直接本地推理
        ml_service.Inference(input_bytes, 0).await
            .map_err(|e| format!("Prefill 推理失败: {}", e))?
    } else {
        // 多机模式：本机推理 → 发给 next_peer → 等最后一个节点返回 logits
        let local_result = ml_service.Inference(input_bytes, 0).await
            .map_err(|e| format!("本机 Prefill 推理失败: {}", e))?;

        // 序列化结果 tensor 并发送
        let tensor_bytes = Serialize_Tensor_Output(&local_result)
            .map_err(|e| format!("Tensor 序列化失败: {}", e))?;

        let mut payload = Vec::with_capacity(8 + tensor_bytes.len());
        payload.extend_from_slice(&(0u64).to_le_bytes());
        payload.extend_from_slice(&tensor_bytes);

        node_handle
            .Send_Bytes(next_peer.as_ref().unwrap(), DataType::Data, payload)
            .await
            .map_err(|e| format!("Prefill 发送失败: {}", e))?;

        // 等待最后一个节点（last_peer）返回 logits
        Wait_For_Pipeline_Result(node_handle, inbound_rx, last_peer.unwrap()).await
            .map_err(|e| format!("等待 Prefill 结果失败: {}", e))?
    };

    let prefill_time = prefill_start.elapsed();
    println!("[Pleiades] Prefill 完成 ({:.3}s)", prefill_time.as_secs_f64());

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
            println!("[Pleiades] 在第 {} 轮遇到 EOS，停止生成", round);
            break;
        }

        let offset = token_ids.len() + round;
        let step_bytes = Token_Ids_To_Bytes(&[next_token]);

        let logits = if peers.is_empty() {
            // 单机模式
            ml_service.Inference(step_bytes, offset).await
                .map_err(|e| format!("第 {} 轮推理失败: {}", round, e))?
        } else {
            // 多机模式
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

    println!(
        "[Pleiades] 生成完成: {} tokens, decode {:.3}s ({:.1} tok/s), total {:.3}s",
        gen_count,
        decode_time.as_secs_f64(),
        gen_count as f64 / decode_time.as_secs_f64(),
        total_time.as_secs_f64()
    );

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

    println!("\n========== 推理结果 ==========");
    println!("{}", result_text);
    println!("==============================\n");

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
///
/// 从 inbound_rx 接收，过滤出来自 last_peer 的 DataType::Data 消息，
/// 解析出 logits Tensor 并返回。其他消息按默认处理。
async fn Wait_For_Pipeline_Result(
    node_handle: &NodeHandle,
    inbound_rx: &mut mpsc::Receiver<InboundRequest>,
    last_peer: PeerId,
) -> Result<candle_core::Tensor, String> {
    loop {
        let req = inbound_rx.recv().await
            .ok_or_else(|| "入站请求通道已关闭".to_string())?;

        if req.data_type == DataType::Data && req.peer == last_peer {
            // 这是我们等的结果！先 ACK
            if let Err(e) = node_handle
                .Send_Reply(req.request_id, DataType::Data, b"ACK".to_vec())
                .await
            {
                error!("ACK 失败: {}", e);
            }

            // 解析 [8B offset][tensor_bytes]
            if req.payload.len() < 8 {
                return Err("Pipeline 结果 payload 过短".to_string());
            }
            let tensor_bytes = &req.payload[8..];

            // 反序列化 Tensor
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

        // 非目标消息，按常规处理
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
) {
    info!(
        "Control: 收到入站请求 (id={}, peer={}, type={:?}, payload_len={}, state={:?})",
        req.request_id, req.peer, req.data_type, req.payload.len(), state
    );

    match req.data_type {
        // ===== 文件传输：始终接受 =====
        DataType::File => {
            println!(
                "[Pleiades] 收到文件传输请求 (来自 {}), 自动接受",
                req.peer
            );
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
                        req.request_id, req.peer, cmd,
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

            // 先 ACK
            if let Err(e) = node_handle
                .Send_Reply(req.request_id, DataType::Data, b"ACK".to_vec())
                .await
            {
                error!("发送 ACK 失败: {}", e);
                return;
            }

            // 解析 [8B offset][tensor_bytes]
            if req.payload.len() < 8 {
                error!("Data payload 过短");
                return;
            }
            let offset_bytes: [u8; 8] = req.payload[0..8].try_into().unwrap();
            let offset = u64::from_le_bytes(offset_bytes) as usize;
            let tensor_bytes = &req.payload[8..];

            debug!("Busy: 收到 tensor (offset={}, {} bytes)", offset, tensor_bytes.len());

            // 执行推理
            let result_tensor = match ml_service.Inference(tensor_bytes.to_vec(), offset).await {
                Ok(tensor) => tensor,
                Err(e) => {
                    error!("推理失败: {}", e);
                    return;
                }
            };

            // 序列化结果
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

            // 转发给 next_peer
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
) {
    match cmd {
        Control_Command::Work => {
            *state = Node_State::Busy;
            println!("[Pleiades] WORK (来自 {}): 状态 → Busy", peer);
            let _ = node_handle
                .Send_Reply(request_id, DataType::Command, b"OK".to_vec())
                .await;
        }

        Control_Command::Load { model_path, start, end } => {
            println!("[Pleiades] LOAD (来自 {}): {} 层 {}-{}", peer, model_path, start, end);
            let full_path = std::path::Path::new(&model_path);
            match ml_service.Load_Model(full_path, start, end, device).await {
                Ok(info) => {
                    println!(
                        "[Pleiades] ✓ 模型加载完成 (arch: {}, input: {}, output: {})",
                        info.architecture, info.has_input_head, info.has_output_head
                    );
                    let _ = node_handle
                        .Send_Reply(request_id, DataType::Command, b"OK".to_vec())
                        .await;
                }
                Err(e) => {
                    error!("模型加载失败: {}", e);
                    let _ = node_handle
                        .Send_Reply(request_id, DataType::Command, format!("ERROR: {}", e).into_bytes())
                        .await;
                }
            }
        }

        Control_Command::Pipeline_Flow { next_peer: target } => {
            *next_peer = Some(target);
            println!("[Pleiades] PIPELINE_FLOW (来自 {}): next → {}", peer, target);
            let _ = node_handle
                .Send_Reply(request_id, DataType::Command, b"OK".to_vec())
                .await;
        }
    }
}

// ============================================================
// 网络事件处理
// ============================================================

fn Handle_Event(event: NetworkEvent) {
    match event {
        NetworkEvent::PeerDiscovered(peer) => {
            println!("[Pleiades] 发现节点: {}", peer);
        }
        NetworkEvent::PeerLeft(peer) => {
            println!("[Pleiades] 节点离开: {}", peer);
        }
        NetworkEvent::ConnectionEstablished(peer) => {
            println!("[Pleiades] 连接建立: {}", peer);
        }
        NetworkEvent::ConnectionClosed(peer) => {
            println!("[Pleiades] 连接断开: {}", peer);
        }
        NetworkEvent::FileStreamReceived { peer, file_path } => {
            println!("[Pleiades] 收到文件: {} (来自 {})", file_path.display(), peer);
        }
        NetworkEvent::FileStreamError { peer, error } => {
            eprintln!("[Pleiades] 文件传输错误: {} (节点 {})", error, peer);
        }
        NetworkEvent::RecordFound { key, value } => {
            info!("DHT 记录查询成功: key={} bytes, value={} bytes", key.len(), value.len());
        }
        NetworkEvent::RecordNotFound { key } => {
            info!("DHT 记录未找到: key={} bytes", key.len());
        }
    }
}
