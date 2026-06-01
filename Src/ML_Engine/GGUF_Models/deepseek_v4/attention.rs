//! Multi-head Latent Attention (MLA) with learnable softmax sink + KV cache.
//! Adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01
//!
//! Key additions over MScanter:
//! - `Mla` has `kv_cache: Option<Tensor>` for incremental KV caching
//! - `forward` is `&mut self` to update the cache
//! - `clear_kv_cache`, `extract_kv_cache`, `restore_kv_cache` for offload support

use super::rope::Rope;
use super::sparse::{compress_topk_idxs, window_topk_idxs, Compressor, Indexer};
use candle_core::{DType, Result, Tensor, D};

// ── Utility functions (used by sparse.rs too) ──

pub(crate) fn linear(x: &Tensor, w: &Tensor) -> Result<Tensor> {
    let dims = x.dims().to_vec();
    let in_f = *dims.last().expect("non-scalar input");
    let rows: usize = dims[..dims.len() - 1].iter().product();
    let out_f = w.dim(0)?;
    let y = x.reshape((rows, in_f))?.matmul(&w.t()?.contiguous()?)?;
    let mut out_dims = dims[..dims.len() - 1].to_vec();
    out_dims.push(out_f);
    y.reshape(out_dims)
}

pub(crate) fn rms_norm(x: &Tensor, gamma: Option<&Tensor>, eps: f64) -> Result<Tensor> {
    let x = x.to_dtype(DType::F32)?;
    let var = x.sqr()?.mean_keepdim(D::Minus1)?;
    let normed = x.broadcast_div(&var.affine(1.0, eps)?.sqrt()?)?;
    match gamma {
        Some(g) => normed.broadcast_mul(&g.to_dtype(DType::F32)?),
        None => Ok(normed),
    }
}

fn rope_tail(rope: &Rope, x: &Tensor, start_pos: usize, rd: usize, inverse: bool) -> Result<Tensor> {
    let d = x.dim(D::Minus1)?;
    let nope = d - rd;
    let head = x.narrow(D::Minus1, 0, nope)?.contiguous()?;
    let tail = x.narrow(D::Minus1, nope, rd)?.contiguous()?;
    let tail = rope.apply(&tail, start_pos, inverse)?;
    Tensor::cat(&[&head, &tail], D::Minus1)
}

pub(crate) fn rope_tail_at(rope: &Rope, x: &Tensor, positions: &[usize], rd: usize) -> Result<Tensor> {
    let d = x.dim(D::Minus1)?;
    let nope = d - rd;
    let head = x.narrow(D::Minus1, 0, nope)?.contiguous()?;
    let tail = x.narrow(D::Minus1, nope, rd)?.contiguous()?;
    let tail = rope.apply_at(&tail, positions, false)?;
    Tensor::cat(&[&head, &tail], D::Minus1)
}

// ── Attention functions ──

pub fn sparse_attn(
    q: &Tensor, kv: &Tensor, sink: &Tensor, topk_idxs: &Tensor, scale: f64,
) -> Result<Tensor> {
    let (b, s, _, _) = q.dims4()?;
    let n = kv.dim(1)?;
    let (_, _, topk) = topk_idxs.dims3()?;
    let idxs = topk_idxs.to_dtype(DType::I64)?.flatten_all()?.to_vec1::<i64>()?;
    let mut mask = vec![f32::NEG_INFINITY; b * s * n];
    for bi in 0..b {
        for i in 0..s {
            let row = (bi * s + i) * topk;
            for t in 0..topk {
                let idx = idxs[row + t];
                if idx >= 0 && (idx as usize) < n {
                    mask[(bi * s + i) * n + idx as usize] = 0.0;
                }
            }
        }
    }
    let mask = Tensor::from_vec(mask, (b, 1, s, n), q.device())?;
    attn_core(q, kv, sink, scale, Some(&mask))
}

fn attn_core(
    q: &Tensor, kv: &Tensor, sink: &Tensor, scale: f64, mask: Option<&Tensor>,
) -> Result<Tensor> {
    let (b, _, h, d) = q.dims4()?;
    let n = kv.dim(1)?;
    let q = q.to_dtype(DType::F32)?;
    let kv = kv.to_dtype(DType::F32)?;
    let qh = q.transpose(1, 2)?.contiguous()?;
    let kvh = kv.unsqueeze(1)?.broadcast_as((b, h, n, d))?.contiguous()?;
    let scores = qh.matmul(&kvh.transpose(2, 3)?.contiguous()?)?.affine(scale, 0.0)?;
    let scores = match mask {
        Some(m) => scores.broadcast_add(m)?,
        None => scores,
    };
    let m = scores.max_keepdim(D::Minus1)?;
    let exp_scores = scores.broadcast_sub(&m)?.exp()?;
    let sum_keys = exp_scores.sum_keepdim(D::Minus1)?;
    let sink_term = sink.reshape((1, h, 1, 1))?.broadcast_sub(&m)?.exp()?;
    let denom = (sum_keys + sink_term)?;
    let weights = exp_scores.broadcast_div(&denom)?;
    weights.matmul(&kvh)?.transpose(1, 2)?.contiguous()
}

// ── MLA Block ──

pub struct Mla {
    pub wq_a: Tensor,
    pub q_norm: Tensor,
    pub wq_b: Tensor,
    pub wkv: Tensor,
    pub kv_norm: Tensor,
    pub wo_a: Tensor,
    pub wo_b: Tensor,
    pub attn_sink: Tensor,
    pub n_heads: usize,
    pub head_dim: usize,
    pub rope_head_dim: usize,
    pub n_groups: usize,
    pub o_lora_rank: usize,
    pub window_size: usize,
    pub compress_ratio: usize,
    pub compressor: Option<Compressor>,
    pub indexer: Option<Indexer>,
    pub eps: f64,
    pub scale: f64,
    /// KV cache: single latent KV tensor [b, seq, head_dim].
    /// Built up incrementally across forward calls.
    pub kv_cache: Option<Tensor>,
}

