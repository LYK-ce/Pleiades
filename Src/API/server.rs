//Presented by KeJi
//Date ： 2026-05-27

//! OpenAI 兼容 API — HTTP Server 启动

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{Router, routing::{get, post}};

use crate::session::SessionManager;
use super::routes::{self, ApiState};

/// 启动一个绑定到指定 session 的 API HTTP server
///
/// 分配一个 slot 常驻不释放，端口自动分配，返回实际端口号。
pub async fn spawn_api_server(
    session_mgr: Arc<Mutex<SessionManager>>,
    session_id: u64,
    model_id: String,
) -> Result<u16, String> {
    // 分配 slot（常驻，由后台 handler task 持有）
    let handle = {
        let mut mgr = session_mgr
            .lock()
            .map_err(|e| format!("session_mgr lock: {}", e))?;
        mgr.allocate_slot(session_id)
            .map_err(|e| format!("allocate_slot: {} (session may not exist or slots exhausted)", e))?
    };

    let prompt_tx = handle.prompt_tx;
    let token_rx = handle.token_rx;

    // 启动后台 handler task，保持 slot 存活
    let request_tx = routes::spawn_slot_handler(prompt_tx, token_rx, session_id);

    let state = ApiState {
        model_id,
        request_tx,
    };

    let app = Router::new()
        .route("/v1/models", get(routes::list_models))
        .route("/v1/chat/completions", post(routes::chat_completions))
        .with_state(state);

    let port = find_available_port(8080)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {}: {}", addr, e))?;

    tracing::info!("API server for session {} listening on http://{}", session_id, addr);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("API server (session {}) exited: {}", session_id, e);
        }
    });

    Ok(port)
}

/// 查找第一个可用端口（从 start 起递增）
fn find_available_port(start: u16) -> Result<u16, String> {
    for port in start..start + 100 {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        if std::net::TcpListener::bind(addr).is_ok() {
            return Ok(port);
        }
    }
    Err(format!("no available port in range {}-{}", start, start + 99))
}
