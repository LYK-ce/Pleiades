//Presented by KeJi
//Date ： 2026-05-16

//! ML Engine 能力层
//!
//! 通过 `MlSession` userdata 暴露给 Lua，类似 Python class：
//!
//! ```lua
//! local sess = ml.load_model("model.gguf", "cpu", 0, 999999)
//! local tokens = sess:encode(prompt)
//! sess:forward(sess:tensorize(tokens), 0)
//! local tok = sess:sample(0.8)
//! sess:unload()
//! ```

#![allow(non_snake_case)]

use std::path::Path;

use candle_core::{Device, Tensor};

use super::gguf_model::{
    GGUF_Decode, GGUF_Encode, GGUF_Load_Model, GGUF_Model,
    GGUF_Model_Inference, GGUF_Unload_Model,
};

// ============================================================
// MlSession — 对外句柄 (Lua userdata)
// ============================================================

/// 推理会话。
///
/// 包装内部 `MlContext`，由 Lua 侧实例化并调用方法。
/// ML Engine 不定义 trait — 直接暴露具体类型给 Lua。
pub struct MlSession {
    ctx: MlContext,
}

// ============================================================
// MlContext — 内部状态（外部不可见）
// ============================================================

struct MlContext {
    /// 模型权重 + tokenizer
    model: GGUF_Model,
    /// 推理输出缓冲区（logits 或 hidden state）
    output: Option<Tensor>,
    /// 自增序列位置（forward offset=None 时自动 += seq_len）
    offset: usize,
    /// 采样随机数生成器状态 (xoshiro)
    rng_state: u64,
    /// EOS token ID
    eos_token_id: u32,
}

// ============================================================
// MlSession — Rust 侧公开方法
// ============================================================

impl MlSession {
    // ─── 构造 / 析构 ───────────────────────────────────────

    /// 加载模型，创建 MlSession。
    pub fn load_model(
        path: &Path,
        device: &str,
        start: usize,
        end: usize,
    ) -> Result<Self, String> {
        let device = match device.to_lowercase().as_str() {
            "cpu" => Device::Cpu,
            "cuda" => match Device::new_cuda(0) {
                Ok(d) => d,
                Err(e) => return Err(format!("CUDA device init failed: {e}")),
            },
            other => return Err(format!("Unsupported device: '{other}'. Use 'cpu' or 'cuda'.")),
        };

        let model = GGUF_Load_Model(start, end, path, &device)
            .map_err(|e| format!("Failed to load model: {e}"))?;

        let eos_token_id = model.inference_config.eos_token;

        Ok(Self {
            ctx: MlContext {
                model,
                output: None,
                offset: 0,
                rng_state: 299792458,
                eos_token_id,
            },
        })
    }

    /// 卸载模型。
    pub fn unload(self) {
        GGUF_Unload_Model(self.ctx.model);
    }

    // ─── 编解码 ────────────────────────────────────────────

    pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> {
        GGUF_Encode(&self.ctx.model, text)
            .map_err(|e| format!("Encode failed: {e}"))
    }

    pub fn decode(&self, token_id: u32) -> Result<String, String> {
        GGUF_Decode(&self.ctx.model, &[token_id])
            .map_err(|e| format!("Decode failed: {e}"))
    }

    // ─── 推理 ──────────────────────────────────────────────

    pub fn tensorize(&self, token_ids: &[u32]) -> Result<Tensor, String> {
        if token_ids.is_empty() {
            return Err("tensorize: token_ids is empty".into());
        }
        Tensor::new(token_ids, &self.ctx.model.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| format!("Tensorize failed: {e}"))
    }

    pub fn forward(&mut self, tensor: &Tensor, offset: Option<usize>) -> Result<(), String> {
        let off = offset.unwrap_or(self.ctx.offset);
        let seq_len = tensor.dims().get(1).copied().unwrap_or(1);

        let output = GGUF_Model_Inference(&mut self.ctx.model, tensor, off)
            .map_err(|e| format!("Forward failed: {e}"))?;

        self.ctx.model.device
            .synchronize()
            .map_err(|e| format!("Device sync failed: {e}"))?;

        self.ctx.output = Some(output);

        if offset.is_none() {
            self.ctx.offset += seq_len;
        }

        Ok(())
    }

