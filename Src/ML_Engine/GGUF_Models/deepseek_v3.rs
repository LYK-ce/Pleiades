//Presented by KeJi
//Date : 2026-05-30

//! DeepSeek V3.2 模型架构支持
//!
//! 基于 candle + GGUF quantized 模式，参考 mistral.rs 的 MLA/MoE 实现。
//! 与 Qwen3 Dense 的核心差异：
//! - MLA (Multi-head Latent Attention): Q/KV 低秩压缩, 解耦 RoPE
//! - DeepSeekMoE: 共享 expert + 细粒度 routed experts + top-k 路由
//!
//! GGUF tensor 命名 (per layer blk.N):
//!   Q:   attn.q_a.weight → q_norm → attn.q_b.weight
//!   KV:  attn.kv_a_proj_with_mqa.weight → kv_norm → attn.kv_b.weight
//!   Out: attn.o.weight
//!   MoE: ffn_gate.weight / ffn_up.weight / ffn_down.weight (shared)
//!        ffn_gate.{e}.weight / ffn_up.{e}.weight / ffn_down.{e}.weight (routed)

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use candle_core::quantized::{gguf_file, QTensor};
use candle_core::{DType, Device, Tensor, D};
use candle_nn::{Activation, Embedding, Linear, Module};
use candle_transformers::fused_moe::FusedMoeGGUF;
use candle_transformers::models::with_tracing::QMatMul;
use candle_transformers::quantized_nn::RmsNorm;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::sync::Arc;

use super::qwen3::{Rotary_Embedding, Gguf, Mlp_Weights};

type Result<T> = candle_core::Result<T>;

// ============================================================
// MLA KV Cache — 缓存压缩后的 latent KV
// ============================================================

/// MLA 专用 KV Cache：缓存压缩后的 kv_latent 和解耦的 k_pe。
/// 
/// 与 candle 标准 ConcatKvCache 的核心差异：
/// - ConcatKvCache 缓存完整的 K/V (n_heads × head_dim)，显存占用大
/// - MLA_KV_Cache 缓存压缩后的 kv_latent (kv_lora_rank << n_heads × head_dim)
///   和解耦的 RoPE key k_pe (仅 qk_rope_dim)
///   → 大幅减少 KV Cache 显存占用
#[derive(Debug, Clone)]
pub struct MLA_KV_Cache {
    pub k: Option<Tensor>,
    pub v: Option<Tensor>,
}

impl MLA_KV_Cache {
    pub fn New(_capacity: usize) -> Self {
        Self { k: None, v: None }
    }

    pub fn Append(&mut self, k: &Tensor, v: &Tensor) -> Result<()> {
        self.k = match self.k.take() {
            None => Some(k.clone()),
            Some(old) => Some(Tensor::cat(&[&old, k], 2)?),
        };
        self.v = match self.v.take() {
            None => Some(v.clone()),
            Some(old) => Some(Tensor::cat(&[&old, v], 2)?),
        };
        Ok(())
    }

    pub fn Reset(&mut self) {
        self.k = None;
        self.v = None;
    }
}

// ============================================================
// MLA 注意力权重 — Multi-head Latent Attention
// ============================================================

#[derive(Debug, Clone)]
pub struct MLA_Weights {
    // Q 路径 (Low-Rank): x → q_a → q_norm → q_b → [nope | rope]
    pub q_a: QMatMul,           // [hidden, q_lora_rank]
    pub q_norm: RmsNorm,        // LayerNorm for Q
    pub q_b: QMatMul,           // [q_lora_rank, n_heads * q_head_dim]

    // KV 路径 (联合压缩): x → kv_a → split[kv_latent | k_pe] → kv_norm → kv_b → [k_nope | v]
    pub kv_a: QMatMul,          // [hidden, kv_lora_rank + qk_rope_dim]
    pub kv_norm: RmsNorm,       // LayerNorm for compressed KV
    pub k_b: Linear,           // [kv_lora_rank, n_heads * qk_nope_dim] (3D in GGUF)
    pub v_b: Linear,           // [kv_lora_rank, n_heads * v_head_dim] (3D in GGUF)

    // 输出投影
    pub o_proj: QMatMul,        // [n_heads * v_head_dim, hidden]

    // 配置
    pub n_heads: usize,
    pub q_lora_rank: usize,     // Q 压缩秩
    pub kv_lora_rank: usize,    // KV 压缩秩
    pub qk_rope_dim: usize,     // 解耦 RoPE 维度
    pub qk_nope_dim: usize,     // 非 RoPE 的 head 维度 (key_length from GGUF)
    pub v_head_dim: usize,      // V 的 head 维度
    pub q_head_dim: usize,      // qk_nope_dim + qk_rope_dim

