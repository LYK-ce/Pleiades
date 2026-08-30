//! Full model assembly: token embedding → mHC-expanded decoder stack → parallel LM head.
//! Adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01

use super::attention::{Head, Mla};
use super::block::Block;
use super::config::Config;
use super::loader::MultiSafeTensors;
use super::mhc::Hc;
use super::moe::{Expert, Gate, Moe, ScoreFunc};
use super::rope::Rope;
use super::sparse::{Compressor, Indexer};
use candle_core::{Device, Result};

pub struct Transformer;

impl Transformer {
    pub fn from_config(cfg: &Config, st: &MultiSafeTensors, dev: &Device) -> Result<super::DeepSeekV4Model> {
        // Hash-routing not yet wired
        if cfg.n_hash_layers > 0 {
            return Err(candle_core::Error::Msg(format!(
                "from_config: hash-routing layers (n_hash_layers={}) not yet supported",
                cfg.n_hash_layers
            )));
        }

        let embed = st.auto_tensor("embed.weight", &[cfg.vocab_size, cfg.dim], dev)?;
        let layers = (0..cfg.n_layers)
            .map(|l| build_block(cfg, st, l, dev))
            .collect::<Result<Vec<_>>>()?;
        let head = build_head(cfg, st, dev)?;
        let ropes = (0..cfg.n_layers)
            .map(|l| build_rope(cfg, l, dev))
            .collect::<Result<Vec<_>>>()?;
        Ok(super::DeepSeekV4Model { embed, layers, head, ropes, hc: cfg.hc_mult })
    }
}

fn build_rope(cfg: &Config, layer: usize, dev: &Device) -> Result<Rope> {
    let (original_seq_len, theta) = if cfg.compress_ratios[layer] != 0 {
        (cfg.original_seq_len, cfg.compress_rope_theta)
    } else {
        (0, cfg.rope_theta)
    };
    Rope::new(
        cfg.rope_head_dim, cfg.max_seq_len,
        original_seq_len, theta,
        cfg.rope_factor, cfg.beta_fast, cfg.beta_slow,
        dev,
    )
}

fn score_func(name: &str) -> Result<ScoreFunc> {
    match name {
        "sqrtsoftplus" => Ok(ScoreFunc::SqrtSoftplus),
        "softmax" => Ok(ScoreFunc::Softmax),
        "sigmoid" => Ok(ScoreFunc::Sigmoid),
        other => Err(candle_core::Error::Msg(format!("unknown score_func {other:?}"))),
    }
}

fn build_mla(cfg: &Config, st: &MultiSafeTensors, layer: usize, dev: &Device) -> Result<Mla> {
    let p = format!("layers.{layer}.attn");
    let ratio = cfg.compress_ratios[layer];
    Ok(Mla {
        wq_a: st.linear(&format!("{p}.wq_a"), cfg.q_lora_rank, cfg.dim, false, dev)?,
        q_norm: st.auto_tensor(&format!("{p}.q_norm.weight"), &[cfg.q_lora_rank], dev)?,
        wq_b: st.linear(&format!("{p}.wq_b"), cfg.n_heads * cfg.head_dim, cfg.q_lora_rank, false, dev)?,
        wkv: st.linear(&format!("{p}.wkv"), cfg.head_dim, cfg.dim, false, dev)?,
        kv_norm: st.auto_tensor(&format!("{p}.kv_norm.weight"), &[cfg.head_dim], dev)?,
        wo_a: st.linear(
            &format!("{p}.wo_a"),
            cfg.o_groups * cfg.o_lora_rank,
            cfg.n_heads * cfg.head_dim / cfg.o_groups,
            false, dev,
        )?,
        wo_b: st.linear(&format!("{p}.wo_b"), cfg.dim, cfg.o_groups * cfg.o_lora_rank, false, dev)?,
        attn_sink: st.auto_tensor(&format!("{p}.attn_sink"), &[cfg.n_heads], dev)?,
        n_heads: cfg.n_heads,
        head_dim: cfg.head_dim,
        rope_head_dim: cfg.rope_head_dim,
        n_groups: cfg.o_groups,
        o_lora_rank: cfg.o_lora_rank,
        window_size: cfg.window_size,
        compress_ratio: ratio,
        compressor: if ratio > 0 {
            Some(build_compressor(&format!("{p}.compressor"), ratio, cfg.head_dim, cfg, st, dev)?)
        } else { None },
        indexer: if ratio == 4 {
            Some(build_indexer(&p, cfg, st, dev)?)
        } else { None },
        eps: cfg.norm_eps,
        scale: (cfg.head_dim as f64).powf(-0.5),
        kv_cache: None,
    })
}

fn build_compressor(
    prefix: &str, ratio: usize, head_dim: usize,
    cfg: &Config, st: &MultiSafeTensors, dev: &Device,
) -> Result<Compressor> {
    let coff = if ratio == 4 { 2 } else { 1 };
    Ok(Compressor {
        wkv: st.linear(&format!("{prefix}.wkv"), coff * head_dim, cfg.dim, false, dev)?,
        wgate: st.linear(&format!("{prefix}.wgate"), coff * head_dim, cfg.dim, false, dev)?,
        ape: st.auto_tensor(&format!("{prefix}.ape"), &[ratio, coff * head_dim], dev)?,
        norm: st.auto_tensor(&format!("{prefix}.norm.weight"), &[head_dim], dev)?,
        compress_ratio: ratio,
        head_dim,
        rope_head_dim: cfg.rope_head_dim,
        eps: cfg.norm_eps,
    })
}

