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
    GGUF_Load_Model, GGUF_Model, GGUF_Model_Inference, GGUF_Unload_Model,
};
use super::lua_tensor::LuaTensor;

// ============================================================
// Message — 对话消息
// ============================================================

/// 对话消息
pub struct Message {
    pub role: String,   // "system", "user", "assistant"
    pub content: String,
}

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
    /// 模型权重（None = 空壳状态）
    model: Option<GGUF_Model>,
    /// tokenizer，独立加载（None = 未加载）
    tokenizer: Option<shimmytok::Tokenizer>,
    /// 自增序列位置（forward offset=None 时自动 += seq_len）
    offset: usize,
    /// 采样随机数生成器状态 (xoshiro)
    rng_state: u64,
    /// EOS token ID
    eos_token_id: u32,
    /// chat template，从 GGUF metadata 读取
    chat_template: Option<String>,
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
                tokenizer: None,
                offset: 0,
                rng_state: default_seed,
                eos_token_id: 151645, // Qwen3 默认 EOS
                chat_template: None,
                device,
            },
        })
    }

    /// 加载模型权重到当前 session。
    ///
    /// 只加载权重，不加载 tokenizer。如需 tokenizer 请调用 `load_tokenizer()`。
    /// 若已有模型，先验证新模型路径有效再卸载旧模型，
    /// 避免因路径无效导致旧模型丢失。
    pub fn load_model(&mut self, path: &Path, start: usize, end: usize) -> Result<(), String> {
        // 先尝试加载新模型（失败则保留旧模型不动）
        let model = GGUF_Load_Model(start, end, path, &self.ctx.device)
            .map_err(|e| format!("Failed to load model: {e}"))?;

        // 新模型加载成功 → 安全卸载旧模型
        if let Some(old) = self.ctx.model.take() {
            GGUF_Unload_Model(old);
        }

        self.ctx.model = Some(model);
        self.ctx.offset = 0;

        Ok(())
    }

    /// 仅加载 tokenizer，不加载模型权重。
    ///
    /// 从 GGUF/PGGUF 文件中提取 tokenizer 和 eos_token_id。
    /// 适合只需要 encode/decode 能力的场景（如 Session）。
    pub fn load_tokenizer(&mut self, path: &Path) -> Result<(), String> {
        let tokenizer = shimmytok::Tokenizer::from_gguf_file(path)
            .map_err(|e| format!("Failed to load tokenizer from {}: {}", path.display(), e))?;

        // 同时从文件中解析 eos_token_id
        let arch_info = super::gguf_model_manager::GGUF_Analyze(path)
            .map_err(|e| format!("Failed to analyze model for eos token: {}", e))?;
        self.ctx.eos_token_id = arch_info.eos_token_id;
        self.ctx.chat_template = arch_info.chat_template.clone();

        self.ctx.tokenizer = Some(tokenizer);
        Ok(())
    }

    /// 卸载模型，回到空壳状态。
    ///
    /// 同时清除模型权重和 tokenizer。不消耗 self，session 可重复 load_model / load_tokenizer。
    pub fn unload(&mut self) {
        if let Some(model) = self.ctx.model.take() {
            GGUF_Unload_Model(model);
        }
        self.ctx.tokenizer = None;
        self.ctx.offset = 0;
        self.ctx.eos_token_id = 151645;
    }

    /// 检查模型是否已加载。
    pub fn has_model(&self) -> bool {
        self.ctx.model.is_some()
    }

    // ─── 编解码 ────────────────────────────────────────────

    pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> {
        let tokenizer = self
            .ctx
            .tokenizer
            .as_ref()
            .ok_or("encode: no tokenizer loaded. Call load_tokenizer() first.")?;
        let format_prompt = format!("<|im_start|>user\n{text}<|im_end|>\n<|im_start|>assistant\n");
        let opts = shimmytok::EncodeOptions::with_parse_special(true, true);
        tokenizer
            .encode_with_options(&format_prompt, &opts)
            .map_err(|e| format!("Encode failed: {e}"))
    }

    /// 使用 chat template 编码 messages 数组。
    ///
    /// 若有 chat_template → apply_template → tokenize。
    /// 若无 → fallback 为当前硬编码 Qwen3 格式（仅支持最后一轮 user）。
    pub fn encode_messages(&self, messages: &[Message]) -> Result<Vec<u32>, String> {
        let tokenizer = self
            .ctx
            .tokenizer
            .as_ref()
            .ok_or("encode_messages: no tokenizer loaded.")?;

        let prompt = self.apply_chat_template(messages);
        let opts = shimmytok::EncodeOptions::with_parse_special(true, true);
        tokenizer
            .encode_with_options(&prompt, &opts)
            .map_err(|e| format!("Encode messages failed: {e}"))
    }

    /// 应用 chat template 到 messages 数组，返回格式化文本。
    ///
    /// 当前使用硬编码 Qwen3 格式。GGUF 中的 tokenizer.chat_template 已读取但未使用，
    /// 因为需要完整 Jinja 引擎才能解析（see Task 6 v2 总文档 §局限）。
    fn apply_chat_template(&self, _messages: &[Message]) -> String {
        let mut result = String::new();
        for msg in _messages {
            if msg.role == "system" {
                result.push_str(&format!("<|im_start|>system\n{}<|im_end|>\n", msg.content));
            } else if msg.role == "user" {
                result.push_str(&format!("<|im_start|>user\n{}<|im_end|>\n", msg.content));
            } else if msg.role == "assistant" {
                result.push_str(&format!("<|im_start|>assistant\n{}<|im_end|>\n", msg.content));
            }
        }
        result.push_str("<|im_start|>assistant\n");
        result
    }

    /// 简易 Jinja 模板渲染 — 保留作为后续扩展参考
    fn render_template(tmpl: &str, messages: &[Message]) -> String {
        // 提取 for 循环内的文本
        let for_tag = "{% for message in messages %}";
        let endfor_tag = "{% endfor %}";
        let mut result = String::new();

        if let Some(for_start) = tmpl.find(for_tag) {
            // for 之前的内容
            result.push_str(&tmpl[..for_start]);
            let body_start = for_start + for_tag.len();
            if let Some(body_end) = tmpl[body_start..].find(endfor_tag) {
                let body = &tmpl[body_start..body_start + body_end];
                let after = &tmpl[body_start + body_end + endfor_tag.len()..];
                for msg in messages {
                    let line = body
                        .replace("{{ message.role }}", &msg.role)
                        .replace("{{ message.content }}", &msg.content);
                    result.push_str(&line);
                }
                result.push_str(after);
            }
        } else {
            // 无模板语法，直接返回原文
            result = tmpl.to_string();
        }
        result
    }

    pub fn decode(&self, token_id: u32) -> Result<String, String> {
        let tokenizer = self
            .ctx
            .tokenizer
            .as_ref()
            .ok_or("decode: no tokenizer loaded. Call load_tokenizer() first.")?;
        let eos = self.ctx.eos_token_id;
        let tokens: &[u32] = if token_id == eos { &[] } else { std::slice::from_ref(&token_id) };
        tokenizer
            .decode(tokens, true)
            .map_err(|e| format!("Decode failed: {e}"))
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
            // f64 避免 u32::MAX 超出 f32 精确表示范围导致的分布偏差
            let random = (self.ctx.rng_state >> 33) as f64 / (u32::MAX as f64);

            let mut cumulative = 0.0f64;
            let mut chosen = (probs_vec.len() - 1) as u32;
            for (i, &p) in probs_vec.iter().enumerate() {
                cumulative += p as f64;
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

    /// 清除 KV Cache（每轮对话开始前调用）
    pub fn reset_kv_cache(&mut self) {
        if let Some(ref mut model) = self.ctx.model {
            super::gguf_model::GGUF_Model_Clear_KV_Cache(model);
        }
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
                let p = std::path::Path::new(&path);
                if p.extension().map_or(true, |e| e != "pgguf") {
                    return Err(mlua::Error::runtime(
                        "load_model 仅支持 .pgguf 格式，请先用 analyze_model 转换 .gguf 文件"
                    ));
                }
                sess.load_model(p, start, end)
                    .map_err(|e| mlua::Error::runtime(e))
            },
        );

        methods.add_method_mut(
            "load_tokenizer",
            |_, sess, path: String| {
                sess.load_tokenizer(std::path::Path::new(&path))
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
    fn test_encode_without_tokenizer_errors() {
        let sess = MlSession::new("cpu").expect("create empty session");
        let result = sess.encode("hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no tokenizer"));
    }

    #[test]
    fn test_decode_without_tokenizer_errors() {
        let sess = MlSession::new("cpu").expect("create empty session");
        let result = sess.decode(123);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no tokenizer"));
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
