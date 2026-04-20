//Presented by KeJi
//Date ： 2026-04-17

//! Control 核心模块 - 状态机 + 单一事件循环
//!
//! 通过一个 `loop { tokio::select! { ... } }` 同时处理五个事件源：
//! 1. cli_rx: CLI/TUI 发来的用户命令
//! 2. inbound_rx: 其他节点发来的网络请求 (InboundRequest) — **始终活跃**
//! 3. event_rx: 网络事件（连接/断开/文件到达等） — **始终活跃**
//! 4. run_fut: 推理 Future（仅在推理状态时活跃）
//! 5. engine_output_rx: Engine 流式输出（仅在推理状态时活跃）
//!
//! ## 状态机
//! - `is_inferring == false`: 空闲状态，接受 Run/SetDevice 等命令
//! - `is_inferring == true`: 推理状态，Input 转发给 Session，Run/SetDevice 被拒绝
//!
//! ## 关键改进
//! - **单一事件循环**: 不再有嵌套 `loop + select!`
//! - **inbound_rx 始终响应**: 推理期间仍能处理入站网络请求
//! - **event_rx 始终响应**: 推理期间仍能处理网络事件

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use libp2p::PeerId;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn, error, debug};

use super::cli_command::CLI_Command;
use super::network_control_command::{Control_Command, Serialize_Command, Deserialize_Command};
use super::ui_message::Ui_Message;
use crate::config::Update_Config;
use crate::ml_engine::ml_inference_service::{Create_Session, Split_Model, Analyze_Model};
use crate::ml_engine::ml_thread_engine::Session_Handle;
use crate::ml_engine::ml_thread_engine_instruction::{
    Instruction, Inference_Input, Set_Target, Pipeline_Params, Pipeline_Result,
    Engine_Input, Engine_Output,
};
use crate::ml_engine::ml_thread_register::{
    TOKENID2, TOKENID3, TENSOR1, TENSOR2, META1, META2, META5,
};
use crate::network::data_protocol::DataType;
use crate::network::network_service::NetworkEvent;
use crate::network::node_handle::{InboundRequest, NodeHandle};
use crate::network::tensor_stream_manager::Tensor_IO_Handle;
use crate::peer_management::PeerHandle;

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
// 推理上下文
// ============================================================

/// 推理上下文 — Setup_Inference 完成后返回，由主循环状态机消费
pub struct Inference_Context {
    /// Session 句柄（用于 Send_Input / Shutdown）
    pub session_handle: Session_Handle,
    /// Engine 流式输出接收端
    pub output_data_rx: mpsc::Receiver<Engine_Output>,
    /// 取消标志
    pub cancel_flag: Arc<AtomicBool>,
    /// 推理指令程序
    pub program: Vec<Instruction>,
    /// 推理参数
    pub params: Pipeline_Params,
    /// 参与推理的远程节点列表
    pub peers: Vec<PeerId>,
    /// 模型名称（不含扩展名）
    pub model_stem: String,
    /// 本机负责的层范围 (start, end)
    pub my_range: (usize, usize),
    /// 参与推理的总设备数
    pub total_devices: usize,
}

// ============================================================
// 辅助函数：发送 UI 消息
// ============================================================

/// 发送 UI 消息的辅助函数（async 版本）
async fn Send_Ui(ui_tx: &mpsc::Sender<Ui_Message>, msg: Ui_Message) {
    let _ = ui_tx.send(msg).await;
}

// ============================================================
// Control 主事件循环（状态机 + 单一 select!）
// ============================================================