fn build_indexer(p: &str, cfg: &Config, st: &MultiSafeTensors, dev: &Device) -> Result<Indexer> {
    let (ih, ihd) = (cfg.index_n_heads, cfg.index_head_dim);
    Ok(Indexer {
        wq_b: st.linear(&format!("{p}.indexer.wq_b"), ih * ihd, cfg.q_lora_rank, false, dev)?,
        weights_proj: st.linear(&format!("{p}.indexer.weights_proj"), ih, cfg.dim, false, dev)?,
        compressor: build_compressor(&format!("{p}.indexer.compressor"), 4, ihd, cfg, st, dev)?,
        n_heads: ih,
        head_dim: ihd,
        rope_head_dim: cfg.rope_head_dim,
        index_topk: cfg.index_topk,
        compress_ratio: 4,
        scale: (ihd as f64).powf(-0.5),
    })
}

fn build_expert(
    prefix: &str, cfg: &Config, st: &MultiSafeTensors, fp4: bool, dev: &Device,
) -> Result<Expert> {
    Ok(Expert {
        w1: st.linear(&format!("{prefix}.w1"), cfg.moe_inter_dim, cfg.dim, fp4, dev)?,
        w2: st.linear(&format!("{prefix}.w2"), cfg.dim, cfg.moe_inter_dim, fp4, dev)?,
        w3: st.linear(&format!("{prefix}.w3"), cfg.moe_inter_dim, cfg.dim, fp4, dev)?,
        swiglu_limit: cfg.swiglu_limit,
    })
}

fn build_moe(cfg: &Config, st: &MultiSafeTensors, layer: usize, dev: &Device) -> Result<Moe> {
    let p = format!("layers.{layer}.ffn");
    let experts = (0..cfg.n_routed_experts)
        .map(|j| build_expert(&format!("{p}.experts.{j}"), cfg, st, true, dev))
        .collect::<Result<Vec<_>>>()?;
    let shared = build_expert(&format!("{p}.shared_experts"), cfg, st, false, dev)?;
    let gate = Gate {
        weight: st.linear(&format!("{p}.gate"), cfg.n_routed_experts, cfg.dim, false, dev)?,
        bias: Some(st.auto_tensor(&format!("{p}.gate.bias"), &[cfg.n_routed_experts], dev)?),
        topk: cfg.n_activated_experts,
        route_scale: cfg.route_scale,
        score_func: score_func(&cfg.score_func)?,
        tid2eid: None,
    };
    Ok(Moe { gate, experts, shared })
}

fn build_hc(prefix: &str, cfg: &Config, st: &MultiSafeTensors, dev: &Device) -> Result<Hc> {
    Ok(Hc {
        hc_fn: st.auto_tensor(&format!("{prefix}_fn"), &[cfg.mix_hc(), cfg.hc_mult * cfg.dim], dev)?,
        hc_base: st.auto_tensor(&format!("{prefix}_base"), &[cfg.mix_hc()], dev)?,
        hc_scale: st.auto_tensor(&format!("{prefix}_scale"), &[3], dev)?,
        hc: cfg.hc_mult,
        sinkhorn_iters: cfg.hc_sinkhorn_iters,
        eps: cfg.hc_eps,
        norm_eps: cfg.norm_eps,
    })
}

fn build_block(cfg: &Config, st: &MultiSafeTensors, layer: usize, dev: &Device) -> Result<Block> {
    Ok(Block {
        attn: build_mla(cfg, st, layer, dev)?,
        ffn: build_moe(cfg, st, layer, dev)?,
        attn_norm: st.auto_tensor(&format!("layers.{layer}.attn_norm.weight"), &[cfg.dim], dev)?,
        ffn_norm: st.auto_tensor(&format!("layers.{layer}.ffn_norm.weight"), &[cfg.dim], dev)?,
        hc_attn: build_hc(&format!("layers.{layer}.hc_attn"), cfg, st, dev)?,
        hc_ffn: build_hc(&format!("layers.{layer}.hc_ffn"), cfg, st, dev)?,
        eps: cfg.norm_eps,
        is_hash_routed: false,
    })
}

fn build_head(cfg: &Config, st: &MultiSafeTensors, dev: &Device) -> Result<Head> {
    Ok(Head {
        weight: st.linear("head", cfg.vocab_size, cfg.dim, false, dev)?,
        norm: st.auto_tensor("norm.weight", &[cfg.dim], dev)?,
        hc_fn: st.auto_tensor("hc_head_fn", &[cfg.hc_mult, cfg.hc_mult * cfg.dim], dev)?,
        hc_base: st.auto_tensor("hc_head_base", &[cfg.hc_mult], dev)?,
        hc_scale: st.auto_tensor("hc_head_scale", &[1], dev)?,
        hc: cfg.hc_mult,
        eps: cfg.norm_eps,
        hc_eps: cfg.hc_eps,
    })
}