    // RoPE (复用 qwen3 的 Rotary_Embedding)
    pub rotary: Arc<Rotary_Embedding>,

    // KV Cache
    pub kv_cache: MLA_KV_Cache,

    span_attn: tracing::Span,
}

impl MLA_Weights {
    pub fn New<R: Read + Seek>(
        gg: &mut Gguf<R>,
        n_heads: usize,
        q_lora_rank: usize,
        kv_lora_rank: usize,
        qk_rope_dim: usize,
        qk_nope_dim: usize,
        v_head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        prefix: &str,
    ) -> Result<Self> {
        let q_a = gg.Qmatmul(&format!("{prefix}.attn_q_a.weight"))?;
        let q_norm = gg.Rms_Norm(&format!("{prefix}.attn_q_a_norm.weight"), rms_norm_eps)?;
        let q_b = gg.Qmatmul(&format!("{prefix}.attn_q_b.weight"))?;

        let kv_a = gg.Qmatmul(&format!("{prefix}.attn_kv_a_mqa.weight"))?;
        let kv_norm = gg.Rms_Norm(&format!("{prefix}.attn_kv_a_norm.weight"), rms_norm_eps)?;
        // Unsloth: k_b/v_b 可能是 3D，dequantize + flatten + Linear
        let k_b_qt = gg.Tensor(&format!("{prefix}.attn_k_b.weight"))?;
        let k_b = {
            let deq = k_b_qt.dequantize(&gg.device)?;
            let dims = deq.dims();
            let w = if dims.len() == 3 { deq.reshape((dims[0] * dims[2], dims[1]))? } else { deq };
            Linear::new(w, None)
        };
        let v_b_qt = gg.Tensor(&format!("{prefix}.attn_v_b.weight"))?;
        let v_b = {
            let deq = v_b_qt.dequantize(&gg.device)?;
            let dims = deq.dims();
            let w = if dims.len() == 3 { deq.reshape((dims[0] * dims[2], dims[1]))? } else { deq };
            Linear::new(w, None)
        };

        let o_proj = gg.Qmatmul(&format!("{prefix}.attn_output.weight"))?;

        let kv_cache = MLA_KV_Cache::New(8192); // 默认 8K context
        let span_attn = tracing::span!(tracing::Level::TRACE, "mla");

        Ok(Self {
            q_a,
            q_norm,
            q_b,
            kv_a,
            kv_norm,
            k_b,
            v_b,
            o_proj,
            n_heads,
            q_lora_rank,
            kv_lora_rank,
            qk_rope_dim,
            qk_nope_dim,
            v_head_dim,
            q_head_dim: qk_nope_dim + qk_rope_dim,
            rotary,
            kv_cache,
            span_attn,
        })
    }

    /// 从已提取的 QTensors 构建（用于 From_Extracted 模式）
    pub fn From_Extracted(
        tensors: &mut HashMap<String, QTensor>,
        n_heads: usize,
        q_lora_rank: usize,
        kv_lora_rank: usize,
        qk_rope_dim: usize,
        qk_nope_dim: usize,
        v_head_dim: usize,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        device: &Device,
        prefix: &str,
    ) -> Result<Self> {
        fn Take_Qmatmul(
            tensors: &mut HashMap<String, QTensor>,
            key: &str,
        ) -> Result<QMatMul> {
            let qt = tensors
                .remove(key)
                .ok_or_else(|| {
                    let mut available: Vec<&String> = tensors.keys().collect();
                    available.sort();
                    candle_core::Error::Msg(format!(
                        "missing tensor: {}. Available tensors for this layer: {:?}",
                        key, available
                    ))
                })?;
            QMatMul::from_weights(Arc::new(qt))
        }

        fn Take_Rmsnorm(
            tensors: &mut HashMap<String, QTensor>,
            key: &str,
            eps: f64,
        ) -> Result<RmsNorm> {
            let qt = tensors
                .remove(key)
                .ok_or_else(|| {
                    let mut available: Vec<&String> = tensors.keys().collect();
                    available.sort();
                    candle_core::Error::Msg(format!(
                        "missing tensor: {}. Available tensors for this layer: {:?}",
                        key, available
                    ))
                })?;
            RmsNorm::from_qtensor(qt, eps)
        }

        /// Flatten 3D GGUF weight [d0, d1, d2] → 2D [d1, d0*d2]，返回 Linear
        fn load_linear_flatten(tensors: &mut HashMap<String, QTensor>, key: &str, dev: &Device) -> Result<Linear> {
            let qt = tensors.remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {key}")))?;
            let deq = qt.dequantize(dev)?;
            let dims = deq.dims();
            let weight = if dims.len() == 3 {
                deq.reshape((dims[0] * dims[2], dims[1]))?  // [n*d2, d1] = [out, in]
            } else {
                deq
            };
            Ok(Linear::new(weight, None))
        }

        let q_a = Take_Qmatmul(tensors, &format!("{prefix}.attn_q_a.weight"))?;
        let q_norm = Take_Rmsnorm(tensors, &format!("{prefix}.attn_q_a_norm.weight"), rms_norm_eps)?;
        let q_b = Take_Qmatmul(tensors, &format!("{prefix}.attn_q_b.weight"))?;

        let kv_a = Take_Qmatmul(tensors, &format!("{prefix}.attn_kv_a_mqa.weight"))?;
        let kv_norm = Take_Rmsnorm(tensors, &format!("{prefix}.attn_kv_a_norm.weight"), rms_norm_eps)?;
        // Unsloth: k_b/v_b 都是 3D，需 flatten（llama.cpp 确认）
        let k_b = load_linear_flatten(tensors, &format!("{prefix}.attn_k_b.weight"), device)?;
        let v_b = load_linear_flatten(tensors, &format!("{prefix}.attn_v_b.weight"), device)?;

        let o_proj = Take_Qmatmul(tensors, &format!("{prefix}.attn_output.weight"))?;

        let kv_cache = MLA_KV_Cache::New(8192);
        let span_attn = tracing::span!(tracing::Level::TRACE, "mla");

        Ok(Self {
            q_a, q_norm, q_b,
            kv_a, kv_norm, k_b, v_b,
            o_proj,
            n_heads,
            q_lora_rank,
            kv_lora_rank,
            qk_rope_dim,
            qk_nope_dim,
            v_head_dim,
            q_head_dim: qk_nope_dim + qk_rope_dim,
            rotary,
            kv_cache,
            span_attn,
        })
    }