/// Control 层主事件循环
///
/// 使用状态机驱动：`is_inferring` 标志区分空闲/推理状态。
/// 推理相关的 `run_fut` 和 `engine_output_rx` 仅在推理状态时参与 select!。
/// `inbound_rx` 和 `event_rx` **始终活跃**，不会被推理阻塞。
pub async fn Control_Loop(
    mut cli_rx: mpsc::Receiver<CLI_Command>,
    mut inbound_rx: mpsc::Receiver<InboundRequest>,
    mut event_rx: mpsc::Receiver<NetworkEvent>,
    node_handle: NodeHandle,
    peer_handle: PeerHandle,
    mut device: String,
    config_path: PathBuf,
    ui_tx: mpsc::Sender<Ui_Message>,
) {
    let mut state = Node_State::Idle;
    let mut next_peer: Option<PeerId> = None;
    // Worker 收到 Load 命令后暂存参数，等 Pipeline_Flow 时再创建 Session
    let mut pending_load: Option<(String, usize, usize)> = None;

    // ── 推理状态变量（仅在 is_inferring == true 时有效）──
    let mut is_inferring = false;
    let mut run_fut: Pin<Box<dyn Future<Output = Result<Pipeline_Result, String>> + Send>>
        = Box::pin(std::future::pending());
    // Engine 输出接收端：使用 closed channel 作为初始值（recv 立刻返回 None）
    let (_dummy_tx, dummy_rx) = mpsc::channel::<Engine_Output>(1);
    drop(_dummy_tx);
    let mut engine_output_rx: mpsc::Receiver<Engine_Output> = dummy_rx;
    let mut inference_session: Option<Session_Handle> = None;
    let mut inference_cancel: Option<Arc<AtomicBool>> = None;
    let mut inference_reply: Option<oneshot::Sender<Result<String, String>>> = None;
    let mut inference_peers: Vec<PeerId> = Vec::new();
    let mut inference_model_stem = String::new();
    let mut inference_start_time: Option<std::time::Instant> = None;
    let mut inference_my_range: (usize, usize) = (0, 0);
    let mut inference_total_devices: usize = 0;

    info!("Control 层事件循环已启动, 节点状态: {:?}, 设备: {}", state, device);
    Send_Ui(&ui_tx, Ui_Message::Device_Change(device.clone())).await;
    Send_Ui(&ui_tx, Ui_Message::Log(format!(
        "节点启动, PeerId: {}", node_handle.Get_Local_Peer_Id()
    ))).await;

    loop {
        tokio::select! {
            // ══════════════════════════════════════════
            // 1. CLI/TUI 命令
            // ══════════════════════════════════════════
            cmd = cli_rx.recv() => {
                match cmd {
                    // ── Run 命令：启动推理设置 ──
                    Some(CLI_Command::Run { model_path, reply }) => {
                        if is_inferring {
                            let _ = reply.send(Err("当前已有推理会话在运行，请等待完成".to_string()));
                        } else {
                            info!("Control: 收到 run 命令 (model: {})", model_path.display());
                            match Setup_Inference(
                                &mut state, &mut next_peer, &node_handle, &peer_handle,
                                &device, &model_path, &mut event_rx, &ui_tx,
                            ).await {
                                Ok(ctx) => {
                                    // 切换到推理状态
                                    is_inferring = true;
                                    let handle_for_run = ctx.session_handle.clone();
                                    let program = ctx.program;
                                    let params = ctx.params;
                                    let cancel = ctx.cancel_flag.clone();
                                    run_fut = Box::pin(async move {
                                        handle_for_run.Run_Program(program, params, cancel)
                                            .await
                                            .map_err(|e| format!("{}", e))
                                    });
                                    engine_output_rx = ctx.output_data_rx;
                                    inference_session = Some(ctx.session_handle);
                                    inference_cancel = Some(ctx.cancel_flag);
                                    inference_reply = Some(reply);
                                    inference_peers = ctx.peers;
                                    inference_model_stem = ctx.model_stem;
                                    inference_start_time = Some(std::time::Instant::now());
                                    inference_my_range = ctx.my_range;
                                    inference_total_devices = ctx.total_devices;

                                    Send_Ui(&ui_tx, Ui_Message::Job_Inference {
                                        model_name: inference_model_stem.clone(),
                                        device_count: inference_total_devices,
                                        layer_range: format!("层{}-{}", inference_my_range.0, inference_my_range.1),
                                        phase: "等待输入 prompt".to_string(),
                                    }).await;
                                    Send_Ui(&ui_tx, Ui_Message::Log("✓ Session 已就绪，请输入 prompt".to_string())).await;
                                }
                                Err(e) => {
                                    error!("推理设置失败: {}", e);
                                    Send_Ui(&ui_tx, Ui_Message::Error(e.clone())).await;
                                    state = Node_State::Idle;
                                    next_peer = None;
                                    Send_Ui(&ui_tx, Ui_Message::Job_Idle).await;
                                    Send_Ui(&ui_tx, Ui_Message::State_Change("Idle".to_string())).await;
                                    let _ = reply.send(Err(e));
                                }
                            }
                        }
                    }

                    // ── Input 命令：转发 prompt 给 Session ──
                    Some(CLI_Command::Input { prompt, reply }) => {
                        if is_inferring {
                            if let Some(ref session) = inference_session {
                                info!("Control: 收到用户 prompt ({} chars)", prompt.len());
                                Send_Ui(&ui_tx, Ui_Message::Job_Inference {
                                    model_name: inference_model_stem.clone(),
                                    device_count: inference_total_devices,
                                    layer_range: format!("层{}-{}", inference_my_range.0, inference_my_range.1),
                                    phase: "推理中".to_string(),
                                }).await;
                                match session.Send_Input(Engine_Input::Prompt(prompt)).await {
                                    Ok(_) => { let _ = reply.send(Ok("prompt 已发送".to_string())); }
                                    Err(e) => { let _ = reply.send(Err(format!("发送 prompt 失败: {}", e))); }
                                }
                            } else {
                                let _ = reply.send(Err("推理会话异常".to_string()));
                            }
                        } else {
                            let _ = reply.send(Err("没有活跃的推理会话，请先执行 run 命令".to_string()));
                        }
                    }

                    // ── SetDevice 命令 ──
                    Some(CLI_Command::SetDevice { device: new_device, reply }) => {
                        if is_inferring {
                            let _ = reply.send(Err("推理进行中，无法切换设备".to_string()));
                        } else {
                            info!("Control: 收到 set-device 命令: {}", new_device);
                            let result = Handle_Set_Device(&mut device, &config_path, &new_device, &ui_tx).await;
                            let _ = reply.send(result);
                        }
                    }

                    // ── Quit 命令 ──
                    Some(CLI_Command::Quit) => {
                        if is_inferring {
                            info!("Control: 推理期间收到退出命令");
                            if let Some(ref cancel) = inference_cancel {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            // 不 break，让 run_fut 检测到 cancel 后自行退出
                        } else {
                            info!("Control: 收到退出命令，正在关闭...");
                            if let Err(e) = node_handle.Stop().await {
                                warn!("网络节点关闭失败: {}", e);
                            }
                            break;
                        }
                    }

                    // ── DisplayPeer 命令（任何状态均可）──
                    Some(CLI_Command::DisplayPeer { reply }) => {
                        info!("Control: 收到 display-peer 命令");
                        let result = Handle_Display_Peer(&peer_handle, &ui_tx).await;
                        let _ = reply.send(result);
                    }

                    // ── CLI 通道关闭 ──
                    None => {
                        info!("Control: CLI 通道关闭，退出");
                        if is_inferring {
                            if let Some(ref cancel) = inference_cancel {
                                cancel.store(true, Ordering::Relaxed);
                            }
                        }
                        if let Err(e) = node_handle.Stop().await {
                            warn!("网络节点关闭失败: {}", e);
                        }
                        break;
                    }
                }
            }

            // ══════════════════════════════════════════
            // 2. 网络入站请求 — 始终活跃
            // ══════════════════════════════════════════
            req = inbound_rx.recv() => {
                match req {
                    Some(inbound_req) => {
                        super::network_inbound_handler::NetworkInboundHandler::Handle_Inbound(
                            &mut state,
                            &mut next_peer,
                            &mut pending_load,
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

            // ══════════════════════════════════════════
            // 3. 网络事件 — 始终活跃
            // ══════════════════════════════════════════
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

            // ══════════════════════════════════════════
            // 4. 推理 Future 完成 — 仅推理状态时活跃
            // ══════════════════════════════════════════
            result = &mut run_fut, if is_inferring => {
                info!("Control: 推理 Future 完成");
                is_inferring = false;
                run_fut = Box::pin(std::future::pending());
                // 重置 engine_output_rx 为 closed channel
                let (_dt, dr) = mpsc::channel::<Engine_Output>(1);
                drop(_dt);
                engine_output_rx = dr;

                let session = inference_session.take();
                let reply = inference_reply.take();
                let peers = std::mem::take(&mut inference_peers);
                let start_time = inference_start_time.take().unwrap_or_else(std::time::Instant::now);
                inference_cancel = None;

                match result {
                    Ok(pipeline_result) => {
                        let final_text = pipeline_result.result_text.clone();
                        let token_count = pipeline_result.generated_tokens.len();
                        info!("Control: 推理完成 ({} tokens)", token_count);

                        // 清理 Session
                        if let Some(s) = session {
                            let _ = s.Shutdown().await;
                        }
                        if !peers.is_empty() {
                            let _ = node_handle.Close_Tensor_Stream().await;
                        }

                        let total_time = start_time.elapsed();
                        let tok_per_sec = if total_time.as_secs_f64() > 0.0 {
                            token_count as f64 / total_time.as_secs_f64()
                        } else { 0.0 };

                        Send_Ui(&ui_tx, Ui_Message::Log(format!(
                            "生成完成: {} tokens, {:.3}s ({:.1} tok/s)",
                            token_count, total_time.as_secs_f64(), tok_per_sec
                        ))).await;

                        Send_Ui(&ui_tx, Ui_Message::Inference_Complete {
                            text: final_text.clone(),
                            tokens: token_count,
                            tok_per_sec,
                            total_secs: total_time.as_secs_f64(),
                        }).await;

                        let mut output = String::new();
                        output.push_str("[Pleiades] ✓ 推理完成\n");
                        output.push_str(&final_text);

                        if let Some(r) = reply {
                            let _ = r.send(Ok(output));
                        }
                    }
                    Err(e) => {
                        error!("Control: 推理失败: {}", e);
                        if let Some(s) = session {
                            let _ = s.Shutdown().await;
                        }
                        if !peers.is_empty() {
                            let _ = node_handle.Close_Tensor_Stream().await;
                        }
                        Send_Ui(&ui_tx, Ui_Message::Error(format!("推理失败: {}", e))).await;
                        if let Some(r) = reply {
                            let _ = r.send(Err(format!("推理失败: {}", e)));
                        }
                    }
                }

                // 恢复 Idle 状态
                state = Node_State::Idle;
                next_peer = None;
                Send_Ui(&ui_tx, Ui_Message::Job_Idle).await;
                Send_Ui(&ui_tx, Ui_Message::State_Change("Idle".to_string())).await;
            }

            // ══════════════════════════════════════════
            // 5. Engine 流式输出 — 仅推理状态时活跃
            // ══════════════════════════════════════════
            engine_msg = engine_output_rx.recv(), if is_inferring => {
                match engine_msg {
                    Some(Engine_Output::Text(text)) => {
                        Send_Ui(&ui_tx, Ui_Message::Inference_Token(text)).await;
                    }
                    Some(Engine_Output::Info(info_msg)) => {
                        Send_Ui(&ui_tx, Ui_Message::Log(format!(
                            "模型信息: {} (layers: {}, eos: {})",
                            info_msg.architecture, info_msg.num_layers, info_msg.eos_token_id
                        ))).await;
                    }
                    Some(Engine_Output::End) => {
                        debug!("Control: 收到 EndOutput 信号");
                    }
                    None => {
                        // Engine 输出通道关闭，等 run_fut 结束
                        debug!("Control: Engine 输出通道关闭");
                    }
                }
            }
        }
    }

    info!("Control 层事件循环已退出");
}

// ============================================================
// Setup_Inference — 推理设置（Phase 1+2）
// ============================================================

/// 推理设置 — 分析模型、切分、分发、配置流水线、创建 Session
///
/// 完成 Phase 1（分析/切分/分发）和 Phase 2（WORK/LOAD/PIPELINE_FLOW/Session 创建）后，
/// 返回 `Inference_Context`，由主循环的状态机启动推理。
///
/// 注意：此函数在执行期间会阻塞主循环（设置是有限时间操作）。
/// 推理本身（Phase 3）在主循环中异步执行，不再阻塞。
async fn Setup_Inference(
    state: &mut Node_State,
    next_peer: &mut Option<PeerId>,
    node_handle: &NodeHandle,
    peer_handle: &PeerHandle,
    device: &str,
    model_path: &Path,
    event_rx: &mut mpsc::Receiver<NetworkEvent>,
    ui_tx: &mpsc::Sender<Ui_Message>,
) -> Result<Inference_Context, String> {

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

    // Step 2: 分析模型 → Analyze_Model（spawn_blocking 包装同步调用）
    let analyze_path = model_path.to_path_buf();
    let model_info = tokio::task::spawn_blocking(move || {
        Analyze_Model(&analyze_path)
    })
    .await
    .map_err(|e| format!("分析任务执行失败: {}", e))?
    .map_err(|e| format!("模型分析失败: {}", e))?;

    let total_layers = model_info.num_layers + 2;
    let eos_token_id = model_info.eos_token_id;
    Send_Ui(ui_tx, Ui_Message::Log(format!(
        "模型: {}, 总层数: {}, EOS: {}",
        model_info.architecture, total_layers, eos_token_id
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

    // Step 4: 切分模型 → Split_Model（spawn_blocking）
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
            let split_path = model_path.to_path_buf();
            let split_output = output_dir.to_path_buf();
            let split_start = *start;
            let split_end = *end;
            tokio::task::spawn_blocking(move || {
                Split_Model(&split_path, split_start, split_end, &split_output)
            })
            .await
            .map_err(|e| format!("切分任务执行失败: {}", e))?
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
            Send_File_With_Events(node_handle, peer, split_file_path, event_rx, ui_tx)
                .await
                .map_err(|e| format!("文件发送失败 (节点 {}): {}", peer, e))?;
            Send_Ui(ui_tx, Ui_Message::Log(format!("  ✓ 已发送 → {}", peer))).await;
        }
        Send_Ui(ui_tx, Ui_Message::Log("✓ 所有模型文件已发送".to_string())).await;
    }

    // ================================================================
    // Phase 2: 发送 WORK → LOAD → PIPELINE_FLOW → 建立 Tensor Stream
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
                .Send_Data(peer, DataType::Command, cmd_bytes)
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
                .Send_Data(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 LOAD 失败 ({}): {}", peer, e))?;
            Send_Ui(ui_tx, Ui_Message::Log(format!("  ✓ 节点 {} 已收到 LOAD 命令", peer))).await;
        }

        // Step 7.5: 所有节点创建 Tensor Stream Manager
        node_handle.Create_Tensor_Stream()
            .await
            .map_err(|e| format!("创建 Tensor Stream 失败: {}", e))?;
        Send_Ui(ui_tx, Ui_Message::Log("✓ 本机 Tensor Stream Manager 已创建".to_string())).await;

        Send_Ui(ui_tx, Ui_Message::Log("发送 PREPARE_CONNECTION 命令给所有节点...".to_string())).await;
        for peer in &peers {
            let cmd_bytes = Serialize_Command(&Control_Command::Prepare_Connection);
            node_handle
                .Send_Data(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 PREPARE_CONNECTION 失败 ({}): {}", peer, e))?;
        }
        Send_Ui(ui_tx, Ui_Message::Log("✓ 所有节点 Tensor Stream Manager 已就绪".to_string())).await;

        // Step 8: 发送 PIPELINE_FLOW 命令
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
                .Send_Data(peer, DataType::Command, cmd_bytes)
                .await
                .map_err(|e| format!("发送 PIPELINE_FLOW 失败 ({}): {}", peer, e))?;
            Send_Ui(ui_tx, Ui_Message::Log(format!("  节点 {} → next: {}", peer, target))).await;
        }

        *next_peer = Some(peers[0]);
        Send_Ui(ui_tx, Ui_Message::Log(format!("  本机 → next: {}", peers[0]))).await;
        Send_Ui(ui_tx, Ui_Message::Log("✓ 流水线配置完成".to_string())).await;

        // Step 8.5: 立刻打开出站 tensor stream
        node_handle.Open_Tensor_Stream(&peers[0])
            .await
            .map_err(|e| format!("打开出站 Tensor Stream 失败: {}", e))?;
        Send_Ui(ui_tx, Ui_Message::Log("✓ 出站 Tensor Stream 已建立".to_string())).await;
    }

    // ================================================================
    // Phase 2.5: 创建 Session → 返回 Inference_Context
    // ================================================================

    let (my_start, my_end) = assignments[0];

    Send_Ui(ui_tx, Ui_Message::Job_Inference {
        model_name: model_stem.clone(),
        device_count: total_devices,
        layer_range: format!("层{}-{}", my_start, my_end),
        phase: "加载模型".to_string(),
    }).await;
    Send_Ui(ui_tx, Ui_Message::Log(format!("加载本机模型 (层 {}-{}, 从原始模型)...", my_start, my_end))).await;

    // 构建 tensor_io（多节点模式需要 tensor stream）
    let tensor_io = if !peers.is_empty() {
        let mut take_result = None;
        for attempt in 0..20 {
            match node_handle.Take_Tensor_Streams().await {
                Ok(streams) => {
                    take_result = Some(streams);
                    break;
                }
                Err(_) => {
                    if attempt < 19 {
                        debug!("等待 Tensor Stream 就绪... (尝试 {}/20)", attempt + 1);
                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                    }
                }
            }
        }

        let (inbound_stream, outbound_stream) = take_result
            .ok_or_else(|| "获取 Tensor Stream 超时（5秒），inbound 流未就绪".to_string())?;

        let rt_handle = tokio::runtime::Handle::current();
        Some(Tensor_IO_Handle::New(inbound_stream, outbound_stream, rt_handle))
    } else {
        None
    };

    // Step 9: 创建 Session
    let (session_handle, output_data_rx, session_model_info) = Create_Session(
        "coordinator".to_string(),
        model_path,
        my_start,
        my_end,
        device.to_string(),
        tensor_io,
    )
    .await
    .map_err(|e| format!("创建 Session 失败: {}", e))?;

    Send_Ui(ui_tx, Ui_Message::Log(format!(
        "✓ 本机模型已加载 (input_head: {}, output_head: {}, tokenizer: {})",
        session_model_info.has_input_head, session_model_info.has_output_head, session_model_info.has_tokenizer
    ))).await;

    // 设置本机状态为 Busy
    *state = Node_State::Busy;
    Send_Ui(ui_tx, Ui_Message::State_Change("Busy".to_string())).await;

    // Step 10: 编排推理指令程序
    let program = Build_Inference_Program(!peers.is_empty());
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let params = Pipeline_Params {
        max_tokens: MAX_GENERATION_ROUNDS,
        temperature: TEMPERATURE,
        seed: SEED,
        eos_token_id: Some(eos_token_id),
    };

    // 返回推理上下文，由主循环状态机启动推理
    Ok(Inference_Context {
        session_handle,
        output_data_rx,
        cancel_flag,
        program,
        params,
        peers,
        model_stem,
        my_range: (my_start, my_end),
        total_devices,
    })
}

// ============================================================
// 推理程序编排
// ============================================================

/// 根据是否分布式模式，构建协调者推理指令程序
///
/// 程序以 `Input` 指令开头 — Session 线程执行到此处时会
/// 阻塞在 `input_data_rx` 上等待用户输入 prompt。
///
/// ## 单机推理程序
/// ```text
/// Input → Encode → Set(META2, max_tokens) → Inference(TOKENID3) → Sample(TENSOR2) → Decode → Output
/// → Loop [ BreakIf, Inference(TOKENID2), Sample(TENSOR2), Decode, Output ]
/// → EndOutput
/// ```
///
/// ## 协调者 Pipeline 推理程序
/// ```text
/// Input → Encode → Set(META2, max_tokens) → Inference(TOKENID3)
/// → Send → Receive → Sample(TENSOR1) → Decode → Output
/// → Loop [ BreakIf, Inference(TOKENID2), Send, Receive, Sample(TENSOR1), Decode, Output ]
/// → SendEOF → EndOutput
/// ```
fn Build_Inference_Program(is_distributed: bool) -> Vec<Instruction> {
    if !is_distributed {
        // 单机推理程序
        vec![
            Instruction::Input,
            Instruction::Encode,
            Instruction::Set { target: Set_Target::Meta(META2, MAX_GENERATION_ROUNDS as f64) },
            Instruction::Prefill { input: TOKENID3 },
            Instruction::CopyMeta { src: META5, dst: META1 },
            Instruction::Sample { tensor_reg: TENSOR2 },
            Instruction::Decode,
            Instruction::Output,
            Instruction::Loop {
                body: vec![
                    Instruction::BreakIf,
                    Instruction::Inference { input: Inference_Input::Tokens(TOKENID2) },
                    Instruction::Sample { tensor_reg: TENSOR2 },
                    Instruction::Decode,
                    Instruction::Output,
                ],
            },
            Instruction::EndOutput,
        ]
    } else {
        // 协调者 Pipeline 推理程序
        vec![
            Instruction::Input,
            Instruction::Encode,
            Instruction::Set { target: Set_Target::Meta(META2, MAX_GENERATION_ROUNDS as f64) },
            Instruction::Prefill { input: TOKENID3 },
            Instruction::Send,
            Instruction::Receive,
            Instruction::CopyMeta { src: META5, dst: META1 },
            Instruction::Sample { tensor_reg: TENSOR1 },
            Instruction::Decode,
            Instruction::Output,
            Instruction::Loop {
                body: vec![
                    Instruction::BreakIf,
                    Instruction::Inference { input: Inference_Input::Tokens(TOKENID2) },
                    Instruction::Send,
                    Instruction::Receive,
                    Instruction::Sample { tensor_reg: TENSOR1 },
                    Instruction::Decode,
                    Instruction::Output,
                ],
            },
            Instruction::SendEOF,
            Instruction::EndOutput,
        ]
    }
}

/// 构建 Worker Relay 推理程序
///
/// ```text
/// Loop [ Receive, BreakIf, Inference(TENSOR1), Send ]
/// ```
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

// ============================================================
// 辅助函数
// ============================================================

/// 发送文件，同时转发网络事件到 TUI
async fn Send_File_With_Events(
    node_handle: &NodeHandle,
    peer: &PeerId,
    file_path: PathBuf,
    event_rx: &mut mpsc::Receiver<NetworkEvent>,
    ui_tx: &mpsc::Sender<Ui_Message>,
) -> Result<(), String> {
    let send_fut = node_handle.Send_File(peer, file_path);
    tokio::pin!(send_fut);

    loop {
        tokio::select! {
            result = &mut send_fut => {
                return result.map_err(|e| format!("{}", e));
            }
            evt = event_rx.recv() => {
                if let Some(event) = evt {
                    Handle_Event(event, ui_tx).await;
                }
            }
        }
    }
}

// ============================================================
// 入站请求处理（被动模式）— 历史代码，由 NetworkInboundHandler 替代
// ============================================================

/// 处理入站请求（其他节点发来的）
async fn Handle_Inbound(
    state: &mut Node_State,
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
            Send_Ui(ui_tx, Ui_Message::Log(format!(
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
                    Handle_Control_Command(
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
            
            if req.payload.len() >= 8 {
                let size_bytes = u64::from_le_bytes([
                    req.payload[0], req.payload[1], req.payload[2], req.payload[3],
                    req.payload[4], req.payload[5], req.payload[6], req.payload[7],
                ]);
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
async fn Handle_Control_Command(
    state: &mut Node_State,
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
            *state = Node_State::Busy;
            Send_Ui(ui_tx, Ui_Message::Log(format!("WORK (来自 {}): 状态 → Busy", peer))).await;
            Send_Ui(ui_tx, Ui_Message::State_Change("Busy".to_string())).await;
            let _ = node_handle
                .Send_Response(request_id, DataType::Command, b"OK".to_vec())
                .await;
        }

        Control_Command::Load { model_path, start, end } => {
            Send_Ui(ui_tx, Ui_Message::Log(format!("LOAD (来自 {}): {} 层 {}-{}", peer, model_path, start, end))).await;
            Send_Ui(ui_tx, Ui_Message::Job_Inference {
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
            Send_Ui(ui_tx, Ui_Message::Log(format!("PREPARE_CONNECTION (来自 {}): 创建 Tensor Stream Manager", peer))).await;
            if let Err(e) = node_handle.Create_Tensor_Stream().await {
                error!("创建 Tensor Stream 失败: {}", e);
                let _ = node_handle
                    .Send_Response(request_id, DataType::Command, format!("ERROR: {}", e).into_bytes())
                    .await;
                return;
            }
            Send_Ui(ui_tx, Ui_Message::Log("✓ Tensor Stream Manager 已创建".to_string())).await;
            let _ = node_handle
                .Send_Response(request_id, DataType::Command, b"OK".to_vec())
                .await;
        }

        Control_Command::Pipeline_Flow { next_peer: target } => {
            *next_peer = Some(target);
            Send_Ui(ui_tx, Ui_Message::Log(format!("PIPELINE_FLOW (来自 {}): next → {}", peer, target))).await;

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

            if let Err(e) = node_handle.Open_Tensor_Stream(&target).await {
                error!("打开出站 Tensor Stream 失败: {}", e);
                let _ = node_handle
                    .Send_Response(request_id, DataType::Command, format!("ERROR: {}", e).into_bytes())
                    .await;
                return;
            }

            let _ = node_handle
                .Send_Response(request_id, DataType::Command, b"OK".to_vec())
                .await;

            Send_Ui(ui_tx, Ui_Message::Job_Inference {
                model_name: model_path_str.clone(),
                device_count: 1,
                layer_range: format!("层{}-{}", layer_start, layer_end),
                phase: "加载模型".to_string(),
            }).await;

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

                    let device_clone = device.to_string();
                    let ui_tx_clone = ui_tx.clone();
                    let node_handle_clone = node_handle.clone();
                    let model_path = PathBuf::from(&model_path_str);

                    tokio::spawn(async move {
                        let session_result = Create_Session(
                            "worker".to_string(),
                            &model_path,
                            layer_start,
                            layer_end,
                            device_clone,
                            Some(tensor_io),
                        ).await;

                        match session_result {
                            Ok((session_handle, _output_data_rx, model_info)) => {
                                Send_Ui(&ui_tx_clone, Ui_Message::Log(format!(
                                    "✓ Worker 模型加载完成 (arch: {}, input: {}, output: {})",
                                    model_info.architecture, model_info.has_input_head, model_info.has_output_head
                                ))).await;

                                Send_Ui(&ui_tx_clone, Ui_Message::Job_Inference {
                                    model_name: model_path_str.clone(),
                                    device_count: 1,
                                    layer_range: format!("层{}-{}", layer_start, layer_end),
                                    phase: "就绪，等待推理数据".to_string(),
                                }).await;

                                let relay_program = Build_Worker_Relay_Program();
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
                                        info!("Worker: Relay 程序完成");
                                        Send_Ui(&ui_tx_clone, Ui_Message::Log("Relay 程序完成".to_string())).await;
                                    }
                                    Err(e) => {
                                        error!("Worker: Relay 程序失败: {}", e);
                                        Send_Ui(&ui_tx_clone, Ui_Message::Error(format!("Relay 失败: {}", e))).await;
                                    }
                                }

                                let _ = session_handle.Shutdown().await;
                            }
                            Err(e) => {
                                error!("Worker: Session 创建失败: {}", e);
                                Send_Ui(&ui_tx_clone, Ui_Message::Error(format!("模型加载失败: {}", e))).await;
                            }
                        }

                        let _ = node_handle_clone.Close_Tensor_Stream().await;
                        Send_Ui(&ui_tx_clone, Ui_Message::Job_Idle).await;
                        Send_Ui(&ui_tx_clone, Ui_Message::State_Change("Idle".to_string())).await;
                    });
                }
                None => {
                    error!("获取 Tensor Stream 超时（5秒），inbound 流未就绪");
                    Send_Ui(ui_tx, Ui_Message::Error("获取 Tensor Stream 超时，inbound 流未就绪".to_string())).await;
                }
            }
        }
    }
}

// ============================================================
// Handle_Set_Device — 设备切换处理
// ============================================================

/// 处理 set-device 命令
///
/// 1. 修改 Control 层持有的 device 变量
/// 2. 将修改写回 config.toml 文件（持久化，下次启动使用新设置）
/// 3. 通知 TUI 更新设备显示
async fn Handle_Set_Device(
    device: &mut String,
    config_path: &Path,
    new_device: &str,
    ui_tx: &mpsc::Sender<Ui_Message>,
) -> Result<String, String> {
    let old_device = device.clone();

    // 更新 Control 层持有的 device
    *device = new_device.to_string();
    info!("设备已切换: {} → {}", old_device, new_device);

    // 写回 config.toml
    if let Err(e) = Update_Config(config_path, "Runtime", "device", new_device) {
        warn!("配置文件写入失败: {} (设备已切换但未持久化)", e);
        Send_Ui(ui_tx, Ui_Message::Log(format!(
            "⚠ 配置文件写入失败: {} (设备已切换但未持久化)", e
        ))).await;
    } else {
        info!("配置文件已更新: device = {}", new_device);
    }

    // 通知 TUI 更新设备显示
    Send_Ui(ui_tx, Ui_Message::Device_Change(new_device.to_string())).await;

    Ok(format!(
        "设备已切换: {} → {}\n配置已保存，下次启动将默认使用 {}",
        old_device.to_uppercase(),
        new_device.to_uppercase(),
        new_device.to_uppercase()
    ))
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
            Send_Ui(ui_tx, Ui_Message::Job_Idle).await;
        }
        
        NetworkEvent::FileStreamProgress { peer, file_name, direction, sent, total } => {
            Send_Ui(ui_tx, Ui_Message::File_Progress {
                file_name,
                direction,
                peer: peer.to_string(),
                sent,
                total,
            }).await;
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

// ============================================================
// Display Peer 命令处理
// ============================================================

/// 处理 display-peer 命令，显示所有节点信息
async fn Handle_Display_Peer(
    peer_handle: &PeerHandle,
    ui_tx: &mpsc::Sender<Ui_Message>,
) -> Result<String, String> {
    use crate::peer_management::PeerStatus;
    
    info!("正在获取节点信息...");
    
    // 获取所有节点
    let peers = match peer_handle.list_peers().await {
        Ok(peers) => peers,
        Err(e) => {
            let error_msg = format!("获取节点列表失败: {}", e);
            warn!("{}", error_msg);
            return Err(error_msg);
        }
    };
    
    if peers.is_empty() {
        let msg = "当前没有连接的节点".to_string();
        Send_Ui(ui_tx, Ui_Message::Log(msg.clone())).await;
        return Ok(msg);
    }
    
    // 构建显示信息
    let mut output = String::new();
    output.push_str("=== 节点信息 ===\n");
    
    for (i, peer_info) in peers.iter().enumerate() {
        let status_str = match peer_info.query_status() {
            PeerStatus::Connected => "已连接",
            PeerStatus::Busy => "忙碌",
            PeerStatus::Connecting => "连接中",
            PeerStatus::Disconnected => "已断开",
        };
        
        let (capability_opt, latency_opt, bandwidth_opt) = peer_info.query_profile();
        let latency_ms = latency_opt.unwrap_or(0);
        let bandwidth_mbps = bandwidth_opt.unwrap_or(0);
        
        // 处理能力信息
        let (has_gpu, memory_mb, compute_score) = if let Some(capability) = capability_opt {
            (
                if capability.has_gpu { "有GPU" } else { "无GPU" },
                capability.memory_mb,
                capability.compute_score
            )
        } else {
            ("未知", 0, 0.0)
        };
        
        output.push_str(&format!(
            "{}. 节点ID: {}\n   状态: {}\n   延迟: {}ms\n   带宽: {}Mbps\n   能力: {}, 内存: {}MB, 计算分: {}\n   最后活跃: {:?}\n",
            i + 1,
            peer_info.peer_id,
            status_str,
            latency_ms,
            bandwidth_mbps,
            has_gpu,
            memory_mb,
            compute_score,
            peer_info.last_active
        ));
    }
    
    output.push_str(&format!("总计: {} 个节点", peers.len()));
    
    // 发送到UI显示
    Send_Ui(ui_tx, Ui_Message::Log(output.clone())).await;
    
    Ok(output)
}
