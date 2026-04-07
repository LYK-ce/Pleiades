//Presented by KeJi
//Date ： 2026-04-04

//! ML 推理服务模块
//!
//! 为上层提供统一的 ML 推理引擎接口，将底层推理引擎（当前为 GGUF/Candle）
//! 封装在独立的 OS 线程中运行，通过 channel 进行通信。
//!
//! 设计目标：
//! - 推理运算在独立 OS 线程中执行，避免阻塞 tokio async 运行时
//! - 通过 channel 通信，保证线程安全
//! - 后端可替换，当前仅实现 GGUF/Candle 后端
//!
//! 当前支持的后端：
//! - GGUF (Candle) - 支持 GGUF 格式量化模型
//!
//! 未来可扩展的后端：
//! - ONNX Runtime
//! - TensorRT-LLM
//!
//! ## API
//! - `Init`: 初始化服务，创建通道和工作线程
//! - `Load_Model`: 加载模型到推理引擎
//! - `Unload_Model`: 卸载当前模型
//! - `Inference`: 单步前向推理（一次 forward pass）
//! - `Encode`: 文本编码为 token IDs
//! - `Decode`: token IDs 解码为文本
//! - `Shutdown`: 关闭服务，停止工作线程

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use candle_core::{Device, Tensor};
use std::path::{Path, PathBuf};
use std::thread;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info};

use super::gguf_model::{
    GGUF_Decode, GGUF_Encode, GGUF_Load_Model, GGUF_Model, GGUF_Model_Inference,
    GGUF_Unload_Model,
};
use super::gguf_model_manager::{GGUF_Analyze, GGUF_Split_Model, Model_Arch_Info};

// ============================================================
// 推理后端枚举（当前仅支持 GGUF，未来可扩展）
// ============================================================

/// 推理引擎后端
///
/// 当前仅实现 GGUF/Candle 后端。
/// 未来扩展时，在此枚举中添加新变体即可。
enum Inference_Backend {
    /// GGUF/Candle 后端
    GGUF(GGUF_Model),
    // 未来扩展：
    // ONNX(ONNX_Model),
    // TensorRT(TRT_Model),
}

// ============================================================
// 模型加载信息
// ============================================================

/// 模型加载后返回的信息
///
/// 上层通过此结构体了解已加载模型的基本属性，
/// 用于决定后续的推理策略（如是否需要 Encode/Decode）。
pub struct Model_Load_Info {
    /// 模型架构名称（如 "qwen3"）
    pub architecture: String,
    /// 总层数（transformer block 数量）
    pub num_layers: usize,
    /// 是否包含输入头（embedding）
    pub has_input_head: bool,
    /// 是否包含输出头（lm_head）
    pub has_output_head: bool,
    /// 是否包含 tokenizer
    pub has_tokenizer: bool,
    /// EOS token ID
    pub eos_token_id: u32,
}

// ============================================================
// 服务命令枚举
// ============================================================

/// ML 推理服务内部命令
///
/// 通过 channel 从 Handle 发送到 Worker 线程
enum Service_Command {
    /// 加载模型
    Load_Model {
        model_path: PathBuf,
        start: usize,
        end: usize,
        device: String,
        reply: oneshot::Sender<Result<Model_Load_Info>>,
    },
    /// 卸载模型
    Unload_Model {
        reply: oneshot::Sender<Result<bool>>,
    },
    /// 单步前向推理（统一字节接口）
    Inference {
        input_bytes: Vec<u8>,
        offset: usize,
        reply: oneshot::Sender<Result<Tensor>>,
    },
    /// 文本编码
    Encode {
        text: String,
        reply: oneshot::Sender<Result<Vec<u32>>>,
    },
    /// Token 解码
    Decode {
        token_ids: Vec<u32>,
        reply: oneshot::Sender<Result<String>>,
    },
    /// 模型分析（不加载，仅读取结构信息）
    Analyze_Model {
        model_path: PathBuf,
        reply: oneshot::Sender<Result<Model_Arch_Info>>,
    },
    /// 切分模型
    Split_Model {
        model_path: PathBuf,
        split_start: usize,
        split_end: usize,
        output_path: PathBuf,
        reply: oneshot::Sender<Result<()>>,
    },
    /// 关闭服务
    Shutdown,
}