    /// MLA 前向传播
    ///
    /// 与 Qwen3 的 Attention_Weights::Forward 对应，但计算流程完全不同：
    ///
    /// 1. Q: x → q_a → q_norm → q_b → split(q_nope, q_pe)
    /// 2. KV: x → kv_a → split(kv_latent, k_pe)
    /// 3. kv_latent → kv_norm → kv_b → split(k_nope, v)
    /// 4. RoPE: 仅对 q_pe, k_pe 做旋转
    /// 5. 完整 K = concat(k_nope, k_pe_broadcast)
    /// 6. 完整 Q = concat(q_nope, q_pe)
    /// 7. Standard scaled dot-product attention
    /// 8. Output projection: o_proj
    pub fn Forward(
        &mut self,
        x: &Tensor,
        attn_mask: Option<&Tensor>,
        offset: usize,
    ) -> Result<Tensor> {
        let _enter = self.span_attn.enter();
        let (b, l, _) = x.dims3()?;
        tracing::info!("MLA forward: b={} l={} n_heads={} qk_nope={} qk_rope={} v_head={} q_head={}",
            b, l, self.n_heads, self.qk_nope_dim, self.qk_rope_dim, self.v_head_dim, self.q_head_dim);

        // ── Q 路径 ──
        let q = self.q_a.forward(x)?;             // [b, l, q_lora_rank]
        let q = self.q_norm.forward(&q)?;          // LayerNorm
        let q = self.q_b.forward(&q)?;             // [b, l, n_heads * q_head_dim]
        tracing::info!("MLA q_b output: {:?}", q.dims());
        let q = q
            .reshape((b, l, self.n_heads, self.q_head_dim))?
            .transpose(1, 2)?;                     // [b, n_heads, l, q_head_dim]

        // 分割 Q: 前 qk_nope_dim → q_nope, 后 qk_rope_dim → q_pe
        let q_nope = q.narrow(3, 0, self.qk_nope_dim)?.contiguous()?;
        let q_pe = q.narrow(3, self.qk_nope_dim, self.qk_rope_dim)?.contiguous()?;

        // ── KV 路径 ──
        let compressed_kv = self.kv_a.forward(x)?;  // [b, l, kv_lora_rank + qk_rope_dim]
        tracing::info!("MLA kv_a output: {:?}", compressed_kv.dims());
        let kv_latent = compressed_kv.narrow(2, 0, self.kv_lora_rank)?.contiguous()?;
        let k_pe_raw = compressed_kv.narrow(2, self.kv_lora_rank, self.qk_rope_dim)?.contiguous()?;

        // k_pe: [b, 1, l, qk_rope_dim]
        let k_pe = k_pe_raw
            .reshape((b, l, 1, self.qk_rope_dim))?
            .transpose(1, 2)?;

        // kv_latent → kv_norm → k_b / v_b
        let ckv = self.kv_norm.forward(&kv_latent)?;
        let k_nope = self.k_b.forward(&ckv)?;        // [b, l, n_heads * qk_nope_dim]
        tracing::info!("MLA k_b: dims={:?} qk_nope={}", k_nope.dims(), self.qk_nope_dim);
        let k_nope = k_nope
            .reshape((b, l, self.n_heads, self.qk_nope_dim))?
            .transpose(1, 2)?
            .contiguous()?;                          // [b, n_heads, l, qk_nope_dim]
        let v = self.v_b.forward(&ckv)?;             // [b, l, n_heads * v_head_dim]
        tracing::info!("MLA v_b: dims={:?} v_head={}", v.dims(), self.v_head_dim);
        let v = v
            .reshape((b, l, self.n_heads, self.v_head_dim))?
            .transpose(1, 2)?
            .contiguous()?;                          // [b, n_heads, l, v_head_dim]

        // ── 解耦 RoPE ──
        // 仅对 q_pe 和 k_pe 做旋转编码
        let (q_pe_roped, k_pe_roped) = self.rotary.Apply(&q_pe, &k_pe, offset)?;

        // ── 组装完整 K, Q ──
        // k_pe 在每个 head 上重复
        let k_pe_broadcast = k_pe_roped.repeat((1, self.n_heads, 1, 1))?;
        let k = Tensor::cat(&[&k_nope, &k_pe_broadcast], 3)?;
        let q = Tensor::cat(&[&q_nope, &q_pe_roped], 3)?;

        // ── 更新 KV Cache（展开的 K, V）──
        let (k, v) = match (&self.kv_cache.k, &self.kv_cache.v) {
            (Some(prev_k), Some(prev_v)) => {
                let k = Tensor::cat(&[prev_k, &k], 2)?;
                let v = Tensor::cat(&[prev_v, &v], 2)?;
                (k, v)
            }
            _ => (k, v),
        };
        self.kv_cache.k = Some(k.clone());
        self.kv_cache.v = Some(v.clone());

        // ── Scaled Dot-Product Attention ──
        let scale = 1.0 / (self.q_head_dim as f64).sqrt();
        let mut scores = (q.matmul(&k.transpose(2, 3)?)? * scale)?;

        if let Some(m) = attn_mask {
            let m_dtype = m.dtype();
            let scores_dtype = scores.dtype();
            let mask = if m_dtype != scores_dtype {
                m.to_dtype(scores_dtype)?
            } else {
                m.clone()
            };
            scores = scores.broadcast_add(&mask)?;
        }

        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let ctx = probs.matmul(&v)?; // [b, n_heads, l, v_head_dim]

        // ── 输出投影 ──
        let reshaped = ctx
            .transpose(1, 2)?
            .reshape((b, l, self.n_heads * self.v_head_dim))?;
        self.o_proj.forward(&reshaped)
    }

