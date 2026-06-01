//! Sparse KV selection — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01

use super::attention::{linear, rms_norm, rope_tail_at};
use super::rope::Rope;
use candle_core::{DType, Device, Result, Tensor, D};

fn softmax_dim(x: &Tensor, dim: usize) -> Result<Tensor> {
    let m = x.max_keepdim(dim)?;
    let e = x.broadcast_sub(&m)?.exp()?;
    e.broadcast_div(&e.sum_keepdim(dim)?)
}

fn gated_pool(values: &Tensor, gates: &Tensor) -> Result<Tensor> {
    let w = softmax_dim(gates, 2)?;
    values.mul(&w)?.sum(2)
}

fn overlap_windows(t: &Tensor, d: usize, fill: f32) -> Result<Tensor> {
    let (b, nb, ratio, _) = t.dims4()?;
    let normal = t.narrow(3, d, d)?;
    let first = t.narrow(3, 0, d)?;
    let fill_block = Tensor::full(fill, (b, 1, ratio, d), t.device())?;
    let prev = if nb > 1 {
        Tensor::cat(&[&fill_block, &first.narrow(1, 0, nb - 1)?], 1)?
    } else {
        fill_block
    };
    Tensor::cat(&[&prev, &normal], 2)
}

pub fn window_topk_idxs(window: usize, seqlen: usize, dev: &Device) -> Result<Tensor> {
    let k = window.min(seqlen);
    let mut data = vec![-1i64; seqlen * k];
    for i in 0..seqlen {
        let lo = (i + 1).saturating_sub(window);
        for j in 0..k {
            let key = lo + j;
            if key <= i { data[i * k + j] = key as i64; }
        }
    }
    Tensor::from_vec(data, (seqlen, k), dev)
}

pub fn compress_topk_idxs(ratio: usize, seqlen: usize, offset: usize, dev: &Device) -> Result<Tensor> {
    let cols = seqlen / ratio;
    let mut data = vec![-1i64; seqlen * cols];
    for i in 0..seqlen {
        let visible = ((i + 1) / ratio).min(cols);
        for c in 0..visible { data[i * cols + c] = (c + offset) as i64; }
    }
    Tensor::from_vec(data, (seqlen, cols), dev)
}

pub struct Compressor {
    pub wkv: Tensor,
    pub wgate: Tensor,
    pub ape: Tensor,
    pub norm: Tensor,
    pub compress_ratio: usize,
    pub head_dim: usize,
    pub rope_head_dim: usize,
    pub eps: f64,
}

impl Compressor {
    pub fn compress(&self, x: &Tensor, rope: &Rope) -> Result<Tensor> {
        let (b, s, _) = x.dims3()?;
        let (ratio, d, rd) = (self.compress_ratio, self.head_dim, self.rope_head_dim);
        let nb = s / ratio;
        let overlap = ratio == 4;
        let proj_d = if overlap { 2 * d } else { d };
        let kv = linear(x, &self.wkv)?.reshape((b, nb, ratio, proj_d))?;
        let ape = self.ape.reshape((1, 1, ratio, proj_d))?;
        let score = linear(x, &self.wgate)?.reshape((b, nb, ratio, proj_d))?.broadcast_add(&ape)?;
        let pooled = if overlap {
            let kv = overlap_windows(&kv, d, 0.0)?;
            let score = overlap_windows(&score, d, f32::NEG_INFINITY)?;
            gated_pool(&kv, &score)?
        } else {
            gated_pool(&kv, &score)?
        };
        let pooled = rms_norm(&pooled, Some(&self.norm), self.eps)?;
        if rd == 0 { return Ok(pooled); }
        let positions: Vec<usize> = (0..nb).map(|i| i * ratio).collect();
        let roped = rope_tail_at(rope, &pooled.reshape((b, nb, 1, d))?, &positions, rd)?;
        roped.reshape((b, nb, d))
    }
}

pub struct Indexer {
    pub wq_b: Tensor,
    pub weights_proj: Tensor,
    pub compressor: Compressor,
    pub n_heads: usize,
    pub head_dim: usize,
    pub rope_head_dim: usize,
    pub index_topk: usize,
    pub compress_ratio: usize,
    pub scale: f64,
}

impl Indexer {
    pub fn select(&self, x: &Tensor, qr: &Tensor, rope: &Rope) -> Result<Tensor> {
        let (b, s, _) = x.dims3()?;
        let (h, hd, rd, ratio) = (self.n_heads, self.head_dim, self.rope_head_dim, self.compress_ratio);
        let nb = s / ratio;
        let offset = s;
        let k = self.index_topk.min(nb);
        let q = linear(qr, &self.wq_b)?.reshape((b, s, h, hd))?;
        let q = if rd > 0 {
            let positions: Vec<usize> = (0..s).collect();
            rope_tail_at(rope, &q, &positions, rd)?
        } else { q };
        let kv = self.compressor.compress(x, rope)?;
        let wsc = self.scale * (h as f64).powf(-0.5);
        let weights = linear(x, &self.weights_proj)?.affine(wsc, 0.0)?;
        let qm = q.reshape((b, s * h, hd))?;
        let kvt = kv.transpose(1, 2)?.contiguous()?;
        let scores = qm.matmul(&kvt)?.reshape((b, s, h, nb))?.relu()?;
        let scores = scores.broadcast_mul(&weights.unsqueeze(D::Minus1)?)?;
        let index_score = scores.sum(2)?;
        let flat = index_score.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()?;
        let mut out = vec![-1i64; b * s * k];
        for bi in 0..b {
            for i in 0..s {
                let visible = (i + 1) / ratio;
                let base = (bi * s + i) * nb;
                let mut cand: Vec<(f32, usize)> = (0..nb)
                    .filter(|&t| t < visible)
                    .map(|t| (flat[base + t], t)).collect();
                cand.sort_by(|a, c| c.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                for (j, &(_, t)) in cand.iter().take(k).enumerate() {
                    out[(bi * s + i) * k + j] = (t + offset) as i64;
                }
            }
        }
        Tensor::from_vec(out, (b, s, k), x.device())
    }
}
