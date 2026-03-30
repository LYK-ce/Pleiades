//Presented by KeJi
//Date : 2026-03-30

//! gguf_model 模块
//!
//! 向上负责向 runtime 提供统一的模型抽象，提供模型运行的接口等，
//! runtime 层应该看不到底层模型的细节，只能当作一个完整的模型来使用。
//! 向下负责调用 gguf_model_manager 进行模型解析、加载等工作。
//!
//! tokenizer 作为独立的 API 供其他组件调用：
//! - GGUF_Encode: 将文本编码为 token IDs
//! - GGUF_Decode: 将 token IDs 解码为文本
//! - GGUF_Model_Inference: 单步前向推理，只让模型所有层推理一次

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use candle_core::{DType, Device, Tensor};
use candle_core::quantized::gguf_file;
use candle_transformers::quantized_nn::RmsNorm;
use candle_transformers::models::with_tracing::QMatMul;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::gguf_model_manager::{GGUF_Analyze, GGUF_Load_Layer, Model_Arch_Info};
use super::gguf_models::{
    Rotary_Embedding, Layer_Weights, Model_Weights,
};

// ============================================================
// 数据结构定义
// ============================================================

/// 推理参数配置，由 runtime 上层进行配置
pub struct Inference_Config {
    /// 生成最大 token 数
    pub max_tokens: usize,
    /// 采样温度（0.0 = greedy，越高越随机）
    pub temperature: f64,
    /// 随机种子
    pub seed: u64,
    /// EOS token ID（遇到此 token 停止生成），从 GGUF metadata 的 tokenizer.ggml.eos_token_id 自动获取
    pub eos_token: u32,
}

impl Default for Inference_Config {
    fn default() -> Self {
        Self {
            max_tokens: 600,
            temperature: 0.8,
            seed: 299792458,
            eos_token: 151645, // 由 GGUF_Load_Model 从模型 metadata 中自动填充
        }
    }
}

/// 组装好的 GGUF 模型，包含模型权重、tokenizer、推理配置以及架构信息
///
/// 此结构体对 runtime 层提供统一的模型抽象：
/// - 包含完整的模型权重（embedding + 所有层 + norm + lm_head）
/// - 包含 tokenizer（如果 GGUF 文件中存在）
/// - 包含 Inference_Config，由 runtime 上层配置
/// - tokenizer 作为独立 API 供外部组件调用，不耦合在推理过程中
pub struct GGUF_Model {
    /// 组装好的模型权重（包含 embedding + 所有层 + norm + lm_head）
    pub model: Model_Weights,
    /// 从 GGUF 文件加载的 tokenizer，如果 GGUF 文件包含此 tokenizer 则加载，否则为 None
    pub tokenizer: Option<shimmytok::Tokenizer>,
    /// 推理参数配置（由 runtime 上层配置）
    pub inference_config: Inference_Config,
    /// 模型架构信息（由 GGUF_Analyze 解析得到）
    pub arch_info: Model_Arch_Info,
    /// 模型文件路径
    pub model_path: PathBuf,
    /// 运行设备
    pub device: Device,
    /// 标记模型是否包含输入头（embedding）
    pub has_input_head: bool,
    /// 标记模型是否包含输出头（lm_head）
    pub has_output_head: bool,
}

// ============================================================
// 核心公开函数
// ============================================================

