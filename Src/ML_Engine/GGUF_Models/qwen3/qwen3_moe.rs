//Presented by KeJi
//Created Date ： 2026-05-30
//Modified Date ： 2026-07-03

//! Qwen3 MoE 模型架构支持
//!
//! 基于 candle-transformers 的 `quantized_qwen3_moe.rs` 参考实现。
//! Qwen3 MoE 与 Qwen3 Dense 的 Attention 完全相同，差异仅在 FFN：
//! - 部分层使用 Dense FFN (SwiGLU gate/up/down)
//! - 部分层使用 MoE (router + shared expert + sparse experts)
//!
//! GGUF tensor 命名 (MoE 层):
//!   Router:     blk.N.ffn_gate_inp.weight
//!   Experts:    blk.N.ffn_gate_exps.weight  [n_experts, intermediate, hidden]
//!               blk.N.ffn_up_exps.weight    [n_experts, intermediate, hidden]
//!               blk.N.ffn_down_exps.weight  [n_experts, hidden, intermediate]

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use candle_core::quantized::{gguf_file, QTensor};
use candle_core::{DType, Device, Tensor};
use candle_nn::{Embedding, Module};
use candle_transformers::fused_moe::{FusedMoeGGUF, MoeCfg};
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use super::super::common::{Mlp_Weights, Rotary_Embedding, Stage, Model, extract_kv_cache_from_stages, restore_kv_cache_to_stages};
use super::{
    Attention_Weights,
    Qwen3_Embedding_Stage, Qwen3_Output_Stage,
};
use crate::ml_engine::gguf_model_manager::{GGUF_Load_Layer, Model_Arch_Info};

type Result<T> = candle_core::Result<T>;

// ============================================================
// MoeOrMlp — Dense 层与 MoE 层的统一枚举
// ============================================================

#[derive(Clone)]
pub enum MoeOrMlp {
    Mlp(Mlp_Weights),
    MoE(Arc<FusedMoeGGUF>),
}

impl fmt::Debug for MoeOrMlp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mlp(_) => write!(f, "Mlp"),
            Self::MoE(_) => write!(f, "MoE(..)"),
        }
    }
}

impl MoeOrMlp {
    pub fn Forward(&self, xs: &Tensor, is_prefill: bool) -> Result<Tensor> {
        match self {
            Self::Mlp(m) => m.forward(xs),
            Self::MoE(m) => m.forward(xs, is_prefill),
        }
    }
}

// ============================================================
// Qwen3MoE_Layer — 单个 Transformer 层
// ============================================================

#[derive(Debug, Clone)]
pub struct Qwen3MoE_Layer {
    pub self_attn: Attention_Weights,
    pub ln1: RmsNorm,
    pub mlp: MoeOrMlp,
    pub ln2: RmsNorm,
}

impl Qwen3MoE_Layer {
    pub fn Build_From_Extracted(
        tensors: &mut HashMap<String, QTensor>,
        num_heads: usize,
        num_kv_heads: usize,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        layer_idx: usize,
        is_moe: bool,
        moe_cfg: &MoeCfg,
        model_dtype: DType,
    ) -> Result<Self> {
        let prefix = format!("blk.{layer_idx}");

        // 内联 RmsNorm 辅助函数
        fn Take_Rmsnorm(tensors: &mut HashMap<String, QTensor>, key: &str, eps: f64) -> Result<RmsNorm> {
            let qt = tensors.remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {key}")))?;
            RmsNorm::from_qtensor(qt, eps)
        }

        // Attention
        let self_attn = Attention_Weights::Build_From_Extracted(
            tensors,
            num_heads,
            num_kv_heads,
            head_dim,
            rms_norm_eps,
            rotary,
            layer_idx,
        )?;

        // FFN — MoE 或 Dense
        let mlp = if is_moe {
            let gate_qt = tensors.remove(&format!("{prefix}.ffn_gate_inp.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_gate_inp.weight")))?;
            let gate_ws = gate_qt.dequantize(&Device::Cpu)?.to_dtype(DType::F32)?;
            let gate = candle_nn::Linear::new(gate_ws, None);

            let gate_experts = Arc::new(
                tensors.remove(&format!("{prefix}.ffn_gate_exps.weight"))
                    .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_gate_exps.weight")))?
            );
            let up_experts = Arc::new(
                tensors.remove(&format!("{prefix}.ffn_up_exps.weight"))
                    .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_up_exps.weight")))?
            );
            let down_experts = Arc::new(
                tensors.remove(&format!("{prefix}.ffn_down_exps.weight"))
                    .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_down_exps.weight")))?
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
            let mlp = Mlp_Weights::Build_From_Extracted(tensors, &prefix)?;
            MoeOrMlp::Mlp(mlp)
        };

        let ln1 = Take_Rmsnorm(tensors, &format!("{prefix}.attn_norm.weight"), rms_norm_eps)?;
        let ln2 = Take_Rmsnorm(tensors, &format!("{prefix}.ffn_norm.weight"), rms_norm_eps)?;

        Ok(Self { self_attn, ln1, mlp, ln2 })
    }
}

