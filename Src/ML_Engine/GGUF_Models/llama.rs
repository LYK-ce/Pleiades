//Presented by KeJi
//Date : 2026-06-02

//! Llama 3.1 模型实现 (GGUF quantized)
//! 基于 Qwen3 架构适配，主要区别：无 QK Normalization

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use candle_core::quantized::QTensor;
use candle_core::{DType, Device, Tensor};
use candle_nn::kv_cache::ConcatKvCache;
use candle_nn::{Embedding, Module};
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use candle_transformers::utils::repeat_kv;
use std::collections::HashMap;
use std::sync::Arc;

type Result<T> = candle_core::Result<T>;

// Re-use public types from Qwen3
use super::qwen3::{Mlp_Weights, Rotary_Embedding};

// ============================================================
// Llama Attention (no QK normalization)
// ============================================================

#[derive(Debug, Clone)]
pub struct Llama_Attention {
    pub q_proj: QMatMul,
    pub k_proj: QMatMul,
    pub v_proj: QMatMul,
    pub o_proj: QMatMul,
    pub num_heads: usize,
    pub num_kv_heads: usize,
    pub num_kv_groups: usize,
    pub head_dim: usize,
    pub rotary_emb: Arc<Rotary_Embedding>,
    pub kv_cache: ConcatKvCache,
    pub span_attn: tracing::Span,
}

impl Llama_Attention {
    pub fn Forward(
        &mut self,
        x: &Tensor,
        offset: usize,
        mask: Option<&Tensor>,
    ) -> Result<Tensor> {
        let _enter = self.span_attn.enter();
        let (b_sz, seq_len, _) = x.dims3()?;
        let q = self.q_proj.forward(x)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;

        let q = q.reshape((b_sz, seq_len, self.num_heads, self.head_dim))?;
        let k = k.reshape((b_sz, seq_len, self.num_kv_heads, self.head_dim))?;
        let v = v.reshape((b_sz, seq_len, self.num_kv_heads, self.head_dim))?;

        // Transpose to (B, H, L, D)
        let q = q.transpose(1, 2)?;
        let k = k.transpose(1, 2)?;
        let v = v.transpose(1, 2)?;

        // RoPE
        let (q, k) = self.rotary_emb.Apply(&q, &k, offset)?;

        // KV Cache
        let (k, v) = self.kv_cache.append(&k, &v)?;

        // GQA repeat
        let k = repeat_kv(k, self.num_kv_groups)?.contiguous()?;
        let v = repeat_kv(v, self.num_kv_groups)?.contiguous()?;

        // SDPA
        let att = (q.matmul(&k.t()?)? / (self.head_dim as f64).sqrt())?;
        let att = match mask {
            Some(m) => (att + m)?,
            None => att,
        };
        let att = candle_nn::ops::softmax_last_dim(&att)?;
        let y = att.matmul(&v)?;
        let y = y.transpose(1, 2)?.reshape((b_sz, seq_len, ()))?;

        self.o_proj.forward(&y)
    }

    pub fn Clear_Kv_Cache(&mut self) { self.kv_cache.reset(); }
}

// ============================================================
// Llama Layer
// ============================================================

#[derive(Debug, Clone)]
pub struct Llama_Layer {
    pub self_attn: Llama_Attention,
    pub mlp: Mlp_Weights,
    pub ln1: RmsNorm,
    pub ln2: RmsNorm,
}

