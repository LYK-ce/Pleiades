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
use super::gguf_models::{Layer_Weights, Model_Weights, Rotary_Embedding};
use super::gguf_models::qwen3_moe::{Qwen3MoE_Model, Qwen3MoE_Layer};
use candle_transformers::fused_moe::MoeCfg;

// ============================================================
// 数据结构定义
// ============================================================

/// 多架构模型枚举 — 当前仅 Qwen3 / Qwen3MoE
pub enum AnyModel {
    Qwen3(Model_Weights),
    Qwen3Moe(Qwen3MoE_Model),
}

impl AnyModel {
    pub fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor> {
        match self {
            AnyModel::Qwen3(m) => m.Forward(input, offset).map_err(|e| anyhow::anyhow!("{e}")),
            AnyModel::Qwen3Moe(m) => m.Forward(input, offset).map_err(|e| anyhow::anyhow!("{e}")),
        }
    }

    pub fn Clear_Kv_Cache(&mut self) {
        match self {
            AnyModel::Qwen3(m) => m.Clear_Kv_Cache(),
            AnyModel::Qwen3Moe(m) => m.Clear_Kv_Cache(),
        }
    }

    /// 提取所有层的 KV Cache，返回 Vec<(k, v)>
    /// 返回的 Tensor 与原模型共享存储（clone 仅增加引用计数）
    pub fn extract_kv_cache(&self) -> Result<Vec<(Tensor, Tensor)>, String> {
        match self {
            AnyModel::Qwen3(m) => extract_qwen_kv_cache(&m.layers),
            AnyModel::Qwen3Moe(m) => extract_qwen_kv_cache(&m.layers),
        }
    }

    /// 恢复所有层的 KV Cache（假设当前 cache 为空，即刚加载的模型）
    pub fn restore_kv_cache(&mut self, kvs: Vec<(Tensor, Tensor)>) -> Result<(), String> {
        match self {
            AnyModel::Qwen3(m) => restore_qwen_kv_cache(&mut m.layers, kvs),
            AnyModel::Qwen3Moe(m) => restore_qwen_kv_cache(&mut m.layers, kvs),
        }
    }
}

/// 提取 Qwen3 / Qwen3MoE 层 KV Cache 的共用实现
fn extract_qwen_kv_cache<T: LayerWithAttention>(
    layers: &[T],
) -> Result<Vec<(Tensor, Tensor)>, String> {
    if layers.is_empty() {
        return Err("extract_kv_cache: model has no layers".into());
    }
    layers.iter().map(|layer| {
        let cache = layer.attention_kv_cache();
        let k = cache.k()
            .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
        let v = cache.v()
            .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
        Ok((k.clone(), v.clone()))
    }).collect()
}

/// 恢复 Qwen3 / Qwen3MoE 层 KV Cache 的共用实现
///
/// 注意：Qwen3MoE_Layer 的 self_attn 也是 Attention_Weights，
/// 这里通过泛型约束接受任何包含 self_attn 的类型
fn restore_qwen_kv_cache<T: LayerWithAttention>(
    layers: &mut [T],
    kvs: Vec<(Tensor, Tensor)>,
) -> Result<(), String> {
    if kvs.len() != layers.len() {
        return Err(format!(
            "restore_kv_cache: layer count mismatch (expected {}, got {})",
            layers.len(), kvs.len()
        ));
    }
    for (layer, (k, v)) in layers.iter_mut().zip(kvs) {
        layer.attention_kv_cache_mut().append(&k, &v)
            .map_err(|e| format!("restore_kv_cache: append failed: {e}"))?;
    }
    Ok(())
}

/// 辅助 trait：允许 extract/restore kv_cache 在 Qwen3 和 Qwen3MoE 层上统一操作
trait LayerWithAttention {
    fn attention_kv_cache(&self) -> &candle_nn::kv_cache::ConcatKvCache;
    fn attention_kv_cache_mut(&mut self) -> &mut candle_nn::kv_cache::ConcatKvCache;
}

impl LayerWithAttention for Layer_Weights {
    fn attention_kv_cache(&self) -> &candle_nn::kv_cache::ConcatKvCache {
        &self.self_attn.kv_cache
    }
    fn attention_kv_cache_mut(&mut self) -> &mut candle_nn::kv_cache::ConcatKvCache {
        &mut self.self_attn.kv_cache
    }
}

impl LayerWithAttention for Qwen3MoE_Layer {
    fn attention_kv_cache(&self) -> &candle_nn::kv_cache::ConcatKvCache {
        &self.self_attn.kv_cache
    }
    fn attention_kv_cache_mut(&mut self) -> &mut candle_nn::kv_cache::ConcatKvCache {
        &mut self.self_attn.kv_cache
    }
}

/// 推理参数—仅保留模型元数据（运行时参数由 Pipeline_Params 提供）
pub struct Inference_Config {
    pub eos_token: u32,
}

impl Default for Inference_Config {
    fn default() -> Self {
        Self { eos_token: 151645 }
    }
}

/// 组装好的 GGUF 模型
///
/// 包含模型权重、tokenizer、推理配置以及架构信息。
pub struct GGUF_Model {
    pub model: AnyModel,
    pub tokenizer: Option<shimmytok::Tokenizer>,
    pub inference_config: Inference_Config,
    pub arch_info: Model_Arch_Info,
    pub model_path: PathBuf,
    pub device: Device,
    pub has_input_head: bool,
    pub has_output_head: bool,
}

// ============================================================
// 架构配置提取辅助函数
// ============================================================

