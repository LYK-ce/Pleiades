//Presented by KeJi
//Date ： 2026-05-27

//! OpenAI 兼容 API — 路由处理器
//!
//! 架构：
//! - 后台 handler task 持有 slot（prompt_tx + token_rx），常驻不释放
//! - HTTP handler 通过 mpsc 发送请求，通过 oneshot 获取响应
//! - 请求串行处理，避免并发竞态

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
};
use futures::stream::Stream;
use tokio::sync::{mpsc, oneshot};

use super::types::*;

// ─── 请求/响应协议 ──────────────────────────────────────────

/// HTTP handler → 后台 handler task 的请求
pub(crate) struct ApiRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    pub stream: bool,
    pub reply_tx: oneshot::Sender<ApiResponse>,
}

/// 后台 handler task → HTTP handler 的响应
pub(crate) enum ApiResponse {
    /// 非流式：完整响应文本
    NonStreaming(String),
    /// 流式：逐 token 通道（读到空或 "\0" 表示结束）
    Stream(mpsc::UnboundedReceiver<String>),
    /// 错误
    Error(String),
}

// ─── 共享状态 ───────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct ApiState {
    pub model_id: String,
    pub request_tx: mpsc::UnboundedSender<ApiRequest>,
}

// ─── 后台 handler task：持有 slot，串行处理请求 ─────────────

pub(crate) fn spawn_slot_handler(
    prompt_tx: tokio::sync::mpsc::UnboundedSender<crate::session::SessionRequest>,
    mut token_rx: mpsc::UnboundedReceiver<String>,
    session_id: u64,
) -> mpsc::UnboundedSender<ApiRequest> {
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<ApiRequest>();

    tokio::spawn(async move {
        tracing::info!("API slot handler started for session {}", session_id);
        while let Some(req) = request_rx.recv().await {
            let session_req = crate::session::SessionRequest {
                messages: req.messages.into_iter().map(|m| {
                    crate::ml_engine::context::Message {
                        role: m.role,
                        content: m.content,
                    }
                }).collect(),
                max_tokens: req.max_tokens,
            };

            if prompt_tx.send(session_req).is_err() {
                let _ = req.reply_tx.send(ApiResponse::Error("session closed".into()));
                break;
            }

            if req.stream {
                let (tx, rx) = mpsc::unbounded_channel();
                let _ = req.reply_tx.send(ApiResponse::Stream(rx));
                while let Some(token) = token_rx.recv().await {
                    if token == "\0" { break; }
                    if tx.send(token).is_err() { break; }
                }
            } else {
                let mut content = String::new();
                while let Some(token) = token_rx.recv().await {
                    if token == "\0" { break; }
                    content.push_str(&token);
                }
                let _ = req.reply_tx.send(ApiResponse::NonStreaming(content));
            }
        }
        tracing::info!("API slot handler for session {} exiting", session_id);
    });

    request_tx
}

// ─── GET /v1/models ─────────────────────────────────────────

pub(crate) async fn list_models(
    State(state): State<ApiState>,
) -> Json<ModelsResponse> {
    Json(ModelsResponse {
        object: "list".into(),
        data: vec![ModelEntry {
            id: state.model_id.clone(),
            object: "model".into(),
            created: 0,
            owned_by: "pleiades".into(),
        }],
    })
}

// ─── POST /v1/chat/completions ──────────────────────────────

pub(crate) async fn chat_completions(
    State(state): State<ApiState>,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<axum::response::Response, StatusCode> {
     // 验证至少有一条消息
    if req.messages.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let completion_id = format!("chatcmpl-{}", rand_id());
    let (reply_tx, reply_rx) = oneshot::channel();

    state
        .request_tx
        .send(ApiRequest {
            messages: req.messages,
            max_tokens: req.max_tokens,
            stream: req.stream,
            reply_tx,
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = reply_rx.await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match response {
        ApiResponse::Error(msg) => {
            let body = serde_json::json!({"error": msg});
            Ok((StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response())
        }
        ApiResponse::NonStreaming(content) => {
            let created = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            Ok(Json(ChatCompletionResponse {
                id: completion_id,
                object: "chat.completion".into(),
                created,
                model: state.model_id.clone(),
                choices: vec![Choice {
                    index: 0,
                    message: ChoiceMessage {
                        role: "assistant".into(),
                        content,
                    },
                    finish_reason: "stop".into(),
                }],
            })
            .into_response())
        }
        ApiResponse::Stream(token_rx) => {
            let stream = sse_token_stream(token_rx, completion_id, state.model_id.clone());
            Ok(Sse::new(stream)
                .keep_alive(KeepAlive::default())
                .into_response())
        }
    }
}

// ─── token_rx → SSE stream ──────────────────────────────────

fn sse_token_stream(
    mut token_rx: mpsc::UnboundedReceiver<String>,
    completion_id: String,
    model_id: String,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    async_stream::stream! {
        let created = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        while let Some(token) = token_rx.recv().await {
            if token == "\0" { break; }
            let chunk = ChatCompletionChunk {
                id: completion_id.clone(),
                object: "chat.completion.chunk".into(),
                created,
                model: model_id.clone(),
                choices: vec![ChunkChoice {
                    index: 0,
                    delta: ChunkDelta { content: Some(token) },
                    finish_reason: None,
                }],
            };
            yield Ok(Event::default().data(
                serde_json::to_string(&chunk).unwrap_or_default(),
            ));
        }

        let finish_chunk = ChatCompletionChunk {
            id: completion_id,
            object: "chat.completion.chunk".into(),
            created,
            model: model_id,
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta { content: None },
                finish_reason: Some("stop".into()),
            }],
        };
        yield Ok(Event::default().data(
            serde_json::to_string(&finish_chunk).unwrap_or_default(),
        ));
        yield Ok(Event::default().data("[DONE]"));
    }
}

// ─── 简单随机 ID ────────────────────────────────────────────

fn rand_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:08x}", nanos)
}