/// GGUF_Load_Model(model_path, device)
///
/// 从 GGUF 模型文件路径读取模型，解析模型信息，根据其信息调用具体的模型部件进行组装
/// 并放置在对应设备上，然后返回一个可供推理的模型。
///
/// # 步骤
/// 1. 调用 GGUF_Analyze 解析模型文件，获取模型架构信息
/// 2. 根据模型架构信息匹配模型架构（目前仅支持 Qwen3）
/// 3. 构建 RotaryEmbedding
/// 4. 如果包含 tokenizer，加载 tokenizer 到 GGUF_Model 当中
/// 5. 逐层加载模型的每一层权重，并组装到对应位置
/// 6. 将组装好的模型放置在对应设备上
/// 7. 返回组装好的模型（含默认 Inference_Config）
pub fn GGUF_Load_Model(model_path: &Path, device: &Device) -> Result<GGUF_Model> {
    // 1. 调用 GGUF_Analyze 解析模型文件，获取模型架构信息
    let arch_info = GGUF_Analyze(model_path)?;

    // 2. 根据模型架构信息，匹配对应的模型架构
    let architecture = arch_info.architecture.to_lowercase();
    match architecture.as_str() {
        "qwen3" => { /* supported */ }
        _ => anyhow::bail!(
            "Unsupported model architecture: '{}'. Currently only 'qwen3' is supported.",
            arch_info.architecture
        ),
    }

    // 3. 打开 GGUF 文件，读取 Content（用于加载非层 tensor）
    let mut file = std::fs::File::open(model_path)?;
    let content = gguf_file::Content::read(&mut file)
        .map_err(|e| anyhow::anyhow!("Failed to read GGUF content: {:?}", e))?;

    // 4. 检测输入头并加载 embedding + tokenizer
    let has_input_head = content.tensor_infos.contains_key("token_embd.weight");
    let mut tokenizer: Option<shimmytok::Tokenizer> = None;

    // 如果 GGUF 文件包含 tokenizer 数据，则加载
    if let Ok(tok) = shimmytok::Tokenizer::from_gguf_file(model_path) {
        tokenizer = Some(tok);
    }

    let embed_tokens = if has_input_head {
        let embed_qtensor = content
            .tensor(&mut file, "token_embd.weight", device)
            .map_err(|e| anyhow::anyhow!("Failed to load token_embd.weight: {}", e))?;
        let embed_tensor = embed_qtensor
            .dequantize(device)
            .map_err(|e| anyhow::anyhow!("Failed to dequantize embedding: {}", e))?;
        candle_nn::Embedding::new(embed_tensor, arch_info.embedding_length)
    } else {
        anyhow::bail!(
            "Model does not contain input head (token_embd.weight). \
             Partial models without embedding are not yet supported."
        );
    };

    // 5. 构建 RotaryEmbedding
    let rotary = Arc::new(
        Rotary_Embedding::New(
            DType::F32,
            arch_info.head_dim,
            arch_info.context_length,
            arch_info.rope_freq_base,
            device,
        )
        .map_err(|e| anyhow::anyhow!("Failed to build RotaryEmbedding: {}", e))?,
    );

    // 6. 逐层加载模型权重并组装
    let layer_indices: Vec<usize> = (0..arch_info.num_layers).collect();
    let layer_weights_list = GGUF_Load_Layer(model_path, &layer_indices, device)?;

    let mut layers = Vec::with_capacity(arch_info.num_layers);
    for mut lw in layer_weights_list {
        let layer = Layer_Weights::From_Extracted(
            &mut lw.tensors,
            arch_info.head_count,
            arch_info.head_count_kv,
            arch_info.head_dim,
            arch_info.rms_norm_eps,
            rotary.clone(),
            lw.layer_index,
        )
        .map_err(|e| anyhow::anyhow!("Layer {} assembly failed: {}", lw.layer_index, e))?;
        layers.push(layer);
    }

    // 7. 加载 output_norm + lm_head
    let has_output_head = content.tensor_infos.contains_key("output_norm.weight");

    let (norm, lm_head) = if has_output_head {
        let norm_qtensor = content
            .tensor(&mut file, "output_norm.weight", device)
            .map_err(|e| anyhow::anyhow!("Failed to load output_norm.weight: {}", e))?;
        let norm = RmsNorm::from_qtensor(norm_qtensor, arch_info.rms_norm_eps)
            .map_err(|e| anyhow::anyhow!("Failed to build output RmsNorm: {}", e))?;

        let lm_head_qtensor = match content.tensor(&mut file, "output.weight", device) {
            Ok(t) => t,
            Err(_) => content
                .tensor(&mut file, "token_embd.weight", device)
                .map_err(|e| anyhow::anyhow!("Failed to load lm_head (fallback): {}", e))?,
        };
        let lm_head = QMatMul::from_weights(lm_head_qtensor.into())
            .map_err(|e| anyhow::anyhow!("Failed to build lm_head QMatMul: {}", e))?;

        (norm, lm_head)
    } else {
        anyhow::bail!(
            "Model does not contain output head (output_norm.weight). \
             Partial models without output head are not yet supported."
        );
    };

    // 8. 组装完整模型并放置在对应设备上
    let model = Model_Weights::From_Dynamic(
        embed_tokens,
        layers,
        norm,
        lm_head,
        device.clone(),
        DType::F32,
    );

    // 9. 返回组装好的模型，eos_token 从模型 metadata 中自动获取
    let mut inference_config = Inference_Config::default();
    inference_config.eos_token = arch_info.eos_token_id;

    Ok(GGUF_Model {
        model,
        tokenizer,
        inference_config,
        arch_info,
        model_path: model_path.to_path_buf(),
        device: device.clone(),
        has_input_head,
        has_output_head,
    })
}

