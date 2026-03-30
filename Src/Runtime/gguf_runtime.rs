//Presented by KeJi
//Date : 2026-03-30

//! gguf_runtime 模块
//!
//! 供上层使用的统一 GGUF Runtime 模块，是系统总 manager 调用的接口。
//! 负责读取、解析、切分、推理的功能。
//! 外部只需操纵 GGUF_Runtime 结构体即可，所有功能均作为其方法提供。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use anyhow::Result;
use candle_core::Device;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use std::path::Path;

use crate::config::Runtime_Config;
use super::gguf_model::{
    GGUF_Model, GGUF_Load_Model, GGUF_Unload_Model,
    GGUF_Model_Inference, GGUF_Encode, GGUF_Decode,
    Inference_Config,
};

// ============================================================
// 数据结构定义
// ============================================================

/// GGUF Runtime 结构体
///
/// 供上层统一调用的 Runtime 接口，封装了模型的加载、推理、卸载等功能。
/// 外部只需操纵此结构体，通过其方法完成所有操作。
///
/// 包含以下属性：
/// - model: GGUF_Model 模型结构体，包含模型的所有信息和权重
pub struct GGUF_Runtime {
    /// 模型结构体，包含模型的所有信息和权重
    pub model: GGUF_Model,
}

// ============================================================
// GGUF_Runtime 方法实现
// ============================================================

impl GGUF_Runtime {
    /// Init(runtime_config) -> GGUF_Runtime
    ///
    /// 初始化方法，传入 Runtime_Config（从 config.toml 中读取并解析），
    /// main.rs 将会传递 config 的 runtime 相关内容给此初始化方法。
    ///
    /// 初始化流程：
    /// 1. 从 Runtime_Config 解析设备类型（cpu/cuda）
    /// 2. 调用 GGUF_Load_Model 加载模型权重到内存中（内部已调用 GGUF_Analyze 分析模型结构）
    /// 3. 根据 Runtime_Config 中的推理参数配置 Inference_Config
    /// 4. 加载完成后即可进行推理
    ///
    /// # 参数
    /// - `runtime_config`: 从 config.toml 解析得到的 Runtime 配置
    ///
    /// # 返回
    /// 初始化好的 GGUF_Runtime 实例
    pub fn Init(runtime_config: &Runtime_Config) -> Result<GGUF_Runtime> {
        // 1. 解析设备类型
        let device_str = runtime_config
            .device
            .as_deref()
            .unwrap_or("cpu");

        let device = match device_str.to_lowercase().as_str() {
            "cpu" => Device::Cpu,
            "cuda" => Device::new_cuda(0)
                .map_err(|e| anyhow::anyhow!("CUDA 设备初始化失败: {}", e))?,
            other => anyhow::bail!(
                "不支持的设备类型: '{}'. 目前仅支持 'cpu' 和 'cuda'.",
                other
            ),
        };

        // 2. 获取模型路径
        let model_path_str = runtime_config
            .model_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Runtime 配置中未指定 model_path"))?;
        let model_path = Path::new(model_path_str);

        if !model_path.exists() {
            anyhow::bail!(
                "模型文件不存在: {}",
                model_path.display()
            );
        }

        // 3. 调用 GGUF_Load_Model 加载模型（内部已调用 GGUF_Analyze 分析模型结构）
        let mut gguf_model = GGUF_Load_Model(model_path, &device)?;

        // 4. 根据 Runtime_Config 配置 Inference_Config
        let default_config = Inference_Config::default();

        gguf_model.inference_config = Inference_Config {
            max_tokens: runtime_config
                .max_token
                .map(|v| v as usize)
                .unwrap_or(default_config.max_tokens),
            temperature: runtime_config
                .temperature
                .unwrap_or(default_config.temperature),
            seed: runtime_config
                .seed
                .unwrap_or(default_config.seed),
            eos_token: gguf_model.inference_config.eos_token, // 保持从 GGUF metadata 获取的值
        };

        Ok(GGUF_Runtime { model: gguf_model })
    }

    /// Inference_Stream(input) -> String
    ///
    /// 传入输入数据，进行完整的流式推理流程：
    /// 1. 判断 runtime 结构体中是否存在输入头，如果存在输入头，调用 Encode 方法将文本编码为 token IDs；
    ///    否则直接将输入视为已编码的 token（此场景暂不支持，预留接口）
    /// 2. 调用 GGUF_Model_Inference 方法进行自回归推理
    /// 3. 推理完成后判断 runtime 结构体是否存在输出头，如果存在输出头，调用 Decode 方法进行解码；
    ///    否则直接返回生成的 token IDs 的字符串表示
    /// 4. 返回推理结果
    ///
    /// # 参数
    /// - `input`: 输入文本
    ///
    /// # 返回
    /// 推理生成的文本结果
    pub fn Inference_Stream(&mut self, input: &str) -> Result<String> {
        // 1. 判断是否存在输入头，进行编码
        let token_ids = if self.model.has_input_head {
            GGUF_Encode(&self.model, input)?
        } else {
            anyhow::bail!(
                "模型不包含输入头 (embedding)，无法直接接受文本输入。\
                 部分模型暂不支持此推理模式。"
            );
        };

        // 2. 读取推理配置参数
        let max_tokens = self.model.inference_config.max_tokens;
        let temperature = self.model.inference_config.temperature;
        let seed = self.model.inference_config.seed;
        let eos_token = self.model.inference_config.eos_token;

        // 创建 logits 处理器（用于采样）
        let sampling = Sampling::All { temperature };
        let mut logits_processor = LogitsProcessor::from_sampling(seed, sampling);

        // Prefill: 首次前向传播处理整个 prompt
        let logits = GGUF_Model_Inference(&mut self.model, &token_ids, 0)?;
        let logits = logits
            .squeeze(0)
            .map_err(|e| anyhow::anyhow!("Squeeze 失败: {}", e))?;
        let mut next_token = logits_processor
            .sample(&logits)
            .map_err(|e| anyhow::anyhow!("采样失败: {}", e))?;

        let mut generated_tokens: Vec<u32> = vec![next_token];

        // Auto-regressive 生成: 逐 token 调用 inference
        for index in 0..max_tokens {
            // 检查 EOS
            if next_token == eos_token {
                break;
            }

            // 单步前向推理（单个 token）
            let offset = token_ids.len() + index;
            let logits = GGUF_Model_Inference(&mut self.model, &[next_token], offset)?;
            let logits = logits
                .squeeze(0)
                .map_err(|e| anyhow::anyhow!("Squeeze 失败: {}", e))?;

            // 采样下一个 token
            next_token = logits_processor
                .sample(&logits)
                .map_err(|e| anyhow::anyhow!("采样失败: {}", e))?;
            generated_tokens.push(next_token);
        }

        // 3. 判断是否存在输出头，进行解码
        let result = if self.model.has_output_head {
            GGUF_Decode(&self.model, &generated_tokens)?
        } else {
            // 无输出头时，直接返回 token IDs 的字符串表示
            format!("{:?}", generated_tokens)
        };

        Ok(result)
    }

    /// Unload(self)
    ///
    /// 卸载 Runtime，释放模型占用的资源。
    /// 消耗 self 所有权，确保资源被完全释放。
    ///
    /// # 返回
    /// - `true`: 卸载成功
    pub fn Unload(self) -> bool {
        GGUF_Unload_Model(self.model)
    }
}
