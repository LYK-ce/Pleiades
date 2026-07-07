//Presented by KeJi
//Created Date ： 2026-07-04
//Modified Date ： 2026-07-04

//! 统一模型抽象 — 模型级别的推理接口

use anyhow::Result;
use candle_core::Tensor;

use super::stage::Stage;

/// 模型 trait — Qwen3 / Qwen3MoE 均实现此 trait
///
/// 对应 Stage 的单步计算，Model 是完整的模型推理单元。
pub trait Model: Send {
    fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor>;
    fn Clear_Kv_Cache(&mut self);
    fn extract_kv_cache(&self) -> Result<Vec<(Tensor, Tensor)>, String>;
    fn restore_kv_cache(&mut self, kvs: Vec<(Tensor, Tensor)>) -> Result<(), String>;
}

/// 从 stages 中提取所有层的 KV Cache
pub fn extract_kv_cache_from_stages(stages: &[Box<dyn Stage>]) -> Result<Vec<(Tensor, Tensor)>, String> {
    stages.iter().filter_map(|s| s.kv_cache()).next()
        .ok_or_else(|| "extract_kv_cache: no kv cache layers".to_string())?;
    stages.iter().filter_map(|s| {
        s.kv_cache().map(|kv| {
            let k = kv.k().ok_or_else(|| "extract_kv_cache: empty".to_string())?;
            let v = kv.v().ok_or_else(|| "extract_kv_cache: empty".to_string())?;
            Ok::<_, String>((k.clone(), v.clone()))
        })
    }).collect()
}

/// 将 KV Cache 恢复到 stages 的各层中
pub fn restore_kv_cache_to_stages(stages: &mut [Box<dyn Stage>], kvs: Vec<(Tensor, Tensor)>) -> Result<(), String> {
    let kv_count = stages.iter().filter(|s| s.kv_cache().is_some()).count();
    if kvs.len() != kv_count {
        return Err(format!("restore_kv_cache: layer count mismatch (expected {}, got {})", kv_count, kvs.len()));
    }
    let mut kv_iter = kvs.into_iter();
    for stage in stages.iter_mut() {
        if let Some(kv) = stage.kv_cache_mut() {
            let (k, v) = kv_iter.next().unwrap();
            kv.append(&k, &v)
                .map_err(|e| format!("restore_kv_cache: append failed: {e}"))?;
        }
    }
    Ok(())
}