// ============================================================
// ML_Service_Handle（对外 API）
// ============================================================

/// ML 推理服务句柄
///
/// 对上层提供的统一推理引擎接口。可 Clone、可 Send，
/// 通过内部 channel 与工作线程通信。
///
/// # 使用流程
/// 1. 调用 `Init()` 获取 Handle
/// 2. 调用 `Load_Model()` 加载模型
/// 3. 调用 `Encode()` 编码文本 → `Inference()` 推理 → `Decode()` 解码结果
/// 4. 调用 `Unload_Model()` 卸载模型
/// 5. 调用 `Shutdown()` 或 drop Handle 时，工作线程自动退出
#[derive(Clone)]
pub struct ML_Service_Handle {
    /// 命令发送通道
    cmd_tx: mpsc::Sender<Service_Command>,
}

impl ML_Service_Handle {
    /// Init() -> ML_Service_Handle
    ///
    /// 初始化 ML 推理服务：
    /// 1. 创建命令通道 (tokio mpsc channel)
    /// 2. 启动工作线程（独立 OS 线程，用于执行 CPU/GPU 密集型推理）
    /// 3. 返回句柄供上层调用
    ///
    /// 工作线程使用 `tokio::sync::mpsc::Receiver::blocking_recv()` 阻塞接收命令，
    /// Handle 端使用 `tokio::sync::mpsc::Sender::send().await` 异步发送命令。
    /// 回复使用 `tokio::sync::oneshot`，Worker 同步 send，Handle 异步 await。
    ///
    /// # 返回
    /// ML_Service_Handle 实例
    pub fn Init() -> ML_Service_Handle {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Service_Command>(32);

        // 启动独立 OS 线程作为推理 Worker
        thread::spawn(move || {
            Worker_Loop(cmd_rx);
        });

        info!("ML Inference Service 已初始化");

        ML_Service_Handle { cmd_tx }
    }

    /// Load_Model(model_path, start, end, device)
    ///
    /// 加载模型到推理引擎。如果已有模型加载，会先自动卸载旧模型。
    ///
    /// 层编号规则（与 GGUF_Load_Layer 一致）：
    ///   0           = 输入层 (embedding)
    ///   1..=N       = 中间层 (transformer block)
    ///   N+1         = 输出层 (output_norm + lm_head)
    ///
    /// # 参数
    /// - `model_path`: 模型文件路径（.gguf 或 .pgguf）
    /// - `start`: 起始层编号
    /// - `end`: 结束层编号
    /// - `device`: 设备类型字符串（"cpu" 或 "cuda"）
    ///
    /// # 返回
    /// `Model_Load_Info` 包含模型基本信息
    pub async fn Load_Model(
        &self,
        model_path: &Path,
        start: usize,
        end: usize,
        device: &str,
    ) -> Result<Model_Load_Info> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Service_Command::Load_Model {
                model_path: model_path.to_path_buf(),
                start,
                end,
                device: device.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 回复通道已关闭"))?
    }

