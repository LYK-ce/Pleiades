//Presented by KeJi
//Created Date ： 2026-03-30
//Modified Date ： 2026-06-17

//! Qwen3 模型权重定义 (Attention + Layer + Model + Config)

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use candle_core::quantized::{gguf_file, QTensor};
use candle_core::{DType, Device, Tensor};
use candle_nn::{kv_cache::ConcatKvCache, Embedding, Module};
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use candle_transformers::utils::repeat_kv;
use std::collections::HashMap;
use std::sync::Arc;

use super::common::{Rotary_Embedding, Mlp_Weights};

type Result<T> = candle_core::Result<T>;

// ============================================================

#[derive(Debug, Clone)]
pub struct Attention_Weights {
    pub q_proj: QMatMul,
    pub k_proj: QMatMul,
    pub v_proj: QMatMul,
    pub o_proj: QMatMul,
    pub q_norm: RmsNorm,
    pub k_norm: RmsNorm,
    pub num_heads: usize,
    pub num_kv_heads: usize,
    pub num_kv_groups: usize,
    pub head_dim: usize,
    pub rotary_emb: Arc<Rotary_Embedding>,
    pub(crate) kv_cache: ConcatKvCache,
    pub(crate) span_attn: tracing::Span,
}

impl Attention_Weights {

    /// Build attention weights from extracted QTensors
    pub fn Build_From_Extracted(
        tensors: &mut HashMap<String, QTensor>,
        num_heads: usize,
        num_kv_heads: usize,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        layer_idx: usize,
    ) -> Result<Self> {
        let prefix = format!("blk.{layer_idx}");

        fn Take_Qmatmul(
            tensors: &mut HashMap<String, QTensor>,
            key: &str,
        ) -> Result<QMatMul> {
            let qt = tensors
                .remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {}", key)))?;
            QMatMul::from_weights(Arc::new(qt))
        }

        fn Take_Rmsnorm(
            tensors: &mut HashMap<String, QTensor>,
            key: &str,
            eps: f64,
        ) -> Result<RmsNorm> {
            let qt = tensors
                .remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {}", key)))?;
            RmsNorm::from_qtensor(qt, eps)
        }

        let q_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_q.weight"))?;
        let k_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_k.weight"))?;
        let v_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_v.weight"))?;
        let o_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_output.weight"))?;
        let q_norm = Take_Rmsnorm(tensors, &format!("{prefix}.attn_q_norm.weight"), rms_norm_eps)?;
        let k_norm = Take_Rmsnorm(tensors, &format!("{prefix}.attn_k_norm.weight"), rms_norm_eps)?;

        let num_kv_groups = num_heads / num_kv_heads;
        let kv_cache = ConcatKvCache::new(2);
        let span_attn = tracing::span!(tracing::Level::TRACE, "attn");

        Ok(Self {
            q_proj,
            k_proj,
            v_proj,
            o_proj,
            q_norm,
            k_norm,
            num_heads,
            num_kv_heads,
            num_kv_groups,
            head_dim,
            rotary_emb: rotary,
            kv_cache,
            span_attn,
        })
    }

