//Presented by KeJi
//Date ： 2026-05-17

//! ML Engine 能力层
//!
//! 通过 `MlSession` userdata 暴露给 Lua，类似 Python class：
//!
//! ```lua
//! local sess = ml.new("cpu")
//! sess:load_model("model.gguf", 0, 999999)
//! local tokens = sess:encode(prompt)
//! local logits = sess:forward(sess:tensorize(tokens), 0)
//! local tok = sess:sample(logits, 0.8)
//! sess:unload()
//! ```

#![allow(non_snake_case)]

use std::path::Path;

use candle_core::{Device, Tensor};

use super::gguf_model::{
    GGUF_Decode, GGUF_Encode, GGUF_Load_Model, GGUF_Model, GGUF_Model_Inference, GGUF_Unload_Model,
};
use super::lua_tensor::LuaTensor;

// ============================================================
// MlSession — 对外句柄 (Lua userdata)
// ============================================================

/// 推理会话。空壳构造 + 按需加载模型。
///
/// ```text
///          ml.new("cpu")
///             │
///             ▼
///    ┌─────────────────┐
///    │ model = None     │  ← 空壳
///    │ device = Cpu     │
///    │ eos = default    │
///    └───────┬─────────┘
///            │ sess:load_model(path, 0, 40)
///            ▼
///    ┌─────────────────┐
///    │ model = Some(..) │  ← 已加载
///    └───────┬─────────┘
///            │ sess:unload()
///            ▼
///    ┌─────────────────┐
///    │ model = None     │  ← 回到空壳
///    └─────────────────┘
/// ```
pub struct MlSession {
    ctx: MlContext,
}

// ============================================================
// MlContext — 内部状态（外部不可见）
// ============================================================

struct MlContext {
    /// 模型权重 + tokenizer（None = 空壳状态）
    model: Option<GGUF_Model>,
    /// 自增序列位置（forward offset=None 时自动 += seq_len）
    offset: usize,
    /// 采样随机数生成器状态 (xoshiro)
    rng_state: u64,
    /// EOS token ID
    eos_token_id: u32,
    /// 运行设备
    device: Device,
}

// ============================================================
// MlSession — Rust 侧公开方法
// ============================================================

impl MlSession {
    // ─── 构造 / 析构 ───────────────────────────────────────

    /// 创建空壳 MlSession（不加载模型）。
    ///
    /// 空壳状态下可用方法：
    /// - `tensorize()` — 纯数据转换，仅需 device
    /// - `get_eos()` — 返回默认 EOS token
    /// - `get_offset()` — 返回 0
    /// - `set_seed()` — 设置采样随机种子
    /// - `load_model()` — 加载模型填充空壳
    pub fn new(device: &str) -> Result<Self, String> {
        let device = match device.to_lowercase().as_str() {
            "cpu" => Device::Cpu,
            "cuda" => match Device::new_cuda(0) {
                Ok(d) => d,
                Err(e) => return Err(format!("CUDA device init failed: {e}")),
            },
            other => {
                return Err(format!(
                    "Unsupported device: '{other}'. Use 'cpu' or 'cuda'."
                ))
            }
        };

        // 默认种子基于系统时间，避免所有 session 使用相同随机序列
        let default_seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(299792458);

        Ok(Self {
            ctx: MlContext {
                model: None,
                offset: 0,
                rng_state: default_seed,
                eos_token_id: 151645, // Qwen3 默认 EOS
                device,
            },
        })
    }

    /// 加载模型到当前 session。
    ///
    /// 若已有模型，先卸载旧模型再加载新模型。
    pub fn load_model(&mut self, path: &Path, start: usize, end: usize) -> Result<(), String> {
        // 先卸载已有模型
        if self.ctx.model.is_some() {
            self.unload();
        }

        let model = GGUF_Load_Model(start, end, path, &self.ctx.device)
            .map_err(|e| format!("Failed to load model: {e}"))?;

        self.ctx.eos_token_id = model.inference_config.eos_token;
        self.ctx.model = Some(model);
        self.ctx.offset = 0;

        Ok(())
    }

