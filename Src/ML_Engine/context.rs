//Presented by KeJi
//Date ： 2026-05-16

//! ML Engine 推理上下文
//!
//! `MlContext` 封装模型运行时状态，提供原子推理操作。
//! 所有方法为同步函数，由 Lua 线程直接调用。

#![allow(non_snake_case)]

use std::path::Path;

use candle_core::{Device, Tensor};

use super::gguf_model::{
    GGUF_Decode, GGUF_Encode, GGUF_Load_Model, GGUF_Model,
    GGUF_Model_Inference, GGUF_Unload_Model,
};

// ============================================================
// MlContext
// ============================================================

/// Lua 线程绑定的推理上下文（外部不可见）
///
/// 5 个字段，无 input 缓冲区——Tensor 由调用方直接传入 `forward()`。
pub struct MlContext {
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

impl MlContext {
    // ============================================================
    // 模型生命周期
    // ============================================================

    /// 加载模型，创建 MlContext。
    ///
    /// 内部调用 `GGUF_Load_Model(start, end, path, device)`。
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
            model,
            output: None,
            offset: 0,
            rng_state: 299792458,
            eos_token_id,
        })
    }

    /// 卸载模型，释放所有权重和 KV Cache。
    pub fn unload_model(self) {
        GGUF_Unload_Model(self.model);
        // MlContext 其余字段自动 drop
    }

    // ============================================================
    // 编解码
    // ============================================================

    /// 文本 → token IDs。自动包装 Qwen3 对话模板。
    pub fn encode(&self, text: &str) -> Result<Vec<u32>, String> {
        GGUF_Encode(&self.model, text)
            .map_err(|e| format!("Encode failed: {e}"))
    }

    /// 单个 token ID → 文本。自动跳过 EOS token。
    pub fn decode(&self, token_id: u32) -> Result<String, String> {
        GGUF_Decode(&self.model, &[token_id])
            .map_err(|e| format!("Decode failed: {e}"))
    }

    // ============================================================
    // 推理
    // ============================================================

    /// 纯数据转换：`Vec<u32>` → `[1, seq_len]` u32 Tensor。
    ///
    /// 不涉及模型计算。Embedding 层查表在 `forward` 内部完成。
    pub fn tensorize(&self, token_ids: &[u32]) -> Result<Tensor, String> {
        Tensor::new(token_ids, &self.model.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| format!("Tensorize failed: {e}"))
    }

    /// 单次前向推理。
    ///
    /// 输入 Tensor 送入模型，结果存入内部 `output`。
    ///
    /// # 参数
    /// - `tensor`: 输入（u32 `[1, seq_len]` 或 f32 `[1, seq_len, hidden]`）
    /// - `offset`: `None` 时使用内部自增 offset
    pub fn forward(&mut self, tensor: &Tensor, offset: Option<usize>) -> Result<(), String> {
        let off = offset.unwrap_or(self.offset);
        let seq_len = tensor.dims().get(1).copied().unwrap_or(1);

        let output = GGUF_Model_Inference(&mut self.model, tensor, off)
            .map_err(|e| format!("Forward failed: {e}"))?;

        self.model.device.synchronize()
            .map_err(|e| format!("Device sync failed: {e}"))?;

        self.output = Some(output);

        if offset.is_none() {
            self.offset += seq_len;
        }

        Ok(())
    }

    // ============================================================
    // 采样
    // ============================================================

    /// 从内部 output（logits）采样下一个 token。
    ///
    /// - `temperature ≤ 0.0`: argmax
    /// - `temperature > 0.0`: softmax 随机采样 (xoshiro)
    pub fn sample(&mut self, temperature: f64) -> Result<u32, String> {
        let logits = self.output.as_ref()
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

            // xoshiro 随机数生成器
            self.rng_state = self
                .rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let random = (self.rng_state >> 33) as f32 / (u32::MAX as f32);

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

    // ============================================================
    // 状态查询
    // ============================================================

    /// 获取内部 output Tensor 的引用（供 TensorStream 发送）。
    pub fn get_output_tensor(&self) -> Option<&Tensor> {
        self.output.as_ref()
    }

    /// 获取 EOS token ID。
    pub fn get_eos(&self) -> u32 {
        self.eos_token_id
    }

    /// 获取当前序列位置偏移量。
    pub fn get_offset(&self) -> usize {
        self.offset
    }

    // ============================================================
    // 辅助
    // ============================================================

    /// 从 logits Tensor 提取最后一个位置的 logits。
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
            _ => Err(format!(
                "Unsupported logits shape: {:?}",
                dims
            )),
        }
    }
}
