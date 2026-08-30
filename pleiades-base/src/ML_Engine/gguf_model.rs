//Presented by KeJi
//Created Date ： 2026-03-30
//Modified Date ： 2026-07-03

//! gguf_model 模块 — Qwen3 / Qwen3MoE 模型组装
//!
//! 向上负责向 runtime 提供统一的模型抽象，提供模型运行的接口等，
//! runtime 层应该看不到底层模型的细节，只能当作一个完整的模型来使用。
//! 向下负责调用 gguf_model_manager 进行模型解析、加载等工作。
//!
//! DeepSeek / Llama 代码已移入 gguf_model_legacy.rs，恢复时参考。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use candle_core::{DType, Device, Tensor};
use candle_core::quantized::gguf_file;
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::gguf_model_manager::{
    GGUF_Analyze_And_Convert, GGUF_Analyze_From_Content, GGUF_Load_Layer, Model_Arch_Info,
};
use super::gguf_models::common::model::Model;
use super::gguf_models::{Model_Weights, Rotary_Embedding};
use super::gguf_models::qwen3::Qwen3MoE_Model;

// ============================================================
// 数据结构定义
// ============================================================


/// 组装好的 GGUF 模型
///
/// 包含模型权重、tokenizer、架构信息。
pub struct GGUF_Model {
    pub model: Box<dyn Model + Send>,
    pub tokenizer: Option<shimmytok::Tokenizer>,
    pub arch_info: Model_Arch_Info,
    pub model_path: PathBuf,
    pub device: Device,
    pub has_input_head: bool,
    pub has_output_head: bool,
}

/// 从 metadata 中读取模型 dtype
fn Read_Model_Dtype(metadata: &std::collections::HashMap<String, gguf_file::Value>) -> DType {
    match metadata.get("general.dtype") {
        Some(v) => match v.to_u32() {
            Ok(0) => DType::F32,
            Ok(1) => DType::F16,
            Ok(30) => DType::BF16,
            _ => DType::F16,
        },
        None => DType::F16,
    }
}

/// 加载输出层（norm + lm_head，含 weight tying fallback）
fn Load_Output_Head(
    content: &gguf_file::Content,
    file: &mut std::fs::File,
    max_layer_index: usize,
    rms_norm_eps: f64,
    device: &Device,
) -> Result<(RmsNorm, QMatMul)> {
    let mut lw = GGUF_Load_Layer(content, file, max_layer_index, device)?;

    let norm_qtensor = lw.tensors.remove("output_norm.weight")
        .ok_or_else(|| anyhow::anyhow!("Output layer does not contain output_norm.weight"))?;
    let norm = RmsNorm::from_qtensor(norm_qtensor, rms_norm_eps)
        .map_err(|e| anyhow::anyhow!("Failed to build output RmsNorm: {e}"))?;

    let lm_head_qtensor = if let Some(qt) = lw.tensors.remove("output.weight") {
        qt
    } else {
        // Weight tying: 使用 embedding 权重作为 lm_head
        let mut embed_lw = GGUF_Load_Layer(content, file, 0, device)
            .map_err(|e| anyhow::anyhow!("Failed to load embedding for lm_head fallback: {e}"))?;
        embed_lw.tensors.remove("token_embd.weight")
            .ok_or_else(|| anyhow::anyhow!("output.weight not found and token_embd fallback failed"))?
    };
    let lm_head = QMatMul::from_weights(lm_head_qtensor.into())
        .map_err(|e| anyhow::anyhow!("Failed to build lm_head: {e}"))?;

    Ok((norm, lm_head))
}

/// 构建 GGUF_Model 并返回
fn Build_GGUF_Model(
    model: Box<dyn Model + Send>,
    arch_info: Model_Arch_Info,
    model_path: PathBuf,
    device: Device,
    has_input_head: bool,
    has_output_head: bool,
) -> GGUF_Model {
    GGUF_Model {
        model,
        tokenizer: None,
        arch_info,
        model_path,
        device,
        has_input_head,
        has_output_head,
    }
}

// ============================================================
// 核心公开函数
// ============================================================

