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
use candle_core::quantized::{gguf_file, QTensor};
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::gguf_model_manager::{GGUF_Analyze_And_Convert, GGUF_Analyze_From_Content, GGUF_Load_Layer, Model_Arch_Info};
use super::gguf_models::{Layer_Weights, Model_Weights, Rotary_Embedding, Attention_Weights, Mlp_Weights};
use super::gguf_models::deepseek_v3::{DeepSeek_Model, DeepSeek_Layer, DeepSeek_Config};
use super::gguf_models::qwen3_moe::{Qwen3MoE_Model, Qwen3MoE_Layer, MoeOrMlp};
use candle_transformers::fused_moe::{FusedMoeGGUF, MoeCfg};
use candle_nn::Linear;

// ============================================================
// 数据结构定义
// ============================================================

/// 多架构模型枚举 — 统一 Qwen3 和 DeepSeek 模型
pub enum AnyModel {
    Qwen3(Model_Weights),
    Qwen3Moe(Qwen3MoE_Model),
    DeepSeek(DeepSeek_Model),
}

impl AnyModel {
    pub fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor> {
        match self {
            AnyModel::Qwen3(m) => m.Forward(input, offset).map_err(|e| anyhow::anyhow!("{e}")),
            AnyModel::Qwen3Moe(m) => m.Forward(input, offset).map_err(|e| anyhow::anyhow!("{e}")),
            AnyModel::DeepSeek(m) => m.Forward(input, offset).map_err(|e| anyhow::anyhow!("{e}")),
        }
    }

    pub fn Clear_Kv_Cache(&mut self) {
        match self {
            AnyModel::Qwen3(m) => m.clear_kv_cache(),
            AnyModel::Qwen3Moe(m) => m.Clear_Kv_Cache(),
            AnyModel::DeepSeek(m) => m.Clear_Kv_Cache(),
        }
    }

    /// 提取所有层的 KV Cache，返回 Vec<(tensor_a, tensor_b)>
    /// - Qwen3/Qwen3Moe: (k, v)
    /// - DeepSeek V3:    (kv_latent, k_pe)
    /// 返回的 Tensor 与原模型共享存储（clone 仅增加引用计数）
    pub fn extract_kv_cache(&self) -> Result<Vec<(Tensor, Tensor)>, String> {
        match self {
            AnyModel::Qwen3(m) => {
                if m.layers.is_empty() {
                    return Err("extract_kv_cache: model has no layers".into());
                }
                m.layers.iter().map(|layer| {
                    let k = layer.self_attn.kv_cache.k()
                        .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
                    let v = layer.self_attn.kv_cache.v()
                        .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
                    Ok((k.clone(), v.clone()))
                }).collect()
            }
            AnyModel::Qwen3Moe(m) => {
                if m.layers.is_empty() {
                    return Err("extract_kv_cache: model has no layers".into());
                }
                m.layers.iter().map(|layer| {
                    let k = layer.self_attn.kv_cache.k()
                        .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
                    let v = layer.self_attn.kv_cache.v()
                        .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
                    Ok((k.clone(), v.clone()))
                }).collect()
            }
            AnyModel::DeepSeek(m) => {
                if m.layers.is_empty() {
                    return Err("extract_kv_cache: model has no layers".into());
                }
                m.layers.iter().map(|layer| {
                    let kv_latent = layer.mla.kv_cache.kv_latent.clone()
                        .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
                    let k_pe = layer.mla.kv_cache.k_pe.clone()
                        .ok_or_else(|| "extract_kv_cache: KV cache is empty (no inference done yet)".to_string())?;
                    Ok((kv_latent, k_pe))
                }).collect()
            }
        }
    }