    pub fn Clear_Kv_Cache(&mut self) {
        self.kv_cache.Reset();
    }
}

// ============================================================
// DeepSeekMoE 权重 — Mixture of Experts
// ============================================================

#[derive(Clone)]
pub struct DeepSeekMoE_Weights {
    pub shared_gate: QMatMul,
    pub shared_up: QMatMul,
    pub shared_down: QMatMul,
    pub routed: Arc<FusedMoeGGUF>,
    pub routed_scaling_factor: f64,
    span: tracing::Span,
}

impl std::fmt::Debug for DeepSeekMoE_Weights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepSeekMoE_Weights")
            .field("routed_scaling_factor", &self.routed_scaling_factor)
            .finish()
    }
}

impl DeepSeekMoE_Weights {
    pub fn New<R: Read + Seek>(
        gg: &mut Gguf<R>,
        n_routed_experts: usize,
        top_k: usize,
        routed_scaling_factor: f64,
        dtype: DType,
        prefix: &str,
    ) -> Result<Self> {
        let shared_gate = gg.Qmatmul(&format!("{prefix}.ffn_gate_shexp.weight"))?;
        let shared_up = gg.Qmatmul(&format!("{prefix}.ffn_up_shexp.weight"))?;
        let shared_down = gg.Qmatmul(&format!("{prefix}.ffn_down_shexp.weight"))?;

        let gate_experts = Arc::new(gg.Tensor(&format!("{prefix}.ffn_gate_exps.weight"))?);
        let up_experts = Arc::new(gg.Tensor(&format!("{prefix}.ffn_up_exps.weight"))?);
        let down_experts = Arc::new(gg.Tensor(&format!("{prefix}.ffn_down_exps.weight"))?);
        let gate_qt = gg.Tensor(&format!("{prefix}.ffn_gate_inp.weight"))?;
        let gate = Linear::new(gate_qt.dequantize(&gg.device)?.to_dtype(DType::F32)?, None);

        let routed = Arc::new(FusedMoeGGUF {
            gate,
            gate_experts,
            up_experts,
            down_experts,
            act: Activation::Silu,
            norm_topk_prob: false,
            num_experts_per_tok: top_k,
            dtype,
        });

        Ok(Self {
            shared_gate, shared_up, shared_down,
            routed,
            routed_scaling_factor,
            span: tracing::span!(tracing::Level::TRACE, "moe"),
        })
    }

