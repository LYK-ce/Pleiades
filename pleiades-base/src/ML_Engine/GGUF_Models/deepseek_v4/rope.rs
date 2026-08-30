//! YaRN rotary position embeddings — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01

use candle_core::{DType, Device, Result, Tensor};

pub struct Rope {
    cos: Tensor,
    sin: Tensor,
}

impl Rope {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dim: usize, max_seq: usize,
        original_seq_len: usize, base: f64,
        factor: f64, beta_fast: f64, beta_slow: f64,
        dev: &Device,
    ) -> Result<Self> {
        let half = dim / 2;
        let mut freqs: Vec<f64> = (0..half)
            .map(|i| 1.0 / base.powf((2 * i) as f64 / dim as f64))
            .collect();

        if original_seq_len > 0 {
            let (low, high) = correction_range(beta_fast, beta_slow, dim, base, original_seq_len);
            let denom = if low == high { 0.001 } else { high - low };
            for (i, f) in freqs.iter_mut().enumerate() {
                let ramp = (((i as f64) - low) / denom).clamp(0.0, 1.0);
                let smooth = 1.0 - ramp;
                *f = *f / factor * (1.0 - smooth) + *f * smooth;
            }
        }

        let mut cos = Vec::with_capacity(max_seq * half);
        let mut sin = Vec::with_capacity(max_seq * half);
        for p in 0..max_seq {
            for &f in &freqs {
                let angle = p as f64 * f;
                cos.push(angle.cos() as f32);
                sin.push(angle.sin() as f32);
            }
        }
        Ok(Self {
            cos: Tensor::from_vec(cos, (max_seq, half), dev)?,
            sin: Tensor::from_vec(sin, (max_seq, half), dev)?,
        })
    }

    pub fn apply(&self, x: &Tensor, start_pos: usize, inverse: bool) -> Result<Tensor> {
        let s = x.dim(1)?;
        let cos = self.cos.narrow(0, start_pos, s)?;
        let sin = self.sin.narrow(0, start_pos, s)?;
        self.apply_rows(x, &cos, &sin, inverse)
    }

    pub fn apply_at(&self, x: &Tensor, positions: &[usize], inverse: bool) -> Result<Tensor> {
        let idx = Tensor::from_vec(
            positions.iter().map(|&p| p as u32).collect::<Vec<_>>(),
            (positions.len(),),
            self.cos.device(),
        )?;
        let cos = self.cos.index_select(&idx, 0)?;
        let sin = self.sin.index_select(&idx, 0)?;
        self.apply_rows(x, &cos, &sin, inverse)
    }

    fn apply_rows(&self, x: &Tensor, cos: &Tensor, sin: &Tensor, inverse: bool) -> Result<Tensor> {
        let (b, s, h, d) = x.dims4()?;
        let half = d / 2;
        let x = x.to_dtype(DType::F32)?;
        let cos = cos.reshape((1, s, 1, half))?;
        let sin = sin.reshape((1, s, 1, half))?;
        let sin = if inverse { sin.neg()? } else { sin };
        let xr = x.reshape((b, s, h, half, 2))?;
        let x_even = xr.narrow(4, 0, 1)?.contiguous()?.reshape((b, s, h, half))?;
        let x_odd = xr.narrow(4, 1, 1)?.contiguous()?.reshape((b, s, h, half))?;
        let out_even = (x_even.broadcast_mul(&cos)? - x_odd.broadcast_mul(&sin)?)?;
        let out_odd = (x_even.broadcast_mul(&sin)? + x_odd.broadcast_mul(&cos)?)?;
        Tensor::stack(&[&out_even, &out_odd], 4)?.reshape((b, s, h, d))
    }
}

fn correction_dim(num_rotations: f64, dim: usize, base: f64, max_seq_len: usize) -> f64 {
    (dim as f64) * (max_seq_len as f64 / (num_rotations * 2.0 * std::f64::consts::PI)).ln()
        / (2.0 * base.ln())
}

fn correction_range(
    low_rot: f64, high_rot: f64, dim: usize, base: f64, max_seq_len: usize,
) -> (f64, f64) {
    let low = correction_dim(low_rot, dim, base, max_seq_len).floor().max(0.0);
    let high = correction_dim(high_rot, dim, base, max_seq_len).ceil().min((dim - 1) as f64);
    (low, high)
}