    /// 恢复所有层的 KV Cache（假设当前 cache 为空，即刚加载的模型）
    pub fn restore_kv_cache(&mut self, kvs: Vec<(Tensor, Tensor)>) -> Result<(), String> {
        match self {
            AnyModel::Qwen3(m) => {
                if kvs.len() != m.layers.len() {
                    return Err(format!("restore_kv_cache: layer count mismatch (expected {}, got {})",
                        m.layers.len(), kvs.len()));
                }
                for (layer, (k, v)) in m.layers.iter_mut().zip(kvs) {
                    layer.self_attn.kv_cache.append(&k, &v)
                        .map_err(|e| format!("restore_kv_cache: append failed: {e}"))?;
                }
            }
            AnyModel::Qwen3Moe(m) => {
                if kvs.len() != m.layers.len() {
                    return Err(format!("restore_kv_cache: layer count mismatch (expected {}, got {})",
                        m.layers.len(), kvs.len()));
                }
                for (layer, (k, v)) in m.layers.iter_mut().zip(kvs) {
                    layer.self_attn.kv_cache.append(&k, &v)
                        .map_err(|e| format!("restore_kv_cache: append failed: {e}"))?;
                }
            }
            AnyModel::DeepSeek(m) => {
                if kvs.len() != m.layers.len() {
                    return Err(format!("restore_kv_cache: layer count mismatch (expected {}, got {})",
                        m.layers.len(), kvs.len()));
                }
                for (layer, (kv_latent, k_pe)) in m.layers.iter_mut().zip(kvs) {
                    layer.mla.kv_cache.Append(&kv_latent, &k_pe)
                        .map_err(|e| format!("restore_kv_cache: append failed: {e}"))?;
                }
            }
        }
        Ok(())
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

/// 组装好的 GGUF 模型，包含模型权重、tokenizer、推理配置以及架构信息
///
/// 此结构体对 ML_Engine 层提供统一的模型抽象：
/// - 包含完整的模型权重（embedding + 所有层 + norm + lm_head）
/// - 包含 tokenizer（如果 GGUF 文件中存在）
/// - 包含 Inference_Config，由上层配置
/// - tokenizer 作为独立 API 供外部组件调用，不耦合在推理过程中
pub struct GGUF_Model {
    /// 组装好的模型权重（包含 embedding + 所有层 + norm + lm_head）
    pub model: AnyModel,
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

/// 从已加载的 QTensors 中构造 Mlp_Weights
fn Mlp_From_Tensors(tensors: &mut HashMap<String, QTensor>, prefix: &str) -> anyhow::Result<Mlp_Weights> {
    let gate_proj = {
        let qt = tensors.remove(&format!("{prefix}.ffn_gate.weight"))
            .ok_or_else(|| anyhow::anyhow!("{prefix}.ffn_gate.weight not found"))?;
        QMatMul::from_weights(Arc::new(qt))
            .map_err(|e| anyhow::anyhow!("{prefix}.ffn_gate.weight: {e}"))?
    };
    let up_proj = {
        let qt = tensors.remove(&format!("{prefix}.ffn_up.weight"))
            .ok_or_else(|| anyhow::anyhow!("{prefix}.ffn_up.weight not found"))?;
        QMatMul::from_weights(Arc::new(qt))
            .map_err(|e| anyhow::anyhow!("{prefix}.ffn_up.weight: {e}"))?
    };
    let down_proj = {
        let qt = tensors.remove(&format!("{prefix}.ffn_down.weight"))
            .ok_or_else(|| anyhow::anyhow!("{prefix}.ffn_down.weight not found"))?;
        QMatMul::from_weights(Arc::new(qt))
            .map_err(|e| anyhow::anyhow!("{prefix}.ffn_down.weight: {e}"))?
    };

    Ok(Mlp_Weights {
        gate_proj,
        up_proj,
        down_proj,
        act_fn: candle_nn::Activation::Silu,
        span: tracing::span!(tracing::Level::TRACE, "mlp"),
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

// ============================================================
// 核心公开函数
// ============================================================

/// GGUF_Load_Model(start, end, model_path, device)
///
/// 从 GGUF 模型文件路径读取模型，解析模型信息，根据其信息调用具体的模型部件,
/// 选择 start 层到 end 层的权重加载然后进行组装
/// 并放置在对应设备上，然后返回一个可供推理的模型。
///
/// 层编号规则（与 GGUF_Load_Layer 一致）：
///   0           = 输入层 (embedding, token_embd.weight)
///   1..=N       = 中间层 (transformer block, blk.0 ~ blk.N-1)
///   N+1         = 输出层 (output_norm.weight + output.weight)
///
/// # 步骤
/// 1. 调用 GGUF_Analyze 解析模型文件，获取模型架构信息
/// 2. 根据模型架构信息匹配模型架构（目前仅支持 Qwen3）
/// 3. 如果是 split 文件（pgguf），判断 start 和 end 是否超出当前文件包含的层范围；
///    如果是完整 gguf 文件，判断 start 和 end 是否在 [0, N+1] 范围内
/// 4. 构建 RotaryEmbedding
/// 5. 如果 start==0，表示包含输入层，加载 embedding 和 tokenizer
/// 6. 根据 start 和 end，调用 GGUF_Load_Layer 逐层加载 transformer block 权重并组装
/// 7. 如果 end==N+1，表示包含输出层，加载 output_norm 和 lm_head
/// 8. 将组装好的模型放置在对应设备上
/// 9. 返回组装好的模型（含默认 Inference_Config）
pub fn GGUF_Load_Model(
    start: usize,
    end: usize,
    model_path: &Path,
    device: &Device,
) -> Result<GGUF_Model> {
    // 0. 路径解析：直接路径不存在时，自动尝试 .pgguf / .gguf 后缀
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
                    model_path.display(),
                    model_path.display(),
                    model_path.display()
                )
            }
        }
    };

    // 1. 自动 GGUF → PGGUF 转换（仅首次，已转换的文件零开销跳过）
    //    转换后 resolved 指向实际文件（可能是 .pgguf）
    let (_arch_info, actual_path) = GGUF_Analyze_And_Convert(&resolved)?;
    let model_path = &actual_path;

    // 1. 打开文件并解析 GGUF Content（仅此一次）
    let mut file = std::fs::File::open(model_path)?;
    let content = gguf_file::Content::read(&mut file)
        .map_err(|e| anyhow::anyhow!("Failed to read GGUF content: {}", e))?;

    // 从已加载的 Content 提取架构信息（零 I/O）
    let arch_info = GGUF_Analyze_From_Content(&content)?;

    // 读取模型 dtype
    let model_dtype = Read_Model_Dtype(&content.metadata);

    // 2. 根据模型架构信息，匹配对应的模型架构
    let architecture = arch_info.architecture.to_lowercase();
    let is_deepseek = match architecture.as_str() {
        "qwen3" | "qwen3moe" => false,
        "deepseek_v3" | "deepseek2" => true,
        _ => anyhow::bail!(
            "Unsupported model architecture: '{}'. Currently 'qwen3', 'qwen3moe', and 'deepseek_v3' are supported.",
            arch_info.architecture
        ),
    };
    let is_qwen3_moe = architecture.as_str() == "qwen3moe";

    // 3. 范围校验
    let max_layer_index = arch_info.num_layers + 1; // N+1 = output 层

    if start > end {
        anyhow::bail!("Invalid layer range: start ({}) > end ({})", start, end);
    }

    // 对于非 split 文件，将 end 钳位到 max_layer_index（支持 usize::MAX 作为 "全部加载" 哨兵）
    let end = if arch_info.is_split {
        // 对于 split 文件，判断 [start, end] 是否在 [split_start, split_end] 范围内
        if start < arch_info.split_start || end > arch_info.split_end {
            anyhow::bail!(
                "Layer range [{}, {}] exceeds split model range [{}, {}]",
                start,
                end,
                arch_info.split_start,
                arch_info.split_end
            );
        }
        end
    } else {
        // 对于完整 gguf 文件，钳位 end 到 [0, N+1]
        end.min(max_layer_index)
    };

    // 4. 构建 RotaryEmbedding
    //    DeepSeek 的 RoPE 应用于 head_dim = qk_rope_dim（解耦部分），而非完整 head_dim
    let rope_head_dim = if is_deepseek {
        // DeepSeek: 从 metadata 获取 qk_rope_dim
        super::gguf_model_manager::Get_Metadata_Usize_From_Map(
            &content.metadata,
            &format!("{}.attention.qk_rope_head_dim", architecture),
        )
        .unwrap_or(64)
    } else {
        arch_info.head_dim
    };

    let rotary = Arc::new(
        Rotary_Embedding::New(
            DType::F32,
            rope_head_dim,
            arch_info.context_length,
            arch_info.rope_freq_base,
            device,
        )
        .map_err(|e| anyhow::anyhow!("Failed to build RotaryEmbedding: {e}"))?,
    );

    // 5. 如果 start==0，表示包含输入层，加载 embedding；否则为 None
    let has_input_head = start == 0;

    let embed_tokens: Option<candle_nn::Embedding> =
        if has_input_head {
            let mut lw = GGUF_Load_Layer(&content, &mut file, 0, device)?;
            let embed_qtensor = lw
                .tensors
                .remove("token_embd.weight")
                .ok_or_else(|| anyhow::anyhow!("Layer 0 does not contain token_embd.weight"))?;
            let embed_tensor = embed_qtensor
                .dequantize(device)
                .map_err(|e| anyhow::anyhow!("Failed to dequantize embedding: {e}"))?;

            Some(candle_nn::Embedding::new(
                embed_tensor,
                arch_info.embedding_length,
            ))
        } else {
            None
        };

    // 6. 根据 start 和 end，调用 GGUF_Load_Layer 逐层加载 transformer block 层
    let block_start = std::cmp::max(start, 1);
    let block_end = std::cmp::min(end, arch_info.num_layers);
    let block_count = if block_start <= block_end {
        block_end - block_start + 1
    } else {
        0
    };

    // 7. 如果 end==N+1，表示包含输出层
    let has_output_head = end == max_layer_index;

    if is_qwen3_moe {
        // ── Qwen3 MoE 加载路径 ──
        let moe_cfg = MoE_Cfg_From_Metadata(&content.metadata, &architecture, arch_info.head_dim)
            .map_err(|e| anyhow::anyhow!("MoE config extraction failed: {e}"))?;

        let mut layers = Vec::with_capacity(block_count);
        for i in block_start..=block_end {
            let mut lw = GGUF_Load_Layer(&content, &mut file, i, device)?;
            let blk_idx = i - 1;
            let prefix = format!("blk.{blk_idx}");

            // FFN: 判断该层是 MoE 还是 Dense
            let mlp = if moe_cfg.num_experts > 0 {
                // 加载 MoE 权重
                let gate_qt = lw.tensors.remove(&format!("{prefix}.ffn_gate_inp.weight"))
                    .ok_or_else(|| anyhow::anyhow!("{prefix}.ffn_gate_inp.weight not found"))?;
                let gate_ws = gate_qt.dequantize(device)?.to_dtype(DType::F32)?;
                let gate = Linear::new(gate_ws, None);

                let gate_experts = Arc::new(
                    lw.tensors.remove(&format!("{prefix}.ffn_gate_exps.weight"))
                        .ok_or_else(|| anyhow::anyhow!("{prefix}.ffn_gate_exps.weight not found"))?
                );
                let up_experts = Arc::new(
                    lw.tensors.remove(&format!("{prefix}.ffn_up_exps.weight"))
                        .ok_or_else(|| anyhow::anyhow!("{prefix}.ffn_up_exps.weight not found"))?
                );
                let down_experts = Arc::new(
                    lw.tensors.remove(&format!("{prefix}.ffn_down_exps.weight"))
                        .ok_or_else(|| anyhow::anyhow!("{prefix}.ffn_down_exps.weight not found"))?
                );

                let fused_moe = FusedMoeGGUF {
                    gate,
                    gate_experts,
                    up_experts,
                    down_experts,
                    act: candle_nn::Activation::Silu,
                    norm_topk_prob: moe_cfg.norm_topk_prob,
                    num_experts_per_tok: moe_cfg.num_experts_per_tok,
                    dtype: model_dtype,
                };
                MoeOrMlp::MoE(Arc::new(fused_moe))
            } else {
                // Dense 层
                let mlp = Mlp_From_Tensors(&mut lw.tensors, &prefix)?;
                MoeOrMlp::Mlp(mlp)
            };

            // Attention: 从 tensors 内联构造
            let attn = {
                fn take_qmatmul(tensors: &mut HashMap<String, QTensor>, key: &str) -> anyhow::Result<QMatMul> {
                    let qt = tensors.remove(key)
                        .ok_or_else(|| anyhow::anyhow!("missing tensor: {key}"))?;
                    QMatMul::from_weights(Arc::new(qt))
                        .map_err(|e| anyhow::anyhow!("qmatmul {key}: {e}"))
                }
                let q_proj = take_qmatmul(&mut lw.tensors, &format!("{prefix}.attn_q.weight"))?;
                let k_proj = take_qmatmul(&mut lw.tensors, &format!("{prefix}.attn_k.weight"))?;
                let v_proj = take_qmatmul(&mut lw.tensors, &format!("{prefix}.attn_v.weight"))?;
                let o_proj = take_qmatmul(&mut lw.tensors, &format!("{prefix}.attn_output.weight"))?;
                let q_norm = take_rmsnorm(&mut lw.tensors, &format!("{prefix}.attn_q_norm.weight"), arch_info.rms_norm_eps)?;
                let k_norm = take_rmsnorm(&mut lw.tensors, &format!("{prefix}.attn_k_norm.weight"), arch_info.rms_norm_eps)?;

                Attention_Weights {
                    q_proj, k_proj, v_proj, o_proj,
                    q_norm, k_norm,
                    num_heads: arch_info.head_count,
                    num_kv_heads: arch_info.head_count_kv,
                    num_kv_groups: arch_info.head_count / arch_info.head_count_kv,
                    head_dim: arch_info.head_dim,
                    rotary_emb: rotary.clone(),
                    kv_cache: candle_nn::kv_cache::ConcatKvCache::new(2),
                    span_attn: tracing::span!(tracing::Level::TRACE, "attn"),
                }
            };

            // Norms
            fn take_rmsnorm(tensors: &mut HashMap<String, QTensor>, key: &str, eps: f64) -> anyhow::Result<RmsNorm> {
                let qt = tensors.remove(key)
                    .ok_or_else(|| anyhow::anyhow!("missing tensor: {key}"))?;
                RmsNorm::from_qtensor(qt, eps)
                    .map_err(|e| anyhow::anyhow!("rmsnorm {key}: {e}"))
            }

            let ln1 = take_rmsnorm(&mut lw.tensors, &format!("{prefix}.attn_norm.weight"), arch_info.rms_norm_eps)?;
            let ln2 = take_rmsnorm(&mut lw.tensors, &format!("{prefix}.ffn_norm.weight"), arch_info.rms_norm_eps)?;

            layers.push(Qwen3MoE_Layer { self_attn: attn, ln1, mlp, ln2 });
        }

        let (norm, lm_head): (Option<RmsNorm>, Option<QMatMul>) = if has_output_head {
            let mut lw = GGUF_Load_Layer(&content, &mut file, max_layer_index, device)?;
            let norm_qtensor = lw.tensors.remove("output_norm.weight")
                .ok_or_else(|| anyhow::anyhow!("Output layer does not contain output_norm.weight"))?;
            let norm = RmsNorm::from_qtensor(norm_qtensor, arch_info.rms_norm_eps)
                .map_err(|e| anyhow::anyhow!("Failed to build output RmsNorm: {e}"))?;
            let lm_head_qtensor = if let Some(qt) = lw.tensors.remove("output.weight") {
                qt
            } else {
                let mut embed_lw = GGUF_Load_Layer(&content, &mut file, 0, device)?;
                embed_lw.tensors.remove("token_embd.weight")
                    .ok_or_else(|| anyhow::anyhow!("output.weight not found and token_embd fallback failed"))?
            };
            let lm_head = QMatMul::from_weights(lm_head_qtensor.into())
                .map_err(|e| anyhow::anyhow!("Failed to build lm_head: {e}"))?;
            (Some(norm), Some(lm_head))
        } else {
            (None, None)
        };

        let model = AnyModel::Qwen3Moe(Qwen3MoE_Model::From_Dynamic(
            embed_tokens, layers, norm, lm_head,
            device.clone(), model_dtype,
        ));

        let mut inference_config = Inference_Config::default();
        inference_config.eos_token = arch_info.eos_token_id;

        return Ok(GGUF_Model {
            model,
            tokenizer: None,
            inference_config,
            arch_info,
            model_path: model_path.to_path_buf(),
            device: device.clone(),
            has_input_head,
            has_output_head,
        });
    }

    if is_deepseek {
        // ── DeepSeek V3.2 加载路径 ──
        let ds_config = DeepSeek_Config::From_Metadata(
            &content.metadata,
            &architecture,
        )?;

        let mut layers = Vec::with_capacity(block_count);
        for i in block_start..=block_end {
            let mut lw = GGUF_Load_Layer(&content, &mut file, i, device)?;
            let blk_idx = i - 1;
            let layer = DeepSeek_Layer::From_Extracted(
                &mut lw.tensors,
                ds_config.n_heads,
                ds_config.q_lora_rank,
                ds_config.kv_lora_rank,
                ds_config.qk_rope_dim,
                ds_config.qk_nope_dim,
                ds_config.v_head_dim,
                ds_config.n_routed_experts,
                ds_config.top_k,
                ds_config.routed_scaling_factor,
                ds_config.rms_norm_eps,
                rotary.clone(),
                blk_idx,
            )
            .map_err(|e| anyhow::anyhow!("Layer {} (blk.{}) assembly failed: {}", i, blk_idx, e))?;
            layers.push(layer);
        }

        let (norm, lm_head): (Option<RmsNorm>, Option<QMatMul>) = if has_output_head {
            let mut lw = GGUF_Load_Layer(&content, &mut file, max_layer_index, device)?;
            let norm_qtensor = lw.tensors.remove("output_norm.weight")
                .ok_or_else(|| anyhow::anyhow!("Output layer does not contain output_norm.weight"))?;
            let norm = RmsNorm::from_qtensor(norm_qtensor, ds_config.rms_norm_eps)
                .map_err(|e| anyhow::anyhow!("Failed to build output RmsNorm: {e}"))?;

            let lm_head_qtensor = if let Some(qt) = lw.tensors.remove("output.weight") {
                qt
            } else {
                let mut embed_lw = GGUF_Load_Layer(&content, &mut file, 0, device)?;
                embed_lw.tensors.remove("token_embd.weight")
                    .ok_or_else(|| anyhow::anyhow!("output.weight not found and token_embd fallback failed"))?
            };
            let lm_head = QMatMul::from_weights(lm_head_qtensor.into())
                .map_err(|e| anyhow::anyhow!("Failed to build lm_head: {e}"))?;
            (Some(norm), Some(lm_head))
        } else {
            (None, None)
        };

        let model = AnyModel::DeepSeek(DeepSeek_Model::From_Dynamic(
            embed_tokens, layers, norm, lm_head,
            device.clone(), model_dtype,
        ));

        let mut inference_config = Inference_Config::default();
        inference_config.eos_token = arch_info.eos_token_id;

        return Ok(GGUF_Model {
            model,
            tokenizer: None,
            inference_config,
            arch_info,
            model_path: model_path.to_path_buf(),
            device: device.clone(),
            has_input_head,
            has_output_head,
        });
    }

    // ── Qwen3 加载路径（现有逻辑）───────────────────────

    let mut layers = Vec::with_capacity(block_count);
    for i in block_start..=block_end {
        let mut lw = GGUF_Load_Layer(&content, &mut file, i, device)?;
        // blk index = layer_index - 1（层 1 -> blk.0, 层 2 -> blk.1, ...）
        let blk_idx = i - 1;
        let layer = Layer_Weights::From_Extracted(
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

    // 7. 如果 end==N+1，表示包含输出层，加载 output_norm 和 lm_head
    let has_output_head = end == max_layer_index;

    let (norm, lm_head): (Option<RmsNorm>, Option<QMatMul>) = if has_output_head {
        // 通过 GGUF_Load_Layer 加载第 N+1 层（output）
        let mut lw = GGUF_Load_Layer(&content, &mut file, max_layer_index, device)?;

        let norm_qtensor = lw
            .tensors
            .remove("output_norm.weight")
            .ok_or_else(|| anyhow::anyhow!("Output layer does not contain output_norm.weight"))?;
        let norm = RmsNorm::from_qtensor(norm_qtensor, arch_info.rms_norm_eps)
            .map_err(|e| anyhow::anyhow!("Failed to build output RmsNorm: {}", e))?;

        // 尝试加载 output.weight，如果不存在则 fallback 到 token_embd.weight（weight tying）
        let lm_head_qtensor = if let Some(qt) = lw.tensors.remove("output.weight") {
            qt
        } else {
            // Weight tying: 使用 embedding 权重作为 lm_head
            let mut embed_lw = GGUF_Load_Layer(&content, &mut file, 0, device).map_err(|e| {
                anyhow::anyhow!("Failed to load embedding for lm_head fallback: {}", e)
            })?;
            embed_lw
                .tensors
                .remove("token_embd.weight")
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "output.weight not found and token_embd.weight fallback also failed"
                    )
                })?
        };
        let lm_head = QMatMul::from_weights(lm_head_qtensor.into())
            .map_err(|e| anyhow::anyhow!("Failed to build lm_head QMatMul: {}", e))?;

        (Some(norm), Some(lm_head))
    } else {
        (None, None)
    };

    // 8. 组装模型并放置在对应设备上
    let model = AnyModel::Qwen3(Model_Weights::From_Dynamic(
        embed_tokens,
        layers,
        norm,
        lm_head,
        device.clone(),
        model_dtype,
    ));

    // 9. 返回组装好的模型，eos_token 从模型 metadata 中自动获取
    let mut inference_config = Inference_Config::default();
    inference_config.eos_token = arch_info.eos_token_id;

    Ok(GGUF_Model {
        model,
        tokenizer: None,  // tokenizer 不再由 GGUF_Load_Model 加载，请使用 MlSession::load_tokenizer()
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

/// GGUF_Model_Inference(model, input, offset)
///
/// 对输入 Tensor 进行单步前向推理（让模型所有层推理一次）。
/// 此函数不进行自回归循环，只执行一次完整的模型前向传播。
///
/// 自回归生成循环由调用方负责管理。
///
/// 模型内部（Model_Weights::Forward）根据自身结构自动适配输入：
/// - 有 embedding 层 (embed_tokens): input 为 token IDs 张量 [batch, seq_len]（u32）
///   → 自动经过 embedding → transformer blocks → 输出
/// - 无 embedding 层: input 为 hidden state 张量 [batch, seq_len, hidden_dim]（f32）
///   → 直接输入 transformer blocks → 输出
///
/// # 参数
/// - `model`: 组装好的 GGUF 模型（可变引用，因为 KV cache 会更新）
/// - `input`: 输入 Tensor（由调用方根据场景构造）
/// - `offset`: 位置偏移量，用于 KV cache 和位置编码（自回归生成时递增）
///
/// # 返回
/// 模型输出 Tensor（logits 或 hidden state，取决于模型是否包含输出头）
pub fn GGUF_Model_Inference(
    model: &mut GGUF_Model,
    input: &Tensor,
    offset: usize,
) -> Result<Tensor> {
    let result = model
        .model
        .Forward(input, offset)
        .map_err(|e| anyhow::anyhow!("Forward pass failed: {e}"))?;

    Ok(result)
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
        anyhow::anyhow!("Model does not have a tokenizer loaded. Cannot encode text.")
    })?;

    let format_prompt = format!("<|im_start|>user\n{input}<|im_end|>\n<|im_start|>assistant\n");

    let opts = shimmytok::EncodeOptions::with_parse_special(true, true);
    let token_ids = tokenizer
        .encode_with_options(&format_prompt, &opts)
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
        anyhow::anyhow!("Model does not have a tokenizer loaded. Cannot decode tokens.")
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