    pub fn From_Extracted(
        tensors: &mut HashMap<String, QTensor>,
        n_routed_experts: usize,
        top_k: usize,
        routed_scaling_factor: f64,
        dtype: DType,
        device: &Device,
        prefix: &str,
    ) -> Result<Self> {
        let shared_gate = QMatMul::from_weights(
            tensors.remove(&format!("{prefix}.ffn_gate_shexp.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_gate_shexp.weight")))?.into()
        )?;
        let shared_up = QMatMul::from_weights(
            tensors.remove(&format!("{prefix}.ffn_up_shexp.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_up_shexp.weight")))?.into()
        )?;
        let shared_down = QMatMul::from_weights(
            tensors.remove(&format!("{prefix}.ffn_down_shexp.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_down_shexp.weight")))?.into()
        )?;

        let gate_qt = tensors.remove(&format!("{prefix}.ffn_gate_inp.weight"))
            .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_gate_inp.weight")))?;
        let gate = Linear::new(gate_qt.dequantize(device)?.to_dtype(DType::F32)?, None);

        let gate_experts = Arc::new(
            tensors.remove(&format!("{prefix}.ffn_gate_exps.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_gate_exps.weight")))?
        );
        let up_experts = Arc::new(
            tensors.remove(&format!("{prefix}.ffn_up_exps.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_up_exps.weight")))?
        );
        let down_experts = Arc::new(
            tensors.remove(&format!("{prefix}.ffn_down_exps.weight"))
                .ok_or_else(|| candle_core::Error::Msg(format!("missing: {prefix}.ffn_down_exps.weight")))?
        );

        let routed = Arc::new(FusedMoeGGUF {
            gate,
            gate_experts,
            up_experts,
            down_experts,
            act: Activation::Silu,
            norm_topk_prob: false,
            num_experts_per_tok: top_k,
            dtype,
        });

        Ok(Self {
            shared_gate, shared_up, shared_down,
            routed,
            routed_scaling_factor,
            span: tracing::span!(tracing::Level::TRACE, "moe"),
        })
    }

    /// MoE Forward
    ///
    /// 对齐 mistral.rs 的 DeepSeek MoE 路由：
    /// 1. router_logits → log_softmax (对全部 expert)
    /// 2. arg_sort → top_k indices (GPU 加速)
    /// 3. gather → top_k log_softmax 值
    /// 4. softmax → 仅在 top_k 内归一化 (和为 1)
    /// 5. 共享 expert 始终激活
    /// 6. 最终输出 = shared_out + sum(routed expert outputs × weight) × scaling_factor
    pub fn Forward(&self, x: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let gate = self.shared_gate.forward(x)?.apply(&Activation::Silu)?;
        let up = self.shared_up.forward(x)?;
        let shared_out = self.shared_down.forward(&(gate * up)?)?;
        let routed_out = self.routed.forward(x, false)?;
        &shared_out + &(routed_out * self.routed_scaling_factor)?
    }
}

#[derive(Clone)]
pub enum DeepSeekFFN {
    Dense(Mlp_Weights),
    MoE(DeepSeekMoE_Weights),
}

impl std::fmt::Debug for DeepSeekFFN {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dense(_) => write!(f, "Dense(Mlp)"),
            Self::MoE(_) => write!(f, "MoE(..)"),
        }
    }
}

impl DeepSeekFFN {
    pub fn Forward(&self, x: &Tensor) -> Result<Tensor> {
        match self {
            Self::Dense(m) => m.forward(x),
            Self::MoE(m) => m.Forward(x),
        }
    }
}

// ============================================================
// DeepSeek_Layer — 单个 Transformer 层
// ============================================================

#[derive(Debug, Clone)]
pub struct DeepSeek_Layer {
    pub mla: MLA_Weights,
    pub ffn: DeepSeekFFN,
    pub ln1: RmsNorm,     // pre-attention norm
    pub ln2: RmsNorm,     // pre-MoE norm
}

