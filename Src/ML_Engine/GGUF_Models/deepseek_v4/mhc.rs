//! Manifold-Constrained Hyper-Connections (mHC) — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01

use candle_core::{DType, Result, Tensor, D};
use candle_nn::ops::softmax;

fn sigmoid(x: &Tensor) -> Result<Tensor> {
    x.neg()?.exp()?.affine(1.0, 1.0)?.recip()
}

pub fn hc_split_sinkhorn(
    mixes: &Tensor, hc_scale: &Tensor, hc_base: &Tensor,
    hc: usize, sinkhorn_iters: usize, eps: f64,
) -> Result<(Tensor, Tensor, Tensor)> {
    let n = mixes.dim(0)?;
    let s = hc_scale.to_vec1::<f32>()?;
    let (s0, s1, s2) = (s[0] as f64, s[1] as f64, s[2] as f64);

    let pre = mixes.narrow(1, 0, hc)?.affine(s0, 0.0)?
        .broadcast_add(&hc_base.narrow(0, 0, hc)?)?;
    let pre = sigmoid(&pre)?.affine(1.0, eps)?;

    let post = mixes.narrow(1, hc, hc)?.affine(s1, 0.0)?
        .broadcast_add(&hc_base.narrow(0, hc, hc)?)?;
    let post = sigmoid(&post)?.affine(2.0, 0.0)?;

    let comb = mixes.narrow(1, 2 * hc, hc * hc)?.affine(s2, 0.0)?
        .broadcast_add(&hc_base.narrow(0, 2 * hc, hc * hc)?)?
        .reshape((n, hc, hc))?;

    let mut comb = softmax(&comb, D::Minus1)?.affine(1.0, eps)?;
    let col = comb.sum_keepdim(1)?.affine(1.0, eps)?;
    comb = comb.broadcast_div(&col)?;

    for _ in 0..sinkhorn_iters.saturating_sub(1) {
        let row = comb.sum_keepdim(D::Minus1)?.affine(1.0, eps)?;
        comb = comb.broadcast_div(&row)?;
        let col = comb.sum_keepdim(1)?.affine(1.0, eps)?;
        comb = comb.broadcast_div(&col)?;
    }

    Ok((pre, post, comb))
}

pub struct Hc {
    pub hc_fn: Tensor,
    pub hc_base: Tensor,
    pub hc_scale: Tensor,
    pub hc: usize,
    pub sinkhorn_iters: usize,
    pub eps: f64,
    pub norm_eps: f64,
}

impl Hc {
    pub fn pre(&self, x: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let (b, s, hcn, d) = x.dims4()?;
        let x = x.to_dtype(DType::F32)?;
        let xf = x.reshape((b * s, hcn * d))?;
        let var = xf.sqr()?.mean_keepdim(1)?;
        let rms = var.affine(1.0, self.norm_eps)?.powf(-0.5)?;
        let mixes = xf.matmul(&self.hc_fn.t()?)?.broadcast_mul(&rms)?;
        let (pre, post, comb) = hc_split_sinkhorn(
            &mixes, &self.hc_scale, &self.hc_base, self.hc, self.sinkhorn_iters, self.eps,
        )?;
        let pre = pre.reshape((b, s, hcn, 1))?;
        let y = pre.broadcast_mul(&x)?.sum(2)?;
        Ok((y, post.reshape((b, s, hcn))?, comb.reshape((b, s, hcn, hcn))?))
    }

    pub fn post(
        &self,
        x: &Tensor,
        residual: &Tensor,
        post: &Tensor,
        comb: &Tensor,
    ) -> Result<Tensor> {
        let (b, s, d) = x.dims3()?;
        let hc = self.hc;
        let x = x.to_dtype(DType::F32)?;
        let residual = residual.to_dtype(DType::F32)?;
        let term1 = post.reshape((b, s, hc, 1))?
            .broadcast_mul(&x.reshape((b, s, 1, d))?)?;
        let comb_e = comb.reshape((b, s, hc, hc, 1))?;
        let res_e = residual.reshape((b, s, hc, 1, d))?;
        let term2 = comb_e.broadcast_mul(&res_e)?.sum(2)?;
        term1 + term2
    }
}
