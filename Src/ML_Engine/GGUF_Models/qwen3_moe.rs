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

use candle_core::quantized::QTensor;
use candle_core::{DType, Device, Tensor};
use candle_nn::{Embedding, Module};
use candle_transformers::fused_moe::{FusedMoeGGUF, MoeCfg};
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use super::common::{Mlp_Weights, Rotary_Embedding};
use super::qwen3::{
    Attention_Weights,
};

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

    pub fn Forward(
        &mut self,
        x: &Tensor,
        mask: Option<&Tensor>,
        offset: usize,
        is_prefill: bool,
    ) -> Result<Tensor> {
        let h = self.ln1.forward(x)?;
        let h = self.self_attn.Forward(&h, mask, offset)?;
        let x = (x + h)?;
        let h2 = self.ln2.forward(&x)?;
        let h2 = self.mlp.Forward(&h2, is_prefill)?;
        x + h2
    }

    pub fn Clear_Kv_Cache(&mut self) {
        self.self_attn.Clear_Kv_Cache();
    }
}

// ============================================================
// Qwen3MoE_Model — 完整 MoE 模型
// ============================================================

#[derive(Debug, Clone)]
pub struct Qwen3MoE_Model {
    pub embed_tokens: Option<Embedding>,
    pub layers: Vec<Qwen3MoE_Layer>,
    pub norm: Option<RmsNorm>,
    pub lm_head: Option<QMatMul>,
    pub device: Device,
    pub dtype: DType,
    span: tracing::Span,
    span_output: tracing::Span,
}

impl Qwen3MoE_Model {
    pub fn Build_Model(
        embed_tokens: Option<Embedding>,
        layers: Vec<Qwen3MoE_Layer>,
        norm: Option<RmsNorm>,
        lm_head: Option<QMatMul>,
        device: Device,
        dtype: DType,
    ) -> Self {
        let span = tracing::span!(tracing::Level::TRACE, "model");
        let span_output = tracing::span!(tracing::Level::TRACE, "output");
        Self {
            embed_tokens, layers, norm, lm_head,
            device, dtype, span, span_output,
        }
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

    pub fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor> {
        let _enter = self.span.enter();

        let mut h = if let Some(ref embed) = self.embed_tokens {
            embed.forward(input)?
        } else {
            input.clone()
        };

        let b = h.dim(0)?;
        let l = h.dim(1)?;
        let is_prefill = l > 1;
        let causal_mask = if l == 1 {
            None
        } else {
            Some(self.Causal_Mask(b, l, offset)?)
        };

        for layer in &mut self.layers {
            h = layer.Forward(&h, causal_mask.as_ref(), offset, is_prefill)?;
        }

        if let (Some(ref norm), Some(ref lm_head)) = (&self.norm, &self.lm_head) {
            let h = norm.forward(&h)?;
            let _enter = self.span_output.enter();
            let last_hidden = h.narrow(1, l - 1, 1)?;
            lm_head.forward(&last_hidden)?.squeeze(1)
        } else {
            Ok(h)
        }
    }

    pub fn Clear_Kv_Cache(&mut self) {
        for layer in &mut self.layers {
            layer.Clear_Kv_Cache();
        }
    }
}
