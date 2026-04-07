//Presented by KeJi
//Date : 2026-03-30

//! Local copy of quantized_qwen3 with public types for single-layer instantiation.
//! Based on candle-transformers 0.9.2 quantized_qwen3.rs

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use candle_core::quantized::{gguf_file, QTensor};
use candle_core::{DType, Device, Tensor};
use candle_nn::{kv_cache::ConcatKvCache, Activation, Embedding, Module};
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use candle_transformers::utils::repeat_kv;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::sync::Arc;

type Result<T> = candle_core::Result<T>;

// ============================================================
// Gguf helper (public, copied from original)
// ============================================================

pub struct Gguf<R: Read + Seek> {
    pub ct: gguf_file::Content,
    pub reader: R,
    pub device: Device,
}

impl<R: Read + Seek> Gguf<R> {
    pub fn New(ct: gguf_file::Content, reader: R, device: Device) -> Self {
        Self { ct, reader, device }
    }

    pub fn Qmatmul(&mut self, name: &str) -> Result<QMatMul> {
        let ws = self.ct.tensor(&mut self.reader, name, &self.device)?;
        QMatMul::from_weights(ws.into())
    }

    pub fn Rms_Norm(&mut self, name: &str, eps: f64) -> Result<RmsNorm> {
        let ws = self.ct.tensor(&mut self.reader, name, &self.device)?;
        RmsNorm::from_qtensor(ws, eps)
    }

    pub fn Metadata(&self) -> &std::collections::HashMap<String, gguf_file::Value> {
        &self.ct.metadata
    }

    pub fn Tensor(&mut self, name: &str) -> Result<QTensor> {
        self.ct.tensor(&mut self.reader, name, &self.device)
    }
}

// ============================================================
// RotaryEmbedding (public, copied from original)
// ============================================================

#[derive(Debug, Clone)]
pub struct Rotary_Embedding {
    sin: Tensor,
    cos: Tensor,
}

impl Rotary_Embedding {
    pub fn New(
        dtype: DType,
        head_dim: usize,
        max_position_embeddings: usize,
        rope_theta: f64,
        dev: &Device,
    ) -> Result<Self> {
        let dim = head_dim;
        let max_seq_len = max_position_embeddings;
        let inv_freq: Vec<_> = (0..dim)
            .step_by(2)
            .map(|i| 1f32 / rope_theta.powf(i as f64 / dim as f64) as f32)
            .collect();
        let inv_freq_len = inv_freq.len();
        let inv_freq =
            Tensor::from_vec(inv_freq, (1, inv_freq_len), dev)?.to_dtype(dtype)?;
        let t = Tensor::arange(0u32, max_seq_len as u32, dev)?
            .to_dtype(dtype)?
            .reshape((max_seq_len, 1))?;
        let freqs = t.matmul(&inv_freq)?;
        Ok(Self {
            sin: freqs.sin()?,
            cos: freqs.cos()?,
        })
    }

    /// Apply RoPE (q, k shape: B x H x L x D)
    pub fn Apply(&self, q: &Tensor, k: &Tensor, offset: usize) -> Result<(Tensor, Tensor)> {
        let (_, _, seq_len, _) = q.dims4()?;
        let cos = self.cos.narrow(0, offset, seq_len)?.to_dtype(q.dtype())?;
        let sin = self.sin.narrow(0, offset, seq_len)?.to_dtype(q.dtype())?;
        let q_embed = candle_nn::rotary_emb::rope(&q.contiguous()?, &cos, &sin)?;
        let k_embed = candle_nn::rotary_emb::rope(&k.contiguous()?, &cos, &sin)?;
        Ok((q_embed, k_embed))
    }
}

// ============================================================
// MlpWeights (now public)
// ============================================================

#[derive(Debug, Clone)]
pub struct Mlp_Weights {
    pub gate_proj: QMatMul,
    pub up_proj: QMatMul,
    pub down_proj: QMatMul,
    pub act_fn: Activation,
    span: tracing::Span,
}