/// GGUF_Load_Model(start, end, model_path, device)
///
/// 从 GGUF 模型文件路径读取模型，选择 start 层到 end 层的权重加载并组装。
///
/// 层编号规则（与 GGUF_Load_Layer 一致）：
///   0           = 输入层 (embedding, token_embd.weight)
///   1..=N       = 中间层 (transformer block, blk.0 ~ blk.N-1)
///   N+1         = 输出层 (output_norm.weight + output.weight)
pub fn GGUF_Load_Model(
    start: usize,
    end: usize,
    model_path: &Path,
    device: &Device,
) -> Result<GGUF_Model> {
    // 0. 路径解析
    let resolved = if model_path.exists() {
        model_path.to_path_buf()
    } else {
        let pgguf = model_path.with_extension("pgguf");
        if pgguf.exists() {
            pgguf
        } else {
            let gguf = model_path.with_extension("gguf");
            if gguf.exists() {
                gguf
            } else {
                anyhow::bail!(
                    "Model file not found: '{}', '{}.pgguf', or '{}.gguf'",
                    model_path.display(), model_path.display(), model_path.display()
                )
            }
        }
    };

    // 1. GGUF → PGGUF 转换（仅首次）
    let (_arch_info, actual_path) = GGUF_Analyze_And_Convert(&resolved)?;
    let model_path = &actual_path;

    // 2. 打开文件并解析
    let mut file = std::fs::File::open(model_path)?;
    let content = gguf_file::Content::read(&mut file)
        .map_err(|e| anyhow::anyhow!("Failed to read GGUF content: {e}"))?;

    let arch_info = GGUF_Analyze_From_Content(&content)?;
    let model_dtype = Read_Model_Dtype(&content.metadata);

    // 3. 架构校验
    let architecture = arch_info.architecture.to_lowercase();
    let is_qwen3_moe = match architecture.as_str() {
        "qwen3" => false,
        "qwen3moe" => true,
        _ => anyhow::bail!(
            "Unsupported model architecture: '{}'. Currently only 'qwen3' and 'qwen3moe' are supported.",
            arch_info.architecture
        ),
    };

    // 4. 范围校验
    let max_layer_index = arch_info.num_layers + 1;
    if start > end {
        anyhow::bail!("Invalid layer range: start ({}) > end ({})", start, end);
    }
    let end = if arch_info.is_split {
        if start < arch_info.split_start || end > arch_info.split_end {
            anyhow::bail!(
                "Layer range [{}, {}] exceeds split model range [{}, {}]",
                start, end, arch_info.split_start, arch_info.split_end
            );
        }
        end
    } else {
        end.min(max_layer_index)
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
        .map_err(|e| anyhow::anyhow!("Failed to build RotaryEmbedding: {e}"))?,
    );

    // 6. 输入层
    let has_input_head = start == 0;
    let embed_tokens = if has_input_head {
        let mut lw = GGUF_Load_Layer(&content, &mut file, 0, device)?;
        let embed_qtensor = lw.tensors.remove("token_embd.weight")
            .ok_or_else(|| anyhow::anyhow!("Layer 0 does not contain token_embd.weight"))?;
        let embed_tensor = embed_qtensor
            .dequantize(device)
            .map_err(|e| anyhow::anyhow!("Failed to dequantize embedding: {e}"))?;
        Some(candle_nn::Embedding::new(embed_tensor, arch_info.embedding_length))
    } else {
        None
    };

    // 7. 逐层加载
    let block_start = std::cmp::max(start, 1);
    let block_end = std::cmp::min(end, arch_info.num_layers);
    let has_output_head = end == max_layer_index;

    // 8. 输出层
    let output_head = if has_output_head {
        let (n, h) = Load_Output_Head(&content, &mut file, max_layer_index,
            arch_info.rms_norm_eps, device)?;
        Some((n, h))
    } else {
        None
    };

    if is_qwen3_moe {
        let moe_cfg = Qwen3MoE_Model::MoE_Cfg_From_Metadata(&content.metadata, &architecture, arch_info.head_dim)
            .map_err(|e| anyhow::anyhow!("MoE config extraction failed: {e}"))?;

        let stages = Qwen3MoE_Model::Load_Stages(
            &content, &mut file, block_start, block_end,
            embed_tokens, output_head, &arch_info, rotary,
            &moe_cfg, model_dtype, device,
        )?;
        let model = Qwen3MoE_Model::Build_Model(stages, device.clone(), model_dtype);
        return Ok(Build_GGUF_Model(Box::new(model), arch_info, model_path.to_path_buf(),
            device.clone(), has_input_head, has_output_head));
    }

    // ── Qwen3 Dense ──
    let stages = Model_Weights::Load_Stages(
        &content, &mut file, block_start, block_end,
        embed_tokens, output_head, &arch_info, rotary, device,
    )?;
    let model = Model_Weights::Build_Model(stages, device.clone(), model_dtype);
    Ok(Build_GGUF_Model(Box::new(model), arch_info, model_path.to_path_buf(),
        device.clone(), has_input_head, has_output_head))
}

/// GGUF_Unload_Model — 卸载模型，释放资源
pub fn GGUF_Unload_Model(model: GGUF_Model) -> bool {
    drop(model);
    true
}

/// GGUF_Model_Inference — 单步前向推理
///
/// 只执行一次完整的模型前向传播，自回归循环由调用方管理。
pub fn GGUF_Model_Inference(
    model: &mut GGUF_Model,
    input: &Tensor,
    offset: usize,
) -> Result<Tensor> {
    model.model.Forward(input, offset)
        .map_err(|e| anyhow::anyhow!("Forward pass failed: {e}"))
}

/// GGUF_Model_Clear_KV_Cache — 清除模型内部 KV Cache
///
/// 在每轮对话开始前调用，确保新旧对话的 KV Cache 不冲突。
pub fn GGUF_Model_Clear_KV_Cache(model: &mut GGUF_Model) {
    model.model.Clear_Kv_Cache();
}
