//Presented by KeJi
//Date ： 2026-04-13

//! ML Session 引擎核心（VM 模式）。
//!
//! 一个 Session 对应一个 OS 线程，一个模型对应一个 Session。
//! Session 通过通道架构与外部通信。
//!
//! ## 通道架构
//! | 通道 | 方向 | 平面 | 内容 |
//! |---|---|---|---|
//! | cmd_rx | 上层 → Session | 命令平面 | Run_Program_VM, Shutdown |
//! | io_handle.input_rx | 前端 → Session | 文本平面(输入) | String (prompt) |
//! | io_handle.output_tx | Session → 前端 | 文本平面(输出) | String (completion) |
//! | tensor_io | 上游/下游节点 ↔ Session | 数据平面(网络) | 张量帧 |

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use super::gguf_model::{
    GGUF_Load_Model, GGUF_Unload_Model, GGUF_Model,
};
use super::gguf_model_manager::Model_Arch_Info;
use super::pipeline::{
    Model_Info, Pipeline_Params, Pipeline_Result,
};
use crate::llm_io::IoHandle;
use crate::tensor_io::Tensor_IO_Endpoint;
use crate::ml_engine::ml_vm::ML_VM;

// ============================================================
// Session 配置
// ============================================================

pub struct Session_Config {
    pub model_path: PathBuf,
    pub layer_start: usize,
    pub layer_end: usize,
    pub device: String,
}

// ============================================================
// Session 命令协议
// ============================================================

pub enum Session_Command {
    Run_Program_VM {
        program: Vec<crate::ml_engine::ml_vm::MlInstruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<Pipeline_Result>>,
    },
    Shutdown,
}

// ============================================================
// Session（推理会话容器，线程私有）
// ============================================================

pub struct Session {
    pub id: String,
    pub(crate) cmd_rx: mpsc::Receiver<Session_Command>,
    pub(crate) io_handle: IoHandle,
    pub(crate) backend: GGUF_Model,
    pub(crate) config: Session_Config,
    pub(crate) tensor_io: Option<Tensor_IO_Endpoint>,
}

// ============================================================
// Session_Handle（上层持有的句柄）
// ============================================================

#[derive(Clone)]
pub struct Session_Handle {
    session_id: String,
    cmd_tx: mpsc::Sender<Session_Command>,
}

impl Session_Handle {
    pub fn New(
        session_id: String,
        cmd_tx: mpsc::Sender<Session_Command>,
    ) -> Self {
        Self { session_id, cmd_tx }
    }

