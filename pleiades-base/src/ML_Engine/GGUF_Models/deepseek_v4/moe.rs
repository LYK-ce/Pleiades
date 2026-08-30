//! Mixture-of-Experts — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01
//!
//! Key additions over MScanter:
//! - `Gate::route_hashed`: hash-routing path for n_hash_layers

use super::attention::linear;
use candle_core::{DType, Result, Tensor, D};

fn sqrtsoftplus(x: &Tensor) -> Result<Tensor> {
    let ln_term = x.abs()?.neg()?.exp()?.affine(1.0, 1.0)?.log()?;
    x.relu()?.broadcast_add(&ln_term)?.sqrt()
}

fn softmax_last(x: &Tensor) -> Result<Tensor> {
    let e = x.broadcast_sub(&x.max_keepdim(D::Minus1)?)?.exp()?;
    e.broadcast_div(&e.sum_keepdim(D::Minus1)?)
}

fn sigmoid_fn(x: &Tensor) -> Result<Tensor> {
    let denom = x.neg()?.exp()?.affine(1.0, 1.0)?;
    Tensor::ones_like(&denom)?.broadcast_div(&denom)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreFunc {
    SqrtSoftplus,
    Softmax,
    Sigmoid,
}

pub struct Gate {
    pub weight: Tensor,
    pub bias: Option<Tensor>,
    pub topk: usize,
    pub route_scale: f64,
    pub score_func: ScoreFunc,
    /// Hash routing table: [vocab_size, n_routed_experts] — maps token IDs to expert IDs.
    /// Present only for hash-routed layers (first n_hash_layers).
    pub tid2eid: Option<Tensor>,
}

impl Gate {
    /// Score-based routing (standard path)
    pub fn route(&self, x: &Tensor) -> Result<(Tensor, Tensor)> {
        let raw = linear(x, &self.weight)?;
        let scores = match self.score_func {
            ScoreFunc::SqrtSoftplus => sqrtsoftplus(&raw)?,
            ScoreFunc::Softmax => softmax_last(&raw)?,
            ScoreFunc::Sigmoid => sigmoid_fn(&raw)?,
        };
        let biased = match &self.bias {
            Some(b) => scores.broadcast_add(b)?,
            None => scores.clone(),
        };
        let (n, e) = scores.dims2()?;
        let k = self.topk.min(e);
        let sc = scores.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()?;
        let bi = biased.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()?;
        let mut wv = vec![0f32; n * k];
        let mut iv = vec![0i64; n * k];
        for r in 0..n {
            let base = r * e;
            let mut cand: Vec<(f32, usize)> = (0..e).map(|j| (bi[base + j], j)).collect();
            cand.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            for (t, &(_, j)) in cand.iter().take(k).enumerate() {
                iv[r * k + t] = j as i64;
                wv[r * k + t] = sc[base + j];
            }
        }
        let weights = Tensor::from_vec(wv, (n, k), x.device())?;
        let indices = Tensor::from_vec(iv, (n, k), x.device())?;
        let weights = if matches!(self.score_func, ScoreFunc::Softmax) {
            weights.affine(self.route_scale, 0.0)?
        } else {
            let denom = weights.sum_keepdim(D::Minus1)?;
            weights.broadcast_div(&denom)?.affine(self.route_scale, 0.0)?
        };
        Ok((weights, indices))
    }

    /// Hash-based routing: looks up expert IDs from tid2eid[input_ids].
    /// Used for hash-routed layers (first n_hash_layers).
    pub fn route_hashed(&self, input_ids: &Tensor) -> Result<(Tensor, Tensor)> {
        let tid2eid = self.tid2eid.as_ref()
            .ok_or_else(|| candle_core::Error::Msg("route_hashed: tid2eid not loaded".into()))?;
        let n = input_ids.dims1()?;
        let e = tid2eid.dim(1)?;
        let k = self.topk.min(e);
        // Look up expert IDs per token from tid2eid table
        let expert_ids = tid2eid.index_select(input_ids, 0)?; // [n, e]
        // Take top k experts (first k columns since tid2eid is pre-ordered)
        let indices = expert_ids.narrow(1, 0, k)?; // [n, k]
        // Uniform weights
        let weights = Tensor::ones((n, k), DType::F32, input_ids.device())?
            .affine(self.route_scale / k as f64, 0.0)?;
        Ok((weights, indices.to_dtype(DType::I64)?))
    }
}

pub struct Expert {
    pub w1: Tensor,
    pub w2: Tensor,
    pub w3: Tensor,
    pub swiglu_limit: f64,
}

impl Expert {
    pub fn forward(&self, x: &Tensor, weights: Option<&Tensor>) -> Result<Tensor> {
        let mut gate = linear(x, &self.w1)?;
        let mut up = linear(x, &self.w3)?;
        if self.swiglu_limit > 0.0 {
            let l = self.swiglu_limit;
            up = up.clamp(-l, l)?;
            gate = gate.minimum(l)?;
        }
        let mut h = gate.silu()?.mul(&up)?;
        if let Some(w) = weights {
            h = h.broadcast_mul(w)?;
        }
        linear(&h, &self.w2)
    }
}

pub struct Moe {
    pub gate: Gate,
    pub experts: Vec<Expert>,
    pub shared: Expert,
}

impl Moe {
    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let (n, dim) = x.dims2()?;
        let (weights, indices) = self.gate.route(x)?;
        let topk = self.gate.topk;
        let wv = weights.flatten_all()?.to_vec1::<f32>()?;
        let iv = indices.flatten_all()?.to_vec1::<i64>()?;

        let mut y = Tensor::zeros((n, dim), DType::F32, x.device())?;
        for (e, expert) in self.experts.iter().enumerate() {
            let mut rows = Vec::new();
            let mut ws = Vec::new();
            for t in 0..n {
                for s in 0..topk {
                    if iv[t * topk + s] == e as i64 {
                        rows.push(t as u32);
                        ws.push(wv[t * topk + s]);
                    }
                }
            }
            if rows.is_empty() { continue; }
            let sel = Tensor::from_vec(rows.clone(), (rows.len(),), x.device())?;
            let xe = x.index_select(&sel, 0)?;
            let we = Tensor::from_vec(ws, (rows.len(), 1), x.device())?;
            let ye = expert.forward(&xe, Some(&we))?;
            y = y.index_add(&sel, &ye, 0)?;
        }
        let ys = self.shared.forward(x, None)?;
        y.broadcast_add(&ys)
    }

    /// Hash-routed forward: uses token IDs instead of hidden states for routing.
    /// Only for hash-routed layers (first n_hash_layers).
    pub fn forward_hashed(&self, x: &Tensor, input_ids: &Tensor) -> Result<Tensor> {
        let (n, dim) = x.dims2()?;
        let (weights, indices) = self.gate.route_hashed(input_ids)?;
        let topk = self.gate.topk;
        let wv = weights.flatten_all()?.to_vec1::<f32>()?;
        let iv = indices.flatten_all()?.to_vec1::<i64>()?;

        let mut y = Tensor::zeros((n, dim), DType::F32, x.device())?;
        for (e, expert) in self.experts.iter().enumerate() {
            let mut rows = Vec::new();
            let mut ws = Vec::new();
            for t in 0..n {
                for s in 0..topk {
                    if iv[t * topk + s] == e as i64 {
                        rows.push(t as u32);
                        ws.push(wv[t * topk + s]);
                    }
                }
            }
            if rows.is_empty() { continue; }
            let sel = Tensor::from_vec(rows.clone(), (rows.len(),), x.device())?;
            let xe = x.index_select(&sel, 0)?;
            let we = Tensor::from_vec(ws, (rows.len(), 1), x.device())?;
            let ye = expert.forward(&xe, Some(&we))?;
            y = y.index_add(&sel, &ye, 0)?;
        }
        let ys = self.shared.forward(x, None)?;
        y.broadcast_add(&ys)
    }
}