impl Mla {
    pub fn forward(&mut self, x: &Tensor, rope: &Rope, start_pos: usize) -> Result<Tensor> {
        let (b, s, _) = x.dims3()?;
        let (h, hd, rd) = (self.n_heads, self.head_dim, self.rope_head_dim);

        // Q
        let qr = rms_norm(&linear(x, &self.wq_a)?, Some(&self.q_norm), self.eps)?;
        let q = linear(&qr, &self.wq_b)?.reshape((b, s, h, hd))?;
        let q = rms_norm(&q, None, self.eps)?;
        let q = rope_tail(rope, &q, start_pos, rd, false)?;

        // KV: single latent head
        let kv_new = rms_norm(&linear(x, &self.wkv)?, Some(&self.kv_norm), self.eps)?;
        let kv_new = rope_tail(rope, &kv_new.reshape((b, s, 1, hd))?, start_pos, rd, false)?;
        let kv_new = kv_new.reshape((b, s, hd))?;

        // Update KV cache
        let kv = if start_pos == 0 {
            self.kv_cache = Some(kv_new.clone());
            kv_new
        } else if let Some(ref cached) = self.kv_cache {
            let updated = Tensor::cat(&[cached, &kv_new], 1)?;
            self.kv_cache = Some(updated.clone());
            updated
        } else {
            self.kv_cache = Some(kv_new.clone());
            kv_new
        };

        // Sparse attention
        let seqlen = kv.dim(1)?;
        let k_win = self.window_size.min(seqlen);
        let window = window_topk_idxs(self.window_size, seqlen, x.device())?
            .unsqueeze(0)?.broadcast_as((b, s, k_win))?.contiguous()?;
        let (kv_attn, idxs) = match &self.compressor {
            Some(comp) => {
                let kv_compress = comp.compress(x, rope)?;
                let offset = seqlen;
                let cidxs = match &self.indexer {
                    Some(idx) => idx.select(x, &qr, rope)?,
                    None => {
                        let c = compress_topk_idxs(self.compress_ratio, seqlen, offset, x.device())?;
                        let cols = c.dim(1)?;
                        c.unsqueeze(0)?.broadcast_as((b, s, cols))?.contiguous()?
                    }
                };
                let kv_attn = Tensor::cat(&[&kv, &kv_compress], 1)?;
                let idxs = Tensor::cat(&[&window, &cidxs], D::Minus1)?;
                (kv_attn, idxs)
            }
            None => (kv, window),
        };
        let idxs = idxs.contiguous()?;
        let o = sparse_attn(&q, &kv_attn, &self.attn_sink, &idxs, self.scale)?;
        let o = rope_tail(rope, &o, start_pos, rd, true)?;

        // Grouped low-rank output projection
        let (g, r) = (self.n_groups, self.o_lora_rank);
        let din = h * hd / g;
        let o = o.reshape((b, s, g, din))?.permute((2, 0, 1, 3))?.reshape((g, b * s, din))?;
        let wa = self.wo_a.reshape((g, r, din))?.transpose(1, 2)?.contiguous()?;
        let og = o.matmul(&wa)?.reshape((g, b, s, r))?.permute((1, 2, 0, 3))?.reshape((b, s, g * r))?;
        linear(&og, &self.wo_b)
    }

    pub fn clear_kv_cache(&mut self) {
        self.kv_cache = None;
    }

    pub fn extract_kv_cache(&self) -> Result<(Tensor, Tensor)> {
        let k = self.kv_cache.clone()
            .ok_or_else(|| candle_core::Error::Msg("extract_kv_cache: empty".into()))?;
        // V4 has single latent KV — return it twice (as (k, v)) for compatibility
        Ok((k.clone(), k))
    }

    pub fn restore_kv_cache(&mut self, k: &Tensor, _v: &Tensor) -> Result<()> {
        self.kv_cache = Some(k.clone());
        Ok(())
    }
}

// ── LM Head ──

pub struct Head {
    pub weight: Tensor,
    pub norm: Tensor,
    pub hc_fn: Tensor,
    pub hc_base: Tensor,
    pub hc_scale: Tensor,
    pub hc: usize,
    pub eps: f64,
    pub hc_eps: f64,
}

fn sigmoid(x: &Tensor) -> Result<Tensor> {
    x.neg()?.exp()?.affine(1.0, 1.0)?.recip()
}

impl Head {
    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let collapsed = self.collapse(x)?;
        let normed = rms_norm(&collapsed, Some(&self.norm), self.eps)?;
        let (b, s, d) = normed.dims3()?;
        let last = normed.narrow(1, s - 1, 1)?.reshape((b, d))?;
        linear(&last, &self.weight)
    }

    fn collapse(&self, x: &Tensor) -> Result<Tensor> {
        let (b, s, hc, d) = x.dims4()?;
        let x = x.to_dtype(DType::F32)?;
        let xf = x.reshape((b * s, hc * d))?;
        let rms = xf.sqr()?.mean_keepdim(1)?.affine(1.0, self.eps)?.powf(-0.5)?;
        let mixes = linear(&xf, &self.hc_fn)?.broadcast_mul(&rms)?;
        let s0 = self.hc_scale.to_vec1::<f32>()?[0] as f64;
        let pre = mixes.affine(s0, 0.0)?.broadcast_add(&self.hc_base)?;
        let pre = sigmoid(&pre)?.affine(1.0, self.hc_eps)?;
        pre.reshape((b, s, hc, 1))?.broadcast_mul(&x)?.sum(2)
    }
}
