//Presented by KeJi
//Date ： 2026-05-27

//! OpenAI 兼容 API — 类型定义

use serde::{Deserialize, Serialize};

// ─── 请求类型 ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    256
}

#[derive(Debug, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

// ─── 响应类型（非流式）──────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
}

#[derive(Debug, Serialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChoiceMessage,
    pub finish_reason: String,
}

#[derive(Debug, Serialize)]
pub struct ChoiceMessage {
    pub role: String,
    pub content: String,
}

// ─── 响应类型（流式 SSE chunk）─────────────────────────────

#[derive(Debug, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
}

#[derive(Debug, Serialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: ChunkDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

// ─── /v1/models 响应 ───────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<ModelEntry>,
}

#[derive(Debug, Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}
