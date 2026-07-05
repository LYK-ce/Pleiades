//Presented by KeJi
//Created Date ： 2026-07-04
//Modified Date ： 2026-07-04

//! 推理阶段抽象 — 模型每个计算步骤统一为 Stage trait

use anyhow::Result;
use candle_core::Tensor;


/// 推理阶段 — Embedding、Transformer层、输出头均实现此 trait
pub trait Stage: Send {
    fn forward(&mut self, x: &Tensor, offset: usize, mask: Option<&Tensor>) -> Result<Tensor>;
    fn kv_cache(&self) -> Option<&candle_nn::kv_cache::ConcatKvCache> { None }
    fn kv_cache_mut(&mut self) -> Option<&mut candle_nn::kv_cache::ConcatKvCache> { None }
    fn clear_kv_cache(&mut self) {
        if let Some(kv) = self.kv_cache_mut() { kv.reset(); }
    }
}




