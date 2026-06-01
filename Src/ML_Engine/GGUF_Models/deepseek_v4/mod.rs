//! DeepSeek-V4 architecture in Rust + candle — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01

pub mod quant;
pub mod config;
pub mod rope;
pub mod loader;
pub mod sparse;
pub mod attention;
pub mod moe;
pub mod mhc;
pub mod block;

use candle_core::{Device, Result, Tensor};
use config::Config;
use loader::MultiSafeTensors;
use block::Block;
use attention::Head;
use rope::Rope;

/// DeepSeek V4 完整模型
pub struct DeepSeekV4Model {
    pub embed: Tensor,
    pub layers: Vec<Block>,
    pub head: Head,
    pub ropes: Vec<Rope>,
    pub hc: usize,
}

impl DeepSeekV4Model {
    /// 从 PGGUF 的多个 safetensors shard tensor blob 加载模型
    pub fn from_pgguf(
        cfg: &Config,
        shard_bytes: &[Vec<u8>],
        dev: &Device,
    ) -> Result<Self> {
        let st = MultiSafeTensors::from_shards(shard_bytes)?;
        Transformer::from_config(cfg, &st, dev)
    }

    /// 前向推理 (单步)
    pub fn forward(&mut self, input_ids: &Tensor, start_pos: usize) -> Result<Tensor> {
        let (b, s) = input_ids.dims2()?;
        let dim = self.embed.dim(1)?;

        let ids = input_ids.flatten_all()?.to_dtype(candle_core::DType::U32)?;
        let h = self.embed.index_select(&ids, 0)?.reshape((b, s, dim))?;
        let mut h = h.unsqueeze(2)?.broadcast_as((b, s, self.hc, dim))?.contiguous()?;

        for (l, layer) in self.layers.iter_mut().enumerate() {
            h = layer.forward(&h, &self.ropes[l], start_pos)?;
        }
        self.head.forward(&h)
    }

    /// 清除 KV cache
    pub fn clear_kv_cache(&mut self) {
        for layer in &mut self.layers {
            layer.clear_kv_cache();
        }
    }

    /// 提取 KV cache
    pub fn extract_kv_cache(&self) -> Result<Vec<(Tensor, Tensor)>> {
        self.layers.iter().map(|layer| {
            layer.extract_kv_cache()
        }).collect()
    }

    /// 恢复 KV cache
    pub fn restore_kv_cache(&mut self, kvs: Vec<(Tensor, Tensor)>) -> Result<()> {
        if kvs.len() != self.layers.len() {
            return Err(candle_core::Error::Msg(format!(
                "restore_kv_cache: layer count mismatch (expected {}, got {})",
                self.layers.len(), kvs.len()
            )));
        }
        for (layer, (k, v)) in self.layers.iter_mut().zip(kvs) {
            layer.restore_kv_cache(&k, &v)?;
        }
        Ok(())
    }
}

// Re-export Transformer from model.rs for from_config
use model::Transformer;
mod model;
