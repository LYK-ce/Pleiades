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

use super::super::common::{Rotary_Embedding, Mlp_Weights, Stage, Model, extract_kv_cache_from_stages, restore_kv_cache_to_stages};
use crate::ml_engine::gguf_model_manager::{GGUF_Load_Layer, Model_Arch_Info};

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
}


/// Qwen3 Embedding 层
pub struct Qwen3_Embedding_Stage(pub(crate) candle_nn::Embedding);
impl Stage for Qwen3_Embedding_Stage {
    fn forward(&mut self, x: &Tensor, _offset: usize, _mask: Option<&Tensor>) -> anyhow::Result<Tensor> {
        self.0.forward(x).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// Qwen3 输出头（norm + lm_head）
pub struct Qwen3_Output_Stage {
    pub(crate) norm: RmsNorm,
    pub(crate) lm_head: QMatMul,
}
impl Stage for Qwen3_Output_Stage {
    fn forward(&mut self, x: &Tensor, _offset: usize, _mask: Option<&Tensor>) -> anyhow::Result<Tensor> {
        let h = self.norm.forward(x).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.lm_head.forward(&h).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

impl Stage for Layer_Weights {
    fn forward(&mut self, x: &Tensor, offset: usize, mask: Option<&Tensor>) -> anyhow::Result<Tensor> {
        let h = self.ln1.forward(x)?;
        let h = self.self_attn.Forward(&h, mask, offset)?;
        let x = (x + h)?;
        let h2 = self.ln2.forward(&x)?;
        let h2 = h2.apply(&self.mlp)?;
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
// ModelWeights (public, full model - kept for reference)
// ============================================================

pub struct Model_Weights {
    pub stages: Vec<Box<dyn Stage>>,
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
}

impl Model for Model_Weights {
    fn Forward(&mut self, input: &Tensor, offset: usize) -> anyhow::Result<Tensor> {
        let _enter = self.span.enter();

        let mut h = self.stages[0].forward(input, offset, None)?;

        let b = h.dim(0)?;
        let l = h.dim(1)?;

        let causal_mask = if l == 1 {
            None
        } else {
            Some(self.Causal_Mask(b, l, offset, None)?)
        };

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

impl Model_Weights {
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
        device: &Device,
    ) -> anyhow::Result<Vec<Box<dyn Stage>>> {
        let block_count = if block_start <= block_end { block_end - block_start + 1 } else { 0 };
        let mut layers = Vec::with_capacity(block_count);
        for i in block_start..=block_end {
            let mut lw = GGUF_Load_Layer(content, file, i, device)?;
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

        let mut stages: Vec<Box<dyn Stage>> = Vec::with_capacity(block_count + 2);
        if let Some(e) = embed_tokens { stages.push(Box::new(Qwen3_Embedding_Stage(e))); }
        for layer in layers { stages.push(Box::new(layer)); }
        if let Some((n, h)) = output_head { stages.push(Box::new(Qwen3_Output_Stage { norm: n, lm_head: h })); }
        Ok(stages)
    }

    /// 从预构建的 stages 创建模型
    pub fn Build_Model(stages: Vec<Box<dyn Stage>>, device: Device, dtype: DType) -> Self {
        let span = tracing::span!(tracing::Level::TRACE, "model");
        let span_output = tracing::span!(tracing::Level::TRACE, "output");
        Self { stages, device, dtype, span, span_output }
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
