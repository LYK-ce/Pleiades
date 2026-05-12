//Presented by KeJi
//Date ： 2026-04-13

//! Pipeline 参数与结果类型。
//!
//! 由 Control 层构造 Pipeline_Params 传入执行引擎，引擎执行完成后返回 Pipeline_Result。
//! Model_Info 由 Create_Session 返回并随 Pipeline_Result 透传。

use std::time::Duration;

// ============================================================
// Pipeline 参数（Control 层 → Engine）
// ============================================================

#[derive(Debug, Clone)]
pub struct Pipeline_Params {
    pub max_tokens: usize,
    pub temperature: f64,
    pub seed: u64,
    pub eos_token_id: Option<u32>,
}

impl Default for Pipeline_Params {
    fn default() -> Self {
        Self {
            max_tokens: 120,
            temperature: 0.8,
            seed: 299792458,
            eos_token_id: None,
        }
    }
}

// ============================================================
// Pipeline 结果（Engine → Control 层）
// ============================================================

#[derive(Debug)]
pub struct Pipeline_Result {
    pub result_text: String,
    pub generated_tokens: Vec<u32>,
    pub prompt_tokens: Vec<u32>,
    pub model_info: Option<Model_Info>,
    pub duration: Duration,
    pub inference_duration: Duration,
    pub total_steps: usize,
}

// ============================================================
// 模型信息
// ============================================================

#[derive(Debug, Clone)]
pub struct Model_Info {
    pub architecture: String,
    pub num_layers: usize,
    pub embedding_length: usize,
    pub has_input_head: bool,
    pub has_output_head: bool,
    pub has_tokenizer: bool,
    pub eos_token_id: u32,
}