impl DeepSeek_Layer {
    /// 从 Gguf reader 加载
    pub fn New<R: Read + Seek>(
        gg: &mut Gguf<R>,
        n_heads: usize,
        q_lora_rank: usize,
        kv_lora_rank: usize,
        qk_rope_dim: usize,
        qk_nope_dim: usize,
        v_head_dim: usize,
        n_routed_experts: usize,
        top_k: usize,
        routed_scaling_factor: f64,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        layer_idx: usize,
    ) -> Result<Self> {
        let prefix = format!("blk.{layer_idx}");
        let ln1 = gg.Rms_Norm(&format!("{prefix}.attn_norm.weight"), rms_norm_eps)?;
        let ln2 = gg.Rms_Norm(&format!("{prefix}.ffn_norm.weight"), rms_norm_eps)?;
        let mla = MLA_Weights::New(
            gg, n_heads, q_lora_rank, kv_lora_rank,
            qk_rope_dim, qk_nope_dim, v_head_dim,
            rms_norm_eps, rotary, &prefix,
        )?;
        let moe = DeepSeekMoE_Weights::New(
            gg, n_routed_experts, top_k, routed_scaling_factor, DType::F16, &prefix,
        )?;
        Ok(Self { mla, ffn: DeepSeekFFN::MoE(moe), ln1, ln2 })
    }

    /// 从已提取的 QTensors 构建
    pub fn From_Extracted(
        tensors: &mut HashMap<String, QTensor>,
        n_heads: usize,
        q_lora_rank: usize,
        kv_lora_rank: usize,
        qk_rope_dim: usize,
        qk_nope_dim: usize,
        v_head_dim: usize,
        n_routed_experts: usize,
        top_k: usize,
        routed_scaling_factor: f64,
        rms_norm_eps: f64,
        rotary: Arc<Rotary_Embedding>,
        layer_idx: usize,
        device: &Device,
        dtype: DType,
    ) -> Result<Self> {
        let prefix = format!("blk.{layer_idx}");

        fn Take_Rmsnorm(tensors: &mut HashMap<String, QTensor>, key: &str, eps: f64) -> Result<RmsNorm> {
            let qt = tensors
                .remove(key)
                .ok_or_else(|| candle_core::Error::Msg(format!("missing tensor: {}", key)))?;
            RmsNorm::from_qtensor(qt, eps)
        }

        let ln1 = Take_Rmsnorm(tensors, &format!("{prefix}.attn_norm.weight"), rms_norm_eps)?;
        let ln2 = Take_Rmsnorm(tensors, &format!("{prefix}.ffn_norm.weight"), rms_norm_eps)?;
        let mla = MLA_Weights::From_Extracted(
            tensors, n_heads, q_lora_rank, kv_lora_rank,
            qk_rope_dim, qk_nope_dim, v_head_dim,
            rms_norm_eps, rotary, device, &prefix,
        )?;
        // 自动检测：有 router(ffn_gate_inp) → MoE，否则 → Dense FFN
        let has_experts = tensors.contains_key(&format!("{prefix}.ffn_gate_inp.weight"));
        let ffn = if has_experts {
            // Log available MoE tensors for debugging
            let mut keys: Vec<&String> = tensors.keys().collect();
            keys.sort();
            tracing::info!("MoE layer {}: tensors={:?}", layer_idx, keys);
            DeepSeekFFN::MoE(DeepSeekMoE_Weights::From_Extracted(
                tensors, n_routed_experts, top_k, routed_scaling_factor, dtype, device, &prefix,
            )?)
        } else {
            DeepSeekFFN::Dense(Mlp_Weights::New_Dense(tensors, &prefix)?)
        };
        Ok(Self { mla, ffn, ln1, ln2 })
    }

    /// 单层 Forward
    pub fn Forward(
        &mut self,
        x: &Tensor,
        mask: Option<&Tensor>,
        offset: usize,
    ) -> Result<Tensor> {
        // Pre-attention norm → MLA → residual
        let h = self.ln1.forward(x)?;
        let h = self.mla.Forward(&h, mask, offset)?;
        let x = (x + h)?;
        // Pre-MoE norm → MoE → residual
        let h2 = self.ln2.forward(&x)?;
        let h2 = self.ffn.Forward(&h2)?;
        x + h2
    }

    pub fn Clear_Kv_Cache(&mut self) {
        self.mla.Clear_Kv_Cache();
    }
}

// ============================================================
// DeepSeek_Model — 完整模型
// ============================================================

#[derive(Debug, Clone)]
pub struct DeepSeek_Model {
    pub embed_tokens: Option<Embedding>,
    pub layers: Vec<DeepSeek_Layer>,
    pub norm: Option<RmsNorm>,
    pub lm_head: Option<QMatMul>,
    pub device: Device,
    pub dtype: DType,
    span: tracing::Span,
    span_output: tracing::Span,
}