    /// 卸载模型，回到空壳状态。
    ///
    /// 不消耗 self，session 可重复 load_model。
    pub fn unload(&mut self) {
        if let Some(model) = self.ctx.model.take() {
            GGUF_Unload_Model(model);
        }
        self.ctx.offset = 0;
        self.ctx.eos_token_id = 151645;
    }

    /// 检查模型是否已加载。
    pub fn has_model(&self) -> bool {
        self.ctx.model.is_some()
    }

    // ─── 编解码 ────────────────────────────────────────────

    pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> {
        let model = self
            .ctx
            .model
            .as_ref()
            .ok_or("encode: no model loaded. Call load_model() first.")?;
        GGUF_Encode(model, text).map_err(|e| format!("Encode failed: {e}"))
    }

    pub fn decode(&self, token_id: u32) -> Result<String, String> {
        let model = self
            .ctx
            .model
            .as_ref()
            .ok_or("decode: no model loaded. Call load_model() first.")?;
        GGUF_Decode(model, &[token_id]).map_err(|e| format!("Decode failed: {e}"))
    }

    // ─── 推理 ──────────────────────────────────────────────

    /// 纯数据转换：`Vec<u32>` → `Tensor[1, seq_len]`。
    ///
    /// 不依赖模型，空壳状态可用。
    pub fn tensorize(&self, token_ids: &[u32]) -> Result<Tensor, String> {
        if token_ids.is_empty() {
            return Err("tensorize: token_ids is empty".into());
        }
        Tensor::new(token_ids, &self.ctx.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| format!("Tensorize failed: {e}"))
    }

    pub fn forward(&mut self, tensor: &Tensor, offset: Option<usize>) -> Result<Tensor, String> {
        let model = self
            .ctx
            .model
            .as_mut()
            .ok_or("forward: no model loaded. Call load_model() first.")?;
        let off = offset.unwrap_or(self.ctx.offset);
        let seq_len = tensor.dims().get(1).copied().unwrap_or(1);

        let output =
            GGUF_Model_Inference(model, tensor, off).map_err(|e| format!("Forward failed: {e}"))?;

        self.ctx
            .device
            .synchronize()
            .map_err(|e| format!("Device sync failed: {e}"))?;

        if offset.is_none() {
            self.ctx.offset += seq_len;
        }

        Ok(output)
    }

    // ─── 采样 ──────────────────────────────────────────────

