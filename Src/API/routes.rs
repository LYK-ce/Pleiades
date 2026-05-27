//Presented by KeJi
//Date ： 2026-05-27

//! OpenAI 兼容 API — 路由处理器

use std::sync::Arc;
use std::sync::Mutex;

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
};
use futures::stream::Stream;
use tokio::sync::mpsc;

use crate::session::{SessionManager, Session_Error};
use super::types::*;

// ─── 共享状态 ───────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct ApiState {
    pub session_mgr: Arc<Mutex<SessionManager>>,
    pub session_id: u64,
    pub model_id: String,
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
    // 提取最后一条 user message 作为 prompt
    let prompt = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let completion_id = format!("chatcmpl-{}", rand_id());

    // 分配 slot
    let handle = {
        let mut mgr = state.session_mgr.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        mgr.allocate_slot(state.session_id).map_err(|e| match e {
            Session_Error::SessionNotFound(_) => StatusCode::NOT_FOUND,
            Session_Error::SlotExhausted(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?
    };

    let prompt_tx = handle.prompt_tx;
    let mut token_rx = handle.token_rx;
    let slot_id = handle.slot_id;

    // 发送 prompt
    prompt_tx.send(prompt).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if req.stream {
        // ── 流式 SSE 响应 ──────────────────────────
        let stream = token_stream(token_rx, completion_id.clone(), state.model_id.clone());
        let sse = Sse::new(stream).keep_alive(KeepAlive::default());
        Ok(sse.into_response())
    } else {
        // ── 非流式 JSON 响应 ───────────────────────
        let mut content = String::new();
        while let Some(token) = token_rx.recv().await {
            if token == "\0" { break; }
            content.push_str(&token);
        }
        // 释放 slot
        {
            let mut mgr = state.session_mgr.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            mgr.close_slot(state.session_id, slot_id);
        }

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
        }).into_response())
    }
}

// ─── token → SSE stream ─────────────────────────────────────

fn token_stream(
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
            let json = serde_json::to_string(&chunk).unwrap_or_default();
            yield Ok(Event::default().data(json));
        }

        // 结束 chunk
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
        let json = serde_json::to_string(&finish_chunk).unwrap_or_default();
        yield Ok(Event::default().data(json));

        // [DONE] 哨兵
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