impl DeepSeek_Model {
    /// 从 Gguf reader 加载完整模型
    pub fn From_Gguf<R: Read + Seek>(
        ct: gguf_file::Content,
        reader: &mut R,
        device: &Device,
    ) -> Result<Self> {
        let mut gg = Gguf::New(ct, reader, device.clone());

        // 从 metadata 提取架构参数
        let architecture = match gg.Metadata().get("general.architecture") {
            Some(v) => v.to_string().map(|s| s.clone()).unwrap_or_else(|_| "deepseek_v3".to_string()),
            None => "deepseek_v3".to_string(),
        };
        let config = DeepSeek_Config::From_Metadata(gg.Metadata(), &architecture)?;
        let dtype = match gg.Metadata().get("general.dtype") {
            Some(v) => match v.to_u32() {
                Ok(0) => DType::F32,
                Ok(1) => DType::F16,
                _ => DType::F16,
            },
            None => DType::F16,
        };

        // Embedding
        let embed_tensor = gg.Tensor("token_embd.weight")?;
        let embed_tokens = Embedding::new(embed_tensor.dequantize(device)?, config.hidden_size);

        // RoPE: DeepSeek 的 RoPE 应用于 head_dim = qk_rope_dim (解耦部分)
        let rotary = Arc::new(Rotary_Embedding::New(
            dtype,
            config.qk_rope_dim,
            config.max_position_embeddings,
            config.rope_freq_base,
            device,
        )?);

        // 逐层加载
        let mut layers = Vec::with_capacity(config.num_layers);
        for i in 0..config.num_layers {
            layers.push(DeepSeek_Layer::New(
                &mut gg,
                config.n_heads,
                config.q_lora_rank,
                config.kv_lora_rank,
                config.qk_rope_dim,
                config.qk_nope_dim,
                config.v_head_dim,
                config.n_routed_experts,
                config.top_k,
                config.routed_scaling_factor,
                config.rms_norm_eps,
                rotary.clone(),
                i,
            )?);
        }

        let norm = gg.Rms_Norm("output_norm.weight", config.rms_norm_eps)?;
        let lm_head_tensor = match gg.Tensor("output.weight") {
            Ok(t) => t,
            Err(_) => gg.Tensor("token_embd.weight")?,
        };
        let lm_head = QMatMul::from_weights(lm_head_tensor.into())?;

        let span = tracing::span!(tracing::Level::TRACE, "model");
        let span_output = tracing::span!(tracing::Level::TRACE, "output");

        Ok(Self {
            embed_tokens: Some(embed_tokens),
            layers,
            norm: Some(norm),
            lm_head: Some(lm_head),
            device: device.clone(),
            dtype,
            span,
            span_output,
        })
    }

    /// 动态组装 (用于 partial loading)
    pub fn From_Dynamic(
        embed_tokens: Option<Embedding>,
        layers: Vec<DeepSeek_Layer>,
        norm: Option<RmsNorm>,
        lm_head: Option<QMatMul>,
        device: Device,
        dtype: DType,
    ) -> Self {
        let span = tracing::span!(tracing::Level::TRACE, "model");
        let span_output = tracing::span!(tracing::Level::TRACE, "output");
        Self {
            embed_tokens, layers, norm, lm_head,
            device, dtype, span, span_output,
        }
    }

    fn Causal_Mask(
        &self,
        b: usize,
        tgt: usize,
        offset: usize,
    ) -> Result<Tensor> {
        let minf = f32::NEG_INFINITY;
        let mask: Vec<_> = (0..tgt)
            .flat_map(|i| {
                (0..(tgt + offset)).map(move |j| {
                    if j <= i + offset { 0. } else { minf }
                })
            })
            .collect();
        Tensor::from_slice(&mask, (b, 1, tgt, tgt + offset), &self.device)?
            .to_dtype(self.dtype)
    }

    /// Forward pass
    pub fn Forward(&mut self, input: &Tensor, offset: usize) -> Result<Tensor> {
        let _enter = self.span.enter();

        let mut h = if let Some(ref embed) = self.embed_tokens {
            embed.forward(input)?
        } else {
            input.clone()
        };

        let b = h.dim(0)?;
        let l = h.dim(1)?;
        let causal_mask = if l == 1 {
            None
        } else {
            Some(self.Causal_Mask(b, l, offset)?)
        };

        for layer in &mut self.layers {
            h = layer.Forward(&h, causal_mask.as_ref(), offset)?;
        }

        if let (Some(ref norm), Some(ref lm_head)) = (&self.norm, &self.lm_head) {
            let h = norm.forward(&h)?;
            let _enter = self.span_output.enter();
            let last_hidden = h.narrow(1, l - 1, 1)?;
            lm_head.forward(&last_hidden)?.squeeze(1)
        } else {
            Ok(h)
        }
    }

    pub fn Clear_Kv_Cache(&mut self) {
        for layer in &mut self.layers {
            layer.Clear_Kv_Cache();
        }
    }
}

