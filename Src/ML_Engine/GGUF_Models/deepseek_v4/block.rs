//! One V4 decoder block — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01
//!
//! Key additions:
//! - `clear_kv_cache`, `extract_kv_cache`, `restore_kv_cache` for offload support
//! - Hash-routing awareness for MoE sublayer

use super::attention::{rms_norm, Mla};
use super::mhc::Hc;
use super::moe::Moe;
use super::rope::Rope;
use candle_core::{Result, Tensor};

pub struct Block {
    pub attn: Mla,
    pub ffn: Moe,
    pub attn_norm: Tensor,
    pub ffn_norm: Tensor,
    pub hc_attn: Hc,
    pub hc_ffn: Hc,
    pub eps: f64,
    /// Whether this block uses hash routing (layer < n_hash_layers)
    pub is_hash_routed: bool,
}

impl Block {
    pub fn forward(&mut self, x: &Tensor, rope: &Rope, start_pos: usize) -> Result<Tensor> {
        // Attention sublayer
        let (collapsed, post, comb) = self.hc_attn.pre(x)?;
        let normed = rms_norm(&collapsed, Some(&self.attn_norm), self.eps)?;
        let attended = self.attn.forward(&normed, rope, start_pos)?;
        let x = self.hc_attn.post(&attended, x, &post, &comb)?;

        // MoE sublayer
        let (collapsed, post, comb) = self.hc_ffn.pre(&x)?;
        let normed = rms_norm(&collapsed, Some(&self.ffn_norm), self.eps)?;
        let (b, s, d) = normed.dims3()?;
        let ff = self.ffn.forward(&normed.reshape((b * s, d))?)?.reshape((b, s, d))?;
        self.hc_ffn.post(&ff, &x, &post, &comb)
    }

    pub fn clear_kv_cache(&mut self) {
        self.attn.clear_kv_cache();
    }

    pub fn extract_kv_cache(&self) -> Result<(Tensor, Tensor)> {
        self.attn.extract_kv_cache()
    }

    pub fn restore_kv_cache(&mut self, k: &Tensor, v: &Tensor) -> Result<()> {
        self.attn.restore_kv_cache(k, v)
    }
}
