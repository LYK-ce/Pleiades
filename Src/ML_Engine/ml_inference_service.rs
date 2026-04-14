//Presented by KeJi
//Date ： 2026-04-13

//! ML 推理服务模块
//!
//! 向 Control 层提供创建 Session、切分模型、分析模型等的方法。
//!
//! ## 设计原则
//! - 采用 Session 设计，一个 Session 对应一个 OS 线程
//! - 一个模型对应一个 Session，换模型需销毁当前 Session 并创建新的
//! - Session 专注推理，模型编排仍由 Control 层负责
//! - Split_Model 和 Analyze_Model 是独立方法，不走 Session
//!
//! ## API
//! - `Create_Session`: 创建推理会话，加载模型，返回 Session_Handle + output_data_rx + Model_Info
//! - `Split_Model`: 切分模型文件（独立方法）
//! - `Analyze_Model`: 分析模型文件结构（独立方法）
//!
//! ## Create_Session 流程
//! ```text
//! Create_Session()
//!   → 创建通道 (cmd, input_data, output_data, ready)
//!   → 启动 OS 线程 (Session_Thread)
//!   → 线程内部: 阻塞加载模型 → 初始化寄存器 → oneshot 发送 Ok(Model_Info) 或 Err
//!   → await 就绪信号
//!   → 成功: 返回 (Session_Handle, output_data_rx, Model_Info)
//!   → 失败: 返回 Err
//! ```

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use std::path::Path;
use std::thread;
use tokio::sync::{mpsc, oneshot};
use tracing::info;

use super::gguf_model_manager::{GGUF_Analyze, GGUF_Split_Model};
use super::ml_thread_engine::{Session_Config, Session_Handle, Session_Thread};
use super::ml_thread_engine_instruction::{Engine_Output, Model_Info};
use crate::network::tensor_stream_manager::Tensor_IO_Handle;

// ============================================================
// Create_Session
// ============================================================

/// 创建推理会话
///
/// async 方法。内部流程：
/// 1. 创建命令通道 (cmd_tx/cmd_rx)
/// 2. 创建数据通道 (input_data_tx/input_data_rx, output_data_tx/output_data_rx)
/// 3. 创建就绪信号通道 (oneshot)
/// 4. 启动 OS 线程，将 cmd_rx, input_data_rx, output_data_tx, tensor_io 及配置传入线程
/// 5. 线程内部：阻塞加载模型 → 初始化寄存器 → 通过 oneshot 发送 Ok(Model_Info) 或 Err
/// 6. await 就绪信号，加载成功返回 (Session_Handle, output_data_rx, Model_Info)，失败返回 Err
///
/// # 参数
/// - `session_id`: Session 的唯一标识
/// - `model_path`: 模型路径
/// - `layer_start`: 从第几层模型开始加载
/// - `layer_end`: 到第几层结束
/// - `device`: 使用的 device，"cpu" 或 "cuda"
/// - `tensor_io`: 张量 IO 句柄 (Option)，包含入站和出站张量流。单机推理时为 None。
///
/// # 返回
/// - `Ok((Session_Handle, mpsc::Receiver<Engine_Output>, Model_Info))`
/// - `Err` 如果模型加载失败
pub async fn Create_Session(
    session_id: String,
    model_path: &Path,
    layer_start: usize,
    layer_end: usize,
    device: String,
    tensor_io: Option<Tensor_IO_Handle>,
) -> Result<(Session_Handle, mpsc::Receiver<Engine_Output>, Model_Info)> {
    info!(
        "ML Service: 创建 Session [{}] (model: {}, layers: {}-{}, device: {})",
        session_id,
        model_path.display(),
        layer_start,
        layer_end,
        device
    );

    // Step 1: 创建命令通道
    let (cmd_tx, cmd_rx) = mpsc::channel(32);

    // Step 2: 创建数据通道
    let (input_data_tx, input_data_rx) = mpsc::channel(32);
    let (output_data_tx, output_data_rx) = mpsc::channel(64);

    // Step 3: 创建就绪信号通道
    let (ready_tx, ready_rx) = oneshot::channel();

    // Step 4: 构造 Session_Config
    let session_config = Session_Config {
        model_path: model_path.to_path_buf(),
        layer_start,
        layer_end,
        device,
    };

    // Step 5: 启动 OS 线程
    let thread_session_id = session_id.clone();
    thread::spawn(move || {
        Session_Thread(
            thread_session_id,
            session_config,
            cmd_rx,
            input_data_rx,
            output_data_tx,
            tensor_io,
            ready_tx,
        );
    });

    // Step 6: await 就绪信号
    let model_info = ready_rx
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Session [{}]: 线程在发送就绪信号前终止",
                session_id
            )
        })??;

    info!(
        "ML Service: Session [{}] 创建成功 (arch: {}, layers: {})",
        session_id, model_info.architecture, model_info.num_layers
    );

    // 构造 Session_Handle
    let session_handle = Session_Handle::New(session_id, cmd_tx, input_data_tx);

    Ok((session_handle, output_data_rx, model_info))
}

// ============================================================
// Split_Model（独立方法，不走 Session）
// ============================================================

/// 切分模型文件
///
/// 根据输入切分模型。这是独立方法，不走 Session。
///
/// # 参数
/// - `path`: 源模型文件路径
/// - `start`: 起始层编号
/// - `end`: 结束层编号
/// - `output`: 输出文件路径
///
/// # 返回
/// `Result<()>`
pub fn Split_Model(path: &Path, start: usize, end: usize, output: &Path) -> Result<()> {
    info!(
        "ML Service: 切分模型 {} (layers {}-{}, output: {})",
        path.display(),
        start,
        end,
        output.display()
    );
    GGUF_Split_Model(path, start, end, output)?;
    info!("ML Service: 模型切分完成");
    Ok(())
}

// ============================================================
// Analyze_Model（独立方法，不走 Session）
// ============================================================

/// 分析模型文件结构
///
/// 根据输入的路径分析模型。这是独立方法，不走 Session。
///
/// # 参数
/// - `path`: 模型文件路径
///
/// # 返回
/// `Result<Model_Info>` 模型结构信息
pub fn Analyze_Model(path: &Path) -> Result<Model_Info> {
    info!("ML Service: 分析模型文件 {}", path.display());
    let arch_info = GGUF_Analyze(path)?;

    let model_info = Model_Info {
        architecture: arch_info.architecture.clone(),
        num_layers: arch_info.num_layers,
        has_input_head: !arch_info.is_split || arch_info.split_start == 0,
        has_output_head: !arch_info.is_split || arch_info.split_end == arch_info.num_layers + 1,
        has_tokenizer: false, // Analyze 不加载 tokenizer
        eos_token_id: arch_info.eos_token_id,
    };

    info!(
        "ML Service: 模型分析完成 (arch: {}, layers: {}, split: {})",
        arch_info.architecture, arch_info.num_layers, arch_info.is_split
    );

    Ok(model_info)
}