// ============================================================
// DeepSeek_Config — 从 GGUF metadata 提取配置
// ============================================================

pub struct DeepSeek_Config {
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub q_lora_rank: usize,
    pub kv_lora_rank: usize,
    pub qk_rope_dim: usize,
    pub qk_nope_dim: usize,     // key_length from GGUF (non-RoPE part)
    pub v_head_dim: usize,
    pub num_layers: usize,
    pub hidden_size: usize,
    pub max_position_embeddings: usize,
    pub rms_norm_eps: f64,
    pub rope_freq_base: f64,
    pub n_routed_experts: usize,
    pub n_shared_experts: usize,
    pub top_k: usize,
    pub routed_scaling_factor: f64,
    pub vocab_size: usize,
}

impl DeepSeek_Config {
    pub fn From_Metadata(
        metadata: &std::collections::HashMap<String, gguf_file::Value>,
        arch: &str,
    ) -> Result<Self> {
        let md_get = |s: &str| match metadata.get(s) {
            None => candle_core::bail!("cannot find {s} in metadata"),
            Some(v) => Ok(v),
        };

        let md_get_usize = |s: &str| {
            md_get(s).and_then(|v| {
                v.to_u32()
                    .map(|u| u as usize)
                    .or_else(|_| v.to_u64().map(|u| u as usize))
                    .map_err(|_| candle_core::Error::Msg(format!("cannot convert {s} to usize")))
            })
        };
        let md_get_usize_opt = |s: &str| -> Option<usize> {
            metadata.get(s).and_then(|v| {
                v.to_u32().map(|u| u as usize)
                    .or_else(|_| v.to_u64().map(|u| u as usize)).ok()
            })
        };

        let md_get_f64 = |s: &str| {
            md_get(s).and_then(|v| {
                v.to_f32()
                    .map(|f| f as f64)
                    .or_else(|_| v.to_f64())
                    .map_err(|_| candle_core::Error::Msg(format!("cannot convert {s} to f64")))
            })
        };
        let md_get_f64_opt = |s: &str| -> Option<f64> {
            metadata.get(s).and_then(|v| {
                v.to_f32().map(|f| f as f64).or_else(|_| v.to_f64()).ok()
            })
        };

        let prefix = |key: &str| format!("{arch}.{key}");

        let n_heads = md_get_usize(&prefix("attention.head_count"))?;
        let n_kv_heads = md_get_usize(&prefix("attention.head_count_kv")).unwrap_or(n_heads);
        let q_lora_rank = md_get_usize(&prefix("attention.q_lora_rank")).unwrap_or(1536);
        let kv_lora_rank = md_get_usize(&prefix("attention.kv_lora_rank")).unwrap_or(512);
        // Unsloth: key_length_mla=q_head_dim, rope.dimension_count=qk_rope_dim, value_length_mla=v_head_dim
        let q_head_dim = md_get_usize_opt(&prefix("attention.key_length_mla")).unwrap_or(192);
        let qk_rope_dim = md_get_usize_opt(&prefix("rope.dimension_count")).unwrap_or(64);
        let qk_nope_dim = q_head_dim - qk_rope_dim;
        let v_head_dim = md_get_usize_opt(&prefix("attention.value_length_mla")).unwrap_or(128);
        let num_layers = md_get_usize(&prefix("block_count"))?;
        let hidden_size = md_get_usize(&prefix("embedding_length"))?;
        let max_position_embeddings = md_get_usize(&prefix("context_length")).unwrap_or(131072);
        let rms_norm_eps = md_get_f64(&prefix("attention.layer_norm_rms_epsilon")).unwrap_or(1e-6);
        let rope_freq_base = md_get_f64(&prefix("rope.freq_base")).unwrap_or(10000.0);
        let n_routed_experts = md_get_usize_opt(&prefix("expert_count")).unwrap_or(256);
        let n_shared_experts = md_get_usize_opt(&prefix("expert_shared_count")).unwrap_or(1);
        let top_k = md_get_usize_opt(&prefix("expert_used_count")).unwrap_or(8);
        let routed_scaling_factor = md_get_f64_opt(&prefix("expert_weights_scale")).unwrap_or(2.5);
        let vocab_size = md_get_usize(&prefix("vocab_size")).unwrap_or(129280);

        Ok(Self {
            n_heads, n_kv_heads,
            q_lora_rank, kv_lora_rank,
            qk_rope_dim, qk_nope_dim, v_head_dim,
            num_layers, hidden_size,
            max_position_embeddings,
            rms_norm_eps, rope_freq_base,
            n_routed_experts, n_shared_experts,
            top_k, routed_scaling_factor,
            vocab_size,
        })
    }
}