    // ─── 采样 ──────────────────────────────────────────────

    pub fn sample(&mut self, temperature: f64) -> Result<u32, String> {
        let logits = self.ctx.output.as_ref()
            .ok_or("No output tensor. Call forward() first.")?;

        let last_logits = Self::extract_last_logits(logits)?;

        let token = if temperature <= 0.0 {
            last_logits
                .argmax(0)
                .map_err(|e| format!("argmax failed: {e}"))?
                .to_scalar::<u32>()
                .map_err(|e| format!("to_scalar failed: {e}"))?
        } else {
            let scaled = (&last_logits / temperature)
                .map_err(|e| format!("temperature scaling failed: {e}"))?;
            let probs = candle_nn::ops::softmax(&scaled, 0)
                .map_err(|e| format!("softmax failed: {e}"))?;
            let probs_vec: Vec<f32> = probs
                .to_vec1::<f32>()
                .map_err(|e| format!("probs → vec failed: {e}"))?;

            self.ctx.rng_state = self.ctx.rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let random = (self.ctx.rng_state >> 33) as f32 / (u32::MAX as f32);

            let mut cumulative = 0.0f32;
            let mut chosen = (probs_vec.len() - 1) as u32;
            for (i, &p) in probs_vec.iter().enumerate() {
                cumulative += p;
                if cumulative > random {
                    chosen = i as u32;
                    break;
                }
            }
            chosen
        };

        Ok(token)
    }

    // ─── 状态查询 ──────────────────────────────────────────

    pub fn get_output_tensor(&self) -> Option<&Tensor> {
        self.ctx.output.as_ref()
    }

    pub fn get_eos(&self) -> u32 {
        self.ctx.eos_token_id
    }

    pub fn get_offset(&self) -> usize {
        self.ctx.offset
    }

    // ─── 辅助 ──────────────────────────────────────────────

    fn extract_last_logits(logits: &Tensor) -> Result<Tensor, String> {
        let dims = logits.dims();
        match dims.len() {
            3 => {
                let seq_len = dims[1];
                let last = logits
                    .get(0)
                    .map_err(|e| format!("get batch 0 failed: {e}"))?
                    .get(seq_len - 1)
                    .map_err(|e| format!("get last position failed: {e}"))?;
                Ok(last)
            }
            2 => logits
                .squeeze(0)
                .map_err(|e| format!("squeeze failed: {e}")),
            1 => Ok(logits.clone()),
            _ => Err(format!("Unsupported logits shape: {:?}", dims)),
        }
    }
}

// ============================================================
// mlua UserData 注册
// ============================================================

impl mlua::UserData for MlSession {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // ─── 编解码 ────────────────────────────────────────
        methods.add_method("encode", |_, sess, text: String| {
            sess.encode(&text)
                .map_err(|e| mlua::Error::runtime(e))
        });

        methods.add_method("decode", |_, sess, token_id: u32| {
            sess.decode(token_id)
                .map_err(|e| mlua::Error::runtime(e))
        });

        // ─── 推理 ──────────────────────────────────────────
        methods.add_method("tensorize", |_, sess, token_ids: Vec<u32>| {
            sess.tensorize(&token_ids)
                .map_err(|e| mlua::Error::runtime(e))
        });

        methods.add_method_mut("forward", |_, sess, (tensor, offset): (mlua::Value, Option<usize>)| {
            // Tensor 跨 Lua 边界需要特殊处理，当前用 Value 占位
            let _ = (tensor, offset);
            Err(mlua::Error::runtime("Tensor passing requires binding layer"))
        });

        // ─── 采样 ──────────────────────────────────────────
        methods.add_method_mut("sample", |_, sess, temperature: f64| {
            sess.sample(temperature)
                .map_err(|e| mlua::Error::runtime(e))
        });

        // ─── 状态查询 ──────────────────────────────────────
        methods.add_method("get_eos", |_, sess, (): ()| {
            Ok(sess.get_eos())
        });

        methods.add_method("get_offset", |_, sess, (): ()| {
            Ok(sess.get_offset())
        });

        // ─── 析构 ──────────────────────────────────────────
        methods.add_method("unload", |_, sess, (): ()| {
            // sess 被 consume，Lua GC 后续会 drop userdata
            Ok(())
        });
    }
}
