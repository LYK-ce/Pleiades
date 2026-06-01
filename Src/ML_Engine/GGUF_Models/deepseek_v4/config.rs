//! Model hyper-parameters — adapted from MScanter/deepseek-v4-candle.
//!
//! Presented by KeJi
//! Date: 2026-06-01

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub vocab_size: usize,
    pub dim: usize,
    pub moe_inter_dim: usize,
    pub n_layers: usize,
    #[serde(default)]
    pub n_hash_layers: usize,
    #[serde(default = "default_mtp_layers")]
    pub n_mtp_layers: usize,
    pub n_heads: usize,

    // MoE
    pub n_routed_experts: usize,
    pub n_shared_experts: usize,
    pub n_activated_experts: usize,
    pub score_func: String,
    pub route_scale: f64,
    pub swiglu_limit: f64,

    // MLA
    pub q_lora_rank: usize,
    pub head_dim: usize,
    pub rope_head_dim: usize,
    pub o_groups: usize,
    pub o_lora_rank: usize,
    pub window_size: usize,
    #[serde(default = "default_max_seq_len")]
    pub max_seq_len: usize,

    // YaRN rope
    pub original_seq_len: usize,
    pub rope_theta: f64,
    pub rope_factor: f64,
    pub beta_fast: f64,
    pub beta_slow: f64,
    pub compress_rope_theta: f64,

    // CSA lightning indexer
    pub index_n_heads: usize,
    pub index_head_dim: usize,
    pub index_topk: usize,

    // Hyper-Connections (mHC)
    pub hc_mult: usize,
    pub hc_sinkhorn_iters: usize,
    #[serde(default = "default_eps")]
    pub hc_eps: f64,

    #[serde(default = "default_eps")]
    pub norm_eps: f64,

    pub compress_ratios: Vec<usize>,

    // Quantization (informational at the config level)
    #[serde(default)]
    pub dtype: Option<String>,
    #[serde(default)]
    pub scale_fmt: Option<String>,
    #[serde(default)]
    pub expert_dtype: Option<String>,
}

fn default_mtp_layers() -> usize { 1 }
fn default_max_seq_len() -> usize { 4096 }
fn default_eps() -> f64 { 1e-6 }

impl Config {
    pub fn mix_hc(&self) -> usize {
        (2 + self.hc_mult) * self.hc_mult
    }

    /// Build Config from GGUF metadata (written by Python wrap_native.py).
    pub fn from_gguf_metadata(
        metadata: &std::collections::HashMap<String, candle_core::quantized::gguf_file::Value>,
    ) -> anyhow::Result<Self> {
        let get_usize = |key: &str| -> anyhow::Result<usize> {
            metadata.get(key)
                .ok_or_else(|| anyhow::anyhow!("missing metadata: {key}"))
                .and_then(|v| v.to_u32().map(|u| u as usize)
                    .or_else(|_| v.to_u64().map(|u| u as usize))
                    .map_err(|_| anyhow::anyhow!("cannot convert {key} to usize")))
        };
        let get_f64 = |key: &str| -> anyhow::Result<f64> {
            metadata.get(key)
                .ok_or_else(|| anyhow::anyhow!("missing metadata: {key}"))
                .and_then(|v| v.to_f32().map(|f| f as f64)
                    .or_else(|_| v.to_f64())
                    .map_err(|_| anyhow::anyhow!("cannot convert {key} to f64")))
        };
        let get_string = |key: &str| -> Option<String> {
            metadata.get(key).and_then(|v| v.to_string().ok().map(|s| s.clone()))
        };
        let get_string_req = |key: &str| -> anyhow::Result<String> {
            get_string(key).ok_or_else(|| anyhow::anyhow!("missing metadata: {key}"))
        };

        let prefix = "deepseek_v4";
        let n_layers = get_usize(&format!("{prefix}.block_count"))?;
        let compress_ratios_str = get_string(&format!("{prefix}.compress_ratios"))
            .unwrap_or_default();
        let compress_ratios: Vec<usize> = if compress_ratios_str.is_empty() {
            vec![0; n_layers]
        } else {
            compress_ratios_str.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect()
        };

        Ok(Config {
            vocab_size: get_usize(&format!("{prefix}.vocab_size")).unwrap_or(0),
            dim: get_usize(&format!("{prefix}.embedding_length"))?,
            moe_inter_dim: get_usize(&format!("{prefix}.feed_forward_length")).unwrap_or(0),
            n_layers,
            n_hash_layers: get_usize(&format!("{prefix}.n_hash_layers")).unwrap_or(0),
            n_mtp_layers: 1,
            n_heads: get_usize(&format!("{prefix}.attention.head_count"))?,
            n_routed_experts: get_usize(&format!("{prefix}.n_routed_experts")).unwrap_or(0),
            n_shared_experts: get_usize(&format!("{prefix}.n_shared_experts")).unwrap_or(0),
            n_activated_experts: get_usize(&format!("{prefix}.n_activated_experts")).unwrap_or(0),
            score_func: get_string_req(&format!("{prefix}.score_func"))?,
            route_scale: get_f64(&format!("{prefix}.route_scale")).unwrap_or(1.0),
            swiglu_limit: get_f64(&format!("{prefix}.swiglu_limit")).unwrap_or(0.0),
            q_lora_rank: get_usize(&format!("{prefix}.q_lora_rank"))?,
            head_dim: get_usize(&format!("{prefix}.attention.key_length"))?,
            rope_head_dim: get_usize(&format!("{prefix}.rope_head_dim")).unwrap_or(0),
            o_groups: get_usize(&format!("{prefix}.o_groups")).unwrap_or(1),
            o_lora_rank: get_usize(&format!("{prefix}.o_lora_rank")).unwrap_or(0),
            window_size: get_usize(&format!("{prefix}.window_size")).unwrap_or(0),
            max_seq_len: get_usize(&format!("{prefix}.context_length")).unwrap_or(4096),
            original_seq_len: get_usize(&format!("{prefix}.original_seq_len")).unwrap_or(0),
            rope_theta: get_f64(&format!("{prefix}.rope.freq_base")).unwrap_or(10000.0),
            rope_factor: get_f64(&format!("{prefix}.rope_factor")).unwrap_or(1.0),
            beta_fast: get_f64(&format!("{prefix}.beta_fast")).unwrap_or(32.0),
            beta_slow: get_f64(&format!("{prefix}.beta_slow")).unwrap_or(1.0),
            compress_rope_theta: get_f64(&format!("{prefix}.compress_rope_theta"))
                .unwrap_or(10000.0),
            index_n_heads: get_usize(&format!("{prefix}.index_n_heads")).unwrap_or(0),
            index_head_dim: get_usize(&format!("{prefix}.index_head_dim")).unwrap_or(0),
            index_topk: get_usize(&format!("{prefix}.index_topk")).unwrap_or(0),
            hc_mult: get_usize(&format!("{prefix}.hc_mult")).unwrap_or(0),
            hc_sinkhorn_iters: get_usize(&format!("{prefix}.hc_sinkhorn_iters")).unwrap_or(0),
            hc_eps: get_f64(&format!("{prefix}.hc_eps")).unwrap_or(1e-6),
            norm_eps: get_f64(&format!("{prefix}.attention.layer_norm_rms_epsilon"))
                .unwrap_or(1e-6),
            compress_ratios,
            dtype: get_string(&format!("{prefix}.dtype")),
            scale_fmt: get_string(&format!("{prefix}.scale_fmt")),
            expert_dtype: get_string(&format!("{prefix}.expert_dtype")),
        })
    }
}