impl Mlp_Weights {
    pub fn New<R: Read + Seek>(gg: &mut Gguf<R>, prefix: &str) -> Result<Self> {
        let gate_proj = gg.Qmatmul(&format!("{prefix}.ffn_gate.weight"))?;
        let up_proj = gg.Qmatmul(&format!("{prefix}.ffn_up.weight"))?;
        let down_proj = gg.Qmatmul(&format!("{prefix}.ffn_down.weight"))?;
        let act_fn = Activation::Silu;
        let span = tracing::span!(tracing::Level::TRACE, "mlp");
        Ok(Self {
            gate_proj,
            up_proj,
            down_proj,
            act_fn,
            span,
        })
    }

    /// Build from extracted QTensors (HashMap key = full tensor name)
    pub fn From_Extracted(tensors: &HashMap<String, QTensor>, prefix: &str) -> Result<Self> {
        let gate_key = format!("{prefix}.ffn_gate.weight");
        let up_key = format!("{prefix}.ffn_up.weight");
        let down_key = format!("{prefix}.ffn_down.weight");

        let gate_proj = Self::Get_Qmatmul(tensors, &gate_key)?;
        let up_proj = Self::Get_Qmatmul(tensors, &up_key)?;
        let down_proj = Self::Get_Qmatmul(tensors, &down_key)?;

        let act_fn = Activation::Silu;
        let span = tracing::span!(tracing::Level::TRACE, "mlp");
        Ok(Self {
            gate_proj,
            up_proj,
            down_proj,
            act_fn,
            span,
        })
    }

    fn Get_Qmatmul(tensors: &HashMap<String, QTensor>, key: &str) -> Result<QMatMul> {
        let _qt = tensors
            .get(key)
            .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {}", key)))?;
        // Clone the QTensor data by re-wrapping; QTensor -> Arc<QTensor> -> QMatMul
        // Since we can't clone QTensor directly, we need to use a workaround
        // Actually QMatMul::from_weights takes Arc<QTensor>
        // We need to get an owned QTensor. Since the HashMap owns it, we'll need &QTensor.
        // Unfortunately QMatMul::from_weights requires Arc<QTensor>.
        // We'll work around this by reading from file in the extract step.
        // For now, this won't work with borrowed QTensors.
        // Let's use a different approach - we'll take ownership from the HashMap.
        candle_core::bail!("use From_Extracted_Owned instead")
    }
}

impl Module for Mlp_Weights {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let gate = self.gate_proj.forward(x)?.apply(&self.act_fn)?;
        let up = self.up_proj.forward(x)?;
        let gated = (gate * up)?;
        self.down_proj.forward(&gated)
    }
}

// ============================================================
// AttentionWeights (now public)
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
    kv_cache: ConcatKvCache,
    span_attn: tracing::Span,
}