    pub fn Forward(
        &mut self,
        x: &Tensor,
        attn_mask: Option<&Tensor>,
        offset: usize,
    ) -> Result<Tensor> {
        let _enter = self.span_attn.enter();
        let (b, l, _) = x.dims3()?;

        let q = self.q_proj.forward(x)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;

        let q = q
            .reshape((b, l, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;
        let k = k
            .reshape((b, l, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;
        let v = v
            .reshape((b, l, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;

        let q_flat = q.flatten(0, 2)?;
        let k_flat = k.flatten(0, 2)?;

        let q_flat = self.q_norm.forward(&q_flat)?;
        let k_flat = self.k_norm.forward(&k_flat)?;
        let q = q_flat.reshape((b, self.num_heads, l, self.head_dim))?;
        let k = k_flat.reshape((b, self.num_kv_heads, l, self.head_dim))?;

        let (q, k) = self.rotary_emb.Apply(&q, &k, offset)?;

        let (k, v) = self.kv_cache.append(&k, &v)?;

        let k = repeat_kv(k, self.num_kv_groups)?.contiguous()?;
        let v = repeat_kv(v, self.num_kv_groups)?.contiguous()?;

        let scale = 1.0 / (self.head_dim as f64).sqrt();
        let mut scores = (q.matmul(&k.transpose(2, 3)?)? * scale)?;
        if let Some(m) = attn_mask {
            let m_dtype = m.dtype();
            let scores_dtype = scores.dtype();
            let mask = if m_dtype != scores_dtype {
                m.to_dtype(scores_dtype)?
            } else {
                m.clone()
            };
            scores = scores.broadcast_add(&mask)?;
        }
        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let ctx = probs.matmul(&v)?; // (B, H, L, D)
        let reshaped_ctx = ctx
            .transpose(1, 2)?
            .reshape((b, l, self.num_heads * self.head_dim))?;
        self.o_proj.forward(&reshaped_ctx)
    }

    pub fn Clear_Kv_Cache(&mut self) {
        self.kv_cache.reset();
    }
}

// ============================================================
// LayerWeights (now public)
// ============================================================

#[derive(Debug, Clone)]
pub struct Layer_Weights {
    pub self_attn: Attention_Weights,
    pub mlp: Mlp_Weights,
    pub ln1: RmsNorm,
    pub ln2: RmsNorm,
}

impl Layer_Weights {

    /// Build a single layer from extracted QTensors
    pub fn Build_From_Extracted(
        tensors: &mut HashMap<String, QTensor>,
        num_attention_heads: usize,
        num_key_value_heads: usize,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        layer_idx: usize,
    ) -> Result<Self> {
        let prefix = format!("blk.{layer_idx}");

        // Inline RmsNorm helper
        fn Take_Rmsnorm(
            tensors: &mut HashMap<String, QTensor>,
            key: &str,
            eps: f64,
        ) -> Result<RmsNorm> {
            let qt = tensors
                .remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {}", key)))?;
            RmsNorm::from_qtensor(qt, eps)
        }

        // Attention
        let self_attn = Attention_Weights::Build_From_Extracted(
            tensors,
            num_attention_heads,
            num_key_value_heads,
            head_dim,
            rms_norm_eps,
            rotary,
            layer_idx,
        )?;

        // MLP
        let mlp = Mlp_Weights::Build_From_Extracted(tensors, &prefix)?;

        // Norms
        let ln1 = Take_Rmsnorm(tensors, &format!("{prefix}.attn_norm.weight"), rms_norm_eps)?;
        let ln2 = Take_Rmsnorm(tensors, &format!("{prefix}.ffn_norm.weight"), rms_norm_eps)?;

        Ok(Self {
            self_attn,
            mlp,
            ln1,
            ln2,
        })
    }

    pub fn Forward(
        &mut self,
        x: &Tensor,
        mask: Option<&Tensor>,
        offset: usize,
    ) -> Result<Tensor> {
        let h = self.ln1.forward(x)?;
        let h = self.self_attn.Forward(&h, mask, offset)?;
        let x = (x + h)?;
        let h2 = self.ln2.forward(&x)?;
        let h2 = h2.apply(&self.mlp)?;
        x + h2
    }

    pub fn Clear_Kv_Cache(&mut self) {
        self.self_attn.Clear_Kv_Cache();
    }
}

// ============================================================
// ModelWeights (public, full model - kept for reference)
// ============================================================

#[derive(Debug, Clone)]
pub struct Model_Weights {
    /// 输入 embedding（如果是部分模型不含输入头，则为 None）
    pub embed_tokens: Option<Embedding>,
    pub layers: Vec<Layer_Weights>,
    /// Output RmsNorm（如果是部分模型不含输出头，则为 None）
    pub norm: Option<RmsNorm>,
    /// LM Head（如果是部分模型不含输出头，则为 None）
    pub lm_head: Option<QMatMul>,
    pub device: Device,
    pub dtype: DType,
    span: tracing::Span,
    span_output: tracing::Span,
}

impl Model_Weights {

    fn Causal_Mask(
        &self,
        b: usize,
        tgt: usize,
        offset: usize,
        sw: Option<usize>,
    ) -> Result<Tensor> {
        let minf = f32::NEG_INFINITY;
        let mask: Vec<_> = (0..tgt)
            .flat_map(|i| {
                (0..(tgt + offset)).map(move |j| {
                    let past_ok = j <= i + offset;
                    let sw_ok = match sw {
                        Some(w) => (i + offset) as i64 - j as i64 <= w as i64,
                        None => true,
                    };
                    if past_ok && sw_ok {
                        0.
                    } else {
                        minf
                    }
                })
            })
            .collect();
        Tensor::from_slice(&mask, (b, 1, tgt, tgt + offset), &self.device)?
            .to_dtype(self.dtype)
    }

    /// Forward pass，支持完整模型和部分模型：
    /// - 如果包含 embed_tokens：input 为 token IDs [batch, seq_len]，自动执行 embedding
    /// - 如果不含 embed_tokens：input 为已嵌入的 hidden state [batch, seq_len, hidden_dim]
    /// - 如果包含 norm + lm_head：返回 logits [batch, vocab_size]
    /// - 如果不含 norm + lm_head：返回最后的 hidden state [batch, seq_len, hidden_dim]
    pub fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor> {
        let _enter = self.span.enter();

        // 如果有 embedding，将 token IDs 转为 hidden state；否则直接使用输入的 hidden state
        let mut h = if let Some(ref embed) = self.embed_tokens {
            embed.forward(input)?
        } else {
            input.clone()
        };

        // 获取 batch size 和 sequence length（从 hidden state 的前两维）
        let b = h.dim(0)?;
        let l = h.dim(1)?;

        let causal_mask = if l == 1 {
            None
        } else {
            Some(self.Causal_Mask(b, l, offset, None)?)
        };
        for layer in &mut self.layers {
            h = layer.Forward(&h, causal_mask.as_ref(), offset)?;
        }

        // 如果有输出头，执行 norm → lm_head 返回 logits；否则返回 hidden state
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

    /// 动态组装: 从预构建的组件创建模型（支持完整模型和部分模型）
    ///
    /// - embed_tokens: 输入 embedding（部分模型可为 None）
    /// - norm / lm_head: 输出头（部分模型可为 None）
    pub fn Build_Model(
        embed_tokens: Option<Embedding>,
        layers: Vec<Layer_Weights>,
        norm: Option<RmsNorm>,
        lm_head: Option<QMatMul>,
        device: Device,
        dtype: DType,
    ) -> Self {
        let span = tracing::span!(tracing::Level::TRACE, "model");
        let span_output = tracing::span!(tracing::Level::TRACE, "output");
        Self {
            embed_tokens,
            layers,
            norm,
            lm_head,
            device,
            dtype,
            span,
            span_output,
        }
    }
}

// ============================================================
// Helper: extract Qwen3 model config from GGUF metadata
// ============================================================

pub struct Qwen3_Config {
    pub num_attention_heads: usize,
    pub num_kv_heads: usize,
    pub head_dim: usize,
    pub num_layers: usize,
    pub hidden_size: usize,
    pub max_position_embeddings: usize,
    pub rms_norm_eps: f64,
    pub rope_freq_base: f64,
}

impl Qwen3_Config {
    /// Extract config from GGUF metadata
    pub fn From_Metadata(
        metadata: &std::collections::HashMap<String, gguf_file::Value>,
    ) -> Result<Self> {
        let md_get = |s: &str| match metadata.get(s) {
            None => candle_core::bail!("cannot find {s} in metadata"),
            Some(v) => Ok(v),
        };

        Ok(Self {
            num_attention_heads: md_get("qwen3.attention.head_count")?.to_u32()? as usize,
            num_kv_heads: md_get("qwen3.attention.head_count_kv")?.to_u32()? as usize,
            head_dim: md_get("qwen3.attention.key_length")?.to_u32()? as usize,
            num_layers: md_get("qwen3.block_count")?.to_u32()? as usize,
            hidden_size: md_get("qwen3.embedding_length")?.to_u32()? as usize,
            max_position_embeddings: md_get("qwen3.context_length")?.to_u32()? as usize,
            rms_norm_eps: md_get("qwen3.attention.layer_norm_rms_epsilon")?.to_f32()? as f64,
            rope_freq_base: md_get("qwen3.rope.freq_base")?.to_f32()? as f64,
        })
    }
}