    pub async fn Run_Program_VM(
        &self,
        program: Vec<crate::ml_engine::ml_vm::MlInstruction>,
        params: Pipeline_Params,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<Pipeline_Result> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Session_Command::Run_Program_VM {
                program,
                params,
                cancel_flag,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Session {} 线程已停止", self.session_id))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("Session {} 回复通道已关闭", self.session_id))?
    }

    pub async fn Shutdown(&self) -> Result<()> {
        self.cmd_tx
            .send(Session_Command::Shutdown)
            .await
            .map_err(|_| {
                anyhow::anyhow!("Session {} 线程已停止（无法发送 Shutdown）", self.session_id)
            })
    }

    pub fn Get_Id(&self) -> &str {
        &self.session_id
    }
}

// ============================================================
// 辅助函数
// ============================================================

/// 从 Model_Arch_Info 构建 layer_sizes_bytes 数组
/// 索引: 0=embedding, 1..=N=blocks, N+1=output
fn build_layer_sizes(arch_info: &Model_Arch_Info) -> Vec<usize> {
    let num_layers = arch_info.num_layers;
    let mut sizes = vec![0usize; num_layers + 2];

    // embedding 层 (layer 0)
    for t in &arch_info.non_layer_tensors {
        if t.name == "token_embd.weight" {
            sizes[0] += t.size_bytes;
        }
    }

    // transformer blocks (layer 1..=N)
    for layer in &arch_info.layers {
        if layer.layer_index < num_layers {
            sizes[layer.layer_index + 1] = layer.total_size_bytes;
        }
    }

    // output 层 (layer N+1)
    for t in &arch_info.non_layer_tensors {
        if t.name == "output_norm.weight" || t.name == "output.weight" {
            sizes[num_layers + 1] += t.size_bytes;
        }
    }

    sizes
}

// ============================================================
// Session_Thread（线程入口）
// ============================================================

pub fn Session_Thread(
    session_id: String,
    session_config: Session_Config,
    cmd_rx: mpsc::Receiver<Session_Command>,
    io_handle: IoHandle,
    tensor_io: Option<Tensor_IO_Endpoint>,
    ready_tx: oneshot::Sender<Result<Model_Info>>,
) {
    info!(
        "Session [{}]: 线程已启动, 加载模型 {} (layers {}-{}, device: {})",
        session_id,
        session_config.model_path.display(),
        session_config.layer_start,
        session_config.layer_end,
        session_config.device
    );

    let device = match session_config.device.to_lowercase().as_str() {
        "cpu" => candle_core::Device::Cpu,
        "cuda" => match candle_core::Device::new_cuda(0) {
            Ok(d) => d,
            Err(e) => {
                warn!("Session [{}]: CUDA 设备初始化失败: {}", session_id, e);
                let _ = ready_tx.send(Err(anyhow::anyhow!("{}", e)));
                return;
            }
        },
        other => {
            warn!(
                "Session [{}]: 不支持的设备类型: '{}'. 目前仅支持 'cpu' 和 'cuda'.",
                session_id, other
            );
            let _ = ready_tx.send(Err(anyhow::anyhow!("不支持的设备类型: {}", other)));
            return;
        }
    };

    let gguf_model = match GGUF_Load_Model(
        session_config.layer_start,
        session_config.layer_end,
        &session_config.model_path,
        &device,
    ) {
        Ok(model) => model,
        Err(e) => {
            warn!("Session [{}]: 模型加载失败: {}", session_id, e);
            let _ = ready_tx.send(Err(anyhow::anyhow!("{}", e)));
            return;
        }
    };

    let model_info = Model_Info {
        architecture: gguf_model.arch_info.architecture.clone(),
        num_layers: gguf_model.arch_info.num_layers,
        embedding_length: gguf_model.arch_info.embedding_length,
        has_input_head: gguf_model.has_input_head,
        has_output_head: gguf_model.has_output_head,
        has_tokenizer: gguf_model.tokenizer.is_some(),
        eos_token_id: gguf_model.arch_info.eos_token_id,
        layer_sizes_bytes: build_layer_sizes(&gguf_model.arch_info),
        num_kv_heads: gguf_model.arch_info.head_count_kv,
        head_dim: gguf_model.arch_info.head_dim,
        context_length: gguf_model.arch_info.context_length,
        vocab_size: gguf_model.arch_info.vocab_size,
    };

    info!(
        "Session [{}]: 模型加载完成 (arch: {}, layers: {}, input_head: {}, output_head: {}, tokenizer: {})",
        session_id,
        model_info.architecture,
        model_info.num_layers,
        model_info.has_input_head,
        model_info.has_output_head,
        model_info.has_tokenizer
    );

    if let Err(_) = ready_tx.send(Ok(model_info)) {
        warn!("Session [{}]: 就绪信号发送失败（调用方可能已取消）", session_id);
        return;
    }

    let mut session = Session {
        id: session_id.clone(),
        cmd_rx,
        io_handle,
        backend: gguf_model,
        config: session_config,
        tensor_io,
    };

    info!("Session [{}]: 进入指令泵循环", session_id);

    loop {
        match session.cmd_rx.blocking_recv() {
            Some(Session_Command::Run_Program_VM {
                program,
                params,
                cancel_flag,
                reply,
            }) => {
                info!(
                    "Session [{}]: 收到 Run_Program_VM ({} 条指令)",
                    session_id,
                    program.len()
                );

                let mut ml_vm = ML_VM::new(
                    &mut session.backend,
                    &mut session.io_handle,
                    &mut session.tensor_io,
                );
                let result = ml_vm.execute_with_params(program, &params, &cancel_flag);
                let _ = reply.send(result.map_err(|e| anyhow::anyhow!("{}", e)));
            }
            Some(Session_Command::Shutdown) => {
                info!("Session [{}]: 收到 Shutdown 命令，正在退出...", session_id);
                break;
            }
            None => {
                info!("Session [{}]: 所有句柄已释放，退出指令泵", session_id);
                break;
            }
        }
    }

    info!("Session [{}]: 卸载模型...", session_id);
    GGUF_Unload_Model(session.backend);

    info!("Session [{}]: 线程已退出", session_id);
}