impl Attention_Weights {
    pub fn New<R: Read + Seek>(
        gg: &mut Gguf<R>,
        num_heads: usize,
        num_kv_heads: usize,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary_emb: Arc<Rotary_Embedding>,
        prefix: &str,
    ) -> Result<Self> {
        let num_kv_groups = num_heads / num_kv_heads;

        let q_proj = gg.Qmatmul(&format!("{prefix}.attn_q.weight"))?;
        let k_proj = gg.Qmatmul(&format!("{prefix}.attn_k.weight"))?;
        let v_proj = gg.Qmatmul(&format!("{prefix}.attn_v.weight"))?;
        let o_proj = gg.Qmatmul(&format!("{prefix}.attn_output.weight"))?;

        let q_norm = gg.Rms_Norm(&format!("{prefix}.attn_q_norm.weight"), rms_norm_eps)?;
        let k_norm = gg.Rms_Norm(&format!("{prefix}.attn_k_norm.weight"), rms_norm_eps)?;

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
            rotary_emb,
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
    /// Load a single layer from GGUF using the Gguf reader
    pub fn New<R: Read + Seek>(
        gg: &mut Gguf<R>,
        num_attention_heads: usize,
        num_key_value_heads: usize,
        head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        layer_idx: usize,
    ) -> Result<Self> {
        let prefix = format!("blk.{layer_idx}");

        let ln1 = gg.Rms_Norm(&format!("{prefix}.attn_norm.weight"), rms_norm_eps)?;
        let ln2 = gg.Rms_Norm(&format!("{prefix}.ffn_norm.weight"), rms_norm_eps)?;
        let self_attn = Attention_Weights::New(
            gg,
            num_attention_heads,
            num_key_value_heads,
            head_dim,
            rms_norm_eps,
            rotary,
            &prefix,
        )?;
        let mlp = Mlp_Weights::New(gg, &prefix)?;
        Ok(Self {
            self_attn,
            mlp,
            ln1,
            ln2,
        })
    }

    /// Build a single layer from extracted QTensors (from GGUF_Extract)
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

        // Inline helpers to avoid nested closure borrow issues
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

        // Attention
        let q_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_q.weight"))?;
        let k_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_k.weight"))?;
        let v_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_v.weight"))?;
        let o_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_output.weight"))?;
        let q_norm = Take_Rmsnorm(tensors, &format!("{prefix}.attn_q_norm.weight"), rms_norm_eps)?;
        let k_norm = Take_Rmsnorm(tensors, &format!("{prefix}.attn_k_norm.weight"), rms_norm_eps)?;

        let num_kv_groups = num_attention_heads / num_key_value_heads;
        let kv_cache = ConcatKvCache::new(2);
        let span_attn = tracing::span!(tracing::Level::TRACE, "attn");

        let self_attn = Attention_Weights {
            q_proj,
            k_proj,
            v_proj,
            o_proj,
            q_norm,
            k_norm,
            num_heads: num_attention_heads,
            num_kv_heads: num_key_value_heads,
            num_kv_groups,
            head_dim,
            rotary_emb: rotary,
            kv_cache,
            span_attn,
        };

        // MLP
        let gate_proj = Take_Qmatmul(tensors, &format!("{prefix}.ffn_gate.weight"))?;
        let up_proj = Take_Qmatmul(tensors, &format!("{prefix}.ffn_up.weight"))?;
        let down_proj = Take_Qmatmul(tensors, &format!("{prefix}.ffn_down.weight"))?;
        let span_mlp = tracing::span!(tracing::Level::TRACE, "mlp");

        let mlp = Mlp_Weights {
            gate_proj,
            up_proj,
            down_proj,
            act_fn: Activation::Silu,
            span: span_mlp,
        };

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
    pub fn From_Gguf<R: Read + Seek>(
        ct: gguf_file::Content,
        reader: &mut R,
        device: &Device,
    ) -> Result<Self> {
        let mut gg = Gguf::New(ct, reader, device.clone());
        let md_get = |s: &str| match gg.Metadata().get(s) {
            None => candle_core::bail!("cannot find {s} in metadata"),
            Some(v) => Ok(v),
        };

        let num_attention_heads = md_get("qwen3.attention.head_count")?.to_u32()? as usize;
        let num_kv_heads = md_get("qwen3.attention.head_count_kv")?.to_u32()? as usize;
        let head_dim = md_get("qwen3.attention.key_length")?.to_u32()? as usize;
        let num_layers = md_get("qwen3.block_count")?.to_u32()? as usize;
        let hidden_size = md_get("qwen3.embedding_length")?.to_u32()? as usize;
        let max_position_embeddings = md_get("qwen3.context_length")?.to_u32()? as usize;
        let rms_norm_eps =
            md_get("qwen3.attention.layer_norm_rms_epsilon")?.to_f32()? as f64;
        let rope_freq_base = md_get("qwen3.rope.freq_base")?.to_f32()? as f64;

        let dtype = match gg.Metadata().get("general.dtype") {
            Some(v) => match v.to_u32() {
                Ok(0) => DType::F32,
                Ok(1) => DType::F16,
                _ => DType::F16,
            },
            None => DType::F16,
        };

        let embed_tensor = gg.Tensor("token_embd.weight")?;
        let embed_tokens = Embedding::new(embed_tensor.dequantize(device)?, hidden_size);

        let rotary = Arc::new(Rotary_Embedding::New(
            dtype,
            head_dim,
            max_position_embeddings,
            rope_freq_base,
            device,
        )?);

        let mut layers = Vec::with_capacity(num_layers);
        for i in 0..num_layers {
            layers.push(Layer_Weights::New(
                &mut gg,
                num_attention_heads,
                num_kv_heads,
                head_dim,
                rms_norm_eps,
                rotary.clone(),
                i,
            )?);
        }

        let norm = gg.Rms_Norm("output_norm.weight", rms_norm_eps)?;
        let lm_head_tensor = match gg.Tensor("output.weight") {
            Ok(tensor) => tensor,
            Err(_) => gg.Tensor("token_embd.weight")?,
        };
        let lm_head = QMatMul::from_weights(lm_head_tensor.into())?;
        let span = tracing::span!(tracing::Level::TRACE, "model");
        let span_output = tracing::span!(tracing::Level::TRACE, "output");
        Ok(Self {
            embed_tokens: Some(embed_tokens),
            layers,
            norm: Some(norm),
            lm_head: Some(lm_head),
            device: device.clone(),
            dtype,
            span,
            span_output,
        })
    }

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
    pub fn From_Dynamic(
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

// ============================================================
// 分割模型: 前半部分 (embedding + layers 0..split_point)
// ============================================================

#[derive(Debug, Clone)]
pub struct Model_First_Half {
    pub embed_tokens: Embedding,
    pub layers: Vec<Layer_Weights>,
    pub device: Device,
    pub dtype: DType,
}

impl Model_First_Half {
    /// 构建前半模型: embedding + 指定范围的层
    pub fn New(
        embed_tokens: Embedding,
        layers: Vec<Layer_Weights>,
        device: Device,
        dtype: DType,
    ) -> Self {
        Self {
            embed_tokens,
            layers,
            device,
            dtype,
        }
    }

    /// 前向推理: input_ids -> hidden_states
    pub fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor> {
        let (b, l) = input.dims2()?;
        let mut h = self.embed_tokens.forward(input)?;
        let causal_mask = if l == 1 {
            None
        } else {
            Some(Build_Causal_Mask(b, l, offset, &self.device, self.dtype)?)
        };
        for layer in &mut self.layers {
            h = layer.Forward(&h, causal_mask.as_ref(), offset)?;
        }
        Ok(h)
    }

    pub fn Clear_Kv_Cache(&mut self) {
        for layer in &mut self.layers {
            layer.Clear_Kv_Cache();
        }
    }
}

// ============================================================
// 分割模型: 后半部分 (layers split_point..end + norm + lm_head)
// ============================================================

#[derive(Debug, Clone)]
pub struct Model_Second_Half {
    pub layers: Vec<Layer_Weights>,
    pub norm: RmsNorm,
    pub lm_head: QMatMul,
    pub device: Device,
    pub dtype: DType,
}

impl Model_Second_Half {
    /// 构建后半模型: 指定范围的层 + output norm + lm_head
    pub fn New(
        layers: Vec<Layer_Weights>,
        norm: RmsNorm,
        lm_head: QMatMul,
        device: Device,
        dtype: DType,
    ) -> Self {
        Self {
            layers,
            norm,
            lm_head,
            device,
            dtype,
        }
    }

    /// 前向推理: hidden_states -> logits
    pub fn Forward(&mut self, hidden: &Tensor, offset: usize) -> Result<Tensor> {
        let (b, l, _) = hidden.dims3()?;
        let causal_mask = if l == 1 {
            None
        } else {
            Some(Build_Causal_Mask(b, l, offset, &self.device, self.dtype)?)
        };
        let mut h = hidden.clone();
        for layer in &mut self.layers {
            h = layer.Forward(&h, causal_mask.as_ref(), offset)?;
        }
        let h = self.norm.forward(&h)?;
        let last_hidden = h.narrow(1, l - 1, 1)?;
        self.lm_head.forward(&last_hidden)?.squeeze(1)
    }

    pub fn Clear_Kv_Cache(&mut self) {
        for layer in &mut self.layers {
            layer.Clear_Kv_Cache();
        }
    }
}

// ============================================================
// 辅助函数: 构建 causal mask
// ============================================================

pub fn Build_Causal_Mask(
    b: usize,
    tgt: usize,
    offset: usize,
    device: &Device,
    dtype: DType,
) -> Result<Tensor> {
    let minf = f32::NEG_INFINITY;
    let mask: Vec<_> = (0..tgt)
        .flat_map(|i| {
            (0..(tgt + offset)).map(move |j| {
                let past_ok = j <= i + offset;
                if past_ok { 0. } else { minf }
            })
        })
        .collect();
    Tensor::from_slice(&mask, (b, 1, tgt, tgt + offset), device)?.to_dtype(dtype)
}
