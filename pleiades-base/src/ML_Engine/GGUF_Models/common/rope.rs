//Presented by KeJi
//Created Date ： 2026-06-17
//Modified Date ： 2026-06-17

//! Rotary Position Embedding (RoPE)
//!
//! 提供预计算 sin/cos 缓存的 RoPE 实现，被 Qwen3、Llama、DeepSeek V3 共用。

use candle_core::{DType, Device, Result, Tensor};

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
        let inv_freq: Vec<_> = (0..head_dim)
            .step_by(2)
            .map(|i| 1f32 / rope_theta.powf(i as f64 / head_dim as f64) as f32)
            .collect();
        let inv_freq_len = inv_freq.len();
        let inv_freq =
            Tensor::from_vec(inv_freq, (1, inv_freq_len), dev)?.to_dtype(dtype)?;
        let t = Tensor::arange(0u32, max_position_embeddings as u32, dev)?
            .to_dtype(dtype)?
            .reshape((max_position_embeddings, 1))?;
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