    /// 对 logits tensor 采样，返回下一个 token ID。
    ///
    /// logits 形状: [1, seq_len, vocab_size] 或 [vocab_size]
    pub fn sample(&mut self, logits: &Tensor, temperature: f64) -> Result<u32, String> {
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
            let probs =
                candle_nn::ops::softmax(&scaled, 0).map_err(|e| format!("softmax failed: {e}"))?;
            let probs_vec: Vec<f32> = probs
                .to_vec1::<f32>()
                .map_err(|e| format!("probs → vec failed: {e}"))?;

            self.ctx.rng_state = self
                .ctx
                .rng_state
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

    /// 返回 EOS token ID。空壳时返回默认值 151645。
    pub fn get_eos(&self) -> u32 {
        self.ctx.eos_token_id
    }

    pub fn get_offset(&self) -> usize {
        self.ctx.offset
    }

    // ─── 随机种子 ──────────────────────────────────────────

    /// 设置采样随机数生成器的种子。
    ///
    /// 用于 temperature > 0 时的 multinomial 采样。设置相同种子可复现推理结果。
    /// 空壳状态下也可调用——种子独立于模型加载。
    pub fn set_seed(&mut self, seed: u64) {
        self.ctx.rng_state = seed;
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
        // ─── 生命周期 ──────────────────────────────────────
        methods.add_method_mut(
            "load_model",
            |_, sess, (path, start, end): (String, usize, usize)| {
                sess.load_model(std::path::Path::new(&path), start, end)
                    .map_err(|e| mlua::Error::runtime(e))
            },
        );

        methods.add_method("has_model", |_, sess, (): ()| Ok(sess.has_model()));

        methods.add_method_mut("unload", |_, sess, (): ()| {
            sess.unload();
            Ok(())
        });

        // ─── 编解码 ────────────────────────────────────────
        methods.add_method("encode", |_, sess, text: String| {
            sess.encode(&text).map_err(|e| mlua::Error::runtime(e))
        });

        methods.add_method("decode", |_, sess, token_id: u32| {
            sess.decode(token_id).map_err(|e| mlua::Error::runtime(e))
        });

        // ─── 推理 ──────────────────────────────────────────
        methods.add_method("tensorize", |_, sess, token_ids: Vec<u32>| {
            let t = sess
                .tensorize(&token_ids)
                .map_err(|e| mlua::Error::runtime(e))?;
            Ok(LuaTensor(t))
        });

        methods.add_method_mut(
            "forward",
            |_, sess, (tensor, offset): (mlua::AnyUserData, Option<usize>)| {
                let t = tensor.borrow::<LuaTensor>().map_err(|e| {
                    mlua::Error::runtime(format!("forward: expected LuaTensor: {e}"))
                })?;
                let output = sess
                    .forward(&t, offset)
                    .map_err(|e| mlua::Error::runtime(e))?;
                Ok(LuaTensor(output))
            },
        );

        // ─── 采样 ──────────────────────────────────────────
        methods.add_method_mut(
            "sample",
            |_, sess, (logits, temperature): (mlua::AnyUserData, f64)| {
                let t = logits.borrow::<LuaTensor>().map_err(|e| {
                    mlua::Error::runtime(format!("sample: expected LuaTensor: {e}"))
                })?;
                sess.sample(&t, temperature)
                    .map_err(|e| mlua::Error::runtime(e))
            },
        );

        // ─── 状态查询 ──────────────────────────────────────
        methods.add_method("get_eos", |_, sess, (): ()| Ok(sess.get_eos()));

        methods.add_method("get_offset", |_, sess, (): ()| Ok(sess.get_offset()));

        // ─── 随机种子 ──────────────────────────────────────
        methods.add_method_mut("set_seed", |_, sess, seed: u64| {
            sess.set_seed(seed);
            Ok(())
        });
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ─── 空壳状态测试 ──────────────────────────────────────

    #[test]
    fn test_new_creates_empty_session() {
        let sess = MlSession::new("cpu").expect("create empty session");
        assert!(!sess.has_model());
        assert_eq!(sess.get_eos(), 151645);
        assert_eq!(sess.get_offset(), 0);
    }

    #[test]
    fn test_tensorize_without_model() {
        let sess = MlSession::new("cpu").expect("create empty session");
        let t = sess.tensorize(&[1, 2, 3, 4, 5]).expect("tensorize");
        let dims = t.dims();
        assert_eq!(dims.len(), 2);
        assert_eq!(dims[0], 1); // batch
        assert_eq!(dims[1], 5); // seq_len
    }

    #[test]
    fn test_tensorize_empty_input_errors() {
        let sess = MlSession::new("cpu").expect("create empty session");
        let result = sess.tensorize(&[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_encode_without_model_errors() {
        let sess = MlSession::new("cpu").expect("create empty session");
        let result = sess.encode("hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no model"));
    }

    #[test]
    fn test_forward_without_model_errors() {
        let mut sess = MlSession::new("cpu").expect("create empty session");
        let t = sess.tensorize(&[1, 2, 3]).expect("tensorize");
        let result = sess.forward(&t, Some(0));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no model"));
    }

    #[test]
    fn test_unload_on_empty_session_is_noop() {
        let mut sess = MlSession::new("cpu").expect("create empty session");
        sess.unload(); // 不应 panic
        assert!(!sess.has_model());
    }

    // ─── 生命周期测试 ──────────────────────────────────────

    #[test]
    fn test_unload_returns_to_empty_state() {
        let mut sess = MlSession::new("cpu").expect("create empty session");
        // 无法真正 load（需要 GGUF 文件），但 unload 应安全
        sess.unload();
        assert_eq!(sess.get_eos(), 151645);
        assert_eq!(sess.get_offset(), 0);
        assert!(!sess.has_model());
    }
}
