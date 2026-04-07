//Presented by KeJi
//Date ： 2026-04-07

//! Control 核心模块 - 调度核心事件循环
//!
//! 通过 `tokio::select!` 同时处理三个事件源：
//! 1. cli_rx: CLI 发来的用户命令
//! 2. inbound_rx: 其他节点发来的网络请求 (InboundRequest)
//! 3. event_rx: 网络事件（连接/断开/文件到达等）
//!
//! ## 第一阶段功能
//! - 处理 run 命令: 查询节点 → 分析模型 → 切分 → 分发文件
//! - 被动接收: 接受文件传输请求，文件保存到 Pleiades_Workspace/
//! - 网络事件: 打印连接/断开/文件到达日志

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::path::Path;
use tokio::sync::mpsc;
use tracing::{info, warn, error};

use super::cli::CLI_Command;
use crate::ml_engine::ml_inference_service::ML_Service_Handle;
use crate::network::data_protocol::DataType;
use crate::network::node::NetworkEvent;
use crate::network::node_handle::{InboundRequest, NodeHandle};

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
///
/// 同时监听 CLI 命令、网络入站请求和网络事件，进行调度处理。
///
/// # 参数
/// - `cli_rx`: 来自 CLI 的命令接收通道
/// - `inbound_rx`: 来自网络层的入站请求接收通道
/// - `event_rx`: 来自网络层的事件接收通道
/// - `ml_service`: ML 推理服务句柄
/// - `node_handle`: 网络节点句柄
pub async fn Control_Loop(
    mut cli_rx: mpsc::Receiver<CLI_Command>,
    mut inbound_rx: mpsc::Receiver<InboundRequest>,
    mut event_rx: mpsc::Receiver<NetworkEvent>,
    ml_service: ML_Service_Handle,
    node_handle: NodeHandle,
) {
    // 初始化节点状态为 Idle
    let mut state = Node_State::Idle;
    info!("Control 层事件循环已启动, 节点状态: {:?}", state);

    loop {
        tokio::select! {
            // 1. CLI 命令
            cmd = cli_rx.recv() => {
                match cmd {
                    Some(CLI_Command::Run { model_path, prompt, reply }) => {
                        info!("Control: 收到 run 命令 (model: {}, prompt: {})",
                            model_path.display(), prompt);
                        let result = Handle_Run(
                            &ml_service,
                            &node_handle,
                            &model_path,
                            &prompt,
                        ).await;
                        let _ = reply.send(result);
                    }
                    Some(CLI_Command::Quit) => {
                        info!("Control: 收到退出命令，正在关闭...");
                        // 优雅退出
                        if let Err(e) = ml_service.Shutdown().await {
                            warn!("ML Service 关闭失败: {}", e);
                        }
                        if let Err(e) = node_handle.Stop().await {
                            warn!("网络节点关闭失败: {}", e);
                        }
                        break;
                    }
                    None => {
                        // CLI 通道关闭（CLI 线程退出）
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
                        Handle_Inbound(&node_handle, inbound_req).await;
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
// 命令处理函数
// ============================================================

/// 处理 run 命令
///
/// 流程：
/// 1. 查询网络中已连接的节点
/// 2. 分析模型结构
/// 3. 计算均分方案
/// 4. 切分模型
/// 5. 分发文件给其他节点
async fn Handle_Run(
    ml_service: &ML_Service_Handle,
    node_handle: &NodeHandle,
    model_path: &Path,
    _prompt: &str,
) -> Result<String, String> {
    let mut output = String::new();

    // Step 1: 查询网络中的节点
    let peers = node_handle
        .Get_Peers()
        .await
        .map_err(|e| format!("查询节点列表失败: {}", e))?;

    let total_devices = peers.len() + 1; // 包含自己
    let msg = format!("[Pleiades] 发现 {} 台设备 (含自己)", total_devices);
    println!("{}", msg);
    output.push_str(&msg);
    output.push('\n');

    // 列出节点信息
    println!(
        "[Pleiades] 本机: {}",
        node_handle.Get_Local_Peer_Id()
    );
    for (i, peer) in peers.iter().enumerate() {
        println!("[Pleiades] 节点 {}: {}", i + 1, peer);
    }

    // Step 2: 分析模型
    let arch_info = ml_service
        .Analyze_Model(model_path)
        .await
        .map_err(|e| format!("模型分析失败: {}", e))?;

    // 总层数: 层0(embedding) + 层1~N(transformer blocks) + 层N+1(output)
    let total_layers = arch_info.num_layers + 2;
    let msg = format!(
        "[Pleiades] 模型: {}, 总层数: {} (embedding + {} transformer blocks + output)",
        arch_info.architecture, total_layers, arch_info.num_layers
    );
    println!("{}", msg);
    output.push_str(&msg);
    output.push('\n');

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

    // 打印分配方案
    println!("[Pleiades] 分配方案:");
    println!(
        "  本机 ({}): 层 {}-{}",
        node_handle.Get_Local_Peer_Id(),
        assignments[0].0,
        assignments[0].1
    );
    for (i, peer) in peers.iter().enumerate() {
        let (start, end) = assignments[i + 1];
        println!("  节点 {} ({}): 层 {}-{}", i + 1, peer, start, end);
    }

    let msg = format!(
        "[Pleiades] 分配: {} 台设备, 每台约 {} 层",
        total_devices, layers_per_node
    );
    output.push_str(&msg);
    output.push('\n');

    // Step 4: 切分模型
    let output_dir = Path::new("Pleiades_Workspace");

    // 确保输出目录存在
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| format!("创建输出目录失败: {}", e))?;
    }

    println!("[Pleiades] 正在切分模型...");
    for (i, (start, end)) in assignments.iter().enumerate() {
        println!("  切分第 {} 段: 层 {}-{}", i, start, end);
        ml_service
            .Split_Model(model_path, *start, *end, output_dir)
            .await
            .map_err(|e| format!("模型切分失败 (层 {}-{}): {}", start, end, e))?;
    }

    let msg = format!("[Pleiades] 模型切分完成, 生成 {} 个文件", total_devices);
    println!("{}", msg);
    output.push_str(&msg);
    output.push('\n');

    // Step 5: 分发文件给其他节点
    //   assignments[0] 是自己的，不用发
    //   assignments[1..] 分别发给 peers[0], peers[1], ...
    if !peers.is_empty() {
        println!("[Pleiades] 正在向其他节点发送模型文件...");

        // 获取原始模型文件名的 stem
        let model_stem = model_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();

        for (i, peer) in peers.iter().enumerate() {
            let (start, end) = assignments[i + 1];
            let split_file_name = format!("{}_split_{}_{}.pgguf", model_stem, start, end);
            let split_file_path = output_dir.join(&split_file_name);

            println!(
                "  发送 {} → 节点 {} ({})",
                split_file_name, i + 1, peer
            );

            node_handle
                .Send_File(peer, split_file_path)
                .await
                .map_err(|e| format!("文件发送失败 (节点 {}): {}", peer, e))?;

            println!("  ✓ 文件已发送到节点 {}", peer);
        }

        let msg = "[Pleiades] 所有模型文件已发送".to_string();
        println!("{}", msg);
        output.push_str(&msg);
        output.push('\n');
    } else {
        let msg = "[Pleiades] 仅有本机, 无需分发文件".to_string();
        println!("{}", msg);
        output.push_str(&msg);
        output.push('\n');
    }

    // 完成
    let msg = "[Pleiades] ✓ 模型分发完成".to_string();
    println!("{}", msg);
    output.push_str(&msg);

    Ok(output)
}

// ============================================================
// 入站请求处理
// ============================================================

/// 处理入站请求（其他节点发来的）
///
/// MVP 阶段：所有请求全部接受
async fn Handle_Inbound(node_handle: &NodeHandle, req: InboundRequest) {
    info!(
        "Control: 收到入站请求 (id={}, peer={}, type={:?}, payload_len={})",
        req.request_id,
        req.peer,
        req.data_type,
        req.payload.len()
    );

    match req.data_type {
        DataType::File => {
            // 文件传输通知，自动接受
            println!(
                "[Pleiades] 收到文件传输请求 (来自 {}), 自动接受",
                req.peer
            );
            if let Err(e) = node_handle
                .Send_Reply(
                    req.request_id,
                    DataType::Command,
                    b"ACCEPT".to_vec(),
                )
                .await
            {
                error!("发送文件接受回复失败: {}", e);
            }
        }
        DataType::Command => {
            // 命令请求，MVP 阶段回复 OK
            println!(
                "[Pleiades] 收到命令请求 (来自 {}): {:?}",
                req.peer,
                String::from_utf8_lossy(&req.payload)
            );
            if let Err(e) = node_handle
                .Send_Reply(
                    req.request_id,
                    DataType::Command,
                    b"OK".to_vec(),
                )
                .await
            {
                error!("发送命令回复失败: {}", e);
            }
        }
        DataType::Data => {
            // 数据请求，MVP 阶段回复 OK
            println!(
                "[Pleiades] 收到数据请求 (来自 {}, {} bytes)",
                req.peer,
                req.payload.len()
            );
            if let Err(e) = node_handle
                .Send_Reply(
                    req.request_id,
                    DataType::Data,
                    b"OK".to_vec(),
                )
                .await
            {
                error!("发送数据回复失败: {}", e);
            }
        }
    }
}

// ============================================================
// 网络事件处理
// ============================================================

/// 处理网络事件
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
            println!(
                "[Pleiades] 收到文件: {} (来自 {})",
                file_path.display(),
                peer
            );
        }
        NetworkEvent::FileStreamError { peer, error } => {
            eprintln!(
                "[Pleiades] 文件传输错误: {} (节点 {})",
                error, peer
            );
        }
        NetworkEvent::RecordFound { key, value } => {
            info!(
                "DHT 记录查询成功: key={} bytes, value={} bytes",
                key.len(),
                value.len()
            );
        }
        NetworkEvent::RecordNotFound { key } => {
            info!("DHT 记录未找到: key={} bytes", key.len());
        }
    }
}