/// 从 GGUF metadata 提取 MoE 配置
fn MoE_Cfg_From_Metadata(
    metadata: &std::collections::HashMap<String, gguf_file::Value>,
    arch: &str,
    hidden_size: usize,
) -> anyhow::Result<MoeCfg> {
    let md_get_usize = |key: &str| -> anyhow::Result<usize> {
        metadata
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("missing metadata key: {key}"))
            .and_then(|v| {
                v.to_u32()
                    .map(|u| u as usize)
                    .or_else(|_| v.to_u64().map(|u| u as usize))
                    .map_err(|_| anyhow::anyhow!("cannot convert {key} to usize"))
            })
    };

    let expert_count = md_get_usize(&format!("{arch}.expert_count")).unwrap_or(0);
    let expert_used_count = md_get_usize(&format!("{arch}.expert_used_count")).unwrap_or(0);
    let expert_feed_forward_length =
        md_get_usize(&format!("{arch}.expert_feed_forward_length")).unwrap_or(0);

    Ok(MoeCfg {
        moe_intermediate_size: expert_feed_forward_length,
        num_experts: expert_count,
        norm_topk_prob: expert_used_count > 0,
        num_experts_per_tok: expert_used_count.max(1),
        hidden_size,
        act: candle_nn::Activation::Silu,
        decoder_sparse_step: None,
    })
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
    any_model: AnyModel,
    arch_info: Model_Arch_Info,
    model_path: PathBuf,
    device: Device,
    has_input_head: bool,
    has_output_head: bool,
) -> GGUF_Model {
    let mut inference_config = Inference_Config::default();
    inference_config.eos_token = arch_info.eos_token_id;
    GGUF_Model {
        model: any_model,
        tokenizer: None,
        inference_config,
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
    let block_count = if block_start <= block_end { block_end - block_start + 1 } else { 0 };
    let has_output_head = end == max_layer_index;

    // 8. 输出层
    let (norm, lm_head) = if has_output_head {
        let (n, h) = Load_Output_Head(&content, &mut file, max_layer_index,
            arch_info.rms_norm_eps, device)?;
        (Some(n), Some(h))
    } else {
        (None, None)
    };

    if is_qwen3_moe {
        // ── Qwen3 MoE ──
        let moe_cfg = MoE_Cfg_From_Metadata(&content.metadata, &architecture, arch_info.head_dim)
            .map_err(|e| anyhow::anyhow!("MoE config extraction failed: {e}"))?;

        let mut layers = Vec::with_capacity(block_count);
        for i in block_start..=block_end {
            let mut lw = GGUF_Load_Layer(&content, &mut file, i, device)?;
            let blk_idx = i - 1;
            let layer = Qwen3MoE_Layer::Build_From_Extracted(
                &mut lw.tensors,
                arch_info.head_count,
                arch_info.head_count_kv,
                arch_info.head_dim,
                arch_info.rms_norm_eps,
                rotary.clone(),
                blk_idx,
                moe_cfg.num_experts > 0,
                &moe_cfg,
                model_dtype,
            )
            .map_err(|e| anyhow::anyhow!("Layer {} (blk.{}) assembly failed: {}", i, blk_idx, e))?;
            layers.push(layer);
        }

        let model = AnyModel::Qwen3Moe(Qwen3MoE_Model::Build_Model(
            embed_tokens, layers, norm, lm_head, device.clone(), model_dtype,
        ));
        return Ok(Build_GGUF_Model(model, arch_info, model_path.to_path_buf(),
            device.clone(), has_input_head, has_output_head));
    }

    // ── Qwen3 Dense ──
    let mut layers = Vec::with_capacity(block_count);
    for i in block_start..=block_end {
        let mut lw = GGUF_Load_Layer(&content, &mut file, i, device)?;
        let blk_idx = i - 1;
        let layer = Layer_Weights::Build_From_Extracted(
            &mut lw.tensors,
            arch_info.head_count,
            arch_info.head_count_kv,
            arch_info.head_dim,
            arch_info.rms_norm_eps,
            rotary.clone(),
            blk_idx,
        )
        .map_err(|e| anyhow::anyhow!("Layer {} (blk.{}) assembly failed: {}", i, blk_idx, e))?;
        layers.push(layer);
    }

    let model = AnyModel::Qwen3(Model_Weights::Build_Model(
        embed_tokens, layers, norm, lm_head, device.clone(), model_dtype,
    ));
    Ok(Build_GGUF_Model(model, arch_info, model_path.to_path_buf(),
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

// ============================================================
// Tokenizer 独立 API
// ============================================================

/// GGUF_Encode — 将文本编码为 token IDs（自动添加 Qwen3 对话格式）
pub fn GGUF_Encode(model: &GGUF_Model, input: &str) -> Result<Vec<u32>> {
    let tokenizer = model.tokenizer.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Model does not have a tokenizer loaded."))?;

    let format_prompt = format!("<|im_start|>user\n{input}<|im_end|>\n<|im_start|>assistant\n");

    let opts = shimmytok::EncodeOptions::with_parse_special(true, true);
    let token_ids = tokenizer
        .encode_with_options(&format_prompt, &opts)
        .map_err(|e| anyhow::anyhow!("Tokenization error: {e}"))?;

    Ok(token_ids)
}

/// GGUF_Decode — 将 token IDs 解码为文本（自动移除末尾 EOS）
pub fn GGUF_Decode(model: &GGUF_Model, token_ids: &[u32]) -> Result<String> {
    let tokenizer = model.tokenizer.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Model does not have a tokenizer loaded."))?;

    let eos_token = model.inference_config.eos_token;
    let decode_tokens: &[u32] = if token_ids.last() == Some(&eos_token) {
        &token_ids[..token_ids.len() - 1]
    } else {
        token_ids
    };

    let output_text = tokenizer
        .decode(decode_tokens, true)
        .map_err(|e| anyhow::anyhow!("Decoding error: {e}"))?;

    Ok(output_text)
}