    /// Unload_Model()
    ///
    /// 卸载当前加载的模型，释放资源（包括 GPU 显存）。
    ///
    /// # 返回
    /// - `Ok(true)`: 卸载成功
    /// - `Err`: 没有已加载的模型
    pub async fn Unload_Model(&self) -> Result<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Service_Command::Unload_Model { reply: reply_tx })
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 回复通道已关闭"))?
    }

    /// Inference(input_bytes, offset)
    ///
    /// 执行单步前向推理（一次 forward pass）。
    /// 不进行自回归循环，自回归生成由调用方（Control 层）管理。
    ///
    /// 统一字节接口：模型根据自身结构决定如何解释输入：
    /// - 有 embedding 层: 字节解释为 token_ids（每 4 字节一个 u32 LE）
    /// - 无 embedding 层: 字节解释为序列化的 tensor（GGUF_Tensor_Packet 格式）
    ///
    /// # 参数
    /// - `input_bytes`: 输入原始字节
    ///   - 有 embedding: token_ids 序列化为字节（每个 u32 → 4 bytes LE）
    ///   - 无 embedding: GGUF_Tensor_Serialize 序列化的 tensor 字节
    /// - `offset`: 位置偏移量（用于 KV cache 和位置编码，自回归生成时递增）
    ///
    /// # 返回
    /// 输出 Tensor（logits 或 hidden_state，取决于模型是否有输出头）
    pub async fn Inference(&self, input_bytes: Vec<u8>, offset: usize) -> Result<Tensor> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Service_Command::Inference {
                input_bytes,
                offset,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 回复通道已关闭"))?
    }

    /// Encode(text)
    ///
    /// 将文本通过 tokenizer 编码为 token IDs。
    /// 需要模型已加载且包含 tokenizer。
    ///
    /// # 参数
    /// - `text`: 输入文本
    ///
    /// # 返回
    /// 编码后的 token IDs
    pub async fn Encode(&self, text: &str) -> Result<Vec<u32>> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Service_Command::Encode {
                text: text.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 回复通道已关闭"))?
    }

    /// Decode(token_ids)
    ///
    /// 将 token IDs 通过 tokenizer 解码为文本。
    /// 需要模型已加载且包含 tokenizer。
    ///
    /// # 参数
    /// - `token_ids`: 需要解码的 token IDs
    ///
    /// # 返回
    /// 解码后的文本
    pub async fn Decode(&self, token_ids: &[u32]) -> Result<String> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Service_Command::Decode {
                token_ids: token_ids.to_vec(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 回复通道已关闭"))?
    }

    /// Analyze_Model(model_path)
    ///
    /// 分析模型文件结构（不加载权重到内存），返回模型架构信息。
    /// 供 Control 层获取模型层数等信息，用于决策分片策略。
    ///
    /// # 参数
    /// - `model_path`: 模型文件路径（.gguf 或 .pgguf）
    ///
    /// # 返回
    /// `Model_Arch_Info` 包含模型完整架构信息
    pub async fn Analyze_Model(&self, model_path: &Path) -> Result<Model_Arch_Info> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Service_Command::Analyze_Model {
                model_path: model_path.to_path_buf(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 回复通道已关闭"))?
    }

    /// Split_Model(model_path, split_start, split_end, output_path)
    ///
    /// 切分模型文件，将指定层范围提取为新的 .pgguf 文件。
    /// 此操作不需要先加载模型，直接对文件进行操作。
    ///
    /// 层编号规则（与 GGUF_Split_Model 一致）：
    ///   0           = 输入层 (embedding)
    ///   1..=N       = 中间层 (transformer block)
    ///   N+1         = 输出层 (output_norm + lm_head)
    ///
    /// # 参数
    /// - `model_path`: 原始模型文件路径（.gguf）
    /// - `split_start`: 切分起始层编号
    /// - `split_end`: 切分结束层编号
    /// - `output_path`: 输出目录路径
    ///
    /// # 返回
    /// `Ok(())` 表示切分成功
    pub async fn Split_Model(
        &self,
        model_path: &Path,
        split_start: usize,
        split_end: usize,
        output_path: &Path,
    ) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.cmd_tx
            .send(Service_Command::Split_Model {
                model_path: model_path.to_path_buf(),
                split_start,
                split_end,
                output_path: output_path.to_path_buf(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;

        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 回复通道已关闭"))?
    }

    /// Shutdown()
    ///
    /// 关闭 ML 推理服务，停止工作线程。
    /// 如果有模型加载，Worker 线程会先自动卸载模型再退出。
    pub async fn Shutdown(&self) -> Result<()> {
        self.cmd_tx
            .send(Service_Command::Shutdown)
            .await
            .map_err(|_| anyhow::anyhow!("ML Service worker 线程已停止"))?;
        Ok(())
    }
}

// ============================================================
// Worker 线程主循环
// ============================================================

/// 工作线程主循环
///
/// 在独立 OS 线程中运行，持有推理后端实例（`Option<Inference_Backend>`），
/// 通过 `blocking_recv()` 阻塞接收命令并执行。
///
/// 退出条件：
/// - 收到 `Shutdown` 命令
/// - 所有 `Sender` 被 drop（channel 关闭）
fn Worker_Loop(mut cmd_rx: mpsc::Receiver<Service_Command>) {
    let mut backend: Option<Inference_Backend> = None;

    info!("ML Service Worker 线程已启动");

    loop {
        // 阻塞等待命令（适用于独立 OS 线程）
        let cmd = match cmd_rx.blocking_recv() {
            Some(cmd) => cmd,
            None => {
                // 所有 Sender 已 drop，退出循环
                info!("ML Service Worker: 所有句柄已释放，退出");
                break;
            }
        };

        match cmd {
            Service_Command::Load_Model {
                model_path,
                start,
                end,
                device,
                reply,
            } => {
                debug!("Worker: 收到 Load_Model 命令");
                let result = Handle_Load_Model(&mut backend, &model_path, start, end, &device);
                let _ = reply.send(result);
            }

            Service_Command::Unload_Model { reply } => {
                debug!("Worker: 收到 Unload_Model 命令");
                let result = Handle_Unload_Model(&mut backend);
                let _ = reply.send(result);
            }

            Service_Command::Inference {
                input_bytes,
                offset,
                reply,
            } => {
                debug!(
                    "Worker: 收到 Inference 命令 (input_bytes: {}, offset: {})",
                    input_bytes.len(),
                    offset
                );
                let result = Handle_Inference(&mut backend, &input_bytes, offset);
                let _ = reply.send(result);
            }

            Service_Command::Encode { text, reply } => {
                debug!("Worker: 收到 Encode 命令");
                let result = Handle_Encode(&backend, &text);
                let _ = reply.send(result);
            }

            Service_Command::Decode { token_ids, reply } => {
                debug!("Worker: 收到 Decode 命令");
                let result = Handle_Decode(&backend, &token_ids);
                let _ = reply.send(result);
            }

            Service_Command::Analyze_Model { model_path, reply } => {
                debug!("Worker: 收到 Analyze_Model 命令");
                let result = Handle_Analyze_Model(&model_path);
                let _ = reply.send(result);
            }

            Service_Command::Split_Model {
                model_path,
                split_start,
                split_end,
                output_path,
                reply,
            } => {
                debug!("Worker: 收到 Split_Model 命令");
                let result = Handle_Split_Model(&model_path, split_start, split_end, &output_path);
                let _ = reply.send(result);
            }

            Service_Command::Shutdown => {
                info!("Worker: 收到 Shutdown 命令，正在退出...");
                // 先卸载模型再退出
                if backend.is_some() {
                    let _ = Handle_Unload_Model(&mut backend);
                }
                break;
            }
        }
    }

    info!("ML Service Worker 线程已退出");
}

// ============================================================
// Worker 内部命令处理函数
// ============================================================

/// 处理 Load_Model 命令
///
/// 根据当前实现，使用 GGUF/Candle 后端加载模型。
/// 未来扩展其他后端时，可通过参数或配置选择后端类型。
fn Handle_Load_Model(
    backend: &mut Option<Inference_Backend>,
    model_path: &Path,
    start: usize,
    end: usize,
    device_str: &str,
) -> Result<Model_Load_Info> {
    // 如果已有模型，先卸载
    if backend.is_some() {
        info!("Worker: 已有模型加载，先卸载旧模型");
        *backend = None;
    }

    // 解析设备
    let device = match device_str.to_lowercase().as_str() {
        "cpu" => Device::Cpu,
        "cuda" => Device::new_cuda(0)
            .map_err(|e| anyhow::anyhow!("CUDA 设备初始化失败: {}", e))?,
        other => anyhow::bail!(
            "不支持的设备类型: '{}'. 目前仅支持 'cpu' 和 'cuda'.",
            other
        ),
    };

    // 当前仅支持 GGUF 后端
    // 未来扩展时，可根据文件扩展名或配置参数选择不同后端
    info!(
        "Worker: 加载 GGUF 模型 {} (layers {}-{}, device: {})",
        model_path.display(),
        start,
        end,
        device_str
    );

    let gguf_model = GGUF_Load_Model(start, end, model_path, &device)?;

    let load_info = Model_Load_Info {
        architecture: gguf_model.arch_info.architecture.clone(),
        num_layers: gguf_model.arch_info.num_layers,
        has_input_head: gguf_model.has_input_head,
        has_output_head: gguf_model.has_output_head,
        has_tokenizer: gguf_model.tokenizer.is_some(),
        eos_token_id: gguf_model.arch_info.eos_token_id,
    };

    *backend = Some(Inference_Backend::GGUF(gguf_model));

    info!(
        "Worker: 模型加载完成 (arch: {}, layers: {}, input_head: {}, output_head: {}, tokenizer: {})",
        load_info.architecture,
        load_info.num_layers,
        load_info.has_input_head,
        load_info.has_output_head,
        load_info.has_tokenizer
    );

    Ok(load_info)
}

/// 处理 Unload_Model 命令
fn Handle_Unload_Model(backend: &mut Option<Inference_Backend>) -> Result<bool> {
    match backend.take() {
        Some(Inference_Backend::GGUF(model)) => {
            info!("Worker: 卸载 GGUF 模型");
            GGUF_Unload_Model(model);
            Ok(true)
        }
        // 未来扩展其他后端的卸载逻辑：
        // Some(Inference_Backend::ONNX(model)) => { ... }
        None => {
            anyhow::bail!("没有已加载的模型")
        }
    }
}

/// 处理 Inference 命令
///
/// 执行单步前向推理，返回输出 Tensor。
/// 根据当前后端类型分派到对应的推理实现。
/// 输入为原始字节，模型内部根据是否有 embedding 层决定解释方式。
fn Handle_Inference(
    backend: &mut Option<Inference_Backend>,
    input_bytes: &[u8],
    offset: usize,
) -> Result<Tensor> {
    match backend {
        Some(Inference_Backend::GGUF(ref mut model)) => {
            GGUF_Model_Inference(model, input_bytes, offset)
        }
        // 未来扩展其他后端的推理逻辑：
        // Some(Inference_Backend::ONNX(ref mut model)) => { ... }
        None => {
            anyhow::bail!("没有已加载的模型，无法推理")
        }
    }
}

/// 处理 Encode 命令
///
/// 将文本通过模型内置的 tokenizer 编码为 token IDs。
fn Handle_Encode(backend: &Option<Inference_Backend>, text: &str) -> Result<Vec<u32>> {
    match backend {
        Some(Inference_Backend::GGUF(ref model)) => GGUF_Encode(model, text),
        // 未来扩展其他后端的编码逻辑：
        // Some(Inference_Backend::ONNX(ref model)) => { ... }
        None => {
            anyhow::bail!("没有已加载的模型，无法编码")
        }
    }
}

/// 处理 Decode 命令
///
/// 将 token IDs 通过模型内置的 tokenizer 解码为文本。
fn Handle_Decode(backend: &Option<Inference_Backend>, token_ids: &[u32]) -> Result<String> {
    match backend {
        Some(Inference_Backend::GGUF(ref model)) => GGUF_Decode(model, token_ids),
        // 未来扩展其他后端的解码逻辑：
        // Some(Inference_Backend::ONNX(ref model)) => { ... }
        None => {
            anyhow::bail!("没有已加载的模型，无法解码")
        }
    }
}

/// 处理 Analyze_Model 命令
///
/// 分析 GGUF 模型文件结构（仅读取元数据，不加载权重）。
/// 当前仅支持 GGUF 格式，未来扩展时可根据文件扩展名分派。
fn Handle_Analyze_Model(model_path: &Path) -> Result<Model_Arch_Info> {
    info!("Worker: 分析模型文件 {}", model_path.display());
    let arch_info = GGUF_Analyze(model_path)?;
    info!(
        "Worker: 模型分析完成 (arch: {}, layers: {}, split: {})",
        arch_info.architecture,
        arch_info.num_layers,
        arch_info.is_split
    );
    Ok(arch_info)
}

/// 处理 Split_Model 命令
///
/// 切分 GGUF 模型文件，将指定层范围提取为新的 .pgguf 文件。
/// 此操作不依赖已加载的后端，直接对文件进行读写操作。
fn Handle_Split_Model(
    model_path: &Path,
    split_start: usize,
    split_end: usize,
    output_path: &Path,
) -> Result<()> {
    info!(
        "Worker: 切分模型文件 {} (layers {}-{}, output: {})",
        model_path.display(),
        split_start,
        split_end,
        output_path.display()
    );
    GGUF_Split_Model(model_path, split_start, split_end, output_path)?;
    info!("Worker: 模型切分完成");
    Ok(())
}