impl Llama_Layer {
    pub fn From_Extracted(
        tensors: &mut HashMap<String, QTensor>,
        num_attention_heads: usize,
        num_key_value_heads: usize,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        layer_idx: usize,
    ) -> Result<Self> {
        let prefix = format!("blk.{layer_idx}");

        fn take_qmatmul(tensors: &mut HashMap<String, QTensor>, key: &str) -> Result<QMatMul> {
            let qt = tensors.remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {key}")))?;
            QMatMul::from_weights(Arc::new(qt))
        }

        fn take_rmsnorm(tensors: &mut HashMap<String, QTensor>, key: &str, eps: f64) -> Result<RmsNorm> {
            let qt = tensors.remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {key}")))?;
            RmsNorm::from_qtensor(qt, eps)
        }

        let q_proj = take_qmatmul(tensors, &format!("{prefix}.attn_q.weight"))?;
        let k_proj = take_qmatmul(tensors, &format!("{prefix}.attn_k.weight"))?;
        let v_proj = take_qmatmul(tensors, &format!("{prefix}.attn_v.weight"))?;
        let o_proj = take_qmatmul(tensors, &format!("{prefix}.attn_output.weight"))?;

        let num_kv_groups = num_attention_heads / num_key_value_heads;
        let self_attn = Llama_Attention {
            q_proj, k_proj, v_proj, o_proj,
            num_heads: num_attention_heads,
            num_kv_heads: num_key_value_heads,
            num_kv_groups,
            head_dim,
            rotary_emb: rotary,
            kv_cache: ConcatKvCache::new(2),
            span_attn: tracing::span!(tracing::Level::TRACE, "attn"),
        };

        let mlp = super::qwen3::Mlp_Weights::New_Dense(tensors, &prefix)?;

        let ln1 = take_rmsnorm(tensors, &format!("{prefix}.attn_norm.weight"), rms_norm_eps)?;
        let ln2 = take_rmsnorm(tensors, &format!("{prefix}.ffn_norm.weight"), rms_norm_eps)?;

        Ok(Self { self_attn, mlp, ln1, ln2 })
    }

    pub fn Forward(&mut self, x: &Tensor, offset: usize, mask: Option<&Tensor>) -> Result<Tensor> {
        let residual = x;
        let x = self.ln1.forward(x)?;
        let x = self.self_attn.Forward(&x, offset, mask)?;
        let x = (x + residual)?;
        let residual = &x;
        let x = self.ln2.forward(&x)?;
        let x = self.mlp.forward(&x)?;
        x + residual
    }

    pub fn Clear_Kv_Cache(&mut self) { self.self_attn.Clear_Kv_Cache(); }
}

// ============================================================
// Llama Model
// ============================================================

#[derive(Debug, Clone)]
pub struct Llama_Model {
    pub embed_tokens: Option<Embedding>,
    pub layers: Vec<Llama_Layer>,
    pub norm: Option<RmsNorm>,
    pub lm_head: Option<QMatMul>,
    pub device: Device,
    pub dtype: DType,
}

impl Llama_Model {
    pub fn From_Dynamic(
        embed_tokens: Option<Embedding>,
        layers: Vec<Llama_Layer>,
        norm: Option<RmsNorm>,
        lm_head: Option<QMatMul>,
        device: Device,
        dtype: DType,
    ) -> Self {
        Self { embed_tokens, layers, norm, lm_head, device, dtype }
    }

    fn causal_mask(seq_len: usize, offset: usize, device: &Device) -> Result<Tensor> {
        let mask: Vec<f32> = (0..seq_len)
            .flat_map(|i| {
                (0..offset + seq_len).map(move |j| {
                    if j > offset + i { f32::NEG_INFINITY } else { 0f32 }
                })
            })
            .collect();
        Tensor::from_vec(mask, (1, 1, seq_len, offset + seq_len), device)
    }

    pub fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor> {
        let (_b_sz, seq_len) = input.dims2()?;
        let mask = if seq_len > 1 {
            Some(Self::causal_mask(seq_len, offset, &self.device)?)
        } else {
            None
        };

        let mut x = match &self.embed_tokens {
            Some(emb) => emb.forward(input)?,
            None => input.clone(),
        };

        for layer in &mut self.layers {
            x = layer.Forward(&x, offset, mask.as_ref())?;
        }

        if let Some(ref norm) = self.norm {
            x = norm.forward(&x)?;
        }

        if let Some(ref lm_head) = self.lm_head {
            x = lm_head.forward(&x)?;
        }

        Ok(x)
    }

    pub fn Clear_Kv_Cache(&mut self) {
        for layer in &mut self.layers {
            layer.Clear_Kv_Cache();
        }
    }
}