/// GGUF_Unload_Model(model)
///
/// 卸载模型，释放模型占用的资源，包括 tokenizer（如果有的话）。
///
/// # 返回
/// - `true`: 卸载成功
pub fn GGUF_Unload_Model(model: GGUF_Model) -> bool {
    drop(model);
    true
}

/// GGUF_Model_Inference(model, token_ids, offset)
///
/// 对输入的 token IDs 进行单步前向推理（让模型所有层推理一次）。
/// 此函数不进行自回归循环，只执行一次完整的模型前向传播：
///   token_ids → embedding → 所有层 → output_norm → lm_head → logits
///
/// 自回归生成循环由调用方负责管理。
///
/// # 参数
/// - `model`: 组装好的 GGUF 模型（可变引用，因为 KV cache 会更新）
/// - `token_ids`: 输入的 token IDs（prefill 阶段为完整 prompt，生成阶段为单个 token）
/// - `offset`: 位置偏移量，用于 KV cache 和位置编码（自回归生成时递增）
///
/// # 返回
/// logits Tensor，shape 为 [batch_size, vocab_size]
pub fn GGUF_Model_Inference(
    model: &mut GGUF_Model,
    token_ids: &[u32],
    offset: usize,
) -> Result<Tensor> {
    let input_tensor =
        Tensor::new(token_ids, &model.device)?.unsqueeze(0)?;

    let logits = model
        .model
        .Forward(&input_tensor, offset)
        .map_err(|e| anyhow::anyhow!("Forward pass failed: {}", e))?;

    Ok(logits)
}

// ============================================================
// Tokenizer 独立 API
// ============================================================

/// GGUF_Encode(model, input)
///
/// 将文本通过 tokenizer 编码为 token IDs，作为模型的输入部分。
/// 自动添加 Qwen3 对话格式模板。
///
/// # 参数
/// - `model`: GGUF 模型（包含 tokenizer）
/// - `input`: 原始输入文本
///
/// # 返回
/// 编码后的 token IDs
pub fn GGUF_Encode(model: &GGUF_Model, input: &str) -> Result<Vec<u32>> {
    let tokenizer = model.tokenizer.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Model does not have a tokenizer loaded. Cannot encode text."
        )
    })?;

    let format_prompt = format!(
        "<|im_start|>user\n{input}<|im_end|>\n<|im_start|>assistant\n"
    );

    let token_ids = tokenizer
        .encode(&format_prompt, true)
        .map_err(|e| anyhow::anyhow!("Tokenization error: {}", e))?;

    Ok(token_ids)
}

/// GGUF_Decode(model, token_ids)
///
/// 将 token IDs 通过 tokenizer 解码为文本，作为模型的输出部分。
/// 自动移除末尾的 EOS token（如果存在）。
///
/// # 参数
/// - `model`: GGUF 模型（包含 tokenizer）
/// - `token_ids`: 需要解码的 token IDs
///
/// # 返回
/// 解码后的文本
pub fn GGUF_Decode(model: &GGUF_Model, token_ids: &[u32]) -> Result<String> {
    let tokenizer = model.tokenizer.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Model does not have a tokenizer loaded. Cannot decode tokens."
        )
    })?;

    // 移除末尾的 EOS token
    let eos_token = model.inference_config.eos_token;
    let decode_tokens: &[u32] = if token_ids.last() == Some(&eos_token) {
        &token_ids[..token_ids.len() - 1]
    } else {
        token_ids
    };

    let output_text = tokenizer
        .decode(decode_tokens, true)
        .map_err(|e| anyhow::anyhow!("Decoding error: {}", e))?;

    Ok(output_text)
}
