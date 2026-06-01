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
use std::path::PathBuf;

use candle_core::{Device, Tensor, DType};

use super::device::parse_device_str;
use super::gguf_model::{
    GGUF_Load_Model, GGUF_Model, GGUF_Model_Inference, GGUF_Unload_Model,
};
use super::lua_tensor::LuaTensor;

// ============================================================
// Message — 对话消息
// ============================================================

/// 对话消息
#[derive(Debug, Clone)]
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

    // ─── Offload 状态 ────────────────────────────────────
    /// offload 后的 KV Cache（均在 CPU），等 to_cuda 恢复
    offloaded_kv: Option<Vec<(Tensor, Tensor)>>,
    /// offload 时保存的模型路径（用于 reload）
    offloaded_model_path: Option<PathBuf>,
    /// offload 时的层范围起始
    offloaded_layer_start: usize,
    /// offload 时的层范围结束
    offloaded_layer_end: usize,
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
        let device = parse_device_str(device)?;

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
                offloaded_kv: None,
                offloaded_model_path: None,
                offloaded_layer_start: 0,
                offloaded_layer_end: 0,
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

        // 保存实际加载范围，供 offload 使用
        self.ctx.offloaded_model_path = Some(path.to_path_buf());
        self.ctx.offloaded_layer_start = start;
        self.ctx.offloaded_layer_end = end;

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

    /// 已废弃：使用 `encode_messages()` 替代。
    #[deprecated(note = "use encode_messages() instead")]
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
    /// 优先使用 GGUF metadata 中的 chat_template（由 load_tokenizer 注入），
    /// 通过 minijinja 渲染。若无模板则 fallback 为硬编码 Qwen3 格式。
    pub fn apply_chat_template(&self, messages: &[Message]) -> String {
        // 1. 优先使用 GGUF metadata 中的 chat_template
        if let Some(ref tmpl_str) = self.ctx.chat_template {
            match Self::render_with_minijinja(tmpl_str, messages) {
                Ok(result) => return result,
                Err(e) => tracing::warn!("chat_template render failed, fallback to qwen3: {}", e),
            }
        }
        // 2. fallback: 硬编码 Qwen3（兼容无 chat_template 的旧模型或 split PGGUF）
        Self::fallback_qwen3_template(messages)
    }

    /// 将 Python 风格的字符串方法调用改写为 minijinja 过滤器语法。
    /// 参考: Crane (https://github.com/lucasjinreal/Crane)
    fn rewrite_python_str_methods(template: &str) -> String {
        let template = Self::rewrite_split_index(template);
        const METHODS: &[&str] = &["startswith", "endswith", "split", "lstrip", "rstrip", "strip"];
        let mut out = template.to_string();
        for method in METHODS {
            let pat = format!(".{}(", method);
            let repl = format!(" | {}(", method);
            out = out.replace(&pat, &repl);
        }
        out
    }

    /// 将 .split(X)[0] → | split(X) | first, .split(X)[-1] → | split(X) | last
    fn rewrite_split_index(template: &str) -> String {
        let mut out = String::with_capacity(template.len());
        let bytes = template.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            // 查找 .split(
            let suffix = b".split(";
            if let Some(pos) = bytes[i..].windows(suffix.len()).position(|w| w == suffix) {
                let abs = i + pos;
                out.push_str(&template[i..abs]);
                // 找到匹配的 )
                let after_dot_split = abs + suffix.len();
                if let Some(end) = Self::find_matching_paren(&template[after_dot_split..]) {
                    let split_args_end = after_dot_split + end + 1; // past the )
                    // 检查后面是否有 [0] 或 [-1]
                    let rest = &template[split_args_end..];
                    if rest.starts_with("[0]") {
                        out.push_str(" | split(");
                        out.push_str(&template[after_dot_split..split_args_end]);
                        out.push_str(" | first");
                        i = split_args_end + 3; // skip [0]
                    } else if rest.starts_with("[-1]") {
                        out.push_str(" | split(");
                        out.push_str(&template[after_dot_split..split_args_end]);
                        out.push_str(" | last");
                        i = split_args_end + 4; // skip [-1]
                    } else {
                        // 不处理的索引，原样保留
                        out.push_str(&template[abs..split_args_end]);
                        out.push_str(&rest[..rest.chars().next().map_or(0, |c| c.len_utf8())]);
                        i = split_args_end + rest.chars().next().map_or(0, |c| c.len_utf8());
                    }
                } else {
                    out.push_str(&template[abs..]);
                    i = len;
                }
            } else {
                out.push_str(&template[i..]);
                break;
            }
        }
        out
    }

    /// 找到匹配的 ) 位置（处理嵌套括号和引号）
    fn find_matching_paren(s: &str) -> Option<usize> {
        let mut depth = 1usize;
        let mut in_single = false;
        let mut in_double = false;
        for (i, c) in s.char_indices() {
            match c {
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '(' if !in_single && !in_double => depth += 1,
                ')' if !in_single && !in_double => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// minijinja 渲染 chat_template
    fn render_with_minijinja(tmpl_str: &str, messages: &[Message]) -> Result<String, String> {
        // 将 Python 风格的方法调用改写为 minijinja 过滤器语法
        // 参考: Crane (https://github.com/lucasjinreal/Crane)
        let tmpl_str = Self::rewrite_python_str_methods(tmpl_str);

        let mut env = minijinja::Environment::new();
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Lenient);

        // Python 兼容内置函数（tojson, namespace, is string 等）
        minijinja_contrib::add_to_environment(&mut env);

        // 注册 Python 兼容的字符串过滤器
        env.add_filter("startswith", |s: &str, prefix: &str| -> bool { s.starts_with(prefix) });
        env.add_filter("endswith", |s: &str, suffix: &str| -> bool { s.ends_with(suffix) });
        env.add_filter("split", |s: String, sep: String| -> Vec<String> {
            s.split(&sep).map(|p| p.to_string()).collect()
        });
        env.add_filter("lstrip", |s: String, chars: Option<String>| -> String {
            match chars {
                None => s.trim_start().to_string(),
                Some(c) => {
                    let ch: Vec<char> = c.chars().collect();
                    s.trim_start_matches(ch.as_slice()).to_string()
                }
            }
        });
        env.add_filter("rstrip", |s: String, chars: Option<String>| -> String {
            match chars {
                None => s.trim_end().to_string(),
                Some(c) => {
                    let ch: Vec<char> = c.chars().collect();
                    s.trim_end_matches(ch.as_slice()).to_string()
                }
            }
        });
        env.add_filter("strip", |s: String, chars: Option<String>| -> String {
            match chars {
                None => s.trim().to_string(),
                Some(c) => {
                    let ch: Vec<char> = c.chars().collect();
                    s.trim_matches(ch.as_slice()).to_string()
                }
            }
        });

        env.add_template("chat", &tmpl_str)
            .map_err(|e| format!("parse template: {}", e))?;
        let tmpl = env.get_template("chat")
            .map_err(|e| format!("get template: {}", e))?;

        let msgs: Vec<minijinja::value::Value> = messages.iter().map(|m| {
            let mut map: std::collections::BTreeMap<String, minijinja::Value> =
                std::collections::BTreeMap::new();
            map.insert("role".into(), m.role.clone().into());
            map.insert("content".into(), m.content.clone().into());
            minijinja::value::Value::from(map)
        }).collect();

        tmpl.render(minijinja::context! { messages => msgs, add_generation_prompt => true })
            .map_err(|e| format!("render: {}", e))
    }

    /// fallback Qwen3 格式（无 chat_template 时使用）
    fn fallback_qwen3_template(messages: &[Message]) -> String {
        let mut result = String::new();
        for msg in messages {
            result.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", msg.role, msg.content));
        }
        result.push_str("<|im_start|>assistant\n");
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

    // ─── Offloading ────────────────────────────────────────

    /// 将模型从 GPU 卸下：提取 KV → 移到 CPU → 释放模型 → 挂起
    pub fn offload_to_cpu(&mut self) -> Result<(), String> {
        let model = self.ctx.model.as_ref()
            .ok_or("offload_to_cpu: no model loaded")?;

        // 验证 reload 信息已在 load_model 时保存
        if self.ctx.offloaded_model_path.is_none() {
            return Err("offload_to_cpu: missing model path (call load_model first)".into());
        }

        // 1. 提取 KV Cache（仍在原设备）
        let kvs = model.model.extract_kv_cache()?;

        // 2. 将 KV tensor 移到 CPU
        let kvs_cpu: Vec<(Tensor, Tensor)> = kvs.into_iter()
            .map(|(k, v)| {
                let k_cpu = k.to_device(&Device::Cpu)
                    .map_err(|e| format!("k to_device cpu: {e}"))?;
                let v_cpu = v.to_device(&Device::Cpu)
                    .map_err(|e| format!("v to_device cpu: {e}"))?;
                Ok((k_cpu, v_cpu))
            })
            .collect::<Result<_, String>>()?;

        // 3. 释放 GPU 模型
        if let Some(old) = self.ctx.model.take() {
            GGUF_Unload_Model(old);
        }

        // 4. 保存挂起状态（reload 信息已在 load_model 时保存）
        self.ctx.offloaded_kv = Some(kvs_cpu);

        Ok(())
    }

    /// 将模型恢复到 GPU：reload 权重 → KV 移回 GPU → 恢复 KV Cache
    pub fn offload_to_cuda(&mut self) -> Result<(), String> {
        if self.ctx.model.is_some() {
            return Err("offload_to_cuda: model is already loaded".into());
        }

        let kvs = self.ctx.offloaded_kv.take()
            .ok_or("offload_to_cuda: no offloaded KV cache")?;
        let model_path = self.ctx.offloaded_model_path.clone()
            .ok_or("offload_to_cuda: no offloaded model path")?;
        let start = self.ctx.offloaded_layer_start;
        let end = self.ctx.offloaded_layer_end;

        // 1. 在目标 device 上重建权重
        let mut model = GGUF_Load_Model(start, end, &model_path, &self.ctx.device)
            .map_err(|e| format!("offload_to_cuda: reload model failed: {e}"))?;

        // 2. 将 KV 移回目标 device 并恢复
        let kvs_device: Vec<(Tensor, Tensor)> = kvs.into_iter()
            .map(|(k, v)| {
                let k_dev = k.to_device(&self.ctx.device)
                    .map_err(|e| format!("k to_device gpu: {e}"))?;
                let v_dev = v.to_device(&self.ctx.device)
                    .map_err(|e| format!("v to_device gpu: {e}"))?;
                Ok((k_dev, v_dev))
            })
            .collect::<Result<_, String>>()?;

        model.model.restore_kv_cache(kvs_device)?;

        self.ctx.model = Some(model);
        // 重新保存 reload 信息，供下一轮 offload_to_cpu 使用
        self.ctx.offloaded_model_path = Some(model_path);
        Ok(())
    }

    /// 将 KV Cache 保存到 .kvcache/ 目录，并释放模型
    pub fn offload_save(&mut self, file_id: &str) -> Result<(), String> {
        // 1. 获取 KV（从模型或已 offload 状态）
        let kvs = if let Some(ref kvs) = self.ctx.offloaded_kv {
            kvs.clone()
        } else {
            let model = self.ctx.model.as_ref()
                .ok_or("offload_save: no model loaded and no offloaded KV")?;

            let kvs_raw = model.model.extract_kv_cache()?;
            kvs_raw.into_iter()
                .map(|(k, v)| {
                    let k_cpu = k.to_device(&Device::Cpu)
                        .map_err(|e| format!("k to_device cpu: {e}"))?;
                    let v_cpu = v.to_device(&Device::Cpu)
                        .map_err(|e| format!("v to_device cpu: {e}"))?;
                    Ok((k_cpu, v_cpu))
                })
                .collect::<Result<Vec<_>, String>>()?
        };

        // 2. 确定文件路径
        let file_path = Path::new(".kvcache").join(file_id);

        // 3. 序列化并写入
        Self::serialize_kv_to_file(
            &file_path,
            &kvs,
            self.ctx.offloaded_model_path.as_ref()
                .ok_or("offload_save: missing model path")?,
            self.ctx.offloaded_layer_start,
            self.ctx.offloaded_layer_end,
            &self.ctx.device,
            self.ctx.rng_state,
            self.ctx.offset,
            self.ctx.eos_token_id,
            self.ctx.chat_template.as_deref(),
        )?;

        // 4. 释放模型（如果还在）
        if self.ctx.model.is_some() {
            if let Some(old) = self.ctx.model.take() {
                GGUF_Unload_Model(old);
            }
            // KV 保留在 CPU
            self.ctx.offloaded_kv = Some(kvs);
        }

        Ok(())
    }

    /// 从 .kvcache/ 恢复 session
    pub fn offload_load(file_id: &str, device_str: &str) -> Result<MlSession, String> {
        let file_path = Path::new(".kvcache").join(file_id);

        // 1. 反序列化
        let (kvs, model_path, start, end, _device, rng_state, offset, eos_token_id, chat_template) =
            Self::deserialize_kv_from_file(&file_path)?;

        let target_device = parse_device_str(device_str)?;

        // 2. 加载模型权重
        let mut model = GGUF_Load_Model(start, end, &model_path, &target_device)
            .map_err(|e| format!("offload_load: load model failed: {e}"))?;

        // 3. KV 移到目标 device 并恢复
        let kvs_device: Vec<(Tensor, Tensor)> = kvs.into_iter()
            .map(|(k, v)| {
                let k_dev = k.to_device(&target_device)
                    .map_err(|e| format!("k to_device: {e}"))?;
                let v_dev = v.to_device(&target_device)
                    .map_err(|e| format!("v to_device: {e}"))?;
                Ok((k_dev, v_dev))
            })
            .collect::<Result<_, String>>()?;

        model.model.restore_kv_cache(kvs_device)?;

        // 4. 组装 MlSession
        let ctx = MlContext {
            model: Some(model),
            tokenizer: None,
            offset,
            rng_state,
            eos_token_id,
            chat_template,
            device: target_device,
            offloaded_kv: None,
            offloaded_model_path: None,
            offloaded_layer_start: 0,
            offloaded_layer_end: 0,
        };

        Ok(MlSession { ctx })
    }

    // ─── 序列化/反序列化 ──────────────────────────────────

    /// KV 文件魔数
    const KVCX_MAGIC: [u8; 4] = *b"KVCX";
    /// KV 文件格式版本
    const KVCX_VERSION: u32 = 1;

    fn write_u32_le(buf: &mut Vec<u8>, v: u32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn write_u64_le(buf: &mut Vec<u8>, v: u64) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    fn serialize_kv_to_file(
        path: &Path,
        kvs: &[(Tensor, Tensor)],
        model_path: &Path,
        start: usize,
        end: usize,
        _device: &Device,
        rng_state: u64,
        offset: usize,
        eos_token_id: u32,
        chat_template: Option<&str>,
    ) -> Result<(), String> {
        let mut buf: Vec<u8> = Vec::new();

        // --- header ---
        buf.extend_from_slice(&Self::KVCX_MAGIC);
        Self::write_u32_le(&mut buf, Self::KVCX_VERSION);

        let model_path_str = model_path.to_string_lossy();
        let model_path_bytes = model_path_str.as_bytes();
        Self::write_u32_le(&mut buf, model_path_bytes.len() as u32);
        buf.extend_from_slice(model_path_bytes);

        Self::write_u32_le(&mut buf, start as u32);
        Self::write_u32_le(&mut buf, end as u32);

        Self::write_u64_le(&mut buf, rng_state);
        Self::write_u64_le(&mut buf, offset as u64);
        Self::write_u32_le(&mut buf, eos_token_id);

        let ct = chat_template.unwrap_or("");
        let ct_bytes = ct.as_bytes();
        Self::write_u32_le(&mut buf, ct_bytes.len() as u32);
        buf.extend_from_slice(ct_bytes);

        Self::write_u32_le(&mut buf, kvs.len() as u32);

        // --- per-layer KV ---
        for (k, v) in kvs {
            Self::serialize_tensor(&mut buf, k)?;
            Self::serialize_tensor(&mut buf, v)?;
        }

        std::fs::write(path, &buf)
            .map_err(|e| format!("write kv file: {e}"))
    }

    fn serialize_tensor(buf: &mut Vec<u8>, t: &Tensor) -> Result<(), String> {
        // Convert to F32 for uniform serialization
        let t_f32 = t.to_dtype(DType::F32)
            .map_err(|e| format!("to_dtype f32: {e}"))?;
        let dims = t_f32.dims();
        let flat: Vec<f32> = t_f32.to_vec1()
            .map_err(|e| format!("to_vec1: {e}"))?;

        Self::write_u32_le(buf, dims.len() as u32);
        for d in dims.iter() {
            Self::write_u64_le(buf, *d as u64);
        }
        Self::write_u64_le(buf, flat.len() as u64);
        let raw: &[u8] = unsafe {
            std::slice::from_raw_parts(flat.as_ptr() as *const u8, flat.len() * 4)
        };
        buf.extend_from_slice(raw);

        Ok(())
    }

    fn deserialize_kv_from_file(
        path: &Path,
    ) -> Result<(Vec<(Tensor, Tensor)>, PathBuf, usize, usize, Device, u64, usize, u32, Option<String>), String> {
        let data = std::fs::read(path)
            .map_err(|e| format!("read kv file {}: {e}", path.display()))?;
        let mut offset = 0usize;

        // --- header ---
        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        if &data[offset..offset + 4] != Self::KVCX_MAGIC {
            return Err("bad magic: not a KVCX file".into());
        }
        offset += 4;

        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let version = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        offset += 4;
        if version != Self::KVCX_VERSION {
            return Err(format!("unsupported version: {}", version));
        }

        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let model_path_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        if offset + model_path_len > data.len() { return Err("unexpected EOF".into()); }
        let model_path_bytes = &data[offset..offset + model_path_len];
        offset += model_path_len;
        let model_path = PathBuf::from(
            std::str::from_utf8(model_path_bytes)
                .map_err(|e| format!("invalid utf8 in model_path: {e}"))?
        );

        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let start = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let end = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        if offset + 8 > data.len() { return Err("unexpected EOF".into()); }
        let rng_state = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        offset += 8;
        if offset + 8 > data.len() { return Err("unexpected EOF".into()); }
        let off = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8;

        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let eos_token_id = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        offset += 4;

        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let ct_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if offset + ct_len > data.len() { return Err("unexpected EOF".into()); }
        let ct_bytes = &data[offset..offset + ct_len];
        offset += ct_len;
        let chat_template = if ct_bytes.is_empty() {
            None
        } else {
            Some(std::str::from_utf8(ct_bytes)
                .map_err(|e| format!("invalid utf8 in chat_template: {e}"))?
                .to_string())
        };

        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let num_layers = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        // --- per-layer KV ---
        let mut kvs: Vec<(Tensor, Tensor)> = Vec::with_capacity(num_layers);
        for _ in 0..num_layers {
            let (k, new_off) = Self::deserialize_tensor(&data, offset)?;
            offset = new_off;
            let (v, new_off) = Self::deserialize_tensor(&data, offset)?;
            offset = new_off;
            kvs.push((k, v));
        }

        Ok((kvs, model_path, start, end, Device::Cpu, rng_state, off, eos_token_id, chat_template))
    }

    fn deserialize_tensor(data: &[u8], mut offset: usize) -> Result<(Tensor, usize), String> {
        if offset + 4 > data.len() { return Err("unexpected EOF".into()); }
        let ndim = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        let mut shape: Vec<usize> = Vec::with_capacity(ndim);
        for _ in 0..ndim {
            if offset + 8 > data.len() { return Err("unexpected EOF".into()); }
            shape.push(u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize);
            offset += 8;
        }

        if offset + 8 > data.len() { return Err("unexpected EOF".into()); }
        let data_len = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as usize;
        offset += 8;

        if offset + data_len > data.len() { return Err("unexpected EOF".into()); }
        let raw = &data[offset..offset + data_len];
        offset += data_len;

        let elem_count = raw.len() / 4;
        let f32s: &[f32] = unsafe {
            std::slice::from_raw_parts(raw.as_ptr() as *const f32, elem_count)
        };

        let t = Tensor::from_vec(f32s.to_vec(), shape.as_slice(), &Device::Cpu)
            .map_err(|e| format!("from_vec: {e}"))?;

        Ok((t, offset))
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
            let messages = vec![Message { role: "user".into(), content: text }];
            sess.encode_messages(&messages).map_err(|e| mlua::Error::runtime(e))
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
            "reset_kv_cache",
            |_, sess, (): ()| {
                sess.reset_kv_cache();
                Ok(())
            },
        );

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

        // ─── Offloading ────────────────────────────────────
        methods.add_method_mut("offload_to_cpu", |_, sess, (): ()| {
            sess.offload_to_cpu().map_err(|e| mlua::Error::runtime(e))
        });

        methods.add_method_mut("offload_to_cuda", |_, sess, (): ()| {
            sess.offload_to_cuda().map_err(|e| mlua::Error::runtime(e))
        });

        methods.add_method_mut("offload_save", |_, sess, file_id: String| {
            sess.offload_save(&file_id).map_err(|e| mlua::Error::runtime(e))
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