impl Stage for Qwen3MoE_Layer {
    fn forward(&mut self, x: &Tensor, offset: usize, mask: Option<&Tensor>) -> anyhow::Result<Tensor> {
        let is_prefill = x.dims3().map(|(_, l, _)| l > 1).unwrap_or(false);
        let h = self.ln1.forward(x)?;
        let h = self.self_attn.Forward(&h, mask, offset)?;
        let x = (x + h)?;
        let h2 = self.ln2.forward(&x)?;
        let h2 = self.mlp.Forward(&h2, is_prefill)?;
        (x + h2).map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn kv_cache(&self) -> Option<&candle_nn::kv_cache::ConcatKvCache> {
        Some(&self.self_attn.kv_cache)
    }
    fn kv_cache_mut(&mut self) -> Option<&mut candle_nn::kv_cache::ConcatKvCache> {
        Some(&mut self.self_attn.kv_cache)
    }
}

// ============================================================
// Qwen3MoE_Model — 完整 MoE 模型
// ============================================================

pub struct Qwen3MoE_Model {
    pub stages: Vec<Box<dyn Stage>>,
    pub device: Device,
    pub dtype: DType,
    span: tracing::Span,
    span_output: tracing::Span,
}

impl Qwen3MoE_Model {
    /// 从 GGUF metadata 提取 MoE 配置
    pub fn MoE_Cfg_From_Metadata(
        metadata: &HashMap<String, gguf_file::Value>,
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

    /// 从 GGUF 文件加载所有层并组装为 stages
    pub fn Load_Stages(
        content: &gguf_file::Content,
        file: &mut std::fs::File,
        block_start: usize,
        block_end: usize,
        embed_tokens: Option<candle_nn::Embedding>,
        output_head: Option<(RmsNorm, QMatMul)>,
        arch_info: &Model_Arch_Info,
        rotary: Arc<Rotary_Embedding>,
        moe_cfg: &MoeCfg,
        model_dtype: DType,
        device: &Device,
    ) -> anyhow::Result<Vec<Box<dyn Stage>>> {
        let block_count = if block_start <= block_end { block_end - block_start + 1 } else { 0 };
        let mut layers = Vec::with_capacity(block_count);
        for i in block_start..=block_end {
            let mut lw = GGUF_Load_Layer(content, file, i, device)?;
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
                moe_cfg,
                model_dtype,
            )
            .map_err(|e| anyhow::anyhow!("Layer {} (blk.{}) assembly failed: {}", i, blk_idx, e))?;
            layers.push(layer);
        }

        let mut stages: Vec<Box<dyn Stage>> = Vec::with_capacity(block_count + 2);
        if let Some(e) = embed_tokens { stages.push(Box::new(Qwen3_Embedding_Stage(e))); }
        for layer in layers { stages.push(Box::new(layer)); }
        if let Some((n, h)) = output_head { stages.push(Box::new(Qwen3_Output_Stage { norm: n, lm_head: h })); }
        Ok(stages)
    }

    pub fn Build_Model(stages: Vec<Box<dyn Stage>>, device: Device, dtype: DType) -> Self {
        let span = tracing::span!(tracing::Level::TRACE, "model");
        let span_output = tracing::span!(tracing::Level::TRACE, "output");
        Self { stages, device, dtype, span, span_output }
    }

    fn Causal_Mask(
        &self,
        b: usize,
        tgt: usize,
        offset: usize,
    ) -> Result<Tensor> {
        let minf = f32::NEG_INFINITY;
        let mask: Vec<_> = (0..tgt)
            .flat_map(|i| {
                (0..(tgt + offset)).map(move |j| {
                    if j <= i + offset { 0. } else { minf }
                })
            })
            .collect();
        Tensor::from_slice(&mask, (b, 1, tgt, tgt + offset), &self.device)?
            .to_dtype(self.dtype)
    }
}

impl Model for Qwen3MoE_Model {
    fn Forward(&mut self, input: &Tensor, offset: usize) -> anyhow::Result<Tensor> {
        let _enter = self.span.enter();

        let mut h = self.stages[0].forward(input, offset, None)?;

        let b = h.dim(0)?;
        let l = h.dim(1)?;
        let causal_mask = if l == 1 { None } else { Some(self.Causal_Mask(b, l, offset)?) };

        for stage in &mut self.stages[1..] {
            h = stage.forward(&h, offset, causal_mask.as_ref())?;
        }

        Ok(h)
    }

    fn Clear_Kv_Cache(&mut self) {
        for stage in &mut self.stages {
            stage.clear_kv_cache();
        }
    }

    fn extract_kv_cache(&self) -> std::result::Result<Vec<(Tensor, Tensor)>, String> {
        extract_kv_cache_from_stages(&self.stages)
    }

    fn restore_kv_cache(&mut self, kvs: Vec<(Tensor, Tensor)>) -> std::result::Result<(), String> {
        restore_kv_cache_to_stages(&mut self.stages, kvs)
    }
}
